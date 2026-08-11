package com.apollohg.editor

import org.json.JSONObject

enum class EditorOrderedListNumberingScheme {
    DECIMAL,
    LOWER_ALPHA,
    UPPER_ALPHA,
    LOWER_ROMAN,
    UPPER_ROMAN;

    companion object {
        fun fromJson(value: String): EditorOrderedListNumberingScheme? = when (value) {
            "decimal" -> DECIMAL
            "lowerAlpha" -> LOWER_ALPHA
            "upperAlpha" -> UPPER_ALPHA
            "lowerRoman" -> LOWER_ROMAN
            "upperRoman" -> UPPER_ROMAN
            else -> null
        }
    }
}

data class EditorOrderedListMarkerTheme(
    val schemes: List<EditorOrderedListNumberingScheme> = listOf(
        EditorOrderedListNumberingScheme.DECIMAL
    ),
    val suffix: String = "."
) {
    companion object {
        fun fromJson(json: JSONObject?): EditorOrderedListMarkerTheme? {
            json ?: return null
            val parsedSchemes = json.optJSONArray("schemes")?.let { values ->
                buildList {
                    for (index in 0 until values.length()) {
                        val rawValue = values.opt(index) as? String ?: continue
                        EditorOrderedListNumberingScheme.fromJson(rawValue)?.let(::add)
                    }
                }
            }.orEmpty()

            return EditorOrderedListMarkerTheme(
                schemes = parsedSchemes.ifEmpty {
                    listOf(EditorOrderedListNumberingScheme.DECIMAL)
                },
                suffix = if (json.optString("suffix") == ")") ")" else "."
            )
        }
    }
}

internal object OrderedListMarkerFormatter {
    private const val MAX_INDEX = 4_294_967_295L

    private val romanTable = listOf(
        1_000L to "m",
        900L to "cm",
        500L to "d",
        400L to "cd",
        100L to "c",
        90L to "xc",
        50L to "l",
        40L to "xl",
        10L to "x",
        9L to "ix",
        5L to "v",
        4L to "iv",
        1L to "i"
    )

    fun label(
        index: Long,
        nestingDepth: Int,
        theme: EditorOrderedListMarkerTheme?
    ): String {
        val resolvedTheme = theme ?: EditorOrderedListMarkerTheme()
        val schemes = resolvedTheme.schemes.ifEmpty {
            listOf(EditorOrderedListNumberingScheme.DECIMAL)
        }
        val scheme = schemes[nestingDepth.coerceAtLeast(0) % schemes.size]
        val suffix = if (resolvedTheme.suffix == ")") ")" else "."
        return formattedIndex(index, scheme) + suffix
    }

    private fun formattedIndex(
        index: Long,
        scheme: EditorOrderedListNumberingScheme
    ): String {
        if (index !in 0..MAX_INDEX) return index.toString()

        return when (scheme) {
            EditorOrderedListNumberingScheme.DECIMAL -> index.toString()
            EditorOrderedListNumberingScheme.LOWER_ALPHA -> alphabeticIndex(index) ?: index.toString()
            EditorOrderedListNumberingScheme.UPPER_ALPHA ->
                alphabeticIndex(index)?.uppercase() ?: index.toString()
            EditorOrderedListNumberingScheme.LOWER_ROMAN -> romanIndex(index) ?: index.toString()
            EditorOrderedListNumberingScheme.UPPER_ROMAN ->
                romanIndex(index)?.uppercase() ?: index.toString()
        }
    }

    private fun alphabeticIndex(index: Long): String? {
        if (index == 0L) return null

        var value = index
        val result = StringBuilder()
        while (value > 0) {
            val offset = ((value - 1) % 26).toInt()
            result.append(('a'.code + offset).toChar())
            value = (value - 1) / 26
        }
        return result.reverse().toString()
    }

    private fun romanIndex(index: Long): String? {
        if (index !in 1..3_999) return null

        var value = index
        return buildString {
            for ((unit, symbol) in romanTable) {
                while (value >= unit) {
                    append(symbol)
                    value -= unit
                }
            }
        }
    }
}
