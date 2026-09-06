package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.Companion.OUTSIDE_TAP_HANDLER_INSTALL_RETRY_DELAY_MS
import com.apollohg.editor.NativeEditorExpoView.Companion.TOOLBAR_FOCUS_PRESERVE_MS
import com.apollohg.editor.NativeEditorExpoView.Companion.TOOLBAR_HIT_SLOP_DP
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.graphics.Point
import android.graphics.Rect
import android.graphics.RectF
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View

internal fun NativeEditorExpoView.installOutsideTapBlurHandlerIfNeeded() {
    val window = resolveActivity(context)?.window ?: return
    if (outsideTapWindow !== window) {
        uninstallOutsideTapBlurHandler()
    }
    NativeEditorOutsideTapDispatcher.register(window, this)
    outsideTapWindow = window
}

internal fun NativeEditorExpoView.scheduleOutsideTapBlurHandlerInstallRetry() {
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

internal fun NativeEditorExpoView.cancelPendingOutsideTapBlurHandlerInstallRetry() {
    pendingOutsideTapHandlerInstallRetry?.let {
        richTextView.editorEditText.removeCallbacks(it)
        pendingOutsideTapHandlerInstallRetry = null
    }
}

internal fun NativeEditorExpoView.uninstallOutsideTapBlurHandler() {
    cancelPendingOutsideTapBlurHandlerInstallRetry()
    val window = outsideTapWindow ?: return
    NativeEditorOutsideTapDispatcher.unregister(window, this)
    outsideTapWindow = null
}

internal fun NativeEditorExpoView.prepareOutsideTapDecisionForWindowEventImpl(event: MotionEvent): NativeEditorOutsideTapDecision {
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

internal fun NativeEditorExpoView.handleOutsideTapDecisionFromWindowDispatcherImpl(decision: NativeEditorOutsideTapDecision) {
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

internal fun NativeEditorExpoView.scheduleOutsideTapBlurFromWindowDispatcherImpl() {
    scheduleOutsideTapBlur()
}

internal fun NativeEditorExpoView.cancelOutsideTapBlurFromWindowDispatcherImpl() {
    cancelPendingOutsideTapBlur()
}

internal fun NativeEditorExpoView.isEditorFocusedForOutsideTapDecision(): Boolean =
    editorFocusedForOutsideTapOverrideForTesting ?: richTextView.editorEditText.hasFocus()

internal fun NativeEditorExpoView.isTouchOutsideEditor(event: MotionEvent): Boolean {
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

internal fun NativeEditorExpoView.markRecentToolbarTouch() {
    lastToolbarTouchUptimeMs = SystemClock.uptimeMillis()
}

internal fun NativeEditorExpoView.clearRecentToolbarTouch() {
    lastToolbarTouchUptimeMs = null
}

internal fun NativeEditorExpoView.shouldPreserveFocusAfterToolbarTouch(): Boolean {
    val lastToolbarTouch = lastToolbarTouchUptimeMs ?: return false
    val elapsedMs = SystemClock.uptimeMillis() - lastToolbarTouch
    return elapsedMs in 0L..TOOLBAR_FOCUS_PRESERVE_MS
}

internal fun NativeEditorExpoView.consumeToolbarFocusPreservationForBlur(): Boolean {
    if (!shouldPreserveFocusAfterToolbarTouch()) {
        return false
    }
    clearRecentToolbarTouch()
    return true
}

internal fun NativeEditorExpoView.isTouchInsideStandaloneToolbar(event: MotionEvent): Boolean =
    isPointInsideStandaloneToolbar(event.rawX, event.rawY, windowOriginOnScreen())

internal fun NativeEditorExpoView.windowOriginOnScreen(): Point {
    val onScreen = IntArray(2)
    val inWindow = IntArray(2)
    getLocationOnScreen(onScreen)
    getLocationInWindow(inWindow)
    return Point(onScreen[0] - inWindow[0], onScreen[1] - inWindow[1])
}

internal fun NativeEditorExpoView.isPointInsideStandaloneToolbarForTestingImpl(
    rawX: Float,
    rawY: Float,
    windowOriginOnScreen: Point
): Boolean = isPointInsideStandaloneToolbar(rawX, rawY, windowOriginOnScreen)

internal fun NativeEditorExpoView.isPointInsideStandaloneToolbar(
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

internal fun NativeEditorExpoView.isTouchInsideKeyboardToolbar(event: MotionEvent): Boolean {
    if (keyboardToolbarView.parent == null || keyboardToolbarView.visibility != View.VISIBLE) {
        return false
    }
    val rect = Rect()
    keyboardToolbarView.getGlobalVisibleRect(rect)
    return rect.contains(event.rawX.toInt(), event.rawY.toInt())
}

internal fun NativeEditorExpoView.resolveActivity(context: Context): Activity? {
    appContext.currentActivity?.let { return it }
    var current: Context? = context
    while (current is ContextWrapper) {
        if (current is Activity) return current
        current = current.baseContext
    }
    return null
}
