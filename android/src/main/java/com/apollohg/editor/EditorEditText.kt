package com.apollohg.editor

import android.content.ClipboardManager
import android.content.Context
import android.graphics.Typeface
import android.graphics.Rect
import android.graphics.RectF
import android.text.Editable
import android.text.Layout
import android.text.Spanned
import android.text.StaticLayout
import android.text.TextWatcher
import android.util.AttributeSet
import android.util.Log
import android.util.TypedValue
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import androidx.appcompat.widget.AppCompatEditText
import kotlin.math.roundToInt
import uniffi.editor_core.*  // UniFFI-generated bindings

/**
 * Custom [AppCompatEditText] subclass that intercepts all text input and routes it
 * through the Rust editor-core engine via UniFFI bindings.
 *
 * Instead of letting Android's EditText internal text storage handle insertions
 * and deletions, this class captures the user's intent (typing, deleting,
 * pasting, autocorrect) and sends it to the Rust editor. The Rust editor
 * returns render elements, which are converted to [android.text.SpannableStringBuilder]
 * via [RenderBridge] and applied back to the EditText.
 *
 * This is the "input interception" pattern: the EditText is effectively
 * a rendering surface, not a text editing engine.
 *
 * ## Composition Handling
 *
 * For CJK input methods, composing text is handled normally by the base
 * [InputConnection]. When composition finalizes, we capture the result and
 * route it through Rust.
 *
 * ## Thread Safety
 *
 * All EditText methods are called on the main thread. The UniFFI calls
 * (`editor_insert_text`, `editor_delete_range`, etc.) are synchronous and
 * fast enough for main-thread use.
 */
class EditorEditText @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = android.R.attr.editTextStyle
) : AppCompatEditText(context, attrs, defStyleAttr) {
    data class SelectedImageGeometry(
        val docPos: Int,
        val rect: RectF
    )

    private data class ImageSelectionRange(
        val start: Int,
        val end: Int
    )

    /**
     * Listener interface for editor events, parallel to iOS's EditorTextViewDelegate.
     */
    interface EditorListener {
        /** Called when the editor's selection changes (anchor and head as scalar offsets). */
        fun onSelectionChanged(anchor: Int, head: Int)

        /** Called when the editor content is updated after a Rust operation. */
        fun onEditorUpdate(updateJSON: String)
    }

    /** The Rust editor instance ID (from editor_create / editor_create_with_max_length). */
    var editorId: Long = 0

    /**
     * Controls whether user input is accepted.
     *
     * When false, all user-input mutation entry points (typing, deletion,
     * paste, composition) are blocked. Unlike [isEnabled], this preserves
     * focus, text selection, and copy capability.
     */
    var isEditable: Boolean = true

    /**
     * Guard flag to prevent re-entrant input interception while we're
     * applying state from Rust (calling [setText] or modifying text storage).
     */
    var isApplyingRustState = false

    /** Listener for editor events. */
    var editorListener: EditorListener? = null
    var onSelectionOrContentMayChange: (() -> Unit)? = null

    /** The base font size in pixels used for unstyled text. */
    private var baseFontSize: Float = textSize

    /** The base text color as an ARGB int. */
    private var baseTextColor: Int = currentTextColor

    /** The base background color before theme overrides. */
    private var baseBackgroundColor: Int = android.graphics.Color.WHITE

    /** Optional render theme supplied by React. */
    var theme: EditorTheme? = null
        private set

    var placeholderText: String = ""
        set(value) {
            field = value
            invalidate()
        }

    var heightBehavior: EditorHeightBehavior = EditorHeightBehavior.FIXED
        private set
    private var imageResizingEnabled = true

    private var contentInsets: EditorContentInsets? = null
    private var viewportBottomInsetPx: Int = 0

    /**
     * The plain text from the last Rust-authorized render.
     * Used by [ReconciliationWatcher] to detect unauthorized divergence.
     */
    private var lastAuthorizedText: String = ""

    /**
     * Number of reconciliation events triggered during this EditText's lifetime.
     * Useful for monitoring and kill-condition analysis.
     */
    var reconciliationCount: Int = 0
        private set

    private var lastHandledHardwareKeyCode: Int? = null
    private var lastHandledHardwareKeyDownTime: Long? = null
    private var explicitSelectedImageRange: ImageSelectionRange? = null

    init {
        // Configure for rich text editing.
        inputType = EditorInfo.TYPE_CLASS_TEXT or
                EditorInfo.TYPE_TEXT_FLAG_MULTI_LINE or
                EditorInfo.TYPE_TEXT_FLAG_AUTO_CORRECT or
                EditorInfo.TYPE_TEXT_FLAG_CAP_SENTENCES

        // Disable built-in spell checking to avoid conflicts with Rust state.
        // The Rust editor is the source of truth for text content.
        isSaveEnabled = false

        // Watch for unauthorized text mutations (IME, accessibility, etc.)
        // and reconcile back to Rust's authoritative state.
        addTextChangedListener(ReconciliationWatcher())
        baseBackgroundColor = android.graphics.Color.WHITE
        isVerticalScrollBarEnabled = true
        overScrollMode = OVER_SCROLL_IF_CONTENT_SCROLLS

        // Pin content to top-start to prevent theme-dependent vertical centering.
        gravity = android.view.Gravity.TOP or android.view.Gravity.START

        // Strip the default EditText theme drawable which carries implicit padding.
        // Background color is applied in setBaseStyle() / applyTheme().
        background = null
        linksClickable = false
        updateEffectivePadding()
    }

    // ── InputConnection Override ────────────────────────────────────────

    /**
     * Create a custom [EditorInputConnection] that intercepts all input
     * from the soft keyboard.
     */
    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        val baseConnection = super.onCreateInputConnection(outAttrs) ?: return null
        return EditorInputConnection(this, baseConnection)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (handleHardwareKeyEvent(event)) {
            return true
        }
        return super.dispatchKeyEvent(event)
    }

    override fun onDraw(canvas: android.graphics.Canvas) {
        super.onDraw(canvas)

        if (!shouldDisplayPlaceholder()) return

        val availableWidth = width - compoundPaddingLeft - compoundPaddingRight
        if (availableWidth <= 0) return

        val previousColor = paint.color
        val saveCount = canvas.save()
        paint.color = currentHintTextColor
        canvas.translate(compoundPaddingLeft.toFloat(), extendedPaddingTop.toFloat())
        StaticLayout.Builder
            .obtain(placeholderText, 0, placeholderText.length, paint, availableWidth)
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(includeFontPadding)
            .build()
            .draw(canvas)
        canvas.restoreToCount(saveCount)
        paint.color = previousColor
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN && imageSpanHitAt(event.x, event.y) == null) {
            clearExplicitSelectedImageRange()
        }
        if (handleImageTap(event)) {
            return true
        }
        if (heightBehavior == EditorHeightBehavior.FIXED) {
            val canScroll = canScrollVertically(-1) || canScrollVertically(1)
            if (canScroll) {
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN,
                    MotionEvent.ACTION_MOVE -> parent?.requestDisallowInterceptTouchEvent(true)
                    MotionEvent.ACTION_UP,
                    MotionEvent.ACTION_CANCEL -> parent?.requestDisallowInterceptTouchEvent(false)
                }
            }
        }
        return super.onTouchEvent(event)
    }

    override fun performClick(): Boolean {
        return super.performClick()
    }

    private fun isRenderedContentEmpty(content: CharSequence? = text): Boolean {
        val renderedContent = content ?: return true
        if (renderedContent.isEmpty()) return true

        for (index in 0 until renderedContent.length) {
            when (renderedContent[index]) {
                EMPTY_BLOCK_PLACEHOLDER, '\n', '\r' -> continue
                else -> return false
            }
        }

        return true
    }

    private fun shouldDisplayPlaceholder(): Boolean {
        return placeholderText.isNotEmpty() && isRenderedContentEmpty()
    }

    fun shouldDisplayPlaceholderForTesting(): Boolean = shouldDisplayPlaceholder()

    // ── Editor Binding ──────────────────────────────────────────────────

    /**
     * Bind this EditText to a Rust editor instance and optionally apply initial content.
     *
     * @param id The editor ID from `editor_create()`.
     * @param initialHTML Optional HTML to set as initial content.
     */
    fun bindEditor(id: Long, initialHTML: String? = null) {
        editorId = id

        if (!initialHTML.isNullOrEmpty()) {
            val renderJSON = editorSetHtml(editorId.toULong(), initialHTML)
            applyRenderJSON(renderJSON)
        } else {
            // Pull current state from Rust (content may already be loaded via bridge).
            val stateJSON = editorGetCurrentState(editorId.toULong())
            applyUpdateJSON(stateJSON)
        }
    }

    /**
     * Unbind from the current editor instance.
     */
    fun unbindEditor() {
        editorId = 0
    }

    fun setBaseStyle(fontSizePx: Float, textColor: Int, backgroundColor: Int) {
        baseFontSize = fontSizePx
        baseTextColor = textColor
        baseBackgroundColor = backgroundColor
        setTextSize(TypedValue.COMPLEX_UNIT_PX, fontSizePx)
        setTextColor(textColor)
        setBackgroundColor(theme?.backgroundColor ?: backgroundColor)
    }

    fun applyTheme(theme: EditorTheme?) {
        this.theme = theme
        setBackgroundColor(theme?.backgroundColor ?: baseBackgroundColor)
        applyContentInsets(theme?.contentInsets)
        if (editorId != 0L) {
            val previousScrollX = scrollX
            val previousScrollY = scrollY
            val stateJSON = editorGetCurrentState(editorId.toULong())
            applyUpdateJSON(stateJSON, notifyListener = false)
            if (heightBehavior == EditorHeightBehavior.FIXED) {
                preserveScrollPosition(previousScrollX, previousScrollY)
            } else {
                requestLayout()
            }
        }
    }

    fun setHeightBehavior(heightBehavior: EditorHeightBehavior) {
        if (this.heightBehavior == heightBehavior) return
        this.heightBehavior = heightBehavior
        isVerticalScrollBarEnabled = heightBehavior == EditorHeightBehavior.FIXED
        overScrollMode = if (heightBehavior == EditorHeightBehavior.FIXED) {
            OVER_SCROLL_IF_CONTENT_SCROLLS
        } else {
            OVER_SCROLL_NEVER
        }
        updateEffectivePadding()
        ensureSelectionVisible()
        requestLayout()
    }

    private fun applyContentInsets(contentInsets: EditorContentInsets?) {
        this.contentInsets = contentInsets
        updateEffectivePadding()
    }

    fun setViewportBottomInsetPx(bottomInsetPx: Int) {
        val clampedInset = bottomInsetPx.coerceAtLeast(0)
        if (viewportBottomInsetPx == clampedInset) return
        viewportBottomInsetPx = clampedInset
        updateEffectivePadding()
        ensureSelectionVisible()
    }

    private fun updateEffectivePadding() {
        val density = resources.displayMetrics.density
        val left = ((contentInsets?.left ?: 0f) * density).toInt()
        val top = ((contentInsets?.top ?: 0f) * density).toInt()
        val right = ((contentInsets?.right ?: 0f) * density).toInt()
        val bottom = ((contentInsets?.bottom ?: 0f) * density).toInt()

        if (heightBehavior == EditorHeightBehavior.FIXED) {
            setPadding(left, 0, right, 0)
            setCompoundDrawablesRelativeWithIntrinsicBounds(null, null, null, null)
        } else {
            setPadding(left, top, right, bottom)
            setCompoundDrawablesRelativeWithIntrinsicBounds(null, null, null, null)
        }
    }

    fun setImageResizingEnabled(enabled: Boolean) {
        if (imageResizingEnabled == enabled) return
        imageResizingEnabled = enabled
        if (!enabled) {
            clearExplicitSelectedImageRange()
        } else {
            onSelectionOrContentMayChange?.invoke()
        }
    }

    fun resolveAutoGrowHeight(): Int {
        val laidOutTextHeight = if (isLaidOut) layout?.height else null
        if (laidOutTextHeight != null && laidOutTextHeight > 0) {
            return laidOutTextHeight + compoundPaddingTop + compoundPaddingBottom
        }

        val availableWidth = (measuredWidth - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(0)
        val currentText = text
        if (availableWidth > 0 && currentText != null) {
            val staticLayout = StaticLayout.Builder
                .obtain(currentText, 0, currentText.length, paint, availableWidth)
                .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                .setIncludePad(includeFontPadding)
                .build()
            val textHeight = staticLayout.height.takeIf { it > 0 } ?: lineHeight
            return textHeight + compoundPaddingTop + compoundPaddingBottom
        }

        val minimumHeight = suggestedMinimumHeight.coerceAtLeast(minHeight)
        return (lineHeight + compoundPaddingTop + compoundPaddingBottom).coerceAtLeast(minimumHeight)
    }

    private fun preserveScrollPosition(previousScrollX: Int, previousScrollY: Int) {
        val restore = {
            val maxScrollX = maxOf(0, computeHorizontalScrollRange() - width)
            val maxScrollY = maxOf(0, computeVerticalScrollRange() - height)
            scrollTo(
                previousScrollX.coerceIn(0, maxScrollX),
                previousScrollY.coerceIn(0, maxScrollY)
            )
        }

        restore()
        post { restore() }
    }

    private fun ensureSelectionVisible() {
        if (heightBehavior != EditorHeightBehavior.FIXED) return
        if (!isLaidOut || width <= 0 || height <= 0) return
        val selectionOffset = selectionEnd.takeIf { it >= 0 } ?: return

        post {
            if (!isLaidOut || layout == null) return@post
            bringPointIntoView(selectionOffset)

            val textLayout = layout ?: return@post
            val clampedOffset = selectionOffset.coerceAtMost(textLayout.text.length)
            val line = textLayout.getLineForOffset(clampedOffset)
            val caretLeft = textLayout.getPrimaryHorizontal(clampedOffset).toInt()
            val rect = Rect(
                caretLeft + totalPaddingLeft,
                textLayout.getLineTop(line) + totalPaddingTop,
                caretLeft + totalPaddingLeft + 1,
                textLayout.getLineBottom(line) + totalPaddingTop
            )
            requestRectangleOnScreen(rect)
        }
    }

    // ── Input Handling: Text Commit ─────────────────────────────────────

    /**
     * Handle committed text from the IME (typed characters, autocomplete).
     *
     * Called by [EditorInputConnection.commitText]. Routes the text through
     * the Rust editor instead of directly inserting into the EditText.
     */
    fun handleTextCommit(text: String) {
        if (!isEditable) return
        if (isApplyingRustState) return
        if (editorId == 0L) {
            // No Rust editor bound — fall through to direct editing (dev mode).
            val editable = this.text ?: return
            val start = selectionStart
            val end = selectionEnd
            editable.replace(start, end, text)
            return
        }

        // Handle Enter/Return as a block split operation.
        if (text == "\n") {
            handleReturnKey()
            return
        }

        val currentText = this.text?.toString() ?: ""
        val start = selectionStart
        val end = selectionEnd

        if (start != end) {
            // Range selection: atomic replace via Rust.
            val scalarStart = PositionBridge.utf16ToScalar(start, currentText)
            val scalarEnd = PositionBridge.utf16ToScalar(end, currentText)
            val updateJSON = editorReplaceTextScalar(
                editorId.toULong(), scalarStart.toUInt(), scalarEnd.toUInt(), text
            )
            applyUpdateJSON(updateJSON)
        } else {
            val scalarPos = PositionBridge.utf16ToScalar(start, currentText)
            insertTextInRust(text, scalarPos)
        }
    }

    // ── Input Handling: Deletion ────────────────────────────────────────

    /**
     * Handle surrounding text deletion from the IME.
     *
     * Called by [EditorInputConnection.deleteSurroundingText].
     *
     * @param beforeLength Number of UTF-16 code units to delete before the cursor.
     * @param afterLength Number of UTF-16 code units to delete after the cursor.
     */
    fun handleDelete(beforeLength: Int, afterLength: Int) {
        if (!isEditable) return
        if (isApplyingRustState) return
        if (editorId == 0L) {
            // Dev mode: direct editing.
            val editable = this.text ?: return
            val cursor = selectionStart
            val delStart = maxOf(0, cursor - beforeLength)
            val delEnd = minOf(editable.length, cursor + afterLength)
            editable.delete(delStart, delEnd)
            return
        }

        val currentText = text?.toString() ?: ""
        val cursor = selectionStart
        val delStart = maxOf(0, cursor - beforeLength)
        val delEnd = minOf(currentText.length, cursor + afterLength)

        val scalarStart = PositionBridge.utf16ToScalar(delStart, currentText)
        val scalarEnd = PositionBridge.utf16ToScalar(delEnd, currentText)

        if (scalarStart < scalarEnd) {
            deleteRangeInRust(scalarStart, scalarEnd)
        }
    }

    /**
     * Handle backspace key press (hardware keyboard or key event).
     *
     * If there's a range selection, deletes the range. Otherwise deletes
     * the grapheme cluster before the cursor.
     */
    fun handleBackspace() {
        if (!isEditable) return
        if (isApplyingRustState) return
        if (editorId == 0L) {
            // Dev mode: direct editing.
            val editable = this.text ?: return
            val start = selectionStart
            val end = selectionEnd
            if (start != end) {
                editable.delete(start, end)
            } else if (start > 0) {
                // Delete one grapheme cluster backward.
                val prevBoundary = PositionBridge.snapToGraphemeBoundary(start - 1, text?.toString() ?: "")
                val adjustedPrev = if (prevBoundary >= start) maxOf(0, start - 1) else prevBoundary
                editable.delete(adjustedPrev, start)
            }
            return
        }

        val currentText = text?.toString() ?: ""
        val start = selectionStart
        val end = selectionEnd

        if (start != end) {
            // Range selection: delete the range.
            val scalarStart = PositionBridge.utf16ToScalar(start, currentText)
            val scalarEnd = PositionBridge.utf16ToScalar(end, currentText)
            deleteRangeInRust(scalarStart, scalarEnd)
        } else if (start > 0) {
            // Cursor: delete one grapheme cluster backward.
            // Find the previous grapheme boundary by snapping (start - 1).
            val breakIter = java.text.BreakIterator.getCharacterInstance()
            breakIter.setText(currentText)
            val prevBoundary = breakIter.preceding(start)
            val prevUtf16 = if (prevBoundary == java.text.BreakIterator.DONE) 0 else prevBoundary

            val scalarStart = PositionBridge.utf16ToScalar(prevUtf16, currentText)
            val scalarEnd = PositionBridge.utf16ToScalar(start, currentText)
            deleteRangeInRust(scalarStart, scalarEnd)
        }
    }

    // ── Input Handling: Composition ─────────────────────────────────────

    /**
     * Handle finalization of IME composition (CJK input, swipe keyboard).
     *
     * Called by [EditorInputConnection.finishComposingText] after the base
     * InputConnection has finalized the composing text.
     */
    /**
     * Handle finalization of IME composition.
     *
     * @param composedText The finalized composed text captured from the InputConnection.
     */
    fun handleCompositionFinished(composedText: String?) {
        if (!isEditable) return
        if (isApplyingRustState) return
        if (editorId == 0L) return
        if (composedText.isNullOrEmpty()) return

        // The cursor is at the end of the composed text. Calculate the insert
        // position as cursor - composed_length (in scalar offsets).
        val currentText = text?.toString() ?: ""
        val cursorUtf16 = selectionStart
        val cursorScalar = PositionBridge.utf16ToScalar(cursorUtf16, currentText)
        val composedScalarLen = composedText.codePointCount(0, composedText.length)
        val insertPos = if (cursorScalar >= composedScalarLen) cursorScalar - composedScalarLen else 0
        insertTextInRust(composedText, insertPos)
    }

    // ── Input Handling: Return Key ──────────────────────────────────────

    /**
     * Handle return/enter key as a block split operation.
     */
    fun handleReturnKey() {
        if (!isEditable) return
        if (isApplyingRustState) return

        val currentText = text?.toString() ?: ""
        val start = selectionStart
        val end = selectionEnd

        if (editorId == 0L) {
            // Dev mode: insert newline directly.
            val editable = this.text ?: return
            editable.replace(start, end, "\n")
            return
        }

        if (start != end) {
            // Range selection: atomic delete-and-split via Rust.
            val scalarStart = PositionBridge.utf16ToScalar(start, currentText)
            val scalarEnd = PositionBridge.utf16ToScalar(end, currentText)
            val updateJSON = editorDeleteAndSplitScalar(
                editorId.toULong(), scalarStart.toUInt(), scalarEnd.toUInt()
            )
            applyUpdateJSON(updateJSON)
        } else {
            val scalarPos = PositionBridge.utf16ToScalar(start, currentText)
            splitBlockInRust(scalarPos)
        }
    }

    /**
     * Handle Shift+Enter as an inline hard break insertion.
     */
    fun handleHardBreak() {
        if (!isEditable) return
        if (isApplyingRustState) return

        if (editorId == 0L) {
            val editable = this.text ?: return
            val start = selectionStart
            val end = selectionEnd
            editable.replace(start, end, "\n")
            return
        }

        val selection = currentScalarSelection() ?: return
        val updateJSON = editorInsertNodeAtSelectionScalar(
            editorId.toULong(),
            selection.first.toUInt(),
            selection.second.toUInt(),
            "hardBreak"
        )
        applyUpdateJSON(updateJSON)
    }

    /**
     * Handle hardware Tab / Shift+Tab as list indent / outdent when the caret is in a list.
     */
    fun handleTab(shiftPressed: Boolean): Boolean {
        if (!isEditable) return false
        if (isApplyingRustState) return false
        if (editorId == 0L) return false
        if (!isSelectionInsideList()) return false
        val selection = currentScalarSelection() ?: return false

        val updateJSON = if (shiftPressed) {
            editorOutdentListItemAtSelectionScalar(
                editorId.toULong(),
                selection.first.toUInt(),
                selection.second.toUInt()
            )
        } else {
            editorIndentListItemAtSelectionScalar(
                editorId.toULong(),
                selection.first.toUInt(),
                selection.second.toUInt()
            )
        }
        applyUpdateJSON(updateJSON)
        return true
    }

    fun handleHardwareKeyDown(keyCode: Int, shiftPressed: Boolean): Boolean {
        if (!isEditable || isApplyingRustState) return false
        return when (keyCode) {
            KeyEvent.KEYCODE_DEL -> {
                handleBackspace()
                true
            }
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> {
                if (shiftPressed) {
                    handleHardBreak()
                } else {
                    handleReturnKey()
                }
                true
            }
            KeyEvent.KEYCODE_TAB -> handleTab(shiftPressed)
            else -> false
        }
    }

    fun handleHardwareKeyEvent(event: KeyEvent?): Boolean {
        if (event == null || !isEditable || isApplyingRustState) return false

        return when (event.action) {
            KeyEvent.ACTION_DOWN -> {
                val supported = when (event.keyCode) {
                    KeyEvent.KEYCODE_DEL,
                    KeyEvent.KEYCODE_ENTER,
                    KeyEvent.KEYCODE_NUMPAD_ENTER,
                    KeyEvent.KEYCODE_TAB -> true
                    else -> false
                }
                if (!supported) return false

                if (lastHandledHardwareKeyCode == event.keyCode &&
                    lastHandledHardwareKeyDownTime == event.downTime) {
                    return true
                }

                if (handleHardwareKeyDown(event.keyCode, event.isShiftPressed)) {
                    lastHandledHardwareKeyCode = event.keyCode
                    lastHandledHardwareKeyDownTime = event.downTime
                    true
                } else {
                    false
                }
            }

            KeyEvent.ACTION_UP -> {
                if (lastHandledHardwareKeyCode == event.keyCode &&
                    lastHandledHardwareKeyDownTime == event.downTime) {
                    lastHandledHardwareKeyCode = null
                    lastHandledHardwareKeyDownTime = null
                    true
                } else {
                    false
                }
            }

            else -> false
        }
    }

    fun performToolbarToggleMark(markName: String) {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        val selection = currentScalarSelection() ?: return
        val updateJSON = editorToggleMarkAtSelectionScalar(
            editorId.toULong(),
            selection.first.toUInt(),
            selection.second.toUInt(),
            markName
        )
        applyUpdateJSON(updateJSON)
    }

    fun performToolbarToggleList(listType: String, isActive: Boolean) {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        val selection = currentScalarSelection() ?: return
        val updateJSON = if (isActive) {
            editorUnwrapFromListAtSelectionScalar(
                editorId.toULong(),
                selection.first.toUInt(),
                selection.second.toUInt()
            )
        } else {
            editorWrapInListAtSelectionScalar(
                editorId.toULong(),
                selection.first.toUInt(),
                selection.second.toUInt(),
                listType
            )
        }
        applyUpdateJSON(updateJSON)
    }

    fun performToolbarToggleBlockquote() {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        val selection = currentScalarSelection() ?: return
        val updateJSON = editorToggleBlockquoteAtSelectionScalar(
            editorId.toULong(),
            selection.first.toUInt(),
            selection.second.toUInt()
        )
        applyUpdateJSON(updateJSON)
    }

    fun performToolbarIndentListItem() {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        val selection = currentScalarSelection() ?: return
        val updateJSON = editorIndentListItemAtSelectionScalar(
            editorId.toULong(),
            selection.first.toUInt(),
            selection.second.toUInt()
        )
        applyUpdateJSON(updateJSON)
    }

    fun performToolbarOutdentListItem() {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        val selection = currentScalarSelection() ?: return
        val updateJSON = editorOutdentListItemAtSelectionScalar(
            editorId.toULong(),
            selection.first.toUInt(),
            selection.second.toUInt()
        )
        applyUpdateJSON(updateJSON)
    }

    fun performToolbarInsertNode(nodeType: String) {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        val selection = currentScalarSelection() ?: return
        val updateJSON = editorInsertNodeAtSelectionScalar(
            editorId.toULong(),
            selection.first.toUInt(),
            selection.second.toUInt(),
            nodeType
        )
        applyUpdateJSON(updateJSON)
    }

    fun performToolbarUndo() {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        applyUpdateJSON(editorUndo(editorId.toULong()))
    }

    fun performToolbarRedo() {
        if (!isEditable || isApplyingRustState || editorId == 0L) return
        applyUpdateJSON(editorRedo(editorId.toULong()))
    }

    // ── Input Handling: Paste ────────────────────────────────────────────

    /**
     * Intercept paste operations to route content through Rust.
     *
     * Attempts to extract HTML from the clipboard first (for rich text paste),
     * falling back to plain text.
     */
    override fun onTextContextMenuItem(id: Int): Boolean {
        if (!isEditable && id == android.R.id.paste) return true
        if (id == android.R.id.paste) {
            handlePaste()
            return true
        }
        return super.onTextContextMenuItem(id)
    }

    /**
     * Block accessibility-initiated text mutations (paste, set text) when not editable.
     * Selection and copy actions remain available.
     */
    override fun performAccessibilityAction(action: Int, arguments: android.os.Bundle?): Boolean {
        if (!isEditable && (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT
                    || action == android.view.accessibility.AccessibilityNodeInfo.ACTION_PASTE)) {
            return false
        }
        return super.performAccessibilityAction(action, arguments)
    }

    private fun handlePaste() {
        if (editorId == 0L) {
            // Dev mode: default paste behavior.
            super.onTextContextMenuItem(android.R.id.paste)
            return
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            ?: return
        val clip = clipboard.primaryClip ?: return
        if (clip.itemCount == 0) return

        val item = clip.getItemAt(0)

        // Try HTML first for rich paste.
        val htmlText = item.htmlText
        if (htmlText != null) {
            pasteHTML(htmlText)
            return
        }

        // Fallback to plain text.
        val plainText = item.text?.toString()
        if (plainText != null) {
            pastePlainText(plainText)
        }
    }

    // ── Selection Change ────────────────────────────────────────────────

    /**
     * Override to notify the listener when selection changes.
     *
     * Converts the EditText selection to scalar offsets and notifies both
     * the listener and the Rust editor.
     */
    override fun onSelectionChanged(selStart: Int, selEnd: Int) {
        super.onSelectionChanged(selStart, selEnd)
        val spannable = text as? Spanned
        if (spannable != null && isExactImageSpanRange(spannable, selStart, selEnd)) {
            explicitSelectedImageRange = ImageSelectionRange(selStart, selEnd)
        }
        ensureSelectionVisible()
        onSelectionOrContentMayChange?.invoke()

        if (isApplyingRustState || editorId == 0L) return

        val currentText = text?.toString() ?: ""
        if (currentText != lastAuthorizedText) return
        val scalarAnchor = PositionBridge.utf16ToScalar(selStart, currentText)
        val scalarHead = PositionBridge.utf16ToScalar(selEnd, currentText)

        // Sync selection to Rust (converts scalar→doc internally).
        editorSetSelectionScalar(
            editorId.toULong(),
            scalarAnchor.toUInt(),
            scalarHead.toUInt()
        )

        // Emit doc positions (not scalar offsets) to match the Selection contract.
        val docAnchor = editorScalarToDoc(editorId.toULong(), scalarAnchor.toUInt()).toInt()
        val docHead = editorScalarToDoc(editorId.toULong(), scalarHead.toUInt()).toInt()
        editorListener?.onSelectionChanged(docAnchor, docHead)
    }

    // ── Rust Integration ────────────────────────────────────────────────

    /**
     * Insert text at a scalar position via the Rust editor.
     */
    private fun insertTextInRust(text: String, atScalarPos: Int) {
        val updateJSON = editorInsertTextScalar(editorId.toULong(), atScalarPos.toUInt(), text)
        applyUpdateJSON(updateJSON)
    }

    /**
     * Delete a scalar range via the Rust editor.
     *
     * @param scalarFrom Start scalar offset (inclusive).
     * @param scalarTo End scalar offset (exclusive).
     */
    private fun deleteRangeInRust(scalarFrom: Int, scalarTo: Int) {
        if (scalarFrom >= scalarTo) return
        val updateJSON = editorDeleteScalarRange(editorId.toULong(), scalarFrom.toUInt(), scalarTo.toUInt())
        applyUpdateJSON(updateJSON)
    }

    /**
     * Split a block at a scalar position via the Rust editor.
     */
    private fun splitBlockInRust(atScalarPos: Int) {
        val updateJSON = editorSplitBlockScalar(editorId.toULong(), atScalarPos.toUInt())
        applyUpdateJSON(updateJSON)
    }

    private fun currentScalarSelection(): Pair<Int, Int>? {
        val currentText = text?.toString() ?: return null
        return Pair(
            PositionBridge.utf16ToScalar(selectionStart, currentText),
            PositionBridge.utf16ToScalar(selectionEnd, currentText)
        )
    }

    fun selectedImageGeometry(): SelectedImageGeometry? {
        if (!imageResizingEnabled) return null
        val spannable = text as? Spanned ?: return null
        val selection = resolvedSelectedImageRange(spannable) ?: return null
        val start = selection.start
        val end = selection.end
        val imageSpan = spannable
            .getSpans(start, end, BlockImageSpan::class.java)
            .firstOrNull() ?: return null
        val spanStart = spannable.getSpanStart(imageSpan)
        val spanEnd = spannable.getSpanEnd(imageSpan)
        if (spanStart != start || spanEnd != end) return null

        val textLayout = layout ?: return null
        val currentText = text?.toString() ?: return null
        val scalarPos = PositionBridge.utf16ToScalar(spanStart, currentText)
        val docPos = if (editorId != 0L) {
            editorScalarToDoc(editorId.toULong(), scalarPos.toUInt()).toInt()
        } else {
            0
        }
        val line = textLayout.getLineForOffset(spanStart.coerceAtMost(maxOf(spannable.length - 1, 0)))
        val rect = resolvedImageRect(textLayout, imageSpan, spanStart, spanEnd)
        return SelectedImageGeometry(
            docPos = docPos,
            rect = rect
        )
    }

    fun resizeImageAtDocPos(docPos: Int, widthPx: Float, heightPx: Float) {
        if (editorId == 0L) return
        val density = resources.displayMetrics.density
        val widthDp = maxOf(48, (widthPx / density).roundToInt())
        val heightDp = maxOf(48, (heightPx / density).roundToInt())
        val updateJSON = editorResizeImageAtDocPos(
            editorId.toULong(),
            docPos.toUInt(),
            widthDp.toUInt(),
            heightDp.toUInt()
        )
        applyUpdateJSON(updateJSON)
    }

    private fun isSelectionInsideList(): Boolean {
        if (editorId == 0L) return false

        return try {
            val state = org.json.JSONObject(editorGetCurrentState(editorId.toULong()))
            val nodes = state.optJSONObject("activeState")?.optJSONObject("nodes")
            nodes?.optBoolean("bulletList", false) == true ||
                nodes?.optBoolean("orderedList", false) == true
        } catch (_: Exception) {
            false
        }
    }

    /**
     * Paste HTML content through Rust.
     */
    private fun pasteHTML(html: String) {
        val updateJSON = editorInsertContentHtml(editorId.toULong(), html)
        applyUpdateJSON(updateJSON)
    }

    /**
     * Paste plain text through Rust.
     */
    private fun pastePlainText(text: String) {
        val currentText = this.text?.toString() ?: ""
        val scalarPos = PositionBridge.utf16ToScalar(selectionStart, currentText)
        insertTextInRust(text, scalarPos)
    }

    // ── Applying Rust State ─────────────────────────────────────────────

    /**
     * Apply a full render update from Rust to the EditText.
     *
     * Parses the update JSON, converts render elements to [android.text.SpannableStringBuilder]
     * via [RenderBridge], and replaces the EditText's content.
     *
     * @param updateJSON The JSON string from editor_insert_text, etc.
     */
    fun applyUpdateJSON(updateJSON: String, notifyListener: Boolean = true) {
        val update = try {
            org.json.JSONObject(updateJSON)
        } catch (_: Exception) {
            return
        }

        val renderElements = update.optJSONArray("renderElements") ?: return

        val spannable = RenderBridge.buildSpannableFromArray(
            renderElements,
            baseFontSize,
            baseTextColor,
            theme,
            resources.displayMetrics.density,
            this
        )

        val previousScrollX = scrollX
        val previousScrollY = scrollY

        explicitSelectedImageRange = null
        isApplyingRustState = true
        setText(spannable)
        lastAuthorizedText = spannable.toString()
        isApplyingRustState = false

        // Apply the selection from the update.
        val selection = update.optJSONObject("selection")
        if (selection != null) {
            applySelectionFromJSON(selection)
        }

        if (notifyListener) {
            editorListener?.onEditorUpdate(updateJSON)
        }
        onSelectionOrContentMayChange?.invoke()
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            requestLayout()
        } else {
            preserveScrollPosition(previousScrollX, previousScrollY)
        }
    }

    /**
     * Apply a render JSON string (just render elements, no update wrapper).
     *
     * Used for initial content loading (set_html / set_json return render
     * elements directly, not wrapped in an EditorUpdate).
     *
     * @param renderJSON The JSON array string of render elements.
     */
    fun applyRenderJSON(renderJSON: String) {
        val spannable = RenderBridge.buildSpannable(
            renderJSON,
            baseFontSize,
            baseTextColor,
            theme,
            resources.displayMetrics.density,
            this
        )

        val previousScrollX = scrollX
        val previousScrollY = scrollY

        explicitSelectedImageRange = null
        isApplyingRustState = true
        setText(spannable)
        lastAuthorizedText = spannable.toString()
        isApplyingRustState = false
        onSelectionOrContentMayChange?.invoke()
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            requestLayout()
        } else {
            preserveScrollPosition(previousScrollX, previousScrollY)
        }
    }

    private fun handleImageTap(event: MotionEvent): Boolean {
        if (!imageResizingEnabled) {
            return false
        }
        if (event.actionMasked != MotionEvent.ACTION_DOWN && event.actionMasked != MotionEvent.ACTION_UP) {
            return false
        }
        val hit = imageSpanHitAt(event.x, event.y) ?: return false
        requestFocus()
        selectExplicitImageRange(hit.first, hit.second)
        if (event.actionMasked == MotionEvent.ACTION_UP) {
            performClick()
        }
        return true
    }

    private fun imageSpanHitAt(x: Float, y: Float): Pair<Int, Int>? {
        val spannable = text as? Spanned ?: return null
        imageSpanRangeNearTouchOffset(spannable, x, y)?.let { return it }
        val textLayout = layout ?: return null
        return imageSpanRectHit(spannable, textLayout, x, y)
    }

    private fun imageSpanRectHit(
        spannable: Spanned,
        textLayout: Layout,
        x: Float,
        y: Float
    ): Pair<Int, Int>? {
        val candidateSpans = spannable.getSpans(0, spannable.length, BlockImageSpan::class.java)
        for (span in candidateSpans) {
            val spanStart = spannable.getSpanStart(span)
            val spanEnd = spannable.getSpanEnd(span)
            if (spanStart < 0 || spanEnd <= spanStart) continue
            val rect = resolvedImageRect(textLayout, span, spanStart, spanEnd)
            if (rect.contains(x, y)) {
                return spanStart to spanEnd
            }
        }
        return null
    }

    override fun onFocusChanged(focused: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
        super.onFocusChanged(focused, direction, previouslyFocusedRect)
        if (!focused) {
            clearExplicitSelectedImageRange()
        }
    }

    private fun selectExplicitImageRange(start: Int, end: Int) {
        explicitSelectedImageRange = ImageSelectionRange(start, end)
        if (selectionStart == start && selectionEnd == end) {
            onSelectionOrContentMayChange?.invoke()
            return
        }
        setSelection(start, end)
    }

    private fun clearExplicitSelectedImageRange() {
        if (explicitSelectedImageRange == null) return
        explicitSelectedImageRange = null
        onSelectionOrContentMayChange?.invoke()
    }

    private fun resolvedSelectedImageRange(spannable: Spanned): ImageSelectionRange? {
        explicitSelectedImageRange?.let { explicit ->
            if (isExactImageSpanRange(spannable, explicit.start, explicit.end)) {
                return explicit
            }
            explicitSelectedImageRange = null
        }

        val start = selectionStart
        val end = selectionEnd
        if (!isExactImageSpanRange(spannable, start, end)) return null
        return ImageSelectionRange(start, end)
    }

    private fun isExactImageSpanRange(spannable: Spanned, start: Int, end: Int): Boolean {
        if (start < 0 || end != start + 1) return false
        val imageSpan = spannable
            .getSpans(start, end, BlockImageSpan::class.java)
            .firstOrNull() ?: return false
        return spannable.getSpanStart(imageSpan) == start && spannable.getSpanEnd(imageSpan) == end
    }

    private fun imageSpanRangeNearTouchOffset(
        spannable: Spanned,
        x: Float,
        y: Float
    ): Pair<Int, Int>? {
        val safeOffset = runCatching { getOffsetForPosition(x, y) }.getOrNull() ?: return null
        val nearbyOffsets = linkedSetOf(
            safeOffset,
            (safeOffset - 1).coerceAtLeast(0),
            (safeOffset + 1).coerceAtMost(spannable.length)
        )
        for (offset in nearbyOffsets) {
            val searchStart = (offset - 1).coerceAtLeast(0)
            val searchEnd = (offset + 1).coerceAtMost(spannable.length)
            val imageSpan = spannable
                .getSpans(searchStart, searchEnd, BlockImageSpan::class.java)
                .firstOrNull() ?: continue
            val spanStart = spannable.getSpanStart(imageSpan)
            val spanEnd = spannable.getSpanEnd(imageSpan)
            if (spanStart >= 0 && spanEnd > spanStart) {
                return spanStart to spanEnd
            }
        }
        return null
    }

    private fun resolvedImageRect(
        textLayout: Layout,
        imageSpan: BlockImageSpan,
        spanStart: Int,
        spanEnd: Int
    ): RectF {
        imageSpan.currentDrawRect()?.let { drawnRect ->
            return drawnRect
        }

        val safeOffset = spanStart.coerceAtMost(maxOf((text?.length ?: 0) - 1, 0))
        val line = textLayout.getLineForOffset(safeOffset)
        val startHorizontal = textLayout.getPrimaryHorizontal(spanStart)
        val endHorizontal = textLayout.getPrimaryHorizontal(spanEnd)
        val (widthPx, heightPx) = imageSpan.currentSizePx()
        val left = compoundPaddingLeft + minOf(startHorizontal, endHorizontal)
        val right = compoundPaddingLeft + maxOf(
            maxOf(startHorizontal, endHorizontal),
            minOf(startHorizontal, endHorizontal) + widthPx
        )
        val top = extendedPaddingTop + textLayout.getLineBottom(line) - heightPx
        return RectF(left, top.toFloat(), right, top + heightPx.toFloat())
    }

    /**
     * Apply a selection from a parsed JSON selection object.
     *
     * The selection JSON matches the format from `serialize_editor_update`:
     * ```json
     * {"type": "text", "anchor": 5, "head": 5}
     * {"type": "node", "pos": 10}
     * {"type": "all"}
     * ```
     *
     * anchor/head from Rust are **document positions** (include structural tokens).
     * We convert doc→scalar via [editorDocToScalar] before converting to UTF-16.
     */
    private fun applySelectionFromJSON(selection: org.json.JSONObject) {
        val type = selection.optString("type", "") ?: return

        isApplyingRustState = true
        try {
            val currentText = text?.toString() ?: ""
            when (type) {
                "text" -> {
                    val docAnchor = selection.optInt("anchor", 0)
                    val docHead = selection.optInt("head", 0)
                    // Convert doc positions to scalar offsets.
                    val scalarAnchor = editorDocToScalar(editorId.toULong(), docAnchor.toUInt()).toInt()
                    val scalarHead = editorDocToScalar(editorId.toULong(), docHead.toUInt()).toInt()
                    val startUtf16 = PositionBridge.scalarToUtf16(minOf(scalarAnchor, scalarHead), currentText)
                    val endUtf16 = PositionBridge.scalarToUtf16(maxOf(scalarAnchor, scalarHead), currentText)
                    val len = text?.length ?: 0
                    setSelection(
                        startUtf16.coerceIn(0, len),
                        endUtf16.coerceIn(0, len)
                    )
                }
                "node" -> {
                    val docPos = selection.optInt("pos", 0)
                    // Convert doc position to scalar offset.
                    val scalarPos = editorDocToScalar(editorId.toULong(), docPos.toUInt()).toInt()
                    val startUtf16 = PositionBridge.scalarToUtf16(scalarPos, currentText)
                    val len = text?.length ?: 0
                    val clamped = startUtf16.coerceIn(0, len)
                    // Select one character (the void node placeholder).
                    val endClamped = (clamped + 1).coerceAtMost(len)
                    setSelection(clamped, endClamped)
                }
                "all" -> {
                    selectAll()
                }
            }
        } finally {
            isApplyingRustState = false
        }
    }

    // ── Reconciliation ─────────────────────────────────────────────────

    /**
     * [TextWatcher] that detects when the EditText's text diverges from the
     * last Rust-authorized content (e.g., due to IME autocorrect, accessibility
     * services, or other Android framework mutations that bypass our
     * [EditorInputConnection]).
     *
     * When divergence is detected, Rust's current state is re-fetched and
     * re-applied — "Rust wins" — to maintain the invariant that the Rust
     * editor-core is the single source of truth for document content.
     */
    private inner class ReconciliationWatcher : TextWatcher {

        override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {
            // No-op: we only need afterTextChanged.
        }

        override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {
            // No-op: we only need afterTextChanged.
        }

        override fun afterTextChanged(s: Editable?) {
            if (isApplyingRustState) return
            if (editorId == 0L) return

            val currentText = s?.toString() ?: ""
            if (currentText == lastAuthorizedText) return

            // Text has diverged from Rust's authorized state.
            reconciliationCount++
            Log.w(
                LOG_TAG,
                "reconciliation: EditText diverged from Rust state" +
                        " (count=$reconciliationCount," +
                        " editText=${currentText.length} chars," +
                        " authorized=${lastAuthorizedText.length} chars)"
            )

            // Re-fetch Rust's current state and re-apply ("Rust wins").
            val stateJSON = editorGetCurrentState(editorId.toULong())
            applyUpdateJSON(stateJSON)
        }
    }

    companion object {
        private const val EMPTY_BLOCK_PLACEHOLDER = '\u200B'
        private const val LOG_TAG = "NativeEditor"
    }
}
