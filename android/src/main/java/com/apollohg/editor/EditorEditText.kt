package com.apollohg.editor

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Rect
import android.graphics.RectF
import android.text.Annotation
import android.text.Selection
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.util.AttributeSet
import android.view.KeyEvent
import android.view.DragEvent
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat

/**
 * Rendering surface that routes input through [EditorV2Driver].
 * IME composition is displayed transiently, then committed against the authorized selection.
 * View methods and driver calls run synchronously on the main thread.
 */
class EditorEditText @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = android.R.attr.editTextStyle
) : EditorTextSurface(context, attrs, defStyleAttr) {

    internal data class AuthoritativeInputSnapshot(
        val renderedText: CharSequence,
        val selectionStart: Int,
        val selectionEnd: Int,
    )
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
        /** A Rust u32 retained in a signed [Long] without narrowing. */
        val docPos: Long,
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
        /** A Rust u32 retained in a signed [Long] without narrowing. */
        data class Mention(val docPos: Long, val label: String) : AccessibleAnnotationTarget
        data class Link(val href: String, val text: String) : AccessibleAnnotationTarget
    }

    /**
     * Listener interface for editor events, parallel to iOS's EditorTextViewDelegate.
     */
    interface EditorListener {
        /** Called when the editor's selection changes (anchor and head as scalar offsets). */
        fun onSelectionChanged(anchor: Int, head: Int)

        /** Called when the editor content is updated after a Rust operation. */
        fun onEditorUpdate(updateJSON: String)

        fun onExternalTextCompositionEnded(resultJson: String) = Unit
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
    var onContentSizeMayChange: (() -> Unit)? = null

    /** The base font size in pixels used for unstyled text. */
    internal var baseFontSize: Float = textSize

    /** The base text color as an ARGB int. */
    internal var baseTextColor: Int = currentTextColor

    /** The base background color before theme overrides. */
    internal var baseBackgroundColor: Int = android.graphics.Color.WHITE

    /** Optional render theme supplied by React. */
    internal val codeHighlightingSession = CodeHighlightingSession()
    internal var codeHighlightingConfiguration: NativeCodeHighlightingConfig? = null
    internal var reuseImagesDuringThemeUpdate = false
    internal var standaloneRenderJSON: String? = null
    var theme: EditorTheme? = null
        internal set

    internal var atomRenderConfiguration: AtomRenderConfiguration? = null
        internal set
    internal var atomHeightRenderApplyCount = 0

    var placeholderText: String = ""
        set(value) {
            if (field == value) return
            field = value
            requestLayout()
            invalidate()
        }

    var heightBehavior: EditorHeightBehavior = EditorHeightBehavior.FIXED
        internal set

    // Cached once per view instance so paired-tap tracking doesn't re-query
    // ViewConfiguration on every MotionEvent.
    internal val touchSlopPx: Float by lazy(LazyThreadSafetyMode.NONE) {
        android.view.ViewConfiguration.get(context).scaledTouchSlop.toFloat()
    }
    internal var imageResizingEnabled = true
    internal val decodedBitmapOwnerId: Long = DecodedBitmapBudget.nextOwnerId()
    internal var imageLoadingPolicy: ImageLoadingPolicy = ImageLoadingPolicy.DEFAULT
        internal set
    internal var imageLoadGeneration: Long = 0L
    internal val imageLoadHandles = mutableListOf<RenderImageLoader.LoadHandle>()
    internal var nativeAutoCapitalize = DEFAULT_AUTO_CAPITALIZE
    internal var nativeAutoCorrect = DEFAULT_AUTO_CORRECT
    internal var nativeKeyboardType = DEFAULT_KEYBOARD_TYPE

    internal var contentInsets: EditorContentInsets? = null
    internal var viewportBottomInsetPx: Int = 0
    internal var viewportBottomOcclusionTopOnScreenPx: Int? = null
    internal val caretVisibilityLocationOnScreen = IntArray(2)
    internal var caretVisibilityRequestPosted = false

    /**
     * The plain text from the last Rust-authorized render.
     * Used by [ReconciliationWatcher] to detect unauthorized divergence.
     */
    internal var lastAuthorizedText: String = ""

    /**
     * Number of reconciliation events triggered during this EditText's lifetime.
     * Useful for monitoring and kill-condition analysis.
     */
    var reconciliationCount: Int = 0
        internal set

    internal var lastHandledHardwareKeySignature: HardwareKeyEventSignature? = null
    internal var recentHandledHardwareKeyDownSignature: HardwareKeyEventSignature? = null
    internal var recentHandledHardwareKeyDownUptimeMs: Long = 0L
    internal var activeInputConnection: EditorInputConnection? = null
    internal var inputConnectionGeneration: Long = 0L
    internal var imeTextCoordinateRevision: Long = 0L
    internal var cachedImeTextCoordinateRevision: Long = -1L
    internal var cachedImeTextCoordinateMapper: ImeTextCoordinateMapper? = null
    internal var composingText: String? = null
    internal var composingReplacementStartUtf16: Int? = null
    internal var composingReplacementEndUtf16: Int? = null
    internal var composingReplacementAuthorizedTextRevision: Long? = null
    internal var didInvalidateCompositionReplacementRange = false
    internal var externalTextComposition: ExternalTextCompositionState? = null
    internal val externalCompositionMarker = Any()
    internal val externalTextCompositionTerminalResults = mutableMapOf<String, String>()
    internal var nativeTextMutationAfterBlurWindow: NativeTextMutationAfterBlurWindow? = null
    internal var nativeTextMutationAdoptionSuppression: NativeTextMutationAdoptionSuppression? = null
    internal var lastAuthorizedTextRevision: Long = 0L
    internal var lastAuthorizedRenderedText: CharSequence? = null
    internal var explicitSelectedImageRange: ImageSelectionRange? = null
    internal var suppressedImageSelectionHighlightColor: Int? = null
    internal var pendingImageGesture: ImageGesture? = null
    internal var lastRenderAppliedPatchForTesting: Boolean = false
    internal var captureApplyUpdateTraceForTesting: Boolean = false
    internal var lastApplyUpdateTraceForTesting: ApplyUpdateTrace? = null
    internal val imeTraceForTesting = java.util.ArrayDeque<String>()
    internal var imeTraceSequence: Long = 0L
    internal var lastImeTraceUptimeMs: Long = 0L
    internal var currentRenderBlocksJson: org.json.JSONArray? = null
    internal var currentRenderBlocksDocumentVersion: String? = null
    internal var currentRenderBlocksNeedFullApply = false
    internal var authorizedVisibleTextNeedsRebuild = false
    internal var logicalSelectionSnapshot: LogicalSelectionSnapshot? = null
    internal var lastAllowedAtomCaretSelection: Pair<Int, Int>? = null
    private var localTextDrag: LocalTextDrag? = null
    internal var lastAppliedDocumentVersion: String? = null
    internal var restartImageLoadsOnAttach = false
    internal var renderAppearanceRevision: Long = 1L
    internal var lastAppliedRenderAppearanceRevision: Long = 0L
    internal var pendingOptimisticRenderText: String? = null
    internal var deferredRustUpdateApplicationDepth: Int = 0
    internal var deferredRustUpdateJSON: String? = null
    internal var deferredRustUpdateLineBoundaryRefreshSource: String? = null
    internal var deferredRustUpdateGeneration: Long = 0L
    internal var recoveringRenderPatchBaseMismatch = false
    internal var externalUpdatePreparationCaptureDepth: Int = 0
    internal var capturedExternalUpdatePreparationJSON: String? = null
    internal var lineBoundaryInputRefreshGeneration: Long = 0L
    internal var restartInputSelectionUpdateGeneration: Long = 0L
    internal var onDeleteRangeInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onDeleteBackwardAtSelectionScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onToggleTaskItemCheckedAtSelectionScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onInsertTextInRustForTesting: ((String, Int) -> Unit)? = null
    internal var onSplitBlockInRustForTesting: ((Int) -> Unit)? = null
    internal var onReplaceTextInRustForTesting: ((Int, Int, String) -> Unit)? = null
    internal var onSetSelectionScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onDeleteAndSplitScalarInRustForTesting: ((Int, Int) -> Unit)? = null
    internal var onInsertContentHtmlInRustForTesting: ((String) -> Unit)? = null
    internal var onResizeImageAtDocPosForTesting: ((Int, Int, Int) -> Unit)? = null
    internal var onMoveSelectionScalarForTesting: ((Int, Int, Int) -> Unit)? = null
    internal var onBeforeRenderRefresh: (() -> Unit)? = null
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
        set(value) {
            if (field === value) return
            (field as? EditorV2Adapter)?.releaseNativeBindingOwner(nativeBindingToken)
            field = value
            invalidateCurrentRenderBlocks()
            (value as? EditorV2Adapter)?.claimNativeBindingIfUnowned(nativeBindingToken)
        }
    internal val nativeBindingToken = nextNativeBindingToken.incrementAndGet()

    internal fun ownsNativeBinding(adapter: EditorV2Adapter): Boolean = ownsNativeBindingImpl(adapter)

    fun lastRenderAppliedPatch(): Boolean = lastRenderAppliedPatchImpl()

    fun lastApplyUpdateTrace(): ApplyUpdateTrace? = lastApplyUpdateTraceImpl()

    internal fun hasDeferredRustUpdateApplicationForTesting(): Boolean =
        hasDeferredRustUpdateApplicationForTestingImpl()

    internal fun inputConnectionGenerationForTesting(): Long = inputConnectionGenerationForTestingImpl()

    internal fun authorizedTextForTesting(): String = authorizedTextForTestingImpl()

    internal fun applyRustUpdateJSONForTesting(updateJSON: String) =
        applyRustUpdateJSONForTestingImpl(updateJSON)

    internal fun recordImeTraceForTesting(event: String, details: String = "") =
        recordImeTraceForTestingImpl(event, details)

    internal fun clearImeTraceForTesting() = clearImeTraceForTestingImpl()

    internal fun imeTraceSnapshotForTesting(): List<String> = imeTraceSnapshotForTestingImpl()

    init {
        initializeEditorView()
    }

    internal val legacyCursorClipPaint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG)
    internal val caretWidthPx: Float by lazy { resolveCaretWidth(attrs, defStyleAttr) }
    internal val caretColor: Int by lazy { resolveCaretColor() }
    internal var editorAccessibilityHint: CharSequence? = null

    fun setEditorAccessibilityHint(hint: CharSequence?) = setEditorAccessibilityHintImpl(hint)

    override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
        super.onInitializeAccessibilityNodeInfo(info)
        AccessibilityNodeInfoCompat.wrap(info).tooltipText = editorAccessibilityHint
    }

    internal fun nativeCursorDrawRect(): RectF? = nativeCursorDrawRectImpl()

    fun setAutoCapitalize(autoCapitalize: String?) = setAutoCapitalizeImpl(autoCapitalize)

    fun setAutoCorrect(autoCorrect: Boolean?) = setAutoCorrectImpl(autoCorrect)

    fun setKeyboardType(keyboardType: String?) = setKeyboardTypeImpl(keyboardType)

    fun setPrivateImeOptionsForEditor(value: String?) = setPrivateImeOptionsForEditorImpl(value)

    /**
     * Create a custom [EditorInputConnection] that intercepts all input
     * from the soft keyboard.
     */
    override fun onSurfaceInputStateChanged() {
        activeInputConnection?.publishInputStateIfNeeded()
    }

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        val baseConnection = super.onCreateInputConnection(outAttrs) ?: return null
        return configureInputConnection(baseConnection, outAttrs)
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        if (!isEditable && isReadOnlyTextMutationKeyEvent(event)) {
            return true
        }
        if (!commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
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

    internal fun handleCompositionKeyEvent(event: KeyEvent, applyBaseEvent: () -> Boolean): Boolean =
        handleCompositionKeyEventImpl(event, applyBaseEvent)

    override fun onDraw(canvas: android.graphics.Canvas) {
        updateAtomBoundaryCursorVisibility()
        drawStyleSheetBoxes(canvas)
        super.onDraw(canvas)
        layout?.let {
            val saved = canvas.save()
            canvas.translate(compoundPaddingLeft.toFloat(), extendedPaddingTop.toFloat())
            EditorTextDecorationDrawing.draw(canvas, it)
            canvas.restoreToCount(saved)
        }

        val placeholderLayout =
            buildPlaceholderLayout(width - compoundPaddingLeft - compoundPaddingRight) ?: return

        val previousColor = paint.color
        val saveCount = canvas.save()
        val placeholderInsets = placeholderContentInsets(width - compoundPaddingLeft - compoundPaddingRight)
        canvas.translate(compoundPaddingLeft + placeholderInsets.left, extendedPaddingTop + placeholderInsets.top)
        placeholderLayout.draw(canvas)
        EditorTextDecorationDrawing.draw(canvas, placeholderLayout)
        canvas.restoreToCount(saveCount)
        paint.color = previousColor
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        super.onMeasure(widthMeasureSpec, heightMeasureSpec)

        val placeholderHeight = resolvePlaceholderHeightForMeasuredWidth(measuredWidth) ?: 0
        val desiredHeight = maxOf(
            measuredHeight,
            placeholderHeight,
        )
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
        if (heightBehavior == EditorHeightBehavior.FIXED && !interaction.hasScrollContainer()) {
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

    override fun onDragEvent(event: DragEvent): Boolean {
        return when (event.action) {
            DragEvent.ACTION_DRAG_STARTED -> {
                val drag = localTextDragFor(event)
                localTextDrag = drag
                drag != null || super.onDragEvent(event)
            }
            DragEvent.ACTION_DROP -> {
                val drag = localTextDrag
                localTextDrag = null
                if (drag == null) {
                    super.onDragEvent(event)
                } else {
                    val currentText = text?.toString().orEmpty()
                    val destinationUtf16 = getOffsetForPosition(event.x, event.y)
                        .coerceIn(0, currentText.length)
                    val destination = PositionBridge.utf16ToScalar(destinationUtf16, currentText)
                    performLocalSelectionDrop(drag, destination) || super.onDragEvent(event)
                }
            }
            DragEvent.ACTION_DRAG_ENDED -> {
                val handled = localTextDrag != null
                localTextDrag = null
                super.onDragEvent(event) || handled
            }
            DragEvent.ACTION_DRAG_ENTERED,
            DragEvent.ACTION_DRAG_LOCATION,
            DragEvent.ACTION_DRAG_EXITED -> super.onDragEvent(event) || localTextDrag != null
            else -> super.onDragEvent(event)
        }
    }

    internal fun performLocalSelectionDropForTesting(
        scalarFrom: Int,
        scalarTo: Int,
        destination: Int,
        documentVersion: String?,
    ): Boolean =
        performLocalSelectionDropForTestingImpl(scalarFrom, scalarTo, destination, documentVersion)

    override fun performClick(): Boolean {
        return super.performClick()
    }

    /**
     * The core's `documentIsEmpty` from the most recent editor update, or null
     * when the current render arrived without one.
     */
    internal var coreReportedDocumentIsEmpty: Boolean? = null

    /** Adopt the core's authoritative empty state from an editor update. */
    fun setCoreReportedDocumentIsEmpty(isEmpty: Boolean?) = setCoreReportedDocumentIsEmptyImpl(isEmpty)

    fun shouldDisplayPlaceholderForTesting(): Boolean = shouldDisplayPlaceholderForTestingImpl()

    /**
     * Bind this EditText to a Rust editor instance and optionally apply initial content.
     *
     * @param id The editor session public ID.
     * @param initialHTML Optional HTML to set as initial content.
     */
    fun bindEditor(id: Long, initialHTML: String? = null, notifyListener: Boolean = true) =
        bindEditorImpl(id, initialHTML, notifyListener)

    /**
     * Unbind from the current editor instance.
     */
    fun unbindEditor() = unbindEditorImpl()

    internal fun handleEditorDestroyedFromRegistry(destroyedEditorId: Long) =
        handleEditorDestroyedFromRegistryImpl(destroyedEditorId)

    fun setBaseStyle(fontSizePx: Float, textColor: Int, backgroundColor: Int) =
        setBaseStyleImpl(fontSizePx, textColor, backgroundColor)

    fun applyTheme(theme: EditorTheme?) = applyThemeImpl(theme)

    fun applyAtomRenderConfiguration(configuration: AtomRenderConfiguration?): Boolean =
        applyAtomRenderConfigurationImpl(configuration)

    internal fun applyAtomHeight(
        atomKey: String,
        heightPx: Int,
        configuration: AtomRenderConfiguration?
    ): Boolean =
        applyAtomHeightImpl(atomKey, heightPx, configuration)

    internal fun atomHeightRenderApplyCountForTesting(): Int = atomHeightRenderApplyCountForTestingImpl()

    fun setHeightBehavior(heightBehavior: EditorHeightBehavior) = setHeightBehaviorImpl(heightBehavior)

    fun setViewportBottomInsetPx(bottomInsetPx: Int) = setViewportBottomInsetPxImpl(bottomInsetPx)

    fun setViewportBottomOcclusionTopOnScreenPx(topPx: Int?) =
        setViewportBottomOcclusionTopOnScreenPxImpl(topPx)

    fun setImageResizingEnabled(enabled: Boolean) = setImageResizingEnabledImpl(enabled)

    fun setImageLoadingPolicyJson(policyJson: String?) = setImageLoadingPolicyJsonImpl(policyJson)

    internal fun currentImageLoadGeneration(): Long = currentImageLoadGenerationImpl()

    internal fun registerImageLoad(handle: RenderImageLoader.LoadHandle) = registerImageLoadImpl(handle)

    internal fun activeImageLoadHandleCountForTesting(): Int = activeImageLoadHandleCountForTestingImpl()

    internal fun onImageSpanSizeMayChange(span: BlockImageSpan) = onImageSpanSizeMayChangeImpl(span)

    fun resolveAutoGrowHeight(): Int = resolveAutoGrowHeightImpl()

    internal fun caretRect(): RectF? = caretRectImpl()

    /**
     * Handle committed text from the IME (typed characters, autocomplete).
     *
     * Called by [EditorInputConnection.commitText]. Routes the text through
     * the Rust editor instead of directly inserting into the EditText.
     */
    fun handleTextCommit(text: String, newCursorPosition: Int = 1) =
        handleTextCommitImpl(text, newCursorPosition)

    internal fun runWithTransientInputMutationGuard(block: () -> Boolean): Boolean =
        runWithTransientInputMutationGuardImpl(block)

    fun beginExternalTextComposition(sessionId: String): String = beginExternalTextCompositionImpl(sessionId)

    fun updateExternalTextComposition(sessionId: String, text: String): String =
        updateExternalTextCompositionImpl(sessionId, text)

    fun commitExternalTextComposition(sessionId: String, finalText: String): String =
        commitExternalTextCompositionImpl(sessionId, finalText)

    fun cancelExternalTextComposition(sessionId: String, cause: String): String =
        cancelExternalTextCompositionImpl(sessionId, cause)

    internal fun commitExternalTextCompositionBeforeInteractionIfNeeded(): Boolean =
        commitExternalTextCompositionBeforeInteractionIfNeededImpl()

    internal fun hasActiveExternalTextCompositionForEditor(): Boolean =
        hasActiveExternalTextCompositionForEditorImpl()

    internal fun authorizedUtf16Range(start: Int, end: Int): Pair<Int, Int> =
        authorizedUtf16RangeImpl(start, end)

    internal fun isCurrentTextAuthorizedForEditor(): Boolean = isCurrentTextAuthorizedForEditorImpl()

    internal fun captureCompositionReplacementRangeIfNeeded() =
        captureCompositionReplacementRangeIfNeededImpl()

    internal fun setCompositionReplacementRange(start: Int, end: Int) =
        setCompositionReplacementRangeImpl(start, end)

    internal fun compositionReplacementRange(): Pair<Int, Int>? = compositionReplacementRangeImpl()

    internal fun consumeInvalidatedCompositionReplacementRangeForEditor(): Boolean =
        consumeInvalidatedCompositionReplacementRangeForEditorImpl()

    internal fun hasInvalidatedCompositionReplacementRangeForEditor(): Boolean =
        hasInvalidatedCompositionReplacementRangeForEditorImpl()

    internal fun setComposingTextForEditor(text: String?) = setComposingTextForEditorImpl(text)

    internal fun composingTextForEditor(): String? = composingTextForEditorImpl()

    internal fun samsungSentenceCapsComposingTextForEditor(composingText: String?): String? =
        samsungSentenceCapsComposingTextForEditorImpl(composingText)

    internal fun applyTransientComposingTextStyleForEditor() = applyTransientComposingTextStyleForEditorImpl()

    internal fun composingTextFromVisibleReplacementForEditor(): String? =
        composingTextFromVisibleReplacementForEditorImpl()

    internal fun clearCompositionTrackingForEditor() = clearCompositionTrackingForEditorImpl()

    internal fun retireInputConnectionForHostDetach() = retireInputConnectionForHostDetachImpl()

    internal fun isEditorDestroyedForInput(): Boolean = isEditorDestroyedForInputImpl()

    internal fun isInputConnectionCurrentForEditor(
        boundEditorId: Long,
        boundGeneration: Long
    ): Boolean =
        isInputConnectionCurrentForEditorImpl(boundEditorId, boundGeneration)

    internal fun imeTextCoordinateMapperForEditor(
        boundGeneration: Long = inputConnectionGeneration,
    ): ImeTextCoordinateMapper? =
        imeTextCoordinateMapperForEditorImpl(boundGeneration)

    internal fun restoreAuthorizedTextIfNeeded() = restoreAuthorizedTextIfNeededImpl()

    fun discardTransientNativeInputForEditorRebind() = discardTransientNativeInputForEditorRebindImpl()

    internal fun discardTransientNativeInputForExternalRecovery() =
        discardTransientNativeInputForExternalRecoveryImpl()

    fun prepareForExternalEditorUpdate(): Boolean = prepareForExternalEditorUpdateImpl()

    /**
     * Performs external-update preflight while retaining a mutation snapshot
     * for the caller that will apply it. This prevents a second state render
     * after a composing commit has already produced and adopted one.
     */
    internal fun hasPendingCompositionForExternalRefresh(): Boolean =
        hasPendingCompositionForExternalRefreshImpl()

    fun prepareForExternalEditorUpdateWithResult(): ExternalEditorUpdatePreparation =
        prepareForExternalEditorUpdateWithResultImpl()

    fun prepareForExternalEditorCommand(): CommandPreparation = prepareForExternalEditorCommandImpl()

    fun handleCompositionCommit(
        text: String,
        replacementStartUtf16: Int,
        replacementEndUtf16: Int,
        newCursorPosition: Int = 1
    ) =
        handleCompositionCommitImpl(text, replacementStartUtf16, replacementEndUtf16, newCursorPosition)

    fun handleCorrectionCommit(
        startUtf16: Int,
        endUtf16: Int,
        renderedOldText: String,
        newText: String
    ): Boolean =
        handleCorrectionCommitImpl(startUtf16, endUtf16, renderedOldText, newText)

    fun handleMissingOldTextCorrectionCommit(
        startUtf16: Int,
        endUtf16: Int,
        renderedOldText: String,
        newText: String
    ): Boolean =
        handleMissingOldTextCorrectionCommitImpl(startUtf16, endUtf16, renderedOldText, newText)

    internal fun missingOldTextCorrectionTokenRangeForEditor(
        text: String,
        offsetUtf16: Int
    ): Pair<Int, Int>? =
        missingOldTextCorrectionTokenRangeForEditorImpl(text, offsetUtf16)

    /**
     * Handle surrounding text deletion from the IME.
     *
     * Called by [EditorInputConnection.deleteSurroundingText].
     *
     * @param beforeLength Number of UTF-16 code units to delete before the cursor.
     * @param afterLength Number of UTF-16 code units to delete after the cursor.
     */
    fun handleDelete(beforeLength: Int, afterLength: Int) = handleDeleteImpl(beforeLength, afterLength)

    /**
     * Handle backspace key press (hardware keyboard or key event).
     *
     * If there's a range selection, deletes the range. Otherwise deletes
     * the grapheme cluster before the cursor.
     */
    fun handleBackspace() = handleBackspaceImpl()

    fun handleForwardDelete() = handleForwardDeleteImpl()

    /**
     * Handle return/enter key as a block split operation.
     */
    fun handleReturnKey() = handleReturnKeyImpl()

    /**
     * Handle Shift+Enter as an inline hard break insertion.
     */
    fun handleHardBreak() = handleHardBreakImpl()

    /**
     * Handle hardware Tab / Shift+Tab as list indent / outdent when the caret is in a list.
     */
    fun handleTab(shiftPressed: Boolean): Boolean = handleTabImpl(shiftPressed)

    fun handleHardwareKeyDown(keyCode: Int, shiftPressed: Boolean): Boolean =
        handleHardwareKeyDownImpl(keyCode, shiftPressed)

    internal fun isReadOnlyTextMutationKeyEvent(event: KeyEvent): Boolean =
        isReadOnlyTextMutationKeyEventImpl(event)

    fun handleHardwareKeyEvent(event: KeyEvent?): Boolean = handleHardwareKeyEventImpl(event)

    internal fun handlePrintableHardwareKeyEvent(
        event: KeyEvent,
        applyBaseEvent: () -> Boolean
    ): Boolean =
        handlePrintableHardwareKeyEventImpl(event, applyBaseEvent)

    fun performToolbarToggleMark(markName: String) = performToolbarToggleMarkImpl(markName)

    fun performToolbarToggleList(listType: String, isActive: Boolean) =
        performToolbarToggleListImpl(listType, isActive)

    fun performToolbarToggleBlockquote() = performToolbarToggleBlockquoteImpl()

    fun performToolbarToggleHeading(level: Int) = performToolbarToggleHeadingImpl(level)

    fun performToolbarIndentListItem() = performToolbarIndentListItemImpl()

    fun performToolbarOutdentListItem() = performToolbarOutdentListItemImpl()

    fun performToolbarInsertNode(nodeType: String) = performToolbarInsertNodeImpl(nodeType)

    fun performToolbarUndo() = performToolbarUndoImpl()

    fun performToolbarRedo() = performToolbarRedoImpl()

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

    /**
     * Override to notify the listener when selection changes.
     *
     * Converts the EditText selection to scalar offsets and notifies both
     * the listener and the Rust editor.
     */
    override fun onSelectionChanged(selStart: Int, selEnd: Int) {
        super.onSelectionChanged(selStart, selEnd)
        updateImageSelectionHighlightAppearance(selStart, selEnd)
        if (restoreSelectionFromAtomBoundaryIfNeeded(selStart, selEnd)) return
        canonicalListCaretOffset(selStart, selEnd)?.let { canonicalOffset ->
            setSelection(canonicalOffset)
            return
        }
        ensureSelectionVisible()
        if (isApplyingRustState) return
        val wasExternallyComposing = externalTextComposition != null
        if (!commitExternalTextCompositionBeforeInteractionIfNeeded()) return
        if (wasExternallyComposing) {
            val editable = text
            if (editable != null) {
                val restoredStart = selStart.coerceIn(0, editable.length)
                val restoredEnd = selEnd.coerceIn(0, editable.length)
                if (selectionStart != restoredStart || selectionEnd != restoredEnd) {
                    runWithTransientInputMutationGuard {
                        Selection.setSelection(editable, restoredStart, restoredEnd)
                        true
                    }
                }
            }
        }
        val spannable = text as? Spanned
        if (spannable != null && isExactImageSpanRange(spannable, selStart, selEnd)) {
            explicitSelectedImageRange = ImageSelectionRange(selStart, selEnd)
        }
        onSelectionOrContentMayChange?.invoke()

        syncCurrentSelectionToRust()
    }

    // Samsung Keyboard may call finishComposingText() and then commitText(" ")
    // for one space tap. Defer the render from finishComposingText() by one
    // loop so setText() does not restart input before the pending space arrives.
    internal fun runWithDeferredRustUpdateApplication(block: () -> Unit) =
        runWithDeferredRustUpdateApplicationImpl(block)

    internal fun authorizeCurrentVisibleTextForPendingImeOperationForEditor(
        logicalCursorAfter: Int? = null
    ) =
        authorizeCurrentVisibleTextForPendingImeOperationForEditorImpl(logicalCursorAfter)

    internal fun captureAuthoritativeInputSnapshotForEditor(): AuthoritativeInputSnapshot =
        captureAuthoritativeInputSnapshotForEditorImpl()

    internal fun deleteScalarRangeForPendingImeOperationForEditor(
        scalarFrom: Int,
        scalarTo: Int,
    ): EditorV2NativeIntentResult? =
        deleteScalarRangeForPendingImeOperationForEditorImpl(scalarFrom, scalarTo)

    internal fun promoteOptimisticInputForEditor(
        render: EditorV2NativeMutationRender,
        logicalCursorAfter: Int,
    ) =
        promoteOptimisticInputForEditorImpl(render, logicalCursorAfter)

    internal fun restoreAuthoritativeInputForEditor(
        snapshot: AuthoritativeInputSnapshot,
        recoveryUpdateJson: String? = null,
    ) =
        restoreAuthoritativeInputForEditorImpl(snapshot, recoveryUpdateJson)

    internal fun handleStructuralBackspace() = handleStructuralBackspaceImpl()

    internal fun handleStructuralDelete(
        utf16From: Int,
        utf16To: Int,
        scalarFrom: Int,
        scalarTo: Int
    ) =
        handleStructuralDeleteImpl(utf16From, utf16To, scalarFrom, scalarTo)

    internal fun applyVisibleCompositionCommitForPendingImeOperationForEditor(
        committedText: String,
        replacementStartUtf16: Int,
        replacementEndUtf16: Int,
        newCursorPosition: Int
    ): Boolean =
        applyVisibleCompositionCommitForPendingImeOperationForEditorImpl(committedText, replacementStartUtf16, replacementEndUtf16, newCursorPosition)

    internal fun commitAlreadyVisibleCompositionMutationForPendingImeOperationForEditor(
        committedText: String,
        newCursorPosition: Int
    ): Boolean =
        commitAlreadyVisibleCompositionMutationForPendingImeOperationForEditorImpl(committedText, newCursorPosition)

    internal fun currentScalarSelection(): Pair<Int, Int>? = currentScalarSelectionImpl()

    internal fun currentLogicalScalarSelectionForInput(): Pair<Int, Int>? =
        currentLogicalScalarSelectionForInputImpl()

    internal fun renderedRangeContainsGeneratedStructure(start: Int, endExclusive: Int): Boolean =
        renderedRangeContainsGeneratedStructureImpl(start, endExclusive)

    internal fun compositionContentRangeForEditor(start: Int, end: Int): Pair<Int, Int>? =
        compositionContentRangeForEditorImpl(start, end)

    internal fun cursorCapsModeForEditor(reqModes: Int, baseCapsMode: Int): Int =
        cursorCapsModeForEditorImpl(reqModes, baseCapsMode)

    internal fun initialSurroundingTextForImeForEditor(
        mapper: ImeTextCoordinateMapper? = null,
    ): ImeInitialSurroundingText? =
        initialSurroundingTextForImeForEditorImpl(mapper)

    fun selectedImageGeometry(): SelectedImageGeometry? = selectedImageGeometryImpl()

    fun resizeImageAtDocPos(docPos: Int, widthPx: Float, heightPx: Float) =
        resizeImageAtDocPosImpl(docPos, widthPx, heightPx)

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
    ): Boolean =
        applyUpdateJSONImpl(updateJSON, notifyListener, refreshInputConnectionForExternalUpdate)

    /**
     * Apply a render JSON string (just render elements, no update wrapper).
     *
     * Used for initial content loading (set_html / set_json return render
     * elements directly, not wrapped in an EditorUpdate).
     *
     * @param renderJSON The JSON array string of render elements.
     */
    fun applyRenderJSON(renderJSON: String) = applyRenderJSONImpl(renderJSON)

    fun mentionHitAt(x: Float, y: Float): MentionHit? = mentionHitAtImpl(x, y)

    fun linkHitAt(x: Float, y: Float): LinkHit? = linkHitAtImpl(x, y)

    internal fun accessibleAnnotations(): List<AccessibleAnnotation> = accessibleAnnotationsImpl()

    internal fun interactiveAnnotationHitAt(x: Float, y: Float): InteractiveAnnotationHit? =
        interactiveAnnotationHitAtImpl(x, y)

    internal var pendingTaskMarkerDownScalar: Int? = null
    internal var pendingTaskMarkerDownX: Float = 0f
    internal var pendingTaskMarkerDownY: Float = 0f

    override fun onFocusChanged(focused: Boolean, direction: Int, previouslyFocusedRect: Rect?) {
        super.onFocusChanged(focused, direction, previouslyFocusedRect)
        updateImageSelectionHighlightAppearance(focused = focused)
        if (focused) {
            clearNativeTextMutationAfterBlurWindow()
        } else {
            beginNativeTextMutationAfterBlurWindow()
            clearExplicitSelectedImageRange()
        }
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        refreshCodeHighlighting()
        if (restartImageLoadsOnAttach) {
            restartImageLoadsOnAttach = false
            rebuildLatestRenderForImages()
        }
    }

    override fun onDetachedFromWindow() {
        codeHighlightingSession.cancel()
        restartImageLoadsOnAttach = hasRenderedImageSpans()
        cancelPendingImageLoads()
        (text as? Spanned)?.getSpans(0, length(), BlockImageSpan::class.java)
            ?.forEach(BlockImageSpan::close)
        super.onDetachedFromWindow()
    }

    internal fun baseTextContextMenuItem(id: Int): Boolean = super.onTextContextMenuItem(id)

    internal fun baseAccessibilityAction(action: Int, arguments: android.os.Bundle?): Boolean =
        super.performAccessibilityAction(action, arguments)

    internal val editorSuggestedMinimumHeight: Int get() = suggestedMinimumHeight
    internal fun editorHorizontalScrollRange(): Int = computeHorizontalScrollRange()
    internal fun editorVerticalScrollRange(): Int = computeVerticalScrollRange()

    companion object {
        private val nextNativeBindingToken = java.util.concurrent.atomic.AtomicLong(Long.MAX_VALUE / 2)
        internal const val DEFAULT_AUTO_CAPITALIZE = "sentences"
        internal const val DEFAULT_AUTO_CORRECT = true
        internal const val DEFAULT_KEYBOARD_TYPE = "default"
        internal const val EMPTY_BLOCK_PLACEHOLDER = '\u200B'
        internal const val IME_TRACE_LIMIT_FOR_TESTING = 80
        internal const val IME_TRACE_LOG_TAG = "NativeEditorIme"
        internal const val NATIVE_TEXT_MUTATION_AFTER_BLUR_WINDOW_MS = 750L
        internal const val RECENT_HANDLED_HARDWARE_KEY_DOWN_WINDOW_MS = 750L
        internal const val LOG_TAG = "NativeEditor"

        internal const val MARKER_TAP_HORIZONTAL_SLOP_DP = 8f
    }
}
