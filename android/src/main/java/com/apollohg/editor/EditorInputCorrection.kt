package com.apollohg.editor

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.text.Selection
import android.text.Spanned
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper


internal data class PendingDuplicateCorrectionCommit(
    val text: String,
    val deadlineMs: Long
)

internal data class PendingCompositionCorrectionCommit(
    val text: String,
    val deadlineMs: Long,
    val generation: Long
)

internal data class GeneratedCompositionAdjustment(
    val leadingText: String,
    val trailingText: String
) {
    fun sanitize(text: String): String {
        var sanitized = text
        if (leadingText.isNotEmpty() && sanitized.startsWith(leadingText)) {
            sanitized = sanitized.substring(leadingText.length)
        }
        if (trailingText.isNotEmpty() && sanitized.endsWith(trailingText)) {
            sanitized = sanitized.substring(0, sanitized.length - trailingText.length)
        }
        return sanitized
    }
}

internal fun EditorInputConnection.rememberPendingDuplicateCorrectionCommit(text: String) {
    pendingDuplicateCorrectionCommit = PendingDuplicateCorrectionCommit(
        text = text,
        deadlineMs = SystemClock.uptimeMillis() + EditorInputConnection.DUPLICATE_CORRECTION_COMMIT_WINDOW_MS
    )
}

internal fun EditorInputConnection.consumePendingDuplicateCorrectionCommitIfNeeded(text: String?): Boolean {
    val pending = pendingDuplicateCorrectionCommit ?: return false
    pendingDuplicateCorrectionCommit = null
    if (text == null) return false
    if (SystemClock.uptimeMillis() > pending.deadlineMs) return false
    return text == pending.text
}

internal fun EditorInputConnection.rememberPendingCompositionCorrectionCommit(text: String) {
    val generation = ++pendingCompositionCorrectionGeneration
    pendingCompositionCorrectionCommit = PendingCompositionCorrectionCommit(
        text = text,
        deadlineMs = SystemClock.uptimeMillis() + EditorInputConnection.DUPLICATE_CORRECTION_COMMIT_WINDOW_MS,
        generation = generation
    )
    Handler(Looper.getMainLooper()).post {
        val pending = pendingCompositionCorrectionCommit ?: return@post
        if (pending.generation != generation) return@post
        applyPendingCompositionCorrectionCommitIfNeeded("commitCorrectionDeferred")
    }
}

internal fun EditorInputConnection.consumePendingCompositionCorrectionCommitIfNeeded(
    text: String?,
    newCursorPosition: Int
): Boolean {
    val pending = pendingCompositionCorrectionCommit ?: return false
    if (SystemClock.uptimeMillis() > pending.deadlineMs) {
        pendingCompositionCorrectionCommit = null
        return false
    }
    if (text != pending.text) return false
    pendingCompositionCorrectionCommit = null
    pendingCompositionCorrectionGeneration += 1L
    editorView.recordImeTraceForTesting(
        "commitTextConsumesPendingCorrection",
        "textLength=${text.length}"
    )
    commitTextToEditor(text, newCursorPosition)
    return true
}

internal fun EditorInputConnection.applyPendingCompositionCorrectionCommitIfNeeded(source: String): Boolean {
    val pending = pendingCompositionCorrectionCommit ?: return false
    pendingCompositionCorrectionCommit = null
    pendingCompositionCorrectionGeneration += 1L
    if (!isCurrentInputSessionFor("applyPendingCompositionCorrection")) return false
    if (!editorView.isEditable || editorView.editorId == 0L) return false
    editorView.recordImeTraceForTesting(
        "applyPendingCompositionCorrection",
        "source=$source textLength=${pending.text.length}"
    )
    commitTextToEditor(pending.text, 1)
    return true
}
