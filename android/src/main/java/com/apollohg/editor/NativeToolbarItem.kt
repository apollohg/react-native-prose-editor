package com.apollohg.editor

import org.json.JSONObject

internal data class NativeToolbarItem(
    val type: ToolbarItemKind,
    val key: String? = null,
    val label: String? = null,
    val icon: NativeToolbarIcon? = null,
    val mark: String? = null,
    val headingLevel: Int? = null,
    val listType: ToolbarListType? = null,
    val command: ToolbarCommand? = null,
    val nodeType: String? = null,
    val isActive: Boolean = false,
    val isDisabled: Boolean = false,
    val placement: ToolbarItemPlacement? = null,
    val presentation: ToolbarGroupPresentation? = null,
    val items: List<NativeToolbarItem> = emptyList(),
    val buttonStyle: EditorToolbarButtonStyle? = null,
    val parentGroupKey: String? = null
) {
    companion object {
        val defaults = listOf(
            NativeToolbarItem(ToolbarItemKind.mark, label = "Bold", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.bold), mark = "bold"),
            NativeToolbarItem(ToolbarItemKind.mark, label = "Italic", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.italic), mark = "italic"),
            NativeToolbarItem(ToolbarItemKind.mark, label = "Underline", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.underline), mark = "underline"),
            NativeToolbarItem(ToolbarItemKind.mark, label = "Strikethrough", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.strike), mark = "strike"),
            NativeToolbarItem(ToolbarItemKind.blockquote, label = "Blockquote", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.blockquote)),
            NativeToolbarItem(ToolbarItemKind.separator),
            NativeToolbarItem(ToolbarItemKind.list, label = "Bullet List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.bulletList), listType = ToolbarListType.bullet_list),
            NativeToolbarItem(ToolbarItemKind.list, label = "Ordered List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.orderedList), listType = ToolbarListType.ordered_list),
            NativeToolbarItem(ToolbarItemKind.command, label = "Indent List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.indentList), command = ToolbarCommand.indentList),
            NativeToolbarItem(ToolbarItemKind.command, label = "Outdent List", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.outdentList), command = ToolbarCommand.outdentList),
            NativeToolbarItem(ToolbarItemKind.node, label = "Line Break", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.lineBreak), nodeType = "hard_break"),
            NativeToolbarItem(ToolbarItemKind.node, label = "Horizontal Rule", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.horizontalRule), nodeType = "horizontal_rule"),
            NativeToolbarItem(ToolbarItemKind.separator),
            NativeToolbarItem(ToolbarItemKind.command, label = "Undo", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.undo), command = ToolbarCommand.undo),
            NativeToolbarItem(ToolbarItemKind.command, label = "Redo", icon = NativeToolbarIcon(defaultId = ToolbarDefaultIconId.redo), command = ToolbarCommand.redo)
        )

        private fun parseItem(
            rawItem: JSONObject,
            allowGroup: Boolean = true,
            allowSeparator: Boolean = true
        ): NativeToolbarItem? {
            val type = runCatching {
                ToolbarItemKind.valueOf(rawItem.getString("type"))
            }.getOrNull() ?: return null
            val key = rawItem.toolbarNullableString("key")
            val placement = rawItem.toolbarNullableString("placement")?.let {
                runCatching { ToolbarItemPlacement.valueOf(it) }.getOrNull()
            }
            val parsed = when (type) {
                ToolbarItemKind.separator -> {
                    if (!allowSeparator) {
                        null
                    } else {
                        NativeToolbarItem(type = type, key = key, placement = placement)
                    }
                }
                ToolbarItemKind.mark -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val mark = rawItem.toolbarNullableString("mark") ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(type, key, label, icon, mark = mark, placement = placement)
                }
                ToolbarItemKind.heading -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val level = rawItem.optInt("level", -1)
                    if (level !in 1..6) return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(type, key, label, icon, headingLevel = level, placement = placement)
                }
                ToolbarItemKind.blockquote -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(type, key, label, icon, placement = placement)
                }
                ToolbarItemKind.list -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val listType = runCatching {
                        ToolbarListType.valueOf(rawItem.getString("listType"))
                    }.getOrNull() ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(type, key, label, icon, listType = listType, placement = placement)
                }
                ToolbarItemKind.command -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val command = runCatching {
                        ToolbarCommand.valueOf(rawItem.getString("command"))
                    }.getOrNull() ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(type, key, label, icon, command = command, placement = placement)
                }
                ToolbarItemKind.node -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val nodeType = rawItem.toolbarNullableString("nodeType") ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(type, key, label, icon, nodeType = nodeType, placement = placement)
                }
                ToolbarItemKind.action -> {
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val keyValue = rawItem.toolbarNullableString("key") ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    NativeToolbarItem(
                        type = type,
                        key = keyValue,
                        label = label,
                        icon = icon,
                        placement = placement,
                        isActive = rawItem.optBoolean("isActive", false),
                        isDisabled = rawItem.optBoolean("isDisabled", false)
                    )
                }
                ToolbarItemKind.group -> {
                    if (!allowGroup) return null
                    val keyValue = rawItem.toolbarNullableString("key") ?: return null
                    val icon = NativeToolbarIcon.fromJson(rawItem.optJSONObject("icon")) ?: return null
                    val label = rawItem.toolbarNullableString("label") ?: return null
                    val presentation = rawItem.toolbarNullableString("presentation")?.let {
                        runCatching { ToolbarGroupPresentation.valueOf(it) }.getOrNull()
                    } ?: ToolbarGroupPresentation.expand
                    val rawChildren = rawItem.optJSONArray("items") ?: return null
                    val children = mutableListOf<NativeToolbarItem>()
                    for (childIndex in 0 until rawChildren.length()) {
                        val rawChild = rawChildren.optJSONObject(childIndex) ?: continue
                        parseItem(rawChild, allowGroup = false, allowSeparator = false)?.let {
                            children += it
                        }
                    }
                    if (children.isEmpty()) return null
                    NativeToolbarItem(
                        type = type,
                        key = keyValue,
                        label = label,
                        icon = icon,
                        placement = placement,
                        presentation = presentation,
                        items = children
                    )
                }
            }
            return parsed?.copy(
                buttonStyle = EditorToolbarButtonStyle.fromJson(
                    rawItem.optJSONObject("buttonStyle")
                )
            )
        }

        fun fromJson(json: String?): List<NativeToolbarItem> {
            if (json.isNullOrBlank()) return defaults
            val rawArray = try {
                org.json.JSONArray(json)
            } catch (_: Exception) {
                return defaults
            }
            val parsed = mutableListOf<NativeToolbarItem>()
            for (index in 0 until rawArray.length()) {
                val rawItem = rawArray.optJSONObject(index) ?: continue
                parseItem(rawItem)?.let { parsed += it }
            }
            return parsed.ifEmpty { defaults }
        }
    }
}
