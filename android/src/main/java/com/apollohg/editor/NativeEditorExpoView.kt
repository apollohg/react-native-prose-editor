package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Rect
import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.ViewGroup
import android.view.ViewTreeObserver
import android.view.Window
import android.view.inputmethod.InputMethodManager
import android.widget.FrameLayout
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import org.json.JSONArray
import org.json.JSONObject
import java.lang.ref.WeakReference
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.WeakHashMap
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference
import uniffi.editor_core.*

private const val DESTROY_INVALIDATION_AWAIT_TIMEOUT_MS = 250L
private const val OUTSIDE_TAP_GESTURE_CONFIRM_DELAY_MS = 150L

internal enum class NativeEditorOutsideTapDecision {
    IGNORE,
    PRESERVE_FOCUS,
    OUTSIDE_EDITOR
}

private class WeakNativeEditorExpoView private constructor(
    val view: WeakReference<NativeEditorExpoView?>
) {
    constructor(view: NativeEditorExpoView) : this(WeakReference(view))

    companion object {
        fun cleared(): WeakNativeEditorExpoView =
            WeakNativeEditorExpoView(WeakReference<NativeEditorExpoView?>(null))
    }
}

internal object NativeEditorViewRegistry {
    private data class CommandPreparationSnapshot(
        val view: NativeEditorExpoView?,
        val isDetached: Boolean,
        val isDestroyed: Boolean
    )

    private val viewsByEditorId = mutableMapOf<Long, WeakNativeEditorExpoView>()
    private val detachedEditorOwnersByEditorId = mutableMapOf<Long, WeakNativeEditorExpoView>()
    private val destroyedEditorIds = mutableSetOf<Long>()
    private val mainHandler = Handler(Looper.getMainLooper())

    @Synchronized
    fun markEditorCreated(editorId: Long) {
        if (editorId == 0L) return
        destroyedEditorIds.remove(editorId)
    }

    @Synchronized
    fun register(editorId: Long, view: NativeEditorExpoView): Boolean {
        if (editorId == 0L) return false
        if (destroyedEditorIds.contains(editorId)) return false
        viewsByEditorId[editorId] = WeakNativeEditorExpoView(view)
        detachedEditorOwnersByEditorId.remove(editorId)
        return true
    }

    @Synchronized
    fun unregister(
        editorId: Long,
        view: NativeEditorExpoView,
        blockCommandsUntilRegistered: Boolean = false
    ) {
        if (editorId == 0L) return
        val registeredView = viewsByEditorId[editorId]?.view?.get()
        if (registeredView === view) {
            viewsByEditorId.remove(editorId)
        }
        if (blockCommandsUntilRegistered) {
            detachedEditorOwnersByEditorId[editorId] = WeakNativeEditorExpoView(view)
        } else {
            val detachedOwner = detachedEditorOwnersByEditorId[editorId]?.view?.get()
            if (registeredView === view || detachedOwner === view) {
                detachedEditorOwnersByEditorId.remove(editorId)
            }
        }
    }

    @Synchronized
    fun isDestroyed(editorId: Long): Boolean = destroyedEditorIds.contains(editorId)

    @Synchronized
    internal fun forceDetachedOwnerClearedForTesting(editorId: Long) {
        detachedEditorOwnersByEditorId[editorId] = WeakNativeEditorExpoView.cleared()
    }

    fun invalidateDestroyedEditor(editorId: Long) {
        if (editorId == 0L) return
        val affectedViews = synchronized(this) {
            destroyedEditorIds.add(editorId)
            val views = listOfNotNull(
                viewsByEditorId.remove(editorId)?.view?.get(),
                detachedEditorOwnersByEditorId.remove(editorId)?.view?.get()
            ).distinct()
            views
        }
        if (affectedViews.isEmpty()) return
        val invalidate = Runnable {
            affectedViews.forEach { view ->
                view.handleEditorDestroyed(editorId)
            }
        }
        if (Looper.myLooper() == Looper.getMainLooper()) {
            invalidate.run()
        } else {
            val latch = CountDownLatch(1)
            val posted = mainHandler.post {
                try {
                    invalidate.run()
                } finally {
                    latch.countDown()
                }
            }
            if (!posted) return
            try {
                latch.await(DESTROY_INVALIDATION_AWAIT_TIMEOUT_MS, TimeUnit.MILLISECONDS)
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            } catch (_: Throwable) {
                return
            }
        }
    }

    fun prepareForCommandJSON(editorId: Long): String {
        val prepare = {
            val snapshot = synchronized(this) {
                val isDestroyed = destroyedEditorIds.contains(editorId)
                if (isDestroyed) {
                    return@synchronized CommandPreparationSnapshot(
                        view = null,
                        isDetached = false,
                        isDestroyed = true
                    )
                }
                val candidate = viewsByEditorId[editorId]?.view?.get()
                if (candidate == null) {
                    viewsByEditorId.remove(editorId)
                }
                val detachedOwner = detachedEditorOwnersByEditorId[editorId]?.view?.get()
                val isDetached = if (detachedOwner == null) {
                    detachedEditorOwnersByEditorId.remove(editorId)
                    false
                } else {
                    true
                }
                CommandPreparationSnapshot(
                    view = candidate,
                    isDetached = isDetached,
                    isDestroyed = false
                )
            }
            snapshot.view?.prepareForEditorCommandJSON()
                ?: commandPreparationJSON(
                    ready = !snapshot.isDetached && !snapshot.isDestroyed,
                    blockedReason = if (snapshot.isDestroyed) {
                        "destroyed"
                    } else if (snapshot.isDetached) {
                        "detached"
                    } else {
                        null
                    }
                )
        }

        if (Looper.myLooper() == Looper.getMainLooper()) {
            return prepare()
        }

        val result = AtomicReference(commandPreparationJSON(ready = false, blockedReason = "unknown"))
        val state = AtomicInteger(PREFLIGHT_STATE_QUEUED)
        val latch = CountDownLatch(1)
        if (!mainHandler.post {
            try {
                if (state.compareAndSet(PREFLIGHT_STATE_QUEUED, PREFLIGHT_STATE_RUNNING)) {
                    result.set(prepare())
                    state.set(PREFLIGHT_STATE_DONE)
                }
            } finally {
                latch.countDown()
            }
        }) {
            return commandPreparationJSON(ready = false, blockedReason = "unknown")
        }
        try {
            if (!latch.await(DESTROY_INVALIDATION_AWAIT_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
                if (state.compareAndSet(PREFLIGHT_STATE_QUEUED, PREFLIGHT_STATE_CANCELLED)) {
                    return commandPreparationJSON(ready = false, blockedReason = "unknown")
                }
                if (state.get() == PREFLIGHT_STATE_RUNNING) {
                    latch.await()
                    return result.get()
                }
                return commandPreparationJSON(ready = false, blockedReason = "unknown")
            }
        } catch (_: InterruptedException) {
            var interrupted = true
            if (state.compareAndSet(PREFLIGHT_STATE_QUEUED, PREFLIGHT_STATE_CANCELLED)) {
                Thread.currentThread().interrupt()
                return commandPreparationJSON(ready = false, blockedReason = "unknown")
            }
            while (state.get() == PREFLIGHT_STATE_RUNNING) {
                try {
                    latch.await()
                    break
                } catch (_: InterruptedException) {
                    interrupted = true
                }
            }
            if (interrupted) {
                Thread.currentThread().interrupt()
            }
            return if (state.get() == PREFLIGHT_STATE_DONE) {
                result.get()
            } else {
                commandPreparationJSON(ready = false, blockedReason = "unknown")
            }
        }
        return result.get()
    }

    fun commandPreparationJSON(
        ready: Boolean,
        updateJSON: String? = null,
        blockedReason: String? = null
    ): String {
        return JSONObject().apply {
            put("ready", ready)
            if (updateJSON != null) {
                put("updateJSON", updateJSON)
            }
            if (!ready && blockedReason != null) {
                put("blockedReason", blockedReason)
            }
        }.toString()
    }

    private const val PREFLIGHT_STATE_QUEUED = 0
    private const val PREFLIGHT_STATE_RUNNING = 1
    private const val PREFLIGHT_STATE_CANCELLED = 2
    private const val PREFLIGHT_STATE_DONE = 3
}

private object NativeEditorOutsideTapDispatcher {
    private val dispatchers = WeakHashMap<Window, OutsideTapTouchDispatcher>()

    fun register(window: Window, view: NativeEditorExpoView): Boolean {
        val host = contentRootFor(window)
        if (host == null) {
            view.traceOutsideTap("register skipped missing content root")
            return false
        }
        val previousDispatcher = dispatchers[window]
        val dispatcher = if (previousDispatcher?.host === host) {
            previousDispatcher
        } else {
            OutsideTapTouchDispatcher(host).also { nextDispatcher ->
                previousDispatcher?.transferViewsTo(nextDispatcher)
                previousDispatcher?.detach()
                dispatchers[window] = nextDispatcher
            }
        }
        dispatchers[window] = dispatcher
        dispatcher.add(view)
        view.traceOutsideTap(
            "register overlayAttached=${dispatcher.isAttached()} " +
                "host=${host.javaClass.name} " +
                "activeViews=${dispatcher.liveViews().size}"
        )
        return dispatcher.isAttached()
    }

    fun unregister(window: Window, view: NativeEditorExpoView) {
        val dispatcher = dispatchers[window] ?: return
        if (!dispatcher.remove(view)) return
        dispatcher.detach()
        dispatchers.remove(window)
    }

    private fun contentRootFor(window: Window): ViewGroup? {
        val decorView = window.decorView
        return decorView.findViewById<View>(android.R.id.content) as? ViewGroup
            ?: decorView as? ViewGroup
    }

    private class OutsideTapTouchDispatcher(
        val host: ViewGroup
    ) : View.OnTouchListener {
        private data class OutsideTapCandidate(
            val view: WeakReference<NativeEditorExpoView>,
            val downRawX: Float,
            val downRawY: Float,
            val editorRectOnDown: Rect?,
            val confirm: Runnable
        )

        private val views = mutableListOf<WeakReference<NativeEditorExpoView>>()
        private val pendingOutsideTapCandidates = mutableListOf<OutsideTapCandidate>()
        private val touchSlopPx = ViewConfiguration.get(host.context).scaledTouchSlop
        private val scrollChangedListener = ViewTreeObserver.OnScrollChangedListener {
            cancelPendingOutsideTapCandidates("scroll")
        }
        private var scrollListenerTreeObserver: ViewTreeObserver? = null
        private var attachmentGeneration = 0
        private val observerView = View(host.context).apply {
            isClickable = false
            isFocusable = false
            isFocusableInTouchMode = false
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }

        init {
            observerView.setOnTouchListener(this)
            attach()
        }

        fun add(view: NativeEditorExpoView) {
            prune()
            attach()
            if (views.any { it.get() === view }) return
            views.add(WeakReference(view))
        }

        fun liveViews(): List<NativeEditorExpoView> {
            prune()
            return views.mapNotNull { it.get() }
        }

        fun transferViewsTo(target: OutsideTapTouchDispatcher) {
            liveViews().forEach { target.add(it) }
            views.clear()
            cancelPendingOutsideTapCandidates("transfer")
        }

        fun remove(view: NativeEditorExpoView): Boolean {
            cancelPendingOutsideTapCandidatesFor(view, "remove view")
            views.removeAll { it.get()?.let { candidate -> candidate === view } != false }
            return views.isEmpty()
        }

        override fun onTouch(view: View, event: MotionEvent): Boolean {
            val activeViews = liveViews()
            if (activeViews.isEmpty()) {
                return false
            }

            when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> handleActionDown(activeViews, event)
                MotionEvent.ACTION_MOVE -> {
                    if (hasMovedBeyondTapSlop(event)) {
                        cancelPendingOutsideTapCandidates("move")
                    }
                }
                MotionEvent.ACTION_UP -> {
                    if (hasMovedBeyondTapSlop(event)) {
                        cancelPendingOutsideTapCandidates("up moved")
                    } else {
                        confirmPendingOutsideTapCandidates("up")
                    }
                }
                MotionEvent.ACTION_CANCEL -> cancelPendingOutsideTapCandidates("cancel")
            }
            return false
        }

        private fun handleActionDown(activeViews: List<NativeEditorExpoView>, event: MotionEvent) {
            cancelPendingOutsideTapCandidates("new down")
            val decisions = activeViews.map { view ->
                view to view.prepareOutsideTapDecisionForWindowEvent(event)
            }
            decisions.forEach { (view, decision) ->
                view.traceOutsideTap(
                    "dispatch overlay action=${event.action} raw=${event.rawX.toInt()},${event.rawY.toInt()} decision=$decision"
                )
                if (decision == NativeEditorOutsideTapDecision.OUTSIDE_EDITOR) {
                    scheduleOutsideTapCandidate(view, event)
                } else {
                    view.handleOutsideTapDecisionFromWindowDispatcher(decision)
                }
            }
        }

        private fun scheduleOutsideTapCandidate(view: NativeEditorExpoView, event: MotionEvent) {
            val editorRect = Rect()
            val editorRectOnDown = if (
                view.richTextView.editorEditText.getGlobalVisibleRect(editorRect) &&
                !editorRect.isEmpty
            ) {
                editorRect
            } else {
                null
            }
            val viewRef = WeakReference(view)
            lateinit var candidate: OutsideTapCandidate
            val confirm = Runnable {
                confirmOutsideTapCandidate(candidate, "delay")
            }
            candidate = OutsideTapCandidate(
                view = viewRef,
                downRawX = event.rawX,
                downRawY = event.rawY,
                editorRectOnDown = editorRectOnDown,
                confirm = confirm
            )
            pendingOutsideTapCandidates.add(candidate)
            ensureScrollListener()
            view.traceOutsideTap("candidate outside tap")
            observerView.postDelayed(confirm, OUTSIDE_TAP_GESTURE_CONFIRM_DELAY_MS)
        }

        private fun confirmPendingOutsideTapCandidates(reason: String) {
            val candidates = pendingOutsideTapCandidates.toList()
            candidates.forEach { candidate ->
                confirmOutsideTapCandidate(candidate, reason)
            }
        }

        private fun confirmOutsideTapCandidate(candidate: OutsideTapCandidate, reason: String) {
            if (!pendingOutsideTapCandidates.remove(candidate)) return
            removeScrollListenerIfIdle()
            observerView.removeCallbacks(candidate.confirm)
            val view = candidate.view.get() ?: return
            if (editorMovedBeyondTapSlop(view, candidate)) {
                view.traceOutsideTap("cancel outside tap candidate reason=$reason moved")
                return
            }
            view.traceOutsideTap("confirm outside tap candidate reason=$reason")
            view.handleOutsideTapDecisionFromWindowDispatcher(NativeEditorOutsideTapDecision.OUTSIDE_EDITOR)
        }

        private fun hasMovedBeyondTapSlop(event: MotionEvent): Boolean =
            pendingOutsideTapCandidates.any { candidate ->
                val dx = event.rawX - candidate.downRawX
                val dy = event.rawY - candidate.downRawY
                dx * dx + dy * dy > touchSlopPx * touchSlopPx
            }

        private fun editorMovedBeyondTapSlop(
            view: NativeEditorExpoView,
            candidate: OutsideTapCandidate
        ): Boolean {
            val editorRectOnDown = candidate.editorRectOnDown ?: return false
            val currentRect = Rect()
            if (!view.richTextView.editorEditText.getGlobalVisibleRect(currentRect)) {
                return true
            }
            val dx = currentRect.left - editorRectOnDown.left
            val dy = currentRect.top - editorRectOnDown.top
            return dx * dx + dy * dy > touchSlopPx * touchSlopPx
        }

        private fun cancelPendingOutsideTapCandidatesFor(view: NativeEditorExpoView, reason: String) {
            val candidates = pendingOutsideTapCandidates.toList()
            candidates.forEach { candidate ->
                if (candidate.view.get() === view) {
                    pendingOutsideTapCandidates.remove(candidate)
                    observerView.removeCallbacks(candidate.confirm)
                    view.traceOutsideTap("cancel outside tap candidate reason=$reason")
                }
            }
            removeScrollListenerIfIdle()
        }

        private fun cancelPendingOutsideTapCandidates(reason: String) {
            val candidates = pendingOutsideTapCandidates.toList()
            pendingOutsideTapCandidates.clear()
            removeScrollListener()
            candidates.forEach { candidate ->
                observerView.removeCallbacks(candidate.confirm)
                candidate.view.get()?.traceOutsideTap("cancel outside tap candidate reason=$reason")
            }
        }

        private fun ensureScrollListener() {
            val activeObserver = scrollListenerTreeObserver
            if (activeObserver?.isAlive == true && activeObserver === host.viewTreeObserver) {
                return
            }
            removeScrollListener()
            val nextObserver = host.viewTreeObserver
            if (nextObserver.isAlive) {
                nextObserver.addOnScrollChangedListener(scrollChangedListener)
                scrollListenerTreeObserver = nextObserver
            }
        }

        private fun removeScrollListenerIfIdle() {
            if (pendingOutsideTapCandidates.isEmpty()) {
                removeScrollListener()
            }
        }

        private fun removeScrollListener() {
            val observer = scrollListenerTreeObserver
            if (observer?.isAlive == true) {
                observer.removeOnScrollChangedListener(scrollChangedListener)
            }
            scrollListenerTreeObserver = null
        }

        fun isAttached(): Boolean = observerView.parent === host

        fun detach() {
            attachmentGeneration += 1
            val generation = attachmentGeneration
            cancelPendingOutsideTapCandidates("detach")
            val parent = observerView.parent as? ViewGroup ?: return
            observerView.visibility = View.GONE
            parent.post {
                if (generation != attachmentGeneration) return@post
                if (observerView.parent === parent) {
                    parent.removeView(observerView)
                }
            }
        }

        private fun attach() {
            attachmentGeneration += 1
            observerView.visibility = View.VISIBLE
            val parent = observerView.parent as? ViewGroup
            if (parent !== host) {
                if (parent != null) {
                    parent.removeView(observerView)
                }
                host.addView(
                    observerView,
                    ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.MATCH_PARENT
                    )
                )
                return
            }
            if (host.indexOfChild(observerView) != host.childCount - 1) {
                val generation = attachmentGeneration
                host.post {
                    if (generation != attachmentGeneration) return@post
                    if (observerView.parent === host && host.indexOfChild(observerView) != host.childCount - 1) {
                        observerView.bringToFront()
                    }
                }
            }
        }

        private fun prune() {
            views.removeAll { it.get() == null }
        }
    }
}

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

    private enum class ToolbarPlacement {
        KEYBOARD,
        INLINE;

        companion object {
            fun fromRaw(raw: String?): ToolbarPlacement =
                if (raw == "inline") INLINE else KEYBOARD
        }
    }

    private sealed class PendingNativeAction {
        data class ToolbarItemPress(val item: NativeToolbarItem) : PendingNativeAction()
        data class MentionSuggestionSelect(val suggestion: NativeMentionSuggestion) : PendingNativeAction()
    }

    private data class PendingNativeActionScope(
        val editorId: Long,
        val documentVersion: Int?,
        val allowedDocumentVersion: Int?,
        val hadFocus: Boolean,
        val hadVisibleToolbar: Boolean,
        val selectionAnchor: Int?,
        val selectionHead: Int?,
        val mentionAnchor: Int? = null,
        val mentionHead: Int? = null,
        val mentionQuery: String? = null
    )

    private data class PendingEditorUpdateEvent(
        val editorId: Long,
        val updateJSON: String
    )

    val richTextView: RichTextEditorView = RichTextEditorView(context)
    private val keyboardToolbarView = EditorKeyboardToolbarView(context)
    private val mainHandler = Handler(Looper.getMainLooper())

    private val onEditorUpdate by EventDispatcher<Map<String, Any>>()
    private val onSelectionChange by EventDispatcher<Map<String, Any>>()
    private val onFocusChange by EventDispatcher<Map<String, Any>>()
    private val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    private val onEditorReady by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onToolbarAction by EventDispatcher<Map<String, Any>>()
    @Suppress("unused")
    private val onAddonEvent by EventDispatcher<Map<String, Any>>()

    /** Guard flag: when true, editor updates originated from JS and should not echo back. */
    var isApplyingJSUpdate = false
    internal var blockEditorUpdatePreflightForTesting = false
    internal var blockThemePreflightForTesting = false
    internal var onToolbarActionForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onAddonEventForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onSelectionChangeForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onFocusChangeForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onContentHeightChangeForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorUpdateForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorReadyForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onOutsideTapTraceForTesting: ((String) -> Unit)? = null
    internal var onRefreshToolbarStateFromEditorSelectionForTesting: (() -> String?)? = null
    internal var onBeforePrepareForEditorCommandForTesting: (() -> Unit)? = null
    private var isAttachedToNativeWindow = false
    private var didApplyAutoFocus = false
    private var heightBehavior = EditorHeightBehavior.FIXED
    private var lastEmittedContentHeight = 0
    private var lastEmittedContentHeightEditorId: Long? = null
    private var outsideTapWindow: Window? = null
    private var pendingOutsideTapHandlerInstallRetry: Runnable? = null
    private var toolbarFramesInWindow: List<RectF> = emptyList()
    private var lastToolbarTouchUptimeMs: Long? = null
    private var editorFocusedForOutsideTapOverrideForTesting: Boolean? = null
    private var pendingOutsideTapBlur: Runnable? = null
    private var pendingKeyboardDismiss: Runnable? = null
    private var pendingToolbarRefocus: Runnable? = null
    private var pendingToolbarRefocusEditorId: Long? = null
    private var pendingToolbarRefocusGeneration = 0
    private var pendingKeyboardToolbarDetachGeneration = 0
    private var autoFocusRequested = false
    private var addons = NativeEditorAddons(null)
    private var mentionQueryState: MentionQueryState? = null
    private var lastMentionEventJson: String? = null
    private var lastMentionEventEditorId: Long? = null
    private var lastThemeJson: String? = null
    private var pendingThemeJson: String? = null
    private var hasPendingTheme = false
    private var pendingThemeRetryScheduled = false
    private var pendingThemeRetryEditorId: Long? = null
    private var pendingThemeRetryGeneration = 0
    private var pendingThemeRetryAttempts = 0
    private var lastAddonsJson: String? = null
    private var lastRemoteSelectionsJson: String? = null
    private var lastToolbarItemsJson: String? = null
    private var lastToolbarFrameJson: String? = null
    private var lastDocumentVersion: Int? = null
    private var toolbarState = NativeToolbarState.empty
    private var showsToolbar = true
    private var toolbarPlacement = ToolbarPlacement.KEYBOARD
    private var currentImeBottom = 0
    private var pendingEditorUpdateJson: String? = null
    private var pendingEditorUpdateEditorId: Long? = null
    private var pendingEditorUpdateRevision = 0
    private var appliedEditorUpdateRevision = 0
    private var pendingEditorResetUpdateJson: String? = null
    private var pendingEditorResetUpdateEditorId: Long? = null
    private var pendingEditorResetUpdateRevision = 0
    private var appliedEditorResetUpdateRevision = 0
    private var lastEditorResetUpdateJsonProp: String? = null
    private var lastEditorResetUpdateEditorIdProp: Long? = null
    private var pendingEditorUpdateRetryScheduled = false
    private var pendingEditorUpdateRetryEditorId: Long? = null
    private var pendingEditorUpdateRetryGeneration = 0
    private var pendingEditorUpdateRetryAttempts = 0
    private var pendingEditorUpdateForcedRecoveryAttempted = false
    private var pendingViewCommandUpdateJson: String? = null
    private var pendingViewCommandUpdateEditorId: Long? = null
    private var pendingViewCommandUpdateRetryScheduled = false
    private var pendingViewCommandUpdateRetryGeneration = 0
    private var pendingViewCommandUpdateRetryAttempts = 0
    private var pendingPreflightWakeScheduled = false
    private var pendingPreflightWakeGeneration = 0
    private var pendingBlurRetry: Runnable? = null
    private var pendingBlurRetryEditorId: Long? = null
    private var pendingBlurRetryGeneration = 0
    private var pendingBlurRetryAttempts = 0
    private var pendingDetachPreflightRetryScheduled = false
    private var pendingDetachPreflightRetryEditorId: Long? = null
    private var pendingDetachPreflightRetryGeneration = 0
    private var pendingDetachPreflightRetryAttempts = 0
    private var pendingNativeAction: PendingNativeAction? = null
    private var pendingNativeActionScope: PendingNativeActionScope? = null
    private var pendingNativeActionRetryScheduled = false
    private var pendingNativeActionRetryEditorId: Long? = null
    private var pendingNativeActionRetryGeneration = 0
    private var pendingNativeActionRetryAttempts = 0
    private var lastReadyEditorId: Long? = null
    private val pendingEditorUpdateEvents = java.util.ArrayDeque<PendingEditorUpdateEvent>()
    private var pendingEditorUpdateDispatchGeneration = 0
    private var pendingEditorUpdateDispatchScheduled = false

    init {
        addView(richTextView, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        richTextView.editorEditText.editorListener = this
        richTextView.onBeforeDetachedFromWindow = {
            prepareForDetachFromWindow()
        }
        keyboardToolbarView.onPressItem = { item ->
            handleToolbarItemPress(item)
        }
        keyboardToolbarView.onSelectMentionSuggestion = { suggestion ->
            insertMentionSuggestion(suggestion)
        }
        keyboardToolbarView.applyState(toolbarState)
        ViewCompat.setOnApplyWindowInsetsListener(keyboardToolbarView) { _, insets ->
            currentImeBottom = insets.getInsets(WindowInsetsCompat.Type.ime()).bottom
            updateKeyboardToolbarLayout()
            updateAttachedKeyboardToolbarForInsets()
            insets
        }

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
                "editorId" to richTextView.editorId
            )
            onFocusChangeForTesting?.invoke(event) ?: onFocusChange(event)
        }
    }

    fun setEditorId(id: Long) {
        if (id != 0L && NativeEditorViewRegistry.isDestroyed(id)) {
            setEditorId(0L)
            return
        }
        val previousEditorId = richTextView.editorId
        if (previousEditorId != id) {
            invalidateAutoGrowContentHeightEmission()
        }
        if (previousEditorId == id && richTextView.editorEditText.editorId == id) {
            if (id != 0L && isAttachedToNativeWindow) {
                if (!NativeEditorViewRegistry.register(id, this)) {
                    handleEditorDestroyed(id)
                    return
                }
                applyPendingEditorResetUpdateIfNeeded()
                applyPendingEditorUpdateIfNeeded()
                applyPendingThemeIfNeeded()
                refreshReadyStateIfSettled()
                applyAutoFocusIfNeeded()
            } else if (id != 0L) {
                NativeEditorViewRegistry.unregister(
                    id,
                    this,
                    blockCommandsUntilRegistered = true
                )
            }
            return
        }
        if (previousEditorId != id) {
            NativeEditorViewRegistry.unregister(previousEditorId, this)
            lastDocumentVersion = null
            cancelPendingToolbarRefocus()
            cancelPendingEditorUpdateRetry()
            if (pendingEditorUpdateEditorId != null && pendingEditorUpdateEditorId != id) {
                clearPendingEditorUpdateState()
            }
            if (pendingEditorResetUpdateEditorId != null && pendingEditorResetUpdateEditorId != id) {
                clearPendingEditorResetUpdateState()
            }
            appliedEditorUpdateRevision = 0
            appliedEditorResetUpdateRevision = 0
            clearPendingViewCommandUpdateRetry()
            cancelPendingThemeRetry()
            if (hasPendingTheme) {
                pendingThemeRetryEditorId = id
            }
            cancelPendingBlurRetry()
            clearPendingNativeActionRetry()
            clearMentionQueryState(resetLastEvent = true)
            lastReadyEditorId = null
        }
        if (!isAttachedToNativeWindow) {
            richTextView.setEditorIdWhileDetached(id)
            if (id != 0L) {
                NativeEditorViewRegistry.unregister(
                    id,
                    this,
                    blockCommandsUntilRegistered = true
                )
            } else {
                toolbarState = NativeToolbarState.empty
                keyboardToolbarView.applyState(toolbarState)
            }
            return
        }

        if (hasPendingEditorResetUpdateForEditor(id) || hasPendingEditorUpdateForEditor(id)) {
            richTextView.setEditorIdWhileDetached(id)
            richTextView.rebindEditorIfNeeded(notifyListener = false)
        } else {
            richTextView.editorId = id
        }
        if (id != 0L) {
            if (!NativeEditorViewRegistry.register(id, this)) {
                handleEditorDestroyed(id)
                return
            }
        } else {
            toolbarState = NativeToolbarState.empty
            keyboardToolbarView.applyState(toolbarState)
        }
        applyPendingEditorResetUpdateIfNeeded()
        applyPendingEditorUpdateIfNeeded()
        applyPendingThemeIfNeeded()
        refreshReadyStateIfSettled()
        applyAutoFocusIfNeeded()
    }

    fun setThemeJson(themeJson: String?) {
        if (lastThemeJson == themeJson && !hasPendingTheme) return
        pendingThemeJson = themeJson
        hasPendingTheme = true
        pendingThemeRetryEditorId = richTextView.editorId
        pendingThemeRetryAttempts = 0
        applyPendingThemeIfNeeded()
    }

    fun setImageLoadingPolicyJson(policyJson: String?) {
        richTextView.editorEditText.setImageLoadingPolicyJson(policyJson)
    }

    private fun applyThemeJson(themeJson: String?) {
        if (lastThemeJson == themeJson) return
        lastThemeJson = themeJson
        val theme = EditorTheme.fromJson(themeJson)
        richTextView.applyTheme(theme)
        keyboardToolbarView.applyTheme(theme?.toolbar)
        keyboardToolbarView.applyMentionTheme(theme?.mentions ?: addons.mentions?.theme)
        keyboardToolbarView.requestLayout()
        updateKeyboardToolbarLayout()
        updateEditorViewportInset(forceMeasureToolbar = true)
        post {
            updateKeyboardToolbarLayout()
            updateEditorViewportInset(forceMeasureToolbar = true)
        }
    }

    fun setHeightBehavior(rawHeightBehavior: String) {
        val nextBehavior = EditorHeightBehavior.fromRaw(rawHeightBehavior)
        if (heightBehavior == nextBehavior) return
        heightBehavior = nextBehavior
        if (nextBehavior != EditorHeightBehavior.AUTO_GROW) {
            lastEmittedContentHeight = 0
            lastEmittedContentHeightEditorId = null
        }
        richTextView.setHeightBehavior(nextBehavior)
        val params = richTextView.layoutParams as LayoutParams
        params.width = LayoutParams.MATCH_PARENT
        params.height = if (nextBehavior == EditorHeightBehavior.AUTO_GROW) {
            LayoutParams.WRAP_CONTENT
        } else {
            LayoutParams.MATCH_PARENT
        }
        richTextView.layoutParams = params
        requestLayout()
        if (nextBehavior == EditorHeightBehavior.AUTO_GROW) {
            post { emitContentHeightIfNeeded(force = true) }
        }
        updateEditorViewportInset()
    }

    private fun invalidateAutoGrowContentHeightEmission() {
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) return
        lastEmittedContentHeight = 0
        lastEmittedContentHeightEditorId = null
        requestLayout()
    }

    fun setAddonsJson(addonsJson: String?) {
        if (lastAddonsJson == addonsJson) return
        clearPendingNativeActionRetry()
        lastAddonsJson = addonsJson
        addons = NativeEditorAddons.fromJson(addonsJson)
        keyboardToolbarView.applyMentionTheme(richTextView.editorEditText.theme?.mentions ?: addons.mentions?.theme)
        refreshMentionQuery()
    }

    fun setRemoteSelectionsJson(remoteSelectionsJson: String?) {
        if (lastRemoteSelectionsJson == remoteSelectionsJson) return
        lastRemoteSelectionsJson = remoteSelectionsJson
        richTextView.setRemoteSelections(
            RemoteSelectionDecoration.fromJson(context, remoteSelectionsJson)
        )
    }

    fun setAutoFocus(autoFocus: Boolean) {
        autoFocusRequested = autoFocus
        applyAutoFocusIfNeeded()
    }

    private fun applyAutoFocusIfNeeded() {
        if (!autoFocusRequested || didApplyAutoFocus || !canFocusCurrentEditor()) return
        didApplyAutoFocus = true
        focus()
    }

    fun setAutoCapitalize(autoCapitalize: String?) {
        richTextView.editorEditText.setAutoCapitalize(autoCapitalize)
    }

    fun setAutoCorrect(autoCorrect: Boolean?) {
        richTextView.editorEditText.setAutoCorrect(autoCorrect)
    }

    fun setKeyboardType(keyboardType: String?) {
        richTextView.editorEditText.setKeyboardType(keyboardType)
    }

    fun setEditable(editable: Boolean) {
        if (richTextView.editorEditText.isEditable == editable) return
        if (!editable) {
            cancelPendingToolbarRefocus()
            clearPendingNativeActionRetry()
        }
        richTextView.editorEditText.isEditable = editable
        updateKeyboardToolbarVisibility()
    }

    fun setShowToolbar(showToolbar: Boolean) {
        if (showsToolbar == showToolbar) return
        if (!showToolbar) {
            cancelPendingToolbarRefocus()
            clearPendingNativeActionRetry()
        }
        showsToolbar = showToolbar
        updateKeyboardToolbarVisibility()
    }

    fun setToolbarPlacement(rawToolbarPlacement: String?) {
        val nextPlacement = ToolbarPlacement.fromRaw(rawToolbarPlacement)
        if (toolbarPlacement == nextPlacement) return
        if (nextPlacement != ToolbarPlacement.KEYBOARD) {
            cancelPendingToolbarRefocus()
            clearPendingNativeActionRetry()
        }
        toolbarPlacement = nextPlacement
        updateKeyboardToolbarVisibility()
    }

    fun setAllowImageResizing(allowImageResizing: Boolean) {
        richTextView.setImageResizingEnabled(allowImageResizing)
    }

    fun setToolbarItemsJson(toolbarItemsJson: String?) {
        if (lastToolbarItemsJson == toolbarItemsJson) return
        clearPendingNativeActionRetry()
        lastToolbarItemsJson = toolbarItemsJson
        keyboardToolbarView.setItems(NativeToolbarItem.fromJson(toolbarItemsJson))
    }

    fun setToolbarFrameJson(toolbarFrameJson: String?) {
        if (lastToolbarFrameJson == toolbarFrameJson) return
        lastToolbarFrameJson = toolbarFrameJson
        if (toolbarFrameJson.isNullOrBlank()) {
            toolbarFramesInWindow = emptyList()
            return
        }

        toolbarFramesInWindow = try {
            val json = JSONObject(toolbarFrameJson)
            val frames = json.optJSONArray("frames")
            if (frames != null) {
                buildList {
                    for (index in 0 until frames.length()) {
                        frames.optJSONObject(index)?.toToolbarFrame()?.let { add(it) }
                    }
                }
            } else {
                listOfNotNull(json.toToolbarFrame())
            }
        } catch (_: Throwable) {
            emptyList()
        }
    }

    private fun JSONObject.toToolbarFrame(): RectF? {
        val x = optDouble("x", Double.NaN)
        val y = optDouble("y", Double.NaN)
        val width = optDouble("width", Double.NaN)
        val height = optDouble("height", Double.NaN)
        if (
            x.isNaN() || x.isInfinite() ||
            y.isNaN() || y.isInfinite() ||
            width.isNaN() || width.isInfinite() ||
            height.isNaN() || height.isInfinite()
        ) {
            return null
        }
        if (width <= 0.0 || height <= 0.0) {
            return null
        }

        return RectF(
            x.toFloat(),
            y.toFloat(),
            (x + width).toFloat(),
            (y + height).toFloat()
        )
    }

    fun setPendingEditorUpdateJson(editorUpdateJson: String?) {
        pendingEditorUpdateJson = editorUpdateJson
    }

    fun setPendingEditorUpdateEditorId(editorUpdateEditorId: Long?) {
        pendingEditorUpdateEditorId = editorUpdateEditorId
    }

    fun setPendingEditorUpdateRevision(editorUpdateRevision: Int) {
        if (pendingEditorUpdateRevision != editorUpdateRevision) {
            pendingEditorUpdateRetryAttempts = 0
            pendingEditorUpdateForcedRecoveryAttempted = false
        }
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    fun setPendingEditorResetUpdateJson(editorResetUpdateJson: String?) {
        lastEditorResetUpdateJsonProp = editorResetUpdateJson
        pendingEditorResetUpdateJson = editorResetUpdateJson
    }

    fun setPendingEditorResetUpdateEditorId(editorResetUpdateEditorId: Long?) {
        lastEditorResetUpdateEditorIdProp = editorResetUpdateEditorId
        pendingEditorResetUpdateEditorId = editorResetUpdateEditorId
    }

    fun setPendingEditorResetUpdateRevision(editorResetUpdateRevision: Int) {
        if (pendingEditorResetUpdateRevision != editorResetUpdateRevision) {
            pendingEditorUpdateRetryAttempts = 0
            pendingEditorUpdateForcedRecoveryAttempted = false
        }
        if (editorResetUpdateRevision != 0 && pendingEditorResetUpdateJson == null) {
            pendingEditorResetUpdateJson = lastEditorResetUpdateJsonProp
        }
        if (editorResetUpdateRevision != 0 && pendingEditorResetUpdateEditorId == null) {
            pendingEditorResetUpdateEditorId = lastEditorResetUpdateEditorIdProp
        }
        pendingEditorResetUpdateRevision = editorResetUpdateRevision
    }

    private fun hasPendingEditorUpdateForEditor(editorId: Long): Boolean =
        pendingEditorUpdateJson != null &&
            pendingEditorUpdateRevision != 0 &&
            pendingEditorUpdateRevision != appliedEditorUpdateRevision &&
            pendingEditorUpdateEditorId == editorId

    private fun hasPendingEditorResetUpdateForEditor(editorId: Long): Boolean =
        pendingEditorResetUpdateJson != null &&
            pendingEditorResetUpdateRevision != 0 &&
            pendingEditorResetUpdateRevision != appliedEditorResetUpdateRevision &&
            pendingEditorResetUpdateEditorId == editorId

    private fun hasPendingEditorUpdateForCurrentEditor(): Boolean =
        hasPendingEditorUpdateForEditor(richTextView.editorId)

    private fun hasPendingEditorResetUpdateForCurrentEditor(): Boolean =
        hasPendingEditorResetUpdateForEditor(richTextView.editorId)

    private fun pendingEditorUpdateCommandPreparationJSON(): String =
        NativeEditorViewRegistry.commandPreparationJSON(
            ready = false,
            blockedReason = "pendingUpdate"
        )

    private fun shouldBlockEditorCommandForPendingUpdate(): Boolean =
        hasPendingEditorResetUpdateForCurrentEditor() || hasPendingEditorUpdateForCurrentEditor()

    private fun refreshReadyStateIfSettled() {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (hasPendingEditorResetUpdateForCurrentEditor()) return
        if (hasPendingEditorUpdateForCurrentEditor()) return
        if (!isAttachedToNativeWindow) return
        if (richTextView.editorEditText.editorId != richTextView.editorId) return
        refreshToolbarStateFromEditorSelection()
        refreshMentionQuery()
        emitEditorReadyIfNeeded()
    }

    fun applyPendingEditorResetUpdateIfNeeded() {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (pendingEditorResetUpdateRevision == 0) return
        val revision = pendingEditorResetUpdateRevision
        val editorId = richTextView.editorId
        val expectedEditorId = pendingEditorResetUpdateEditorId
        if (expectedEditorId == null) return
        if (expectedEditorId != editorId) return
        if (pendingEditorResetUpdateJson == null) {
            clearPendingEditorResetUpdateState(resetAppliedRevision = false)
            refreshReadyStateIfSettled()
            return
        }
        val updateJson = pendingEditorResetUpdateJson ?: return
        if (revision == appliedEditorResetUpdateRevision) {
            clearPendingEditorResetUpdateState(resetAppliedRevision = false)
            emitEditorReady(editorUpdateRevision = revision)
            refreshReadyStateIfSettled()
            return
        }
        if (editorId != 0L && !isAttachedToNativeWindow) return
        val apply = Runnable {
            if (editorId != richTextView.editorId) return@Runnable
            if (expectedEditorId != richTextView.editorId) return@Runnable
            if (editorId != 0L && !isAttachedToNativeWindow) return@Runnable
            if (revision != pendingEditorResetUpdateRevision) return@Runnable
            if (revision == appliedEditorResetUpdateRevision) {
                clearPendingEditorResetUpdateState(resetAppliedRevision = false)
                emitEditorReady(editorUpdateRevision = revision)
                refreshReadyStateIfSettled()
                return@Runnable
            }
            if (applyEditorResetUpdate(updateJson)) {
                appliedEditorResetUpdateRevision = revision
                clearPendingEditorResetUpdateState(resetAppliedRevision = false)
                emitEditorReady(editorUpdateRevision = revision)
                refreshReadyStateIfSettled()
            } else {
                schedulePendingEditorUpdateRetry()
            }
        }
        if (Looper.myLooper() == Looper.getMainLooper()) {
            apply.run()
        } else if (!post(apply)) {
            richTextView.post(apply)
        }
    }

    fun applyPendingEditorUpdateIfNeeded() {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (pendingEditorUpdateRevision == 0) return
        val revision = pendingEditorUpdateRevision
        val editorId = richTextView.editorId
        val expectedEditorId = pendingEditorUpdateEditorId
        if (expectedEditorId == null) return
        if (expectedEditorId != editorId) return
        if (pendingEditorUpdateJson == null) {
            clearPendingEditorUpdateState(resetAppliedRevision = false)
            refreshReadyStateIfSettled()
            return
        }
        val updateJson = pendingEditorUpdateJson ?: return
        if (pendingEditorUpdateRevision == appliedEditorUpdateRevision) {
            clearPendingEditorUpdateState(resetAppliedRevision = false)
            emitEditorReady(editorUpdateRevision = revision)
            refreshReadyStateIfSettled()
            return
        }
        if (editorId != 0L && !isAttachedToNativeWindow) return
        val apply = Runnable {
            if (editorId != richTextView.editorId) return@Runnable
            if (expectedEditorId != richTextView.editorId) return@Runnable
            if (editorId != 0L && !isAttachedToNativeWindow) return@Runnable
            if (revision != pendingEditorUpdateRevision) return@Runnable
            if (revision == appliedEditorUpdateRevision) {
                clearPendingEditorUpdateState(resetAppliedRevision = false)
                emitEditorReady(editorUpdateRevision = revision)
                refreshReadyStateIfSettled()
                return@Runnable
            }
            if (applyEditorUpdate(updateJson, scheduleViewCommandRetry = false)) {
                appliedEditorUpdateRevision = revision
                pendingEditorUpdateJson = null
                pendingEditorUpdateEditorId = null
                pendingEditorUpdateRevision = 0
                pendingEditorUpdateRetryAttempts = 0
                pendingEditorUpdateForcedRecoveryAttempted = false
                cancelPendingEditorUpdateRetry()
                emitEditorReady(editorUpdateRevision = revision)
                refreshReadyStateIfSettled()
            } else {
                schedulePendingEditorUpdateRetry()
            }
        }
        if (Looper.myLooper() == Looper.getMainLooper()) {
            apply.run()
        } else if (!post(apply)) {
            richTextView.post(apply)
        }
    }

    private fun clearPendingEditorUpdateState(resetAppliedRevision: Boolean = true) {
        pendingEditorUpdateJson = null
        pendingEditorUpdateEditorId = null
        pendingEditorUpdateRevision = 0
        if (resetAppliedRevision) {
            appliedEditorUpdateRevision = 0
        }
        cancelPendingEditorUpdateRetry()
    }

    private fun clearPendingEditorResetUpdateState(resetAppliedRevision: Boolean = true) {
        pendingEditorResetUpdateJson = null
        pendingEditorResetUpdateEditorId = null
        pendingEditorResetUpdateRevision = 0
        if (resetAppliedRevision) {
            appliedEditorResetUpdateRevision = 0
        }
    }

    private fun cancelPendingEditorUpdateRetry() {
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = null
        pendingEditorUpdateRetryAttempts = 0
        pendingEditorUpdateForcedRecoveryAttempted = false
        pendingEditorUpdateRetryGeneration += 1
    }

    private fun schedulePendingEditorUpdateRetry() {
        if (pendingEditorUpdateRetryScheduled) return
        val pastFastRetryBudget =
            pendingEditorUpdateRetryAttempts >= MAX_PENDING_UPDATE_RETRY_ATTEMPTS
        if (
            pastFastRetryBudget &&
            !pendingEditorUpdateForcedRecoveryAttempted &&
            richTextView.editorId != 0L &&
            richTextView.editorEditText.editorId == richTextView.editorId
        ) {
            pendingEditorUpdateForcedRecoveryAttempted = true
            richTextView.editorEditText.discardTransientNativeInputForExternalRecovery()
        }
        if (!pastFastRetryBudget) {
            pendingEditorUpdateRetryAttempts += 1
        }
        pendingEditorUpdateRetryEditorId = richTextView.editorId
        pendingEditorUpdateRetryScheduled = true
        pendingEditorUpdateRetryGeneration += 1
        val retryGeneration = pendingEditorUpdateRetryGeneration
        val delayMs = if (pastFastRetryBudget) {
            PENDING_UPDATE_RECOVERY_RETRY_DELAY_MS
        } else {
            NATIVE_ACTION_RETRY_DELAY_MS * pendingEditorUpdateRetryAttempts
        }
        val retry = Runnable {
            if (retryGeneration != pendingEditorUpdateRetryGeneration) return@Runnable
            if (pendingEditorUpdateRetryEditorId != richTextView.editorId) {
                clearPendingEditorUpdateState()
                return@Runnable
            }
            pendingEditorUpdateRetryScheduled = false
            pendingEditorUpdateRetryEditorId = null
            applyPendingEditorResetUpdateIfNeeded()
            applyPendingEditorUpdateIfNeeded()
        }
        mainHandler.postDelayed(retry, delayMs)
    }

    private fun clearPendingThemeRetry() {
        pendingThemeJson = null
        hasPendingTheme = false
        cancelPendingThemeRetry()
    }

    private fun cancelPendingThemeRetry() {
        pendingThemeRetryScheduled = false
        pendingThemeRetryEditorId = null
        pendingThemeRetryAttempts = 0
        pendingThemeRetryGeneration += 1
    }

    private fun applyPendingThemeIfNeeded() {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (!hasPendingTheme) return
        val themeJson = pendingThemeJson
        val editorId = richTextView.editorId
        if (pendingThemeRetryEditorId != editorId) {
            pendingThemeRetryEditorId = editorId
        }
        if (
            blockThemePreflightForTesting ||
            !richTextView.editorEditText.prepareForExternalEditorUpdate()
        ) {
            schedulePendingThemeRetry()
            return
        }
        pendingThemeJson = null
        hasPendingTheme = false
        cancelPendingThemeRetry()
        applyThemeJson(themeJson)
    }

    private fun schedulePendingThemeRetry() {
        if (pendingThemeRetryScheduled) return
        if (pendingThemeRetryAttempts >= MAX_PENDING_UPDATE_RETRY_ATTEMPTS) return
        pendingThemeRetryAttempts += 1
        pendingThemeRetryEditorId = richTextView.editorId
        pendingThemeRetryScheduled = true
        pendingThemeRetryGeneration += 1
        val retryGeneration = pendingThemeRetryGeneration
        val delayMs = NATIVE_ACTION_RETRY_DELAY_MS * pendingThemeRetryAttempts
        val retry = Runnable {
            if (retryGeneration != pendingThemeRetryGeneration) return@Runnable
            if (pendingThemeRetryEditorId != richTextView.editorId) {
                clearPendingThemeRetry()
                return@Runnable
            }
            pendingThemeRetryScheduled = false
            applyPendingThemeIfNeeded()
        }
        mainHandler.postDelayed(retry, delayMs)
    }

    private fun clearPendingViewCommandUpdateRetry() {
        pendingViewCommandUpdateJson = null
        pendingViewCommandUpdateEditorId = null
        pendingViewCommandUpdateRetryScheduled = false
        pendingViewCommandUpdateRetryAttempts = 0
        pendingViewCommandUpdateRetryGeneration += 1
    }

    private fun scheduleViewCommandUpdateRetry(updateJson: String) {
        if (pendingViewCommandUpdateJson != updateJson) {
            pendingViewCommandUpdateRetryAttempts = 0
        }
        pendingViewCommandUpdateJson = updateJson
        pendingViewCommandUpdateEditorId = richTextView.editorId
        if (pendingViewCommandUpdateRetryScheduled) return
        if (pendingViewCommandUpdateRetryAttempts >= MAX_PENDING_UPDATE_RETRY_ATTEMPTS) return
        pendingViewCommandUpdateRetryAttempts += 1
        pendingViewCommandUpdateRetryScheduled = true
        pendingViewCommandUpdateRetryGeneration += 1
        val retryGeneration = pendingViewCommandUpdateRetryGeneration
        val delayMs = NATIVE_ACTION_RETRY_DELAY_MS * pendingViewCommandUpdateRetryAttempts
        val retry = Runnable {
            if (retryGeneration != pendingViewCommandUpdateRetryGeneration) return@Runnable
            val retryJson = pendingViewCommandUpdateJson ?: run {
                pendingViewCommandUpdateRetryScheduled = false
                return@Runnable
            }
            if (pendingViewCommandUpdateEditorId != richTextView.editorId || richTextView.editorId == 0L) {
                clearPendingViewCommandUpdateRetry()
                return@Runnable
            }
            if (handleDestroyedCurrentEditorIfNeeded()) {
                clearPendingViewCommandUpdateRetry()
                return@Runnable
            }
            pendingViewCommandUpdateRetryScheduled = false
            if (applyEditorUpdate(retryJson, scheduleViewCommandRetry = true)) {
                clearPendingViewCommandUpdateRetry()
            }
        }
        mainHandler.postDelayed(retry, delayMs)
    }

    private fun schedulePendingPreflightWake() {
        if (pendingPreflightWakeScheduled) return
        pendingPreflightWakeScheduled = true
        pendingPreflightWakeGeneration += 1
        val wakeGeneration = pendingPreflightWakeGeneration
        mainHandler.post {
            if (wakeGeneration != pendingPreflightWakeGeneration) return@post
            pendingPreflightWakeScheduled = false
            wakePendingPreflightWork()
        }
    }

    private fun cancelPendingPreflightWake() {
        pendingPreflightWakeScheduled = false
        pendingPreflightWakeGeneration += 1
    }

    private fun wakePendingPreflightWork() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            schedulePendingPreflightWake()
            return
        }
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (pendingEditorResetUpdateJson != null) {
            applyPendingEditorResetUpdateIfNeeded()
        }
        if (pendingEditorUpdateJson != null) {
            pendingEditorUpdateRetryAttempts = 0
            pendingEditorUpdateForcedRecoveryAttempted = false
            applyPendingEditorUpdateIfNeeded()
        }
        if (hasPendingTheme) {
            pendingThemeRetryAttempts = 0
            applyPendingThemeIfNeeded()
        }
        pendingViewCommandUpdateJson?.let { updateJson ->
            pendingViewCommandUpdateRetryAttempts = 0
            pendingViewCommandUpdateRetryScheduled = false
            pendingViewCommandUpdateRetryGeneration += 1
            if (applyEditorUpdate(updateJson, scheduleViewCommandRetry = true)) {
                clearPendingViewCommandUpdateRetry()
            }
        }
        retryPendingNativeActionFromWake()
    }

    private fun clearPendingNativeActionRetry() {
        pendingNativeAction = null
        pendingNativeActionScope = null
        pendingNativeActionRetryEditorId = null
        pendingNativeActionRetryScheduled = false
        pendingNativeActionRetryAttempts = 0
        pendingNativeActionRetryGeneration += 1
    }

    private fun currentNativeActionScope(action: PendingNativeAction): PendingNativeActionScope {
        val selection = richTextView.editorEditText.currentScalarSelection()
        val mentionScope = when (action) {
            is PendingNativeAction.MentionSuggestionSelect ->
                mentionQueryState ?: addons.mentions?.let { currentMentionQueryState(it.trigger) }
            is PendingNativeAction.ToolbarItemPress -> null
        }
        return PendingNativeActionScope(
            editorId = richTextView.editorId,
            documentVersion = lastDocumentVersion,
            allowedDocumentVersion = documentVersionFromUpdateJSON(pendingEditorUpdateJson),
            hadFocus = isEditorEffectivelyFocusedForNativeAction(),
            hadVisibleToolbar = isNativeActionToolbarVisible(action),
            selectionAnchor = selection?.first,
            selectionHead = selection?.second,
            mentionAnchor = mentionScope?.anchor,
            mentionHead = mentionScope?.head,
            mentionQuery = mentionScope?.query
        )
    }

    private fun isPendingNativeActionScopeCurrent(
        action: PendingNativeAction,
        scope: PendingNativeActionScope
    ): Boolean {
        if (scope.editorId != richTextView.editorId) return false
        if (scope.hadFocus != isEditorEffectivelyFocusedForNativeAction()) return false
        if (scope.hadVisibleToolbar != isNativeActionToolbarVisible(action)) return false
        if (
            scope.documentVersion != lastDocumentVersion &&
            (scope.allowedDocumentVersion == null || scope.allowedDocumentVersion != lastDocumentVersion)
        ) {
            return false
        }
        val selection = richTextView.editorEditText.currentScalarSelection()
        if (scope.selectionAnchor != selection?.first || scope.selectionHead != selection?.second) {
            return false
        }
        if (action is PendingNativeAction.MentionSuggestionSelect) {
            val mentions = addons.mentions ?: return false
            val currentQuery = currentMentionQueryState(mentions.trigger) ?: return false
            if (
                scope.mentionAnchor != currentQuery.anchor ||
                scope.mentionHead != currentQuery.head ||
                scope.mentionQuery != currentQuery.query
            ) {
                return false
            }
        }
        return true
    }

    private fun isNativeActionToolbarVisible(action: PendingNativeAction): Boolean {
        if (!showsToolbar || toolbarPlacement != ToolbarPlacement.KEYBOARD) return false
        if (keyboardToolbarView.parent == null || keyboardToolbarView.visibility != View.VISIBLE) return false
        if (action is PendingNativeAction.MentionSuggestionSelect) {
            return keyboardToolbarView.isShowingMentionSuggestions
        }
        return true
    }

    private fun isEditorEffectivelyFocusedForNativeAction(): Boolean =
        richTextView.editorEditText.hasFocus() ||
            (pendingToolbarRefocus != null && pendingToolbarRefocusEditorId == richTextView.editorId)

    private fun clearPendingNativeActionRetryIfScopeChanged() {
        val action = pendingNativeAction ?: return
        val scope = pendingNativeActionScope ?: return
        if (!isPendingNativeActionScopeCurrent(action, scope)) {
            clearPendingNativeActionRetry()
        }
    }

    private fun schedulePendingNativeActionRetry(action: PendingNativeAction) {
        val isSameAction = pendingNativeAction == action
        if (isSameAction) {
            pendingNativeActionRetryAttempts += 1
        } else {
            pendingNativeActionRetryAttempts = 1
            pendingNativeActionScope = currentNativeActionScope(action)
        }
        if (pendingNativeActionRetryAttempts > MAX_NATIVE_ACTION_RETRY_ATTEMPTS) {
            pendingNativeAction = action
            pendingNativeActionRetryEditorId = richTextView.editorId
            pendingNativeActionRetryScheduled = false
            return
        }
        pendingNativeAction = action
        pendingNativeActionRetryEditorId = richTextView.editorId
        if (pendingNativeActionRetryScheduled) return
        pendingNativeActionRetryScheduled = true
        pendingNativeActionRetryGeneration += 1
        val retryGeneration = pendingNativeActionRetryGeneration
        val retry = Runnable {
            if (retryGeneration != pendingNativeActionRetryGeneration) return@Runnable
            val retryAction = pendingNativeAction ?: run {
                pendingNativeActionRetryScheduled = false
                return@Runnable
            }
            val retryScope = pendingNativeActionScope ?: run {
                clearPendingNativeActionRetry()
                return@Runnable
            }
            if (pendingNativeActionRetryEditorId != richTextView.editorId || richTextView.editorId == 0L) {
                clearPendingNativeActionRetry()
                return@Runnable
            }
            if (!isPendingNativeActionScopeCurrent(retryAction, retryScope)) {
                clearPendingNativeActionRetry()
                return@Runnable
            }
            pendingNativeActionRetryScheduled = false
            val allowNextRetry = pendingNativeActionRetryAttempts < MAX_NATIVE_ACTION_RETRY_ATTEMPTS
            when (retryAction) {
                is PendingNativeAction.ToolbarItemPress ->
                    handleToolbarItemPress(retryAction.item, allowPreflightRetry = allowNextRetry)
                is PendingNativeAction.MentionSuggestionSelect ->
                    insertMentionSuggestion(retryAction.suggestion, allowPreflightRetry = allowNextRetry)
            }
        }
        mainHandler.postDelayed(retry, NATIVE_ACTION_RETRY_DELAY_MS)
    }

    private fun retryPendingNativeActionFromWake() {
        val action = pendingNativeAction ?: return
        val scope = pendingNativeActionScope ?: run {
            clearPendingNativeActionRetry()
            return
        }
        if (!isPendingNativeActionScopeCurrent(action, scope)) {
            clearPendingNativeActionRetry()
            return
        }
        pendingNativeActionRetryAttempts = 0
        pendingNativeActionRetryScheduled = false
        when (action) {
            is PendingNativeAction.ToolbarItemPress ->
                handleToolbarItemPress(action.item, allowPreflightRetry = true)
            is PendingNativeAction.MentionSuggestionSelect ->
                insertMentionSuggestion(action.suggestion, allowPreflightRetry = true)
        }
    }

    private fun documentVersionFromUpdateJSON(updateJSON: String?): Int? =
        try {
            if (updateJSON == null) null
            else {
                val version = JSONObject(updateJSON).optInt("documentVersion", Int.MIN_VALUE)
                version.takeIf { it != Int.MIN_VALUE }
            }
        } catch (_: Throwable) {
            null
        }

    private fun noteDocumentVersionFromUpdateJSON(updateJSON: String?) {
        documentVersionFromUpdateJSON(updateJSON)?.let { version ->
            lastDocumentVersion = version
        }
    }

    private fun addPreflightUpdateToEvent(
        event: MutableMap<String, Any>,
        updateJSON: String?
    ) {
        if (updateJSON == null) return
        event["updateJson"] = updateJSON
        documentVersionFromUpdateJSON(updateJSON)?.let { version ->
            event["documentVersion"] = version
        }
    }

    private fun emitAddonEvent(payload: Map<String, Any>) {
        onAddonEventForTesting?.invoke(payload) ?: onAddonEvent(payload)
    }

    private fun canFocusCurrentEditor(): Boolean {
        val editorId = richTextView.editorId
        return editorId != 0L &&
            isAttachedToNativeWindow &&
            !NativeEditorViewRegistry.isDestroyed(editorId)
    }

    fun focus() {
        focusInternal(cancelPendingOutsideTapBlur = true)
    }

    private fun focusInternal(cancelPendingOutsideTapBlur: Boolean) {
        if (!canFocusCurrentEditor()) return
        if (cancelPendingOutsideTapBlur) {
            cancelPendingOutsideTapBlur()
        }
        cancelPendingKeyboardDismiss()
        cancelPendingBlurRetry()
        richTextView.editorEditText.requestFocus()
        richTextView.editorEditText.post {
            if (!canFocusCurrentEditor()) return@post
            val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            imm?.showSoftInput(richTextView.editorEditText, InputMethodManager.SHOW_IMPLICIT)
        }
    }

    fun blur() {
        cancelPendingOutsideTapBlur()
        cancelPendingKeyboardDismiss()
        cancelPendingToolbarRefocus()
        clearRecentToolbarTouch()
        performBlur(deferKeyboardDismiss = false, allowRetry = true)
    }

    private fun performBlur(deferKeyboardDismiss: Boolean, allowRetry: Boolean) {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (!richTextView.editorEditText.prepareForExternalEditorUpdate()) {
            if (allowRetry && pendingBlurRetryAttempts < MAX_PENDING_UPDATE_RETRY_ATTEMPTS) {
                schedulePendingBlurRetry(deferKeyboardDismiss)
                return
            }
            if (handleDestroyedCurrentEditorIfNeeded()) return
            richTextView.editorEditText.restoreAuthorizedTextIfNeeded()
        }
        completeBlur(deferKeyboardDismiss)
    }

    private fun completeBlur(deferKeyboardDismiss: Boolean) {
        cancelPendingBlurRetry()
        traceOutsideTap(
            "complete blur deferKeyboardDismiss=$deferKeyboardDismiss focusedBefore=${richTextView.editorEditText.hasFocus()}"
        )
        richTextView.editorEditText.clearFocus()
        traceOutsideTap("complete blur focusedAfter=${richTextView.editorEditText.hasFocus()}")
        if (deferKeyboardDismiss) {
            val dismiss = Runnable {
                pendingKeyboardDismiss = null
                if (!richTextView.editorEditText.hasFocus()) {
                    val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
                    imm?.hideSoftInputFromWindow(richTextView.editorEditText.windowToken, 0)
                }
            }
            pendingKeyboardDismiss = dismiss
            richTextView.editorEditText.post(dismiss)
            return
        }
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        imm?.hideSoftInputFromWindow(richTextView.editorEditText.windowToken, 0)
    }

    private fun schedulePendingBlurRetry(deferKeyboardDismiss: Boolean) {
        pendingBlurRetry?.let {
            mainHandler.removeCallbacks(it)
            pendingBlurRetry = null
        }
        pendingBlurRetryAttempts += 1
        pendingBlurRetryEditorId = richTextView.editorId
        pendingBlurRetryGeneration += 1
        val retryGeneration = pendingBlurRetryGeneration
        val delayMs = NATIVE_ACTION_RETRY_DELAY_MS * pendingBlurRetryAttempts
        val retry = Runnable {
            pendingBlurRetry = null
            if (retryGeneration != pendingBlurRetryGeneration) return@Runnable
            if (pendingBlurRetryEditorId != richTextView.editorId) {
                pendingBlurRetryEditorId = null
                return@Runnable
            }
            pendingBlurRetryEditorId = null
            if (handleDestroyedCurrentEditorIfNeeded()) return@Runnable
            performBlur(deferKeyboardDismiss, allowRetry = true)
        }
        pendingBlurRetry = retry
        mainHandler.postDelayed(retry, delayMs)
    }

    private fun blurWithDeferredKeyboardDismiss() {
        cancelPendingKeyboardDismiss()
        cancelPendingToolbarRefocus()
        clearRecentToolbarTouch()
        performBlur(deferKeyboardDismiss = true, allowRetry = true)
    }

    private fun scheduleToolbarRefocus() {
        cancelPendingToolbarRefocus()
        val editorId = richTextView.editorId
        pendingToolbarRefocusEditorId = editorId
        pendingToolbarRefocusGeneration += 1
        val refocusGeneration = pendingToolbarRefocusGeneration
        val refocus = Runnable {
            pendingToolbarRefocus = null
            if (refocusGeneration != pendingToolbarRefocusGeneration) return@Runnable
            if (pendingToolbarRefocusEditorId != richTextView.editorId) return@Runnable
            pendingToolbarRefocusEditorId = null
            focusInternal(cancelPendingOutsideTapBlur = false)
        }
        pendingToolbarRefocus = refocus
        richTextView.editorEditText.post(refocus)
    }

    private fun cancelPendingToolbarRefocus() {
        pendingToolbarRefocus?.let {
            richTextView.editorEditText.removeCallbacks(it)
            pendingToolbarRefocus = null
        }
        pendingToolbarRefocusEditorId = null
        pendingToolbarRefocusGeneration += 1
    }

    private fun scheduleOutsideTapBlur() {
        cancelPendingOutsideTapBlur()
        traceOutsideTap("schedule outside blur focused=${richTextView.editorEditText.hasFocus()}")
        val blur = Runnable {
            pendingOutsideTapBlur = null
            traceOutsideTap("run outside blur focused=${richTextView.editorEditText.hasFocus()}")
            if (richTextView.editorEditText.hasFocus()) {
                blurWithDeferredKeyboardDismiss()
            }
        }
        pendingOutsideTapBlur = blur
        richTextView.editorEditText.postDelayed(blur, OUTSIDE_TAP_BLUR_DELAY_MS)
    }

    private fun cancelPendingOutsideTapBlur() {
        pendingOutsideTapBlur?.let {
            traceOutsideTap("cancel outside blur")
            richTextView.editorEditText.removeCallbacks(it)
            pendingOutsideTapBlur = null
        }
    }

    private fun cancelPendingKeyboardDismiss() {
        pendingKeyboardDismiss?.let {
            richTextView.editorEditText.removeCallbacks(it)
            pendingKeyboardDismiss = null
        }
    }

    private fun cancelPendingBlurRetry() {
        pendingBlurRetry?.let {
            mainHandler.removeCallbacks(it)
            pendingBlurRetry = null
        }
        pendingBlurRetryEditorId = null
        pendingBlurRetryAttempts = 0
        pendingBlurRetryGeneration += 1
    }

    fun getCaretRectJson(): String? {
        if (width <= 0 || height <= 0) return null
        val rect = richTextView.caretRect() ?: return null
        val density = resources.displayMetrics.density
        return JSONObject()
            .put("x", rect.left / density)
            .put("y", rect.top / density)
            .put("width", rect.width() / density)
            .put("height", rect.height() / density)
            .put("editorWidth", width / density)
            .put("editorHeight", height / density)
            .toString()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        handleAttachedToWindow()
    }

    internal fun handleEditorDestroyed(editorId: Long) {
        if (richTextView.editorId != editorId && richTextView.editorEditText.editorId != editorId) {
            return
        }
        cancelPendingEditorUpdateRetry()
        clearPendingViewCommandUpdateRetry()
        cancelPendingThemeRetry()
        cancelPendingBlurRetry()
        cancelPendingDetachPreflightRetry()
        cancelPendingOutsideTapBlur()
        cancelPendingKeyboardDismiss()
        cancelPendingToolbarRefocus()
        cancelPendingPreflightWake()
        clearPendingNativeActionRetry()
        clearRecentToolbarTouch()
        uninstallOutsideTapBlurHandler()
        detachKeyboardToolbarIfNeeded()
        richTextView.setViewportBottomInsetPx(0)
        val editText = richTextView.editorEditText
        if (editText.hasFocus()) {
            editText.clearFocus()
        }
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        imm?.hideSoftInputFromWindow(editText.windowToken, 0)
        clearMentionQueryState(resetLastEvent = true)
        pendingEditorUpdateJson = null
        pendingEditorUpdateEditorId = null
        pendingEditorUpdateRevision = 0
        appliedEditorUpdateRevision = 0
        pendingEditorResetUpdateJson = null
        pendingEditorResetUpdateEditorId = null
        pendingEditorResetUpdateRevision = 0
        appliedEditorResetUpdateRevision = 0
        lastEditorResetUpdateJsonProp = null
        lastEditorResetUpdateEditorIdProp = null
        lastDocumentVersion = null
        lastReadyEditorId = null
        toolbarState = NativeToolbarState.empty
        keyboardToolbarView.applyState(toolbarState)
        keyboardToolbarView.visibility = View.GONE
        richTextView.editorId = 0L
    }

    private fun handleDestroyedCurrentEditorIfNeeded(): Boolean {
        val editorId = richTextView.editorId.takeIf { it != 0L }
            ?: richTextView.editorEditText.editorId.takeIf { it != 0L }
            ?: return false
        if (!NativeEditorViewRegistry.isDestroyed(editorId)) return false
        handleEditorDestroyed(editorId)
        return true
    }

    private fun handleAttachedToWindow() {
        isAttachedToNativeWindow = true
        cancelPendingDetachPreflightRetry()
        richTextView.clearDeferredEditorUnbind()
        val editorId = richTextView.editorId
        if (editorId == 0L) return
        if (NativeEditorViewRegistry.isDestroyed(editorId)) {
            handleEditorDestroyed(editorId)
            return
        }
        if (!NativeEditorViewRegistry.register(editorId, this)) {
            handleEditorDestroyed(editorId)
            return
        }
        richTextView.rebindEditorIfNeeded(
            notifyListener = !hasPendingEditorResetUpdateForEditor(editorId) &&
                !hasPendingEditorUpdateForEditor(editorId)
        )
        if (hasPendingTheme) {
            pendingThemeRetryEditorId = editorId
        }
        applyPendingEditorResetUpdateIfNeeded()
        applyPendingEditorUpdateIfNeeded()
        applyPendingThemeIfNeeded()
        refreshReadyStateIfSettled()
        applyAutoFocusIfNeeded()
    }

    private fun emitEditorReady(editorUpdateRevision: Int? = null): Boolean {
        val editorId = richTextView.editorId
        if (editorId == 0L) return false
        if (!isAttachedToNativeWindow) return false
        if (richTextView.editorEditText.editorId != editorId) return false
        if (hasPendingEditorResetUpdateForCurrentEditor()) return false
        if (hasPendingEditorUpdateForCurrentEditor()) return false
        lastReadyEditorId = editorId
        val payload = mutableMapOf<String, Any>("editorId" to editorId)
        editorUpdateRevision?.let { payload["editorUpdateRevision"] = it }
        onEditorReadyForTesting?.invoke(payload) ?: onEditorReady(payload)
        return true
    }

    private fun emitEditorReadyIfNeeded() {
        val editorId = richTextView.editorId
        if (lastReadyEditorId == editorId) return
        emitEditorReady()
    }

    override fun onDetachedFromWindow() {
        prepareForDetachFromWindow()
        super.onDetachedFromWindow()
        handleDetachedFromWindow()
    }

    private fun prepareForDetachFromWindow() {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        val editorId = richTextView.editorId
        if (editorId == 0L || richTextView.editorEditText.editorId == 0L) return
        if (richTextView.editorEditText.prepareForExternalEditorUpdate()) {
            cancelPendingDetachPreflightRetry()
            richTextView.clearDeferredEditorUnbind()
            return
        }
        richTextView.deferEditorUnbindOnNextDetach()
        schedulePendingDetachPreflightRetry(editorId)
    }

    private fun schedulePendingDetachPreflightRetry(editorId: Long) {
        if (pendingDetachPreflightRetryScheduled) return
        if (pendingDetachPreflightRetryAttempts >= MAX_PENDING_UPDATE_RETRY_ATTEMPTS) {
            if (handleDestroyedCurrentEditorIfNeeded()) return
            richTextView.editorEditText.restoreAuthorizedTextIfNeeded()
            cancelPendingDetachPreflightRetry()
            richTextView.unbindEditorForDetachedViewIfNeeded()
            return
        }
        pendingDetachPreflightRetryAttempts += 1
        pendingDetachPreflightRetryEditorId = editorId
        pendingDetachPreflightRetryScheduled = true
        pendingDetachPreflightRetryGeneration += 1
        val retryGeneration = pendingDetachPreflightRetryGeneration
        val delayMs = NATIVE_ACTION_RETRY_DELAY_MS * pendingDetachPreflightRetryAttempts
        mainHandler.postDelayed({
            if (retryGeneration != pendingDetachPreflightRetryGeneration) return@postDelayed
            pendingDetachPreflightRetryScheduled = false
            if (isAttachedToNativeWindow || pendingDetachPreflightRetryEditorId != richTextView.editorId) {
                cancelPendingDetachPreflightRetry()
                return@postDelayed
            }
            if (handleDestroyedCurrentEditorIfNeeded()) return@postDelayed
            if (richTextView.editorEditText.prepareForExternalEditorUpdate()) {
                cancelPendingDetachPreflightRetry()
                richTextView.unbindEditorForDetachedViewIfNeeded()
                return@postDelayed
            }
            schedulePendingDetachPreflightRetry(editorId)
        }, delayMs)
    }

    private fun cancelPendingDetachPreflightRetry() {
        pendingDetachPreflightRetryScheduled = false
        pendingDetachPreflightRetryEditorId = null
        pendingDetachPreflightRetryAttempts = 0
        pendingDetachPreflightRetryGeneration += 1
    }

    private fun handleDetachedFromWindow() {
        isAttachedToNativeWindow = false
        NativeEditorViewRegistry.unregister(
            richTextView.editorId,
            this,
            blockCommandsUntilRegistered = true
        )
        cancelPendingOutsideTapBlur()
        cancelPendingKeyboardDismiss()
        cancelPendingToolbarRefocus()
        cancelPendingBlurRetry()
        cancelPendingEditorUpdateRetry()
        clearPendingViewCommandUpdateRetry()
        cancelPendingThemeRetry()
        clearPendingNativeActionRetry()
        cancelPendingPreflightWake()
        lastReadyEditorId = null
        uninstallOutsideTapBlurHandler()
        currentImeBottom = 0
        keyboardToolbarView.visibility = View.GONE
        detachKeyboardToolbarIfNeeded()
        richTextView.setViewportBottomInsetPx(0)
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

    private fun emitContentHeightIfNeeded(force: Boolean) {
        if (heightBehavior != EditorHeightBehavior.AUTO_GROW) return
        val editText = richTextView.editorEditText
        val resolvedEditHeight = editText.resolveAutoGrowHeight()
        val resolvedContainerHeight =
            resolvedEditHeight +
                richTextView.paddingTop +
                richTextView.paddingBottom +
                paddingTop +
                paddingBottom
        val contentHeight = (
            when {
                editText.isLaidOut && (editText.layout?.height ?: 0) > 0 -> {
                    maxOf(
                        (editText.layout?.height ?: 0) +
                            editText.compoundPaddingTop +
                            editText.compoundPaddingBottom +
                            richTextView.paddingTop +
                            richTextView.paddingBottom +
                            paddingTop +
                            paddingBottom,
                        resolvedContainerHeight
                    )
                }
                richTextView.measuredHeight > 0 -> {
                    maxOf(
                        richTextView.measuredHeight + paddingTop + paddingBottom,
                        resolvedContainerHeight
                    )
                }
                editText.measuredHeight > 0 -> {
                    maxOf(
                        editText.measuredHeight +
                            richTextView.paddingTop +
                            richTextView.paddingBottom +
                            paddingTop +
                            paddingBottom,
                        resolvedContainerHeight
                    )
                }
                else -> {
                    resolvedContainerHeight
                }
            }
        ).coerceAtLeast(0)
        if (contentHeight <= 0) return
        val editorId = richTextView.editorId
        if (
            !force &&
            contentHeight == lastEmittedContentHeight &&
            editorId == lastEmittedContentHeightEditorId
        ) {
            return
        }
        lastEmittedContentHeight = contentHeight
        lastEmittedContentHeightEditorId = editorId
        val event = mapOf(
            "contentHeight" to contentHeight,
            "editorId" to editorId
        )
        onContentHeightChangeForTesting?.invoke(event) ?: onContentHeightChange(event)
    }

    /** Applies an editor update from JS without echoing it back through events. */
    fun applyEditorUpdate(updateJson: String): Boolean =
        applyEditorUpdate(updateJson, scheduleViewCommandRetry = true)

    /** Applies a reset-style update from JS, discarding pending native composition. */
    fun applyEditorResetUpdate(updateJson: String): Boolean {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            val postedEditorId = richTextView.editorId
            val apply = Runnable {
                if (postedEditorId != richTextView.editorId) return@Runnable
                applyEditorResetUpdate(updateJson)
            }
            if (!post(apply)) {
                richTextView.post(apply)
            }
            return false
        }
        if (handleDestroyedCurrentEditorIfNeeded()) {
            return false
        }
        if (!isEditorReadyForNativeUpdate()) {
            return false
        }
        clearPendingEditorUpdateState(resetAppliedRevision = false)
        clearPendingViewCommandUpdateRetry()
        isApplyingJSUpdate = true
        val applied = try {
            richTextView.editorEditText.applyUpdateJSON(
                updateJson,
                refreshInputConnectionForExternalUpdate = true
            )
            clearPendingEditorUpdateDispatchQueue("jsResetUpdate")
            true
        } catch (error: Throwable) {
            Log.w(LOG_TAG, "Failed to apply JS editor reset update", error)
            false
        } finally {
            isApplyingJSUpdate = false
        }
        if (applied) {
            refreshReadyStateIfSettled()
        }
        return applied
    }

    private fun isEditorReadyForNativeUpdate(): Boolean {
        val editorId = richTextView.editorId
        return editorId == 0L || (isAttachedToNativeWindow && richTextView.editorEditText.editorId == editorId)
    }

    private fun applyEditorUpdate(
        updateJson: String,
        scheduleViewCommandRetry: Boolean,
        expectedEditorId: Long? = null
    ): Boolean {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            val postedEditorId = expectedEditorId ?: richTextView.editorId
            val apply = Runnable {
                if (postedEditorId != richTextView.editorId) return@Runnable
                applyEditorUpdate(updateJson, scheduleViewCommandRetry, postedEditorId)
            }
            if (!post(apply)) {
                richTextView.post(apply)
            }
            return false
        }
        if (expectedEditorId != null && expectedEditorId != richTextView.editorId) {
            return false
        }
        if (handleDestroyedCurrentEditorIfNeeded()) {
            return false
        }
        if (!isEditorReadyForNativeUpdate()) {
            if (scheduleViewCommandRetry) {
                scheduleViewCommandUpdateRetry(updateJson)
            }
            return false
        }
        if (
            blockEditorUpdatePreflightForTesting ||
            !richTextView.editorEditText.prepareForExternalEditorUpdate()
        ) {
            if (scheduleViewCommandRetry) {
                scheduleViewCommandUpdateRetry(updateJson)
            }
            return false
        }
        isApplyingJSUpdate = true
        return try {
            richTextView.editorEditText.applyUpdateJSON(
                updateJson,
                refreshInputConnectionForExternalUpdate = true
            )
            clearPendingEditorUpdateDispatchQueue("jsUpdate")
            true
        } catch (error: Throwable) {
            Log.w(LOG_TAG, "Failed to apply JS editor update", error)
            if (scheduleViewCommandRetry) {
                scheduleViewCommandUpdateRetry(updateJson)
            }
            false
        } finally {
            isApplyingJSUpdate = false
        }
    }

    fun prepareForEditorCommandJSON(): String {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            return NativeEditorViewRegistry.commandPreparationJSON(
                ready = false,
                blockedReason = "unknown"
            )
        }
        if (handleDestroyedCurrentEditorIfNeeded()) {
            return NativeEditorViewRegistry.commandPreparationJSON(
                ready = false,
                blockedReason = "destroyed"
            )
        }
        if (richTextView.editorId != 0L && !isAttachedToNativeWindow) {
            return NativeEditorViewRegistry.commandPreparationJSON(
                ready = false,
                blockedReason = "detached"
            )
        }
        if (richTextView.editorId != 0L && richTextView.editorEditText.editorId != richTextView.editorId) {
            return NativeEditorViewRegistry.commandPreparationJSON(
                ready = false,
                blockedReason = "detached"
            )
        }
        if (shouldBlockEditorCommandForPendingUpdate()) {
            return pendingEditorUpdateCommandPreparationJSON()
        }
        isApplyingJSUpdate = true
        return try {
            onBeforePrepareForEditorCommandForTesting?.invoke()
            val preparation = richTextView.editorEditText.prepareForExternalEditorCommand()
            NativeEditorViewRegistry.commandPreparationJSON(
                ready = preparation.ready,
                updateJSON = preparation.updateJSON,
                blockedReason = if (preparation.ready) null else "composition"
            )
        } finally {
            isApplyingJSUpdate = false
        }
    }

    override fun onSelectionChanged(anchor: Int, head: Int) {
        val stateJson = refreshToolbarStateFromEditorSelection()
        refreshMentionQuery()
        clearPendingNativeActionRetryIfScopeChanged()
        schedulePendingPreflightWake()
        richTextView.refreshRemoteSelections()
        val event = mutableMapOf<String, Any>(
            "anchor" to anchor,
            "head" to head,
            "editorId" to richTextView.editorId
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
        if (isApplyingJSUpdate) {
            dispatchEditorUpdate(
                PendingEditorUpdateEvent(
                    editorId = richTextView.editorId,
                    updateJSON = updateJSON
                ),
                emitToJS = false
            )
            return
        }
        pendingEditorUpdateEvents.addLast(
            PendingEditorUpdateEvent(
                editorId = richTextView.editorId,
                updateJSON = updateJSON
            )
        )
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateQueued",
            "queue=${pendingEditorUpdateEvents.size} jsonLength=${updateJSON.length}"
        )
        schedulePendingEditorUpdateDispatch()
    }

    internal fun pendingEditorUpdateEventCountForTesting(): Int =
        pendingEditorUpdateEvents.size

    private fun schedulePendingEditorUpdateDispatch() {
        pendingEditorUpdateDispatchScheduled = true
        val generation = ++pendingEditorUpdateDispatchGeneration
        mainHandler.postDelayed({
            if (generation != pendingEditorUpdateDispatchGeneration) return@postDelayed
            pendingEditorUpdateDispatchScheduled = false
            drainPendingEditorUpdateEvents()
        }, EDITOR_UPDATE_EVENT_DEBOUNCE_MS)
    }

    private fun drainPendingEditorUpdateEvents() {
        if (pendingEditorUpdateEvents.isEmpty()) return
        val startedAt = System.nanoTime()
        var drainedCount = 0
        while (pendingEditorUpdateEvents.isNotEmpty()) {
            val event = pendingEditorUpdateEvents.removeFirst()
            if (event.editorId != richTextView.editorId) {
                richTextView.editorEditText.recordImeTraceForTesting(
                    "nativeViewEditorUpdateSkipped",
                    "reason=staleEditor queuedEditor=${event.editorId} currentEditor=${richTextView.editorId}"
                )
                continue
            }
            dispatchEditorUpdate(event, emitToJS = true)
            drainedCount += 1
        }
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateDrained",
            "count=$drainedCount totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    private fun clearPendingEditorUpdateDispatchQueue(reason: String) {
        if (pendingEditorUpdateEvents.isEmpty() && !pendingEditorUpdateDispatchScheduled) return
        val clearedCount = pendingEditorUpdateEvents.size
        pendingEditorUpdateEvents.clear()
        pendingEditorUpdateDispatchScheduled = false
        pendingEditorUpdateDispatchGeneration += 1
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateQueueCleared",
            "reason=$reason count=$clearedCount"
        )
    }

    private fun dispatchEditorUpdate(event: PendingEditorUpdateEvent, emitToJS: Boolean) {
        val updateJSON = event.updateJSON
        val startedAt = System.nanoTime()
        noteDocumentVersionFromUpdateJSON(updateJSON)
        val noteNanos = System.nanoTime() - startedAt
        val toolbarStartedAt = System.nanoTime()
        NativeToolbarState.fromUpdateJson(updateJSON)?.let { state ->
            toolbarState = state
            keyboardToolbarView.applyState(state)
        }
        val toolbarNanos = System.nanoTime() - toolbarStartedAt
        val mentionStartedAt = System.nanoTime()
        refreshMentionQuery()
        val mentionNanos = System.nanoTime() - mentionStartedAt
        val retryStartedAt = System.nanoTime()
        clearPendingNativeActionRetryIfScopeChanged()
        schedulePendingPreflightWake()
        richTextView.refreshRemoteSelections()
        val retryNanos = System.nanoTime() - retryStartedAt
        if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            post {
                requestLayout()
                emitContentHeightIfNeeded(force = false)
            }
        }
        val emitStartedAt = System.nanoTime()
        if (emitToJS) {
            val payload = mapOf<String, Any>(
                "updateJson" to updateJSON,
                "editorId" to event.editorId
            )
            onEditorUpdateForTesting?.invoke(payload) ?: onEditorUpdate(payload)
        }
        val totalNanos = System.nanoTime() - startedAt
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateDispatch",
            "emitToJS=$emitToJS jsonLength=${updateJSON.length} noteUs=${nanosToMicros(noteNanos)} toolbarUs=${nanosToMicros(toolbarNanos)} mentionUs=${nanosToMicros(mentionNanos)} retryUs=${nanosToMicros(retryNanos)} emitUs=${nanosToMicros(System.nanoTime() - emitStartedAt)} totalUs=${nanosToMicros(totalNanos)}"
        )
    }

    private fun installOutsideTapBlurHandlerIfNeeded() {
        val window = resolveActivity(context)?.window ?: return
        if (outsideTapWindow !== window) {
            uninstallOutsideTapBlurHandler()
        }
        if (NativeEditorOutsideTapDispatcher.register(window, this)) {
            outsideTapWindow = window
        } else if (outsideTapWindow === window) {
            outsideTapWindow = null
        }
    }

    private fun scheduleOutsideTapBlurHandlerInstallRetry() {
        cancelPendingOutsideTapBlurHandlerInstallRetry()
        val retry = Runnable {
            pendingOutsideTapHandlerInstallRetry = null
            if (richTextView.editorEditText.hasFocus()) {
                installOutsideTapBlurHandlerIfNeeded()
            }
        }
        pendingOutsideTapHandlerInstallRetry = retry
        richTextView.editorEditText.postDelayed(retry, OUTSIDE_TAP_HANDLER_INSTALL_RETRY_DELAY_MS)
    }

    private fun cancelPendingOutsideTapBlurHandlerInstallRetry() {
        pendingOutsideTapHandlerInstallRetry?.let {
            richTextView.editorEditText.removeCallbacks(it)
            pendingOutsideTapHandlerInstallRetry = null
        }
    }

    private fun uninstallOutsideTapBlurHandler() {
        cancelPendingOutsideTapBlurHandlerInstallRetry()
        val window = outsideTapWindow ?: return
        NativeEditorOutsideTapDispatcher.unregister(window, this)
        outsideTapWindow = null
    }

    internal fun prepareOutsideTapDecisionForWindowEvent(event: MotionEvent): NativeEditorOutsideTapDecision {
        if (!isAttachedToNativeWindow) {
            traceOutsideTap("decision ignored detached")
            return NativeEditorOutsideTapDecision.IGNORE
        }
        if (event.action != MotionEvent.ACTION_DOWN) {
            traceOutsideTap("decision ignored action=${event.action}")
            return NativeEditorOutsideTapDecision.IGNORE
        }
        if (!isEditorFocusedForOutsideTapDecision()) {
            traceOutsideTap("decision ignored not focused")
            return NativeEditorOutsideTapDecision.IGNORE
        }

        val decision = if (isTouchOutsideEditor(event)) {
            NativeEditorOutsideTapDecision.OUTSIDE_EDITOR
        } else {
            NativeEditorOutsideTapDecision.PRESERVE_FOCUS
        }
        traceOutsideTap("decision raw=${event.rawX.toInt()},${event.rawY.toInt()} value=$decision")
        return decision
    }

    internal fun handleOutsideTapDecisionFromWindowDispatcher(decision: NativeEditorOutsideTapDecision) {
        traceOutsideTap("handle decision=$decision")
        when (decision) {
            NativeEditorOutsideTapDecision.IGNORE -> {
                if (!richTextView.editorEditText.hasFocus()) {
                    cancelPendingOutsideTapBlur()
                }
            }
            NativeEditorOutsideTapDecision.PRESERVE_FOCUS -> cancelPendingOutsideTapBlur()
            NativeEditorOutsideTapDecision.OUTSIDE_EDITOR -> {
                clearRecentToolbarTouch()
                cancelPendingToolbarRefocus()
                scheduleOutsideTapBlur()
            }
        }
    }

    internal fun scheduleOutsideTapBlurFromWindowDispatcher() {
        scheduleOutsideTapBlur()
    }

    internal fun cancelOutsideTapBlurFromWindowDispatcher() {
        cancelPendingOutsideTapBlur()
    }

    private fun isEditorFocusedForOutsideTapDecision(): Boolean =
        editorFocusedForOutsideTapOverrideForTesting ?: richTextView.editorEditText.hasFocus()

    private fun isTouchOutsideEditor(event: MotionEvent): Boolean {
        if (isTouchInsideKeyboardToolbar(event)) {
            markRecentToolbarTouch()
            return false
        }
        if (isTouchInsideStandaloneToolbar(event)) {
            markRecentToolbarTouch()
            return false
        }
        val rect = Rect()
        richTextView.editorEditText.getGlobalVisibleRect(rect)
        val isOutside = !rect.contains(event.rawX.toInt(), event.rawY.toInt())
        if (isOutside) {
            clearRecentToolbarTouch()
        }
        return isOutside
    }

    private fun markRecentToolbarTouch() {
        lastToolbarTouchUptimeMs = SystemClock.uptimeMillis()
    }

    private fun clearRecentToolbarTouch() {
        lastToolbarTouchUptimeMs = null
    }

    private fun shouldPreserveFocusAfterToolbarTouch(): Boolean {
        val lastToolbarTouch = lastToolbarTouchUptimeMs ?: return false
        val elapsedMs = SystemClock.uptimeMillis() - lastToolbarTouch
        return elapsedMs in 0L..TOOLBAR_FOCUS_PRESERVE_MS
    }

    private fun consumeToolbarFocusPreservationForBlur(): Boolean {
        if (!shouldPreserveFocusAfterToolbarTouch()) {
            return false
        }
        clearRecentToolbarTouch()
        return true
    }

    internal fun markRecentToolbarTouchForTesting() {
        markRecentToolbarTouch()
    }

    internal fun shouldPreserveFocusAfterToolbarTouchForTesting(): Boolean =
        shouldPreserveFocusAfterToolbarTouch()

    internal fun setEditorFocusedForOutsideTapDecisionForTesting(isFocused: Boolean?) {
        editorFocusedForOutsideTapOverrideForTesting = isFocused
    }

    internal fun setAttachedToNativeWindowForTesting(isAttached: Boolean) {
        isAttachedToNativeWindow = isAttached
    }

    internal fun handleAttachedToWindowForTesting() {
        handleAttachedToWindow()
    }

    internal fun traceOutsideTap(message: String) {
        onOutsideTapTraceForTesting?.invoke(message)
    }

    internal fun handleDetachedFromWindowForTesting() {
        prepareForDetachFromWindow()
        handleDetachedFromWindow()
    }

    internal fun performBlurForTesting(deferKeyboardDismiss: Boolean = false) {
        performBlur(deferKeyboardDismiss = deferKeyboardDismiss, allowRetry = true)
    }

    internal fun pendingBlurRetryAttemptsForTesting(): Int = pendingBlurRetryAttempts

    internal fun pendingDetachPreflightRetryAttemptsForTesting(): Int =
        pendingDetachPreflightRetryAttempts

    internal fun hasPendingOutsideTapBlurForTesting(): Boolean = pendingOutsideTapBlur != null

    internal fun isOutsideTapBlurHandlerInstalledForTesting(): Boolean = outsideTapWindow != null

    internal fun hasPendingKeyboardDismissForTesting(): Boolean = pendingKeyboardDismiss != null

    internal fun hasPendingPreflightWakeForTesting(): Boolean = pendingPreflightWakeScheduled

    internal fun hasPendingToolbarRefocusForTesting(): Boolean = pendingToolbarRefocus != null

    internal fun isKeyboardToolbarAttachedForTesting(): Boolean = keyboardToolbarView.parent != null

    internal fun currentImeBottomForTesting(): Int = currentImeBottom

    internal fun setCurrentImeBottomForTesting(bottom: Int) {
        currentImeBottom = bottom
    }

    internal fun updateAttachedKeyboardToolbarForInsetsForTesting() {
        updateAttachedKeyboardToolbarForInsets()
    }

    internal fun scheduleToolbarRefocusForTesting() {
        scheduleToolbarRefocus()
    }

    internal fun focusFromToolbarPreserveForTesting() {
        focusInternal(cancelPendingOutsideTapBlur = false)
    }

    internal fun applyAutoFocusForTesting() {
        applyAutoFocusIfNeeded()
    }

    internal fun installOutsideTapBlurHandlerForTesting() {
        installOutsideTapBlurHandlerIfNeeded()
    }

    internal fun uninstallOutsideTapBlurHandlerForTesting() {
        uninstallOutsideTapBlurHandler()
    }

    internal fun schedulePendingPreflightWakeForTesting() {
        schedulePendingPreflightWake()
    }

    internal fun hasPendingNativeActionForTesting(): Boolean = pendingNativeAction != null

    internal fun pendingNativeActionRetryAttemptsForTesting(): Int = pendingNativeActionRetryAttempts

    internal fun lastDocumentVersionForTesting(): Int? = lastDocumentVersion

    internal fun setLastDocumentVersionForTesting(documentVersion: Int?) {
        lastDocumentVersion = documentVersion
    }

    internal fun refreshToolbarStateFromEditorSelectionForTesting(): String? =
        refreshToolbarStateFromEditorSelection()

    internal fun handleToolbarItemPressForTesting(item: NativeToolbarItem) {
        handleToolbarItemPress(item)
    }

    internal fun insertMentionSuggestionForTesting(suggestion: NativeMentionSuggestion) {
        insertMentionSuggestion(suggestion)
    }

    internal fun wakePendingPreflightWorkForTesting() {
        wakePendingPreflightWork()
    }

    internal fun emitEditorReadyForTesting(editorUpdateRevision: Int? = null): Boolean =
        emitEditorReady(editorUpdateRevision)

    internal fun pendingEditorUpdateJsonForTesting(): String? = pendingEditorUpdateJson

    internal fun pendingEditorUpdateRevisionForTesting(): Int = pendingEditorUpdateRevision

    internal fun pendingEditorResetUpdateJsonForTesting(): String? = pendingEditorResetUpdateJson

    internal fun pendingEditorResetUpdateRevisionForTesting(): Int =
        pendingEditorResetUpdateRevision

    internal fun setAppliedEditorUpdateRevisionForTesting(editorUpdateRevision: Int) {
        appliedEditorUpdateRevision = editorUpdateRevision
    }

    internal fun pendingEditorUpdateEditorIdForTesting(): Long? = pendingEditorUpdateEditorId

    internal fun pendingEditorResetUpdateEditorIdForTesting(): Long? =
        pendingEditorResetUpdateEditorId

    internal fun pendingViewCommandUpdateJsonForTesting(): String? = pendingViewCommandUpdateJson

    internal fun pendingViewCommandUpdateRetryAttemptsForTesting(): Int =
        pendingViewCommandUpdateRetryAttempts

    internal fun scheduleViewCommandUpdateRetryForTesting(updateJson: String) {
        scheduleViewCommandUpdateRetry(updateJson)
    }

    internal fun pendingThemeJsonForTesting(): String? = pendingThemeJson.takeIf { hasPendingTheme }

    internal fun lastThemeJsonForTesting(): String? = lastThemeJson

    internal fun pendingThemeRetryAttemptsForTesting(): Int = pendingThemeRetryAttempts

    internal fun applyPendingThemeForTesting() {
        applyPendingThemeIfNeeded()
    }

    private fun isTouchInsideStandaloneToolbar(event: MotionEvent): Boolean {
        val visibleWindowFrame = Rect()
        getWindowVisibleDisplayFrame(visibleWindowFrame)
        return isPointInsideStandaloneToolbar(event.rawX, event.rawY, visibleWindowFrame)
    }

    internal fun isPointInsideStandaloneToolbarForTesting(
        rawX: Float,
        rawY: Float,
        visibleWindowFrame: Rect
    ): Boolean = isPointInsideStandaloneToolbar(rawX, rawY, visibleWindowFrame)

    private fun isPointInsideStandaloneToolbar(
        rawX: Float,
        rawY: Float,
        visibleWindowFrame: Rect
    ): Boolean {
        if (toolbarFramesInWindow.isEmpty()) {
            return false
        }
        // toolbarFrame is in DP from React Native's measureInWindow, while
        // rawX/rawY are screen pixels. Normalize the event into the visible
        // window before comparing so shifted fallback rectangles cannot
        // preserve focus for unrelated outside taps.
        val density = resources.displayMetrics.density
        val hitSlopPx = TOOLBAR_HIT_SLOP_DP * density
        val eventX = rawX - visibleWindowFrame.left
        val eventY = rawY - visibleWindowFrame.top
        for (toolbarFrame in toolbarFramesInWindow) {
            val windowFrameInPx = RectF(
                toolbarFrame.left * density,
                toolbarFrame.top * density,
                toolbarFrame.right * density,
                toolbarFrame.bottom * density
            ).apply {
                inset(-hitSlopPx, -hitSlopPx)
            }
            if (windowFrameInPx.contains(eventX, eventY)) {
                return true
            }
        }
        return false
    }

    private fun isTouchInsideKeyboardToolbar(event: MotionEvent): Boolean {
        if (keyboardToolbarView.parent == null || keyboardToolbarView.visibility != View.VISIBLE) {
            return false
        }
        val rect = Rect()
        keyboardToolbarView.getGlobalVisibleRect(rect)
        return rect.contains(event.rawX.toInt(), event.rawY.toInt())
    }

    private companion object {
        private const val TOOLBAR_HIT_SLOP_DP = 8f
        private const val TOOLBAR_FOCUS_PRESERVE_MS = 750L
        private const val OUTSIDE_TAP_BLUR_DELAY_MS = 100L
        private const val OUTSIDE_TAP_HANDLER_INSTALL_RETRY_DELAY_MS = 64L
        private const val NATIVE_ACTION_RETRY_DELAY_MS = 16L
        private const val EDITOR_UPDATE_EVENT_DEBOUNCE_MS = 64L
        private const val PENDING_UPDATE_RECOVERY_RETRY_DELAY_MS = 250L
        private const val MAX_NATIVE_ACTION_RETRY_ATTEMPTS = 3
        private const val MAX_PENDING_UPDATE_RETRY_ATTEMPTS = 5
        private const val LOG_TAG = "NativeEditor"

        private fun nanosToMicros(nanos: Long): Long = nanos / 1_000L
    }

    private fun resolveActivity(context: Context): Activity? {
        appContext.currentActivity?.let { return it }
        var current: Context? = context
        while (current is ContextWrapper) {
            if (current is Activity) return current
            current = current.baseContext
        }
        return null
    }

    private fun refreshMentionQuery() {
        val mentions = addons.mentions
        if (mentions == null || !richTextView.editorEditText.hasFocus()) {
            clearMentionQueryState()
            emitMentionQueryChange("", "@", 0, 0, false)
            return
        }

        val queryState = currentMentionQueryState(mentions.trigger)
        if (queryState == null) {
            clearMentionQueryState()
            emitMentionQueryChange("", mentions.trigger, 0, 0, false)
            return
        }

        mentionQueryState = queryState
        val suggestions = filteredMentionSuggestions(queryState, mentions)
        keyboardToolbarView.applyMentionTheme(richTextView.editorEditText.theme?.mentions ?: mentions.theme)
        syncKeyboardToolbarMentionSuggestions(suggestions, mentions.trigger)
        emitMentionQueryChange(
            queryState.query,
            queryState.trigger,
            queryState.anchor,
            queryState.head,
            true
        )
    }

    private fun clearMentionQueryState(resetLastEvent: Boolean = false) {
        mentionQueryState = null
        if (resetLastEvent) {
            lastMentionEventJson = null
            lastMentionEventEditorId = null
        }
        syncKeyboardToolbarMentionSuggestions(emptyList())
    }

    private fun currentMentionQueryState(trigger: String): MentionQueryState? {
        val editor = richTextView.editorEditText
        if (editor.selectionStart != editor.selectionEnd) return null
        val text = editor.text?.toString() ?: return null
        val cursorUtf16 = editor.selectionStart
        val cursorScalar = PositionBridge.utf16ToScalar(cursorUtf16, text)
        return resolveMentionQueryState(
            text = text,
            cursorScalar = cursorScalar,
            trigger = trigger,
            isCaretInsideMention = isCaretInsideMention(cursorUtf16)
        )
    }

    private fun isCaretInsideMention(cursorUtf16: Int): Boolean {
        val editable = richTextView.editorEditText.text ?: return false
        val checkOffsets = listOf(cursorUtf16, (cursorUtf16 - 1).coerceAtLeast(0))
        return checkOffsets.any { offset ->
            editable.getSpans(offset, offset, android.text.Annotation::class.java).any { span ->
                span.key == "nativeVoidNodeType" && span.value == "mention"
            }
        }
    }

    private fun filteredMentionSuggestions(
        queryState: MentionQueryState,
        config: NativeMentionsAddonConfig
    ): List<NativeMentionSuggestion> {
        val normalizedQuery = queryState.query.trim().lowercase()
        if (normalizedQuery.isEmpty()) return config.suggestions
        return config.suggestions.filter { suggestion ->
            suggestion.title.lowercase().contains(normalizedQuery) ||
                suggestion.label.lowercase().contains(normalizedQuery) ||
                (suggestion.subtitle?.lowercase()?.contains(normalizedQuery) == true)
        }
    }

    private fun syncKeyboardToolbarMentionSuggestions(
        suggestions: List<NativeMentionSuggestion>,
        trigger: String = addons.mentions?.trigger ?: "@"
    ) {
        keyboardToolbarView.setMentionSuggestions(suggestions, trigger)
        keyboardToolbarView.requestLayout()
        post {
            updateKeyboardToolbarLayout()
            updateEditorViewportInset()
        }
    }

    private fun emitMentionQueryChange(
        query: String,
        trigger: String,
        anchor: Int,
        head: Int,
        isActive: Boolean
    ) {
        val eventJson = JSONObject()
            .put("type", "mentionsQueryChange")
            .put("query", query)
            .put("trigger", trigger)
            .put("range", JSONObject().put("anchor", anchor).put("head", head))
            .put("isActive", isActive)
            .apply {
                lastDocumentVersion?.let { put("documentVersion", it) }
            }
            .toString()
        val editorId = richTextView.editorId
        if (eventJson == lastMentionEventJson && editorId == lastMentionEventEditorId) return
        lastMentionEventJson = eventJson
        lastMentionEventEditorId = editorId
        emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to editorId))
    }

    private fun resolvedMentionAttrs(
        trigger: String,
        suggestion: NativeMentionSuggestion
    ): JSONObject {
        val attrs = JSONObject(suggestion.attrs.toString())
        if (!attrs.has("label")) {
            attrs.put("label", suggestion.label)
        }
        if (!attrs.has("mentionSuggestionChar")) {
            attrs.put("mentionSuggestionChar", trigger)
        }
        return attrs
    }

    private fun emitMentionSelect(trigger: String, suggestion: NativeMentionSuggestion, attrs: JSONObject) {
        val eventJson = JSONObject()
            .put("type", "mentionsSelect")
            .put("trigger", trigger)
            .put("suggestionKey", suggestion.key)
            .put("attrs", attrs)
            .apply {
                lastDocumentVersion?.let { put("documentVersion", it) }
            }
            .toString()
        emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to richTextView.editorId))
    }

    private fun emitMentionSelectRequest(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: JSONObject,
        range: MentionQueryState,
        preflightUpdateJSON: String?
    ) {
        val eventJson = JSONObject()
            .put("type", "mentionsSelectRequest")
            .put("trigger", trigger)
            .put("suggestionKey", suggestion.key)
            .put("attrs", attrs)
            .put("range", JSONObject().put("anchor", range.anchor).put("head", range.head))
            .apply {
                if (preflightUpdateJSON != null) {
                    put("updateJson", preflightUpdateJSON)
                }
                (documentVersionFromUpdateJSON(preflightUpdateJSON) ?: lastDocumentVersion)
                    ?.let { put("documentVersion", it) }
            }
            .toString()
        emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to richTextView.editorId))
    }

    private fun insertMentionSuggestion(
        suggestion: NativeMentionSuggestion,
        allowPreflightRetry: Boolean = true
    ) {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (!richTextView.editorEditText.isEditable) {
            clearPendingNativeActionRetry()
            return
        }
        val mentions = addons.mentions ?: return
        if (shouldBlockEditorCommandForPendingUpdate()) {
            if (allowPreflightRetry) {
                schedulePendingNativeActionRetry(
                    PendingNativeAction.MentionSuggestionSelect(suggestion)
                )
            }
            return
        }
        val preparation = richTextView.editorEditText.prepareForExternalEditorCommand()
        if (!preparation.ready) {
            if (allowPreflightRetry) {
                schedulePendingNativeActionRetry(
                    PendingNativeAction.MentionSuggestionSelect(suggestion)
                )
            }
            return
        }
        val preflightUpdateJSON = preparation.updateJSON
        noteDocumentVersionFromUpdateJSON(preflightUpdateJSON)
        clearPendingNativeActionRetry()
        val queryState = currentMentionQueryState(mentions.trigger) ?: run {
            clearMentionQueryState()
            return
        }
        val freshSuggestions = filteredMentionSuggestions(queryState, mentions)
        if (freshSuggestions.none { it.key == suggestion.key }) {
            refreshMentionQuery()
            return
        }
        mentionQueryState = queryState
        val attrs = resolvedMentionAttrs(mentions.trigger, suggestion)
        if (mentions.resolveSelectionAttrs || mentions.resolveTheme) {
            emitMentionSelectRequest(mentions.trigger, suggestion, attrs, queryState, preflightUpdateJSON)
            lastMentionEventJson = null
            clearMentionQueryState()
            return
        }
        val docJson = JSONObject()
            .put("type", "doc")
            .put(
                "content",
                JSONArray().put(
                    JSONObject()
                        .put("type", "mention")
                        .put("attrs", attrs)
                )
            )

        val updateJson = editorInsertContentJsonAtSelectionScalar(
            richTextView.editorId.toULong(),
            queryState.anchor.toUInt(),
            queryState.head.toUInt(),
            docJson.toString()
        )
        richTextView.editorEditText.applyUpdateJSON(updateJson)
        emitMentionSelect(mentions.trigger, suggestion, attrs)
        lastMentionEventJson = null
        clearMentionQueryState()
    }

    private fun refreshToolbarStateFromEditorSelection(): String? {
        if (richTextView.editorId == 0L) return null
        if (handleDestroyedCurrentEditorIfNeeded()) return null
        onRefreshToolbarStateFromEditorSelectionForTesting?.let { callback ->
            val stateJson = callback()
            noteDocumentVersionFromUpdateJSON(stateJson)
            return stateJson
        }
        val stateJson = editorGetSelectionState(richTextView.editorId.toULong())
        noteDocumentVersionFromUpdateJSON(stateJson)
        val state = NativeToolbarState.fromUpdateJson(stateJson) ?: return null
        toolbarState = state
        keyboardToolbarView.applyState(state)
        return stateJson
    }

    private fun ensureKeyboardToolbarAttached() {
        val host = resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content) ?: return
        pendingKeyboardToolbarDetachGeneration += 1
        if (keyboardToolbarView.parent === host) {
            updateKeyboardToolbarLayout()
            return
        }
        (keyboardToolbarView.parent as? ViewGroup)?.removeView(keyboardToolbarView)
        host.addView(
            keyboardToolbarView,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM or Gravity.START
            )
        )
        updateKeyboardToolbarLayout()
        ViewCompat.requestApplyInsets(keyboardToolbarView)
    }

    private fun detachKeyboardToolbarIfNeeded() {
        pendingKeyboardToolbarDetachGeneration += 1
        val generation = pendingKeyboardToolbarDetachGeneration
        val parent = keyboardToolbarView.parent as? ViewGroup ?: return
        keyboardToolbarView.visibility = View.GONE
        parent.post {
            if (generation != pendingKeyboardToolbarDetachGeneration) return@post
            if (keyboardToolbarView.parent === parent) {
                parent.removeView(keyboardToolbarView)
            }
        }
    }

    private fun updateKeyboardToolbarLayout() {
        val params = keyboardToolbarView.layoutParams as? FrameLayout.LayoutParams ?: return
        val toolbarTheme = richTextView.editorEditText.theme?.toolbar
        val density = resources.displayMetrics.density
        params.gravity = Gravity.BOTTOM or Gravity.START
        val horizontalInsetPx = ((toolbarTheme?.resolvedHorizontalInset() ?: 0f) * density).toInt()
        val keyboardOffsetPx = ((toolbarTheme?.resolvedKeyboardOffset() ?: 0f) * density).toInt()
        params.leftMargin = horizontalInsetPx
        params.rightMargin = horizontalInsetPx
        params.bottomMargin = currentImeBottom + keyboardOffsetPx
        keyboardToolbarView.layoutParams = params
    }

    private fun updateAttachedKeyboardToolbarForInsets() {
        if (currentImeBottom <= 0) {
            clearPendingNativeActionRetry()
        }
        keyboardToolbarView.visibility = if (currentImeBottom > 0) View.VISIBLE else View.INVISIBLE
        updateEditorViewportInset()
    }

    private fun updateKeyboardToolbarVisibility() {
        val shouldAttach =
            showsToolbar &&
                canFocusCurrentEditor() &&
                toolbarPlacement == ToolbarPlacement.KEYBOARD &&
                richTextView.editorEditText.isEditable &&
                richTextView.editorEditText.hasFocus()

        if (!shouldAttach) {
            keyboardToolbarView.visibility = View.GONE
            detachKeyboardToolbarIfNeeded()
            updateEditorViewportInset()
            return
        }

        ensureKeyboardToolbarAttached()
        keyboardToolbarView.visibility = if (currentImeBottom > 0) View.VISIBLE else View.INVISIBLE
        updateEditorViewportInset()
    }

    private fun updateEditorViewportInset(forceMeasureToolbar: Boolean = false) {
        val shouldReserveToolbarSpace =
            heightBehavior == EditorHeightBehavior.FIXED &&
                showsToolbar &&
                toolbarPlacement == ToolbarPlacement.KEYBOARD &&
                richTextView.editorEditText.isEditable &&
                richTextView.editorEditText.hasFocus() &&
                currentImeBottom > 0

        if (!shouldReserveToolbarSpace) {
            richTextView.setViewportBottomInsetPx(0)
            return
        }

        val hostWidth = (resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content)?.width ?: width)
            .coerceAtLeast(0)
        val toolbarTheme = richTextView.editorEditText.theme?.toolbar
        val density = resources.displayMetrics.density
        val horizontalInsetPx = ((toolbarTheme?.resolvedHorizontalInset() ?: 0f) * density).toInt()
        if (forceMeasureToolbar || keyboardToolbarView.measuredHeight == 0) {
            val availableWidth = (hostWidth - horizontalInsetPx * 2).coerceAtLeast(0)
            val widthSpec = MeasureSpec.makeMeasureSpec(availableWidth, MeasureSpec.AT_MOST)
            val heightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
            keyboardToolbarView.measure(widthSpec, heightSpec)
        }
        val toolbarHeight = keyboardToolbarView.measuredHeight.coerceAtLeast(keyboardToolbarView.height)
        richTextView.setViewportBottomInsetPx(toolbarHeight.coerceAtLeast(0))
    }

    private fun handleListToggle(listType: String) {
        val isActive = toolbarState.nodes[listType] == true
        richTextView.editorEditText.performToolbarToggleList(listType, isActive)
    }

    private fun handleToolbarItemPress(
        item: NativeToolbarItem,
        allowPreflightRetry: Boolean = true
    ) {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        if (!richTextView.editorEditText.isEditable) {
            clearPendingNativeActionRetry()
            return
        }
        var preflightUpdateJSON: String? = null
        val needsEditorPreflight = when (item.type) {
            ToolbarItemKind.mark,
            ToolbarItemKind.heading,
            ToolbarItemKind.blockquote,
            ToolbarItemKind.list,
            ToolbarItemKind.command,
            ToolbarItemKind.node,
            ToolbarItemKind.action -> true
            ToolbarItemKind.group,
            ToolbarItemKind.separator -> false
        }
        if (needsEditorPreflight) {
            if (shouldBlockEditorCommandForPendingUpdate()) {
                if (allowPreflightRetry) {
                    schedulePendingNativeActionRetry(PendingNativeAction.ToolbarItemPress(item))
                }
                return
            }
            val preparation = richTextView.editorEditText.prepareForExternalEditorCommand()
            if (!preparation.ready) {
                if (allowPreflightRetry) {
                    schedulePendingNativeActionRetry(PendingNativeAction.ToolbarItemPress(item))
                }
                return
            }
            preflightUpdateJSON = preparation.updateJSON
            noteDocumentVersionFromUpdateJSON(preflightUpdateJSON)
            clearPendingNativeActionRetry()
        }
        if (handleDestroyedCurrentEditorIfNeeded()) return
        when (item.type) {
            ToolbarItemKind.mark -> item.mark?.let { richTextView.editorEditText.performToolbarToggleMark(it) }
            ToolbarItemKind.heading -> item.headingLevel?.let { richTextView.editorEditText.performToolbarToggleHeading(it) }
            ToolbarItemKind.blockquote -> richTextView.editorEditText.performToolbarToggleBlockquote()
            ToolbarItemKind.list -> item.listType?.name?.let { handleListToggle(it) }
            ToolbarItemKind.command -> when (item.command) {
                ToolbarCommand.indentList -> richTextView.editorEditText.performToolbarIndentListItem()
                ToolbarCommand.outdentList -> richTextView.editorEditText.performToolbarOutdentListItem()
                ToolbarCommand.undo -> richTextView.editorEditText.performToolbarUndo()
                ToolbarCommand.redo -> richTextView.editorEditText.performToolbarRedo()
                null -> Unit
            }
            ToolbarItemKind.node -> item.nodeType?.let { richTextView.editorEditText.performToolbarInsertNode(it) }
            ToolbarItemKind.action -> item.key?.let {
                if (handleDestroyedCurrentEditorIfNeeded()) return
                val payload = mutableMapOf<String, Any>(
                    "key" to it,
                    "editorId" to richTextView.editorId
                )
                addPreflightUpdateToEvent(payload, preflightUpdateJSON)
                if (!payload.containsKey("documentVersion")) {
                    lastDocumentVersion?.let { version ->
                        payload["documentVersion"] = version
                    }
                }
                onToolbarActionForTesting?.invoke(payload) ?: onToolbarAction(payload)
            }
            ToolbarItemKind.group -> Unit
            ToolbarItemKind.separator -> Unit
        }
    }

}
