package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.DashPathEffect
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PathMeasure
import android.graphics.RectF
import android.text.Layout
import android.text.Spanned

internal object EditorTextDecorationDrawing {
    fun drawRun(canvas: Canvas, source: android.text.TextPaint, style: EditorTextStyle, left: Float, right: Float, baseline: Float) {
        if (!style.hasCustomDecoration()) return
        val paint = android.text.TextPaint(source).apply {
            color = style.textDecorationColor ?: style.color ?: source.color
            this.style = Paint.Style.STROKE
            strokeWidth = maxOf(1f, textSize / 16f)
            pathEffect = when (style.textDecorationStyle) {
                "dotted" -> DashPathEffect(floatArrayOf(strokeWidth, strokeWidth * 1.5f), 0f)
                "dashed" -> DashPathEffect(floatArrayOf(strokeWidth * 3f, strokeWidth * 2f), 0f)
                else -> null
            }
        }
        fun stroke(y: Float) {
            canvas.drawLine(left, y, right, y, paint)
            if (style.textDecorationStyle == "double") canvas.drawLine(left, y + paint.strokeWidth * 2f, right, y + paint.strokeWidth * 2f, paint)
        }
        if (style.textDecorationLine?.contains("underline") == true) stroke(baseline + maxOf(paint.strokeWidth, paint.fontMetrics.descent * .5f))
        if (style.textDecorationLine?.contains("line-through") == true) stroke(baseline + paint.fontMetrics.ascent * .35f)
    }

    fun draw(canvas: Canvas, layout: Layout) {
        val text = layout.text as? Spanned ?: return
        text.getSpans(0, text.length, EditorResolvedTextSpan::class.java).forEach { span ->
            val style = span.style
            if (!style.hasCustomDecoration()) return@forEach
            val start = text.getSpanStart(span).coerceAtLeast(0)
            val end = text.getSpanEnd(span).coerceAtMost(text.length)
            if (end <= start) return@forEach
            val paint = android.text.TextPaint(layout.paint)
            span.updateMeasureState(paint)
            paint.color = style.textDecorationColor ?: style.color ?: layout.paint.color
            paint.style = Paint.Style.STROKE
            paint.strokeWidth = maxOf(1f, paint.textSize / 16f)
            paint.pathEffect = when (style.textDecorationStyle) {
                "dotted" -> DashPathEffect(floatArrayOf(paint.strokeWidth, paint.strokeWidth * 1.5f), 0f)
                "dashed" -> DashPathEffect(floatArrayOf(paint.strokeWidth * 3f, paint.strokeWidth * 2f), 0f)
                else -> null
            }
            for (line in layout.getLineForOffset(start)..layout.getLineForOffset(end - 1)) {
                val a = maxOf(start, layout.getLineStart(line))
                var b = minOf(end, layout.getLineEnd(line))
                if (b > a && text[b - 1] == '\n') b--
                if (b <= a) continue
                val selection = Path()
                layout.getSelectionPath(a, b, selection)
                val measure = PathMeasure(selection, false)
                do {
                    val contour = Path()
                    measure.getSegment(0f, measure.length, contour, true)
                    val bounds = RectF()
                    contour.computeBounds(bounds, true)
                    if (bounds.width() > 0) {
                        val baseline = layout.getLineBaseline(line).toFloat()
                        fun stroke(y: Float) {
                            canvas.drawLine(bounds.left, y, bounds.right, y, paint)
                            if (style.textDecorationStyle == "double") canvas.drawLine(bounds.left, y + paint.strokeWidth * 2f, bounds.right, y + paint.strokeWidth * 2f, paint)
                        }
                        if (style.textDecorationLine?.contains("underline") == true) stroke(baseline + maxOf(paint.strokeWidth, paint.fontMetrics.descent * .5f))
                        if (style.textDecorationLine?.contains("line-through") == true) stroke(baseline + paint.fontMetrics.ascent * .35f)
                    }
                } while (measure.nextContour())
            }
        }
    }
}

internal fun EditorTextStyle.hasCustomDecoration(): Boolean = textDecorationLine != null && textDecorationLine != "none" && (textDecorationColor != null || textDecorationStyle != null && textDecorationStyle != "solid")
