package com.apollohg.editor

import android.graphics.Paint
import android.text.Layout
import android.text.Spanned
import android.text.TextPaint
import android.text.style.MetricAffectingSpan

/**
 * Vertical geometry for the text caret, clipped to the rendered glyph height.
 *
 * Android's [android.widget.Editor] draws the native caret from
 * `Layout.editorTextLineTop(line)` to `Layout.editorTextLineBottom(line)`. When a
 * [ParagraphSpacerSpan] inflates a line's descent to create inter-block
 * spacing, `getLineBottom` includes that gap and the caret stretches into it.
 * `getLineBottomWithoutSpacing` cannot help: the inflation lives in the line's
 * DESCENT column, not the line-spacing EXTRA column it subtracts.
 *
 * The baseline is provably independent of descent inflation
 * (`getLineBaseline(line) == getLineTop(line) - ascent`), so anchoring the
 * caret bottom at `baseline + raw font descent` clips it to the glyph height.
 * This mirrors the trim already used for blockquote stripes
 * ([BlockquoteSpan.resolvedStripeBottom]).
 */
object CaretGeometry {
    data class VerticalBounds(val top: Float, val bottom: Float)

    /**
     * Whether the manually-drawn caret should be visible. The native caret is
     * suppressed, so this gates our replacement: only when the field is focused,
     * its window is focused, and the selection is a collapsed insertion point
     * (a range selection shows the selection highlight instead).
     */
    fun shouldRender(
        focused: Boolean,
        windowFocused: Boolean,
        selectionStart: Int,
        selectionEnd: Int
    ): Boolean = focused &&
        windowFocused &&
        selectionStart >= 0 &&
        selectionStart == selectionEnd

    fun verticalBounds(layout: Layout, offset: Int, paint: Paint): VerticalBounds {
        return verticalBounds(layout, offset, paint, layout.text)
    }

    fun verticalBounds(
        layout: Layout,
        offset: Int,
        fallbackPaint: Paint,
        text: CharSequence,
    ): VerticalBounds {
        val line = layout.getLineForOffset(offset.coerceIn(0, layout.text.length))
        val top = layout.editorTextLineTop(line).toFloat()
        val resolvedPaint = resolvedPaintAtOffset(
            fallbackPaint,
            text,
            offset,
            layout.getLineStart(line),
        )
        val bottom = layout.getLineBaseline(line) + resolvedPaint.fontMetrics.descent
        return VerticalBounds(top, bottom)
    }

    private fun resolvedPaintAtOffset(
        fallbackPaint: Paint,
        text: CharSequence,
        offset: Int,
        lineStart: Int,
    ): TextPaint {
        val resolved = TextPaint(fallbackPaint)
        val spanned = text as? Spanned ?: return resolved
        if (spanned.isEmpty()) {
            spanned.getSpans(0, 0, MetricAffectingSpan::class.java).forEach { it.updateMeasureState(resolved) }
            return resolved
        }
        val clampedOffset = offset.coerceIn(0, spanned.length)
        val probe = if (clampedOffset > lineStart) {
            clampedOffset - 1
        } else {
            clampedOffset.coerceAtMost(spanned.length - 1)
        }
        spanned.getSpans(probe, probe + 1, MetricAffectingSpan::class.java)
            .filterNot { it is ParagraphSpacerSpan }
            .forEach { it.updateMeasureState(resolved) }
        return resolved
    }
}
