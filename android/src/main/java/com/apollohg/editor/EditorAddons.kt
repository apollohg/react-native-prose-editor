package com.apollohg.editor

import org.json.JSONObject

data class NativeMentionSuggestion(
    val key: String,
    val title: String,
    val subtitle: String?,
    val label: String,
    val attrs: JSONObject
) {
    companion object {
        fun fromJson(json: JSONObject?): NativeMentionSuggestion? {
            json ?: return null
            val key = json.optString("key", "")
            val title = json.optString("title", "")
            val label = json.optString("label", "")
            if (key.isBlank() || title.isBlank() || label.isBlank()) return null
            return NativeMentionSuggestion(
                key = key,
                title = title,
                subtitle = json.takeUnless { it.isNull("subtitle") }?.optString("subtitle"),
                label = label,
                attrs = json.optJSONObject("attrs") ?: JSONObject()
            )
        }
    }
}

data class NativeMentionsAddonConfig(
    val trigger: String,
    val suggestions: List<NativeMentionSuggestion>,
    val theme: EditorMentionTheme?
) {
    companion object {
        fun fromJson(json: JSONObject?): NativeMentionsAddonConfig? {
            json ?: return null
            val trigger = json.optString("trigger", "@").ifBlank { "@" }
            val rawSuggestions = json.optJSONArray("suggestions")
            val suggestions = mutableListOf<NativeMentionSuggestion>()
            if (rawSuggestions != null) {
                for (index in 0 until rawSuggestions.length()) {
                    val suggestion = NativeMentionSuggestion.fromJson(
                        rawSuggestions.optJSONObject(index)
                    )
                    if (suggestion != null) {
                        suggestions.add(suggestion)
                    }
                }
            }
            return NativeMentionsAddonConfig(
                trigger = trigger,
                suggestions = suggestions,
                theme = EditorMentionTheme.fromJson(json.optJSONObject("theme"))
            )
        }
    }
}

data class NativeEditorAddons(
    val mentions: NativeMentionsAddonConfig?
) {
    companion object {
        fun fromJson(json: String?): NativeEditorAddons {
            if (json.isNullOrBlank()) return NativeEditorAddons(null)
            val root = try {
                JSONObject(json)
            } catch (_: Exception) {
                return NativeEditorAddons(null)
            }
            return NativeEditorAddons(
                mentions = NativeMentionsAddonConfig.fromJson(root.optJSONObject("mentions"))
            )
        }
    }
}

data class MentionQueryState(
    val query: String,
    val trigger: String,
    val anchor: Int,
    val head: Int
)

internal fun isMentionIdentifierCodePoint(codePoint: Int): Boolean {
    return Character.isLetterOrDigit(codePoint) || codePoint == '_'.code || codePoint == '-'.code
}

internal fun resolveMentionQueryState(
    text: String,
    cursorScalar: Int,
    trigger: String,
    isCaretInsideMention: Boolean
): MentionQueryState? {
    if (isCaretInsideMention) return null

    val scalars = text.codePoints().toArray()
    if (cursorScalar > scalars.size) return null
    val triggerCodePoint = trigger.codePointAt(0)

    var start = cursorScalar
    while (start > 0) {
        val previous = scalars[start - 1]
        if (Character.isWhitespace(previous) ||
            previous == '\n'.code ||
            previous == 0xFFFC ||
            (!isMentionIdentifierCodePoint(previous) && previous != triggerCodePoint)
        ) {
            break
        }
        start -= 1
    }

    if (start >= scalars.size || scalars[start] != triggerCodePoint) return null
    if (start > 0) {
        val previous = scalars[start - 1]
        if (isMentionIdentifierCodePoint(previous)) {
            return null
        }
    }

    val query = String(scalars, start + 1, cursorScalar - (start + 1))
    if (query.any { it.isWhitespace() }) return null

    return MentionQueryState(
        query = query,
        trigger = trigger,
        anchor = start,
        head = cursorScalar
    )
}
