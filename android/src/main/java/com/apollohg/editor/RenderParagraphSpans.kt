package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.text.Spanned
import android.text.style.LineHeightSpan
import android.text.style.ReplacementSpan

class FixedLineHeightSpan(
    private val lineHeightPx: Int
) : LineHeightSpan {
    override fun chooseHeight(
        text: CharSequence,
        start: Int,
        end: Int,
        spanstartv: Int,
        v: Int,
        fm: android.graphics.Paint.FontMetricsInt
    ) {
        val paragraphSpacing = if (end > start && text[end - 1] == '\n' && text is Spanned) {
            text.getSpans(end - 1, end, ParagraphSpacerSpan::class.java)
                .maxOfOrNull { it.spacingPx }
                ?: 0
        } else {
            0
        }
        val currentHeight = fm.descent - paragraphSpacing - fm.ascent
        if (lineHeightPx <= 0 || currentHeight <= 0) return
        if (lineHeightPx == currentHeight) return

        val extra = lineHeightPx - currentHeight
        fm.descent += extra
        fm.bottom = fm.descent
    }
}

/**
 * Adds vertical spacing after a paragraph by increasing the descent of the
 * inter-block newline character.
 *
 * Uses [ReplacementSpan] (not [LineHeightSpan]/[android.text.style.ParagraphStyle])
 * because Android's StaticLayout normalizes ParagraphStyle metrics across all
 * lines in a paragraph, making per-line spacing impossible.
 *
 * ReplacementSpan only affects the single character it covers, so the extra
 * descent applies only to the newline's line — creating a gap below the
 * preceding paragraph without inflating other lines.
 */
class ParagraphSpacerSpan(
    internal val spacingPx: Int,
    private val baseFontSize: Int,
    private val textColor: Int
) : ReplacementSpan() {
    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        if (fm != null && spacingPx > 0) {
            // Keep the natural ascent/top (from baseFontSize) so the newline
            // line doesn't shrink above the baseline. Add spacing as descent.
            val savedSize = paint.textSize
            paint.textSize = baseFontSize.toFloat()
            paint.getFontMetricsInt(fm)
            paint.textSize = savedSize
            fm.descent += spacingPx
            fm.bottom = fm.descent
        }
        return 0
    }

    override fun draw(
        canvas: Canvas,
        text: CharSequence,
        start: Int,
        end: Int,
        x: Float,
        top: Int,
        y: Int,
        bottom: Int,
        paint: Paint
    ) {
        // Draw nothing — pure spacing.
    }
}

class MarkerGapSpan(private val widthPx: Float) : ReplacementSpan() {
    override fun getSize(paint: Paint, text: CharSequence, start: Int, end: Int, fm: Paint.FontMetricsInt?): Int =
        kotlin.math.ceil(widthPx).toInt()

    override fun draw(canvas: Canvas, text: CharSequence, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) = Unit
}

internal class OrderedListMarkerSpan(
    internal val label: String
) : ReplacementSpan() {
    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int = kotlin.math.ceil(paint.measureText(label).toDouble()).toInt()

    override fun draw(
        canvas: Canvas,
        text: CharSequence,
        start: Int,
        end: Int,
        x: Float,
        top: Int,
        y: Int,
        bottom: Int,
        paint: Paint
    ) {
        canvas.drawText(label, x, y.toFloat(), paint)
    }
}

class CenteredBulletSpan(
    private val textColor: Int,
    private val markerWidthPx: Float,
    private val bulletRadiusPx: Float,
    private val bodyFontSizePx: Float,
    private val markerGapToTextPx: Float
) : ReplacementSpan() {
    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        return kotlin.math.ceil(markerWidthPx).toInt()
    }

    override fun draw(
        canvas: Canvas,
        text: CharSequence,
        start: Int,
        end: Int,
        x: Float,
        top: Int,
        y: Int,
        bottom: Int,
        paint: Paint
    ) {
        val previousColor = paint.color
        val previousStyle = paint.style
        val previousSize = paint.textSize

        paint.color = textColor
        paint.style = Paint.Style.FILL

        // Use body text metrics (not the marker's inflated font) for centering.
        paint.textSize = bodyFontSizePx
        val fm = paint.fontMetrics
        val centerX = resolvedCenterX(x)
        val centerY = y + (fm.ascent + fm.descent) / 2f
        canvas.drawCircle(centerX, centerY, bulletRadiusPx, paint)

        paint.color = previousColor
        paint.style = previousStyle
        paint.textSize = previousSize
    }

    fun textSideGapPx(x: Float): Float {
        return (x + markerWidthPx) - (resolvedCenterX(x) + bulletRadiusPx)
    }

    private fun resolvedCenterX(x: Float): Float {
        return x + markerWidthPx - markerGapToTextPx - bulletRadiusPx
    }
}
