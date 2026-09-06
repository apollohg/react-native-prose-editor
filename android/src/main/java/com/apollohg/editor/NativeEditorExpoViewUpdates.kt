package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.Companion.LOG_TAG
import android.os.Looper
import android.util.Log

/** Applies an editor update from JS without echoing it back through events. */
internal fun NativeEditorExpoView.applyEditorUpdateImpl(updateJson: String): Boolean =
    applyEditorUpdateOutcome(updateJson, scheduleViewCommandRetry = true) ==
        PendingEditorUpdateApplyOutcome.APPLIED

    /** Applies a reset-style update from JS, discarding pending native composition. */
internal fun NativeEditorExpoView.applyEditorResetUpdateImpl(updateJson: String): Boolean {
    return applyEditorResetUpdateOutcome(updateJson) == PendingEditorUpdateApplyOutcome.APPLIED
}

internal fun NativeEditorExpoView.applyEditorResetUpdateOutcome(updateJson: String): PendingEditorUpdateApplyOutcome {
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

internal fun NativeEditorExpoView.isEditorReadyForNativeUpdate(): Boolean {
    val editorId = richTextView.editorId
    return editorId == 0L || (isAttachedToNativeWindow && richTextView.editorEditText.editorId == editorId)
}

    @Synchronized
internal fun NativeEditorExpoView.markRemoteCommitRebaseScheduledImpl(editorId: Long): Boolean {
    if (remoteCommitRebaseScheduled && remoteCommitRebaseEditorId == editorId) return false
    remoteCommitRebaseScheduled = true
    remoteCommitRebaseEditorId = editorId
    return true
}

    @Synchronized
internal fun NativeEditorExpoView.clearRemoteCommitRebaseScheduled(editorId: Long) {
    if (remoteCommitRebaseEditorId != editorId) return
    remoteCommitRebaseScheduled = false
    remoteCommitRebaseEditorId = null
}

internal fun NativeEditorExpoView.applyRemoteCommitRefreshImpl(expectedEditorId: Long) {
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

internal fun NativeEditorExpoView.applyEditorUpdateOutcome(
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

internal fun NativeEditorExpoView.prepareForEditorCommandJSONImpl(): String {
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
