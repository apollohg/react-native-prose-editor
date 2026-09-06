package com.apollohg.editor

import com.apollohg.editor.NativeEditorExpoView.PendingNativeAction
import org.json.JSONArray
import org.json.JSONObject

internal fun NativeEditorExpoView.refreshMentionQuery() {
    val mentions = addons.mentions
    if (mentions == null || !richTextView.editorEditText.hasFocus()) {
        clearMentionQueryState()
        emitMentionQueryChange("", "@", 0, 0, false)
        return
    }

    val queryState = currentMentionQueryState(mentions.trigger)
    if (queryState == null) {
        clearMentionQueryState()
        emitMentionQueryChange("", mentions.trigger, 0, 0, false)
        return
    }

    mentionQueryState = queryState
    val suggestions = filteredMentionSuggestions(queryState, mentions)
    keyboardToolbarView.applyMentionTheme(richTextView.editorEditText.theme?.mentions ?: mentions.theme)
    syncKeyboardToolbarMentionSuggestions(suggestions, mentions.trigger)
    emitMentionQueryChange(
        queryState.query,
        queryState.trigger,
        queryState.anchor,
        queryState.head,
        true
    )
}

internal fun NativeEditorExpoView.clearMentionQueryState(resetLastEvent: Boolean = false) {
    mentionQueryState = null
    if (resetLastEvent) {
        lastMentionEventJson = null
        lastMentionEventEditorId = null
    }
    syncKeyboardToolbarMentionSuggestions(emptyList())
}

internal fun NativeEditorExpoView.currentMentionQueryState(trigger: String): MentionQueryState? {
    val editor = richTextView.editorEditText
    if (editor.selectionStart != editor.selectionEnd) return null
    val text = editor.text?.toString() ?: return null
    val cursorUtf16 = editor.selectionStart
    val cursorScalar = PositionBridge.utf16ToScalar(cursorUtf16, text)
    return resolveMentionQueryState(
        text = text,
        cursorScalar = cursorScalar,
        trigger = trigger,
        isCaretInsideMention = isCaretInsideMention(cursorUtf16)
    )
}

internal fun NativeEditorExpoView.isCaretInsideMention(cursorUtf16: Int): Boolean {
    val editable = richTextView.editorEditText.text ?: return false
    val checkOffsets = listOf(cursorUtf16, (cursorUtf16 - 1).coerceAtLeast(0))
    return checkOffsets.any { offset ->
        editable.getSpans(offset, offset, android.text.Annotation::class.java).any { span ->
            span.key == "nativeVoidNodeType" && span.value == "mention"
        }
    }
}

internal fun NativeEditorExpoView.filteredMentionSuggestions(
    queryState: MentionQueryState,
    config: NativeMentionsAddonConfig
): List<NativeMentionSuggestion> {
    val normalizedQuery = queryState.query.trim().lowercase()
    if (normalizedQuery.isEmpty()) return config.suggestions
    return config.suggestions.filter { suggestion ->
        suggestion.title.lowercase().contains(normalizedQuery) ||
            suggestion.label.lowercase().contains(normalizedQuery) ||
            (suggestion.subtitle?.lowercase()?.contains(normalizedQuery) == true)
    }
}

internal fun NativeEditorExpoView.syncKeyboardToolbarMentionSuggestions(
    suggestions: List<NativeMentionSuggestion>,
    trigger: String = addons.mentions?.trigger ?: "@"
) {
    keyboardToolbarView.setMentionSuggestions(suggestions, trigger)
    keyboardToolbarView.requestLayout()
    post {
        updateKeyboardToolbarLayout()
        updateEditorViewportInset()
    }
}

internal fun NativeEditorExpoView.emitMentionQueryChange(
    query: String,
    trigger: String,
    anchor: Int,
    head: Int,
    isActive: Boolean
) {
    val eventJson = JSONObject()
        .put("type", "mentionsQueryChange")
        .put("query", query)
        .put("trigger", trigger)
        .put("range", JSONObject().put("anchor", anchor).put("head", head))
        .put("isActive", isActive)
        .apply {
            lastDocumentVersion?.let { put("documentVersion", it) }
        }
        .toString()
    val editorId = richTextView.editorId
    if (eventJson == lastMentionEventJson && editorId == lastMentionEventEditorId) return
    lastMentionEventJson = eventJson
    lastMentionEventEditorId = editorId
    emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to eventEditorId(editorId)))
}

internal fun NativeEditorExpoView.resolvedMentionAttrs(
    trigger: String,
    suggestion: NativeMentionSuggestion
): JSONObject {
    val attrs = JSONObject(suggestion.attrs.toString())
    if (!attrs.has("label")) {
        attrs.put("label", suggestion.label)
    }
    if (!attrs.has("mentionSuggestionChar")) {
        attrs.put("mentionSuggestionChar", trigger)
    }
    return attrs
}

internal fun NativeEditorExpoView.emitMentionSelect(trigger: String, suggestion: NativeMentionSuggestion, attrs: JSONObject) {
    val eventJson = JSONObject()
        .put("type", "mentionsSelect")
        .put("trigger", trigger)
        .put("suggestionKey", suggestion.key)
        .put("attrs", attrs)
        .apply {
            lastDocumentVersion?.let { put("documentVersion", it) }
        }
        .toString()
    emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to eventEditorId(richTextView.editorId)))
}

internal fun NativeEditorExpoView.emitMentionSelectRequest(
    trigger: String,
    suggestion: NativeMentionSuggestion,
    attrs: JSONObject,
    range: MentionQueryState,
    preflightUpdateJSON: String?
) {
    val eventJson = JSONObject()
        .put("type", "mentionsSelectRequest")
        .put("trigger", trigger)
        .put("suggestionKey", suggestion.key)
        .put("attrs", attrs)
        .put("range", JSONObject().put("anchor", range.anchor).put("head", range.head))
        .apply {
            if (preflightUpdateJSON != null) {
                put("updateJson", preflightUpdateJSON)
            }
            (documentVersionFromUpdateJSON(preflightUpdateJSON) ?: lastDocumentVersion)
                ?.let { put("documentVersion", it) }
        }
        .toString()
    emitAddonEvent(mapOf("eventJson" to eventJson, "editorId" to eventEditorId(richTextView.editorId)))
}

internal fun NativeEditorExpoView.insertMentionSuggestion(
    suggestion: NativeMentionSuggestion,
    allowPreflightRetry: Boolean = true
) {
    if (handleDestroyedCurrentEditorIfNeeded()) return
    if (!richTextView.editorEditText.isEditable) {
        clearPendingNativeActionRetry()
        return
    }
    val mentions = addons.mentions ?: return
    if (shouldBlockEditorCommandForPendingUpdate()) {
        if (allowPreflightRetry) {
            schedulePendingNativeActionRetry(
                PendingNativeAction.MentionSuggestionSelect(suggestion)
            )
        }
        return
    }
    val preparation = richTextView.editorEditText.prepareForExternalEditorCommand()
    if (!preparation.ready) {
        if (allowPreflightRetry) {
            schedulePendingNativeActionRetry(
                PendingNativeAction.MentionSuggestionSelect(suggestion)
            )
        }
        return
    }
    val preflightUpdateJSON = preparation.updateJSON
    noteDocumentVersionFromUpdateJSON(preflightUpdateJSON)
    clearPendingNativeActionRetry()
    val queryState = currentMentionQueryState(mentions.trigger) ?: run {
        clearMentionQueryState()
        return
    }
    val freshSuggestions = filteredMentionSuggestions(queryState, mentions)
    if (freshSuggestions.none { it.key == suggestion.key }) {
        refreshMentionQuery()
        return
    }
    mentionQueryState = queryState
    val attrs = resolvedMentionAttrs(mentions.trigger, suggestion)
    if (mentions.resolveSelectionAttrs || mentions.resolveTheme) {
        emitMentionSelectRequest(mentions.trigger, suggestion, attrs, queryState, preflightUpdateJSON)
        lastMentionEventJson = null
        clearMentionQueryState()
        return
    }
    val docJson = JSONObject()
        .put("type", "doc")
        .put(
            "content",
            JSONArray().put(
                JSONObject()
                    .put("type", "mention")
                    .put("attrs", attrs)
            )
        )

    val updateJson = richTextView.editorEditText.v2Driver?.insertContentJsonAtSelection(
        docJson.toString(),
        queryState.anchor,
        queryState.head
    )
    if (updateJson != null) {
        richTextView.editorEditText.applyUpdateJSON(updateJson)
    }
    emitMentionSelect(mentions.trigger, suggestion, attrs)
    lastMentionEventJson = null
    clearMentionQueryState()
}
