package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.text.style.ReplacementSpan

internal fun resolvedCheckboxStyle(sheet: EditorStyleSheet, checked: Boolean): EditorElementStyle {
    val base = EditorElementStyle(EditorTextStyle(), EditorBoxStyle(border = EditorEdges(1f, 1f, 1f, 1f), corners = EditorCorners(3f, 3f, 3f, 3f)), size = 18f, gap = 6f)
        .mergedWith(sheet["taskCheckbox"])
    return if (checked) base.mergedWith(sheet["taskCheckbox"]?.checked) else base
}

internal fun drawCheckbox(canvas: Canvas, bounds: RectF, box: EditorBoxStyle, checked: Boolean, checkColor: Int) {
    EditorBoxDrawing.draw(canvas, bounds, box)
    if (!checked) return
    val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = checkColor; style = Paint.Style.STROKE; strokeWidth = maxOf(1f, bounds.width() * 0.1f); strokeCap = Paint.Cap.ROUND; strokeJoin = Paint.Join.ROUND }
    val path = Path().apply {
        moveTo(bounds.left + bounds.width() * .22f, bounds.top + bounds.height() * .52f)
        lineTo(bounds.left + bounds.width() * .43f, bounds.top + bounds.height() * .72f)
        lineTo(bounds.left + bounds.width() * .8f, bounds.top + bounds.height() * .28f)
    }
    canvas.drawPath(path, paint)
}

internal class EditorCheckboxSpan(private val style: EditorElementStyle, private val checked: Boolean, private val density: Float) : ReplacementSpan() {
    private val size = ((style.size ?: 18f) * density).toInt()
    override fun getSize(paint: Paint, text: CharSequence, start: Int, end: Int, fm: Paint.FontMetricsInt?): Int {
        fm?.let { it.ascent = minOf(it.ascent, -size); it.top = it.ascent }
        return size
    }
    override fun draw(canvas: Canvas, text: CharSequence, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) {
        val center = y + (paint.fontMetrics.ascent + paint.fontMetrics.descent) / 2f
        drawCheckbox(canvas, RectF(x, center - size / 2f, x + size, center + size / 2f), style.box.scaled(density), checked, style.checkColor ?: paint.color)
    }
}
