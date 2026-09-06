package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.ActiveExternalTextComposition
import org.json.JSONObject

internal fun NativeEditorExpoView.beginExternalTextCompositionImpl(sessionId: String): String {
    val resultJson = richTextView.editorEditText.beginExternalTextComposition(sessionId)
    val started = runCatching {
        val result = JSONObject(resultJson)
        result.optString("type") == "active" && result.opt("sessionId") == sessionId
    }.getOrDefault(false)
    if (started) {
        activeExternalTextComposition = ActiveExternalTextComposition(
            sessionId = sessionId,
            editorId = eventEditorId(richTextView.editorId),
        )
    }
    return resultJson
}

internal fun NativeEditorExpoView.updateExternalTextCompositionImpl(sessionId: String, text: String): String =
    richTextView.editorEditText.updateExternalTextComposition(sessionId, text)

internal fun NativeEditorExpoView.commitExternalTextCompositionImpl(sessionId: String, finalText: String): String =
    richTextView.editorEditText.commitExternalTextComposition(sessionId, finalText)

internal fun NativeEditorExpoView.cancelExternalTextCompositionImpl(sessionId: String, cause: String): String =
    richTextView.editorEditText.cancelExternalTextComposition(sessionId, cause)

internal fun NativeEditorExpoView.cancelActiveExternalTextComposition(cause: String) {
    val composition = activeExternalTextComposition ?: return
    richTextView.editorEditText.cancelExternalTextComposition(composition.sessionId, cause)
}
