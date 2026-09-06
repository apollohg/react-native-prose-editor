package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.Companion.MAX_PENDING_UPDATE_RETRY_ATTEMPTS
import com.apollohg.editor.NativeEditorExpoView.Companion.NATIVE_ACTION_RETRY_DELAY_MS
import android.content.Context
import android.view.View
import android.view.inputmethod.InputMethodManager
import org.json.JSONObject

internal fun NativeEditorExpoView.getCaretRectJsonImpl(): String? {
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

internal fun NativeEditorExpoView.handleEditorDestroyedImpl(editorId: Long) {
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

internal fun NativeEditorExpoView.handleDestroyedCurrentEditorIfNeeded(): Boolean {
    val editorId = richTextView.editorId.takeIf { it != 0L }
        ?: richTextView.editorEditText.editorId.takeIf { it != 0L }
        ?: return false
    if (!NativeEditorViewRegistry.isDestroyed(editorId)) return false
    handleEditorDestroyed(editorId)
    return true
}

internal fun NativeEditorExpoView.handleAttachedToWindow() {
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
        pendingThemeRetry.bind(editorId)
    }
    if (hasPendingAtoms) {
        pendingAtomsRetry.bind(editorId)
    }
    applyPendingEditorResetUpdateIfNeeded()
    applyPendingEditorUpdateIfNeeded()
    applyPendingThemeIfNeeded()
    applyPendingAtomsIfNeeded()
    refreshReadyStateIfSettled()
    applyAutoFocusIfNeeded()
}

internal fun NativeEditorExpoView.emitEditorReady(editorUpdateRevision: Long? = null): Boolean {
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

internal fun NativeEditorExpoView.emitEditorReadyIfNeeded() {
    val editorId = richTextView.editorId
    if (lastReadyEditorId == editorId) return
    emitEditorReady()
}

internal fun NativeEditorExpoView.prepareForDetachFromWindow() {
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

internal fun NativeEditorExpoView.schedulePendingDetachPreflightRetry(editorId: Long) {
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

internal fun NativeEditorExpoView.cancelPendingDetachPreflightRetry() {
    pendingDetachPreflightRetryScheduled = false
    pendingDetachPreflightRetryEditorId = null
    pendingDetachPreflightRetryAttempts = 0
    pendingDetachPreflightRetryGeneration += 1
}

internal fun NativeEditorExpoView.handleDetachedFromWindow() {
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

internal fun NativeEditorExpoView.emitContentHeightIfNeeded(force: Boolean) {
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

internal fun NativeEditorExpoView.publishAutoGrowStyleHeight(contentHeightPx: Int?) {
    val heightDp = contentHeightPx?.let { it.toDouble() / resources.displayMetrics.density }
    if (heightDp == lastPublishedAutoGrowHeightDp) return
    if (autoGrowStyleSizePublisher.publish(heightDp)) {
        lastPublishedAutoGrowHeightDp = heightDp
    }
}
