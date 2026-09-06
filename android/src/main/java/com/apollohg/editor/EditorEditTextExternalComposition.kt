package com.apollohg.editor

import android.text.Selection
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.view.inputmethod.BaseInputConnection
import org.json.JSONObject

internal fun EditorEditText.runWithTransientInputMutationGuardImpl(block: () -> Boolean): Boolean {
    val wasApplyingRustState = isApplyingRustState
    isApplyingRustState = true
    return try {
        block()
    } finally {
        isApplyingRustState = wasApplyingRustState
    }
}

internal fun EditorEditText.beginExternalTextCompositionImpl(sessionId: String): String {
    val driver = v2Driver
    if (!hasLiveEditor() || !isEditable || driver == null) {
        return externalCompositionErrorJSON(
            sessionId,
            "EXTERNAL_COMPOSITION_UNAVAILABLE",
            "The native editor is unavailable or not editable"
        )
    }
    if (externalTextCompositionTerminalResults.containsKey(sessionId)) {
        return externalCompositionEndedErrorJSON(sessionId)
    }
    val selection = try {
        driver.selectionJson()?.let(::JSONObject)
    } catch (_: Exception) {
        null
    }
    if (selection?.optString("type") != "text") {
        return externalCompositionErrorJSON(
            sessionId,
            "EXTERNAL_COMPOSITION_SELECTION_INCOMPATIBLE",
            "External composition requires a text selection"
        )
    }

    if (externalTextComposition != null) {
        if (!finishExternalTextComposition("consumer", finalText = null, cancel = false)) {
            return externalCompositionErrorJSON(
                sessionId,
                "EXTERNAL_COMPOSITION_COMMIT_FAILED",
                "The previous external text composition could not be committed"
            )
        }
        if (externalTextCompositionTerminalResults.containsKey(sessionId)) {
            return externalCompositionEndedErrorJSON(sessionId)
        }
    }
    if (!prepareForExternalEditorUpdateInternal().ready) {
        return externalCompositionErrorJSON(
            sessionId,
            "EXTERNAL_COMPOSITION_UNAVAILABLE",
            "Pending native input could not be committed"
        )
    }

    captureCompositionReplacementRangeIfNeeded()
    val (replacementStart, replacementEnd) = compositionReplacementRange()
        ?: return externalCompositionErrorJSON(
            sessionId,
            "EXTERNAL_COMPOSITION_SELECTION_INCOMPATIBLE",
            "External composition requires a text selection"
        )
    externalTextComposition = ExternalTextCompositionState(
        sessionId = sessionId,
        latestText = "",
        replacementStartUtf16 = replacementStart,
        replacementEndUtf16 = replacementEnd,
        startingAuthorizedText = lastAuthorizedText,
        startingAuthorizedRenderedText = lastAuthorizedRenderedText?.let(::SpannableStringBuilder)
    )
    return externalCompositionActiveJSON(sessionId)
}

internal fun EditorEditText.updateExternalTextCompositionImpl(sessionId: String, text: String): String {
    val state = externalTextComposition
    if (state?.sessionId != sessionId) {
        return externalCompositionEndedErrorJSON(sessionId)
    }
    renderExternalTextComposition(text)
    return externalCompositionActiveJSON(sessionId)
}

internal fun EditorEditText.commitExternalTextCompositionImpl(sessionId: String, finalText: String): String {
    if (externalTextComposition?.sessionId != sessionId) {
        return externalTextCompositionTerminalResults[sessionId]
            ?: externalCompositionEndedErrorJSON(sessionId)
    }
    finishExternalTextComposition("consumer", finalText, cancel = false)
    return externalTextCompositionTerminalResults[sessionId]
        ?: externalCompositionEndedErrorJSON(sessionId)
}

internal fun EditorEditText.cancelExternalTextCompositionImpl(sessionId: String, cause: String): String {
    if (cause !in setOf("consumer", "documentChange", "lifecycle")) {
        return externalCompositionErrorJSON(
            sessionId,
            "EXTERNAL_COMPOSITION_CANCEL_CAUSE_INVALID",
            "The external composition cancellation cause is invalid"
        )
    }
    if (externalTextComposition?.sessionId != sessionId) {
        return externalTextCompositionTerminalResults[sessionId]
            ?: externalCompositionEndedErrorJSON(sessionId)
    }
    finishExternalTextComposition(cause, finalText = null, cancel = true)
    return externalTextCompositionTerminalResults[sessionId]
        ?: externalCompositionEndedErrorJSON(sessionId)
}

internal fun EditorEditText.commitExternalTextCompositionBeforeInteractionIfNeededImpl(): Boolean =
    externalTextComposition == null ||
        finishExternalTextComposition("interaction", finalText = null, cancel = false)

internal fun EditorEditText.hasActiveExternalTextCompositionForEditorImpl(): Boolean =
    externalTextComposition != null

internal fun EditorEditText.commitExternalTextCompositionForDocumentChangeIfNeeded(): Boolean =
    externalTextComposition == null ||
        finishExternalTextComposition("documentChange", finalText = null, cancel = false)

internal fun EditorEditText.cancelExternalTextCompositionForLifecycleIfNeeded() {
    if (externalTextComposition != null) {
        finishExternalTextComposition("lifecycle", finalText = null, cancel = true)
    }
}

internal fun EditorEditText.renderExternalTextComposition(text: String) {
    val state = externalTextComposition ?: return
    val editable = this.text ?: return
    val visibleStart = state.replacementStartUtf16.coerceIn(0, editable.length)
    val visibleEnd = if (editable.toString() == state.startingAuthorizedText) {
        state.replacementEndUtf16
    } else {
        visibleStart + state.latestText.length
    }.coerceIn(visibleStart, editable.length)
    runWithTransientInputMutationGuard {
        editable.replace(visibleStart, visibleEnd, text)
        BaseInputConnection.removeComposingSpans(editable)
        if (text.isNotEmpty()) {
            editable.setSpan(
                externalCompositionMarker,
                visibleStart,
                visibleStart + text.length,
                Spanned.SPAN_EXCLUSIVE_EXCLUSIVE or Spanned.SPAN_COMPOSING
            )
        }
        Selection.setSelection(editable, visibleStart + text.length)
        true
    }
    state.latestText = text
    setComposingTextForEditor(text)
    invalidate()
}

internal fun EditorEditText.finishExternalTextComposition(
    cause: String,
    finalText: String?,
    cancel: Boolean
): Boolean {
    val state = externalTextComposition ?: return true
    if (finalText != null && finalText != state.latestText) {
        renderExternalTextComposition(finalText)
    }
    if (finalText != null) state.latestText = finalText
    val authoritativeCancellationUpdate = if (cancel) {
        v2Driver?.currentStateJson()
    } else {
        null
    }
    externalTextComposition = null
    clearCompositionTrackingForEditor()

    if (cancel) {
        restoreAuthorizedExternalComposition(state, authoritativeCancellationUpdate)
        emitExternalTextCompositionEnd(
            state.sessionId,
            externalCompositionEndedJSON(
                state.sessionId,
                "cancelled",
                cause,
                state.latestText
            )
        )
        return true
    }

    val driver = v2Driver
    restoreAuthorizedExternalComposition(state)
    val scalarFrom = PositionBridge.utf16ToScalar(
        state.replacementStartUtf16,
        state.startingAuthorizedText
    )
    val scalarTo = PositionBridge.utf16ToScalar(
        state.replacementEndUtf16,
        state.startingAuthorizedText
    )
    val nativeOutcome = if (driver is EditorV2Adapter) {
        driver.replaceTextRangeNative(scalarFrom, scalarTo, state.latestText)
    } else {
        null
    }
    if (nativeOutcome is EditorV2NativeIntentResult.Recovered) {
        applyUpdateJSON(nativeOutcome.updateJson, notifyListener = false)
        return failExternalTextCompositionCommit(state, cause)
    }
    val updateJSON = when (nativeOutcome) {
        is EditorV2NativeIntentResult.Applied -> nativeOutcome.render.updateJson
        EditorV2NativeIntentResult.Rejected -> null
        is EditorV2NativeIntentResult.Recovered -> null
        null -> driver?.replaceTextRange(scalarFrom, scalarTo, state.latestText)
    }
    if (updateJSON == null) {
        val recoveryJSON = (driver as? EditorV2Adapter)?.recoverNativeRender()
            ?: if (driver is EditorV2Adapter) null else driver?.currentStateJson()
        recoveryJSON?.let {
            applyUpdateJSON(it, notifyListener = false)
        }
        return failExternalTextCompositionCommit(state, cause)
    }

    applyUpdateJSON(updateJSON, notifyListener = false)
    val documentChanged = (nativeOutcome as? EditorV2NativeIntentResult.Applied)
        ?.render
        ?.documentChanged
        ?: (text?.toString() != state.startingAuthorizedText)
    if (documentChanged) {
        editorListener?.onEditorUpdate(updateJSON)
    }
    emitExternalTextCompositionEnd(
        state.sessionId,
        externalCompositionEndedJSON(
            state.sessionId,
            "committed",
            cause,
            state.latestText
        )
    )
    return true
}

internal fun EditorEditText.failExternalTextCompositionCommit(
    state: ExternalTextCompositionState,
    cause: String
): Boolean {
    val resultJSON = externalCompositionEndedJSON(
        state.sessionId,
        "cancelled",
        cause,
        state.latestText,
        externalCompositionErrorPayload(
            "EXTERNAL_COMPOSITION_COMMIT_FAILED",
            "The external text composition could not be committed"
        )
    )
    emitExternalTextCompositionEnd(state.sessionId, resultJSON)
    return false
}

internal fun EditorEditText.restoreAuthorizedExternalComposition(
    state: ExternalTextCompositionState,
    authoritativeUpdateJSON: String? = null
) {
    val snapshot = state.startingAuthorizedRenderedText ?: state.startingAuthorizedText
    runWithTransientInputMutationGuard {
        beginBatchEdit()
        try {
            setText(snapshot)
            val length = text?.length ?: 0
            Selection.setSelection(
                text,
                state.replacementStartUtf16.coerceIn(0, length),
                state.replacementEndUtf16.coerceIn(0, length)
            )
        } finally {
            endBatchEdit()
        }
        true
    }
    authoritativeUpdateJSON?.let {
        applyUpdateJSON(it, notifyListener = false)
    }
}

internal fun EditorEditText.emitExternalTextCompositionEnd(sessionId: String, resultJson: String) {
    if (externalTextCompositionTerminalResults.putIfAbsent(sessionId, resultJson) != null) return
    editorListener?.onExternalTextCompositionEnded(resultJson)
}

internal fun EditorEditText.externalCompositionActiveJSON(sessionId: String): String =
    JSONObject()
        .put("version", 1)
        .put("type", "active")
        .put("sessionId", sessionId)
        .toString()

internal fun EditorEditText.externalCompositionEndedJSON(
    sessionId: String,
    outcome: String,
    cause: String,
    text: String,
    error: JSONObject? = null
): String = JSONObject()
    .put("version", 1)
    .put("type", "ended")
    .put("sessionId", sessionId)
    .put("outcome", outcome)
    .put("cause", cause)
    .put("text", text)
    .apply { if (error != null) put("error", error) }
    .toString()

internal fun EditorEditText.externalCompositionEndedErrorJSON(sessionId: String): String =
    externalCompositionErrorJSON(
        sessionId,
        "EXTERNAL_COMPOSITION_ENDED",
        "The external text composition session has ended"
    )

internal fun EditorEditText.externalCompositionErrorJSON(
    sessionId: String?,
    code: String,
    message: String
): String = JSONObject()
    .put("version", 1)
    .put("type", "error")
    .put("sessionId", sessionId ?: JSONObject.NULL)
    .put("error", externalCompositionErrorPayload(code, message))
    .toString()

internal fun EditorEditText.externalCompositionErrorPayload(code: String, message: String): JSONObject =
    JSONObject()
        .put("domain", "lifecycle")
        .put("code", code)
        .put("message", message)
        .put("requestId", JSONObject.NULL)
        .put("operationIndex", JSONObject.NULL)
        .put("limit", JSONObject.NULL)
        .put("actual", JSONObject.NULL)
        .put("details", JSONObject.NULL)
