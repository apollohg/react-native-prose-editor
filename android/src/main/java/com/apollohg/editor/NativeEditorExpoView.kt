package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Point
import android.graphics.Rect
import android.graphics.RectF
import android.os.Build
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
import android.widget.ScrollView
import androidx.core.widget.NestedScrollView
import androidx.core.view.ViewCompat
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
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.abs
import uniffi.editor_core.editorV2GetState

private const val DESTROY_INVALIDATION_AWAIT_TIMEOUT_MS = 250L
private const val ATOM_NATIVE_ID_PREFIX = "prose-atom:"
private val nextNativeEditorErrorCallbackToken = AtomicLong(0)

private class ExpoAutoGrowStyleSizePublisher(private val view: ExpoView) {
    private data class Binding(
        val proxy: Any,
        val method: java.lang.reflect.Method,
        val stateWrapperGetter: java.lang.reflect.Method,
    )

    private val binding: Binding? by lazy(LazyThreadSafetyMode.NONE) {
        runCatching {
            val proxy = requireNotNull(
                view.javaClass.methods
                    .first { it.name == "getShadowNodeProxy" && it.parameterCount == 0 }
                    .invoke(view)
            )
            val method = proxy.javaClass.methods
                .first { it.name == "setStyleSize" && it.parameterCount == 2 }
            val stateWrapperGetter = view.javaClass.methods
                .first { it.name == "getStateWrapper" && it.parameterCount == 0 }
            Binding(proxy, method, stateWrapperGetter)
        }.getOrNull()
    }

    fun publish(heightDp: Double?): Boolean {
        val resolvedBinding = binding ?: return false
        return runCatching {
            if (resolvedBinding.stateWrapperGetter.invoke(view) == null) return false
            resolvedBinding.method.invoke(resolvedBinding.proxy, null, heightDp)
        }.isSuccess
    }
}

internal enum class NativeEditorOutsideTapDecision {
    IGNORE,
    PRESERVE_FOCUS,
    OUTSIDE_EDITOR
}

internal data class NativeEditorOutsideTapRouteTestState(
    val isRegistered: Boolean,
    val hasCallbackReconciler: Boolean
)

private enum class PendingEditorUpdateApplyOutcome {
    APPLIED,
    RETRYABLE_DEFERRED,
    PERMANENTLY_REJECTED
}

private enum class PendingEditorUpdateKind {
    ORDINARY,
    RESET
}

internal enum class NativeEditorDestroyReservationResult {
    RESERVED,
    ALREADY_IN_PROGRESS,
    UNAVAILABLE,
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

    private val liveEditorIds = mutableSetOf<Long>()
    private val viewsByEditorId = mutableMapOf<Long, MutableList<WeakNativeEditorExpoView>>()
    private val inputViewsByEditorId = mutableMapOf<Long, MutableList<WeakReference<EditorEditText>>>()
    private val detachedEditorOwnersByEditorId = mutableMapOf<Long, WeakNativeEditorExpoView>()
    private val destroyingEditorIds = mutableSetOf<Long>()
    private val destroyReservationWasLive = mutableMapOf<Long, Boolean>()
    private val mainHandler = Handler(Looper.getMainLooper())
    @Volatile
    internal var onFinalizeDestroyForTesting: ((Long) -> Unit)? = null

    @Synchronized
    fun markEditorCreated(editorId: Long) {
        if (editorId == 0L) return
        liveEditorIds.add(editorId)
        destroyingEditorIds.remove(editorId)
        destroyReservationWasLive.remove(editorId)
    }

    @Synchronized
    fun register(editorId: Long, view: NativeEditorExpoView): Boolean {
        if (editorId == 0L) return false
        if (destroyingEditorIds.contains(editorId)) return false
        val isKnownDetachedOwner = detachedEditorOwnersByEditorId[editorId]?.view?.get() === view
        if (
            !liveEditorIds.contains(editorId) &&
            !isKnownDetachedOwner &&
            !rustEditorExists(editorId)
        ) return false
        val views = viewsByEditorId.getOrPut(editorId) { mutableListOf() }
        views.removeAll { it.view.get() == null || it.view.get() === view }
        views += WeakNativeEditorExpoView(view)
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
        val registeredViews = viewsByEditorId[editorId]
        val wasRegistered = registeredViews?.any { it.view.get() === view } == true
        registeredViews?.removeAll { it.view.get() == null || it.view.get() === view }
        if (registeredViews?.isEmpty() == true) viewsByEditorId.remove(editorId)
        if (blockCommandsUntilRegistered) {
            detachedEditorOwnersByEditorId[editorId] = WeakNativeEditorExpoView(view)
        } else {
            val detachedOwner = detachedEditorOwnersByEditorId[editorId]?.view?.get()
            if (wasRegistered || detachedOwner === view) {
                detachedEditorOwnersByEditorId.remove(editorId)
            }
        }
    }

    @Synchronized
    fun isDestroyed(editorId: Long): Boolean = destroyingEditorIds.contains(editorId)

    @Synchronized
    internal fun retainedDestroyedIdCountForTests(): Int = destroyingEditorIds.size

    @Synchronized
    internal fun forceDetachedOwnerClearedForTesting(editorId: Long) {
        detachedEditorOwnersByEditorId[editorId] = WeakNativeEditorExpoView.cleared()
    }

    @Synchronized
    internal fun forceRegisteredViewsClearedForTesting(editorId: Long) {
        viewsByEditorId[editorId] = mutableListOf(WeakNativeEditorExpoView.cleared())
    }

    @Synchronized
    internal fun boundViewReferenceCountForTests(editorId: Long): Int =
        viewsByEditorId[editorId]?.size ?: 0

    @Synchronized
    private fun liveViewsFor(editorId: Long): List<NativeEditorExpoView> =
        viewsByEditorId[editorId]?.mapNotNull { it.view.get() }.orEmpty()

    fun rebaseAfterRemoteCommit(handle: String) {
        val viewToken = EditorV2Registry.viewTokenForHandle(handle) ?: return
        liveViewsFor(viewToken).forEach { view ->
            if (view.markRemoteCommitRebaseScheduled(viewToken)) {
                mainHandler.post { view.applyRemoteCommitRefresh(viewToken) }
            }
        }
    }

    @Synchronized
    fun registerInputView(editorId: Long, view: EditorEditText) {
        if (editorId == 0L || destroyingEditorIds.contains(editorId)) return
        if (!liveEditorIds.contains(editorId) && !rustEditorExists(editorId)) return
        val views = inputViewsByEditorId.getOrPut(editorId) { mutableListOf() }
        views.removeAll { it.get() == null || it.get() === view }
        views += WeakReference(view)
    }

    @Synchronized
    fun acquireDestroyReservation(editorId: Long): NativeEditorDestroyReservationResult {
        if (editorId == 0L) return NativeEditorDestroyReservationResult.UNAVAILABLE
        if (destroyingEditorIds.contains(editorId)) {
            return NativeEditorDestroyReservationResult.ALREADY_IN_PROGRESS
        }
        if (!liveEditorIds.contains(editorId) && !rustEditorExists(editorId)) {
            return NativeEditorDestroyReservationResult.UNAVAILABLE
        }
        destroyingEditorIds.add(editorId)
        destroyReservationWasLive[editorId] = liveEditorIds.remove(editorId)
        return NativeEditorDestroyReservationResult.RESERVED
    }

    fun beginDestroy(editorId: Long): Boolean =
        acquireDestroyReservation(editorId) == NativeEditorDestroyReservationResult.RESERVED

    /** Undo an in-flight reservation after an FFI result that is retryable or malformed. */
    @Synchronized
    fun rollbackDestroy(editorId: Long) {
        if (!destroyingEditorIds.remove(editorId)) return
        if (destroyReservationWasLive.remove(editorId) == true) {
            liveEditorIds.add(editorId)
        }
    }

    fun invalidateDestroyedEditor(editorId: Long) {
        if (!beginDestroy(editorId)) return
        finalizeDestroy(editorId)
    }

    /**
     * Finalize a reserved destroy. Off-main callers may time out while the
     * main-thread invalidation is still queued, so completion is reported only
     * after the reservation itself has been released.
     */
    fun finalizeDestroy(editorId: Long, onCompleted: (() -> Unit)? = null) {
        val affectedViews = synchronized(this) {
            if (!destroyingEditorIds.contains(editorId)) return@synchronized null
            val views = listOfNotNull(
                *viewsByEditorId.remove(editorId)
                    .orEmpty()
                    .mapNotNull { it.view.get() }
                    .toTypedArray(),
                detachedEditorOwnersByEditorId.remove(editorId)?.view?.get()
            ).distinct()
            val inputViews = inputViewsByEditorId.remove(editorId)
                .orEmpty()
                .mapNotNull { it.get() }
                .distinct()
            views to inputViews
        }
        if (affectedViews == null) {
            onCompleted?.invoke()
            return
        }
        fun releaseReservation() {
            synchronized(this) {
                destroyingEditorIds.remove(editorId)
                destroyReservationWasLive.remove(editorId)
            }
            onCompleted?.invoke()
        }
        invokeDestroyTestingHook(onFinalizeDestroyForTesting, editorId)
        if (affectedViews.first.isEmpty() && affectedViews.second.isEmpty()) {
            releaseReservation()
            return
        }
        val invalidate = Runnable {
            try {
                affectedViews.first.forEach { view ->
                    view.handleEditorDestroyed(editorId)
                }
                affectedViews.second.forEach { view ->
                    view.handleEditorDestroyedFromRegistry(editorId)
                }
            } finally {
                releaseReservation()
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
            if (!posted) {
                releaseReservation()
                return
            }
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
                val isDestroyed = destroyingEditorIds.contains(editorId)
                if (isDestroyed) {
                    return@synchronized CommandPreparationSnapshot(
                        view = null,
                        isDetached = false,
                        isDestroyed = true
                    )
                }
                val registeredViews = viewsByEditorId[editorId]
                registeredViews?.removeAll { it.view.get() == null }
                val candidate = registeredViews?.firstNotNullOfOrNull { it.view.get() }
                if (registeredViews?.isEmpty() == true) {
                    viewsByEditorId.remove(editorId)
                }
                val detachedOwner = detachedEditorOwnersByEditorId[editorId]?.view?.get()
                val isDetached = if (detachedOwner == null) {
                    detachedEditorOwnersByEditorId.remove(editorId)
                    false
                } else {
                    true
                }
                val missingFromRust =
                    !liveEditorIds.contains(editorId) &&
                        candidate == null &&
                        !isDetached &&
                        !rustEditorExists(editorId)
                CommandPreparationSnapshot(
                    view = candidate,
                    isDetached = isDetached,
                    isDestroyed = missingFromRust
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

    private fun rustEditorExists(viewToken: Long): Boolean =
        EditorV2Registry.adapterForViewToken(viewToken) != null

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
    private val dispatchers = WeakHashMap<Window, WeakReference<OutsideTapWindowRoute>>()

    fun register(window: Window, view: NativeEditorExpoView): Boolean {
        val dispatcher = dispatcherFor(window) ?: OutsideTapWindowRoute(window).also {
            dispatchers[window] = WeakReference(it)
        }
        dispatcher.add(view)
        val isAttached = dispatcher.reconcileCallback()
        view.traceOutsideTap(
            "register callbackAttached=$isAttached activeViews=${dispatcher.liveViews().size}"
        )
        return isAttached
    }

    fun unregister(window: Window, view: NativeEditorExpoView) {
        val dispatcher = dispatcherFor(window) ?: return
        if (!dispatcher.remove(view)) return
        dispatcher.detach()
        removeDispatcher(window, dispatcher)
    }

    internal fun dispatchForTesting(window: Window, event: MotionEvent): Boolean =
        window.callback?.dispatchTouchEvent(event) ?: false

    internal fun setCycleBreakDispatcherForTesting(
        window: Window,
        dispatcher: ((MotionEvent) -> Boolean)?
    ): Boolean {
        val route = dispatcherFor(window) ?: return false
        route.setCycleBreakDispatcherForTesting(dispatcher)
        return true
    }

    internal fun clearViewReferenceAndReconcileForTesting(
        window: Window,
        view: NativeEditorExpoView
    ): NativeEditorOutsideTapRouteTestState {
        val route = dispatcherFor(window)
            ?: return NativeEditorOutsideTapRouteTestState(
                isRegistered = false,
                hasCallbackReconciler = false
            )
        route.clearViewReferenceForTesting(view)
        route.reconcileCallback()
        return NativeEditorOutsideTapRouteTestState(
            isRegistered = dispatchers[window]?.get() === route,
            hasCallbackReconciler = route.hasCallbackReconcilerForTesting()
        )
    }

    private fun dispatcherFor(window: Window): OutsideTapWindowRoute? {
        val reference = dispatchers[window] ?: return null
        val dispatcher = reference.get()
        if (dispatcher == null) {
            dispatchers.remove(window)
            return null
        }
        if (dispatcher.hasLiveViews()) {
            return dispatcher
        }
        dispatcher.detach()
        removeDispatcher(window, dispatcher)
        return null
    }

    private fun removeDispatcher(window: Window, dispatcher: OutsideTapWindowRoute) {
        if (dispatchers[window]?.get() === dispatcher) {
            dispatchers.remove(window)
        }
    }

    private class OutsideTapWindowRoute(
        window: Window
    ) {
        private data class OutsideTapCandidate(
            val view: WeakReference<NativeEditorExpoView>,
            val downRawX: Float,
            val downRawY: Float,
            val editorRectOnDown: Rect?
        )

        private val views = mutableListOf<WeakReference<NativeEditorExpoView>>()
        private val pendingOutsideTapCandidates = mutableListOf<OutsideTapCandidate>()
        private val windowRef = WeakReference(window)
        private val touchSlopPx = ViewConfiguration.get(window.context).scaledTouchSlop
        private var callback: OutsideTapWindowCallback? = null
        private var callbackBase: Window.Callback? = null
        private var callbackTreeObserver: ViewTreeObserver? = null
        private var cycleBreakDispatcherForTesting: ((MotionEvent) -> Boolean)? = null
        private var observationDepth = 0
        private val callbackReconciler = ViewTreeObserver.OnPreDrawListener {
            reconcileCallback()
            true
        }

        fun add(view: NativeEditorExpoView) {
            prune()
            if (views.any { it.get() === view }) return
            views.add(WeakReference(view))
            ensureCallbackReconciler()
        }

        fun hasLiveViews(): Boolean {
            prune()
            return views.isNotEmpty()
        }

        fun setCycleBreakDispatcherForTesting(dispatcher: ((MotionEvent) -> Boolean)?) {
            cycleBreakDispatcherForTesting = dispatcher
        }

        fun clearViewReferenceForTesting(view: NativeEditorExpoView) {
            views.firstOrNull { it.get() === view }?.clear()
            prune()
        }

        fun hasCallbackReconcilerForTesting(): Boolean = callbackTreeObserver?.isAlive == true

        fun liveViews(): List<NativeEditorExpoView> {
            prune()
            return views.mapNotNull { it.get() }
        }

        fun remove(view: NativeEditorExpoView): Boolean {
            cancelPendingOutsideTapCandidatesFor(view, "remove view")
            views.removeAll { it.get()?.let { candidate -> candidate === view } != false }
            return views.isEmpty()
        }

        fun reconcileCallback(): Boolean {
            val window = windowRef.get() ?: return false
            if (!hasLiveViews()) {
                detach()
                removeDispatcher(window, this)
                return false
            }
            ensureCallbackReconciler()
            val activeCallback = callback
            if (activeCallback != null && window.callback === activeCallback) {
                return true
            }

            val foreignCallback = window.callback ?: return false
            val replacement = OutsideTapWindowCallback(this, foreignCallback)
            callbackBase = foreignCallback
            callback = replacement
            window.callback = replacement
            if (window.callback === replacement) {
                return true
            }

            callback = null
            callbackBase = null
            return false
        }

        fun detach() {
            cancelPendingOutsideTapCandidates("detach")
            removeCallbackReconciler()
            val window = windowRef.get()
            val activeCallback = callback
            val baseCallback = callbackBase
            if (window != null && activeCallback != null && window.callback === activeCallback && baseCallback != null) {
                window.callback = baseCallback
            }
            callback = null
            callbackBase = null
            cycleBreakDispatcherForTesting = null
        }

        fun dispatchTouchEvent(baseCallback: Window.Callback, event: MotionEvent): Boolean {
            if (observationDepth > 0) {
                return baseCallback.dispatchTouchEvent(event)
            }
            val activeViews = liveViews()
            if (activeViews.isEmpty()) {
                detach()
                windowRef.get()?.let { window -> removeDispatcher(window, this) }
                return baseCallback.dispatchTouchEvent(event)
            }

            observationDepth += 1
            return try {
                when (event.actionMasked) {
                    MotionEvent.ACTION_DOWN -> {
                        handleActionDown(activeViews, event)
                        baseCallback.dispatchTouchEvent(event)
                    }
                    MotionEvent.ACTION_MOVE -> {
                        val result = baseCallback.dispatchTouchEvent(event)
                        if (hasMovedBeyondTapSlop(event)) {
                            cancelPendingOutsideTapCandidates("move")
                        }
                        result
                    }
                    MotionEvent.ACTION_UP -> {
                        val result = baseCallback.dispatchTouchEvent(event)
                        if (hasMovedBeyondTapSlop(event)) {
                            cancelPendingOutsideTapCandidates("up moved")
                        } else {
                            confirmPendingOutsideTapCandidates("up")
                        }
                        result
                    }
                    MotionEvent.ACTION_CANCEL -> {
                        val result = baseCallback.dispatchTouchEvent(event)
                        cancelPendingOutsideTapCandidates("cancel")
                        result
                    }
                    else -> baseCallback.dispatchTouchEvent(event)
                }
            } finally {
                observationDepth -= 1
            }
        }

        fun dispatchTouchEventOnCallbackReentry(
            baseCallback: Window.Callback,
            event: MotionEvent
        ): Boolean = cycleBreakDispatcherForTesting?.invoke(event)
            ?: windowRef.get()?.superDispatchTouchEvent(event)
            ?: baseCallback.dispatchTouchEvent(event)

        private fun ensureCallbackReconciler() {
            val window = windowRef.get() ?: return
            val currentObserver = callbackTreeObserver
            val nextObserver = window.decorView.viewTreeObserver ?: return
            if (currentObserver === nextObserver && currentObserver.isAlive) return
            removeCallbackReconciler()
            if (nextObserver.isAlive) {
                nextObserver.addOnPreDrawListener(callbackReconciler)
                callbackTreeObserver = nextObserver
            }
        }

        private fun handleActionDown(activeViews: List<NativeEditorExpoView>, event: MotionEvent) {
            cancelPendingOutsideTapCandidates("new down")
            val decisions = activeViews.map { view ->
                view to view.prepareOutsideTapDecisionForWindowEvent(event)
            }
            decisions.forEach { (view, decision) ->
                view.traceOutsideTap(
                    "dispatch callback action=${event.action} raw=${event.rawX.toInt()},${event.rawY.toInt()} decision=$decision"
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
            pendingOutsideTapCandidates.add(
                OutsideTapCandidate(
                    view = WeakReference(view),
                    downRawX = event.rawX,
                    downRawY = event.rawY,
                    editorRectOnDown = editorRectOnDown
                )
            )
            view.traceOutsideTap("candidate outside tap")
        }

        private fun confirmPendingOutsideTapCandidates(reason: String) {
            pendingOutsideTapCandidates.toList().forEach { candidate ->
                confirmOutsideTapCandidate(candidate, reason)
            }
        }

        private fun confirmOutsideTapCandidate(candidate: OutsideTapCandidate, reason: String) {
            if (!pendingOutsideTapCandidates.remove(candidate)) return
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
            pendingOutsideTapCandidates.toList().forEach { candidate ->
                if (candidate.view.get() === view) {
                    pendingOutsideTapCandidates.remove(candidate)
                    view.traceOutsideTap("cancel outside tap candidate reason=$reason")
                }
            }
        }

        private fun cancelPendingOutsideTapCandidates(reason: String) {
            val candidates = pendingOutsideTapCandidates.toList()
            pendingOutsideTapCandidates.clear()
            candidates.forEach { candidate ->
                candidate.view.get()?.traceOutsideTap("cancel outside tap candidate reason=$reason")
            }
        }

        private fun removeCallbackReconciler() {
            val observer = callbackTreeObserver
            if (observer?.isAlive == true) {
                observer.removeOnPreDrawListener(callbackReconciler)
            }
            callbackTreeObserver = null
        }

        private fun prune() {
            views.removeAll { it.get() == null }
        }

        private class OutsideTapWindowCallback(
            private val route: OutsideTapWindowRoute,
            private val baseCallback: Window.Callback
        ) : Window.Callback by baseCallback {
            private var dispatchDepth = 0

            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                if (dispatchDepth > 0) {
                    return route.dispatchTouchEventOnCallbackReentry(baseCallback, event)
                }
                dispatchDepth += 1
                return try {
                    route.dispatchTouchEvent(baseCallback, event)
                } finally {
                    dispatchDepth -= 1
                }
            }
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

    private data class PendingEditorUpdateEvent(
        /** Captured public source identity; never derive this after a rebind. */
        val editorId: String,
        /** Captured canonical document revision used by TS echo suppression. */
        val documentRevision: String,
        val viewUpdateJSON: String,
        val atomicUpdateJSON: String
    )

    private data class NativeCommitKey(
        val editorId: String,
        val documentRevision: String,
    )

    private data class EditorErrorBinding(
        val adapter: EditorV2Adapter,
        val editorId: String,
        val viewToken: Long,
        val callbackToken: Long,
        val generation: Long,
    )

    private data class PendingEditorErrorEvent(
        /** Capture every identity at callback time; never derive it after a rebind. */
        val adapter: EditorV2Adapter,
        val editorId: String,
        val viewToken: Long,
        val callbackToken: Long,
        val bindingGeneration: Long,
        val error: EditorV2Error,
    )

    private data class PreflightUpdateEvent(
        val updateJSON: String,
        val documentRevision: String
    )

    private data class ActiveExternalTextComposition(
        val sessionId: String,
        val editorId: String,
    )

    val richTextView: RichTextEditorView = RichTextEditorView(context)
    private val keyboardToolbarView = EditorKeyboardToolbarView(context)
    private val mainHandler = Handler(Looper.getMainLooper())
    private val keyboardToolbarImeAnimationController = KeyboardToolbarImeAnimationController(
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

    private val onEditorUpdate by EventDispatcher<Map<String, Any>>()
    private val onEditorError by EventDispatcher<Map<String, Any>>()
    private val onExternalTextCompositionEnd by EventDispatcher<Map<String, Any>>()
    private val onSelectionChange by EventDispatcher<Map<String, Any>>()
    private val onFocusChange by EventDispatcher<Map<String, Any>>()
    private val onContentHeightChange by EventDispatcher<Map<String, Any>>()
    private val onAtomLayout by EventDispatcher<Map<String, Any>>()
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
    internal var onAtomLayoutForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorUpdateForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorErrorForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onExternalTextCompositionEndForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onEditorReadyForTesting: ((Map<String, Any>) -> Unit)? = null
    internal var onOutsideTapTraceForTesting: ((String) -> Unit)? = null
    internal var onRefreshToolbarStateFromEditorSelectionForTesting: (() -> String?)? = null
    internal var onBeforePrepareForEditorCommandForTesting: (() -> Unit)? = null
    private var isAttachedToNativeWindow = false
    private var didApplyAutoFocus = false
    private var heightBehavior = EditorHeightBehavior.FIXED
    private var lastEmittedContentHeight = 0
    private var lastEmittedContentHeightEditorId: Long? = null
    private val autoGrowStyleSizePublisher = ExpoAutoGrowStyleSizePublisher(this)
    private var lastPublishedAutoGrowHeightDp: Double? = null
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
    private var lastAtomsJson: String? = null
    private val reactChildren = mutableListOf<View>()
    private val atomScrollTouchSlopPx = ViewConfiguration.get(context).scaledTouchSlop
    private var atomScrollGestureActive = false
    private var atomScrollGestureIntercepted = false
    private var atomScrollGestureForwarding = false
    private var atomScrollDownX = 0f
    private var atomScrollDownY = 0f
    private var lastRemoteSelectionsJson: String? = null
    private var lastToolbarItemsJson: String? = null
    private var lastToolbarFrameJson: String? = null
    private var lastDocumentVersion: String? = null
    private var renderedDocumentRevision: String? = null
    @Volatile
    private var remoteCommitRebaseScheduled = false
    private var remoteCommitRebaseEditorId: Long? = null
    private var activeExternalTextComposition: ActiveExternalTextComposition? = null
    private var toolbarState = NativeToolbarState.empty
    private var showsToolbar = true
    private var toolbarPlacement = ToolbarPlacement.KEYBOARD
    private var currentImeBottom = 0
    private var pendingEditorUpdateJson: String? = null
    private var pendingEditorUpdateEditorId: Long? = null
    private var pendingEditorUpdateRevision = 0L
    private var appliedEditorUpdateRevision = 0L
    /** Permanently rejected prop revisions are consumed per bound editor. */
    private var consumedEditorUpdateRevision = 0L
    private var consumedEditorUpdateEditorId: Long? = null
    private var pendingEditorResetUpdateJson: String? = null
    private var pendingEditorResetUpdateEditorId: Long? = null
    private var pendingEditorResetUpdateRevision = 0L
    private var appliedEditorResetUpdateRevision = 0L
    /** Permanently rejected reset revisions are consumed per bound editor. */
    private var consumedEditorResetUpdateRevision = 0L
    private var consumedEditorResetUpdateEditorId: Long? = null
    private var lastEditorUpdateJsonProp: String? = null
    private var lastEditorUpdateEditorIdProp: Long? = null
    private var lastEditorResetUpdateJsonProp: String? = null
    private var lastEditorResetUpdateEditorIdProp: Long? = null
    private var pendingEditorUpdateRetryScheduled = false
    private var pendingEditorUpdateRetryEditorId: Long? = null
    private var pendingEditorUpdateRetryKind: PendingEditorUpdateKind? = null
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
    private val pendingEditorUpdateKeys = mutableSetOf<NativeCommitKey>()
    private var pendingEditorUpdateDispatchGeneration = 0
    private var pendingEditorUpdateDispatchScheduled = false
    private val pendingEditorErrorEvents = java.util.ArrayDeque<PendingEditorErrorEvent>()
    private var pendingEditorErrorDispatchGeneration = 0
    private var pendingEditorErrorDispatchScheduled = false
    private var editorErrorBinding: EditorErrorBinding? = null
    private var nextEditorErrorBindingGeneration = 0L

    /** Public v2 handles are decimal strings; [RichTextEditorView] uses an opaque local token. */
    private fun publicHandleForViewToken(viewToken: Long): String? =
        EditorV2Registry.handleForViewToken(viewToken)

    /** Events never expose a signed widget token as a v2 editor id. */
    private fun eventEditorId(viewToken: Long): String =
        publicHandleForViewToken(viewToken) ?: "0"

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

    fun setEditorHandle(handle: String?) {
        val viewToken = handle?.let(EditorV2Registry::viewTokenForHandle)
        setEditorId(viewToken ?: 0L)
    }

    /**
     * Internal-only widget binding. This token is allocated by
     * [EditorV2Registry] and is never a public session identifier.
     */
    fun setEditorId(id: Long) {
        if (id != 0L && NativeEditorViewRegistry.isDestroyed(id)) {
            setEditorId(0L)
            return
        }
        val previousEditorId = richTextView.editorId
        if (previousEditorId != id) {
            cancelActiveExternalTextComposition("lifecycle")
            invalidateAutoGrowContentHeightEmission()
            drainPendingEditorUpdateEvents()
            clearEditorErrorBinding("editorRebind")
        }
        if (previousEditorId == id && richTextView.editorEditText.editorId == id) {
            if (id != 0L && isAttachedToNativeWindow) {
                if (!NativeEditorViewRegistry.register(id, this)) {
                    handleEditorDestroyed(id)
                    return
                }
                bindEditorErrorCallbackIfLive(id)
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
            richTextView.emitAtomLayoutIfAvailable(force = true)
            return
        }
        if (previousEditorId != id) {
            NativeEditorViewRegistry.unregister(previousEditorId, this)
            lastDocumentVersion = null
            renderedDocumentRevision = null
            cancelPendingToolbarRefocus()
            cancelPendingEditorUpdateRetry()
            if (pendingEditorUpdateEditorId != null && pendingEditorUpdateEditorId != id) {
                clearPendingEditorUpdateState()
            }
            if (pendingEditorResetUpdateEditorId != null && pendingEditorResetUpdateEditorId != id) {
                clearPendingEditorResetUpdateState()
            }
            appliedEditorUpdateRevision = 0L
            appliedEditorResetUpdateRevision = 0L
            consumedEditorUpdateRevision = 0L
            consumedEditorUpdateEditorId = null
            consumedEditorResetUpdateRevision = 0L
            consumedEditorResetUpdateEditorId = null
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
            richTextView.emitAtomLayoutIfAvailable(force = true)
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
            bindEditorErrorCallbackIfLive(id)
        } else {
            toolbarState = NativeToolbarState.empty
            keyboardToolbarView.applyState(toolbarState)
        }
        applyPendingEditorResetUpdateIfNeeded()
        applyPendingEditorUpdateIfNeeded()
        applyPendingThemeIfNeeded()
        refreshReadyStateIfSettled()
        applyAutoFocusIfNeeded()
        richTextView.emitAtomLayoutIfAvailable(force = true)
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
            publishAutoGrowStyleHeight(null)
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

    fun setAtomsJson(atomsJson: String?) {
        if (lastAtomsJson == atomsJson) return
        val configuration = AtomRenderConfiguration.fromJson(atomsJson)
        if (!richTextView.applyAtomRenderConfiguration(configuration)) return
        lastAtomsJson = atomsJson
    }

    internal val atomChildCount: Int
        get() = reactChildren.size

    internal fun atomChildAt(index: Int): View? = reactChildren.getOrNull(index)

    internal fun addAtomChild(child: View, index: Int) {
        reactChildren.remove(child)
        reactChildren.add(index.coerceIn(0, reactChildren.size), child)
        val atomKey = atomKey(child)
        if (atomKey == null) {
            (child.parent as? ViewGroup)?.removeView(child)
            super.addView(child, childCount)
            return
        }
        (child.parent as? ViewGroup)?.removeView(child)
        super.addView(child, childCount)
        richTextView.mountAtomChild(child, atomKey)
    }

    internal fun removeAtomChildAt(index: Int) {
        reactChildren.getOrNull(index)?.let(::removeAtomChild)
    }

    internal fun removeAtomChild(child: View) {
        reactChildren.remove(child)
        if (!richTextView.unmountAtomChild(child)) {
            (child.parent as? ViewGroup)?.removeView(child)
        }
    }

    override fun dispatchTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            atomScrollGestureActive = atomChildAt(event.x, event.y) != null
            atomScrollGestureIntercepted = false
            atomScrollGestureForwarding = atomScrollGestureActive
            atomScrollDownX = event.x
            atomScrollDownY = event.y
        }
        if (!atomScrollGestureActive) return super.dispatchTouchEvent(event)

        val horizontalIntent =
            !atomScrollGestureIntercepted &&
                atomScrollGestureForwarding &&
                event.actionMasked == MotionEvent.ACTION_MOVE &&
                atomScrollMovedHorizontallyBeyondSlop(event)
        val scrollHandled = when {
            horizontalIntent -> {
                atomScrollGestureForwarding = false
                cancelAtomScrollTouch(event)
            }
            atomScrollGestureForwarding -> dispatchAtomScrollTouch(event)
            else -> false
        }
        if (
            !atomScrollGestureIntercepted &&
            atomScrollGestureForwarding &&
            event.actionMasked == MotionEvent.ACTION_MOVE &&
            atomScrollMovedVerticallyBeyondSlop(event) &&
            richTextView.editorScrollView.let {
                it.canScrollVertically(-1) || it.canScrollVertically(1)
            }
        ) {
            atomScrollGestureIntercepted = true
            val cancel = MotionEvent.obtain(event)
            cancel.action = MotionEvent.ACTION_CANCEL
            super.dispatchTouchEvent(cancel)
            cancel.recycle()
        }
        val handled = if (atomScrollGestureIntercepted) true else super.dispatchTouchEvent(event)
        if (
            event.actionMasked == MotionEvent.ACTION_UP ||
            event.actionMasked == MotionEvent.ACTION_CANCEL
        ) {
            atomScrollGestureActive = false
            atomScrollGestureIntercepted = false
            atomScrollGestureForwarding = false
        }
        return handled || scrollHandled
    }

    private fun atomChildAt(x: Float, y: Float): View? = reactChildren.lastOrNull { child ->
        atomKey(child) != null &&
            child.visibility == View.VISIBLE &&
            x >= child.x &&
            x < child.x + child.width &&
            y >= child.y &&
            y < child.y + child.height
    }

    private fun atomScrollMovedVerticallyBeyondSlop(event: MotionEvent): Boolean {
        val dx = abs(event.x - atomScrollDownX)
        val dy = abs(event.y - atomScrollDownY)
        return dy > atomScrollTouchSlopPx && dy > dx
    }

    private fun atomScrollMovedHorizontallyBeyondSlop(event: MotionEvent): Boolean {
        val dx = abs(event.x - atomScrollDownX)
        val dy = abs(event.y - atomScrollDownY)
        return dx > atomScrollTouchSlopPx && dx >= dy
    }

    private fun cancelAtomScrollTouch(event: MotionEvent): Boolean {
        val cancel = MotionEvent.obtain(event)
        cancel.action = MotionEvent.ACTION_CANCEL
        val handled = dispatchAtomScrollTouch(cancel)
        cancel.recycle()
        return handled
    }

    private fun dispatchAtomScrollTouch(event: MotionEvent): Boolean {
        val editorLocation = IntArray(2)
        val scrollLocation = IntArray(2)
        getLocationOnScreen(editorLocation)
        richTextView.editorScrollView.getLocationOnScreen(scrollLocation)
        val scrollEvent = MotionEvent.obtain(event)
        scrollEvent.offsetLocation(
            (editorLocation[0] - scrollLocation[0]).toFloat(),
            (editorLocation[1] - scrollLocation[1]).toFloat(),
        )
        val handled = richTextView.editorScrollView.onTouchEvent(scrollEvent)
        scrollEvent.recycle()
        return handled
    }

    private fun emitAtomLayout(widthPx: Float, positions: List<AtomLayoutPosition>) {
        val density = resources.displayMetrics.density.takeIf { it > 0f } ?: 1f
        val event = mapOf<String, Any>(
            "width" to widthPx / density,
            "positions" to positions.map { position ->
                mapOf(
                    "key" to position.key,
                    "x" to position.xPx / density,
                    "y" to position.yPx / density,
                )
            },
            "editorId" to eventEditorId(richTextView.editorId)
        )
        onAtomLayoutForTesting?.invoke(event) ?: onAtomLayout(event)
    }

    private fun atomKey(child: View): String? {
        val nativeId = child.getTag(com.facebook.react.R.id.view_tag_native_id) as? String
        if (nativeId == null || !nativeId.startsWith(ATOM_NATIVE_ID_PREFIX)) return null
        return nativeId.removePrefix(ATOM_NATIVE_ID_PREFIX).takeIf(String::isNotEmpty)
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

    fun setAndroidInputOptionsJson(optionsJson: String?) {
        val options = optionsJson?.let { runCatching { JSONObject(it) }.getOrNull() }
        val privateImeOptions = options?.opt("privateImeOptions") as? String
        richTextView.editorEditText.setPrivateImeOptionsForEditor(privateImeOptions)
    }

    fun setEditable(editable: Boolean) {
        if (richTextView.editorEditText.isEditable == editable) return
        if (!editable) {
            cancelActiveExternalTextComposition("lifecycle")
            cancelPendingToolbarRefocus()
            clearPendingNativeActionRetry()
        }
        richTextView.editorEditText.isEditable = editable
        updateKeyboardToolbarVisibility()
    }

    fun beginExternalTextComposition(sessionId: String): String {
        val resultJson = richTextView.editorEditText.beginExternalTextComposition(sessionId)
        val started = runCatching {
            val result = JSONObject(resultJson)
            result.optString("type") == "active" && result.opt("sessionId") == sessionId
        }.getOrDefault(false)
        if (started) {
            activeExternalTextComposition = ActiveExternalTextComposition(
                sessionId = sessionId,
                editorId = eventEditorId(richTextView.editorId),
            )
        }
        return resultJson
    }

    fun updateExternalTextComposition(sessionId: String, text: String): String =
        richTextView.editorEditText.updateExternalTextComposition(sessionId, text)

    fun commitExternalTextComposition(sessionId: String, finalText: String): String =
        richTextView.editorEditText.commitExternalTextComposition(sessionId, finalText)

    fun cancelExternalTextComposition(sessionId: String, cause: String): String =
        richTextView.editorEditText.cancelExternalTextComposition(sessionId, cause)

    private fun cancelActiveExternalTextComposition(cause: String) {
        val composition = activeExternalTextComposition ?: return
        richTextView.editorEditText.cancelExternalTextComposition(composition.sessionId, cause)
    }

    fun setAccessibilityLabel(label: String?) {
        richTextView.editorEditText.contentDescription = label
    }

    fun setAccessibilityHint(hint: String?) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            richTextView.editorEditText.tooltipText = null
        }
        richTextView.editorEditText.setEditorAccessibilityHint(hint)
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
        lastEditorUpdateJsonProp = editorUpdateJson
        pendingEditorUpdateJson = editorUpdateJson
    }

    fun setPendingEditorUpdateEditorHandle(editorUpdateEditorHandle: String?) {
        val viewToken = editorUpdateEditorHandle?.let(EditorV2Registry::viewTokenForHandle)
        lastEditorUpdateEditorIdProp = viewToken
        pendingEditorUpdateEditorId = viewToken
    }

    /** Internal widget/test hook; production props always use decimal handles. */
    internal fun setPendingEditorUpdateEditorId(viewToken: Long?) {
        lastEditorUpdateEditorIdProp = viewToken
        pendingEditorUpdateEditorId = viewToken
    }

    fun setPendingEditorUpdateRevision(editorUpdateRevision: Long) {
        if (pendingEditorUpdateRevision != editorUpdateRevision) {
            pendingEditorUpdateRetryAttempts = 0
            pendingEditorUpdateForcedRecoveryAttempted = false
        }
        if (editorUpdateRevision != 0L && pendingEditorUpdateJson == null) {
            pendingEditorUpdateJson = lastEditorUpdateJsonProp
        }
        if (editorUpdateRevision != 0L && pendingEditorUpdateEditorId == null) {
            pendingEditorUpdateEditorId = lastEditorUpdateEditorIdProp
        }
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    fun setPendingEditorResetUpdateJson(editorResetUpdateJson: String?) {
        lastEditorResetUpdateJsonProp = editorResetUpdateJson
        pendingEditorResetUpdateJson = editorResetUpdateJson
    }

    fun setPendingEditorResetUpdateEditorHandle(editorResetUpdateEditorHandle: String?) {
        val viewToken = editorResetUpdateEditorHandle?.let(EditorV2Registry::viewTokenForHandle)
        lastEditorResetUpdateEditorIdProp = viewToken
        pendingEditorResetUpdateEditorId = viewToken
    }

    /** Internal widget/test hook; production props always use decimal handles. */
    internal fun setPendingEditorResetUpdateEditorId(viewToken: Long?) {
        lastEditorResetUpdateEditorIdProp = viewToken
        pendingEditorResetUpdateEditorId = viewToken
    }

    fun setPendingEditorResetUpdateRevision(editorResetUpdateRevision: Long) {
        if (pendingEditorResetUpdateRevision != editorResetUpdateRevision) {
            pendingEditorUpdateRetryAttempts = 0
            pendingEditorUpdateForcedRecoveryAttempted = false
        }
        if (editorResetUpdateRevision != 0L && pendingEditorResetUpdateJson == null) {
            pendingEditorResetUpdateJson = lastEditorResetUpdateJsonProp
        }
        if (editorResetUpdateRevision != 0L && pendingEditorResetUpdateEditorId == null) {
            pendingEditorResetUpdateEditorId = lastEditorResetUpdateEditorIdProp
        }
        pendingEditorResetUpdateRevision = editorResetUpdateRevision
    }

    private fun isConsumedEditorUpdateRevision(editorId: Long, revision: Long): Boolean =
        revision != 0L &&
            consumedEditorUpdateEditorId == editorId &&
            consumedEditorUpdateRevision == revision

    private fun isConsumedEditorResetUpdateRevision(editorId: Long, revision: Long): Boolean =
        revision != 0L &&
            consumedEditorResetUpdateEditorId == editorId &&
            consumedEditorResetUpdateRevision == revision

    private fun consumeEditorUpdateRevision(editorId: Long, revision: Long) {
        consumedEditorUpdateEditorId = editorId
        consumedEditorUpdateRevision = revision
    }

    private fun consumeEditorResetUpdateRevision(editorId: Long, revision: Long) {
        consumedEditorResetUpdateEditorId = editorId
        consumedEditorResetUpdateRevision = revision
    }

    private fun hasPendingEditorUpdateForEditor(editorId: Long): Boolean =
        pendingEditorUpdateJson != null &&
            pendingEditorUpdateRevision != 0L &&
            pendingEditorUpdateRevision != appliedEditorUpdateRevision &&
            !isConsumedEditorUpdateRevision(editorId, pendingEditorUpdateRevision) &&
            pendingEditorUpdateEditorId == editorId

    private fun hasPendingEditorResetUpdateForEditor(editorId: Long): Boolean =
        pendingEditorResetUpdateJson != null &&
            pendingEditorResetUpdateRevision != 0L &&
            pendingEditorResetUpdateRevision != appliedEditorResetUpdateRevision &&
            !isConsumedEditorResetUpdateRevision(editorId, pendingEditorResetUpdateRevision) &&
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
        if (pendingEditorResetUpdateRevision == 0L) return
        val revision = pendingEditorResetUpdateRevision
        val editorId = richTextView.editorId
        val expectedEditorId = pendingEditorResetUpdateEditorId
        if (expectedEditorId == null) return
        if (expectedEditorId != editorId) return
        if (isConsumedEditorResetUpdateRevision(editorId, revision)) {
            clearPendingEditorResetUpdateState(resetAppliedRevision = false)
            refreshReadyStateIfSettled()
            return
        }
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
            when (applyEditorResetUpdateOutcome(updateJson)) {
                PendingEditorUpdateApplyOutcome.APPLIED -> {
                    appliedEditorResetUpdateRevision = revision
                    clearPendingEditorResetUpdateState(resetAppliedRevision = false)
                    emitEditorReady(editorUpdateRevision = revision)
                    refreshReadyStateIfSettled()
                }
                PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED -> {
                    schedulePendingEditorUpdateRetry(PendingEditorUpdateKind.RESET)
                }
                PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED -> {
                    consumeEditorResetUpdateRevision(editorId, revision)
                    clearPendingEditorResetUpdateState(resetAppliedRevision = false)
                    refreshReadyStateIfSettled()
                }
            }
        }
        if (Looper.myLooper() == Looper.getMainLooper()) {
            apply.run()
        } else if (!post(apply)) {
            richTextView.post(apply)
        }
    }

    fun applyPendingEditorUpdateIfNeeded() {
        if (handleDestroyedCurrentEditorIfNeeded()) {
            return
        }
        if (pendingEditorUpdateRevision == 0L) {
            return
        }
        val revision = pendingEditorUpdateRevision
        val editorId = richTextView.editorId
        val expectedEditorId = pendingEditorUpdateEditorId
        if (expectedEditorId == null) {
            return
        }
        if (expectedEditorId != editorId) {
            return
        }
        if (isConsumedEditorUpdateRevision(editorId, revision)) {
            clearPendingEditorUpdateState(resetAppliedRevision = false)
            refreshReadyStateIfSettled()
            return
        }
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
        if (editorId != 0L && !isAttachedToNativeWindow) {
            return
        }
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
            val outcome = applyEditorUpdateOutcome(
                updateJson,
                scheduleViewCommandRetry = false,
            )
            when (outcome) {
                PendingEditorUpdateApplyOutcome.APPLIED -> {
                    appliedEditorUpdateRevision = revision
                    pendingEditorUpdateJson = null
                    pendingEditorUpdateEditorId = null
                    pendingEditorUpdateRevision = 0L
                    pendingEditorUpdateRetryAttempts = 0
                    pendingEditorUpdateForcedRecoveryAttempted = false
                    cancelPendingEditorUpdateRetry(PendingEditorUpdateKind.ORDINARY)
                    emitEditorReady(editorUpdateRevision = revision)
                    refreshReadyStateIfSettled()
                }
                PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED -> {
                    schedulePendingEditorUpdateRetry(PendingEditorUpdateKind.ORDINARY)
                }
                PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED -> {
                    consumeEditorUpdateRevision(editorId, revision)
                    clearPendingEditorUpdateState(resetAppliedRevision = false)
                    refreshReadyStateIfSettled()
                }
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
        pendingEditorUpdateRevision = 0L
        if (resetAppliedRevision) {
            appliedEditorUpdateRevision = 0L
        }
        cancelPendingEditorUpdateRetry(PendingEditorUpdateKind.ORDINARY)
    }

    private fun clearPendingEditorResetUpdateState(resetAppliedRevision: Boolean = true) {
        pendingEditorResetUpdateJson = null
        pendingEditorResetUpdateEditorId = null
        pendingEditorResetUpdateRevision = 0L
        if (resetAppliedRevision) {
            appliedEditorResetUpdateRevision = 0L
        }
        cancelPendingEditorUpdateRetry(PendingEditorUpdateKind.RESET)
    }

    private fun cancelPendingEditorUpdateRetry(kind: PendingEditorUpdateKind? = null) {
        if (kind != null && pendingEditorUpdateRetryKind != null && pendingEditorUpdateRetryKind != kind) {
            return
        }
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = null
        pendingEditorUpdateRetryKind = null
        pendingEditorUpdateRetryAttempts = 0
        pendingEditorUpdateForcedRecoveryAttempted = false
        pendingEditorUpdateRetryGeneration += 1
    }

    private fun schedulePendingEditorUpdateRetry(kind: PendingEditorUpdateKind) {
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
        pendingEditorUpdateRetryKind = kind
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
                when (pendingEditorUpdateRetryKind) {
                    PendingEditorUpdateKind.ORDINARY -> clearPendingEditorUpdateState()
                    PendingEditorUpdateKind.RESET -> clearPendingEditorResetUpdateState()
                    null -> Unit
                }
                return@Runnable
            }
            pendingEditorUpdateRetryScheduled = false
            pendingEditorUpdateRetryEditorId = null
            pendingEditorUpdateRetryKind = null
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
            if (
                applyEditorUpdateOutcome(retryJson, scheduleViewCommandRetry = true) !=
                    PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
            ) {
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
            if (
                applyEditorUpdateOutcome(updateJson, scheduleViewCommandRetry = true) !=
                    PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
            ) {
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

    private fun documentVersionFromUpdateJSON(updateJSON: String?): String? =
        try {
            if (updateJSON == null) null
            else canonicalV2U64(JSONObject(updateJSON).opt("documentVersion") as? String)
        } catch (_: Throwable) {
            null
        }

    private fun noteDocumentVersionFromUpdateJSON(updateJSON: String?) {
        documentVersionFromUpdateJSON(updateJSON)?.let { version ->
            lastDocumentVersion = version
        }
    }

    private fun isSupersededEditorUpdate(updateJson: String): Boolean {
        val rendered = renderedDocumentRevision?.toULongOrNull() ?: return false
        val incoming = documentVersionFromUpdateJSON(updateJson)?.toULongOrNull() ?: return false
        return incoming < rendered
    }

    private fun preflightUpdateEventFromJSON(updateJSON: String?): PreflightUpdateEvent? {
        val update = updateJSON ?: return null
        val documentRevision = documentVersionFromUpdateJSON(update) ?: return null
        return PreflightUpdateEvent(updateJSON = update, documentRevision = documentRevision)
    }

    private fun addPreflightUpdateToEvent(
        event: MutableMap<String, Any>,
        preflightUpdate: PreflightUpdateEvent?
    ) {
        preflightUpdate ?: return
        event["updateJson"] = preflightUpdate.updateJSON
        event["documentRevision"] = preflightUpdate.documentRevision
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
        cancelActiveExternalTextComposition("lifecycle")
        clearEditorErrorBinding("registryInvalidation")
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
        pendingEditorUpdateRevision = 0L
        appliedEditorUpdateRevision = 0L
        pendingEditorResetUpdateJson = null
        pendingEditorResetUpdateEditorId = null
        pendingEditorResetUpdateRevision = 0L
        appliedEditorResetUpdateRevision = 0L
        lastEditorUpdateJsonProp = null
        lastEditorUpdateEditorIdProp = null
        lastEditorResetUpdateJsonProp = null
        lastEditorResetUpdateEditorIdProp = null
        lastDocumentVersion = null
        renderedDocumentRevision = null
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
        clearEditorErrorBinding("attachRebind")
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
        bindEditorErrorCallbackIfLive(editorId)
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

    private fun emitEditorReady(editorUpdateRevision: Long? = null): Boolean {
        val editorId = richTextView.editorId
        if (editorId == 0L) return false
        if (!isAttachedToNativeWindow) return false
        if (richTextView.editorEditText.editorId != editorId) return false
        if (hasPendingEditorResetUpdateForCurrentEditor()) return false
        if (hasPendingEditorUpdateForCurrentEditor()) return false
        lastReadyEditorId = editorId
        val payload = mutableMapOf<String, Any>("editorId" to eventEditorId(editorId))
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
        richTextView.editorEditText.retireInputConnectionForHostDetach()
        super.onDetachedFromWindow()
        handleDetachedFromWindow()
    }

    private fun prepareForDetachFromWindow() {
        if (handleDestroyedCurrentEditorIfNeeded()) return
        val editorId = richTextView.editorId
        if (editorId == 0L || richTextView.editorEditText.editorId == 0L) return
        if (activeExternalTextComposition != null) {
            cancelPendingDetachPreflightRetry()
            richTextView.deferEditorUnbindOnNextDetach()
            schedulePendingDetachPreflightRetry(editorId)
            return
        }
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
            if (activeExternalTextComposition != null) {
                cancelActiveExternalTextComposition("lifecycle")
            } else {
                richTextView.editorEditText.restoreAuthorizedTextIfNeeded()
            }
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
            if (activeExternalTextComposition != null) {
                schedulePendingDetachPreflightRetry(editorId)
                return@postDelayed
            }
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
        clearEditorErrorBinding("detach")
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
        keyboardToolbarImeAnimationController.reset()
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
        publishAutoGrowStyleHeight(contentHeight)
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
            "editorId" to eventEditorId(editorId)
        )
        onContentHeightChangeForTesting?.invoke(event) ?: onContentHeightChange(event)
    }

    private fun publishAutoGrowStyleHeight(contentHeightPx: Int?) {
        val heightDp = contentHeightPx?.let { it.toDouble() / resources.displayMetrics.density }
        if (heightDp == lastPublishedAutoGrowHeightDp) return
        if (autoGrowStyleSizePublisher.publish(heightDp)) {
            lastPublishedAutoGrowHeightDp = heightDp
        }
    }

    /** Applies an editor update from JS without echoing it back through events. */
    fun applyEditorUpdate(updateJson: String): Boolean =
        applyEditorUpdateOutcome(updateJson, scheduleViewCommandRetry = true) ==
            PendingEditorUpdateApplyOutcome.APPLIED

    /** Applies a reset-style update from JS, discarding pending native composition. */
    fun applyEditorResetUpdate(updateJson: String): Boolean {
        return applyEditorResetUpdateOutcome(updateJson) == PendingEditorUpdateApplyOutcome.APPLIED
    }

    private fun applyEditorResetUpdateOutcome(updateJson: String): PendingEditorUpdateApplyOutcome {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            val postedEditorId = richTextView.editorId
            val apply = Runnable {
                if (postedEditorId != richTextView.editorId) return@Runnable
                applyEditorResetUpdateOutcome(updateJson)
            }
            if (!post(apply)) {
                richTextView.post(apply)
            }
            return PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
        }
        cancelActiveExternalTextComposition("documentChange")
        if (handleDestroyedCurrentEditorIfNeeded()) {
            return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
        val adapter = EditorV2Registry.adapterForViewToken(richTextView.editorId)
        if (adapter != null && !adapter.validateExternalRender(updateJson)) {
            return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
        if (!isEditorReadyForNativeUpdate()) {
            return PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
        }
        // The reset must be a valid external snapshot before it is allowed to
        // supersede any distinct ordinary pending update.
        clearPendingEditorUpdateState(resetAppliedRevision = false)
        clearPendingViewCommandUpdateRetry()
        val adoptedUpdateJson = if (adapter == null) updateJson else {
            adapter.adoptExternalRender(updateJson)
                ?: return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
        drainPendingEditorUpdateEvents()
        isApplyingJSUpdate = true
        val applied = try {
            richTextView.editorEditText.applyUpdateJSON(
                adoptedUpdateJson,
                refreshInputConnectionForExternalUpdate = true
            )
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
        return if (applied) {
            PendingEditorUpdateApplyOutcome.APPLIED
        } else {
            PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
    }

    private fun isEditorReadyForNativeUpdate(): Boolean {
        val editorId = richTextView.editorId
        return editorId == 0L || (isAttachedToNativeWindow && richTextView.editorEditText.editorId == editorId)
    }

    @Synchronized
    internal fun markRemoteCommitRebaseScheduled(editorId: Long): Boolean {
        if (remoteCommitRebaseScheduled && remoteCommitRebaseEditorId == editorId) return false
        remoteCommitRebaseScheduled = true
        remoteCommitRebaseEditorId = editorId
        return true
    }

    @Synchronized
    private fun clearRemoteCommitRebaseScheduled(editorId: Long) {
        if (remoteCommitRebaseEditorId != editorId) return
        remoteCommitRebaseScheduled = false
        remoteCommitRebaseEditorId = null
    }

    internal fun applyRemoteCommitRefresh(expectedEditorId: Long) {
        clearRemoteCommitRebaseScheduled(expectedEditorId)
        if (richTextView.editorId != expectedEditorId) return
        if (richTextView.editorId == 0L || !isEditorReadyForNativeUpdate()) return
        if (isApplyingJSUpdate) return
        // Preparing an external update commits a live composition. The commit
        // re-bases the adapter itself, so leave the half-typed word alone.
        if (richTextView.editorEditText.hasPendingCompositionForExternalRefresh()) return
        val adapter = EditorV2Registry.adapterForViewToken(richTextView.editorId) ?: return
        val errorBindingOwnsAdapter = editorErrorBinding?.let { binding ->
            binding.adapter === adapter &&
                binding.viewToken == expectedEditorId &&
                adapter.isNativeBindingOwner(binding.callbackToken)
        } == true
        if (!errorBindingOwnsAdapter && !richTextView.editorEditText.ownsNativeBinding(adapter)) return
        val preflight = richTextView.editorEditText.prepareForExternalEditorUpdateWithResult()
        if (!preflight.ready) return
        val update = preflight.adoptedUpdateJSON ?: adapter.refreshFromRustState(null) ?: return
        val applied = richTextView.editorEditText.applyUpdateJSON(
            update,
            refreshInputConnectionForExternalUpdate = true
        )
        if (!applied) {
            val recovery = adapter.recoverNativeRender() ?: return
            richTextView.editorEditText.applyUpdateJSON(
                recovery,
                refreshInputConnectionForExternalUpdate = true
            )
        }
    }

    private fun applyEditorUpdateOutcome(
        updateJson: String,
        scheduleViewCommandRetry: Boolean,
        expectedEditorId: Long? = null
    ): PendingEditorUpdateApplyOutcome {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            val postedEditorId = expectedEditorId ?: richTextView.editorId
            val apply = Runnable {
                if (postedEditorId != richTextView.editorId) return@Runnable
                applyEditorUpdateOutcome(updateJson, scheduleViewCommandRetry, postedEditorId)
            }
            if (!post(apply)) {
                richTextView.post(apply)
            }
            return PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
        }
        if (expectedEditorId != null && expectedEditorId != richTextView.editorId) {
            return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
        if (handleDestroyedCurrentEditorIfNeeded()) {
            return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
        val adapter = EditorV2Registry.adapterForViewToken(richTextView.editorId)
        if (adapter != null && !adapter.validateExternalRender(updateJson)) {
            return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
        }
        if (adapter != null && isSupersededEditorUpdate(updateJson)) {
            richTextView.editorEditText.recordImeTraceForTesting(
                "pendingEditorUpdateSuperseded",
                "updateRevision=${documentVersionFromUpdateJSON(updateJson)}" +
                    " rendered=$renderedDocumentRevision"
            )
            return PendingEditorUpdateApplyOutcome.APPLIED
        }
        if (!isEditorReadyForNativeUpdate()) {
            if (scheduleViewCommandRetry) {
                scheduleViewCommandUpdateRetry(updateJson)
            }
            return PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
        }
        val preflight = if (blockEditorUpdatePreflightForTesting) {
            EditorEditText.ExternalEditorUpdatePreparation(
                ready = false,
                adoptedUpdateJSON = null
            )
        } else {
            richTextView.editorEditText.prepareForExternalEditorUpdateWithResult()
        }
        if (!preflight.ready) {
            if (scheduleViewCommandRetry) {
                scheduleViewCommandUpdateRetry(updateJson)
            }
            return PendingEditorUpdateApplyOutcome.RETRYABLE_DEFERRED
        }
        // A composition preflight can commit native state. Its adapter path
        // has already rendered and adopted the post-operation snapshot, so
        // reuse that exact result rather than rendering Rust state again or
        // installing the now-stale external snapshot.
        val adoptedUpdateJson = preflight.adoptedUpdateJSON ?: if (adapter == null) {
            updateJson
        } else {
            adapter.adoptExternalRender(updateJson) ?: run {
                return PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
            }
        }
        drainPendingEditorUpdateEvents()
        isApplyingJSUpdate = true
        return try {
            richTextView.editorEditText.applyUpdateJSON(
                adoptedUpdateJson,
                refreshInputConnectionForExternalUpdate = true
            )
            PendingEditorUpdateApplyOutcome.APPLIED
        } catch (error: Throwable) {
            Log.w(LOG_TAG, "Failed to apply JS editor update", error)
            PendingEditorUpdateApplyOutcome.PERMANENTLY_REJECTED
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
            pendingEditorUpdateKeys.remove(NativeCommitKey(event.editorId, event.documentRevision))
            if (event.editorId != eventEditorId(richTextView.editorId)) {
                richTextView.editorEditText.recordImeTraceForTesting(
                    "nativeViewEditorUpdateSkipped",
                    "reason=staleEditor queuedEditor=${event.editorId} currentEditor=${eventEditorId(richTextView.editorId)}"
                )
                continue
            }
            val isCurrentRevision = event.documentRevision == renderedDocumentRevision
            dispatchEditorUpdate(event, emitToJS = true, applyViewState = isCurrentRevision)
            drainedCount += 1
        }
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateDrained",
            "count=$drainedCount totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    private fun dispatchEditorUpdate(
        event: PendingEditorUpdateEvent,
        emitToJS: Boolean,
        applyViewState: Boolean = true,
    ) {
        val updateJSON = event.viewUpdateJSON
        val startedAt = System.nanoTime()
        if (applyViewState) noteDocumentVersionFromUpdateJSON(updateJSON)
        val noteNanos = System.nanoTime() - startedAt
        val toolbarStartedAt = System.nanoTime()
        if (applyViewState) {
            NativeToolbarState.fromUpdateJson(updateJSON)?.let { state ->
                toolbarState = state
                keyboardToolbarView.applyState(state)
            }
        }
        val toolbarNanos = System.nanoTime() - toolbarStartedAt
        val mentionStartedAt = System.nanoTime()
        if (applyViewState) refreshMentionQuery()
        val mentionNanos = System.nanoTime() - mentionStartedAt
        val retryStartedAt = System.nanoTime()
        if (applyViewState) {
            clearPendingNativeActionRetryIfScopeChanged()
            schedulePendingPreflightWake()
            richTextView.refreshRemoteSelections()
        }
        val retryNanos = System.nanoTime() - retryStartedAt
        if (applyViewState && heightBehavior == EditorHeightBehavior.AUTO_GROW) {
            post {
                requestLayout()
                emitContentHeightIfNeeded(force = false)
            }
        }
        val emitStartedAt = System.nanoTime()
        if (emitToJS) {
            val payload = mapOf<String, Any>(
                "updateJson" to event.atomicUpdateJSON,
                "editorId" to event.editorId,
                "documentRevision" to event.documentRevision,
            )
            onEditorUpdateForTesting?.invoke(payload) ?: onEditorUpdate(payload)
        }
        val totalNanos = System.nanoTime() - startedAt
        richTextView.editorEditText.recordImeTraceForTesting(
            "nativeViewEditorUpdateDispatch",
            "emitToJS=$emitToJS jsonLength=${updateJSON.length} noteUs=${nanosToMicros(noteNanos)} toolbarUs=${nanosToMicros(toolbarNanos)} mentionUs=${nanosToMicros(mentionNanos)} retryUs=${nanosToMicros(retryNanos)} emitUs=${nanosToMicros(System.nanoTime() - emitStartedAt)} totalUs=${nanosToMicros(totalNanos)}"
        )
    }

    private fun bindEditorErrorCallbackIfLive(viewToken: Long) {
        if (!isAttachedToNativeWindow || richTextView.editorId != viewToken ||
            richTextView.editorEditText.editorId != viewToken
        ) return
        val adapter = EditorV2Registry.adapterForViewToken(viewToken) ?: return
        val editorId = publicHandleForViewToken(viewToken) ?: return
        val existing = editorErrorBinding
        if (existing?.adapter === adapter && existing.viewToken == viewToken &&
            existing.editorId == editorId
        ) return
        clearEditorErrorBinding("claim")
        val binding = EditorErrorBinding(
            adapter = adapter,
            editorId = editorId,
            viewToken = viewToken,
            callbackToken = nextNativeEditorErrorCallbackToken.incrementAndGet(),
            generation = ++nextEditorErrorBindingGeneration,
        )
        editorErrorBinding = binding
        adapter.bindAutonomousErrorOwner(
            binding.callbackToken,
            callback = { error -> queueEditorError(binding, error) },
            onReleased = { releaseEditorErrorBinding(binding) },
        )
    }

    private fun releaseEditorErrorBinding(binding: EditorErrorBinding) {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            mainHandler.post { releaseEditorErrorBinding(binding) }
            return
        }
        if (editorErrorBinding != binding) return
        editorErrorBinding = null
        nextEditorErrorBindingGeneration += 1
        clearPendingEditorErrorDispatchQueue("ownerReleased")
    }

    private fun clearEditorErrorBinding(reason: String) {
        val binding = editorErrorBinding
        editorErrorBinding = null
        nextEditorErrorBindingGeneration += 1
        binding?.adapter?.clearAutonomousErrorOwner(binding.callbackToken)
        clearPendingEditorErrorDispatchQueue(reason)
    }

    private fun queueEditorError(binding: EditorErrorBinding, error: EditorV2Error) {
        val event = PendingEditorErrorEvent(
            adapter = binding.adapter,
            editorId = binding.editorId,
            viewToken = binding.viewToken,
            callbackToken = binding.callbackToken,
            bindingGeneration = binding.generation,
            error = error,
        )
        if (Looper.myLooper() == Looper.getMainLooper()) {
            enqueueEditorError(event)
        } else {
            mainHandler.post { enqueueEditorError(event) }
        }
    }

    private fun enqueueEditorError(event: PendingEditorErrorEvent) {
        if (!isLiveEditorErrorBinding(event)) return
        pendingEditorErrorEvents.addLast(event)
        if (!pendingEditorErrorDispatchScheduled) {
            pendingEditorErrorDispatchScheduled = true
            val generation = ++pendingEditorErrorDispatchGeneration
            mainHandler.post {
                if (generation != pendingEditorErrorDispatchGeneration) return@post
                pendingEditorErrorDispatchScheduled = false
                drainPendingEditorErrorEvents()
            }
        }
    }

    private fun drainPendingEditorErrorEvents() {
        while (pendingEditorErrorEvents.isNotEmpty()) {
            // Remove before dispatch so a reentrant lifecycle transition cannot deliver it twice.
            val event = pendingEditorErrorEvents.removeFirst()
            if (!isLiveEditorErrorBinding(event)) continue
            val payload = mapOf<String, Any>(
                "editorId" to event.editorId,
                "error" to event.error.toJSMap(),
            )
            dispatchEditorError(payload)
        }
    }

    private fun dispatchEditorError(payload: Map<String, Any>) {
        onEditorErrorForTesting?.let { callback ->
            callback(payload)
            return
        }
        if (!appContext.hasActiveReactInstance) return
        onEditorError(payload)
    }

    private fun isLiveEditorErrorBinding(event: PendingEditorErrorEvent): Boolean {
        val binding = editorErrorBinding ?: return false
        return isAttachedToNativeWindow &&
            !NativeEditorViewRegistry.isDestroyed(event.viewToken) &&
            binding.adapter === event.adapter &&
            binding.editorId == event.editorId &&
            binding.viewToken == event.viewToken &&
            binding.callbackToken == event.callbackToken &&
            binding.generation == event.bindingGeneration &&
            richTextView.editorId == event.viewToken &&
            richTextView.editorEditText.editorId == event.viewToken &&
            publicHandleForViewToken(event.viewToken) == event.editorId &&
            EditorV2Registry.adapterForViewToken(event.viewToken) === event.adapter &&
            event.adapter.ownsAutonomousErrorOwner(event.callbackToken)
    }

    private fun clearPendingEditorErrorDispatchQueue(reason: String) {
        val clearedCount = pendingEditorErrorEvents.size
        pendingEditorErrorEvents.clear()
        pendingEditorErrorDispatchScheduled = false
        pendingEditorErrorDispatchGeneration += 1
        if (clearedCount > 0) {
            richTextView.editorEditText.recordImeTraceForTesting(
                "nativeViewEditorErrorQueueCleared",
                "reason=$reason count=$clearedCount"
            )
        }
    }

    internal fun pendingEditorErrorEventCountForTesting(): Int = pendingEditorErrorEvents.size

    internal fun editorErrorCallbackTokenForTesting(): Long? = editorErrorBinding?.callbackToken

    private fun installOutsideTapBlurHandlerIfNeeded() {
        val window = resolveActivity(context)?.window ?: return
        if (outsideTapWindow !== window) {
            uninstallOutsideTapBlurHandler()
        }
        NativeEditorOutsideTapDispatcher.register(window, this)
        outsideTapWindow = window
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
        richTextView.editorEditText.retireInputConnectionForHostDetach()
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

    internal fun setOutsideTapCycleBreakDispatcherForTesting(
        dispatcher: ((MotionEvent) -> Boolean)?
    ): Boolean {
        val window = resolveActivity(context)?.window ?: return false
        return NativeEditorOutsideTapDispatcher.setCycleBreakDispatcherForTesting(window, dispatcher)
    }

    internal fun clearOutsideTapRouteViewReferenceAndReconcileForTesting():
        NativeEditorOutsideTapRouteTestState {
        val window = resolveActivity(context)?.window
            ?: return NativeEditorOutsideTapRouteTestState(
                isRegistered = false,
                hasCallbackReconciler = false
            )
        return NativeEditorOutsideTapDispatcher.clearViewReferenceAndReconcileForTesting(window, this)
    }

    internal fun dispatchOutsideTapWindowEventForTesting(event: MotionEvent): Boolean {
        val window = resolveActivity(context)?.window ?: return false
        return NativeEditorOutsideTapDispatcher.dispatchForTesting(window, event)
    }

    internal fun schedulePendingPreflightWakeForTesting() {
        schedulePendingPreflightWake()
    }

    internal fun hasPendingNativeActionForTesting(): Boolean = pendingNativeAction != null

    internal fun pendingNativeActionRetryAttemptsForTesting(): Int = pendingNativeActionRetryAttempts

    internal fun lastDocumentVersionForTesting(): String? = lastDocumentVersion

    internal fun setLastDocumentVersionForTesting(documentVersion: String?) {
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

    internal fun emitEditorReadyForTesting(editorUpdateRevision: Long? = null): Boolean =
        emitEditorReady(editorUpdateRevision)

    internal fun pendingEditorUpdateJsonForTesting(): String? = pendingEditorUpdateJson

    internal fun pendingEditorUpdateRevisionForTesting(): Long = pendingEditorUpdateRevision

    internal fun pendingEditorResetUpdateJsonForTesting(): String? = pendingEditorResetUpdateJson

    internal fun pendingEditorResetUpdateRevisionForTesting(): Long =
        pendingEditorResetUpdateRevision

    internal fun setAppliedEditorUpdateRevisionForTesting(editorUpdateRevision: Long) {
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

    private fun isTouchInsideStandaloneToolbar(event: MotionEvent): Boolean =
        isPointInsideStandaloneToolbar(event.rawX, event.rawY, windowOriginOnScreen())

    private fun windowOriginOnScreen(): Point {
        val onScreen = IntArray(2)
        val inWindow = IntArray(2)
        getLocationOnScreen(onScreen)
        getLocationInWindow(inWindow)
        return Point(onScreen[0] - inWindow[0], onScreen[1] - inWindow[1])
    }

    internal fun isPointInsideStandaloneToolbarForTesting(
        rawX: Float,
        rawY: Float,
        windowOriginOnScreen: Point
    ): Boolean = isPointInsideStandaloneToolbar(rawX, rawY, windowOriginOnScreen)

    private fun isPointInsideStandaloneToolbar(
        rawX: Float,
        rawY: Float,
        windowOriginOnScreen: Point
    ): Boolean {
        if (toolbarFramesInWindow.isEmpty()) {
            return false
        }
        // toolbarFrame is in DP from Fabric's measureInWindow, which offsets by
        // the surface's getLocationInWindow. rawX/rawY are screen pixels, so
        // normalize them into the same window space rather than the visible
        // display frame, whose top also excludes the status bar and cutout.
        val density = resources.displayMetrics.density
        val hitSlopPx = TOOLBAR_HIT_SLOP_DP * density
        val eventX = rawX - windowOriginOnScreen.x
        val eventY = rawY - windowOriginOnScreen.y
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
        emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to eventEditorId(editorId)))
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
        emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to eventEditorId(richTextView.editorId)))
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
        emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to eventEditorId(richTextView.editorId)))
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

        val updateJson = richTextView.editorEditText.v2Driver?.insertContentJsonAtSelection(
            docJson.toString(),
            queryState.anchor,
            queryState.head
        )
        if (updateJson != null) {
            richTextView.editorEditText.applyUpdateJSON(updateJson)
        }
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
        val stateJson = richTextView.editorEditText.v2Driver?.currentStateJson() ?: return null
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
        keyboardToolbarImeAnimationController.cancel()
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
            showsToolbar &&
                toolbarPlacement == ToolbarPlacement.KEYBOARD &&
                richTextView.editorEditText.isEditable &&
                richTextView.editorEditText.hasFocus() &&
                currentImeBottom > 0

        if (!shouldReserveToolbarSpace) {
            richTextView.setViewportBottomOcclusionTopOnScreenPx(null)
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
        val keyboardOffsetPx = ((toolbarTheme?.resolvedKeyboardOffset() ?: 0f) * density).toInt()
        val toolbarTopOnScreenPx = resolveToolbarTopOnScreenPx(
            toolbarHeight = toolbarHeight,
            keyboardOffsetPx = keyboardOffsetPx
        )
        richTextView.setViewportBottomOcclusionTopOnScreenPx(toolbarTopOnScreenPx)
        richTextView.setViewportBottomInsetPx(
            resolveToolbarViewportInsetPx(
                toolbarHeight = toolbarHeight,
                keyboardOffsetPx = keyboardOffsetPx,
                toolbarTopOnScreenPx = toolbarTopOnScreenPx
            )
        )
    }

    private fun resolveToolbarTopOnScreenPx(
        toolbarHeight: Int,
        keyboardOffsetPx: Int
    ): Int? {
        val host = resolveActivity(context)?.findViewById<ViewGroup>(android.R.id.content)
            ?: return null
        if (host.height <= 0) return null
        val hostLocation = IntArray(2)
        host.getLocationOnScreen(hostLocation)
        return hostLocation[1] + host.height - currentImeBottom - keyboardOffsetPx - toolbarHeight
    }

    private fun resolveToolbarViewportInsetPx(
        toolbarHeight: Int,
        keyboardOffsetPx: Int,
        toolbarTopOnScreenPx: Int?
    ): Int {
        val fallbackInset = (toolbarHeight + keyboardOffsetPx).coerceAtLeast(0)
        val toolbarTop = toolbarTopOnScreenPx ?: return fallbackInset
        var foundScrollViewport = false
        var viewportInset = 0

        fun includeScrollViewport(view: View) {
            if (view.height <= 0) return
            val location = IntArray(2)
            view.getLocationOnScreen(location)
            foundScrollViewport = true
            viewportInset = maxOf(viewportInset, location[1] + view.height - toolbarTop)
        }

        if (heightBehavior == EditorHeightBehavior.FIXED) {
            includeScrollViewport(richTextView.editorScrollView)
        } else {
            var ancestor = parent
            while (ancestor is View) {
                if (ancestor is ScrollView || ancestor is NestedScrollView) {
                    includeScrollViewport(ancestor)
                }
                ancestor = (ancestor as View).parent
            }
        }

        return if (foundScrollViewport) viewportInset.coerceAtLeast(0) else fallbackInset
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
        var preflightUpdate: PreflightUpdateEvent? = null
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
            preflightUpdate = preflightUpdateEventFromJSON(preparation.updateJSON)
            preflightUpdate?.let { lastDocumentVersion = it.documentRevision }
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
                    "editorId" to eventEditorId(richTextView.editorId)
                )
                addPreflightUpdateToEvent(payload, preflightUpdate)
                onToolbarActionForTesting?.invoke(payload) ?: onToolbarAction(payload)
            }
            ToolbarItemKind.group -> Unit
            ToolbarItemKind.separator -> Unit
        }
    }

}
