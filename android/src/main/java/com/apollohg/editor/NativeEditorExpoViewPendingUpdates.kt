package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.Companion.MAX_PENDING_UPDATE_RETRY_ATTEMPTS
import com.apollohg.editor.NativeEditorExpoView.Companion.PENDING_UPDATE_RECOVERY_RETRY_DELAY_MS
import com.apollohg.editor.NativeEditorExpoView.Companion.NATIVE_ACTION_RETRY_DELAY_MS
import android.os.Looper

internal fun NativeEditorExpoView.setPendingEditorUpdateJsonImpl(editorUpdateJson: String?) {
    lastEditorUpdateJsonProp = editorUpdateJson
    pendingEditorUpdateJson = editorUpdateJson
}

internal fun NativeEditorExpoView.setPendingEditorUpdateEditorHandleImpl(editorUpdateEditorHandle: String?) {
    val viewToken = editorUpdateEditorHandle?.let(EditorV2Registry::viewTokenForHandle)
    lastEditorUpdateEditorIdProp = viewToken
    pendingEditorUpdateEditorId = viewToken
}

    /** Internal widget/test hook; production props always use decimal handles. */
internal fun NativeEditorExpoView.setPendingEditorUpdateEditorIdImpl(viewToken: Long?) {
    lastEditorUpdateEditorIdProp = viewToken
    pendingEditorUpdateEditorId = viewToken
}

internal fun NativeEditorExpoView.setPendingEditorUpdateRevisionImpl(editorUpdateRevision: Long) {
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

internal fun NativeEditorExpoView.setPendingEditorResetUpdateJsonImpl(editorResetUpdateJson: String?) {
    lastEditorResetUpdateJsonProp = editorResetUpdateJson
    pendingEditorResetUpdateJson = editorResetUpdateJson
}

internal fun NativeEditorExpoView.setPendingEditorResetUpdateEditorHandleImpl(editorResetUpdateEditorHandle: String?) {
    val viewToken = editorResetUpdateEditorHandle?.let(EditorV2Registry::viewTokenForHandle)
    lastEditorResetUpdateEditorIdProp = viewToken
    pendingEditorResetUpdateEditorId = viewToken
}

    /** Internal widget/test hook; production props always use decimal handles. */
internal fun NativeEditorExpoView.setPendingEditorResetUpdateEditorIdImpl(viewToken: Long?) {
    lastEditorResetUpdateEditorIdProp = viewToken
    pendingEditorResetUpdateEditorId = viewToken
}

internal fun NativeEditorExpoView.setPendingEditorResetUpdateRevisionImpl(editorResetUpdateRevision: Long) {
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

internal fun NativeEditorExpoView.isConsumedEditorUpdateRevision(editorId: Long, revision: Long): Boolean =
    revision != 0L &&
        consumedEditorUpdateEditorId == editorId &&
        consumedEditorUpdateRevision == revision

internal fun NativeEditorExpoView.isConsumedEditorResetUpdateRevision(editorId: Long, revision: Long): Boolean =
    revision != 0L &&
        consumedEditorResetUpdateEditorId == editorId &&
        consumedEditorResetUpdateRevision == revision

internal fun NativeEditorExpoView.consumeEditorUpdateRevision(editorId: Long, revision: Long) {
    consumedEditorUpdateEditorId = editorId
    consumedEditorUpdateRevision = revision
}

internal fun NativeEditorExpoView.consumeEditorResetUpdateRevision(editorId: Long, revision: Long) {
    consumedEditorResetUpdateEditorId = editorId
    consumedEditorResetUpdateRevision = revision
}

internal fun NativeEditorExpoView.hasPendingEditorUpdateForEditor(editorId: Long): Boolean =
    pendingEditorUpdateJson != null &&
        pendingEditorUpdateRevision != 0L &&
        pendingEditorUpdateRevision != appliedEditorUpdateRevision &&
        !isConsumedEditorUpdateRevision(editorId, pendingEditorUpdateRevision) &&
        pendingEditorUpdateEditorId == editorId

internal fun NativeEditorExpoView.hasPendingEditorResetUpdateForEditor(editorId: Long): Boolean =
    pendingEditorResetUpdateJson != null &&
        pendingEditorResetUpdateRevision != 0L &&
        pendingEditorResetUpdateRevision != appliedEditorResetUpdateRevision &&
        !isConsumedEditorResetUpdateRevision(editorId, pendingEditorResetUpdateRevision) &&
        pendingEditorResetUpdateEditorId == editorId

internal fun NativeEditorExpoView.hasPendingEditorUpdateForCurrentEditor(): Boolean =
    hasPendingEditorUpdateForEditor(richTextView.editorId)

internal fun NativeEditorExpoView.hasPendingEditorResetUpdateForCurrentEditor(): Boolean =
    hasPendingEditorResetUpdateForEditor(richTextView.editorId)

internal fun NativeEditorExpoView.pendingEditorUpdateCommandPreparationJSON(): String =
    NativeEditorViewRegistry.commandPreparationJSON(
        ready = false,
        blockedReason = "pendingUpdate"
    )

internal fun NativeEditorExpoView.shouldBlockEditorCommandForPendingUpdate(): Boolean =
    hasPendingEditorResetUpdateForCurrentEditor() || hasPendingEditorUpdateForCurrentEditor()

internal fun NativeEditorExpoView.refreshReadyStateIfSettled() {
    if (handleDestroyedCurrentEditorIfNeeded()) return
    if (hasPendingEditorResetUpdateForCurrentEditor()) return
    if (hasPendingEditorUpdateForCurrentEditor()) return
    if (!isAttachedToNativeWindow) return
    if (richTextView.editorEditText.editorId != richTextView.editorId) return
    refreshToolbarStateFromEditorSelection()
    refreshMentionQuery()
    emitEditorReadyIfNeeded()
}

internal fun NativeEditorExpoView.applyPendingEditorResetUpdateIfNeededImpl() {
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

internal fun NativeEditorExpoView.applyPendingEditorUpdateIfNeededImpl() {
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
        val resetJson = pendingEditorUpdateResetJson
        val outcome = if (resetJson != null) {
            applyEditorResetUpdateOutcome(updateJson, resetJson)
        } else applyEditorUpdateOutcome(
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

internal fun NativeEditorExpoView.clearPendingEditorUpdateState(resetAppliedRevision: Boolean = true) {
    pendingEditorUpdateJson = null
    pendingEditorUpdateEditorId = null
    pendingEditorUpdateRevision = 0L
    if (resetAppliedRevision) {
        pendingEditorUpdateResetJson = null
        appliedEditorUpdateRevision = 0L
    }
    cancelPendingEditorUpdateRetry(PendingEditorUpdateKind.ORDINARY)
}

internal fun NativeEditorExpoView.clearPendingEditorResetUpdateState(resetAppliedRevision: Boolean = true) {
    pendingEditorResetUpdateJson = null
    pendingEditorResetUpdateEditorId = null
    pendingEditorResetUpdateRevision = 0L
    if (resetAppliedRevision) {
        appliedEditorResetUpdateRevision = 0L
    }
    cancelPendingEditorUpdateRetry(PendingEditorUpdateKind.RESET)
}

internal fun NativeEditorExpoView.cancelPendingEditorUpdateRetry(kind: PendingEditorUpdateKind? = null) {
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

internal fun NativeEditorExpoView.schedulePendingEditorUpdateRetry(kind: PendingEditorUpdateKind) {
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
