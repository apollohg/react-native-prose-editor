package com.apollohg.editor

import org.json.JSONObject
import kotlin.math.roundToInt

internal fun physicalToolbarBorderWidth(widthDp: Float, density: Float): Int = when {
    widthDp <= 0f -> 0
    else -> maxOf(1, (widthDp * density).roundToInt())
}

internal data class NativeToolbarState(
    val marks: Map<String, Boolean>,
    val nodes: Map<String, Boolean>,
    val commands: Map<String, Boolean>,
    val allowedMarks: Set<String>,
    val insertableNodes: Set<String>,
    val canUndo: Boolean,
    val canRedo: Boolean
) {
    companion object {
        val empty = NativeToolbarState(
            marks = emptyMap(),
            nodes = emptyMap(),
            commands = emptyMap(),
            allowedMarks = emptySet(),
            insertableNodes = emptySet(),
            canUndo = false,
            canRedo = false
        )

        fun fromUpdateJson(updateJson: String): NativeToolbarState? {
            val root = try {
                JSONObject(updateJson)
            } catch (_: Exception) {
                return null
            }
            val activeState = root.optJSONObject("activeState") ?: JSONObject()
            val historyState = root.optJSONObject("historyState") ?: JSONObject()
            return NativeToolbarState(
                marks = boolMap(activeState.optJSONObject("marks")),
                nodes = boolMap(activeState.optJSONObject("nodes")),
                commands = boolMap(activeState.optJSONObject("commands")),
                allowedMarks = stringSet(activeState.optJSONArray("allowedMarks")),
                insertableNodes = stringSet(activeState.optJSONArray("insertableNodes")),
                canUndo = historyState.optBoolean("canUndo", false),
                canRedo = historyState.optBoolean("canRedo", false)
            )
        }

        private fun boolMap(json: JSONObject?): Map<String, Boolean> {
            json ?: return emptyMap()
            val result = mutableMapOf<String, Boolean>()
            val keys = json.keys()
            while (keys.hasNext()) {
                val key = keys.next()
                result[key] = json.optBoolean(key, false)
            }
            return result
        }

        private fun stringSet(array: org.json.JSONArray?): Set<String> {
            array ?: return emptySet()
            val result = linkedSetOf<String>()
            for (index in 0 until array.length()) {
                array.optString(index, null)?.let { result.add(it) }
            }
            return result
        }
    }
}

internal enum class ToolbarCommand {
    indentList,
    outdentList,
    undo,
    redo,
}

internal object EditorNodeTypes {
    fun listItemType(listType: String): String = when (listType) {
        "bullet_list", "ordered_list" -> "list_item"
        "task_list" -> "task_item"
        "taskList" -> "taskItem"
        else -> "listItem"
    }

    fun isHardBreak(nodeType: String?): Boolean =
        nodeType == "hardBreak" || nodeType == "hard_break"

    fun isHorizontalRule(nodeType: String?): Boolean =
        nodeType == "horizontalRule" || nodeType == "horizontal_rule"

    fun isListItem(nodeType: String): Boolean =
        nodeType == "listItem" || nodeType == "list_item" ||
            nodeType == "taskItem" || nodeType == "task_item"

    fun isListContainer(nodeType: String): Boolean =
        nodeType == "bulletList" || nodeType == "bullet_list" ||
            nodeType == "orderedList" || nodeType == "ordered_list" ||
            nodeType == "taskList" || nodeType == "task_list"

    fun preferredHardBreak(insertableNodes: Set<String>): String =
        if (insertableNodes.contains("hard_break")) "hard_break" else "hardBreak"
}

internal enum class ToolbarListType {
    bullet_list,
    ordered_list,
    bulletList,
    orderedList,
}

internal enum class ToolbarDefaultIconId {
    bold,
    italic,
    underline,
    strike,
    link,
    image,
    h1,
    h2,
    h3,
    h4,
    h5,
    h6,
    blockquote,
    bulletList,
    orderedList,
    indentList,
    outdentList,
    lineBreak,
    horizontalRule,
    undo,
    redo,
}

internal enum class ToolbarItemKind {
    mark,
    heading,
    blockquote,
    list,
    command,
    node,
    action,
    group,
    separator,
}

internal enum class ToolbarGroupPresentation {
    expand,
    menu,
}

internal enum class ToolbarItemPlacement {
    start,
    scroll,
    end,
}
