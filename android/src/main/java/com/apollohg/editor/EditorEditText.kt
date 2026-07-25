package com.apollohg.editor

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Typeface
import android.graphics.Rect
import android.graphics.RectF
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.Settings
import android.text.Annotation
import android.text.Editable
import android.text.InputType
import android.text.Layout
import android.text.Selection
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.text.TextWatcher
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
import android.text.style.UnderlineSpan
import android.util.AttributeSet
import android.util.Log
import android.util.TypedValue
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.appcompat.widget.AppCompatEditText
import kotlin.math.roundToInt

/**
 * Custom [AppCompatEditText] subclass that intercepts all text input and routes it
 * through the Rust editor-core engine via the [EditorV2Driver] interface.
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
 * For CJK input methods, swipe keyboards, and some autocorrect flows, composing
 * text is rendered transiently by the base [InputConnection]. The final commit
 * is routed through Rust against the original authorized selection.
 *
 * ## Thread Safety
 *
 * All EditText methods are called on the main thread. The [EditorV2Driver]
 * calls (`insertText`, `deleteScalarRange`, etc.) are synchronous and
 * fast enough for main-thread use.
 */
class EditorEditText @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = android.R.attr.editTextStyle
) : AppCompatEditText(context, attrs, defStyleAttr) {
    data class ApplyUpdateTrace(
        val attemptedPatch: Boolean,
        val usedPatch: Boolean,
        val skippedRender: Boolean,
        val parseNanos: Long,
        val resolveRenderBlocksNanos: Long,
        val patchEligibilityNanos: Long,
        val buildRenderNanos: Long,
        val applyRenderNanos: Long,
        val selectionNanos: Long,
        val postApplyNanos: Long,
        val totalNanos: Long
    )

    internal data class ImeInitialSurroundingText(
        val text: String,
        val selectionStart: Int,
        val selectionEnd: Int,
        val originalSelectionStart: Int,
        val originalSelectionEnd: Int,
        val removedPlaceholderCount: Int
    )

    data class SelectedImageGeometry(
        val docPos: Int,
        val rect: RectF
    )

    data class MentionHit(
        val docPos: Int,
        val label: String
    )

    data class CommandPreparation(
        val ready: Boolean,
        val updateJSON: String?
    )

    data class ExternalEditorUpdatePreparation(
        val ready: Boolean,
        /** A preflight mutation's already-adopted render snapshot, if one occurred. */
        val adoptedUpdateJSON: String?
    )

    private data class HardwareKeyEventSignature(
        val keyCode: Int,
        val downTime: Long,
        val repeatCount: Int
    )

    data class LinkHit(
        val href: String,
        val text: String
    )

    internal data class AccessibleAnnotation(
        val target: AccessibleAnnotationTarget,
        val label: String,
        val role: String,
        val bounds: Rect,
        val annotation: Annotation,
        val start: Int,
        val end: Int
    )

    internal data class InteractiveAnnotationHit(
        val target: AccessibleAnnotationTarget,
        val annotation: Annotation,
        val start: Int,
        val end: Int
    )

    internal sealed interface AccessibleAnnotationTarget {
        data class Mention(val docPos: Int, val label: String) : AccessibleAnnotationTarget
        data class Link(val href: String, val text: String) : AccessibleAnnotationTarget
    }

    private data class ParsedRenderPatch(
        val startIndex: Int,
        val deleteCount: Int,
        val renderBlocks: org.json.JSONArray
    )

    private data class RenderReplaceRange(
        val start: Int,
        val endExclusive: Int
    )

    private data class PatchApplyTrace(
        val applied: Boolean,
        val eligibilityNanos: Long,
        val buildRenderNanos: Long,
        val applyRenderNanos: Long
    )

    private data class ImageSelectionRange(
        val start: Int,
        val end: Int
    )

    private data class ImageSpanHit(
        val span: BlockImageSpan,
        val start: Int,
        val end: Int
    )

    private data class ImageGesture(
        val target: BlockImageSpan,
        val pointerId: Int,
        val downX: Float,
        val downY: Float
    )

    private data class NativeTextMutation(
        val scalarFrom: Int,
        val scalarTo: Int,
        val replacementText: String,
        val resultingText: String,
        val replacementStartUtf16: Int,
        val replacementEndUtf16: Int,
        val selectionScalarAnchor: Int?,
        val selectionScalarHead: Int?
    )

    private data class NativeTextMutationAfterBlurWindow(
        val editorId: Long,
        val authorizedTextRevision: Long,
        val deadlineMs: Long,
        var didAdoptMutation: Boolean = false
    )

    private data class NativeTextMutationAdoptionSuppression(
        val editorId: Long,
        val authorizedTextRevision: Long
    )

    private interface TransientComposingTextStyleSpan

    private class TransientComposingSizeSpan(sizePx: Int) :
        AbsoluteSizeSpan(sizePx, false),
        TransientComposingTextStyleSpan

    private class TransientComposingColorSpan(color: Int) :
        ForegroundColorSpan(color),
        TransientComposingTextStyleSpan

    private class TransientComposingTypefaceSpan(family: String) :
        TypefaceSpan(family),
        TransientComposingTextStyleSpan

    private class TransientComposingStyleSpan(style: Int) :
        StyleSpan(style),
        TransientComposingTextStyleSpan

    /**
     * Listener interface for editor events, parallel to iOS's EditorTextViewDelegate.
     */
    interface EditorListener {
        /** Called when the editor's selection changes (anchor and head as scalar offsets). */
        fun onSelectionChanged(anchor: Int, head: Int)

        /** Called when the editor content is updated after a Rust operation. */
        fun onEditorUpdate(updateJSON: String)
    }

    /** The editor session public ID (used to look up the [EditorV2Driver] adapter). */
    var editorId: Long = 0

    /**
     * Controls whether user input is accepted.
     *
     * When false, all user-input mutation entry points (typing, deletion,
     * paste, composition) are blocked. Unlike [isEnabled], this preserves
     * focus, text selection, and copy capability.
     */
    var isEditable: Boolean = true
        set(value) {
            if (field == value) return
            if (!value) {
                discardTransientNativeInputForReadOnly()
            }
            field = value
            if (value) {
                restartInputForEditorIfFocused("editable")
            }
        }

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
            if (field == value) return
            field = value
            requestLayout()
            invalidate()
        }

    var heightBehavior: EditorHeightBehavior = EditorHeightBehavior.FIXED
        private set

    // Cached once per view instance so paired-tap tracking doesn't re-query
    // ViewConfiguration on every MotionEvent.
    private val touchSlopPx: Float by lazy(LazyThreadSafetyMode.NONE) {
        android.view.ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    }
    private var imageResizingEnabled = true
    internal var imageLoadingPolicy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
        private set
    private var imageLoadGeneration: Long = 0L
    private val imageLoadHandles = mutableListOf<RenderImageLoader.LoadHandle>()
    private var nativeAutoCapitalize = DEFAULT_AUTO_CAPITALIZE
    private var nativeAutoCorrect = DEFAULT_AUTO_CORRECT
    private var nativeKeyboardType = DEFAULT_KEYBOARD_TYPE

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

    private var lastHandledHardwareKeySignature: HardwareKeyEventSignature? = null
    private var recentHandledHardwareKeyDownSignature: HardwareKeyEventSignature? = null
    private var recentHandledHardwareKeyDownUptimeMs: Long = 0L
    private var activeInputConnection: EditorInputConnection? = null
    private var inputConnectionGeneration: Long = 0L
    private var composingText: String? = null
    private var composingReplacementStartUtf16: Int? = null
    private var composingReplacementEndUtf16: Int? = null
    private var composingReplacementAuthorizedTextRevision: Long? = null
    private var didInvalidateCompositionReplacementRange = false
    private var nativeTextMutationAfterBlurWindow: NativeTextMutationAfterBlurWindow? = null
    private var nativeTextMutationAdoptionSuppression: NativeTextMutationAdoptionSuppression? = null
    private var lastAuthorizedTextRevision: Long = 0L
    private var lastAuthorizedRenderedText: CharSequence? = null
    private var explicitSelectedImageRange: ImageSelectionRange? = null
    private var pendingImageGesture: ImageGesture? = null
    private var lastRenderAppliedPatchForTesting: Boolean = false
    internal var captureApplyUpdateTraceForTesting: Boolean = false
    private var lastApplyUpdateTraceForTesting: ApplyUpdateTrace? = null
    private val imeTraceForTesting = java.util.ArrayDeque<String>()
    private var imeTraceSequence: Long = 0L
    private var lastImeTraceUptimeMs: Long = 0L
    private var currentRenderBlocksJson: org.json.JSONArray? = null
    private var restartImageLoadsOnAttach = false
    private var renderAppearanceRevision: Long = 1L
    private var lastAppliedRenderAppearanceRevision: Long = 0L
    private var pendingOptimisticRenderText: String? = null
    private var deferredRustUpdateApplicationDepth: Int = 0
    private var deferredRustUpdateJSON: String? = null
    private var deferredRustUpdateLineBoundaryRefreshSource: String? = null
    private var deferredRustUpdateGeneration: Long = 0L
    private var externalUpdatePreparationCaptureDepth: Int = 0
    private var capturedExternalUpdatePreparationJSON: String? = null
    private var lineBoundaryInputRefreshGeneration: Long = 0L
    private var restartInputSelectionUpdateGeneration: Long = 0L
    internal var onDeleteRangeInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onDeleteBackwardAtSelectionScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onToggleTaskItemCheckedAtSelectionScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onInsertTextInRustForTesting: ((String, Int) -> Unit)? = null
    internal var onReplaceTextInRustForTesting: ((Int, Int, String) -> Unit)? = null
    internal var onSetSelectionScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onDeleteAndSplitScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onInsertContentHtmlInRustForTesting: ((String) -> Unit)? = null
    internal var onInsertContentJsonAtSelectionScalarForTesting: ((Int, Int, String) -> Unit)? = null
    internal var blockExternalEditorUpdatePreparationForTesting = false
    internal var blockExternalEditorCommandPreparationForTesting = false
    internal var throwOnNextApplyUpdateForTesting: Throwable? = null

    /**
     * The v2 driver for this editor session: when an adapter is attached,
     * every engine mutation below routes through it (typed v2
     * transactions/results). It is the ONLY engine path — a null driver
     * means the view has no engine traffic.
     */
    internal var v2Driver: EditorV2Driver? = null

    fun lastRenderAppliedPatch(): Boolean = lastRenderAppliedPatchForTesting
    fun lastApplyUpdateTrace(): ApplyUpdateTrace? = lastApplyUpdateTraceForTesting
    internal fun hasDeferredRustUpdateApplicationForTesting(): Boolean = deferredRustUpdateJSON != null
    internal fun inputConnectionGenerationForTesting(): Long = inputConnectionGeneration

    internal fun applyRustUpdateJSONForTesting(updateJSON: String) {
        applyRustUpdateJSON(updateJSON)
    }

    internal fun recordImeTraceForTesting(event: String, details: String = "") {
        if (imeTraceForTesting.size >= IME_TRACE_LIMIT_FOR_TESTING) {
            imeTraceForTesting.removeFirst()
        }
        imeTraceForTesting.addLast(
            if (details.isEmpty()) event else "$event:$details"
        )
        if (Log.isLoggable(IME_TRACE_LOG_TAG, Log.VERBOSE)) {
            val now = SystemClock.uptimeMillis()
            val deltaMs = if (lastImeTraceUptimeMs == 0L) 0L else now - lastImeTraceUptimeMs
            lastImeTraceUptimeMs = now
            imeTraceSequence += 1L
            val textLength = text?.length ?: -1
            val selection = "${selectionStart}..${selectionEnd}"
            val composingRange = "${composingReplacementStartUtf16 ?: -1}.." +
                "${composingReplacementEndUtf16 ?: -1}"
            val composingRevisionMatches =
                composingReplacementAuthorizedTextRevision == lastAuthorizedTextRevision
            val message = buildString {
                append("#").append(imeTraceSequence)
                append(" +").append(deltaMs).append("ms ")
                append(event)
                if (details.isNotEmpty()) {
                    append(" ").append(details)
                }
                append(" editor=").append(editorId)
                append(" gen=").append(inputConnectionGeneration)
                append(" activeIc=").append(activeInputConnection != null)
                append(" focus=").append(hasFocus())
                append(" applying=").append(isApplyingRustState)
                append(" editable=").append(isEditable)
                append(" textLen=").append(textLength)
                append(" authLen=").append(lastAuthorizedText.length)
                append(" sel=").append(selection)
                append(" composingTextLen=").append(composingText?.length ?: -1)
                append(" composingRange=").append(composingRange)
                append(" composingRevOk=").append(composingRevisionMatches)
                append(" invalidComp=").append(didInvalidateCompositionReplacementRange)
                append(" deferredRustUpdate=").append(deferredRustUpdateJSON != null)
                append(" scroll=").append(scrollX).append(",").append(scrollY)
            }
            Log.v(IME_TRACE_LOG_TAG, message)
        }
    }

    internal fun clearImeTraceForTesting() {
        imeTraceForTesting.clear()
        imeTraceSequence = 0L
        lastImeTraceUptimeMs = 0L
    }

    internal fun imeTraceSnapshotForTesting(): List<String> =
        imeTraceForTesting.toList()

    private fun nanosToMicros(nanos: Long): Long = nanos / 1_000L

    init {
        // Configure for rich text editing.
        inputType = resolvedInputType()

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

        // Suppress the platform caret and draw our own. Android's Editor anchors
        // the native caret to getLineBottom(line), which a ParagraphSpacerSpan
        // inflates — stretching the caret into the inter-block gap. Our caret is
        // clipped to the glyph height via [CaretGeometry]. See [drawCustomCaret].
        isCursorVisible = false

        updateEffectivePadding()
    }

    // ── Custom caret ────────────────────────────────────────────────────────

    private var caretBlinkVisible = true
    private val caretPaint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG)
    private val caretWidthPx: Float by lazy { maxOf(MIN_CARET_WIDTH_PX, resources.displayMetrics.density) }
    private val caretColor: Int by lazy { resolveCaretColor() }
    private val caretBlinkRunnable = object : Runnable {
        override fun run() {
            caretBlinkVisible = !caretBlinkVisible
            invalidate()
            postDelayed(this, CARET_BLINK_INTERVAL_MS)
        }
    }

    /**
     * Reset the caret to solid-on and (re)schedule the blink. Called whenever the
     * caret could appear or move (focus, window focus, selection change) so it
     * behaves like the platform caret: solid immediately after a move, then blinks.
     */
    private fun restartCaretBlink() {
        removeCallbacks(caretBlinkRunnable)
        caretBlinkVisible = true
        if (CaretGeometry.shouldRender(isFocused, hasWindowFocus(), selectionStart, selectionEnd)) {
            postDelayed(caretBlinkRunnable, CARET_BLINK_INTERVAL_MS)
        }
        invalidate()
    }

    private fun stopCaretBlink() {
        removeCallbacks(caretBlinkRunnable)
        caretBlinkVisible = false
        invalidate()
    }

    private fun drawCustomCaret(canvas: android.graphics.Canvas) {
        if (!caretBlinkVisible) return
        if (!CaretGeometry.shouldRender(isFocused, hasWindowFocus(), selectionStart, selectionEnd)) return
        val rect = customCaretDrawRect() ?: return
        caretPaint.color = caretColor
        canvas.drawRect(rect.left, rect.top, rect.right, rect.bottom, caretPaint)
    }

    /**
     * The caret rectangle in canvas (content) coordinates — clipped to the glyph
     * height via [CaretGeometry]. No scroll offset is applied: at onDraw time the
     * framework canvas is already in scrolled content space, exactly like the text
     * Layout it paints. (This differs from [caretRect], which reports view-relative
     * coordinates to JS.)
     */
    internal fun customCaretDrawRect(): RectF? {
        val textLayout = layout ?: return null
        val offset = selectionEnd.coerceIn(0, textLayout.text.length)
        val bounds = CaretGeometry.verticalBounds(textLayout, offset, paint)
        val left = totalPaddingLeft + textLayout.getPrimaryHorizontal(offset)
        return RectF(left, totalPaddingTop + bounds.top, left + caretWidthPx, totalPaddingTop + bounds.bottom)
    }

    /**
     * The native caret is tinted by the theme's `colorControlActivated`; resolve
     * the same value so the replacement keeps the platform appearance, falling
     * back to the text color when the attribute is not a color.
     */
    private fun resolveCaretColor(): Int {
        val resolved = TypedValue()
        val found = context.theme.resolveAttribute(
            android.R.attr.colorControlActivated,
            resolved,
            true
        )
        val isColor = resolved.type >= TypedValue.TYPE_FIRST_COLOR_INT &&
            resolved.type <= TypedValue.TYPE_LAST_COLOR_INT
        return if (found && isColor) resolved.data else currentTextColor
    }

    fun setAutoCapitalize(autoCapitalize: String?) {
        val next = when (autoCapitalize) {
            "none",
            "sentences",
            "words",
            "characters" -> autoCapitalize
            else -> DEFAULT_AUTO_CAPITALIZE
        }
        if (nativeAutoCapitalize == next) return
        nativeAutoCapitalize = next
        applyInputTraits()
    }

    fun setAutoCorrect(autoCorrect: Boolean?) {
        val next = autoCorrect ?: DEFAULT_AUTO_CORRECT
        if (nativeAutoCorrect == next) return
        nativeAutoCorrect = next
        applyInputTraits()
    }

    fun setKeyboardType(keyboardType: String?) {
        val next = when (keyboardType) {
            "default",
            "email-address",
            "numeric",
            "phone-pad",
            "ascii-capable",
            "numbers-and-punctuation",
            "url",
            "number-pad",
            "name-phone-pad",
            "decimal-pad",
            "twitter",
            "web-search",
            "visible-password",
            "ascii-capable-number-pad" -> keyboardType
            else -> DEFAULT_KEYBOARD_TYPE
        }
        if (nativeKeyboardType == next) return
        nativeKeyboardType = next
        applyInputTraits()
    }

    private fun applyInputTraits() {
        val nextInputType = resolvedInputType()
        if (inputType == nextInputType) return

        val currentStart = selectionStart
        val currentEnd = selectionEnd
        val authorizedSelection = authorizedSelectionForTransientInputRestore(
            currentStart,
            currentEnd
        )
        discardTransientInputAndRestoreAuthorizedTextForEditor()
        setRawInputType(nextInputType)

        val editable = text
        if (editable != null && authorizedSelection != null) {
            setSelection(
                authorizedSelection.first.coerceIn(0, editable.length),
                authorizedSelection.second.coerceIn(0, editable.length)
            )
        }

        if (hasFocus()) {
            restartInputForEditor()
        }
    }

    private fun resolvedInputType(): Int {
        var nextInputType = when (nativeKeyboardType) {
            "email-address" -> InputType.TYPE_CLASS_TEXT or
                InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS
            "url" -> InputType.TYPE_CLASS_TEXT or
                InputType.TYPE_TEXT_VARIATION_URI
            "phone-pad" -> InputType.TYPE_CLASS_PHONE
            "number-pad" -> InputType.TYPE_CLASS_NUMBER
            "decimal-pad" -> InputType.TYPE_CLASS_NUMBER or
                InputType.TYPE_NUMBER_FLAG_DECIMAL
            "numeric" -> InputType.TYPE_CLASS_NUMBER or
                InputType.TYPE_NUMBER_FLAG_DECIMAL or
                InputType.TYPE_NUMBER_FLAG_SIGNED
            "visible-password" -> InputType.TYPE_CLASS_TEXT or
                InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
            else -> InputType.TYPE_CLASS_TEXT
        }

        if ((nextInputType and InputType.TYPE_MASK_CLASS) == InputType.TYPE_CLASS_TEXT) {
            nextInputType = nextInputType or InputType.TYPE_TEXT_FLAG_MULTI_LINE
            nextInputType = nextInputType or when (nativeAutoCapitalize) {
                "none" -> 0
                "words" -> InputType.TYPE_TEXT_FLAG_CAP_WORDS
                "characters" -> InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS
                else -> InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
            }
            nextInputType = nextInputType or if (nativeAutoCorrect) {
                InputType.TYPE_TEXT_FLAG_AUTO_CORRECT
            } else {
                InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            }
        }

        return nextInputType
    }

    // ── InputConnection Override ────────────────────────────────────────

    /**
     * Create a custom [EditorInputConnection] that intercepts all input
     * from the soft keyboard.
     */
    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        val baseConnection = super.onCreateInputConnection(outAttrs) ?: return null
        val originalInitialCapsMode = outAttrs.initialCapsMode
        outAttrs.initialCapsMode = cursorCapsModeForEditor(
            reqModes = outAttrs.inputType,
            baseCapsMode = outAttrs.initialCapsMode
        )
        val initialSurroundingText = applyInitialSurroundingTextForIme(outAttrs)
        val generation = nextInputConnectionGenerationForEditor()
        NativeEditorViewRegistry.registerInputView(editorId, this)
        recordImeTraceForTesting(
            "createInputConnection",
            "boundEditor=$editorId boundGen=$generation inputType=$inputType initialCaps=$originalInitialCapsMode->${outAttrs.initialCapsMode} " +
                "imeContextPlaceholdersRemoved=${initialSurroundingText?.removedPlaceholderCount ?: 0} " +
                "imeContextSel=${initialSurroundingText?.selectionStart ?: outAttrs.initialSelStart}..${initialSurroundingText?.selectionEnd ?: outAttrs.initialSelEnd} " +
                "imeContextRawSel=${initialSurroundingText?.originalSelectionStart ?: selectionStart}..${initialSurroundingText?.originalSelectionEnd ?: selectionEnd} " +
                "imeContextBeforeTail=\"${initialSurroundingText?.textBeforeSelectionTailForImeLog() ?: ""}\""
        )
        return EditorInputConnection(this, baseConnection, editorId, generation).also {
            activeInputConnection = it
        }
    }

    private fun applyInitialSurroundingTextForIme(outAttrs: EditorInfo): ImeInitialSurroundingText? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        val initialText = initialSurroundingTextForImeForEditor() ?: return null

        outAttrs.initialSelStart = initialText.selectionStart
        outAttrs.initialSelEnd = initialText.selectionEnd
        outAttrs.setInitialSurroundingText(initialText.text)
        return initialText
    }

    private fun ImeInitialSurroundingText.textBeforeSelectionTailForImeLog(limit: Int = 24): String {
        val end = selectionStart.coerceIn(0, text.length)
        val start = maxOf(0, end - limit)
        return text.substring(start, end).toImeTraceSnippet()
    }

    private fun String.toImeTraceSnippet(): String {
        val builder = StringBuilder(length)
        forEach { ch ->
            when (ch) {
                '\n' -> builder.append("\\n")
                '\r' -> builder.append("\\r")
                '\t' -> builder.append("\\t")
                '\\' -> builder.append("\\\\")
                '"' -> builder.append("\\\"")
                else -> {
                    if (ch.code < 0x20 || ch == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]) {
                        builder.append("\\u")
                        builder.append(ch.code.toString(16).padStart(4, '0'))
                    } else {
                        builder.append(ch)
                    }
                }
            }
        }
        return builder.toString()
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (!isEditable && isReadOnlyTextMutationKeyEvent(event)) {
            return true
        }
        if (handleCompositionKeyEvent(event) { super.dispatchKeyEvent(event) }) {
            return true
        }
        if (handleHardwareKeyEvent(event)) {
            return true
        }
        if (handlePrintableHardwareKeyEvent(event) { super.dispatchKeyEvent(event) }) {
            return true
        }
        return super.dispatchKeyEvent(event)
    }

    internal fun handleCompositionKeyEvent(event: KeyEvent, applyBaseEvent: () -> Boolean): Boolean {
        val inputConnection = activeInputConnection ?: return false
        if (!inputConnection.hasPendingComposition()) return false
        if (!isCompositionKeyCode(event.keyCode)) return false
        if (event.action == KeyEvent.ACTION_DOWN) {
            val signature = hardwareKeyEventSignature(event)
            if (
                lastHandledHardwareKeySignature == signature ||
                didRecentlyHandleHardwareKeyDown(signature)
            ) {
                return true
            }
            markHandledHardwareKeyDown(signature)
            runWithTransientInputMutationGuard {
                when (event.keyCode) {
                    KeyEvent.KEYCODE_DEL,
                    KeyEvent.KEYCODE_FORWARD_DEL -> inputConnection.deleteTransientTextForHardwareKeyEvent(event)
                    else -> applyBaseEvent()
                }
            }
            inputConnection.refreshComposingTextFromEditableForEditor()
            return true
        }
        if (event.action == KeyEvent.ACTION_UP) {
            if (lastHandledHardwareKeySignature?.let {
                    it.keyCode == event.keyCode && it.downTime == event.downTime
                } == true) {
                lastHandledHardwareKeySignature = null
            }
            return true
        }
        return false
    }

    private fun isCompositionKeyCode(keyCode: Int): Boolean =
        when (keyCode) {
            KeyEvent.KEYCODE_DEL,
            KeyEvent.KEYCODE_FORWARD_DEL,
            KeyEvent.KEYCODE_ENTER,
            KeyEvent.KEYCODE_NUMPAD_ENTER,
            KeyEvent.KEYCODE_TAB -> true
            else -> false
        }

    override fun onDraw(canvas: android.graphics.Canvas) {
        super.onDraw(canvas)
        drawCustomCaret(canvas)

        val placeholderLayout =
            buildPlaceholderLayout(width - compoundPaddingLeft - compoundPaddingRight) ?: return

        val previousColor = paint.color
        val saveCount = canvas.save()
        canvas.translate(compoundPaddingLeft.toFloat(), extendedPaddingTop.toFloat())
        placeholderLayout.draw(canvas)
        canvas.restoreToCount(saveCount)
        paint.color = previousColor
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)

        val placeholderHeight = resolvePlaceholderHeightForMeasuredWidth(measuredWidth) ?: return
        val desiredHeight = maxOf(measuredHeight, placeholderHeight)
        val resolvedHeight = when (MeasureSpec.getMode(heightMeasureSpec)) {
            MeasureSpec.EXACTLY -> measuredHeight
            MeasureSpec.AT_MOST -> desiredHeight.coerceAtMost(MeasureSpec.getSize(heightMeasureSpec))
            else -> desiredHeight
        }

        if (resolvedHeight != measuredHeight) {
            setMeasuredDimension(measuredWidth, resolvedHeight)
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN && imageSpanHitAt(event.x, event.y) == null) {
            clearExplicitSelectedImageRange()
        }
        if (handleTaskListMarkerTap(event)) {
            return true
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

    /**
     * The core's `documentIsEmpty` from the most recent editor update, or null
     * when the current render arrived without one.
     */
    private var coreReportedDocumentIsEmpty: Boolean? = null

    /**
     * Whether the document holds nothing the user authored.
     *
     * The core answers this and the answer arrives on every editor update, so
     * it is used verbatim. Deriving it here cannot work: an empty list item
     * contributes no characters, so scanning the rendered content reports it as
     * empty and leaves the placeholder sitting on top of a visible bullet.
     *
     * The character scan below is only the fallback for renders that arrive
     * without an editor update.
     */
    private fun isRenderedContentEmpty(content: CharSequence? = text): Boolean {
        coreReportedDocumentIsEmpty?.let { return it }

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

    /** Adopt the core's authoritative empty state from an editor update. */
    fun setCoreReportedDocumentIsEmpty(isEmpty: Boolean?) {
        if (coreReportedDocumentIsEmpty == isEmpty) return
        coreReportedDocumentIsEmpty = isEmpty
        invalidate()
    }

    private fun shouldDisplayPlaceholder(): Boolean {
        return placeholderText.isNotEmpty() && isRenderedContentEmpty()
    }

    fun shouldDisplayPlaceholderForTesting(): Boolean = shouldDisplayPlaceholder()

    private fun buildPlaceholderLayout(availableWidth: Int): StaticLayout? {
        if (!shouldDisplayPlaceholder()) return null
        if (availableWidth <= 0) return null

        val placeholderPaint = resolvedPlaceholderPaint()
        return StaticLayout.Builder
            .obtain(
                placeholderText,
                0,
                placeholderText.length,
                placeholderPaint,
                availableWidth
            )
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(includeFontPadding)
            .build()
    }

    private fun resolvedPlaceholderPaint(): TextPaint {
        val textStyle = theme?.effectiveTextStyle("paragraph")
        val resolvedTextSize = textStyle?.fontSize?.times(resources.displayMetrics.density) ?: baseFontSize
        val resolvedTypeface = resolvePlaceholderTypeface(textStyle)

        return TextPaint(paint).apply {
            color = theme?.placeholderColor ?: currentHintTextColor
            textSize = resolvedTextSize
            typeface = resolvedTypeface
        }
    }

    private fun resolvePlaceholderTypeface(textStyle: EditorTextStyle?): Typeface {
        val baseTypeface = typeface ?: Typeface.DEFAULT
        val requestedStyle = textStyle?.typefaceStyle() ?: Typeface.NORMAL
        val family = textStyle?.fontFamily?.takeIf { it.isNotBlank() }

        return when {
            family != null -> Typeface.create(family, requestedStyle)
            requestedStyle != Typeface.NORMAL -> Typeface.create(baseTypeface, requestedStyle)
            else -> baseTypeface
        }
    }

    private fun resolvePlaceholderHeightForMeasuredWidth(widthPx: Int): Int? {
        val availableWidth = (widthPx - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(0)
        return resolvePlaceholderHeightForAvailableWidth(availableWidth)
    }

    private fun resolvePlaceholderHeightForAvailableWidth(availableWidth: Int): Int? {
        val placeholderLayout = buildPlaceholderLayout(availableWidth) ?: return null
        val placeholderHeight = placeholderLayout.height.takeIf { it > 0 } ?: lineHeight
        return placeholderHeight + compoundPaddingTop + compoundPaddingBottom
    }

    // ── Editor Binding ──────────────────────────────────────────────────

    /**
     * Bind this EditText to a Rust editor instance and optionally apply initial content.
     *
     * @param id The editor session public ID.
     * @param initialHTML Optional HTML to set as initial content.
     */
    fun bindEditor(id: Long, initialHTML: String? = null, notifyListener: Boolean = true) {
        if (id != 0L && NativeEditorViewRegistry.isDestroyed(id)) {
            discardTransientNativeInputForEditorRebind()
            editorId = 0L
            return
        }
        if (editorId != id) {
            discardTransientNativeInputForEditorRebind()
        }
        editorId = id
        v2Driver = EditorV2Registry.adapterForViewToken(id)
        val driver = v2Driver
        if (driver != null) {
            if (!initialHTML.isNullOrEmpty()) {
                driver.setContentHtml(initialHTML)?.let { applyUpdateJSON(it, notifyListener = false) }
            } else {
                driver.currentStateJson()?.let { applyUpdateJSON(it, notifyListener = notifyListener) }
            }
            return
        }
    }

    /**
     * Unbind from the current editor instance.
     */
    fun unbindEditor() {
        if (editorId != 0L) {
            discardTransientNativeInputForEditorRebind()
        }
        editorId = 0
        v2Driver = null
    }

    internal fun handleEditorDestroyedFromRegistry(destroyedEditorId: Long) {
        if (editorId != destroyedEditorId) return
        unbindEditor()
    }

    fun setBaseStyle(fontSizePx: Float, textColor: Int, backgroundColor: Int) {
        if (baseFontSize != fontSizePx || baseTextColor != textColor) {
            renderAppearanceRevision += 1L
        }
        baseFontSize = fontSizePx
        baseTextColor = textColor
        baseBackgroundColor = backgroundColor
        setTextSize(TypedValue.COMPLEX_UNIT_PX, fontSizePx)
        setTextColor(textColor)
        setBackgroundColor(theme?.backgroundColor ?: backgroundColor)
    }

    fun applyTheme(theme: EditorTheme?) {
        this.theme = theme
        renderAppearanceRevision += 1L
        setBackgroundColor(theme?.backgroundColor ?: baseBackgroundColor)
        applyContentInsets(theme?.contentInsets)
        if (hasLiveEditor()) {
            val previousScrollX = scrollX
            val previousScrollY = scrollY
            val stateJSON = v2Driver?.currentStateJson() ?: return
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

    fun setImageLoadingPolicyJson(policyJson: String?) {
        val nextPolicy = ImageLoadingPolicy.fromJson(policyJson)
        if (nextPolicy == imageLoadingPolicy) return
        imageLoadingPolicy = nextPolicy
        if (!rebuildLatestRenderForImages()) cancelPendingImageLoads()
    }

    internal fun currentImageLoadGeneration(): Long = imageLoadGeneration

    internal fun registerImageLoad(handle: RenderImageLoader.LoadHandle) {
        synchronized(imageLoadHandles) {
            imageLoadHandles += handle
        }
        handle.onFinished {
            synchronized(imageLoadHandles) {
                imageLoadHandles.remove(handle)
            }
        }
    }

    internal fun activeImageLoadHandleCountForTesting(): Int = synchronized(imageLoadHandles) {
        imageLoadHandles.size
    }

    private fun cancelPendingImageLoads() {
        imageLoadGeneration += 1L
        val handles = synchronized(imageLoadHandles) {
            imageLoadHandles.toList().also { imageLoadHandles.clear() }
        }
        handles.forEach { it.cancel() }
    }

    private fun rebuildLatestRenderForImages(): Boolean {
        val currentContent = text as? Spanned ?: return false
        val imageSpans = currentContent.getSpans(
            0,
            currentContent.length,
            BlockImageSpan::class.java
        )
        if (imageSpans.isEmpty()) return false

        val previousSelectionStart = selectionStart
        val previousSelectionEnd = selectionEnd
        val previousScrollX = scrollX
        val previousScrollY = scrollY
        cancelPendingImageLoads()
        val spannable = SpannableStringBuilder(currentContent)
        imageSpans.forEach { span ->
            val start = currentContent.getSpanStart(span)
            val end = currentContent.getSpanEnd(span)
            val flags = currentContent.getSpanFlags(span)
            spannable.removeSpan(span)
            if (start >= 0 && end > start) {
                spannable.setSpan(span.reloadedFor(this), start, end, flags)
            }
        }
        applyRenderedSpannable(spannable, usedPatch = false)
        if (previousSelectionStart >= 0 && previousSelectionEnd >= 0) {
            val length = text?.length ?: 0
            setSelection(
                previousSelectionStart.coerceIn(0, length),
                previousSelectionEnd.coerceIn(0, length)
            )
        }
        scrollTo(previousScrollX, previousScrollY)
        post { scrollTo(previousScrollX, previousScrollY) }
        requestLayout()
        invalidate()
        onSelectionOrContentMayChange?.invoke()
        return true
    }

    private fun hasRenderedImageSpans(): Boolean {
        val content = text as? Spanned ?: return false
        return content.getSpans(0, content.length, BlockImageSpan::class.java).isNotEmpty()
    }

    fun resolveAutoGrowHeight(): Int {
        val availableWidth = (measuredWidth - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(0)
        val placeholderHeight = resolvePlaceholderHeightForAvailableWidth(availableWidth)
        val laidOutTextHeight = if (isLaidOut) layout?.height else null
        if (laidOutTextHeight != null && laidOutTextHeight > 0) {
            return maxOf(
                laidOutTextHeight + compoundPaddingTop + compoundPaddingBottom,
                placeholderHeight ?: 0
            )
        }

        val currentText = text
        if (availableWidth > 0 && currentText != null) {
            val staticLayout = StaticLayout.Builder
                .obtain(currentText, 0, currentText.length, paint, availableWidth)
                .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                .setIncludePad(includeFontPadding)
                .build()
            val textHeight = staticLayout.height.takeIf { it > 0 } ?: lineHeight
            return maxOf(
                textHeight + compoundPaddingTop + compoundPaddingBottom,
                placeholderHeight ?: 0
            )
        }

        val minimumHeight = suggestedMinimumHeight.coerceAtLeast(minHeight)
        return maxOf(
            placeholderHeight ?: 0,
            (lineHeight + compoundPaddingTop + compoundPaddingBottom).coerceAtLeast(minimumHeight)
        )
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

    internal fun caretRect(): RectF? {
        val textLayout = layout ?: return null
        val selectionOffset = selectionEnd.takeIf { it >= 0 } ?: return null
        val clampedOffset = selectionOffset.coerceIn(0, textLayout.text.length)
        val caretLeft = textLayout.getPrimaryHorizontal(clampedOffset)
        // Clip the caret to the rendered glyph height so a ParagraphSpacerSpan's
        // inflated descent does not stretch it into the inter-block gap.
        val bounds = CaretGeometry.verticalBounds(textLayout, clampedOffset, paint)
        val left = totalPaddingLeft + caretLeft - scrollX
        val top = totalPaddingTop + bounds.top - scrollY
        val bottom = totalPaddingTop + bounds.bottom - scrollY
        return RectF(left, top, left + 1f, bottom)
    }

    // ── Input Handling: Text Commit ─────────────────────────────────────

    /**
     * Handle committed text from the IME (typed characters, autocomplete).
     *
     * Called by [EditorInputConnection.commitText]. Routes the text through
     * the Rust editor instead of directly inserting into the EditText.
     */
    fun handleTextCommit(text: String, newCursorPosition: Int = 1) {
        val startedAt = System.nanoTime()
        if (!isEditable) {
            recordImeTraceForTesting("handleTextCommitNoop", "reason=notEditable textLength=${text.length}")
            return
        }
        if (isApplyingRustState) {
            recordImeTraceForTesting("handleTextCommitNoop", "reason=applyingRust textLength=${text.length}")
            return
        }
        val selectionRange = normalizedUtf16SelectionRange()
        if (selectionRange == null) {
            recordImeTraceForTesting("handleTextCommitNoop", "reason=noSelection textLength=${text.length}")
            return
        }
        if (editorId == 0L) {
            // No Rust editor bound — fall through to direct editing (dev mode).
            val editable = this.text ?: return
            val (start, end) = selectionRange
            editable.replace(start, end, text)
            recordImeTraceForTesting(
                "handleTextCommitDirect",
                "textLength=${text.length} utf16Sel=$start..$end totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
            )
            return
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) {
            recordImeTraceForTesting("handleTextCommitNoop", "reason=destroyedEditor textLength=${text.length}")
            return
        }

        // Handle Enter/Return as a block split operation.
        if (text == "\n") {
            recordImeTraceForTesting(
                "handleTextCommit",
                "route=return utf16Sel=${selectionRange.first}..${selectionRange.second}"
            )
            handleReturnKey()
            recordImeTraceForTesting(
                "handleTextCommitDone",
                "route=return totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
            )
            return
        }

        val currentText = this.text?.toString() ?: ""
        val scalarSelectionRange = normalizedScalarSelectionRange(currentText)
        if (scalarSelectionRange == null) {
            recordImeTraceForTesting("handleTextCommitNoop", "reason=noScalarSelection textLength=${text.length}")
            return
        }
        val (scalarStart, scalarEnd) = scalarSelectionRange
        val requestedCursor = requestedCursorScalar(
            scalarStart,
            scalarEnd,
            currentText,
            text,
            newCursorPosition
        )
        recordImeTraceForTesting(
            "handleTextCommit",
            "textLength=${text.length} cursor=$newCursorPosition utf16Sel=${selectionRange.first}..${selectionRange.second} scalarSel=$scalarStart..$scalarEnd requestedCursor=$requestedCursor"
        )
        val didApplyOptimisticVisibleText = applyOptimisticPlainTextCommitIfPossible(
            startUtf16 = selectionRange.first,
            endUtf16 = selectionRange.second,
            committedText = text,
            newCursorPosition = newCursorPosition
        )
        if (didApplyOptimisticVisibleText) {
            recordImeTraceForTesting(
                "optimisticVisibleTextCommit",
                "textLength=${text.length} utf16Sel=${selectionRange.first}..${selectionRange.second}"
            )
        }
        insertPlainTextRangeInRust(
            scalarStart,
            scalarEnd,
            text,
            requestedCursorScalar = requestedCursor
        )
        recordImeTraceForTesting(
            "handleTextCommitDone",
            "textLength=${text.length} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    private data class OptimisticInlineSpan(
        val span: Any,
        val flags: Int
    )

    private fun applyOptimisticPlainTextCommitIfPossible(
        startUtf16: Int,
        endUtf16: Int,
        committedText: String,
        newCursorPosition: Int
    ): Boolean {
        if (newCursorPosition != 1) return false
        if (startUtf16 != endUtf16) return false
        if (committedText.isEmpty()) return false
        if (committedText.codePointCount(0, committedText.length) != 1) return false
        if (committedText.indexOf('\n') >= 0 || committedText.indexOf('\r') >= 0) return false
        if (hasCompositionTrackingForEditor()) return false
        val editable = text ?: return false
        val currentText = editable.toString()
        if (currentText != lastAuthorizedText) return false
        if (startUtf16 < 0 || endUtf16 < startUtf16 || endUtf16 > editable.length) return false
        val spanned = editable as? Spanned
        if (spanned != null && spannedRangeContainsImageSpan(spanned, startUtf16, endUtf16)) return false

        val inlineSpans = spanned?.let {
            optimisticInlineSpansForInsertion(it, startUtf16)
        }.orEmpty()
        var didApply = false
        runWithTransientInputMutationGuard {
            editable.replace(startUtf16, endUtf16, committedText)
            val insertedEnd = startUtf16 + committedText.length
            applyOptimisticInlineSpans(editable, startUtf16, insertedEnd, inlineSpans)
            Selection.setSelection(editable, insertedEnd, insertedEnd)
            didApply = true
            true
        }
        if (didApply) {
            pendingOptimisticRenderText = editable.toString()
        }
        return didApply
    }

    private fun optimisticInlineSpansForInsertion(
        spanned: Spanned,
        insertionStart: Int
    ): List<OptimisticInlineSpan> {
        if (spanned.isEmpty()) return emptyList()
        val sourceIndex = when {
            insertionStart > 0 -> insertionStart - 1
            insertionStart < spanned.length -> insertionStart
            else -> return emptyList()
        }
        val queryStart = sourceIndex.coerceIn(0, spanned.length - 1)
        val queryEnd = (queryStart + 1).coerceAtMost(spanned.length)
        val spans = mutableListOf<OptimisticInlineSpan>()
        spanned.getSpans(queryStart, queryEnd, Any::class.java).forEach { span ->
            if (spanned.getSpanStart(span) > queryStart || spanned.getSpanEnd(span) <= queryStart) {
                return@forEach
            }
            cloneOptimisticInlineSpan(span)?.let { clone ->
                spans.add(OptimisticInlineSpan(clone, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE))
            }
        }
        return spans
    }

    private fun cloneOptimisticInlineSpan(span: Any): Any? =
        when (span) {
            is ForegroundColorSpan -> ForegroundColorSpan(span.foregroundColor)
            is BackgroundColorSpan -> BackgroundColorSpan(span.backgroundColor)
            is AbsoluteSizeSpan -> AbsoluteSizeSpan(span.size, span.dip)
            is StyleSpan -> StyleSpan(span.style)
            is UnderlineSpan -> UnderlineSpan()
            is StrikethroughSpan -> StrikethroughSpan()
            else -> null
        }

    private fun applyOptimisticInlineSpans(
        editable: Editable,
        start: Int,
        end: Int,
        inlineSpans: List<OptimisticInlineSpan>
    ) {
        if (start >= end || end > editable.length) return
        var hasColor = false
        var hasSize = false
        inlineSpans.forEach { spec ->
            hasColor = hasColor || spec.span is ForegroundColorSpan
            hasSize = hasSize || spec.span is AbsoluteSizeSpan
            editable.setSpan(spec.span, start, end, spec.flags)
        }
        val textStyle = theme?.effectiveTextStyle("paragraph")
        if (!hasColor) {
            editable.setSpan(
                ForegroundColorSpan(textStyle?.color ?: baseTextColor),
                start,
                end,
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }
        if (!hasSize) {
            val resolvedTextSize = textStyle?.fontSize?.times(resources.displayMetrics.density) ?: baseFontSize
            editable.setSpan(
                AbsoluteSizeSpan(resolvedTextSize.toInt(), false),
                start,
                end,
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }
    }

    internal fun runWithTransientInputMutationGuard(block: () -> Boolean): Boolean {
        val wasApplyingRustState = isApplyingRustState
        isApplyingRustState = true
        return try {
            block()
        } finally {
            isApplyingRustState = wasApplyingRustState
        }
    }

    internal fun authorizedUtf16Range(start: Int, end: Int): Pair<Int, Int> {
        if (start == end) {
            val snapped = PositionBridge.snapToScalarBoundary(
                start,
                lastAuthorizedText,
                biasForward = true
            )
            return snapped to snapped
        }
        return PositionBridge.snapRangeToScalarBoundaries(start, end, lastAuthorizedText)
    }

    internal fun isCurrentTextAuthorizedForEditor(): Boolean =
        (text?.toString() ?: "") == lastAuthorizedText

    internal fun captureCompositionReplacementRangeIfNeeded() {
        if (didInvalidateCompositionReplacementRange) return
        if (compositionReplacementRange() != null) return
        val (start, end) = normalizedUtf16SelectionRange() ?: return
        setCompositionReplacementRange(start, end)
    }

    internal fun setCompositionReplacementRange(start: Int, end: Int) {
        if (didInvalidateCompositionReplacementRange) return
        val replacementRange = authorizedUtf16Range(start, end)
        composingReplacementStartUtf16 = replacementRange.first
        composingReplacementEndUtf16 = replacementRange.second
        composingReplacementAuthorizedTextRevision = lastAuthorizedTextRevision
        didInvalidateCompositionReplacementRange = false
    }

    internal fun compositionReplacementRange(): Pair<Int, Int>? {
        val start = composingReplacementStartUtf16 ?: return null
        val end = composingReplacementEndUtf16 ?: return null
        if (composingReplacementAuthorizedTextRevision != lastAuthorizedTextRevision) {
            clearCompositionTrackingForEditor()
            didInvalidateCompositionReplacementRange = true
            return null
        }
        return start to end
    }

    private fun authorizedSelectionForTransientInputRestore(
        currentStart: Int,
        currentEnd: Int
    ): Pair<Int, Int>? {
        compositionReplacementRange()?.let { return it }
        return if (
            currentStart >= 0 &&
            currentEnd >= 0 &&
            currentStart <= lastAuthorizedText.length &&
            currentEnd <= lastAuthorizedText.length
        ) {
            currentStart to currentEnd
        } else {
            null
        }
    }

    internal fun consumeInvalidatedCompositionReplacementRangeForEditor(): Boolean {
        val invalidated = didInvalidateCompositionReplacementRange
        didInvalidateCompositionReplacementRange = false
        return invalidated
    }

    internal fun hasInvalidatedCompositionReplacementRangeForEditor(): Boolean =
        didInvalidateCompositionReplacementRange

    internal fun setComposingTextForEditor(text: String?) {
        composingText = text
    }

    internal fun composingTextForEditor(): String? = composingText

    internal fun samsungSentenceCapsComposingTextForEditor(composingText: String?): String? {
        if (composingText.isNullOrEmpty()) return composingText
        if (!isSamsungKeyboardActiveForEditor()) return composingText
        if ((inputType and InputType.TYPE_TEXT_FLAG_CAP_SENTENCES) != InputType.TYPE_TEXT_FLAG_CAP_SENTENCES) {
            return composingText
        }
        val (replacementStart, replacementEnd) = compositionReplacementRange() ?: return composingText
        if (replacementStart != replacementEnd) return composingText
        val authorizedSpanned = lastAuthorizedRenderedText as? Spanned
            ?: SpannableStringBuilder(lastAuthorizedText)
        if (!isRenderedLineStartForSentenceCaps(authorizedSpanned, replacementStart)) {
            return composingText
        }

        val firstCodePoint = Character.codePointAt(composingText, 0)
        if (!Character.isLowerCase(firstCodePoint)) return composingText
        val adjusted = buildString(composingText.length) {
            appendCodePoint(Character.toTitleCase(firstCodePoint))
            append(composingText.substring(Character.charCount(firstCodePoint)))
        }
        recordImeTraceForTesting(
            "samsungSentenceCapsFallback",
            "range=$replacementStart..$replacementEnd textLength=${composingText.length}"
        )
        return adjusted
    }

    internal fun applyTransientComposingTextStyleForEditor() {
        val editable = text ?: return
        removeTransientComposingTextStyleSpans(editable)

        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        if (start < 0 || end < 0 || start >= end || end > editable.length) return

        val textStyle = theme?.effectiveTextStyle("paragraph")
        val resolvedTextSize = textStyle?.fontSize?.times(resources.displayMetrics.density) ?: baseFontSize
        val resolvedTextColor = textStyle?.color ?: baseTextColor

        editable.setSpan(
            TransientComposingSizeSpan(resolvedTextSize.toInt()),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        editable.setSpan(
            TransientComposingColorSpan(resolvedTextColor),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )

        val typefaceStyle = textStyle?.typefaceStyle() ?: Typeface.NORMAL
        if (typefaceStyle != Typeface.NORMAL) {
            editable.setSpan(
                TransientComposingStyleSpan(typefaceStyle),
                start,
                end,
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }

        val fontFamily = textStyle?.fontFamily?.takeIf { it.isNotBlank() }
        if (fontFamily != null) {
            editable.setSpan(
                TransientComposingTypefaceSpan(fontFamily),
                start,
                end,
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
            )
        }
    }

    private fun removeTransientComposingTextStyleSpans(editable: Editable) {
        editable
            .getSpans(0, editable.length, TransientComposingTextStyleSpan::class.java)
            .forEach(editable::removeSpan)
    }

    internal fun composingTextFromVisibleReplacementForEditor(): String? {
        val (start, end) = compositionReplacementRange() ?: return null
        val authorizedText = lastAuthorizedText
        val currentText = text?.toString() ?: return null
        if (start < 0 || end < start || end > authorizedText.length) return null

        val authorizedPrefix = authorizedText.substring(0, start)
        val authorizedSuffix = authorizedText.substring(end)
        if (!currentText.startsWith(authorizedPrefix)) return null
        if (!currentText.endsWith(authorizedSuffix)) return null

        val replacementEnd = currentText.length - authorizedSuffix.length
        if (replacementEnd < authorizedPrefix.length) return null
        return currentText.substring(authorizedPrefix.length, replacementEnd)
    }

    internal fun clearCompositionTrackingForEditor() {
        composingText = null
        composingReplacementStartUtf16 = null
        composingReplacementEndUtf16 = null
        composingReplacementAuthorizedTextRevision = null
    }

    private fun hasCompositionTrackingForEditor(): Boolean =
        composingText != null ||
            composingReplacementStartUtf16 != null ||
            composingReplacementEndUtf16 != null ||
            composingReplacementAuthorizedTextRevision != null

    private fun retireInputConnectionForEditor() {
        recordImeTraceForTesting("retireInputConnection")
        activeInputConnection?.clearCompositionTrackingForEditor()
        invalidateInputConnectionsForEditor()
        clearCompositionTrackingForEditor()
        clearCompositionInvalidationForEditor()
        clearNativeComposingSpans()
    }

    internal fun isEditorDestroyedForInput(): Boolean =
        editorId != 0L && NativeEditorViewRegistry.isDestroyed(editorId)

    private fun hasLiveEditor(): Boolean =
        editorId != 0L && !isEditorDestroyedForInput()

    private fun discardTransientInputForDestroyedEditorIfNeeded(): Boolean {
        if (!isEditorDestroyedForInput()) return false
        retireInputConnectionForEditor()
        clearNativeTextMutationAfterBlurWindow()
        clearNativeTextMutationAdoptionSuppression()
        return true
    }

    private fun discardTransientInputAndRestoreAuthorizedTextForEditor() {
        retireInputConnectionForEditor()
        clearNativeTextMutationAfterBlurWindow()
        restoreAuthorizedTextSnapshotForEditor()
        suppressNativeTextMutationAdoptionForCurrentRevision()
    }

    private fun restoreAuthorizedTextSnapshotForEditor() {
        if ((text?.toString() ?: "") == lastAuthorizedText) return
        val authorizedSnapshot = lastAuthorizedRenderedText ?: lastAuthorizedText
        val wasApplyingRustState = isApplyingRustState
        isApplyingRustState = true
        beginBatchEdit()
        try {
            setText(authorizedSnapshot)
        } finally {
            endBatchEdit()
            isApplyingRustState = wasApplyingRustState
        }
    }

    private fun restartInputAfterCompositionInvalidationIfNeeded(shouldRestart: Boolean) {
        if (!shouldRestart) return
        restartInputForEditorIfFocused("focused")
    }

    private fun restartInputForEditorIfFocused(source: String) {
        if (!hasFocus()) return
        restartInputForEditor(source)
    }

    private fun restartInputForEditor(source: String = "explicit") {
        recordImeTraceForTesting("restartInput", "source=$source")
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        imm?.restartInput(this)
        scheduleSelectionUpdateAfterRestartInput(source)
    }

    private fun scheduleSelectionUpdateAfterRestartInput(source: String) {
        val generation = ++restartInputSelectionUpdateGeneration
        post {
            if (generation != restartInputSelectionUpdateGeneration) return@post
            if (!hasFocus()) return@post
            val start = selectionStart
            val end = selectionEnd
            if (start < 0 || end < 0) {
                recordImeTraceForTesting(
                    "updateSelectionAfterRestartSkipped",
                    "source=$source reason=selection start=$start end=$end"
                )
                return@post
            }
            val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            imm?.updateSelection(this, start, end, -1, -1)
            recordImeTraceForTesting(
                "updateSelectionAfterRestart",
                "source=$source sel=$start..$end"
            )
        }
    }

    private fun scheduleLineBoundaryInputRefreshForEditor(source: String) {
        if (!hasFocus()) return
        val generation = ++lineBoundaryInputRefreshGeneration
        recordImeTraceForTesting(
            "lineBoundaryInputRefreshScheduled",
            "source=$source generation=$generation"
        )
        post {
            if (generation != lineBoundaryInputRefreshGeneration) return@post
            if (!hasFocus()) return@post
            if (!isCursorAtRenderedLineStartForSentenceCaps()) {
                recordImeTraceForTesting(
                    "lineBoundaryInputRefreshSkipped",
                    "source=$source reason=cursor"
                )
                return@post
            }
            restartInputForEditor("lineBoundary:$source")
        }
    }

    private fun clearCompositionInvalidationForEditor() {
        didInvalidateCompositionReplacementRange = false
    }

    private fun nextInputConnectionGenerationForEditor(): Long {
        return inputConnectionGeneration
    }

    internal fun isInputConnectionCurrentForEditor(
        boundEditorId: Long,
        boundGeneration: Long
    ): Boolean =
        editorId == boundEditorId &&
            inputConnectionGeneration == boundGeneration &&
            !isEditorDestroyedForInput()

    private fun invalidateInputConnectionsForEditor() {
        inputConnectionGeneration += 1L
        recordImeTraceForTesting("invalidateInputConnections", "nextGen=$inputConnectionGeneration")
        activeInputConnection = null
    }

    private fun clearNativeComposingSpans() {
        val editable = text ?: return
        BaseInputConnection.removeComposingSpans(editable)
        removeTransientComposingTextStyleSpans(editable)
    }

    internal fun restoreAuthorizedTextIfNeeded() {
        if (!hasLiveEditor()) return
        if ((text?.toString() ?: "") == lastAuthorizedText) return
        recordImeTraceForTesting(
            "restoreAuthorizedText",
            "authorizedLength=${lastAuthorizedText.length}"
        )
        val stateJSON = v2Driver?.currentStateJson() ?: return
        applyUpdateJSON(stateJSON)
    }

    fun discardTransientNativeInputForEditorRebind() {
        retireInputConnectionForEditor()
        nativeTextMutationAfterBlurWindow = null
        clearNativeTextMutationAdoptionSuppression()
        clearImeTraceForTesting()
    }

    internal fun discardTransientNativeInputForExternalRecovery() {
        retireInputConnectionForEditor()
        nativeTextMutationAfterBlurWindow = null
        restoreAuthorizedTextIfNeeded()
        suppressNativeTextMutationAdoptionForCurrentRevision()
    }

    private fun discardTransientNativeInputForReadOnly() {
        discardTransientNativeInputForExternalRecovery()
    }

    fun prepareForExternalEditorUpdate(): Boolean =
        prepareForExternalEditorUpdateInternal().ready

    /**
     * Performs external-update preflight while retaining a mutation snapshot
     * for the caller that will apply it. This prevents a second state render
     * after a composing commit has already produced and adopted one.
     */
    fun prepareForExternalEditorUpdateWithResult(): ExternalEditorUpdatePreparation {
        externalUpdatePreparationCaptureDepth += 1
        if (externalUpdatePreparationCaptureDepth == 1) {
            capturedExternalUpdatePreparationJSON = null
        }
        return try {
            val preparation = prepareForExternalEditorUpdateInternal()
            ExternalEditorUpdatePreparation(
                ready = preparation.ready,
                adoptedUpdateJSON = capturedExternalUpdatePreparationJSON
            )
        } finally {
            externalUpdatePreparationCaptureDepth -= 1
            if (externalUpdatePreparationCaptureDepth == 0) {
                capturedExternalUpdatePreparationJSON = null
            }
        }
    }

    private fun prepareForExternalEditorUpdateInternal(): ExternalEditorUpdatePreparation {
        if (blockExternalEditorUpdatePreparationForTesting) {
            return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) {
            return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
        }
        val inputConnection = activeInputConnection
        if (inputConnection?.flushPendingCompositionForExternalMutation() == false) {
            return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
        }
        return ExternalEditorUpdatePreparation(
            ready = drainNativeTextMutationIfNeeded(
                allowAfterBlur = true,
                preserveInputConnectionForExternalUpdate = true
            ),
            adoptedUpdateJSON = null
        )
    }

    fun prepareForExternalEditorCommand(): CommandPreparation {
        if (blockExternalEditorCommandPreparationForTesting) {
            return CommandPreparation(ready = false, updateJSON = null)
        }
        val previousAuthorizedText = lastAuthorizedText
        val preflight = prepareForExternalEditorUpdateWithResult()
        if (!preflight.ready) {
            return CommandPreparation(ready = false, updateJSON = null)
        }
        if (preflight.adoptedUpdateJSON != null) {
            return CommandPreparation(ready = true, updateJSON = preflight.adoptedUpdateJSON)
        }
        if (!hasLiveEditor() || lastAuthorizedText == previousAuthorizedText) {
            return CommandPreparation(ready = true, updateJSON = null)
        }
        val commandStateJSON = v2Driver?.currentStateJson()
        return CommandPreparation(
            ready = true,
            updateJSON = commandStateJSON
        )
    }

    fun handleCompositionCommit(
        text: String,
        replacementStartUtf16: Int,
        replacementEndUtf16: Int,
        newCursorPosition: Int = 1
    ) {
        val startedAt = System.nanoTime()
        if (!isEditable) {
            recordImeTraceForTesting("handleCompositionCommitNoop", "reason=notEditable textLength=${text.length}")
            return
        }
        if (isApplyingRustState) {
            recordImeTraceForTesting("handleCompositionCommitNoop", "reason=applyingRust textLength=${text.length}")
            return
        }
        if (!hasLiveEditor()) {
            recordImeTraceForTesting("handleCompositionCommitNoop", "reason=noLiveEditor textLength=${text.length}")
            return
        }

        val authorizedText = lastAuthorizedText
        val (startUtf16, endUtf16) = PositionBridge.snapRangeToScalarBoundaries(
            replacementStartUtf16,
            replacementEndUtf16,
            authorizedText
        )
        val scalarStart = PositionBridge.utf16ToScalar(startUtf16, authorizedText)
        val scalarEnd = PositionBridge.utf16ToScalar(endUtf16, authorizedText)

        if (
            startUtf16 <= endUtf16 &&
            endUtf16 <= authorizedText.length &&
            authorizedText.substring(startUtf16, endUtf16) == text
        ) {
            val requestedCursor = requestedCursorScalar(
                scalarStart,
                scalarEnd,
                authorizedText,
                text,
                newCursorPosition
            ) ?: scalarEnd
            recordImeTraceForTesting(
                "handleCompositionCommitNoop",
                "reason=alreadyAuthorized textLength=${text.length} requestedCursor=$requestedCursor range=$startUtf16..$endUtf16"
            )
            restoreAuthorizedTextIfNeeded()
            applyRequestedCursorScalar(requestedCursor)
            return
        }

        if (text == "\n") {
            recordImeTraceForTesting(
                "handleCompositionCommit",
                "route=return textLength=${text.length} utf16Range=$startUtf16..$endUtf16 scalarRange=$scalarStart..$scalarEnd"
            )
            if (scalarStart != scalarEnd) {
                deleteAndSplitInRust(scalarStart, scalarEnd)
            } else {
                splitBlockInRust(scalarStart)
            }
            recordImeTraceForTesting(
                "handleCompositionCommitDone",
                "route=return totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
            )
            return
        }

        val requestedCursor = requestedCursorScalar(
            scalarStart,
            scalarEnd,
            authorizedText,
            text,
            newCursorPosition
        )
        recordImeTraceForTesting(
            "handleCompositionCommit",
            "textLength=${text.length} cursor=$newCursorPosition utf16Range=$startUtf16..$endUtf16 scalarRange=$scalarStart..$scalarEnd requestedCursor=$requestedCursor"
        )
        insertPlainTextRangeInRust(
            scalarStart,
            scalarEnd,
            text,
            requestedCursorScalar = requestedCursor
        )
        recordImeTraceForTesting(
            "handleCompositionCommitDone",
            "textLength=${text.length} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    fun handleCorrectionCommit(
        offsetUtf16: Int,
        oldText: String,
        newText: String
    ): Boolean {
        if (!isEditable) return true
        if (isApplyingRustState) return true
        if (!hasLiveEditor()) return false

        val authorizedText = lastAuthorizedText
        if (offsetUtf16 < 0) {
            recordImeTraceForTesting(
                "correctionExplicitNoop",
                "reason=invalidOffset offset=$offsetUtf16 oldLength=${oldText.length} newLength=${newText.length}"
            )
            return false
        }
        val endUtf16 = offsetUtf16 + oldText.length
        if (endUtf16 < offsetUtf16 || endUtf16 > authorizedText.length) {
            recordImeTraceForTesting(
                "correctionExplicitNoop",
                "reason=outOfBounds offset=$offsetUtf16 oldLength=${oldText.length} authorizedLength=${authorizedText.length}"
            )
            return false
        }
        if (authorizedText.substring(offsetUtf16, endUtf16) != oldText) {
            recordImeTraceForTesting(
                "correctionExplicitNoop",
                "reason=staleText offset=$offsetUtf16 oldLength=${oldText.length} newLength=${newText.length}"
            )
            return false
        }

        val (startUtf16, snappedEndUtf16) = PositionBridge.snapRangeToScalarBoundaries(
            offsetUtf16,
            endUtf16,
            authorizedText
        )
        if (
            startUtf16 != offsetUtf16 ||
            snappedEndUtf16 != endUtf16 ||
            startUtf16 > snappedEndUtf16
        ) {
            recordImeTraceForTesting(
                "correctionExplicitNoop",
                "reason=unsnappedScalarBoundary range=$offsetUtf16..$endUtf16 snapped=$startUtf16..$snappedEndUtf16"
            )
            return false
        }

        val scalarStart = PositionBridge.utf16ToScalar(startUtf16, authorizedText)
        val scalarEnd = PositionBridge.utf16ToScalar(snappedEndUtf16, authorizedText)
        recordImeTraceForTesting(
            "correctionExplicitApply",
            "range=$scalarStart..$scalarEnd newLength=${newText.length}"
        )
        insertPlainTextRangeInRust(scalarStart, scalarEnd, newText)
        return true
    }

    fun handleMissingOldTextCorrectionCommit(
        offsetUtf16: Int,
        newText: String
    ): Boolean {
        if (!isEditable) return true
        if (isApplyingRustState) return true
        if (!hasLiveEditor()) return false

        val authorizedText = lastAuthorizedText
        val tokenRange = missingOldTextCorrectionTokenRange(authorizedText, offsetUtf16)
            ?: run {
                recordImeTraceForTesting(
                    "correctionInferredNoop",
                    "reason=noToken offset=$offsetUtf16 newLength=${newText.length}"
                )
                return false
            }
        val (startUtf16, endUtf16) = tokenRange

        val (snappedStartUtf16, snappedEndUtf16) = PositionBridge.snapRangeToScalarBoundaries(
            startUtf16,
            endUtf16,
            authorizedText
        )
        if (snappedStartUtf16 >= snappedEndUtf16) {
            recordImeTraceForTesting(
                "correctionInferredNoop",
                "reason=emptySnappedRange token=$startUtf16..$endUtf16 snapped=$snappedStartUtf16..$snappedEndUtf16"
            )
            return false
        }

        val scalarStart = PositionBridge.utf16ToScalar(snappedStartUtf16, authorizedText)
        val scalarEnd = PositionBridge.utf16ToScalar(snappedEndUtf16, authorizedText)
        recordImeTraceForTesting(
            "correctionInferredApply",
            "range=$scalarStart..$scalarEnd utf16=$snappedStartUtf16..$snappedEndUtf16 newLength=${newText.length}"
        )
        insertPlainTextRangeInRust(scalarStart, scalarEnd, newText)
        return true
    }

    private fun missingOldTextCorrectionTokenRange(
        text: String,
        offsetUtf16: Int
    ): Pair<Int, Int>? {
        if (offsetUtf16 < 0 || offsetUtf16 >= text.length) return null

        val tokenOffset = PositionBridge.snapToScalarBoundary(
            offsetUtf16,
            text,
            biasForward = false
        )
        if (tokenOffset < 0 || tokenOffset >= text.length) return null
        if (!isMissingOldTextCorrectionTokenCodePointAt(text, tokenOffset)) return null

        var startUtf16 = tokenOffset
        while (startUtf16 > 0) {
            val previousUtf16 = Character.offsetByCodePoints(text, startUtf16, -1)
            if (!isMissingOldTextCorrectionTokenCodePointAt(text, previousUtf16)) break
            startUtf16 = previousUtf16
        }

        var endUtf16 = tokenOffset + Character.charCount(Character.codePointAt(text, tokenOffset))
        while (endUtf16 < text.length) {
            if (!isMissingOldTextCorrectionTokenCodePointAt(text, endUtf16)) break
            endUtf16 += Character.charCount(Character.codePointAt(text, endUtf16))
        }

        return if (startUtf16 < endUtf16) startUtf16 to endUtf16 else null
    }

    private fun isMissingOldTextCorrectionTokenCodePointAt(text: String, utf16Offset: Int): Boolean {
        if (utf16Offset < 0 || utf16Offset >= text.length) return false
        val codePoint = Character.codePointAt(text, utf16Offset)
        if (isMissingOldTextCorrectionCoreTokenCodePoint(codePoint)) return true
        if (!isMissingOldTextCorrectionJoinerCodePoint(codePoint)) return false

        val previousCodePoint = previousCodePointBefore(text, utf16Offset) ?: return false
        val nextUtf16Offset = utf16Offset + Character.charCount(codePoint)
        val nextCodePoint = nextCodePointAt(text, nextUtf16Offset) ?: return false
        return isMissingOldTextCorrectionCoreTokenCodePoint(previousCodePoint) &&
            isMissingOldTextCorrectionCoreTokenCodePoint(nextCodePoint)
    }

    private fun isMissingOldTextCorrectionCoreTokenCodePoint(codePoint: Int): Boolean {
        if (Character.isLetterOrDigit(codePoint)) return true
        return when (Character.getType(codePoint)) {
            Character.NON_SPACING_MARK.toInt(),
            Character.COMBINING_SPACING_MARK.toInt(),
            Character.ENCLOSING_MARK.toInt(),
            Character.CONNECTOR_PUNCTUATION.toInt(),
            Character.MATH_SYMBOL.toInt(),
            Character.CURRENCY_SYMBOL.toInt(),
            Character.MODIFIER_SYMBOL.toInt(),
            Character.OTHER_SYMBOL.toInt(),
            Character.SURROGATE.toInt() -> true
            else -> false
        }
    }

    private fun isMissingOldTextCorrectionJoinerCodePoint(codePoint: Int): Boolean =
        codePoint == '\''.code ||
            codePoint == 0x2018 ||
            codePoint == 0x2019 ||
            codePoint == 0x201B ||
            codePoint == 0xFF07 ||
            codePoint == '-'.code ||
            codePoint == 0x2010 ||
            codePoint == 0x2011 ||
            codePoint == 0x2012 ||
            codePoint == 0x2013 ||
            codePoint == 0x2014 ||
            codePoint == 0x2212 ||
            codePoint == 0x200D

    private fun previousCodePointBefore(text: String, utf16Offset: Int): Int? {
        if (utf16Offset <= 0 || utf16Offset > text.length) return null
        val previousUtf16 = Character.offsetByCodePoints(text, utf16Offset, -1)
        return Character.codePointAt(text, previousUtf16)
    }

    private fun nextCodePointAt(text: String, utf16Offset: Int): Int? {
        if (utf16Offset < 0 || utf16Offset >= text.length) return null
        return Character.codePointAt(text, utf16Offset)
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
        val selectionRange = normalizedUtf16SelectionRange()
        if (editorId == 0L) {
            // Dev mode: direct editing.
            val editable = this.text ?: return
            val (selectionStart, selectionEnd) = selectionRange ?: return
            val delStart: Int
            val delEnd: Int
            if (selectionStart != selectionEnd) {
                delStart = selectionStart
                delEnd = selectionEnd
            } else {
                delStart = maxOf(0, selectionStart - beforeLength.coerceAtLeast(0))
                delEnd = minOf(editable.length, selectionStart + afterLength.coerceAtLeast(0))
            }
            editable.delete(delStart, delEnd)
            return
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) return

        val currentText = text?.toString() ?: ""
        val (selectionStart, selectionEnd) = selectionRange ?: return
        if (selectionStart != selectionEnd) {
            val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
            deleteRangeInRust(scalarStart, scalarEnd)
            return
        }
        val cursor = selectionStart
        if (beforeLength > 0 &&
            afterLength == 0 &&
            cursor > 0 &&
            currentText.getOrNull(cursor - 1) == EMPTY_BLOCK_PLACEHOLDER
        ) {
            val scalarCursor = PositionBridge.utf16ToScalar(cursor, currentText)
            deleteBackwardAtSelectionScalarInRust(scalarCursor, scalarCursor)
            return
        }
        val rawDelStart = maxOf(0, cursor - beforeLength.coerceAtLeast(0))
        val rawDelEnd = minOf(currentText.length, cursor + afterLength.coerceAtLeast(0))
        val (delStart, delEnd) = PositionBridge.snapRangeToScalarBoundaries(
            rawDelStart,
            rawDelEnd,
            currentText
        )

        val scalarStart = PositionBridge.utf16ToScalar(delStart, currentText)
        val scalarEnd = PositionBridge.utf16ToScalar(delEnd, currentText)

        if (scalarStart < scalarEnd) {
            deleteRangeInRust(scalarStart, scalarEnd)
        } else if (beforeLength > 0 && afterLength == 0) {
            deleteBackwardAtSelectionScalarInRust(scalarEnd, scalarEnd)
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
        val selectionRange = normalizedUtf16SelectionRange() ?: return
        if (editorId == 0L) {
            // Dev mode: direct editing.
            val editable = this.text ?: return
            val (start, end) = selectionRange
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
        if (discardTransientInputForDestroyedEditorIfNeeded()) return

        val currentText = text?.toString() ?: ""
        val (start, end) = selectionRange

        if (start != end) {
            // Range selection: delete the range.
            val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
            deleteRangeInRust(scalarStart, scalarEnd)
        } else if (start > 0) {
            if (currentText.getOrNull(start - 1) == EMPTY_BLOCK_PLACEHOLDER) {
                val scalarCursor = PositionBridge.utf16ToScalar(start, currentText)
                deleteBackwardAtSelectionScalarInRust(scalarCursor, scalarCursor)
                return
            }
            // Cursor: delete one grapheme cluster backward.
            // Find the previous grapheme boundary by snapping (start - 1).
            val breakIter = java.text.BreakIterator.getCharacterInstance()
            breakIter.setText(currentText)
            val prevBoundary = breakIter.preceding(start)
            val prevUtf16 = if (prevBoundary == java.text.BreakIterator.DONE) 0 else prevBoundary

            val scalarStart = PositionBridge.utf16ToScalar(prevUtf16, currentText)
            val scalarEnd = PositionBridge.utf16ToScalar(start, currentText)
            if (scalarStart < scalarEnd) {
                deleteRangeInRust(scalarStart, scalarEnd)
            } else {
                deleteBackwardAtSelectionScalarInRust(scalarEnd, scalarEnd)
            }
        } else {
            deleteBackwardAtSelectionScalarInRust(0, 0)
        }
    }

    fun handleForwardDelete() {
        if (!isEditable) return
        if (isApplyingRustState) return
        val selectionRange = normalizedUtf16SelectionRange() ?: return
        if (editorId == 0L) {
            val editable = this.text ?: return
            val (start, end) = selectionRange
            if (start != end) {
                editable.delete(start, end)
            } else if (start < editable.length) {
                val breakIter = java.text.BreakIterator.getCharacterInstance()
                breakIter.setText(editable.toString())
                val nextBoundary = breakIter.following(start)
                val nextUtf16 = if (nextBoundary == java.text.BreakIterator.DONE) {
                    editable.length
                } else {
                    nextBoundary
                }
                editable.delete(start, nextUtf16.coerceIn(start, editable.length))
            }
            return
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) return

        val currentText = text?.toString() ?: ""
        val (start, end) = selectionRange
        if (start != end) {
            val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
            deleteRangeInRust(scalarStart, scalarEnd)
        } else if (start < currentText.length) {
            val breakIter = java.text.BreakIterator.getCharacterInstance()
            breakIter.setText(currentText)
            val nextBoundary = breakIter.following(start)
            val nextUtf16 = if (nextBoundary == java.text.BreakIterator.DONE) {
                currentText.length
            } else {
                nextBoundary
            }
            val (utf16Start, utf16End) = PositionBridge.snapRangeToScalarBoundaries(
                start,
                nextUtf16.coerceIn(start, currentText.length),
                currentText
            )
            val scalarStart = PositionBridge.utf16ToScalar(utf16Start, currentText)
            val scalarEnd = PositionBridge.utf16ToScalar(utf16End, currentText)
            if (scalarStart < scalarEnd) {
                deleteRangeInRust(scalarStart, scalarEnd)
            }
        }
    }

    // ── Input Handling: Return Key ──────────────────────────────────────

    /**
     * Handle return/enter key as a block split operation.
     */
    fun handleReturnKey() {
        if (!isEditable) return
        if (isApplyingRustState) return

        val currentText = text?.toString() ?: ""
        val (start, end) = normalizedUtf16SelectionRange() ?: return

        if (editorId == 0L) {
            // Dev mode: insert newline directly.
            val editable = this.text ?: return
            editable.replace(start, end, "\n")
            return
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) return

        if (start != end) {
            // Range selection: atomic delete-and-split via Rust.
            val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
            deleteAndSplitInRust(scalarStart, scalarEnd)
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
        if (discardTransientInputForDestroyedEditorIfNeeded()) return

        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.insertNode("hardBreak", selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    /**
     * Handle hardware Tab / Shift+Tab as list indent / outdent when the caret is in a list.
     */
    fun handleTab(shiftPressed: Boolean): Boolean {
        if (!isEditable) return false
        if (isApplyingRustState) return false
        if (!hasLiveEditor()) return false
        if (!isSelectionInsideList()) return false
        val selection = currentScalarSelection() ?: return false

        v2Driver?.let { driver ->
            val update = if (shiftPressed) {
                driver.outdentListItem(selection.first, selection.second)
            } else {
                driver.indentListItem(selection.first, selection.second)
            }
            update?.let { applyUpdateJSON(it) }
        }
        return true
    }

    fun handleHardwareKeyDown(keyCode: Int, shiftPressed: Boolean): Boolean {
        if (!isEditable || isApplyingRustState) return false
        return when (keyCode) {
            KeyEvent.KEYCODE_DEL -> {
                handleBackspace()
                true
            }
            KeyEvent.KEYCODE_FORWARD_DEL -> {
                handleForwardDelete()
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

    private fun isSupportedHardwareMutationKey(keyCode: Int): Boolean =
        when (keyCode) {
            KeyEvent.KEYCODE_DEL,
            KeyEvent.KEYCODE_FORWARD_DEL,
            KeyEvent.KEYCODE_ENTER,
            KeyEvent.KEYCODE_NUMPAD_ENTER,
            KeyEvent.KEYCODE_TAB -> true
            else -> false
        }

    internal fun isReadOnlyTextMutationKeyEvent(event: KeyEvent): Boolean {
        if (isSupportedHardwareMutationKey(event.keyCode) ||
            event.keyCode == KeyEvent.KEYCODE_FORWARD_DEL
        ) {
            return true
        }
        if (event.keyCode == KeyEvent.KEYCODE_INSERT && event.isShiftPressed) {
            return true
        }
        if (event.isCtrlPressed || event.isMetaPressed) {
            return when (event.keyCode) {
                KeyEvent.KEYCODE_V,
                KeyEvent.KEYCODE_X,
                KeyEvent.KEYCODE_Z,
                KeyEvent.KEYCODE_Y -> true
                else -> false
            }
        }
        if (!keyEventCharacters(event).isNullOrEmpty()) return true
        return event.unicodeChar != 0
    }

    fun handleHardwareKeyEvent(event: KeyEvent?): Boolean {
        if (event == null || !isEditable || isApplyingRustState) return false

        return when (event.action) {
            KeyEvent.ACTION_DOWN -> {
                if (!isSupportedHardwareMutationKey(event.keyCode)) return false

                val signature = hardwareKeyEventSignature(event)
                if (
                    lastHandledHardwareKeySignature == signature ||
                    didRecentlyHandleHardwareKeyDown(signature)
                ) {
                    return true
                }

                if (handleHardwareKeyDown(event.keyCode, event.isShiftPressed)) {
                    markHandledHardwareKeyDown(signature)
                    true
                } else {
                    false
                }
            }

            KeyEvent.ACTION_UP -> {
                if (lastHandledHardwareKeySignature?.let {
                        it.keyCode == event.keyCode && it.downTime == event.downTime
                    } == true) {
                    lastHandledHardwareKeySignature = null
                    true
                } else {
                    false
                }
            }

            else -> false
        }
    }

    internal fun handlePrintableHardwareKeyEvent(
        event: KeyEvent,
        applyBaseEvent: () -> Boolean
    ): Boolean {
        if (!isEditable || isApplyingRustState || !isPrintableHardwareMutationKey(event)) {
            return false
        }
        val signature = hardwareKeyEventSignature(event)
        return when (event.action) {
            KeyEvent.ACTION_DOWN -> {
                if (
                    lastHandledHardwareKeySignature == signature ||
                    didRecentlyHandleHardwareKeyDown(signature)
                ) {
                    true
                } else {
                    val inputConnection = activeInputConnection?.takeIf {
                        it.hasPendingComposition()
                    }
                    if (inputConnection != null) {
                        var didMutate = false
                        runWithTransientInputMutationGuard {
                            didMutate = insertTransientHardwareText(keyEventText(event))
                            didMutate
                        }
                        if (!didMutate) return false
                        inputConnection.refreshComposingTextFromEditableForEditor()
                    } else {
                        applyBaseEvent()
                    }
                    markHandledHardwareKeyDown(signature)
                    true
                }
            }
            KeyEvent.ACTION_UP -> {
                if (lastHandledHardwareKeySignature?.let {
                        it.keyCode == event.keyCode && it.downTime == event.downTime
                    } == true) {
                    lastHandledHardwareKeySignature = null
                }
                false
            }
            else -> false
        }
    }

    private fun isPrintableHardwareMutationKey(event: KeyEvent): Boolean {
        if (isSupportedHardwareMutationKey(event.keyCode)) return false
        if (event.isCtrlPressed || event.isMetaPressed) return false
        return !keyEventText(event).isNullOrEmpty()
    }

    private fun hardwareKeyEventSignature(event: KeyEvent): HardwareKeyEventSignature =
        HardwareKeyEventSignature(
            keyCode = event.keyCode,
            downTime = event.downTime,
            repeatCount = event.repeatCount
        )

    @Suppress("DEPRECATION")
    private fun keyEventCharacters(event: KeyEvent): String? = event.characters

    private fun keyEventText(event: KeyEvent): String? {
        val characters = keyEventCharacters(event)
        if (!characters.isNullOrEmpty()) return characters
        val unicodeChar = event.unicodeChar
        if (unicodeChar == 0) return null
        return runCatching {
            String(Character.toChars(unicodeChar))
        }.getOrNull()
    }

    private fun insertTransientHardwareText(insertedText: String?): Boolean {
        if (insertedText.isNullOrEmpty()) return false
        val editable = text ?: return false
        val currentText = editable.toString()
        val rawStart = selectionStart
        val rawEnd = selectionEnd
        if (rawStart < 0 || rawEnd < 0) return false
        val start = rawStart.coerceIn(0, editable.length)
        val end = rawEnd.coerceIn(0, editable.length)
        val normalizedStart = minOf(start, end)
        val normalizedEnd = maxOf(start, end)
        val (replaceStart, replaceEnd) = PositionBridge.snapRangeToScalarBoundaries(
            normalizedStart,
            normalizedEnd,
            currentText
        )
        editable.replace(replaceStart, replaceEnd, insertedText)
        val cursor = (replaceStart + insertedText.length).coerceIn(0, editable.length)
        setSelection(cursor)
        return true
    }

    private fun markHandledHardwareKeyDown(signature: HardwareKeyEventSignature) {
        lastHandledHardwareKeySignature = signature
        recentHandledHardwareKeyDownSignature = signature
        recentHandledHardwareKeyDownUptimeMs = SystemClock.uptimeMillis()
    }

    private fun didRecentlyHandleHardwareKeyDown(signature: HardwareKeyEventSignature): Boolean {
        val recentSignature = recentHandledHardwareKeyDownSignature ?: return false
        val elapsedMs = SystemClock.uptimeMillis() - recentHandledHardwareKeyDownUptimeMs
        if (elapsedMs > RECENT_HANDLED_HARDWARE_KEY_DOWN_WINDOW_MS) {
            recentHandledHardwareKeyDownSignature = null
            recentHandledHardwareKeyDownUptimeMs = 0L
            return false
        }
        return recentSignature == signature
    }

    fun performToolbarToggleMark(markName: String) {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.toggleMark(markName, selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarToggleList(listType: String, isActive: Boolean) {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            val update = if (isActive) {
                driver.unwrapFromList(selection.first, selection.second)
            } else {
                driver.wrapInList(listType, selection.first, selection.second)
            }
            update?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarToggleBlockquote() {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.toggleBlockquote(selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarToggleHeading(level: Int) {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        if (level !in 1..6) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.toggleHeading(level, selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarIndentListItem() {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.indentListItem(selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarOutdentListItem() {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.outdentListItem(selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarInsertNode(nodeType: String) {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        val selection = currentScalarSelection() ?: return
        v2Driver?.let { driver ->
            driver.insertNode(nodeType, selection.first, selection.second)?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarUndo() {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        v2Driver?.let { driver ->
            driver.undo()?.let { applyUpdateJSON(it) }
        }
    }

    fun performToolbarRedo() {
        if (!isEditable || isApplyingRustState || !hasLiveEditor()) return
        v2Driver?.let { driver ->
            driver.redo()?.let { applyUpdateJSON(it) }
        }
    }

    // ── Input Handling: Paste ────────────────────────────────────────────

    /**
     * Intercept paste operations to route content through Rust.
     *
     * Attempts to extract HTML from the clipboard first (for rich text paste),
     * falling back to plain text.
     */
    override fun onTextContextMenuItem(id: Int): Boolean {
        if (!isEditable && isMutatingContextMenuItem(id)) return true
        if (id == android.R.id.cut) {
            handleCut()
            return true
        }
        if (id == android.R.id.paste || id == android.R.id.pasteAsPlainText) {
            handlePaste(plainTextOnly = id == android.R.id.pasteAsPlainText)
            return true
        }
        return super.onTextContextMenuItem(id)
    }

    private fun isMutatingContextMenuItem(id: Int): Boolean =
        id == android.R.id.paste ||
            id == android.R.id.pasteAsPlainText ||
            id == android.R.id.cut

    /**
     * Block accessibility-initiated text mutations (paste, cut, set text) when not editable.
     * Selection and copy actions remain available.
     */
    override fun performAccessibilityAction(action: Int, arguments: android.os.Bundle?): Boolean {
        if (!isEditable && (
                action == android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT ||
                    action == android.view.accessibility.AccessibilityNodeInfo.ACTION_PASTE ||
                    action == android.view.accessibility.AccessibilityNodeInfo.ACTION_CUT
                )
        ) {
            return false
        }
        if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT) {
            return handleAccessibilitySetText(arguments)
        }
        return super.performAccessibilityAction(action, arguments)
    }

    private fun handlePaste(plainTextOnly: Boolean) {
        if (editorId == 0L) {
            // Dev mode: default paste behavior.
            super.onTextContextMenuItem(
                if (plainTextOnly) android.R.id.pasteAsPlainText else android.R.id.paste
            )
            return
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) return
        if (!prepareForExternalEditorUpdate()) return

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            ?: return
        val clip = clipboard.primaryClip ?: return
        if (clip.itemCount == 0) return

        val item = clip.getItemAt(0)

        // Try HTML first for rich paste.
        val htmlText = item.htmlText
        if (!plainTextOnly && htmlText != null) {
            pasteHTML(htmlText)
            return
        }

        // Fallback to plain text.
        val plainText = item.text?.toString() ?: item.coerceToText(context)?.toString()
        if (plainText != null) {
            pastePlainText(plainText)
        }
    }

    private fun handleCut() {
        if (editorId == 0L) {
            super.onTextContextMenuItem(android.R.id.cut)
            return
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) return
        if (!prepareForExternalEditorUpdate()) return

        val currentText = text?.toString() ?: return
        val (selectionStart, selectionEnd) = normalizedUtf16SelectionRange(currentText) ?: return
        if (selectionStart == selectionEnd) return

        val (utf16Start, utf16End) = PositionBridge.snapRangeToScalarBoundaries(
            selectionStart,
            selectionEnd,
            currentText
        )
        if (utf16Start >= utf16End) return

        val selectedText = currentText.substring(utf16Start, utf16End)
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
        clipboard?.setPrimaryClip(ClipData.newPlainText(null, selectedText))

        val scalarStart = PositionBridge.utf16ToScalar(utf16Start, currentText)
        val scalarEnd = PositionBridge.utf16ToScalar(utf16End, currentText)
        deleteRangeInRust(scalarStart, scalarEnd)
    }

    private fun handleAccessibilitySetText(arguments: android.os.Bundle?): Boolean {
        val replacement = arguments
            ?.getCharSequence(
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE
            )
            ?.toString()
            ?: return false
        if (editorId == 0L) {
            return super.performAccessibilityAction(
                android.view.accessibility.AccessibilityNodeInfo.ACTION_SET_TEXT,
                arguments
            )
        }
        if (discardTransientInputForDestroyedEditorIfNeeded()) return false
        if (!prepareForExternalEditorUpdate()) return false

        val currentText = text?.toString() ?: return false
        val scalarStart = 0
        val scalarEnd = currentText.codePointCount(0, currentText.length)
        insertPlainTextRangeInRust(scalarStart, scalarEnd, replacement)
        return true
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
        if (isApplyingRustState) return
        val spannable = text as? Spanned
        if (spannable != null && isExactImageSpanRange(spannable, selStart, selEnd)) {
            explicitSelectedImageRange = ImageSelectionRange(selStart, selEnd)
        }
        ensureSelectionVisible()
        onSelectionOrContentMayChange?.invoke()
        // Keep the custom caret solid at its new position, then resume blinking.
        restartCaretBlink()

        syncCurrentSelectionToRust()
    }

    private fun syncCurrentSelectionToRust() {
        if (!hasLiveEditor()) return

        val currentText = text?.toString() ?: ""
        if (currentText != lastAuthorizedText) return
        val (scalarAnchor, scalarHead) = rawScalarSelection(currentText) ?: return

        v2Driver?.let { driver ->
            val mapping = driver.syncSelection(scalarAnchor, scalarHead)
            if (mapping != null) {
                editorListener?.onSelectionChanged(mapping[0], mapping[1])
            }
            return
        }
        onSetSelectionScalarInRustForTesting?.invoke(scalarAnchor, scalarHead)
    }

    // ── Rust Integration ────────────────────────────────────────────────

    // Samsung Keyboard may call finishComposingText() and then commitText(" ")
    // for one space tap. Defer the render from finishComposingText() by one
    // loop so setText() does not restart input before the pending space arrives.
    internal fun runWithDeferredRustUpdateApplication(block: () -> Unit) {
        recordImeTraceForTesting(
            "deferRustUpdateBegin",
            "depth=$deferredRustUpdateApplicationDepth pending=${deferredRustUpdateJSON != null}"
        )
        deferredRustUpdateApplicationDepth += 1
        try {
            block()
        } finally {
            deferredRustUpdateApplicationDepth -= 1
            recordImeTraceForTesting(
                "deferRustUpdateEnd",
                "depth=$deferredRustUpdateApplicationDepth pending=${deferredRustUpdateJSON != null}"
            )
            if (deferredRustUpdateApplicationDepth == 0) {
                scheduleDeferredRustUpdateApplication()
            }
        }
    }

    private fun applyRustUpdateJSON(
        updateJSON: String,
        lineBoundaryRefreshSource: String? = null
    ) {
        if (externalUpdatePreparationCaptureDepth > 0) {
            capturedExternalUpdatePreparationJSON = updateJSON
            // Keep the visible IME commit authorized until the external view
            // consumes this exact adapter-adopted snapshot below.
            authorizeCurrentVisibleTextForDeferredRustUpdate()
            recordImeTraceForTesting(
                "rustUpdateCapturedForExternalPreflight",
                "jsonLength=${updateJSON.length} depth=$externalUpdatePreparationCaptureDepth"
            )
            return
        }
        if (deferredRustUpdateApplicationDepth > 0) {
            deferredRustUpdateJSON = updateJSON
            deferredRustUpdateLineBoundaryRefreshSource = lineBoundaryRefreshSource
            recordImeTraceForTesting(
                "rustUpdateDeferred",
                "jsonLength=${updateJSON.length} depth=$deferredRustUpdateApplicationDepth"
            )
            authorizeCurrentVisibleTextForDeferredRustUpdate()
            return
        }
        cancelDeferredRustUpdateApplication()
        recordImeTraceForTesting(
            "rustUpdateApply",
            "mode=immediate jsonLength=${updateJSON.length}"
        )
        if (applyUpdateJSON(updateJSON)) {
            lineBoundaryRefreshSource?.let(::scheduleLineBoundaryInputRefreshForEditor)
        }
    }

    private fun authorizeCurrentVisibleTextForDeferredRustUpdate() {
        lastAuthorizedText = text?.toString().orEmpty()
        lastAuthorizedRenderedText = text?.let { SpannableStringBuilder(it) }
        lastAuthorizedTextRevision += 1L
        currentRenderBlocksJson = null
        clearNativeTextMutationAdoptionSuppression()
        clearNativeTextMutationAfterBlurWindow()
    }

    internal fun authorizeCurrentVisibleTextForPendingImeOperationForEditor() {
        pendingOptimisticRenderText = null
        authorizeCurrentVisibleTextForDeferredRustUpdate()
        recordImeTraceForTesting(
            "authorizePendingImeVisibleText",
            "textLength=${lastAuthorizedText.length}"
        )
    }

    internal fun deleteScalarRangeForPendingImeOperationForEditor(scalarFrom: Int, scalarTo: Int) {
        deleteRangeInRust(scalarFrom, scalarTo)
    }

    internal fun applyVisibleCompositionCommitForPendingImeOperationForEditor(
        committedText: String,
        replacementStartUtf16: Int,
        replacementEndUtf16: Int,
        newCursorPosition: Int
    ): Boolean {
        val editable = text ?: return false
        val currentText = editable.toString()
        val (startUtf16, endUtf16) = PositionBridge.snapRangeToScalarBoundaries(
            replacementStartUtf16,
            replacementEndUtf16,
            currentText
        )
        if (startUtf16 > endUtf16 || endUtf16 > editable.length) return false
        var didApply = false
        runWithTransientInputMutationGuard {
            editable.replace(startUtf16, endUtf16, committedText)
            val insertedEnd = startUtf16 + committedText.length
            val requestedCursor = when {
                newCursorPosition > 0 -> insertedEnd + newCursorPosition - 1
                newCursorPosition < 0 -> startUtf16 + newCursorPosition
                else -> insertedEnd
            }.coerceIn(0, editable.length)
            Selection.setSelection(editable, requestedCursor, requestedCursor)
            didApply = true
            true
        }
        if (didApply) {
            pendingOptimisticRenderText = null
        }
        return didApply
    }

    internal fun commitAlreadyVisibleCompositionMutationForPendingImeOperationForEditor(
        committedText: String,
        newCursorPosition: Int
    ): Boolean {
        if (committedText.isEmpty()) return false
        val currentText = text?.toString() ?: return false
        val mutation = nativeTextMutationFromAuthorizedDiff(currentText) ?: return false
        val tokenRange = committedTokenRangeAroundMutation(
            currentText,
            mutation.replacementStartUtf16,
            mutation.replacementEndUtf16
        ) ?: run {
            recordImeTraceForTesting(
                "alreadyVisibleCompositionNoop",
                "reason=noToken committedLength=${committedText.length} visibleRange=${mutation.replacementStartUtf16}..${mutation.replacementEndUtf16}"
            )
            return false
        }
        val visibleToken = currentText.substring(tokenRange.first, tokenRange.second)
        if (visibleToken != committedText) {
            recordImeTraceForTesting(
                "alreadyVisibleCompositionNoop",
                "reason=tokenMismatch committedLength=${committedText.length} tokenLength=${visibleToken.length} visibleRange=${mutation.replacementStartUtf16}..${mutation.replacementEndUtf16}"
            )
            return false
        }

        val authorizedText = lastAuthorizedText
        val requestedCursor = requestedCursorScalar(
            mutation.scalarFrom,
            mutation.scalarTo,
            authorizedText,
            mutation.replacementText,
            newCursorPosition
        )
        recordImeTraceForTesting(
            "alreadyVisibleCompositionApply",
            "range=${mutation.scalarFrom}..${mutation.scalarTo} replacementLength=${mutation.replacementText.length} committedLength=${committedText.length} requestedCursor=$requestedCursor"
        )
        pendingOptimisticRenderText = null
        insertPlainTextRangeInRust(
            mutation.scalarFrom,
            mutation.scalarTo,
            mutation.replacementText,
            requestedCursorScalar = requestedCursor
        )
        return true
    }

    private fun committedTokenRangeAroundMutation(
        currentText: String,
        replacementStartUtf16: Int,
        replacementEndUtf16: Int
    ): Pair<Int, Int>? {
        if (currentText.isEmpty()) return null
        val start = replacementStartUtf16.coerceIn(0, currentText.length)
        val end = replacementEndUtf16.coerceIn(start, currentText.length)
        val probe = when {
            start < end -> start
            start < currentText.length -> start
            start > 0 -> Character.offsetByCodePoints(currentText, start, -1)
            else -> return null
        }
        val tokenRange = missingOldTextCorrectionTokenRange(currentText, probe) ?: return null
        return if (start < end) {
            tokenRange.takeIf { it.first <= start && it.second >= end }
        } else {
            tokenRange.takeIf { start >= it.first && start <= it.second }
        }
    }

    private fun scheduleDeferredRustUpdateApplication() {
        val pendingUpdateJSON = deferredRustUpdateJSON ?: return
        val pendingLineBoundaryRefreshSource = deferredRustUpdateLineBoundaryRefreshSource
        val generation = ++deferredRustUpdateGeneration
        recordImeTraceForTesting(
            "rustUpdateDeferredScheduled",
            "generation=$generation jsonLength=${pendingUpdateJSON.length}"
        )
        Handler(Looper.getMainLooper()).post {
            if (generation != deferredRustUpdateGeneration) {
                recordImeTraceForTesting(
                    "rustUpdateDeferredSkip",
                    "reason=generation generation=$generation current=$deferredRustUpdateGeneration"
                )
                return@post
            }
            if (deferredRustUpdateJSON != pendingUpdateJSON) {
                recordImeTraceForTesting("rustUpdateDeferredSkip", "reason=replaced generation=$generation")
                return@post
            }
            deferredRustUpdateJSON = null
            deferredRustUpdateLineBoundaryRefreshSource = null
            recordImeTraceForTesting(
                "rustUpdateApply",
                "mode=deferred generation=$generation jsonLength=${pendingUpdateJSON.length}"
            )
            if (applyUpdateJSON(pendingUpdateJSON)) {
                pendingLineBoundaryRefreshSource?.let(::scheduleLineBoundaryInputRefreshForEditor)
            }
        }
    }

    private fun cancelDeferredRustUpdateApplication() {
        if (deferredRustUpdateJSON == null) return
        recordImeTraceForTesting(
            "rustUpdateDeferredCancel",
            "generation=$deferredRustUpdateGeneration"
        )
        deferredRustUpdateJSON = null
        deferredRustUpdateLineBoundaryRefreshSource = null
        deferredRustUpdateGeneration += 1L
    }

    /**
     * Insert text at a scalar position via the Rust editor.
     */
    private fun insertTextInRust(text: String, atScalarPos: Int) {
        if (!hasLiveEditor()) return
        onInsertTextInRustForTesting?.let { callback ->
            callback(text, atScalarPos)
            return
        }
        v2Driver?.let { driver ->
            driver.insertText(text, atScalarPos)?.let { applyRustUpdateJSON(it) }
        }
    }

    private fun replaceTextRangeInRust(scalarFrom: Int, scalarTo: Int, text: String) {
        if (!hasLiveEditor()) return
        onReplaceTextInRustForTesting?.let { callback ->
            callback(scalarFrom, scalarTo, text)
            return
        }
        v2Driver?.let { driver ->
            driver.replaceTextRange(scalarFrom, scalarTo, text)?.let { applyRustUpdateJSON(it) }
        }
    }

    private fun insertPlainTextRangeInRust(
        scalarFrom: Int,
        scalarTo: Int,
        text: String,
        requestedCursorScalar: Int? = null
    ) {
        if (!hasLiveEditor()) return
        recordImeTraceForTesting(
            "rustPlainTextRoute",
            "range=$scalarFrom..$scalarTo textLength=${text.length} requestedCursor=$requestedCursorScalar"
        )
        if (text.isEmpty()) {
            if (scalarFrom != scalarTo) {
                deleteRangeInRust(scalarFrom, scalarTo)
            }
            applyRequestedCursorScalar(requestedCursorScalar)
            return
        }
        if (text.indexOf('\n') >= 0 || text.indexOf('\r') >= 0) {
            val docJson = plainTextDocumentFragmentJson(text)
            onInsertContentJsonAtSelectionScalarForTesting?.let { callback ->
                callback(scalarFrom, scalarTo, docJson)
                applyRequestedCursorScalar(requestedCursorScalar)
                return
            }
            v2Driver?.let { driver ->
                driver.insertContentJsonAtSelection(docJson, scalarFrom, scalarTo)?.let { applyRustUpdateJSON(it) }
            }
            applyRequestedCursorScalar(requestedCursorScalar)
            return
        }

        if (scalarFrom != scalarTo) {
            replaceTextRangeInRust(scalarFrom, scalarTo, text)
        } else {
            insertTextInRust(text, scalarFrom)
        }
        applyRequestedCursorScalar(requestedCursorScalar)
    }

    private fun requestedCursorScalar(
        scalarFrom: Int,
        scalarTo: Int,
        currentText: String,
        insertedText: String,
        newCursorPosition: Int
    ): Int? {
        if (newCursorPosition == 1) return null
        val insertedScalarLength = insertedText.codePointCount(0, insertedText.length)
        val currentScalarLength = currentText.codePointCount(0, currentText.length)
        val nextScalarLength =
            (currentScalarLength - (scalarTo - scalarFrom) + insertedScalarLength).coerceAtLeast(0)
        val requested = if (newCursorPosition > 0) {
            scalarFrom + insertedScalarLength + newCursorPosition - 1
        } else {
            scalarFrom + newCursorPosition
        }
        return requested.coerceIn(0, nextScalarLength)
    }

    private fun applyRequestedCursorScalar(requestedCursorScalar: Int?) {
        val requested = requestedCursorScalar ?: return
        if (!hasLiveEditor()) return
        val currentText = text?.toString().orEmpty()
        val safeScalar = requested.coerceAtLeast(0)
        val cursorDriver = v2Driver
        if (cursorDriver != null) {
            cursorDriver.syncSelectionQuiet(safeScalar, safeScalar)
        } else {
            onSetSelectionScalarInRustForTesting?.let { callback ->
                callback(safeScalar, safeScalar)
            }
        }
        val localScalar = safeScalar.coerceIn(0, currentText.codePointCount(0, currentText.length))
        val safeUtf16 = PositionBridge.scalarToUtf16(localScalar, currentText)
            .coerceIn(0, currentText.length)
        if (selectionStart != safeUtf16 || selectionEnd != safeUtf16) {
            setSelection(safeUtf16, safeUtf16)
        }
    }

    private fun plainTextDocumentFragmentJson(text: String): String {
        val normalizedText = text.replace("\r\n", "\n").replace('\r', '\n')
        val content = org.json.JSONArray()
        for (line in normalizedText.split('\n')) {
            val paragraph = org.json.JSONObject().put("type", "paragraph")
            if (line.isNotEmpty()) {
                paragraph.put(
                    "content",
                    org.json.JSONArray().put(
                        org.json.JSONObject()
                            .put("type", "text")
                            .put("text", line)
                    )
                )
            }
            content.put(paragraph)
        }
        return org.json.JSONObject()
            .put("type", "doc")
            .put("content", content)
            .toString()
    }

    private fun nativeTextMutationFromAuthorizedDiff(currentText: String): NativeTextMutation? {
        val authorizedText = lastAuthorizedText
        if (currentText == authorizedText) return null

        var prefix = 0
        val sharedLength = minOf(authorizedText.length, currentText.length)
        while (
            prefix < sharedLength &&
            authorizedText[prefix] == currentText[prefix]
        ) {
            prefix++
        }
        prefix = minOf(
            PositionBridge.snapToScalarBoundary(prefix, authorizedText, biasForward = false),
            PositionBridge.snapToScalarBoundary(prefix, currentText, biasForward = false)
        )

        var authorizedEnd = authorizedText.length
        var currentEnd = currentText.length
        while (
            authorizedEnd > prefix &&
            currentEnd > prefix &&
            authorizedText[authorizedEnd - 1] == currentText[currentEnd - 1]
        ) {
            authorizedEnd--
            currentEnd--
        }
        authorizedEnd = PositionBridge.snapToScalarBoundary(
            authorizedEnd,
            authorizedText,
            biasForward = true
        )
        currentEnd = PositionBridge.snapToScalarBoundary(
            currentEnd,
            currentText,
            biasForward = true
        )

        val replacementText = currentText.substring(prefix, currentEnd)
        val rawSelectionStart = selectionStart
        val rawSelectionEnd = selectionEnd
        val selectionAnchorUtf16 = rawSelectionStart
            .takeIf { it >= 0 }
            ?.let { PositionBridge.snapToScalarBoundary(it, currentText, biasForward = true) }
        val selectionHeadUtf16 = rawSelectionEnd
            .takeIf { it >= 0 }
            ?.let { PositionBridge.snapToScalarBoundary(it, currentText, biasForward = true) }
        return NativeTextMutation(
            scalarFrom = PositionBridge.utf16ToScalar(prefix, authorizedText),
            scalarTo = PositionBridge.utf16ToScalar(authorizedEnd, authorizedText),
            replacementText = replacementText,
            resultingText = currentText,
            replacementStartUtf16 = prefix,
            replacementEndUtf16 = currentEnd,
            selectionScalarAnchor = selectionAnchorUtf16?.let {
                PositionBridge.utf16ToScalar(it, currentText)
            },
            selectionScalarHead = selectionHeadUtf16?.let {
                PositionBridge.utf16ToScalar(it, currentText)
            }
        )
    }

    private fun shouldAdoptNativeTextMutation(
        mutation: NativeTextMutation,
        allowAfterBlur: Boolean = false
    ): Boolean {
        if (!isEditable) return false
        if (isNativeTextMutationAdoptionSuppressedForCurrentRevision()) return false
        if (!hasFocus()) {
            return allowAfterBlur &&
                canAdoptNativeTextMutationAfterBlur() &&
                shouldAdoptFinalNativeTextMutation(mutation)
        }
        return shouldAdoptFinalNativeTextMutation(mutation)
    }

    private fun shouldAdoptFinalNativeTextMutation(mutation: NativeTextMutation): Boolean {
        if (composingTextForEditor() != null) return false
        val trackedRange = compositionReplacementRange() ?: return true
        val authorizedText = lastAuthorizedText
        val trackedStart = PositionBridge.utf16ToScalar(trackedRange.first, authorizedText)
        val trackedEnd = PositionBridge.utf16ToScalar(trackedRange.second, authorizedText)
        if (trackedStart == trackedEnd) {
            return mutation.scalarFrom == trackedStart &&
                mutation.scalarTo == trackedStart &&
                mutation.replacementText.isNotEmpty()
        }
        if (mutation.scalarFrom == mutation.scalarTo) {
            return mutation.replacementText.isNotEmpty() &&
                mutation.scalarFrom >= trackedStart &&
                mutation.scalarFrom <= trackedEnd
        }
        return mutation.scalarFrom < trackedEnd && mutation.scalarTo > trackedStart
    }

    private fun drainNativeTextMutationIfNeeded(
        allowAfterBlur: Boolean,
        preserveInputConnectionForExternalUpdate: Boolean = false
    ): Boolean {
        if (editorId == 0L) return true
        if (discardTransientInputForDestroyedEditorIfNeeded()) return false
        val editable = text
        val currentText = editable?.toString() ?: ""
        if (currentText == lastAuthorizedText) return true

        val mutation = nativeTextMutationFromAuthorizedDiff(currentText)
        if (mutation != null && shouldAdoptNativeTextMutation(mutation, allowAfterBlur)) {
            commitNativeTextMutation(
                mutation,
                preserveInputConnectionForExternalUpdate = preserveInputConnectionForExternalUpdate
            )
            return true
        }
        recordImeTraceForTesting(
            "nativeMutationNoop",
            "reason=${if (mutation == null) "noDiffRange" else "notAdoptable"} allowAfterBlur=$allowAfterBlur currentLength=${currentText.length} authorizedLength=${lastAuthorizedText.length}"
        )
        return false
    }

    private fun beginNativeTextMutationAfterBlurWindow() {
        if (!hasLiveEditor()) {
            clearNativeTextMutationAfterBlurWindow()
            return
        }
        nativeTextMutationAfterBlurWindow = NativeTextMutationAfterBlurWindow(
            editorId = editorId,
            authorizedTextRevision = lastAuthorizedTextRevision,
            deadlineMs = SystemClock.uptimeMillis() + NATIVE_TEXT_MUTATION_AFTER_BLUR_WINDOW_MS
        )
    }

    private fun clearNativeTextMutationAfterBlurWindow() {
        nativeTextMutationAfterBlurWindow = null
    }

    private fun suppressNativeTextMutationAdoptionForCurrentRevision() {
        if (!hasLiveEditor()) {
            clearNativeTextMutationAdoptionSuppression()
            return
        }
        nativeTextMutationAdoptionSuppression = NativeTextMutationAdoptionSuppression(
            editorId = editorId,
            authorizedTextRevision = lastAuthorizedTextRevision
        )
    }

    private fun clearNativeTextMutationAdoptionSuppression() {
        nativeTextMutationAdoptionSuppression = null
    }

    private fun isNativeTextMutationAdoptionSuppressedForCurrentRevision(): Boolean {
        val suppression = nativeTextMutationAdoptionSuppression ?: return false
        if (
            suppression.editorId != editorId ||
            suppression.authorizedTextRevision != lastAuthorizedTextRevision
        ) {
            nativeTextMutationAdoptionSuppression = null
            return false
        }
        return true
    }

    private fun canAdoptNativeTextMutationAfterBlur(): Boolean {
        val window = nativeTextMutationAfterBlurWindow ?: return false
        val now = SystemClock.uptimeMillis()
        if (now > window.deadlineMs ||
            window.editorId != editorId ||
            window.authorizedTextRevision != lastAuthorizedTextRevision ||
            window.didAdoptMutation
        ) {
            nativeTextMutationAfterBlurWindow = null
            return false
        }
        return true
    }

    private fun commitNativeTextMutation(
        mutation: NativeTextMutation,
        preserveInputConnectionForExternalUpdate: Boolean = false
    ) {
        if (!hasLiveEditor()) return
        val startedAt = System.nanoTime()
        if ((text?.toString() ?: "") != mutation.resultingText) {
            recordImeTraceForTesting(
                "nativeMutationNoop",
                "reason=staleResult range=${mutation.scalarFrom}..${mutation.scalarTo} replacementLength=${mutation.replacementText.length}"
            )
            return
        }
        val shouldRestartInput = hasFocus()
        if (preserveInputConnectionForExternalUpdate) {
            clearInputStateForExternalReplacementPreservingConnection()
        } else {
            retireInputConnectionForEditor()
        }
        nativeTextMutationAfterBlurWindow?.didAdoptMutation = true
        clearNativeTextMutationAfterBlurWindow()

        recordImeTraceForTesting(
            "nativeMutationApply",
            "range=${mutation.scalarFrom}..${mutation.scalarTo} replacementLength=${mutation.replacementText.length} restartInput=$shouldRestartInput preserveInputConnection=$preserveInputConnectionForExternalUpdate"
        )
        if (mutation.replacementText.isEmpty()) {
            deleteRangeInRust(mutation.scalarFrom, mutation.scalarTo)
        } else {
            insertPlainTextRangeInRust(
                mutation.scalarFrom,
                mutation.scalarTo,
                mutation.replacementText
            )
        }
        restoreSelectionAfterNativeTextMutation(mutation)
        if (shouldRestartInput) {
            restartInputForEditor(
                if (preserveInputConnectionForExternalUpdate) "externalUpdatePreflight" else "explicit"
            )
        }
        recordImeTraceForTesting(
            "nativeMutationApplyDone",
            "totalUs=${nanosToMicros(System.nanoTime() - startedAt)} restartInput=$shouldRestartInput"
        )
    }

    private fun restoreSelectionAfterNativeTextMutation(mutation: NativeTextMutation) {
        val selectionScalarAnchor = mutation.selectionScalarAnchor ?: return
        val selectionScalarHead = mutation.selectionScalarHead ?: return
        val currentText = text?.toString() ?: return
        val anchorUtf16 = PositionBridge.scalarToUtf16(selectionScalarAnchor, currentText)
        val headUtf16 = PositionBridge.scalarToUtf16(selectionScalarHead, currentText)
        val length = currentText.length
        setSelection(anchorUtf16.coerceIn(0, length), headUtf16.coerceIn(0, length))
    }

    /**
     * Delete a scalar range via the Rust editor.
     *
     * @param scalarFrom Start scalar offset (inclusive).
     * @param scalarTo End scalar offset (exclusive).
     */
    private fun deleteRangeInRust(scalarFrom: Int, scalarTo: Int) {
        if (!hasLiveEditor()) return
        if (scalarFrom >= scalarTo) return
        onDeleteRangeInRustForTesting?.let { callback ->
            callback(scalarFrom, scalarTo)
            return
        }
        v2Driver?.let { driver ->
            driver.deleteScalarRange(scalarFrom, scalarTo)?.let { applyRustUpdateJSON(it) }
        }
    }

    private fun deleteBackwardAtSelectionScalarInRust(scalarAnchor: Int, scalarHead: Int) {
        if (!hasLiveEditor()) return
        onDeleteBackwardAtSelectionScalarInRustForTesting?.let { callback ->
            callback(scalarAnchor, scalarHead)
            return
        }
        v2Driver?.let { driver ->
            driver.deleteBackwardAtSelection(scalarAnchor, scalarHead)?.let { applyRustUpdateJSON(it) }
        }
    }

    private fun toggleTaskItemCheckedAtSelectionScalarInRust(scalarAnchor: Int, scalarHead: Int) {
        if (!hasLiveEditor()) return
        onToggleTaskItemCheckedAtSelectionScalarInRustForTesting?.let { callback ->
            callback(scalarAnchor, scalarHead)
            return
        }
        v2Driver?.let { driver ->
            driver.toggleTaskItemCheckedAtSelection(scalarAnchor, scalarHead)?.let { applyRustUpdateJSON(it) }
        }
    }

    /**
     * Split a block at a scalar position via the Rust editor.
     */
    private fun splitBlockInRust(atScalarPos: Int) {
        if (!hasLiveEditor()) return
        v2Driver?.let { driver ->
            driver.splitBlockAt(atScalarPos)?.let { result ->
                applyRustUpdateJSON(
                    result.updateJson,
                    lineBoundaryRefreshSource = if (result.committed) "splitBlock" else null,
                )
            }
        }
    }

    private fun deleteAndSplitInRust(scalarFrom: Int, scalarTo: Int) {
        if (!hasLiveEditor()) return
        onDeleteAndSplitScalarInRustForTesting?.let { callback ->
            callback(scalarFrom, scalarTo)
            return
        }
        v2Driver?.let { driver ->
            driver.deleteAndSplit(scalarFrom, scalarTo)?.let { result ->
                applyRustUpdateJSON(
                    result.updateJson,
                    lineBoundaryRefreshSource = if (result.committed) "deleteAndSplit" else null,
                )
            }
        }
    }

    internal fun currentScalarSelection(): Pair<Int, Int>? {
        val currentText = text?.toString() ?: return null
        return normalizedScalarSelectionRange(currentText)
    }

    internal fun cursorCapsModeForEditor(reqModes: Int, baseCapsMode: Int): Int {
        val sentenceCapsMode = InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        if ((reqModes and sentenceCapsMode) != sentenceCapsMode) return baseCapsMode
        if ((baseCapsMode and sentenceCapsMode) == sentenceCapsMode) return baseCapsMode
        if (!isCursorAtRenderedLineStartForSentenceCaps()) return baseCapsMode
        return baseCapsMode or sentenceCapsMode
    }

    internal fun textBeforeCursorForImeContextForEditor(n: Int, flags: Int): CharSequence? {
        if (n <= 0) return ""
        val content = text ?: return null
        val start = selectionStart
        val end = selectionEnd
        if (start < 0 || end < 0) return null
        val cursor = minOf(start, end).coerceIn(0, content.length)
        var effectiveCursor = cursor
        while (
            effectiveCursor > 0 &&
            content[effectiveCursor - 1] == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]
        ) {
            effectiveCursor -= 1
        }
        val contextStart = maxOf(0, effectiveCursor - n)
        val context = content.subSequence(contextStart, effectiveCursor)
        return if ((flags and InputConnection.GET_TEXT_WITH_STYLES) != 0) {
            context
        } else {
            context.toString()
        }
    }

    internal fun initialSurroundingTextForImeForEditor(): ImeInitialSurroundingText? {
        val rawText = text?.toString() ?: return null
        val placeholder = LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]
        if (rawText.indexOf(placeholder) < 0) return null
        val start = selectionStart
        val end = selectionEnd
        if (start < 0 || end < 0) return null
        val rawSelectionStart = start.coerceIn(0, rawText.length)
        val rawSelectionEnd = end.coerceIn(0, rawText.length)

        val sanitized = StringBuilder(rawText.length)
        var removedCount = 0
        var removedBeforeSelectionStart = 0
        var removedBeforeSelectionEnd = 0
        rawText.forEachIndexed { index, ch ->
            if (ch == placeholder) {
                removedCount += 1
                if (index < rawSelectionStart) removedBeforeSelectionStart += 1
                if (index < rawSelectionEnd) removedBeforeSelectionEnd += 1
            } else {
                sanitized.append(ch)
            }
        }

        return ImeInitialSurroundingText(
            text = sanitized.toString(),
            selectionStart = rawSelectionStart - removedBeforeSelectionStart,
            selectionEnd = rawSelectionEnd - removedBeforeSelectionEnd,
            originalSelectionStart = rawSelectionStart,
            originalSelectionEnd = rawSelectionEnd,
            removedPlaceholderCount = removedCount
        )
    }

    private fun isCursorAtRenderedLineStartForSentenceCaps(): Boolean {
        val currentText = text ?: return false
        val start = selectionStart
        val end = selectionEnd
        if (start < 0 || end < 0 || start != end) return false

        val cursor = end.coerceIn(0, currentText.length)
        return isRenderedLineStartForSentenceCaps(currentText, cursor)
    }

    private fun isRenderedLineStartForSentenceCaps(text: Spanned, cursor: Int): Boolean {
        val cursor = cursor.coerceIn(0, text.length)
        if (cursor == 0) return true

        val lineStart = lastRenderedLineBreakBefore(text, cursor) + 1
        var index = lineStart
        while (index < cursor && isIgnoredSentenceCapsLinePrefix(text[index])) {
            index += 1
        }
        if (index == cursor) return true

        val markerEnd = renderedListMarkerEnd(text, index, cursor) ?: return false
        index = markerEnd
        while (index < cursor && isIgnoredSentenceCapsLinePrefix(text[index])) {
            index += 1
        }
        return index == cursor
    }

    private fun isSamsungKeyboardActiveForEditor(): Boolean {
        val inputMethodId = Settings.Secure.getString(
            context.contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD
        ) ?: return false
        return inputMethodId.contains("samsung", ignoreCase = true) ||
            inputMethodId.contains("honeyboard", ignoreCase = true)
    }

    private fun lastRenderedLineBreakBefore(text: CharSequence, cursor: Int): Int {
        var index = cursor.coerceAtMost(text.length) - 1
        while (index >= 0) {
            when (text[index]) {
                '\n', '\r' -> return index
            }
            index -= 1
        }
        return -1
    }

    private fun isIgnoredSentenceCapsLinePrefix(ch: Char): Boolean =
        ch == ' ' ||
            ch == '\t' ||
            ch == '\u00A0' ||
            ch == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]

    private fun renderedListMarkerEnd(text: Spanned, start: Int, endExclusive: Int): Int? {
        if (start >= endExclusive) return null
        if (renderedTaskListMarkerEnd(text, start, endExclusive) != null) {
            return start + 2
        }
        if (text[start] == LayoutConstants.UNORDERED_LIST_BULLET[0]) {
            return start + 1
        }

        var index = start
        while (index < endExclusive && text[index].isDigit()) {
            index += 1
        }
        if (index == start || index >= endExclusive) return null
        return when (text[index]) {
            '.', ')' -> index + 1
            else -> null
        }
    }

    private fun renderedTaskListMarkerEnd(text: Spanned, start: Int, endExclusive: Int): Int? {
        if (start + 1 >= endExclusive) return null
        val marker = text[start]
        if (marker != LayoutConstants.TASK_LIST_MARKER_UNCHECKED[0] &&
            marker != LayoutConstants.TASK_LIST_MARKER_CHECKED[0]
        ) return null
        if (text[start + 1] != ' ') return null
        val isMarker = text.getSpans(start, start + 1, Annotation::class.java)
            .any { it.key == RenderBridge.NATIVE_TASK_LIST_MARKER_ANNOTATION }
        return if (isMarker) start + 2 else null
    }

    private fun normalizedUtf16SelectionRange(currentText: String): Pair<Int, Int>? {
        val start = selectionStart
        val end = selectionEnd
        if (start < 0 || end < 0) return null
        val clampedStart = start.coerceIn(0, currentText.length)
        val clampedEnd = end.coerceIn(0, currentText.length)
        return minOf(clampedStart, clampedEnd) to maxOf(clampedStart, clampedEnd)
    }

    private fun normalizedUtf16SelectionRange(): Pair<Int, Int>? {
        val currentText = text?.toString() ?: return null
        return normalizedUtf16SelectionRange(currentText)
    }

    private fun normalizedScalarSelectionRange(currentText: String): Pair<Int, Int>? {
        val (start, end) = normalizedUtf16SelectionRange(currentText) ?: return null
        val (snappedStart, snappedEnd) = if (start == end) {
            val snapped = PositionBridge.snapToScalarBoundary(
                start,
                currentText,
                biasForward = true
            )
            snapped to snapped
        } else {
            PositionBridge.snapRangeToScalarBoundaries(start, end, currentText)
        }
        return PositionBridge.utf16ToScalar(snappedStart, currentText) to
            PositionBridge.utf16ToScalar(snappedEnd, currentText)
    }

    private fun rawScalarSelection(currentText: String): Pair<Int, Int>? {
        val anchor = selectionStart
        val head = selectionEnd
        if (anchor < 0 || head < 0) return null
        val clampedAnchor = anchor.coerceIn(0, currentText.length)
        val clampedHead = head.coerceIn(0, currentText.length)
        if (clampedAnchor == clampedHead) {
            val snapped = PositionBridge.snapToScalarBoundary(
                clampedAnchor,
                currentText,
                biasForward = true
            )
            val scalar = PositionBridge.utf16ToScalar(snapped, currentText)
            return scalar to scalar
        }
        val (rangeStart, rangeEnd) = PositionBridge.snapRangeToScalarBoundaries(
            minOf(clampedAnchor, clampedHead),
            maxOf(clampedAnchor, clampedHead),
            currentText
        )
        val snappedAnchor = if (clampedAnchor < clampedHead) rangeStart else rangeEnd
        val snappedHead = if (clampedAnchor < clampedHead) rangeEnd else rangeStart
        return PositionBridge.utf16ToScalar(snappedAnchor, currentText) to
            PositionBridge.utf16ToScalar(snappedHead, currentText)
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
        val docPos = v2Driver?.docPositionForScalar(scalarPos) ?: scalarPos
        val line = textLayout.getLineForOffset(spanStart.coerceAtMost(maxOf(spannable.length - 1, 0)))
        val rect = resolvedImageRect(textLayout, imageSpan, spanStart, spanEnd)
        return SelectedImageGeometry(
            docPos = docPos,
            rect = rect
        )
    }

    fun resizeImageAtDocPos(docPos: Int, widthPx: Float, heightPx: Float) {
        if (!hasLiveEditor()) return
        val density = resources.displayMetrics.density
        val widthDp = maxOf(48, (widthPx / density).roundToInt())
        val heightDp = maxOf(48, (heightPx / density).roundToInt())
        v2Driver?.let { driver ->
            driver.resizeImageAtDocPos(docPos, widthDp, heightDp)?.let { applyUpdateJSON(it) }
        }
    }

    private fun isSelectionInsideList(): Boolean {
        if (!hasLiveEditor()) return false

        return try {
            val stateJson = v2Driver?.currentStateJson() ?: return false
            val state = org.json.JSONObject(stateJson)
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
        if (!hasLiveEditor()) return
        syncCurrentSelectionToRust()
        onInsertContentHtmlInRustForTesting?.let { callback ->
            callback(html)
            return
        }
        v2Driver?.let { driver ->
            val selection = currentScalarSelection()
            val update = if (selection != null) {
                driver.insertContentHtmlAtSelection(html, selection.first, selection.second)
            } else {
                null
            }
            update?.let { applyUpdateJSON(it) }
        }
    }

    /**
     * Paste plain text through Rust.
     */
    private fun pastePlainText(text: String) {
        val (scalarStart, scalarEnd) = currentScalarSelection() ?: return
        insertPlainTextRangeInRust(scalarStart, scalarEnd, text)
    }

    // ── Applying Rust State ─────────────────────────────────────────────

    private fun parseRenderPatch(raw: org.json.JSONObject?): ParsedRenderPatch? {
        if (raw == null) return null
        val renderBlocks = raw.optJSONArray("renderBlocks") ?: return null
        return ParsedRenderPatch(
            startIndex = raw.optInt("startIndex", -1),
            deleteCount = raw.optInt("deleteCount", -1),
            renderBlocks = renderBlocks
        ).takeIf { it.startIndex >= 0 && it.deleteCount >= 0 }
    }

    private fun hasTopLevelChildMetadata(content: Spanned): Boolean =
        content.getSpans(0, content.length, Annotation::class.java).any {
            it.key == RenderBridge.NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION
        }

    private fun firstCharacterOffsetForTopLevelChildIndex(content: Spanned, index: Int): Int? {
        val targetValue = index.toString()
        return content
            .getSpans(0, content.length, Annotation::class.java)
            .asSequence()
            .filter { it.key == RenderBridge.NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION && it.value == targetValue }
            .mapNotNull { span ->
                val spanStart = content.getSpanStart(span)
                val spanEnd = content.getSpanEnd(span)
                if (spanStart < 0 || spanEnd <= spanStart) {
                    null
                } else {
                    var candidate = spanStart
                    while (candidate < spanEnd && candidate < content.length) {
                        when (content[candidate]) {
                            '\n', '\r' -> candidate += 1
                            else -> return@mapNotNull candidate
                        }
                    }
                    null
                }
            }
            .minOrNull()
    }

    private fun replacementRangeForRenderPatch(
        content: Spanned,
        startIndex: Int,
        deleteCount: Int
    ): RenderReplaceRange? {
        val start = firstCharacterOffsetForTopLevelChildIndex(content, startIndex)
            ?: if (deleteCount == 0) content.length else return null
        val endExclusive = firstCharacterOffsetForTopLevelChildIndex(content, startIndex + deleteCount)
            ?: content.length
        if (start > endExclusive) return null
        return RenderReplaceRange(start = start, endExclusive = endExclusive)
    }

    private fun spannedRangeContainsImageSpan(content: Spanned, start: Int, endExclusive: Int): Boolean {
        if (start >= endExclusive) return false
        return content.getSpans(start, endExclusive, BlockImageSpan::class.java).isNotEmpty()
    }

    private fun spannedContainsImageSpan(content: Spanned): Boolean =
        spannedRangeContainsImageSpan(content, 0, content.length)

    private fun applyRenderedSpannable(
        spannable: CharSequence,
        replaceRange: RenderReplaceRange? = null,
        usedPatch: Boolean,
        preserveInputConnectionForExternalUpdate: Boolean = false
    ) {
        val startedAt = System.nanoTime()
        val previousScrollX = scrollX
        val previousScrollY = scrollY
        val hadCompositionTracking = hasCompositionTrackingForEditor()
        var shouldRestartInput = false
        val mode = if (replaceRange != null) "replace" else "setText"
        isApplyingRustState = true
        beginBatchEdit()
        try {
            if (replaceRange != null) {
                editableText.replace(replaceRange.start, replaceRange.endExclusive, spannable)
            } else {
                setText(spannable)
            }
            lastAuthorizedText = text?.toString().orEmpty()
            lastAuthorizedRenderedText = text?.let { SpannableStringBuilder(it) }
            lastAuthorizedTextRevision += 1L
            clearNativeTextMutationAdoptionSuppression()
            if (hadCompositionTracking && preserveInputConnectionForExternalUpdate) {
                clearInputStateForExternalReplacementPreservingConnection()
                shouldRestartInput = true
            } else if (hadCompositionTracking) {
                retireInputConnectionForEditor()
                shouldRestartInput = true
            } else {
                clearCompositionTrackingForEditor()
            }
            lastRenderAppliedPatchForTesting = usedPatch
            clearNativeTextMutationAfterBlurWindow()
        } finally {
            endBatchEdit()
            isApplyingRustState = false
        }
        recordImeTraceForTesting(
            "applyRenderedSpannable",
            "mode=$mode usedPatch=$usedPatch incomingLength=${spannable.length} replace=${replaceRange?.start}..${replaceRange?.endExclusive} hadComposition=$hadCompositionTracking restartInput=$shouldRestartInput applyUs=${nanosToMicros(System.nanoTime() - startedAt)} scroll=$previousScrollX,$previousScrollY->$scrollX,$scrollY layout=${layout != null}"
        )
        invalidateRenderedContent()
        restartInputAfterCompositionInvalidationIfNeeded(shouldRestartInput)
    }

    private fun invalidateRenderedContent() {
        invalidate()
        postInvalidateOnAnimation()
    }

    private fun authorizeVisibleTextForMatchedOptimisticRender(spannable: CharSequence) {
        val startedAt = System.nanoTime()
        val visibleText = text?.toString().orEmpty()
        lastAuthorizedText = visibleText
        lastAuthorizedRenderedText = text?.let { SpannableStringBuilder(it) }
            ?: SpannableStringBuilder(spannable)
        lastAuthorizedTextRevision += 1L
        clearNativeTextMutationAdoptionSuppression()
        clearCompositionTrackingForEditor()
        lastRenderAppliedPatchForTesting = false
        clearNativeTextMutationAfterBlurWindow()
        recordImeTraceForTesting(
            "reuseOptimisticVisibleTextRender",
            "textLength=${visibleText.length} applyUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    private fun buildPatchedSpannable(patch: ParsedRenderPatch): android.text.SpannableStringBuilder =
        RenderBridge.buildSpannableFromBlocks(
            patch.renderBlocks,
            startIndex = patch.startIndex,
            baseFontSize = baseFontSize,
            textColor = baseTextColor,
            theme = theme,
            density = resources.displayMetrics.density,
            hostView = this
        )

    private fun cloneJsonArray(array: org.json.JSONArray): org.json.JSONArray =
        org.json.JSONArray().also { clone ->
            for (index in 0 until array.length()) {
                clone.put(array.opt(index))
            }
        }

    private fun normalizedJsonValue(value: Any?): Any? =
        if (value === org.json.JSONObject.NULL) null else value

    private fun jsonValuesEqual(left: Any?, right: Any?): Boolean {
        val normalizedLeft = normalizedJsonValue(left)
        val normalizedRight = normalizedJsonValue(right)
        if (normalizedLeft === normalizedRight) return true
        if (normalizedLeft == null || normalizedRight == null) return false

        if (normalizedLeft is org.json.JSONArray && normalizedRight is org.json.JSONArray) {
            if (normalizedLeft.length() != normalizedRight.length()) return false
            for (index in 0 until normalizedLeft.length()) {
                if (!jsonValuesEqual(normalizedLeft.opt(index), normalizedRight.opt(index))) {
                    return false
                }
            }
            return true
        }

        if (normalizedLeft is org.json.JSONObject && normalizedRight is org.json.JSONObject) {
            if (normalizedLeft.length() != normalizedRight.length()) return false
            val keys = normalizedLeft.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                if (!normalizedRight.has(key)) return false
                if (!jsonValuesEqual(normalizedLeft.opt(key), normalizedRight.opt(key))) {
                    return false
                }
            }
            return true
        }

        if (normalizedLeft is Number && normalizedRight is Number) {
            return normalizedLeft.toDouble() == normalizedRight.toDouble()
        }

        return normalizedLeft == normalizedRight
    }

    private fun renderBlocksEqual(
        current: org.json.JSONArray,
        updated: org.json.JSONArray
    ): Boolean {
        if (current.length() != updated.length()) return false
        for (index in 0 until current.length()) {
            if (!jsonValuesEqual(current.opt(index), updated.opt(index))) {
                return false
            }
        }
        return true
    }

    private fun mergeRenderBlocks(
        current: org.json.JSONArray,
        patch: ParsedRenderPatch
    ): org.json.JSONArray? {
        if (
            patch.startIndex < 0 ||
            patch.deleteCount < 0 ||
            patch.startIndex > current.length() ||
            patch.startIndex + patch.deleteCount > current.length()
        ) {
            return null
        }

        return org.json.JSONArray().also { merged ->
            for (index in 0 until patch.startIndex) {
                merged.put(current.opt(index))
            }
            for (index in 0 until patch.renderBlocks.length()) {
                merged.put(patch.renderBlocks.opt(index))
            }
            for (index in (patch.startIndex + patch.deleteCount) until current.length()) {
                merged.put(current.opt(index))
            }
        }
    }

    private fun applyRenderPatchIfPossible(patch: ParsedRenderPatch): PatchApplyTrace {
        val eligibilityStartedAt = System.nanoTime()
        val content = text as? Spanned ?: return PatchApplyTrace(
            applied = false,
            eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
            buildRenderNanos = 0L,
            applyRenderNanos = 0L
        )
        if (!hasTopLevelChildMetadata(content)) {
            return PatchApplyTrace(
                applied = false,
                eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
                buildRenderNanos = 0L,
                applyRenderNanos = 0L
            )
        }

        val replaceRange = replacementRangeForRenderPatch(content, patch.startIndex, patch.deleteCount)
            ?: return PatchApplyTrace(
                applied = false,
                eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
                buildRenderNanos = 0L,
                applyRenderNanos = 0L
            )
        if (spannedRangeContainsImageSpan(content, replaceRange.start, replaceRange.endExclusive)) {
            return PatchApplyTrace(
                applied = false,
                eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
                buildRenderNanos = 0L,
                applyRenderNanos = 0L
            )
        }
        val eligibilityNanos = System.nanoTime() - eligibilityStartedAt

        val buildStartedAt = System.nanoTime()
        val patchedSpannable = buildPatchedSpannable(patch)
        val buildRenderNanos = System.nanoTime() - buildStartedAt
        if (spannedContainsImageSpan(patchedSpannable)) {
            return PatchApplyTrace(
                applied = false,
                eligibilityNanos = eligibilityNanos,
                buildRenderNanos = buildRenderNanos,
                applyRenderNanos = 0L
            )
        }

        val applyStartedAt = System.nanoTime()
        applyRenderedSpannable(
            spannable = patchedSpannable,
            replaceRange = replaceRange,
            usedPatch = true
        )
        return PatchApplyTrace(
            applied = true,
            eligibilityNanos = eligibilityNanos,
            buildRenderNanos = buildRenderNanos,
            applyRenderNanos = System.nanoTime() - applyStartedAt
        )
    }

    /**
     * Apply a full render update from Rust to the EditText.
     *
     * Parses the update JSON, converts render elements to [android.text.SpannableStringBuilder]
     * via [RenderBridge], and replaces the EditText's content.
     *
     * @param updateJSON The JSON string from an [EditorV2Driver] transaction result.
     */
    fun applyUpdateJSON(
        updateJSON: String,
        notifyListener: Boolean = true,
        refreshInputConnectionForExternalUpdate: Boolean = false
    ): Boolean {
        throwOnNextApplyUpdateForTesting?.let { error ->
            throwOnNextApplyUpdateForTesting = null
            throw error
        }
        val totalStartedAt = System.nanoTime()
        val previousVisibleText = text?.toString().orEmpty()
        val parseStartedAt = totalStartedAt
        val update = try {
            org.json.JSONObject(updateJSON)
        } catch (error: Exception) {
            recordImeTraceForTesting(
                "applyUpdateJSONNoop",
                "reason=parseError jsonLength=${updateJSON.length} error=${error.javaClass.simpleName}"
            )
            return false
        }
        cancelDeferredRustUpdateApplication()
        val parseNanos = System.nanoTime() - parseStartedAt

        // The core is the authority on empty state; adopt it before anything
        // reconsiders the placeholder.
        setCoreReportedDocumentIsEmpty(
            if (update.has("documentIsEmpty")) update.optBoolean("documentIsEmpty") else null
        )

        val resolveRenderBlocksStartedAt = System.nanoTime()
        val renderElements = update.optJSONArray("renderElements")
        val renderBlocks = update.optJSONArray("renderBlocks")
        val renderPatch = parseRenderPatch(update.optJSONObject("renderPatch"))
        val resolvedRenderBlocks = renderBlocks
            ?: renderPatch?.let { patch ->
                currentRenderBlocksJson?.let { mergeRenderBlocks(it, patch) }
            }
        val resolveRenderBlocksNanos = System.nanoTime() - resolveRenderBlocksStartedAt
        val shouldSkipRender = !refreshInputConnectionForExternalUpdate &&
            resolvedRenderBlocks != null &&
            currentRenderBlocksJson?.let { current ->
                renderBlocksEqual(current, resolvedRenderBlocks)
            } == true &&
            text?.toString() == lastAuthorizedText &&
            lastAppliedRenderAppearanceRevision == renderAppearanceRevision
        val previousScrollX = scrollX
        val previousScrollY = scrollY

        explicitSelectedImageRange = null
        val buildRenderNanos: Long
        val applyRenderNanos: Long
        if (shouldSkipRender) {
            pendingOptimisticRenderText = null
            lastRenderAppliedPatchForTesting = false
            currentRenderBlocksJson = resolvedRenderBlocks?.let(::cloneJsonArray)
            clearNativeTextMutationAdoptionSuppression()
            clearNativeTextMutationAfterBlurWindow()
            buildRenderNanos = 0L
            applyRenderNanos = 0L
        } else {
            cancelPendingImageLoads()
            // Android's Editable.replace(...) path benchmarks substantially slower than
            // rebuilding from merged render blocks, so patch payloads are treated as a
            // transport optimization only. We still resolve the merged block state above,
            // then apply it through the faster full-text path here.
            val buildStartedAt = System.nanoTime()
            val fullSpannable = if (resolvedRenderBlocks != null) {
                RenderBridge.buildSpannableFromBlocks(
                    resolvedRenderBlocks,
                    baseFontSize = baseFontSize,
                    textColor = baseTextColor,
                    theme = theme,
                    density = resources.displayMetrics.density,
                    hostView = this
                )
            } else if (renderElements != null) {
                RenderBridge.buildSpannableFromArray(
                    renderElements,
                    baseFontSize,
                    baseTextColor,
                    theme,
                    resources.displayMetrics.density,
                    this
                )
            } else {
                recordImeTraceForTesting(
                    "applyUpdateJSONNoop",
                    "reason=noRenderPayload jsonLength=${updateJSON.length}"
                )
                return false
            }
            buildRenderNanos = System.nanoTime() - buildStartedAt
            currentRenderBlocksJson = resolvedRenderBlocks?.let(::cloneJsonArray)
            val applyStartedAt = System.nanoTime()
            val optimisticText = pendingOptimisticRenderText
            val canReuseOptimisticVisibleText =
                    optimisticText != null &&
                    text?.toString() == optimisticText &&
                    fullSpannable.toString() == optimisticText &&
                    !spannedContainsImageSpan(fullSpannable)
            if (canReuseOptimisticVisibleText) {
                authorizeVisibleTextForMatchedOptimisticRender(fullSpannable)
            } else {
                applyRenderedSpannable(
                    fullSpannable,
                    usedPatch = false,
                    preserveInputConnectionForExternalUpdate = refreshInputConnectionForExternalUpdate
                )
            }
            pendingOptimisticRenderText = null
            applyRenderNanos = System.nanoTime() - applyStartedAt
            lastAppliedRenderAppearanceRevision = renderAppearanceRevision
        }

        // Apply the selection from the update.
        val selectionStartedAt = System.nanoTime()
        val selection = update.optJSONObject("selection")
        if (selection != null) {
            applySelectionFromJSON(selection)
        }
        val selectionNanos = System.nanoTime() - selectionStartedAt

        val postApplyStartedAt = System.nanoTime()
        if (notifyListener) {
            editorListener?.onEditorUpdate(updateJSON)
        }
        onSelectionOrContentMayChange?.invoke()
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            requestLayout()
        } else {
            preserveScrollPosition(previousScrollX, previousScrollY)
        }
        refreshInputConnectionAfterExternalTextReplacementIfNeeded(
            enabled = refreshInputConnectionForExternalUpdate,
            previousVisibleText = previousVisibleText
        )
        val postApplyNanos = System.nanoTime() - postApplyStartedAt

        val totalNanos = System.nanoTime() - totalStartedAt
        recordImeTraceForTesting(
            "applyUpdateJSON",
            "notify=$notifyListener skippedRender=$shouldSkipRender attemptedPatch=${renderPatch != null} jsonLength=${updateJSON.length} parseUs=${nanosToMicros(parseNanos)} resolveUs=${nanosToMicros(resolveRenderBlocksNanos)} buildUs=${nanosToMicros(buildRenderNanos)} applyUs=${nanosToMicros(applyRenderNanos)} selectionUs=${nanosToMicros(selectionNanos)} postUs=${nanosToMicros(postApplyNanos)} totalUs=${nanosToMicros(totalNanos)}"
        )

        if (captureApplyUpdateTraceForTesting) {
            lastApplyUpdateTraceForTesting = ApplyUpdateTrace(
                attemptedPatch = renderPatch != null,
                usedPatch = false,
                skippedRender = shouldSkipRender,
                parseNanos = parseNanos,
                resolveRenderBlocksNanos = resolveRenderBlocksNanos,
                patchEligibilityNanos = 0L,
                buildRenderNanos = buildRenderNanos,
                applyRenderNanos = applyRenderNanos,
                selectionNanos = selectionNanos,
                postApplyNanos = postApplyNanos,
                totalNanos = totalNanos
            )
        }
        return !shouldSkipRender
    }

    private fun refreshInputConnectionAfterExternalTextReplacementIfNeeded(
        enabled: Boolean,
        previousVisibleText: String
    ) {
        if (!enabled || !hasFocus()) return
        val currentVisibleText = text?.toString().orEmpty()
        if (currentVisibleText == previousVisibleText) return
        clearInputStateForExternalReplacementPreservingConnection()
        restartInputForEditor("externalUpdate")
    }

    private fun clearInputStateForExternalReplacementPreservingConnection() {
        activeInputConnection?.clearCompositionTrackingForEditor()
        clearCompositionTrackingForEditor()
        clearCompositionInvalidationForEditor()
        clearNativeComposingSpans()
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
        cancelPendingImageLoads()
        restartImageLoadsOnAttach = false
        val startedAt = System.nanoTime()
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
        currentRenderBlocksJson = null
        pendingOptimisticRenderText = null
        applyRenderedSpannable(spannable, usedPatch = false)
        onSelectionOrContentMayChange?.invoke()
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            requestLayout()
        } else {
            preserveScrollPosition(previousScrollX, previousScrollY)
        }
        recordImeTraceForTesting(
            "applyRenderJSON",
            "jsonLength=${renderJSON.length} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    private fun textOffsetHitAt(x: Float, y: Float): Pair<Spanned, Int>? {
        val spannable = text as? Spanned ?: return null
        val layout = layout ?: return null
        if (spannable.isEmpty()) return null

        val localX = x - totalPaddingLeft + scrollX
        val localY = y - totalPaddingTop + scrollY
        if (localY < 0f || localY > layout.height.toFloat()) {
            return null
        }

        val line = layout.getLineForVertical(localY.toInt())
        val lineLeft = layout.getLineLeft(line)
        val lineRight = layout.getLineRight(line)
        if (localX < lineLeft || localX > lineRight) {
            return null
        }

        val offset = layout.getOffsetForHorizontal(line, localX)
            .coerceIn(0, maxOf(spannable.length - 1, 0))
        return spannable to offset
    }

    fun mentionHitAt(x: Float, y: Float): MentionHit? {
        val (spannable, offset) = textOffsetHitAt(x, y) ?: return null
        val annotations = spannable.getSpans(
            offset,
            (offset + 1).coerceAtMost(spannable.length),
            Annotation::class.java
        )
        val mentionAnnotation = annotations.firstOrNull {
            it.key == "nativeVoidNodeType" && it.value == "mention"
        } ?: return null
        val docPos = annotations.firstOrNull { it.key == "nativeDocPos" }
            ?.value
            ?.toIntOrNull() ?: return null
        val start = spannable.getSpanStart(mentionAnnotation)
        val end = spannable.getSpanEnd(mentionAnnotation)
        if (start < 0 || end <= start) {
            return null
        }

        return MentionHit(
            docPos = docPos,
            label = spannable.subSequence(start, end).toString()
        )
    }

    fun linkHitAt(x: Float, y: Float): LinkHit? {
        val (spannable, offset) = textOffsetHitAt(x, y) ?: return null
        val annotations = spannable.getSpans(
            offset,
            (offset + 1).coerceAtMost(spannable.length),
            Annotation::class.java
        )
        val linkAnnotation = annotations.firstOrNull {
            it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION && it.value.isNotBlank()
        } ?: return null
        val start = spannable.getSpanStart(linkAnnotation)
        val end = spannable.getSpanEnd(linkAnnotation)
        if (start < 0 || end <= start) {
            return null
        }

        return LinkHit(
            href = linkAnnotation.value,
            text = spannable.subSequence(start, end).toString()
        )
    }

    internal fun accessibleAnnotations(): List<AccessibleAnnotation> {
        val spannable = text as? Spanned ?: return emptyList()
        val textLayout = layout ?: return emptyList()
        val annotations = spannable.getSpans(0, spannable.length, Annotation::class.java)
        val results = mutableListOf<Pair<Int, AccessibleAnnotation>>()
        annotations.forEach { annotation ->
            val start = spannable.getSpanStart(annotation)
            val end = spannable.getSpanEnd(annotation)
            if (start < 0 || end <= start) return@forEach
            val label = spannable.subSequence(start, end).toString()
            val target = when {
                annotation.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION &&
                    annotation.value.isNotBlank() -> AccessibleAnnotationTarget.Link(
                    href = annotation.value,
                    text = label
                )
                annotation.key == "nativeVoidNodeType" && annotation.value == "mention" -> {
                    val docPos = annotations.firstOrNull { candidate ->
                        candidate.key == "nativeDocPos" &&
                            spannable.getSpanStart(candidate) == start &&
                            spannable.getSpanEnd(candidate) == end
                    }?.value?.toIntOrNull() ?: return@forEach
                    AccessibleAnnotationTarget.Mention(docPos, label)
                }
                else -> return@forEach
            }
            val role = when (target) {
                is AccessibleAnnotationTarget.Link -> "link"
                is AccessibleAnnotationTarget.Mention -> "mention"
            }
            results += start to AccessibleAnnotation(
                target = target,
                label = label,
                role = role,
                bounds = annotationBounds(textLayout, start, end),
                annotation = annotation,
                start = start,
                end = end
            )
        }
        return results.sortedBy { it.first }.map { it.second }
    }

    internal fun interactiveAnnotationHitAt(x: Float, y: Float): InteractiveAnnotationHit? {
        val (spannable, offset) = textOffsetHitAt(x, y) ?: return null
        val annotations = spannable.getSpans(
            offset,
            (offset + 1).coerceAtMost(spannable.length),
            Annotation::class.java
        )
        val mentionAnnotation = annotations.firstOrNull {
            it.key == "nativeVoidNodeType" && it.value == "mention"
        }
        val annotation = mentionAnnotation ?: annotations.firstOrNull {
            it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION && it.value.isNotBlank()
        } ?: return null
        val start = spannable.getSpanStart(annotation)
        val end = spannable.getSpanEnd(annotation)
        if (start < 0 || end <= start) return null
        val label = spannable.subSequence(start, end).toString()
        val target = if (annotation === mentionAnnotation) {
            val docPos = annotations.firstOrNull {
                it.key == "nativeDocPos" &&
                    spannable.getSpanStart(it) == start &&
                    spannable.getSpanEnd(it) == end
            }?.value?.toIntOrNull() ?: return null
            AccessibleAnnotationTarget.Mention(docPos, label)
        } else {
            AccessibleAnnotationTarget.Link(annotation.value, label)
        }
        return InteractiveAnnotationHit(target, annotation, start, end)
    }

    private fun annotationBounds(textLayout: Layout, start: Int, end: Int): Rect {
        val firstLine = textLayout.getLineForOffset(start)
        val lastLine = textLayout.getLineForOffset((end - 1).coerceAtLeast(start))
        val bounds = Rect()
        for (line in firstLine..lastLine) {
            val segmentStart = maxOf(start, textLayout.getLineStart(line))
            val segmentEnd = minOf(end, textLayout.getLineEnd(line))
            val left = totalPaddingLeft + minOf(
                textLayout.getPrimaryHorizontal(segmentStart),
                textLayout.getPrimaryHorizontal(segmentEnd)
            ) - scrollX
            val right = totalPaddingLeft + maxOf(
                textLayout.getPrimaryHorizontal(segmentStart),
                textLayout.getPrimaryHorizontal(segmentEnd)
            ) - scrollX
            val lineBounds = Rect(
                kotlin.math.floor(left.toDouble()).toInt(),
                totalPaddingTop + textLayout.getLineTop(line) - scrollY,
                kotlin.math.ceil(right.toDouble()).toInt().coerceAtLeast(
                    kotlin.math.floor(left.toDouble()).toInt() + 1
                ),
                totalPaddingTop + textLayout.getLineBottom(line) - scrollY
            )
            if (bounds.isEmpty) bounds.set(lineBounds) else bounds.union(lineBounds)
        }
        return bounds
    }

    private fun handleImageTap(event: MotionEvent): Boolean {
        if (!imageResizingEnabled) {
            pendingImageGesture = null
            return false
        }
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val hit = if (event.pointerCount == 1) imageSpanHitAt(event.x, event.y) else null
                pendingImageGesture = hit?.let {
                    ImageGesture(
                        target = it.span,
                        pointerId = event.getPointerId(event.actionIndex),
                        downX = event.x,
                        downY = event.y
                    )
                }
                if (hit != null) requestFocus()
                return hit != null
            }
            MotionEvent.ACTION_MOVE -> {
                val gesture = pendingImageGesture ?: return false
                if (
                    event.pointerCount != 1 ||
                    event.findPointerIndex(gesture.pointerId) < 0 ||
                    movedBeyondImageTouchSlop(gesture, event)
                ) {
                    pendingImageGesture = null
                    return false
                }
                return true
            }
            MotionEvent.ACTION_POINTER_DOWN,
            MotionEvent.ACTION_POINTER_UP,
            MotionEvent.ACTION_CANCEL -> {
                pendingImageGesture = null
                return false
            }
            MotionEvent.ACTION_UP -> {
                val gesture = pendingImageGesture
                pendingImageGesture = null
                if (
                    gesture == null ||
                    event.pointerCount != 1 ||
                    event.getPointerId(event.actionIndex) != gesture.pointerId ||
                    movedBeyondImageTouchSlop(gesture, event)
                ) {
                    return false
                }
                val hit = imageSpanHitAt(event.x, event.y) ?: return false
                if (hit.span !== gesture.target) return false
                requestFocus()
                selectExplicitImageRange(hit.start, hit.end)
                performClick()
                return true
            }
            else -> return false
        }
    }

    private fun movedBeyondImageTouchSlop(gesture: ImageGesture, event: MotionEvent): Boolean {
        val deltaX = event.x - gesture.downX
        val deltaY = event.y - gesture.downY
        return deltaX * deltaX + deltaY * deltaY > touchSlopPx * touchSlopPx
    }

    private var pendingTaskMarkerDownScalar: Int? = null
    private var pendingTaskMarkerDownX: Float = 0f
    private var pendingTaskMarkerDownY: Float = 0f

    /**
     * Task-marker taps are recognized as a paired DOWN+UP on the same marker
     * within touch slop. DOWN is never consumed (so scrolls and selection
     * gestures that start on a checkbox keep working); only the matching UP
     * is consumed.
     */
    private fun handleTaskListMarkerTap(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                pendingTaskMarkerDownScalar = taskListMarkerScalarHitAt(event.x, event.y)
                pendingTaskMarkerDownX = event.x
                pendingTaskMarkerDownY = event.y
                return false
            }
            MotionEvent.ACTION_MOVE -> {
                if (pendingTaskMarkerDownScalar != null && !withinTouchSlop(event)) {
                    pendingTaskMarkerDownScalar = null
                }
                return false
            }
            MotionEvent.ACTION_UP -> {
                val downScalar = pendingTaskMarkerDownScalar ?: return false
                pendingTaskMarkerDownScalar = null
                if (!withinTouchSlop(event)) return false
                val upScalar = taskListMarkerScalarHitAt(event.x, event.y) ?: return false
                if (upScalar != downScalar) return false
                requestFocus()
                toggleTaskItemCheckedAtSelectionScalarInRust(upScalar, upScalar)
                performClick()
                return true
            }
            MotionEvent.ACTION_CANCEL -> {
                pendingTaskMarkerDownScalar = null
                return false
            }
            else -> return false
        }
    }

    private fun withinTouchSlop(event: MotionEvent): Boolean {
        return kotlin.math.abs(event.x - pendingTaskMarkerDownX) <= touchSlopPx &&
            kotlin.math.abs(event.y - pendingTaskMarkerDownY) <= touchSlopPx
    }

    private fun taskListMarkerScalarHitAt(x: Float, y: Float): Int? {
        val spanned = text as? Spanned ?: return null
        val textLayout = layout ?: return null
        if (spanned.isEmpty()) return null

        val localX = x + scrollX - totalPaddingLeft
        val localY = y + scrollY - totalPaddingTop
        if (localY < 0) return null

        val line = textLayout.getLineForVertical(localY.toInt().coerceAtLeast(0))
        val lineTop = textLayout.getLineTop(line).toFloat()
        val lineBottom = textLayout.getLineBottom(line).toFloat()
        if (localY < lineTop || localY > lineBottom) {
            return null
        }
        val lineStart = textLayout.getLineStart(line).coerceIn(0, spanned.length)
        val lineEnd = textLayout.getLineEnd(line).coerceIn(lineStart, spanned.length)
        val markerEnd = renderedTaskListMarkerEnd(spanned, lineStart, lineEnd) ?: return null
        val markerRight = textLayout.getPrimaryHorizontal(markerEnd).coerceAtLeast(
            textLayout.getPrimaryHorizontal(lineStart)
        ) + MARKER_TAP_HORIZONTAL_SLOP_DP * resources.displayMetrics.density
        if (localX > markerRight) {
            return null
        }

        // Confirmed hit: only now pay for the String conversion the
        // PositionBridge scalar math requires.
        val currentText = spanned.toString()
        val snappedUtf16 = PositionBridge.snapToScalarBoundary(
            lineStart,
            currentText,
            biasForward = true
        )
        return PositionBridge.utf16ToScalar(snappedUtf16, currentText)
    }

    private fun imageSpanHitAt(x: Float, y: Float): ImageSpanHit? {
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
    ): ImageSpanHit? {
        val candidateSpans = spannable.getSpans(0, spannable.length, BlockImageSpan::class.java)
        for (span in candidateSpans) {
            val spanStart = spannable.getSpanStart(span)
            val spanEnd = spannable.getSpanEnd(span)
            if (spanStart < 0 || spanEnd <= spanStart) continue
            val rect = resolvedImageRect(textLayout, span, spanStart, spanEnd)
            if (rect.contains(x, y)) {
                return ImageSpanHit(span, spanStart, spanEnd)
            }
        }
        return null
    }

    override fun onFocusChanged(focused: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
        super.onFocusChanged(focused, direction, previouslyFocusedRect)
        if (focused) {
            clearNativeTextMutationAfterBlurWindow()
            restartCaretBlink()
        } else {
            beginNativeTextMutationAfterBlurWindow()
            clearExplicitSelectedImageRange()
            stopCaretBlink()
        }
    }

    override fun onWindowFocusChanged(hasWindowFocus: Boolean) {
        super.onWindowFocusChanged(hasWindowFocus)
        if (hasWindowFocus) restartCaretBlink() else stopCaretBlink()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        restartCaretBlink()
        if (restartImageLoadsOnAttach) {
            restartImageLoadsOnAttach = false
            rebuildLatestRenderForImages()
        }
    }

    override fun onDetachedFromWindow() {
        restartImageLoadsOnAttach = hasRenderedImageSpans()
        cancelPendingImageLoads()
        removeCallbacks(caretBlinkRunnable)
        super.onDetachedFromWindow()
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
    ): ImageSpanHit? {
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
                return ImageSpanHit(imageSpan, spanStart, spanEnd)
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
     * We convert doc→scalar via the v2 driver ([EditorV2Driver.scalarPositionForDoc])
     * before converting to UTF-16.
     */
    private fun applySelectionFromJSON(selection: org.json.JSONObject) {
        val type = selection.optString("type", "") ?: return
        if (isEditorDestroyedForInput()) {
            recordImeTraceForTesting("applySelectionFromJSONSkipped", "reason=destroyed type=$type")
            return
        }

        isApplyingRustState = true
        try {
            val currentText = text?.toString() ?: ""
            when (type) {
                "text" -> {
                    val docAnchor = exactV2ScalarInt(selection.opt("anchor") as? Number) ?: return
                    val docHead = exactV2ScalarInt(selection.opt("head") as? Number) ?: return
                    // The frozen v2 update includes exact scalar positions alongside its
                    // structural document positions. Prefer those values: a cursor inside
                    // an empty trailing block sits at a document boundary that cannot be
                    // reconstructed from rendered structural tokens alone.
                    val selectionDriver = v2Driver ?: return
                    val scalarAnchor = exactV2ScalarInt(selection.opt("anchorScalar") as? Number)
                        ?: selectionDriver.scalarPositionForDoc(docAnchor)
                        ?: docAnchor
                    val scalarHead = exactV2ScalarInt(selection.opt("headScalar") as? Number)
                        ?: selectionDriver.scalarPositionForDoc(docHead)
                        ?: docHead
                    val anchorUtf16 = PositionBridge.scalarToUtf16(scalarAnchor, currentText)
                    val headUtf16 = PositionBridge.scalarToUtf16(scalarHead, currentText)
                    val len = text?.length ?: 0
                    recordImeTraceForTesting(
                        "applySelectionFromJSON",
                        "doc=$docAnchor..$docHead scalar=$scalarAnchor..$scalarHead utf16=$anchorUtf16..$headUtf16 len=$len"
                    )
                    setSelection(
                        anchorUtf16.coerceIn(0, len),
                        headUtf16.coerceIn(0, len)
                    )
                }
                "node" -> {
                    val docPos = exactV2ScalarInt(selection.opt("pos") as? Number) ?: return
                    // Convert doc position to scalar offset.
                    val nodeSelectionDriver = v2Driver ?: return
                    val scalarPos = nodeSelectionDriver.scalarPositionForDoc(docPos) ?: docPos
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
            if (!hasLiveEditor()) return

            val currentText = s?.toString() ?: ""
            if (currentText == lastAuthorizedText) return

            val mutation = nativeTextMutationFromAuthorizedDiff(currentText)
            if (mutation != null && shouldAdoptNativeTextMutation(mutation, allowAfterBlur = true)) {
                commitNativeTextMutation(mutation)
                return
            }

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
            val stateJSON = v2Driver?.currentStateJson() ?: return
            applyUpdateJSON(stateJSON)
        }
    }

    companion object {
        private const val DEFAULT_AUTO_CAPITALIZE = "sentences"
        private const val DEFAULT_AUTO_CORRECT = true
        private const val DEFAULT_KEYBOARD_TYPE = "default"
        private const val EMPTY_BLOCK_PLACEHOLDER = '\u200B'
        private const val IME_TRACE_LIMIT_FOR_TESTING = 80
        private const val IME_TRACE_LOG_TAG = "NativeEditorIme"
        private const val NATIVE_TEXT_MUTATION_AFTER_BLUR_WINDOW_MS = 750L
        private const val RECENT_HANDLED_HARDWARE_KEY_DOWN_WINDOW_MS = 750L
        private const val LOG_TAG = "NativeEditor"

        // Platform caret blink half-period (Editor.BLINK).
        private const val CARET_BLINK_INTERVAL_MS = 500L
        private const val MIN_CARET_WIDTH_PX = 2f
        private const val MARKER_TAP_HORIZONTAL_SLOP_DP = 8f
    }
}
