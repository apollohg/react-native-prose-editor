package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.Companion.MAX_PENDING_UPDATE_RETRY_ATTEMPTS
import com.apollohg.editor.NativeEditorExpoView.Companion.NATIVE_ACTION_RETRY_DELAY_MS
import com.apollohg.editor.NativeEditorExpoView.Companion.OUTSIDE_TAP_BLUR_DELAY_MS
import android.content.Context
import android.view.inputmethod.InputMethodManager

internal fun NativeEditorExpoView.canFocusCurrentEditor(): Boolean {
    val editorId = richTextView.editorId
    return editorId != 0L &&
        isAttachedToNativeWindow &&
        !NativeEditorViewRegistry.isDestroyed(editorId)
}

internal fun NativeEditorExpoView.focusImpl() {
    focusInternal(cancelPendingOutsideTapBlur = true)
}

internal fun NativeEditorExpoView.focusInternal(cancelPendingOutsideTapBlur: Boolean) {
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

internal fun NativeEditorExpoView.blurImpl() {
    cancelPendingOutsideTapBlur()
    cancelPendingKeyboardDismiss()
    cancelPendingToolbarRefocus()
    clearRecentToolbarTouch()
    performBlur(deferKeyboardDismiss = false, allowRetry = true)
}

internal fun NativeEditorExpoView.performBlur(deferKeyboardDismiss: Boolean, allowRetry: Boolean) {
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

internal fun NativeEditorExpoView.completeBlur(deferKeyboardDismiss: Boolean) {
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

internal fun NativeEditorExpoView.schedulePendingBlurRetry(deferKeyboardDismiss: Boolean) {
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

internal fun NativeEditorExpoView.blurWithDeferredKeyboardDismiss() {
    cancelPendingKeyboardDismiss()
    cancelPendingToolbarRefocus()
    clearRecentToolbarTouch()
    performBlur(deferKeyboardDismiss = true, allowRetry = true)
}

internal fun NativeEditorExpoView.scheduleToolbarRefocus() {
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

internal fun NativeEditorExpoView.cancelPendingToolbarRefocus() {
    pendingToolbarRefocus?.let {
        richTextView.editorEditText.removeCallbacks(it)
        pendingToolbarRefocus = null
    }
    pendingToolbarRefocusEditorId = null
    pendingToolbarRefocusGeneration += 1
}

internal fun NativeEditorExpoView.scheduleOutsideTapBlur() {
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

internal fun NativeEditorExpoView.cancelPendingOutsideTapBlur() {
    pendingOutsideTapBlur?.let {
        traceOutsideTap("cancel outside blur")
        richTextView.editorEditText.removeCallbacks(it)
        pendingOutsideTapBlur = null
    }
}

internal fun NativeEditorExpoView.cancelPendingKeyboardDismiss() {
    pendingKeyboardDismiss?.let {
        richTextView.editorEditText.removeCallbacks(it)
        pendingKeyboardDismiss = null
    }
}

internal fun NativeEditorExpoView.cancelPendingBlurRetry() {
    pendingBlurRetry?.let {
        mainHandler.removeCallbacks(it)
        pendingBlurRetry = null
    }
    pendingBlurRetryEditorId = null
    pendingBlurRetryAttempts = 0
    pendingBlurRetryGeneration += 1
}
