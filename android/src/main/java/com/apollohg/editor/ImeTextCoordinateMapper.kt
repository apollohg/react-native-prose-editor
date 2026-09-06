package com.apollohg.editor

import android.text.NoCopySpan
import android.text.Annotation
import android.text.SpannableString
import android.text.Selection
import android.text.Spanned

internal class ImeTextCoordinateMapper private constructor(
    val visibleText: CharSequence,
    val generation: Long,
    private val rawToImeOffsets: IntArray,
    private val imeToRawBeforeOffsets: IntArray,
    private val imeToRawAfterOffsets: IntArray,
) {
    enum class Affinity {
        BEFORE,
        AFTER,
    }

    fun rawToIme(offset: Int): Int =
        rawToImeOffsets[offset.coerceIn(0, rawToImeOffsets.lastIndex)]

    fun imeToRaw(offset: Int, affinity: Affinity): Int {
        val clamped = offset.coerceIn(0, imeToRawBeforeOffsets.lastIndex)
        return when (affinity) {
            Affinity.BEFORE -> imeToRawBeforeOffsets[clamped]
            Affinity.AFTER -> imeToRawAfterOffsets[clamped]
        }
    }

    companion object {
        fun build(raw: CharSequence, generation: Long): ImeTextCoordinateMapper {
            val rawToIme = IntArray(raw.length + 1)
            val visible = StringBuilder(raw.length)
            val generatedMarkers = BooleanArray(raw.length)
            if (raw is Spanned) {
                raw.getSpans(0, raw.length, Annotation::class.java)
                    .filter { it.key == RenderBridge.NATIVE_LIST_MARKER_ANNOTATION }
                    .forEach { marker ->
                        for (offset in raw.getSpanStart(marker) until raw.getSpanEnd(marker)) {
                            generatedMarkers[offset] = true
                        }
                    }
            }
            var visibleOffset = 0
            rawToIme[0] = 0
            for (rawOffset in raw.indices) {
                if (!isInvisiblePlaceholder(raw[rawOffset]) && !generatedMarkers[rawOffset]) {
                    visible.append(raw[rawOffset])
                    visibleOffset += 1
                }
                rawToIme[rawOffset + 1] = visibleOffset
            }

            val imeToRawBefore = IntArray(visibleOffset + 1) { Int.MAX_VALUE }
            val imeToRawAfter = IntArray(visibleOffset + 1) { -1 }
            for (rawOffset in rawToIme.indices) {
                val imeOffset = rawToIme[rawOffset]
                imeToRawBefore[imeOffset] = minOf(imeToRawBefore[imeOffset], rawOffset)
                imeToRawAfter[imeOffset] = maxOf(imeToRawAfter[imeOffset], rawOffset)
            }

            val visibleText = if (raw is Spanned) {
                SpannableString(visible.toString()).apply {
                    raw.getSpans(0, raw.length, Any::class.java).forEach { span ->
                        val rawStart = raw.getSpanStart(span)
                        val rawEnd = raw.getSpanEnd(span)
                        val flags = raw.getSpanFlags(span)
                        if (
                            rawStart >= 0 &&
                            rawEnd >= rawStart &&
                            span !is NoCopySpan &&
                            span !== Selection.SELECTION_START &&
                            span !== Selection.SELECTION_END &&
                            flags and Spanned.SPAN_COMPOSING == 0
                        ) {
                            setSpan(
                                span,
                                rawToIme[rawStart.coerceIn(0, raw.length)],
                                rawToIme[rawEnd.coerceIn(0, raw.length)],
                                flags,
                            )
                        }
                    }
                }
            } else {
                visible.toString()
            }

            return ImeTextCoordinateMapper(
                visibleText = visibleText,
                generation = generation,
                rawToImeOffsets = rawToIme,
                imeToRawBeforeOffsets = imeToRawBefore,
                imeToRawAfterOffsets = imeToRawAfter,
            )
        }

        private fun isInvisiblePlaceholder(character: Char): Boolean =
            character == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]
    }
}
