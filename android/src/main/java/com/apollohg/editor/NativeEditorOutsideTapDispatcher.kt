package com.apollohg.editor

import android.graphics.Rect
import android.view.MotionEvent
import android.view.ViewConfiguration
import android.view.ViewTreeObserver
import android.view.Window
import java.lang.ref.WeakReference
import java.util.WeakHashMap

internal object NativeEditorOutsideTapDispatcher {
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
