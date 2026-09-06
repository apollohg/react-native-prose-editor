package com.apollohg.editor

import android.view.MotionEvent

internal fun NativeEditorExpoView.markRecentToolbarTouchForTestingImpl() {
    markRecentToolbarTouch()
}

internal fun NativeEditorExpoView.shouldPreserveFocusAfterToolbarTouchForTestingImpl(): Boolean =
    shouldPreserveFocusAfterToolbarTouch()

internal fun NativeEditorExpoView.setEditorFocusedForOutsideTapDecisionForTestingImpl(isFocused: Boolean?) {
    editorFocusedForOutsideTapOverrideForTesting = isFocused
}

internal fun NativeEditorExpoView.setAttachedToNativeWindowForTestingImpl(isAttached: Boolean) {
    isAttachedToNativeWindow = isAttached
}

internal fun NativeEditorExpoView.handleAttachedToWindowForTestingImpl() {
    handleAttachedToWindow()
}

internal fun NativeEditorExpoView.traceOutsideTapImpl(message: String) {
    onOutsideTapTraceForTesting?.invoke(message)
}

internal fun NativeEditorExpoView.handleDetachedFromWindowForTestingImpl() {
    prepareForDetachFromWindow()
    richTextView.editorEditText.retireInputConnectionForHostDetach()
    handleDetachedFromWindow()
}

internal fun NativeEditorExpoView.performBlurForTestingImpl(deferKeyboardDismiss: Boolean = false) {
    performBlur(deferKeyboardDismiss = deferKeyboardDismiss, allowRetry = true)
}

internal fun NativeEditorExpoView.pendingBlurRetryAttemptsForTestingImpl(): Int = pendingBlurRetryAttempts

internal fun NativeEditorExpoView.pendingDetachPreflightRetryAttemptsForTestingImpl(): Int =
    pendingDetachPreflightRetryAttempts

internal fun NativeEditorExpoView.hasPendingOutsideTapBlurForTestingImpl(): Boolean = pendingOutsideTapBlur != null

internal fun NativeEditorExpoView.isOutsideTapBlurHandlerInstalledForTestingImpl(): Boolean = outsideTapWindow != null

internal fun NativeEditorExpoView.hasPendingKeyboardDismissForTestingImpl(): Boolean = pendingKeyboardDismiss != null

internal fun NativeEditorExpoView.hasPendingPreflightWakeForTestingImpl(): Boolean = pendingPreflightWakeScheduled

internal fun NativeEditorExpoView.hasPendingToolbarRefocusForTestingImpl(): Boolean = pendingToolbarRefocus != null

internal fun NativeEditorExpoView.isKeyboardToolbarAttachedForTestingImpl(): Boolean = keyboardToolbarView.parent != null

internal fun NativeEditorExpoView.currentImeBottomForTestingImpl(): Int = currentImeBottom

internal fun NativeEditorExpoView.setCurrentImeBottomForTestingImpl(bottom: Int) {
    currentImeBottom = bottom
}

internal fun NativeEditorExpoView.updateAttachedKeyboardToolbarForInsetsForTestingImpl() {
    updateAttachedKeyboardToolbarForInsets()
}

internal fun NativeEditorExpoView.scheduleToolbarRefocusForTestingImpl() {
    scheduleToolbarRefocus()
}

internal fun NativeEditorExpoView.focusFromToolbarPreserveForTestingImpl() {
    focusInternal(cancelPendingOutsideTapBlur = false)
}

internal fun NativeEditorExpoView.applyAutoFocusForTestingImpl() {
    applyAutoFocusIfNeeded()
}

internal fun NativeEditorExpoView.installOutsideTapBlurHandlerForTestingImpl() {
    installOutsideTapBlurHandlerIfNeeded()
}

internal fun NativeEditorExpoView.uninstallOutsideTapBlurHandlerForTestingImpl() {
    uninstallOutsideTapBlurHandler()
}

internal fun NativeEditorExpoView.setOutsideTapCycleBreakDispatcherForTestingImpl(
    dispatcher: ((MotionEvent) -> Boolean)?
): Boolean {
    val window = resolveActivity(context)?.window ?: return false
    return NativeEditorOutsideTapDispatcher.setCycleBreakDispatcherForTesting(window, dispatcher)
}

internal fun NativeEditorExpoView.clearOutsideTapRouteViewReferenceAndReconcileForTestingImpl():
    NativeEditorOutsideTapRouteTestState {
    val window = resolveActivity(context)?.window
        ?: return NativeEditorOutsideTapRouteTestState(
            isRegistered = false,
            hasCallbackReconciler = false
        )
    return NativeEditorOutsideTapDispatcher.clearViewReferenceAndReconcileForTesting(window, this)
}

internal fun NativeEditorExpoView.dispatchOutsideTapWindowEventForTestingImpl(event: MotionEvent): Boolean {
    val window = resolveActivity(context)?.window ?: return false
    return NativeEditorOutsideTapDispatcher.dispatchForTesting(window, event)
}

internal fun NativeEditorExpoView.schedulePendingPreflightWakeForTestingImpl() {
    schedulePendingPreflightWake()
}

internal fun NativeEditorExpoView.hasPendingNativeActionForTestingImpl(): Boolean = pendingNativeAction != null

internal fun NativeEditorExpoView.pendingNativeActionRetryAttemptsForTestingImpl(): Int = pendingNativeActionRetryAttempts

internal fun NativeEditorExpoView.lastDocumentVersionForTestingImpl(): String? = lastDocumentVersion

internal fun NativeEditorExpoView.setLastDocumentVersionForTestingImpl(documentVersion: String?) {
    lastDocumentVersion = documentVersion
}

internal fun NativeEditorExpoView.refreshToolbarStateFromEditorSelectionForTestingImpl(): String? =
    refreshToolbarStateFromEditorSelection()

internal fun NativeEditorExpoView.handleToolbarItemPressForTestingImpl(item: NativeToolbarItem) {
    handleToolbarItemPress(item)
}

internal fun NativeEditorExpoView.insertMentionSuggestionForTestingImpl(suggestion: NativeMentionSuggestion) {
    insertMentionSuggestion(suggestion)
}

internal fun NativeEditorExpoView.wakePendingPreflightWorkForTestingImpl() {
    wakePendingPreflightWork()
}

internal fun NativeEditorExpoView.emitEditorReadyForTestingImpl(editorUpdateRevision: Long? = null): Boolean =
    emitEditorReady(editorUpdateRevision)

internal fun NativeEditorExpoView.pendingEditorUpdateJsonForTestingImpl(): String? = pendingEditorUpdateJson

internal fun NativeEditorExpoView.pendingEditorUpdateRevisionForTestingImpl(): Long = pendingEditorUpdateRevision

internal fun NativeEditorExpoView.pendingEditorResetUpdateJsonForTestingImpl(): String? = pendingEditorResetUpdateJson

internal fun NativeEditorExpoView.pendingEditorResetUpdateRevisionForTestingImpl(): Long =
    pendingEditorResetUpdateRevision

internal fun NativeEditorExpoView.setAppliedEditorUpdateRevisionForTestingImpl(editorUpdateRevision: Long) {
    appliedEditorUpdateRevision = editorUpdateRevision
}

internal fun NativeEditorExpoView.pendingEditorUpdateEditorIdForTestingImpl(): Long? = pendingEditorUpdateEditorId

internal fun NativeEditorExpoView.pendingEditorResetUpdateEditorIdForTestingImpl(): Long? =
    pendingEditorResetUpdateEditorId

internal fun NativeEditorExpoView.pendingViewCommandUpdateJsonForTestingImpl(): String? = pendingViewCommandUpdateJson

internal fun NativeEditorExpoView.pendingViewCommandUpdateRetryAttemptsForTestingImpl(): Int =
    pendingViewCommandUpdateRetryAttempts

internal fun NativeEditorExpoView.scheduleViewCommandUpdateRetryForTestingImpl(updateJson: String) {
    scheduleViewCommandUpdateRetry(updateJson)
}

internal fun NativeEditorExpoView.pendingThemeJsonForTestingImpl(): String? = pendingThemeJson.takeIf { hasPendingTheme }

internal fun NativeEditorExpoView.pendingAtomsJsonForTestingImpl(): String? = pendingAtomsJson.takeIf { hasPendingAtoms }

internal fun NativeEditorExpoView.lastAtomsJsonForTestingImpl(): String? = lastAtomsJson

internal fun NativeEditorExpoView.lastThemeJsonForTestingImpl(): String? = lastThemeJson

internal fun NativeEditorExpoView.pendingThemeRetryAttemptsForTestingImpl(): Int = pendingThemeRetry.attempts

internal fun NativeEditorExpoView.applyPendingThemeForTestingImpl() {
    applyPendingThemeIfNeeded()
}
