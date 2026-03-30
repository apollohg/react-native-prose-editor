package com.apollohg.editor

import android.icu.text.BreakIterator

/**
 * Converts between Android UTF-16 offsets and Rust editor-core scalar offsets,
 * then snaps UTF-16 positions to grapheme boundaries when Android reports a
 * cursor inside a composed character.
 */
object PositionBridge {

    /**
     * Counts code points from the start of the string up to the given UTF-16 offset.
     */
    fun utf16ToScalar(utf16Offset: Int, text: String): Int {
        if (utf16Offset <= 0) return 0

        val endIndex = minOf(utf16Offset, text.length)
        var scalarCount = 0
        var utf16Pos = 0

        while (utf16Pos < endIndex) {
            val codePoint = Character.codePointAt(text, utf16Pos)
            val charCount = Character.charCount(codePoint)
            utf16Pos += charCount
            scalarCount++
        }

        return scalarCount
    }

    /**
     * Counts UTF-16 code units from the start of the string up to the given scalar offset.
     */
    fun scalarToUtf16(scalarOffset: Int, text: String): Int {
        if (scalarOffset <= 0) return 0

        var utf16Len = 0
        var scalarsSeen = 0

        var i = 0
        while (i < text.length && scalarsSeen < scalarOffset) {
            val codePoint = Character.codePointAt(text, i)
            val charCount = Character.charCount(codePoint)
            utf16Len += charCount
            scalarsSeen++
            i += charCount
        }

        return utf16Len
    }

    /**
     * Biases forward to the next grapheme boundary when Android reports an
     * offset inside a composed character sequence.
     */
    fun snapToGraphemeBoundary(utf16Offset: Int, text: String): Int {
        if (text.isEmpty()) return 0

        val clampedOffset = utf16Offset.coerceIn(0, text.length)

        if (clampedOffset == 0 || clampedOffset == text.length) {
            return clampedOffset
        }

        val breakIterator = BreakIterator.getCharacterInstance()
        breakIterator.setText(text)

        if (breakIterator.isBoundary(clampedOffset)) {
            return clampedOffset
        }

        val nextBoundary = breakIterator.following(clampedOffset)
        return if (nextBoundary == BreakIterator.DONE) text.length else nextBoundary
    }
}
