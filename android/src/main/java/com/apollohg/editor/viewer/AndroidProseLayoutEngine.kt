package com.apollohg.editor.viewer

import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import com.apollohg.editor.ProseViewerError
import kotlin.math.max

/** Creates immutable StaticLayout fragments without depending on a mounted View. */
internal interface AndroidProseLayoutEngine {
    fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout
}

internal class StaticLayoutAndroidProseLayoutEngine : AndroidProseLayoutEngine {
    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout {
        if (widthPx <= 0 || !density.isFinite() || density <= 0f) {
            return PreparedProseLayout.error(
                key,
                0,
                ProseViewerError.invalidWidth(),
            )
        }
        if (document.isEmpty && collapsesWhenEmpty) {
            return PreparedProseLayout(key, widthPx, 0, emptyList(), retainedBytes = 0)
        }
        val paint = TextPaint(TextPaint.ANTI_ALIAS_FLAG).apply {
            textSize = 14f * density
        }
        val blocks = ArrayList<PreparedProseBlock>(document.paragraphs.size)
        var top = 0
        document.paragraphs.forEach { paragraph ->
            val text = if (paragraph.isEmpty()) "\u200B" else paragraph
            val layout = StaticLayout.Builder.obtain(text, 0, text.length, paint, widthPx)
                .setAlignment(Layout.Alignment.ALIGN_NORMAL)
                .setIncludePad(false)
                .setBreakStrategy(Layout.BREAK_STRATEGY_HIGH_QUALITY)
                .build()
            val bottom = top + max(0, layout.height)
            blocks += PreparedProseBlock(layout, top, bottom)
            top = bottom
        }
        val retainedBytes = document.retainedBytes + blocks.sumOf { block ->
            (block.layout.text.length * Char.SIZE_BYTES + block.bottomPx - block.topPx).toLong()
        }
        return PreparedProseLayout(key, widthPx, top, blocks.toList(), retainedBytes = retainedBytes)
    }
}
