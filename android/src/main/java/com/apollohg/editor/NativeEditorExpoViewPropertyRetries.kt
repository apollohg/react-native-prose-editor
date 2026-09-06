package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.PendingPropertyRetry
import com.apollohg.editor.NativeEditorExpoView.PendingPropertyRetryResult
import com.apollohg.editor.NativeEditorExpoView.Companion.MAX_PENDING_UPDATE_RETRY_ATTEMPTS
import com.apollohg.editor.NativeEditorExpoView.Companion.NATIVE_ACTION_RETRY_DELAY_MS
import android.os.Looper

internal fun NativeEditorExpoView.clearPendingThemeRetry() {
    pendingThemeJson = null
    hasPendingTheme = false
    cancelPendingThemeRetry()
}

internal fun NativeEditorExpoView.cancelPendingThemeRetry() {
    pendingThemeRetry.cancel()
}

internal fun NativeEditorExpoView.applyPendingThemeIfNeeded() {
    if (handleDestroyedCurrentEditorIfNeeded()) return
    if (!hasPendingTheme) return
    val themeJson = pendingThemeJson
    val editorId = richTextView.editorId
    if (pendingThemeRetry.editorId != editorId) {
        pendingThemeRetry.bind(editorId)
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

internal fun NativeEditorExpoView.schedulePendingThemeRetry() {
    schedulePendingPropertyRetry(
        pendingThemeRetry,
        onEditorChanged = ::clearPendingThemeRetry,
        apply = ::applyPendingThemeIfNeeded,
    )
}

internal fun NativeEditorExpoView.cancelPendingAtomsRetry() {
    pendingAtomsRetry.cancel()
}

internal fun NativeEditorExpoView.applyPendingAtomsIfNeeded() {
    if (handleDestroyedCurrentEditorIfNeeded()) return
    if (!hasPendingAtoms) return
    val atomsJson = pendingAtomsJson
    val editorId = richTextView.editorId
    if (pendingAtomsRetry.editorId != editorId) {
        pendingAtomsRetry.bind(editorId)
    }
    val configuration = AtomRenderConfiguration.fromJson(atomsJson)
    if (!richTextView.applyAtomRenderConfiguration(configuration)) {
        schedulePendingAtomsRetry()
        return
    }
    lastAtomsJson = atomsJson
    pendingAtomsJson = null
    hasPendingAtoms = false
    cancelPendingAtomsRetry()
}

internal fun NativeEditorExpoView.schedulePendingAtomsRetry() {
    schedulePendingPropertyRetry(
        pendingAtomsRetry,
        onEditorChanged = ::cancelPendingAtomsRetry,
        apply = ::applyPendingAtomsIfNeeded,
    )
}

internal fun NativeEditorExpoView.schedulePendingPropertyRetry(
    state: PendingPropertyRetry,
    onEditorChanged: () -> Unit,
    apply: () -> Unit,
) {
    val (generation, attempt) = state.schedule(
        richTextView.editorId,
        MAX_PENDING_UPDATE_RETRY_ATTEMPTS,
    ) ?: return
    val retry = Runnable {
        when (state.consume(generation, richTextView.editorId)) {
            PendingPropertyRetryResult.STALE -> return@Runnable
            PendingPropertyRetryResult.EDITOR_CHANGED -> onEditorChanged()
            PendingPropertyRetryResult.READY -> apply()
        }
    }
    mainHandler.postDelayed(retry, NATIVE_ACTION_RETRY_DELAY_MS * attempt)
}

internal fun NativeEditorExpoView.clearPendingViewCommandUpdateRetry() {
    pendingViewCommandUpdateJson = null
    pendingViewCommandUpdateEditorId = null
    pendingViewCommandUpdateRetryScheduled = false
    pendingViewCommandUpdateRetryAttempts = 0
    pendingViewCommandUpdateRetryGeneration += 1
}

internal fun NativeEditorExpoView.scheduleViewCommandUpdateRetry(updateJson: String) {
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

internal fun NativeEditorExpoView.schedulePendingPreflightWake() {
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

internal fun NativeEditorExpoView.cancelPendingPreflightWake() {
    pendingPreflightWakeScheduled = false
    pendingPreflightWakeGeneration += 1
}

internal fun NativeEditorExpoView.wakePendingPreflightWork() {
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
        pendingThemeRetry.resetAttempts()
        applyPendingThemeIfNeeded()
    }
    if (hasPendingAtoms) {
        pendingAtomsRetry.resetAttempts()
        applyPendingAtomsIfNeeded()
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
