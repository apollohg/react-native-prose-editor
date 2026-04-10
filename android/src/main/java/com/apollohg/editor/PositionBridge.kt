package com.apollohg.editor

import android.icu.text.BreakIterator

/**
 * Converts between Android UTF-16 offsets and Rust editor-core scalar offsets,
 * then snaps UTF-16 positions to grapheme boundaries when Android reports a
 * cursor inside a composed character.
 */
object PositionBridge {
    private data class ConversionTable(
        val utf16ToScalar: IntArray,
        val scalarToUtf16: IntArray,
    )

    private val cacheLock = Any()
    @Volatile private var cachedText: String? = null
    @Volatile private var cachedTable: ConversionTable? = null

    /**
     * Counts code points from the start of the string up to the given UTF-16 offset.
     */
    fun utf16ToScalar(utf16Offset: Int, text: String): Int {
        val conversionTable = conversionTableFor(text)
        val endIndex = utf16Offset.coerceIn(0, conversionTable.utf16ToScalar.size - 1)
        return conversionTable.utf16ToScalar[endIndex]
    }

    /**
     * Counts UTF-16 code units from the start of the string up to the given scalar offset.
     */
    fun scalarToUtf16(scalarOffset: Int, text: String): Int {
        if (scalarOffset <= 0) return 0

        val conversionTable = conversionTableFor(text)
        val clampedScalar = scalarOffset.coerceIn(0, conversionTable.scalarToUtf16.size - 1)
        return conversionTable.scalarToUtf16[clampedScalar]
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

    private fun conversionTableFor(text: String): ConversionTable {
        val lastText = cachedText
        val lastTable = cachedTable
        if (lastText == text && lastTable != null) {
            return lastTable
        }

        synchronized(cacheLock) {
            val synchronizedText = cachedText
            val synchronizedTable = cachedTable
            if (synchronizedText == text && synchronizedTable != null) {
                return synchronizedTable
            }

            val scalarCount = text.codePointCount(0, text.length)
            val utf16ToScalar = IntArray(text.length + 1)
            val scalarToUtf16 = IntArray(scalarCount + 1)

            var utf16Pos = 0
            var scalarPos = 0
            while (utf16Pos < text.length) {
                val codePoint = Character.codePointAt(text, utf16Pos)
                val charCount = Character.charCount(codePoint)
                val nextUtf16Pos = utf16Pos + charCount
                scalarPos += 1

                for (offset in (utf16Pos + 1)..nextUtf16Pos) {
                    utf16ToScalar[offset] = scalarPos
                }
                scalarToUtf16[scalarPos] = nextUtf16Pos
                utf16Pos = nextUtf16Pos
            }

            return ConversionTable(
                utf16ToScalar = utf16ToScalar,
                scalarToUtf16 = scalarToUtf16,
            ).also {
                cachedText = text
                cachedTable = it
            }
        }
    }
}
