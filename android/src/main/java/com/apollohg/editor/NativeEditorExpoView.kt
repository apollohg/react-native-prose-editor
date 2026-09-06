package com.apollohg.editor

import android.content.Context
import android.graphics.Point
import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.view.View
import android.view.Window
import androidx.core.view.ViewCompat
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import org.json.JSONObject

/**
 * Expo Modules wrapper view that hosts a [RichTextEditorView] and bridges
 * editor events to React Native via [EventDispatcher].
 *
 * Registered as the native view component in [NativeEditorModule].
 */
class NativeEditorExpoView(
    context: Context,
    appContext: AppContext
) : ExpoView(context, appContext), EditorEditText.EditorListener {

    override val shouldUseAndroidLayout = true

    internal enum class ToolbarPlacement {
        KEYBOARD,
        INLINE;

        companion object {
            fun fromRaw(raw: String?): ToolbarPlacement =
                if (raw == "inline") INLINE else KEYBOARD
        }
    }

    internal sealed class PendingNativeAction {
        data class ToolbarItemPress(val item: NativeToolbarItem) : PendingNativeAction()
        data class MentionSuggestionSelect(val suggestion: NativeMentionSuggestion) : PendingNativeAction()
    }

    internal data class PendingNativeActionScope(
        val editorId: Long,
        val documentVersion: String?,
        val allowedDocumentVersion: String?,
        val hadFocus: Boolean,
        val hadVisibleToolbar: Boolean,
        val selectionAnchor: Int?,
        val selectionHead: Int?,
        val mentionAnchor: Int? = null,
        val mentionHead: Int? = null,
        val mentionQuery: String? = null
    )

    internal data class PendingEditorUpdateEvent(
        /** Captured public source identity; never derive this after a rebind. */
        val editorId: String,
        /** Captured canonical document revision used by TS echo suppression. */
        val documentRevision: String,
        val viewUpdateJSON: String,
        val atomicUpdateJSON: String
    )

    internal data class NativeCommitKey(
        val editorId: String,
        val documentRevision: String,
    )

    internal data class EditorErrorBinding(
        val adapter: EditorV2Adapter,
        val editorId: String,
        val viewToken: Long,
        val callbackToken: Long,
        val generation: Long,
    )

    internal data class PendingEditorErrorEvent(
        /** Capture every identity at callback time; never derive it after a rebind. */
        val adapter: EditorV2Adapter,
        val editorId: String,
        val viewToken: Long,
        val callbackToken: Long,
        val bindingGeneration: Long,
        val error: EditorV2Error,
    )

    internal data class PreflightUpdateEvent(
        val updateJSON: String,
        val documentRevision: String
    )

    internal data class ActiveExternalTextComposition(
        val sessionId: String,
        val editorId: String,
    )

    internal enum class PendingPropertyRetryResult {
        STALE,
        EDITOR_CHANGED,
        READY,
    }

    internal class PendingPropertyRetry {
        var editorId: Long? = null
            internal set
        var attempts = 0
            internal set
        private var scheduled = false
        private var generation = 0

        fun bind(editorId: Long) {
            this.editorId = editorId
        }

        fun resetAttempts() {
            attempts = 0
        }

        fun cancel() {
            scheduled = false
            editorId = null
            attempts = 0
            generation += 1
        }

        fun schedule(editorId: Long, maxAttempts: Int): Pair<Int, Int>? {
            if (scheduled || attempts >= maxAttempts) return null
            attempts += 1
            this.editorId = editorId
            scheduled = true
            generation += 1
            return generation to attempts
        }

        fun consume(scheduledGeneration: Int, currentEditorId: Long): PendingPropertyRetryResult {
            if (scheduledGeneration != generation) return PendingPropertyRetryResult.STALE
            if (editorId != currentEditorId) return PendingPropertyRetryResult.EDITOR_CHANGED
            scheduled = false
            return PendingPropertyRetryResult.READY
        }
    }

    val richTextView: RichTextEditorView = RichTextEditorView(context)
    internal val keyboardToolbarView = EditorKeyboardToolbarView(context)
    internal val mainHandler = Handler(Looper.getMainLooper())
    internal val keyboardToolbarImeAnimationController = KeyboardToolbarImeAnimationController(
        toolbarView = keyboardToolbarView,
        onTargetImeBottomChanged = { bottom ->
            currentImeBottom = bottom
            updateKeyboardToolbarLayout()
            updateEditorViewportInset()
        },
        onImeAnimationSettled = {
            updateAttachedKeyboardToolbarForInsets()
        }
    )

    internal val onEditorUpdate by EventDispatcher<Map<String, Any>>()
    internal val onEditorError by EventDispatcher<Map<String, Any>>()
    private val onExternalTextCompositionEnd by EventDispatcher<Map<String, Any>>()
    private val onSelectionChange by EventDispatcher<Map<String, Any>>()
    private val onFocusChange by EventDispatcher<Map<String, Any>>()
    internal val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    internal val onAtomLayout by EventDispatcher<Map<String, Any>>()
    internal val onEditorReady by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    internal val onToolbarAction by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    internal val onAddonEvent by EventDispatcher<Map<String, Any>>()

    /** Guard flag: when true, editor updates originated from JS and should not echo back. */
    var isApplyingJSUpdate = false
    internal var blockEditorUpdatePreflightForTesting = false
    internal var blockThemePreflightForTesting = false
    internal var onToolbarActionForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onAddonEventForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onSelectionChangeForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onFocusChangeForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onContentHeightChangeForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onAtomLayoutForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorUpdateForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorErrorForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onExternalTextCompositionEndForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorReadyForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onOutsideTapTraceForTesting: ((String) -> Unit)? = null
    internal var onRefreshToolbarStateFromEditorSelectionForTesting: (() -> String?)? = null
    internal var onBeforePrepareForEditorCommandForTesting: (() -> Unit)? = null
    internal var isAttachedToNativeWindow = false
    internal var didApplyAutoFocus = false
    internal var heightBehavior = EditorHeightBehavior.FIXED
    internal var lastEmittedContentHeight = 0
    internal var lastEmittedContentHeightEditorId: Long? = null
    internal val autoGrowStyleSizePublisher = ExpoAutoGrowStyleSizePublisher(this)
    internal var lastPublishedAutoGrowHeightDp: Double? = null
    internal var outsideTapWindow: Window? = null
    internal var pendingOutsideTapHandlerInstallRetry: Runnable? = null
    internal var toolbarFramesInWindow: List<RectF> = emptyList()
    internal var lastToolbarTouchUptimeMs: Long? = null
    internal var editorFocusedForOutsideTapOverrideForTesting: Boolean? = null
    internal var pendingOutsideTapBlur: Runnable? = null
    internal var pendingKeyboardDismiss: Runnable? = null
    internal var pendingToolbarRefocus: Runnable? = null
    internal var pendingToolbarRefocusEditorId: Long? = null
    internal var pendingToolbarRefocusGeneration = 0
    internal var pendingKeyboardToolbarDetachGeneration = 0
    internal var autoFocusRequested = false
    internal var addons = NativeEditorAddons(null)
    internal var mentionQueryState: MentionQueryState? = null
    internal var lastMentionEventJson: String? = null
    internal var lastMentionEventEditorId: Long? = null
    internal var lastThemeJson: String? = null
    internal var pendingThemeJson: String? = null
    internal var hasPendingTheme = false
    internal val pendingThemeRetry = PendingPropertyRetry()
    internal var pendingAtomsJson: String? = null
    internal var hasPendingAtoms = false
    internal val pendingAtomsRetry = PendingPropertyRetry()
    internal var lastAddonsJson: String? = null
    internal var lastAtomsJson: String? = null
    internal val reactChildren = mutableListOf<View>()
    internal var lastRemoteSelectionsJson: String? = null
    internal var lastToolbarItemsJson: String? = null
    internal var lastToolbarFrameJson: String? = null
    internal var lastDocumentVersion: String? = null
    internal var renderedDocumentRevision: String? = null
    @Volatile
    internal var remoteCommitRebaseScheduled = false
    internal var remoteCommitRebaseEditorId: Long? = null
    internal var activeExternalTextComposition: ActiveExternalTextComposition? = null
    internal var toolbarState = NativeToolbarState.empty
    internal var showsToolbar = true
    internal var toolbarPlacement = ToolbarPlacement.KEYBOARD
    internal var currentImeBottom = 0
    internal var pendingEditorUpdateResetJson: String? = null
    internal var pendingEditorUpdateJson: String? = null
    @set:JvmName("setPendingEditorUpdateEditorIdState")
    internal var pendingEditorUpdateEditorId: Long? = null
    internal var pendingEditorUpdateRevision = 0L
    internal var appliedEditorUpdateRevision = 0L
    /** Permanently rejected prop revisions are consumed per bound editor. */
    internal var consumedEditorUpdateRevision = 0L
    internal var consumedEditorUpdateEditorId: Long? = null
    internal var pendingEditorResetUpdateJson: String? = null
    @set:JvmName("setPendingEditorResetUpdateEditorIdState")
    internal var pendingEditorResetUpdateEditorId: Long? = null
    internal var pendingEditorResetUpdateRevision = 0L
    internal var appliedEditorResetUpdateRevision = 0L
    /** Permanently rejected reset revisions are consumed per bound editor. */
    internal var consumedEditorResetUpdateRevision = 0L
    internal var consumedEditorResetUpdateEditorId: Long? = null
    internal var lastEditorUpdateJsonProp: String? = null
    internal var lastEditorUpdateEditorIdProp: Long? = null
    internal var lastEditorResetUpdateJsonProp: String? = null
    internal var lastEditorResetUpdateEditorIdProp: Long? = null
    internal var pendingEditorUpdateRetryScheduled = false
    internal var pendingEditorUpdateRetryEditorId: Long? = null
    internal var pendingEditorUpdateRetryKind: PendingEditorUpdateKind? = null
    internal var pendingEditorUpdateRetryGeneration = 0
    internal var pendingEditorUpdateRetryAttempts = 0
    internal var pendingEditorUpdateForcedRecoveryAttempted = false
    internal var pendingViewCommandUpdateJson: String? = null
    internal var pendingViewCommandUpdateEditorId: Long? = null
    internal var pendingViewCommandUpdateRetryScheduled = false
    internal var pendingViewCommandUpdateRetryGeneration = 0
    internal var pendingViewCommandUpdateRetryAttempts = 0
    internal var pendingPreflightWakeScheduled = false
    internal var pendingPreflightWakeGeneration = 0
    internal var pendingBlurRetry: Runnable? = null
    internal var pendingBlurRetryEditorId: Long? = null
    internal var pendingBlurRetryGeneration = 0
    internal var pendingBlurRetryAttempts = 0
    internal var pendingDetachPreflightRetryScheduled = false
    internal var pendingDetachPreflightRetryEditorId: Long? = null
    internal var pendingDetachPreflightRetryGeneration = 0
    internal var pendingDetachPreflightRetryAttempts = 0
    internal var pendingNativeAction: PendingNativeAction? = null
    internal var pendingNativeActionScope: PendingNativeActionScope? = null
    internal var pendingNativeActionRetryScheduled = false
    internal var pendingNativeActionRetryEditorId: Long? = null
    internal var pendingNativeActionRetryGeneration = 0
    internal var pendingNativeActionRetryAttempts = 0
    internal var lastReadyEditorId: Long? = null
    internal val pendingEditorUpdateEvents = java.util.ArrayDeque<PendingEditorUpdateEvent>()
    internal val pendingEditorUpdateKeys = mutableSetOf<NativeCommitKey>()
    internal var pendingEditorUpdateDispatchGeneration = 0
    internal var pendingEditorUpdateDispatchScheduled = false
    internal val pendingEditorErrorEvents = java.util.ArrayDeque<PendingEditorErrorEvent>()
    internal var pendingEditorErrorDispatchGeneration = 0
    internal var pendingEditorErrorDispatchScheduled = false
    internal var editorErrorBinding: EditorErrorBinding? = null
    internal var nextEditorErrorBindingGeneration = 0L

    init {
        addView(richTextView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        richTextView.onAtomLayoutChange = ::emitAtomLayout
        richTextView.editorEditText.editorListener = this
        richTextView.onBeforeDetachedFromWindow = {
            prepareForDetachFromWindow()
        }
        richTextView.onAutoGrowHeightMayChange = {
            if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
                requestLayout()
                emitContentHeightIfNeeded(force = false)
            }
        }
        keyboardToolbarView.onPressItem = { item ->
            handleToolbarItemPress(item)
        }
        keyboardToolbarView.onSelectMentionSuggestion = { suggestion ->
            insertMentionSuggestion(suggestion)
        }
        keyboardToolbarView.applyState(toolbarState)
        ViewCompat.setOnApplyWindowInsetsListener(keyboardToolbarView) { _, insets ->
            keyboardToolbarImeAnimationController.onApplyWindowInsets(insets)
            insets
        }
        ViewCompat.setWindowInsetsAnimationCallback(
            keyboardToolbarView,
            keyboardToolbarImeAnimationController.animationCallback
        )

        // Observe EditText focus changes.
        richTextView.editorEditText.setOnFocusChangeListener { _, hasFocus ->
            if (hasFocus) {
                cancelPendingToolbarRefocus()
                installOutsideTapBlurHandlerIfNeeded()
                scheduleOutsideTapBlurHandlerInstallRetry()
                refreshMentionQuery()
            } else {
                if (consumeToolbarFocusPreservationForBlur()) {
                    scheduleToolbarRefocus()
                    return@setOnFocusChangeListener
                }
                uninstallOutsideTapBlurHandler()
                clearMentionQueryState()
                clearPendingNativeActionRetry()
            }
            updateKeyboardToolbarVisibility()
            val event = mapOf<String, Any>(
                "isFocused" to hasFocus,
                "editorId" to eventEditorId(richTextView.editorId)
            )
            onFocusChangeForTesting?.invoke(event) ?: onFocusChange(event)
        }
    }

    fun setEditorHandle(handle: String?) = setEditorHandleImpl(handle)

    /**
     * Internal-only widget binding. This token is allocated by
     * [EditorV2Registry] and is never a public session identifier.
     */
    fun setEditorId(id: Long) = setEditorIdImpl(id)

    fun setThemeJson(themeJson: String?) = setThemeJsonImpl(themeJson)

    fun setImageLoadingPolicyJson(policyJson: String?) = setImageLoadingPolicyJsonImpl(policyJson)

    fun setHeightBehavior(rawHeightBehavior: String) = setHeightBehaviorImpl(rawHeightBehavior)

    fun setAddonsJson(addonsJson: String?) = setAddonsJsonImpl(addonsJson)

    fun setAtomsJson(atomsJson: String?) = setAtomsJsonImpl(atomsJson)

    internal val atomChildCount: Int
        get() = reactChildren.size

    internal fun atomChildAt(index: Int): View? = atomChildAtImpl(index)

    internal fun addAtomChild(child: View, index: Int) = addAtomChildImpl(child, index)

    internal fun removeAtomChildAt(index: Int) = removeAtomChildAtImpl(index)

    internal fun removeAtomChild(child: View) = removeAtomChildImpl(child)

    fun setRemoteSelectionsJson(remoteSelectionsJson: String?) =
        setRemoteSelectionsJsonImpl(remoteSelectionsJson)

    fun setAutoFocus(autoFocus: Boolean) = setAutoFocusImpl(autoFocus)

    fun setAutoCapitalize(autoCapitalize: String?) = setAutoCapitalizeImpl(autoCapitalize)

    fun setAutoCorrect(autoCorrect: Boolean?) = setAutoCorrectImpl(autoCorrect)

    fun setKeyboardType(keyboardType: String?) = setKeyboardTypeImpl(keyboardType)

    fun setAndroidInputOptionsJson(optionsJson: String?) = setAndroidInputOptionsJsonImpl(optionsJson)

    fun setEditable(editable: Boolean) = setEditableImpl(editable)

    fun beginExternalTextComposition(sessionId: String): String = beginExternalTextCompositionImpl(sessionId)

    fun updateExternalTextComposition(sessionId: String, text: String): String =
        updateExternalTextCompositionImpl(sessionId, text)

    fun commitExternalTextComposition(sessionId: String, finalText: String): String =
        commitExternalTextCompositionImpl(sessionId, finalText)

    fun cancelExternalTextComposition(sessionId: String, cause: String): String =
        cancelExternalTextCompositionImpl(sessionId, cause)

    fun setAccessibilityLabel(label: String?) = setAccessibilityLabelImpl(label)

    fun setAccessibilityHint(hint: String?) = setAccessibilityHintImpl(hint)

    fun setShowToolbar(showToolbar: Boolean) = setShowToolbarImpl(showToolbar)

    fun setToolbarPlacement(rawToolbarPlacement: String?) = setToolbarPlacementImpl(rawToolbarPlacement)

    fun setAllowImageResizing(allowImageResizing: Boolean) = setAllowImageResizingImpl(allowImageResizing)

    fun setToolbarItemsJson(toolbarItemsJson: String?) = setToolbarItemsJsonImpl(toolbarItemsJson)

    fun setToolbarFrameJson(toolbarFrameJson: String?) = setToolbarFrameJsonImpl(toolbarFrameJson)

    fun setPendingEditorUpdateJson(editorUpdateJson: String?) =
        setPendingEditorUpdateJsonImpl(editorUpdateJson)

    fun setPendingEditorUpdateEditorHandle(editorUpdateEditorHandle: String?) =
        setPendingEditorUpdateEditorHandleImpl(editorUpdateEditorHandle)

    /** Internal widget/test hook; production props always use decimal handles. */
    internal fun setPendingEditorUpdateEditorId(viewToken: Long?) =
        setPendingEditorUpdateEditorIdImpl(viewToken)

    fun setPendingEditorUpdateRevision(editorUpdateRevision: Long) =
        setPendingEditorUpdateRevisionImpl(editorUpdateRevision)

    fun setPendingEditorResetUpdateJson(editorResetUpdateJson: String?) =
        setPendingEditorResetUpdateJsonImpl(editorResetUpdateJson)

    fun setPendingEditorResetUpdateEditorHandle(editorResetUpdateEditorHandle: String?) =
        setPendingEditorResetUpdateEditorHandleImpl(editorResetUpdateEditorHandle)

    /** Internal widget/test hook; production props always use decimal handles. */
    internal fun setPendingEditorResetUpdateEditorId(viewToken: Long?) =
        setPendingEditorResetUpdateEditorIdImpl(viewToken)

    fun setPendingEditorResetUpdateRevision(editorResetUpdateRevision: Long) =
        setPendingEditorResetUpdateRevisionImpl(editorResetUpdateRevision)

    fun applyPendingEditorResetUpdateIfNeeded() = applyPendingEditorResetUpdateIfNeededImpl()

    fun applyPendingEditorUpdateIfNeeded() = applyPendingEditorUpdateIfNeededImpl()

    fun focus() = focusImpl()

    fun blur() = blurImpl()

    fun getCaretRectJson(): String? = getCaretRectJsonImpl()

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        handleAttachedToWindow()
    }

    internal fun handleEditorDestroyed(editorId: Long) = handleEditorDestroyedImpl(editorId)

    override fun onDetachedFromWindow() {
        prepareForDetachFromWindow()
        richTextView.editorEditText.retireInputConnectionForHostDetach()
        super.onDetachedFromWindow()
        handleDetachedFromWindow()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) {
            super.onMeasure(widthMeasureSpec, heightMeasureSpec)
            return
        }

        val childWidthSpec = getChildMeasureSpec(
            widthMeasureSpec,
            paddingLeft + paddingRight,
            richTextView.layoutParams.width
        )
        val childHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        richTextView.measure(childWidthSpec, childHeightSpec)

        val measuredWidth = resolveSize(
            richTextView.measuredWidth + paddingLeft + paddingRight,
            widthMeasureSpec
        )
        val desiredHeight = richTextView.measuredHeight + paddingTop + paddingBottom
        val measuredHeight = when (MeasureSpec.getMode(heightMeasureSpec)) {
            MeasureSpec.AT_MOST -> desiredHeight.coerceAtMost(MeasureSpec.getSize(heightMeasureSpec))
            else -> desiredHeight
        }
        setMeasuredDimension(measuredWidth, measuredHeight)
        emitContentHeightIfNeeded(force = false)
    }

    /**
     * Auto-grow measures content-sized because RN's exact specs can be stale,
     * zero, or oversized. The frame it actually assigns is only trustworthy
     * here, so a taller one is filled now — otherwise the extra space a
     * minimum height creates belongs to no view and cannot take a tap.
     */
    override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) {
        super.onLayout(changed, left, top, right, bottom)
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) return
        val available = (bottom - top) - paddingTop - paddingBottom
        if (available <= richTextView.height) return
        richTextView.layout(
            richTextView.left,
            paddingTop,
            richTextView.right,
            paddingTop + available,
        )
    }

    /** Applies an editor update from JS without echoing it back through events. */
    fun applyEditorUpdate(updateJson: String): Boolean = applyEditorUpdateImpl(updateJson)

    /** Applies a reset-style update from JS, discarding pending native composition. */
    fun applyEditorResetUpdate(updateJson: String): Boolean = applyEditorResetUpdateImpl(updateJson)

    @Synchronized
    internal fun markRemoteCommitRebaseScheduled(editorId: Long): Boolean =
        markRemoteCommitRebaseScheduledImpl(editorId)

    internal fun applyRemoteCommitRefresh(expectedEditorId: Long) =
        applyRemoteCommitRefreshImpl(expectedEditorId)

    fun prepareForEditorCommandJSON(): String = prepareForEditorCommandJSONImpl()

    override fun onSelectionChanged(anchor: Int, head: Int) {
        val stateJson = refreshToolbarStateFromEditorSelection()
        refreshMentionQuery()
        clearPendingNativeActionRetryIfScopeChanged()
        schedulePendingPreflightWake()
        richTextView.refreshRemoteSelections()
        val event = mutableMapOf<String, Any>(
            "anchor" to anchor,
            "head" to head,
            "editorId" to eventEditorId(richTextView.editorId)
        )
        lastDocumentVersion?.let {
            event["documentVersion"] = it
        }
        if (stateJson != null) {
            event["stateJson"] = stateJson
        }
        onSelectionChangeForTesting?.invoke(event) ?: onSelectionChange(event)
    }

    override fun onEditorUpdate(updateJSON: String) {
        val documentRevision = documentVersionFromUpdateJSON(updateJSON)
        if (documentRevision == null) {
            richTextView.editorEditText.recordImeTraceForTesting(
                "nativeViewEditorUpdateSkipped",
                "reason=invalidDocumentRevision jsonLength=${updateJSON.length}"
            )
            return
        }
        renderedDocumentRevision = documentRevision
        val sourceEditorId = eventEditorId(richTextView.editorId)
        val adapter = EditorV2Registry.adapterForViewToken(richTextView.editorId)
        val cachedAtomicUpdateJSON =
            adapter?.atomicRenderJson(matchingDocumentRevision = documentRevision)
        if (adapter != null && cachedAtomicUpdateJSON == null) {
            richTextView.editorEditText.recordImeTraceForTesting(
                "nativeViewEditorUpdateSkipped",
                "reason=missingAtomicSnapshot documentRevision=$documentRevision"
            )
            return
        }
        val atomicUpdateJSON = cachedAtomicUpdateJSON ?: updateJSON
        if (isApplyingJSUpdate) {
            dispatchEditorUpdate(
                PendingEditorUpdateEvent(
                    editorId = sourceEditorId,
                    documentRevision = documentRevision,
                    viewUpdateJSON = updateJSON,
                    atomicUpdateJSON = atomicUpdateJSON
                ),
                emitToJS = false
            )
            return
        }
        val event = PendingEditorUpdateEvent(
                editorId = sourceEditorId,
                documentRevision = documentRevision,
                viewUpdateJSON = updateJSON,
                atomicUpdateJSON = atomicUpdateJSON
            )
        val key = NativeCommitKey(event.editorId, event.documentRevision)
        if (!pendingEditorUpdateKeys.add(key)) return
        pendingEditorUpdateEvents.addLast(event)
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateQueued",
            "queue=${pendingEditorUpdateEvents.size} jsonLength=${updateJSON.length}"
        )
        schedulePendingEditorUpdateDispatch()
    }

    override fun onExternalTextCompositionEnded(resultJson: String) {
        val sessionId = runCatching {
            JSONObject(resultJson).opt("sessionId") as? String
        }.getOrNull()
        val matchingComposition = activeExternalTextComposition?.takeIf {
            it.sessionId == sessionId
        }
        if (matchingComposition != null) {
            activeExternalTextComposition = null
        }
        val payload = mapOf<String, Any>(
            "editorId" to (matchingComposition?.editorId ?: eventEditorId(richTextView.editorId)),
            "resultJson" to resultJson,
        )
        onExternalTextCompositionEndForTesting?.invoke(payload)
            ?: onExternalTextCompositionEnd(payload)
        wakePendingPreflightWork()
    }

    internal fun pendingEditorUpdateEventCountForTesting(): Int =
        pendingEditorUpdateEventCountForTestingImpl()

    internal fun pendingEditorErrorEventCountForTesting(): Int = pendingEditorErrorEventCountForTestingImpl()

    internal fun editorErrorCallbackTokenForTesting(): Long? = editorErrorCallbackTokenForTestingImpl()

    internal fun prepareOutsideTapDecisionForWindowEvent(event: MotionEvent): NativeEditorOutsideTapDecision =
        prepareOutsideTapDecisionForWindowEventImpl(event)

    internal fun handleOutsideTapDecisionFromWindowDispatcher(decision: NativeEditorOutsideTapDecision) =
        handleOutsideTapDecisionFromWindowDispatcherImpl(decision)

    internal fun scheduleOutsideTapBlurFromWindowDispatcher() =
        scheduleOutsideTapBlurFromWindowDispatcherImpl()

    internal fun cancelOutsideTapBlurFromWindowDispatcher() = cancelOutsideTapBlurFromWindowDispatcherImpl()

    internal fun markRecentToolbarTouchForTesting() = markRecentToolbarTouchForTestingImpl()

    internal fun shouldPreserveFocusAfterToolbarTouchForTesting(): Boolean =
        shouldPreserveFocusAfterToolbarTouchForTestingImpl()

    internal fun setEditorFocusedForOutsideTapDecisionForTesting(isFocused: Boolean?) =
        setEditorFocusedForOutsideTapDecisionForTestingImpl(isFocused)

    internal fun setAttachedToNativeWindowForTesting(isAttached: Boolean) =
        setAttachedToNativeWindowForTestingImpl(isAttached)

    internal fun handleAttachedToWindowForTesting() = handleAttachedToWindowForTestingImpl()

    internal fun traceOutsideTap(message: String) = traceOutsideTapImpl(message)

    internal fun handleDetachedFromWindowForTesting() = handleDetachedFromWindowForTestingImpl()

    internal fun performBlurForTesting(deferKeyboardDismiss: Boolean = false) =
        performBlurForTestingImpl(deferKeyboardDismiss)

    internal fun pendingBlurRetryAttemptsForTesting(): Int = pendingBlurRetryAttemptsForTestingImpl()

    internal fun pendingDetachPreflightRetryAttemptsForTesting(): Int =
        pendingDetachPreflightRetryAttemptsForTestingImpl()

    internal fun hasPendingOutsideTapBlurForTesting(): Boolean = hasPendingOutsideTapBlurForTestingImpl()

    internal fun isOutsideTapBlurHandlerInstalledForTesting(): Boolean =
        isOutsideTapBlurHandlerInstalledForTestingImpl()

    internal fun hasPendingKeyboardDismissForTesting(): Boolean = hasPendingKeyboardDismissForTestingImpl()

    internal fun hasPendingPreflightWakeForTesting(): Boolean = hasPendingPreflightWakeForTestingImpl()

    internal fun hasPendingToolbarRefocusForTesting(): Boolean = hasPendingToolbarRefocusForTestingImpl()

    internal fun isKeyboardToolbarAttachedForTesting(): Boolean = isKeyboardToolbarAttachedForTestingImpl()

    internal fun currentImeBottomForTesting(): Int = currentImeBottomForTestingImpl()

    internal fun setCurrentImeBottomForTesting(bottom: Int) = setCurrentImeBottomForTestingImpl(bottom)

    internal fun updateAttachedKeyboardToolbarForInsetsForTesting() =
        updateAttachedKeyboardToolbarForInsetsForTestingImpl()

    internal fun scheduleToolbarRefocusForTesting() = scheduleToolbarRefocusForTestingImpl()

    internal fun focusFromToolbarPreserveForTesting() = focusFromToolbarPreserveForTestingImpl()

    internal fun applyAutoFocusForTesting() = applyAutoFocusForTestingImpl()

    internal fun installOutsideTapBlurHandlerForTesting() = installOutsideTapBlurHandlerForTestingImpl()

    internal fun uninstallOutsideTapBlurHandlerForTesting() = uninstallOutsideTapBlurHandlerForTestingImpl()

    internal fun setOutsideTapCycleBreakDispatcherForTesting(
        dispatcher: ((MotionEvent) -> Boolean)?
    ): Boolean =
        setOutsideTapCycleBreakDispatcherForTestingImpl(dispatcher)

    internal fun clearOutsideTapRouteViewReferenceAndReconcileForTesting():
        NativeEditorOutsideTapRouteTestState =
        clearOutsideTapRouteViewReferenceAndReconcileForTestingImpl()

    internal fun dispatchOutsideTapWindowEventForTesting(event: MotionEvent): Boolean =
        dispatchOutsideTapWindowEventForTestingImpl(event)

    internal fun schedulePendingPreflightWakeForTesting() = schedulePendingPreflightWakeForTestingImpl()

    internal fun hasPendingNativeActionForTesting(): Boolean = hasPendingNativeActionForTestingImpl()

    internal fun pendingNativeActionRetryAttemptsForTesting(): Int =
        pendingNativeActionRetryAttemptsForTestingImpl()

    internal fun lastDocumentVersionForTesting(): String? = lastDocumentVersionForTestingImpl()

    internal fun setLastDocumentVersionForTesting(documentVersion: String?) =
        setLastDocumentVersionForTestingImpl(documentVersion)

    internal fun refreshToolbarStateFromEditorSelectionForTesting(): String? =
        refreshToolbarStateFromEditorSelectionForTestingImpl()

    internal fun handleToolbarItemPressForTesting(item: NativeToolbarItem) =
        handleToolbarItemPressForTestingImpl(item)

    internal fun insertMentionSuggestionForTesting(suggestion: NativeMentionSuggestion) =
        insertMentionSuggestionForTestingImpl(suggestion)

    internal fun wakePendingPreflightWorkForTesting() = wakePendingPreflightWorkForTestingImpl()

    internal fun emitEditorReadyForTesting(editorUpdateRevision: Long? = null): Boolean =
        emitEditorReadyForTestingImpl(editorUpdateRevision)

    internal fun pendingEditorUpdateJsonForTesting(): String? = pendingEditorUpdateJsonForTestingImpl()

    internal fun pendingEditorUpdateRevisionForTesting(): Long = pendingEditorUpdateRevisionForTestingImpl()

    internal fun pendingEditorResetUpdateJsonForTesting(): String? =
        pendingEditorResetUpdateJsonForTestingImpl()

    internal fun pendingEditorResetUpdateRevisionForTesting(): Long =
        pendingEditorResetUpdateRevisionForTestingImpl()

    internal fun setAppliedEditorUpdateRevisionForTesting(editorUpdateRevision: Long) =
        setAppliedEditorUpdateRevisionForTestingImpl(editorUpdateRevision)

    internal fun pendingEditorUpdateEditorIdForTesting(): Long? = pendingEditorUpdateEditorIdForTestingImpl()

    internal fun pendingEditorResetUpdateEditorIdForTesting(): Long? =
        pendingEditorResetUpdateEditorIdForTestingImpl()

    internal fun pendingViewCommandUpdateJsonForTesting(): String? =
        pendingViewCommandUpdateJsonForTestingImpl()

    internal fun pendingViewCommandUpdateRetryAttemptsForTesting(): Int =
        pendingViewCommandUpdateRetryAttemptsForTestingImpl()

    internal fun scheduleViewCommandUpdateRetryForTesting(updateJson: String) =
        scheduleViewCommandUpdateRetryForTestingImpl(updateJson)

    internal fun pendingThemeJsonForTesting(): String? = pendingThemeJsonForTestingImpl()

    internal fun pendingAtomsJsonForTesting(): String? = pendingAtomsJsonForTestingImpl()

    internal fun lastAtomsJsonForTesting(): String? = lastAtomsJsonForTestingImpl()

    internal fun lastThemeJsonForTesting(): String? = lastThemeJsonForTestingImpl()

    internal fun pendingThemeRetryAttemptsForTesting(): Int = pendingThemeRetryAttemptsForTestingImpl()

    internal fun applyPendingThemeForTesting() = applyPendingThemeForTestingImpl()

    internal fun isPointInsideStandaloneToolbarForTesting(
        rawX: Float,
        rawY: Float,
        windowOriginOnScreen: Point
    ): Boolean =
        isPointInsideStandaloneToolbarForTestingImpl(rawX, rawY, windowOriginOnScreen)

    internal companion object {
        internal const val TOOLBAR_HIT_SLOP_DP = 8f
        internal const val TOOLBAR_FOCUS_PRESERVE_MS = 750L
        internal const val OUTSIDE_TAP_BLUR_DELAY_MS = 100L
        internal const val OUTSIDE_TAP_HANDLER_INSTALL_RETRY_DELAY_MS = 64L
        internal const val NATIVE_ACTION_RETRY_DELAY_MS = 16L
        internal const val EDITOR_UPDATE_EVENT_DEBOUNCE_MS = 64L
        internal const val PENDING_UPDATE_RECOVERY_RETRY_DELAY_MS = 250L
        internal const val MAX_NATIVE_ACTION_RETRY_ATTEMPTS = 3
        internal const val MAX_PENDING_UPDATE_RETRY_ATTEMPTS = 5
        internal const val LOG_TAG = "NativeEditor"

        internal fun nanosToMicros(nanos: Long): Long = nanos / 1_000L
    }

    internal fun addNativeAtomView(child: View, index: Int) = super.addView(child, index)


}
