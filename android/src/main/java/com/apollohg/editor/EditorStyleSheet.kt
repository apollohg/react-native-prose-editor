package com.apollohg.editor

import org.json.JSONObject

data class EditorEdges(val top: Float = 0f, val right: Float = 0f, val bottom: Float = 0f, val left: Float = 0f) {
    fun scaled(density: Float) = EditorEdges(top * density, right * density, bottom * density, left * density)
    operator fun plus(other: EditorEdges) = EditorEdges(top + other.top, right + other.right, bottom + other.bottom, left + other.left)
}

data class EditorCorners(val topLeft: Float = 0f, val topRight: Float = 0f, val bottomRight: Float = 0f, val bottomLeft: Float = 0f) {
    fun scaled(density: Float) = EditorCorners(topLeft * density, topRight * density, bottomRight * density, bottomLeft * density)
}

data class EditorBoxStyle(
    val backgroundColor: Int? = null,
    val padding: EditorEdges = EditorEdges(),
    val margin: EditorEdges = EditorEdges(),
    val border: EditorEdges = EditorEdges(),
    val borderColors: List<Int> = List(4) { android.graphics.Color.BLACK },
    val corners: EditorCorners = EditorCorners(),
    val borderStyle: String = "solid",
) {
    val inset: EditorEdges get() = padding + border
    val outerInset: EditorEdges get() = inset + margin
    fun scaled(density: Float) = copy(padding = padding.scaled(density), margin = margin.scaled(density), border = border.scaled(density), corners = corners.scaled(density))
}

data class EditorElementStyle(
    val text: EditorTextStyle,
    val box: EditorBoxStyle,
    val indent: Float? = null,
    val baseIndentMultiplier: Float? = null,
    val scale: Float? = null,
    val gap: Float? = null,
    val ordered: EditorOrderedListMarkerTheme? = null,
    val checked: EditorElementStyle? = null,
    val resizeMode: String = "contain",
    val size: Float? = null,
    val checkColor: Int? = null,
    val height: Float? = null,
    val declaredProperties: Set<String> = emptySet(),
)

class EditorStyleSheet private constructor(val styles: Map<String, EditorElementStyle>) {
    operator fun get(element: String): EditorElementStyle? = styles[canonicalElement(element)]

    fun resolveText(element: String, ancestors: List<String> = emptyList(), marks: List<String> = emptyList()): EditorTextStyle {
        val name = canonicalElement(element)
        var result = EditorTextStyle().mergedWith(this["text"]?.text)
            .mergedWith(semanticText(name))
        ancestors.forEach { result = result.mergedWith(this[it]?.text?.copy(backgroundColor = null)) }
        result = result.mergedWith(this[name]?.text?.copy(backgroundColor = null))
        val active = marks.map(::canonicalMark).toSet()
        listOf("inlineCode", "bold", "italic", "link", "underline", "strike").filter { it in active }.forEach {
            result = result.mergedWith(semanticText(it)).mergedWith(this[it]?.text)
        }
        return result
    }

    fun box(element: String): EditorBoxStyle = this[element]?.box ?: defaultBox(canonicalElement(element))

    companion object {
        internal fun decodeTheme(root: JSONObject): EditorTheme? {
            if ((root.opt("version") as? Number)?.toDouble() != 1.0 || root.has("styles") && root.optJSONObject("styles") == null) return null
            val values = root.optJSONObject("styles") ?: JSONObject()
            if (values.keys().asSequence().any { values.optJSONObject(it) == null }) return null
            val sheet = EditorStyleSheet(values.keys().asSequence().associateWith { name ->
                decodeElement(values.getJSONObject(name), defaultBox(name))
            })
            val content = sheet.box("content").outerInset
            val marker = sheet["listMarker"]
            val list = sheet["bulletList"]
            return EditorTheme(
                text = sheet["text"]?.text,
                paragraph = sheet["paragraph"]?.text,
                headings = (1..6).associate { "h$it" to sheet.resolveText("h$it") },
                list = EditorListTheme(indent = list?.indent, baseIndentMultiplier = list?.baseIndentMultiplier,
                    itemSpacing = 0f, spacingAfter = 0f, markerColor = marker?.text?.color,
                    markerScale = marker?.scale, markerGap = marker?.gap, orderedMarker = marker?.ordered),
                blockquote = EditorBlockquoteTheme(text = sheet["blockquote"]?.text, indent = 0f, borderWidth = 0f, markerGap = 0f),
                codeBlock = EditorCodeBlockTheme(text = sheet.resolveText("codeBlock"), paddingHorizontal = 0f, paddingVertical = 0f),
                mentions = EditorMentionTheme.fromJson(root.optJSONObject("mentions")),
                horizontalRule = EditorHorizontalRuleTheme(color = sheet.box("horizontalRule").backgroundColor, thickness = sheet["horizontalRule"]?.height ?: 1f, verticalMargin = 0f),
                toolbar = EditorToolbarTheme.fromJson(root.optJSONObject("toolbar")),
                placeholderColor = sheet["placeholder"]?.text?.color,
                backgroundColor = sheet.box("content").backgroundColor,
                contentInsets = EditorContentInsets(content.top, content.right, content.bottom, content.left),
                styleSheet = sheet,
            )
        }

        internal fun decodeElement(json: JSONObject, fallback: EditorBoxStyle = EditorBoxStyle()): EditorElementStyle {
            fun number(key: String, default: Float) = json.optNullableFloat(key) ?: default
            fun edges(prefix: String, defaults: EditorEdges, suffix: String = "") = EditorEdges(
                number("${prefix}Top$suffix", defaults.top), number("${prefix}Right$suffix", defaults.right),
                number("${prefix}Bottom$suffix", defaults.bottom), number("${prefix}Left$suffix", defaults.left),
            )
            val box = EditorBoxStyle(
                backgroundColor = parseColor(json.optNullableString("backgroundColor")) ?: fallback.backgroundColor,
                padding = edges("padding", fallback.padding), margin = edges("margin", fallback.margin),
                border = edges("border", fallback.border, "Width"),
                borderColors = listOf("Top", "Right", "Bottom", "Left").mapIndexed { index, side ->
                    parseColor(json.optNullableString("border${side}Color")) ?: fallback.borderColors[index]
                },
                corners = EditorCorners(number("borderTopLeftRadius", fallback.corners.topLeft), number("borderTopRightRadius", fallback.corners.topRight), number("borderBottomRightRadius", fallback.corners.bottomRight), number("borderBottomLeftRadius", fallback.corners.bottomLeft)),
                borderStyle = json.optNullableString("borderStyle") ?: fallback.borderStyle,
            )
            return EditorElementStyle(
                text = EditorTextStyle.fromJson(json) ?: EditorTextStyle(), box = box,
                indent = json.optNullableFloat("indent"), baseIndentMultiplier = json.optNullableFloat("baseIndentMultiplier"),
                scale = json.optNullableFloat("scale"), gap = json.optNullableFloat("gap"),
                ordered = EditorOrderedListMarkerTheme.fromJson(json.optJSONObject("ordered")),
                checked = json.optJSONObject("checked")?.let { decodeElement(it, box) },
                resizeMode = json.optNullableString("resizeMode") ?: "contain",
                size = json.optNullableFloat("size"), checkColor = parseColor(json.optNullableString("checkColor")),
                height = json.optNullableFloat("height"),
                declaredProperties = json.keys().asSequence().toSet(),
            )
        }
    }
}

internal fun canonicalElement(name: String): String = when (name) {
    "bullet_list" -> "bulletList"
    "ordered_list" -> "orderedList"
    "task_list" -> "taskList"
    "list_item" -> "listItem"
    "task_item" -> "taskItem"
    "horizontal_rule" -> "horizontalRule"
    "code_block" -> "codeBlock"
    else -> name
}

internal fun canonicalMark(name: String): String = when (name) {
    "code" -> "inlineCode"
    "strong" -> "bold"
    "em" -> "italic"
    "strikethrough" -> "strike"
    else -> name
}

internal fun semanticText(name: String): EditorTextStyle = when (name) {
    "h1", "h2", "h3", "h4", "h5", "h6" -> EditorTextStyle(fontSize = listOf(32f, 28f, 24f, 21f, 19f, 17f)[name.last().digitToInt() - 1], fontWeight = "700")
    "codeBlock" -> EditorTextStyle(fontFamily = "monospace")
    "inlineCode" -> EditorTextStyle(fontFamily = "monospace", backgroundColor = LayoutConstants.CODE_BACKGROUND_COLOR)
    "bold" -> EditorTextStyle(fontWeight = "700")
    "italic" -> EditorTextStyle(fontStyle = "italic")
    "link" -> EditorTextStyle(color = LayoutConstants.DEFAULT_LINK_COLOR, textDecorationLine = "underline")
    "underline" -> EditorTextStyle(textDecorationLine = "underline")
    "strike" -> EditorTextStyle(textDecorationLine = "line-through")
    else -> EditorTextStyle()
}

internal fun defaultBox(name: String): EditorBoxStyle = when (name) {
    "codeBlock" -> EditorBoxStyle(backgroundColor = LayoutConstants.CODE_BACKGROUND_COLOR, padding = EditorEdges(8f, 12f, 8f, 12f), corners = EditorCorners(8f, 8f, 8f, 8f))
    "blockquote" -> EditorBoxStyle(padding = EditorEdges(left = 10f), border = EditorEdges(left = 3f), borderColors = List(4) { 0xFFC7C7CC.toInt() })
    "listItem", "taskItem" -> EditorBoxStyle(margin = EditorEdges(bottom = 4f))
    "h1", "h2", "h3", "h4", "h5", "h6" -> EditorBoxStyle(margin = EditorEdges(bottom = 10f))
    "horizontalRule" -> EditorBoxStyle(backgroundColor = 0xFFC7C7CC.toInt(), margin = EditorEdges(top = 12f, bottom = 12f))
    else -> EditorBoxStyle()
}

internal fun mergeDecoration(previous: String?, next: String?): String? = when {
    next == null -> previous
    next == "none" -> "none"
    else -> listOfNotNull(previous?.takeUnless { it == "none" }, next).flatMap { it.split(' ') }.distinct().joinToString(" ")
}
