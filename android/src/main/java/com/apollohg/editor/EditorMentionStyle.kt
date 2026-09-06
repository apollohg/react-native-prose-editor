package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.text.TextPaint
import android.text.style.ReplacementSpan

internal fun EditorElementStyle.mergedWith(other: EditorElementStyle?): EditorElementStyle {
    other ?: return this
    val keys = other.declaredProperties
    fun sides(prefix: String, current: EditorEdges, next: EditorEdges, suffix: String = "") = EditorEdges(
        if ("${prefix}Top$suffix" in keys) next.top else current.top,
        if ("${prefix}Right$suffix" in keys) next.right else current.right,
        if ("${prefix}Bottom$suffix" in keys) next.bottom else current.bottom,
        if ("${prefix}Left$suffix" in keys) next.left else current.left,
    )
    return copy(
        text = text.mergedWith(other.text),
        box = box.copy(
            backgroundColor = if ("backgroundColor" in keys) other.box.backgroundColor else box.backgroundColor,
            padding = sides("padding", box.padding, other.box.padding),
            margin = sides("margin", box.margin, other.box.margin),
            border = sides("border", box.border, other.box.border, "Width"),
            borderColors = listOf("Top", "Right", "Bottom", "Left").mapIndexed { index, side -> if ("border${side}Color" in keys) other.box.borderColors[index] else box.borderColors[index] },
            corners = EditorCorners(
                if ("borderTopLeftRadius" in keys) other.box.corners.topLeft else box.corners.topLeft,
                if ("borderTopRightRadius" in keys) other.box.corners.topRight else box.corners.topRight,
                if ("borderBottomRightRadius" in keys) other.box.corners.bottomRight else box.corners.bottomRight,
                if ("borderBottomLeftRadius" in keys) other.box.corners.bottomLeft else box.corners.bottomLeft,
            ),
            borderStyle = if ("borderStyle" in keys) other.box.borderStyle else box.borderStyle,
        ),
        declaredProperties = declaredProperties + keys,
        size = other.size ?: size, gap = other.gap ?: gap, checkColor = other.checkColor ?: checkColor,
    )
}

internal fun resolvedMentionStyle(base: EditorTextStyle, theme: EditorTheme?, local: EditorMentionTheme?): EditorElementStyle {
    var result = EditorElementStyle(base, EditorBoxStyle(backgroundColor = 0x1F007AFF, padding = EditorEdges(2f, 4f, 2f, 4f), corners = EditorCorners(6f, 6f, 6f, 6f)))
        .mergedWith(theme?.styleSheet?.get("mention"))
    listOf(theme?.mentions?.node, local?.node).forEach { node ->
        if (node != null) {
            result = result.copy(
                text = result.text.mergedWith(EditorTextStyle(color = node.textColor, fontWeight = node.fontWeight)),
                box = result.box.copy(
                    backgroundColor = node.backgroundColor ?: result.box.backgroundColor,
                    border = node.borderWidth?.let { EditorEdges(it, it, it, it) } ?: result.box.border,
                    borderColors = node.borderColor?.let { List(4) { _ -> it } } ?: result.box.borderColors,
                    corners = node.borderRadius?.let { EditorCorners(it, it, it, it) } ?: result.box.corners,
                ),
            ).mergedWith(node.style)
        }
    }
    return result
}

internal class EditorMentionSpan(private val style: EditorElementStyle, private val density: Float) : ReplacementSpan() {
    private fun paint(base: Paint): TextPaint = TextPaint(base).also { EditorResolvedTextSpan(style.text, density).updateDrawState(it) }
    override fun getSize(paint: Paint, text: CharSequence, start: Int, end: Int, fm: Paint.FontMetricsInt?): Int {
        val resolved = paint(paint)
        val inset = style.box.inset.scaled(density)
        fm?.let {
            resolved.getFontMetricsInt(it)
            val extra = ((style.text.lineHeight?.times(density) ?: 0f).toInt() - (it.descent - it.ascent)).coerceAtLeast(0)
            it.ascent -= extra / 2
            it.descent += extra - extra / 2
            it.ascent -= inset.top.toInt()
            it.descent += inset.bottom.toInt()
            it.top = it.ascent
            it.bottom = it.descent
        }
        return kotlin.math.ceil(resolved.measureText(text, start, end) + inset.left + inset.right).toInt()
    }
    override fun draw(canvas: Canvas, text: CharSequence, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) {
        val resolved = paint(paint)
        val box = style.box.scaled(density)
        val metrics = resolved.fontMetricsInt.apply {
            val extra = ((style.text.lineHeight?.times(density) ?: 0f).toInt() - (descent - ascent)).coerceAtLeast(0)
            ascent -= extra / 2
            descent += extra - extra / 2
        }
        val rect = RectF(x, y + metrics.ascent - box.inset.top, x + getSize(paint, text, start, end, null), y + metrics.descent + box.inset.bottom)
        EditorBoxDrawing.draw(canvas, rect, box)
        resolved.bgColor = 0
        canvas.drawText(text, start, end, x + box.inset.left, y.toFloat(), resolved)
        EditorTextDecorationDrawing.drawRun(canvas, resolved, style.text, x + box.inset.left, rect.right - box.inset.right, y.toFloat())
    }
}
