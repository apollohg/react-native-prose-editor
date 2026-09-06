package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.PendingNativeActionScope
import com.apollohg.editor.NativeEditorExpoView.PendingNativeAction
import com.apollohg.editor.NativeEditorExpoView.ToolbarPlacement
import com.apollohg.editor.NativeEditorExpoView.Companion.MAX_NATIVE_ACTION_RETRY_ATTEMPTS
import com.apollohg.editor.NativeEditorExpoView.Companion.NATIVE_ACTION_RETRY_DELAY_MS
import android.view.View

internal fun NativeEditorExpoView.clearPendingNativeActionRetry() {
    pendingNativeAction = null
    pendingNativeActionScope = null
    pendingNativeActionRetryEditorId = null
    pendingNativeActionRetryScheduled = false
    pendingNativeActionRetryAttempts = 0
    pendingNativeActionRetryGeneration += 1
}

internal fun NativeEditorExpoView.currentNativeActionScope(action: PendingNativeAction): PendingNativeActionScope {
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

internal fun NativeEditorExpoView.isPendingNativeActionScopeCurrent(
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

internal fun NativeEditorExpoView.isNativeActionToolbarVisible(action: PendingNativeAction): Boolean {
    if (!showsToolbar || toolbarPlacement != ToolbarPlacement.KEYBOARD) return false
    if (keyboardToolbarView.parent == null || keyboardToolbarView.visibility != View.VISIBLE) return false
    if (action is PendingNativeAction.MentionSuggestionSelect) {
        return keyboardToolbarView.isShowingMentionSuggestions
    }
    return true
}

internal fun NativeEditorExpoView.isEditorEffectivelyFocusedForNativeAction(): Boolean =
    richTextView.editorEditText.hasFocus() ||
        (pendingToolbarRefocus != null && pendingToolbarRefocusEditorId == richTextView.editorId)

internal fun NativeEditorExpoView.clearPendingNativeActionRetryIfScopeChanged() {
    val action = pendingNativeAction ?: return
    val scope = pendingNativeActionScope ?: return
    if (!isPendingNativeActionScopeCurrent(action, scope)) {
        clearPendingNativeActionRetry()
    }
}

internal fun NativeEditorExpoView.schedulePendingNativeActionRetry(action: PendingNativeAction) {
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

internal fun NativeEditorExpoView.retryPendingNativeActionFromWake() {
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
