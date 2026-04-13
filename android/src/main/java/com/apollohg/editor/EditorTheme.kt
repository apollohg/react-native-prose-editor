package com.apollohg.editor

import android.graphics.Color
import android.graphics.Typeface
import org.json.JSONObject

data class EditorTextStyle(
    val fontFamily: String? = null,
    val fontSize: Float? = null,
    val fontWeight: String? = null,
    val fontStyle: String? = null,
    val color: Int? = null,
    val lineHeight: Float? = null,
    val spacingAfter: Float? = null
) {
    companion object {
        fun fromJson(json: JSONObject?): EditorTextStyle? {
            json ?: return null
            return EditorTextStyle(
                fontFamily = json.optNullableString("fontFamily"),
                fontSize = json.optNullableFloat("fontSize"),
                fontWeight = json.optNullableString("fontWeight"),
                fontStyle = json.optNullableString("fontStyle"),
                color = parseColor(json.optNullableString("color")),
                lineHeight = json.optNullableFloat("lineHeight"),
                spacingAfter = json.optNullableFloat("spacingAfter")
            )
        }
    }

    fun mergedWith(other: EditorTextStyle?): EditorTextStyle {
        other ?: return this
        return copy(
            fontFamily = other.fontFamily ?: fontFamily,
            fontSize = other.fontSize ?: fontSize,
            fontWeight = other.fontWeight ?: fontWeight,
            fontStyle = other.fontStyle ?: fontStyle,
            color = other.color ?: color,
            lineHeight = other.lineHeight ?: lineHeight,
            spacingAfter = other.spacingAfter ?: spacingAfter
        )
    }

    fun typefaceStyle(): Int {
        val bold = fontWeight == "bold" || fontWeight?.toIntOrNull()?.let { it >= 600 } == true
        val italic = fontStyle == "italic"
        return when {
            bold && italic -> Typeface.BOLD_ITALIC
            bold -> Typeface.BOLD
            italic -> Typeface.ITALIC
            else -> Typeface.NORMAL
        }
    }
}

data class EditorListTheme(
    val indent: Float? = null,
    val itemSpacing: Float? = null,
    val markerColor: Int? = null,
    val markerScale: Float? = null
) {
    companion object {
        fun fromJson(json: JSONObject?): EditorListTheme? {
            json ?: return null
            return EditorListTheme(
                indent = json.optNullableFloat("indent"),
                itemSpacing = json.optNullableFloat("itemSpacing"),
                markerColor = parseColor(json.optNullableString("markerColor")),
                markerScale = json.optNullableFloat("markerScale")
            )
        }
    }
}

data class EditorHorizontalRuleTheme(
    val color: Int? = null,
    val thickness: Float? = null,
    val verticalMargin: Float? = null
) {
    companion object {
        fun fromJson(json: JSONObject?): EditorHorizontalRuleTheme? {
            json ?: return null
            return EditorHorizontalRuleTheme(
                color = parseColor(json.optNullableString("color")),
                thickness = json.optNullableFloat("thickness"),
                verticalMargin = json.optNullableFloat("verticalMargin")
            )
        }
    }
}

data class EditorBlockquoteTheme(
    val text: EditorTextStyle? = null,
    val indent: Float? = null,
    val borderColor: Int? = null,
    val borderWidth: Float? = null,
    val markerGap: Float? = null
) {
    companion object {
        fun fromJson(json: JSONObject?): EditorBlockquoteTheme? {
            json ?: return null
            return EditorBlockquoteTheme(
                text = EditorTextStyle.fromJson(json.optJSONObject("text")),
                indent = json.optNullableFloat("indent"),
                borderColor = parseColor(json.optNullableString("borderColor")),
                borderWidth = json.optNullableFloat("borderWidth"),
                markerGap = json.optNullableFloat("markerGap")
            )
        }
    }
}

data class EditorMentionTheme(
    val textColor: Int? = null,
    val backgroundColor: Int? = null,
    val borderColor: Int? = null,
    val borderWidth: Float? = null,
    val borderRadius: Float? = null,
    val fontWeight: String? = null,
    val popoverBackgroundColor: Int? = null,
    val popoverBorderColor: Int? = null,
    val popoverBorderWidth: Float? = null,
    val popoverBorderRadius: Float? = null,
    val popoverShadowColor: Int? = null,
    val optionTextColor: Int? = null,
    val optionSecondaryTextColor: Int? = null,
    val optionHighlightedBackgroundColor: Int? = null,
    val optionHighlightedTextColor: Int? = null
) {
    fun mergedWith(other: EditorMentionTheme?): EditorMentionTheme {
        other ?: return this
        return copy(
            textColor = other.textColor ?: textColor,
            backgroundColor = other.backgroundColor ?: backgroundColor,
            borderColor = other.borderColor ?: borderColor,
            borderWidth = other.borderWidth ?: borderWidth,
            borderRadius = other.borderRadius ?: borderRadius,
            fontWeight = other.fontWeight ?: fontWeight,
            popoverBackgroundColor = other.popoverBackgroundColor ?: popoverBackgroundColor,
            popoverBorderColor = other.popoverBorderColor ?: popoverBorderColor,
            popoverBorderWidth = other.popoverBorderWidth ?: popoverBorderWidth,
            popoverBorderRadius = other.popoverBorderRadius ?: popoverBorderRadius,
            popoverShadowColor = other.popoverShadowColor ?: popoverShadowColor,
            optionTextColor = other.optionTextColor ?: optionTextColor,
            optionSecondaryTextColor = other.optionSecondaryTextColor ?: optionSecondaryTextColor,
            optionHighlightedBackgroundColor =
                other.optionHighlightedBackgroundColor ?: optionHighlightedBackgroundColor,
            optionHighlightedTextColor =
                other.optionHighlightedTextColor ?: optionHighlightedTextColor
        )
    }

    companion object {
        fun fromJson(json: JSONObject?): EditorMentionTheme? {
            json ?: return null
            return EditorMentionTheme(
                textColor = parseColor(json.optNullableString("textColor")),
                backgroundColor = parseColor(json.optNullableString("backgroundColor")),
                borderColor = parseColor(json.optNullableString("borderColor")),
                borderWidth = json.optNullableFloat("borderWidth"),
                borderRadius = json.optNullableFloat("borderRadius"),
                fontWeight = json.optNullableString("fontWeight"),
                popoverBackgroundColor = parseColor(json.optNullableString("popoverBackgroundColor")),
                popoverBorderColor = parseColor(json.optNullableString("popoverBorderColor")),
                popoverBorderWidth = json.optNullableFloat("popoverBorderWidth"),
                popoverBorderRadius = json.optNullableFloat("popoverBorderRadius"),
                popoverShadowColor = parseColor(json.optNullableString("popoverShadowColor")),
                optionTextColor = parseColor(json.optNullableString("optionTextColor")),
                optionSecondaryTextColor = parseColor(json.optNullableString("optionSecondaryTextColor")),
                optionHighlightedBackgroundColor = parseColor(json.optNullableString("optionHighlightedBackgroundColor")),
                optionHighlightedTextColor = parseColor(json.optNullableString("optionHighlightedTextColor"))
            )
        }
    }
}

enum class EditorToolbarAppearance {
    CUSTOM,
    NATIVE;

    companion object {
        fun fromRaw(raw: String?): EditorToolbarAppearance? =
            when (raw?.trim()?.lowercase()) {
                "custom" -> CUSTOM
                "native" -> NATIVE
                else -> null
            }
    }
}

data class EditorToolbarTheme(
    val appearance: EditorToolbarAppearance? = null,
    val backgroundColor: Int? = null,
    val borderColor: Int? = null,
    val borderWidth: Float? = null,
    val borderRadius: Float? = null,
    val keyboardOffset: Float? = null,
    val horizontalInset: Float? = null,
    val separatorColor: Int? = null,
    val buttonColor: Int? = null,
    val buttonActiveColor: Int? = null,
    val buttonDisabledColor: Int? = null,
    val buttonActiveBackgroundColor: Int? = null,
    val buttonBorderRadius: Float? = null
) {
    fun resolvedKeyboardOffset(): Float = keyboardOffset ?: if (appearance == EditorToolbarAppearance.NATIVE) 8f else 0f

    fun resolvedHorizontalInset(): Float = horizontalInset ?: if (appearance == EditorToolbarAppearance.NATIVE) 0f else 0f

    fun resolvedBorderRadius(): Float = if (appearance == EditorToolbarAppearance.NATIVE) 32f else (borderRadius ?: 0f)

    fun resolvedBorderWidth(): Float = borderWidth ?: if (appearance == EditorToolbarAppearance.NATIVE) 0f else 1f

    fun resolvedButtonBorderRadius(): Float = if (appearance == EditorToolbarAppearance.NATIVE) 20f else (buttonBorderRadius ?: 6f)

    companion object {
        fun fromJson(json: JSONObject?): EditorToolbarTheme? {
            json ?: return null
            return EditorToolbarTheme(
                appearance = EditorToolbarAppearance.fromRaw(json.optNullableString("appearance")),
                backgroundColor = parseColor(json.optNullableString("backgroundColor")),
                borderColor = parseColor(json.optNullableString("borderColor")),
                borderWidth = json.optNullableFloat("borderWidth"),
                borderRadius = json.optNullableFloat("borderRadius"),
                keyboardOffset = json.optNullableFloat("keyboardOffset"),
                horizontalInset = json.optNullableFloat("horizontalInset"),
                separatorColor = parseColor(json.optNullableString("separatorColor")),
                buttonColor = parseColor(json.optNullableString("buttonColor")),
                buttonActiveColor = parseColor(json.optNullableString("buttonActiveColor")),
                buttonDisabledColor = parseColor(json.optNullableString("buttonDisabledColor")),
                buttonActiveBackgroundColor = parseColor(json.optNullableString("buttonActiveBackgroundColor")),
                buttonBorderRadius = json.optNullableFloat("buttonBorderRadius")
            )
        }
    }
}

data class EditorContentInsets(
    val top: Float? = null,
    val right: Float? = null,
    val bottom: Float? = null,
    val left: Float? = null
) {
    companion object {
        fun fromJson(json: JSONObject?): EditorContentInsets? {
            json ?: return null
            return EditorContentInsets(
                top = json.optNullableFloat("top"),
                right = json.optNullableFloat("right"),
                bottom = json.optNullableFloat("bottom"),
                left = json.optNullableFloat("left")
            )
        }
    }
}

data class EditorTheme(
    val text: EditorTextStyle? = null,
    val paragraph: EditorTextStyle? = null,
    val blockquote: EditorBlockquoteTheme? = null,
    val headings: Map<String, EditorTextStyle> = emptyMap(),
    val list: EditorListTheme? = null,
    val horizontalRule: EditorHorizontalRuleTheme? = null,
    val mentions: EditorMentionTheme? = null,
    val toolbar: EditorToolbarTheme? = null,
    val backgroundColor: Int? = null,
    val borderRadius: Float? = null,
    val contentInsets: EditorContentInsets? = null
) {
    companion object {
        fun fromJson(json: String?): EditorTheme? {
            if (json.isNullOrBlank()) return null
            val root = try {
                JSONObject(json)
            } catch (_: Exception) {
                return null
            }

            val headings = mutableMapOf<String, EditorTextStyle>()
            for (level in listOf("h1", "h2", "h3", "h4", "h5", "h6")) {
                val style = EditorTextStyle.fromJson(root.optJSONObject("headings")?.optJSONObject(level))
                if (style != null) {
                    headings[level] = style
                }
            }

            return EditorTheme(
                text = EditorTextStyle.fromJson(root.optJSONObject("text")),
                paragraph = EditorTextStyle.fromJson(root.optJSONObject("paragraph")),
                blockquote = EditorBlockquoteTheme.fromJson(root.optJSONObject("blockquote")),
                headings = headings,
                list = EditorListTheme.fromJson(root.optJSONObject("list")),
                horizontalRule = EditorHorizontalRuleTheme.fromJson(root.optJSONObject("horizontalRule")),
                mentions = EditorMentionTheme.fromJson(root.optJSONObject("mentions")),
                toolbar = EditorToolbarTheme.fromJson(root.optJSONObject("toolbar")),
                backgroundColor = parseColor(root.optNullableString("backgroundColor")),
                borderRadius = root.optNullableFloat("borderRadius"),
                contentInsets = EditorContentInsets.fromJson(root.optJSONObject("contentInsets"))
            )
        }
    }

    fun effectiveTextStyle(nodeType: String, inBlockquote: Boolean = false): EditorTextStyle {
        var style = text ?: EditorTextStyle()
        style = style.mergedWith(if (inBlockquote) blockquote?.text else null)
        if (nodeType == "paragraph") {
            style = style.mergedWith(paragraph)
            if (paragraph?.lineHeight == null) {
                style = style.copy(lineHeight = null)
            }
        }
        style = style.mergedWith(headings[nodeType])
        return style
    }
}

private fun parseColor(raw: String?): Int? {
    val value = raw?.trim()?.lowercase() ?: return null
    if (value.isEmpty()) return null

    parseCssHexColor(value)?.let { return it }

    try {
        return Color.parseColor(value)
    } catch (_: IllegalArgumentException) {
        // Fall through to rgb()/rgba() parsing.
    }

    return when {
        value.startsWith("rgb(") && value.endsWith(")") -> {
            val parts = value.removePrefix("rgb(").removeSuffix(")")
                .split(',')
                .map { it.trim() }
            if (parts.size != 3) return null
            val red = parts[0].toDoubleOrNull() ?: return null
            val green = parts[1].toDoubleOrNull() ?: return null
            val blue = parts[2].toDoubleOrNull() ?: return null
            Color.argb(255, red.toInt(), green.toInt(), blue.toInt())
        }
        value.startsWith("rgba(") && value.endsWith(")") -> {
            val parts = value.removePrefix("rgba(").removeSuffix(")")
                .split(',')
                .map { it.trim() }
            if (parts.size != 4) return null
            val red = parts[0].toDoubleOrNull() ?: return null
            val green = parts[1].toDoubleOrNull() ?: return null
            val blue = parts[2].toDoubleOrNull() ?: return null
            val alpha = parts[3].toDoubleOrNull() ?: return null
            Color.argb((alpha * 255f).toInt(), red.toInt(), green.toInt(), blue.toInt())
        }
        else -> null
    }
}

private fun parseCssHexColor(value: String): Int? {
    if (!value.startsWith("#")) return null
    val hex = value.removePrefix("#")

    return when (hex.length) {
        3 -> {
            val red = "${hex[0]}${hex[0]}".toIntOrNull(16) ?: return null
            val green = "${hex[1]}${hex[1]}".toIntOrNull(16) ?: return null
            val blue = "${hex[2]}${hex[2]}".toIntOrNull(16) ?: return null
            Color.argb(255, red, green, blue)
        }
        4 -> {
            val red = "${hex[0]}${hex[0]}".toIntOrNull(16) ?: return null
            val green = "${hex[1]}${hex[1]}".toIntOrNull(16) ?: return null
            val blue = "${hex[2]}${hex[2]}".toIntOrNull(16) ?: return null
            val alpha = "${hex[3]}${hex[3]}".toIntOrNull(16) ?: return null
            Color.argb(alpha, red, green, blue)
        }
        6 -> {
            val red = hex.substring(0, 2).toIntOrNull(16) ?: return null
            val green = hex.substring(2, 4).toIntOrNull(16) ?: return null
            val blue = hex.substring(4, 6).toIntOrNull(16) ?: return null
            Color.argb(255, red, green, blue)
        }
        8 -> {
            val red = hex.substring(0, 2).toIntOrNull(16) ?: return null
            val green = hex.substring(2, 4).toIntOrNull(16) ?: return null
            val blue = hex.substring(4, 6).toIntOrNull(16) ?: return null
            val alpha = hex.substring(6, 8).toIntOrNull(16) ?: return null
            Color.argb(alpha, red, green, blue)
        }
        else -> null
    }
}

private fun JSONObject.optNullableString(key: String): String? {
    if (!has(key) || isNull(key)) return null
    return optString(key).takeUnless { it == "null" }
}

private fun JSONObject.optNullableFloat(key: String): Float? {
    if (!has(key) || isNull(key)) return null
    return optDouble(key).takeIf { !it.isNaN() }?.toFloat()
}
