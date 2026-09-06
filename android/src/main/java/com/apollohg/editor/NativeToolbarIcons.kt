package com.apollohg.editor

import android.content.Context
import android.graphics.Typeface
import org.json.JSONObject

internal data class NativeToolbarIcon(
    val defaultId: ToolbarDefaultIconId? = null,
    val glyphText: String? = null,
    val fallbackText: String? = null,
    val materialIconName: String? = null
) {
    companion object {
        private val defaultGlyphs = mapOf(
            ToolbarDefaultIconId.bold to "B",
            ToolbarDefaultIconId.italic to "I",
            ToolbarDefaultIconId.underline to "U",
            ToolbarDefaultIconId.strike to "S",
            ToolbarDefaultIconId.link to "🔗",
            ToolbarDefaultIconId.image to "🖼",
            ToolbarDefaultIconId.h1 to "H1",
            ToolbarDefaultIconId.h2 to "H2",
            ToolbarDefaultIconId.h3 to "H3",
            ToolbarDefaultIconId.h4 to "H4",
            ToolbarDefaultIconId.h5 to "H5",
            ToolbarDefaultIconId.h6 to "H6",
            ToolbarDefaultIconId.blockquote to "❝",
            ToolbarDefaultIconId.bulletList to "•≡",
            ToolbarDefaultIconId.orderedList to "1.",
            ToolbarDefaultIconId.indentList to "→",
            ToolbarDefaultIconId.outdentList to "←",
            ToolbarDefaultIconId.lineBreak to "↵",
            ToolbarDefaultIconId.horizontalRule to "—",
            ToolbarDefaultIconId.undo to "↩",
            ToolbarDefaultIconId.redo to "↪"
        )
        private val defaultMaterialIcons = mapOf(
            ToolbarDefaultIconId.bold to "format-bold",
            ToolbarDefaultIconId.italic to "format-italic",
            ToolbarDefaultIconId.underline to "format-underlined",
            ToolbarDefaultIconId.strike to "strikethrough-s",
            ToolbarDefaultIconId.link to "link",
            ToolbarDefaultIconId.image to "image",
            ToolbarDefaultIconId.blockquote to "format-quote",
            ToolbarDefaultIconId.bulletList to "format-list-bulleted",
            ToolbarDefaultIconId.orderedList to "format-list-numbered",
            ToolbarDefaultIconId.indentList to "format-indent-increase",
            ToolbarDefaultIconId.outdentList to "format-indent-decrease",
            ToolbarDefaultIconId.lineBreak to "keyboard-return",
            ToolbarDefaultIconId.horizontalRule to "horizontal-rule",
            ToolbarDefaultIconId.h1 to "title",
            ToolbarDefaultIconId.h2 to "title",
            ToolbarDefaultIconId.h3 to "title",
            ToolbarDefaultIconId.h4 to "title",
            ToolbarDefaultIconId.h5 to "title",
            ToolbarDefaultIconId.h6 to "title",
            ToolbarDefaultIconId.undo to "undo",
            ToolbarDefaultIconId.redo to "redo"
        )

        fun fromJson(raw: JSONObject?): NativeToolbarIcon? {
            raw ?: return null
            return when (raw.optString("type")) {
                "default" -> {
                    val id = runCatching {
                        ToolbarDefaultIconId.valueOf(raw.getString("id"))
                    }.getOrNull() ?: return null
                    NativeToolbarIcon(defaultId = id)
                }
                "glyph" -> {
                    val text = raw.optString("text")
                    if (text.isBlank()) null else NativeToolbarIcon(glyphText = text)
                }
                "platform" -> {
                    val materialName = raw.optJSONObject("android")
                        ?.takeIf { it.optString("type") == "material" }
                        ?.toolbarNullableString("name")
                    val fallback = raw.toolbarNullableString("fallbackText")
                    if (materialName.isNullOrBlank() && fallback.isNullOrBlank()) {
                        null
                    } else {
                        NativeToolbarIcon(
                            fallbackText = fallback,
                            materialIconName = materialName
                        )
                    }
                }
                else -> null
            }
        }

        fun defaultMaterialIconName(defaultId: ToolbarDefaultIconId?): String? =
            defaultId?.let { defaultMaterialIcons[it] }
    }

    fun resolvedGlyphText(): String =
        glyphText?.takeIf { it.isNotBlank() }
            ?: fallbackText?.takeIf { it.isNotBlank() }
            ?: defaultId?.let { defaultGlyphs[it] }
            ?: "?"

    fun resolvedMaterialIconName(): String? =
        materialIconName?.takeIf { it.isNotBlank() }
            ?: Companion.defaultMaterialIconName(defaultId)
}

internal object MaterialIconRegistry {
    private const val FONT_ASSET_PATH = "editor-icons/MaterialIcons.ttf"
    private const val GLYPHMAP_ASSET_PATH = "editor-icons/MaterialIcons.json"

    @Volatile
    private var typeface: Typeface? = null

    @Volatile
    private var glyphMap: Map<String, String>? = null

    fun typeface(context: Context): Typeface? {
        val cached = typeface
        if (cached != null) return cached
        return runCatching {
            Typeface.createFromAsset(context.assets, FONT_ASSET_PATH)
        }.getOrNull()?.also { loaded ->
            typeface = loaded
        }
    }

    fun glyphForName(context: Context, name: String?): String? {
        if (name.isNullOrBlank()) return null
        val map = glyphMap ?: loadGlyphMap(context).also { loaded ->
            glyphMap = loaded
        }
        return map[name]
    }

    private fun loadGlyphMap(context: Context): Map<String, String> {
        val assetText = runCatching {
            context.assets.open(GLYPHMAP_ASSET_PATH).bufferedReader().use { it.readText() }
        }.getOrNull() ?: return emptyMap()

        val json = runCatching { JSONObject(assetText) }.getOrNull() ?: return emptyMap()
        val result = linkedMapOf<String, String>()
        val keys = json.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            val codePoint = json.optInt(key, -1)
            if (codePoint > 0) {
                result[key] = String(Character.toChars(codePoint))
            }
        }
        return result
    }
}

internal data class NativeToolbarResolvedIcon(
    val text: String,
    val typeface: Typeface? = null
)

internal fun NativeToolbarIcon.resolveForAndroid(context: Context): NativeToolbarResolvedIcon {
    val materialName = resolvedMaterialIconName()
    val materialGlyph = MaterialIconRegistry.glyphForName(context, materialName)
    val materialTypeface = MaterialIconRegistry.typeface(context)
    if (materialGlyph != null && materialTypeface != null) {
        return NativeToolbarResolvedIcon(
            text = materialGlyph,
            typeface = materialTypeface
        )
    }

    return NativeToolbarResolvedIcon(
        text = resolvedGlyphText(),
        typeface = null
    )
}

internal fun JSONObject.toolbarNullableString(key: String): String? {
    if (!has(key) || isNull(key)) return null
    return optString(key).takeUnless { it == "null" }
}
