package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.ColorFilter
import android.graphics.PixelFormat
import android.graphics.drawable.Drawable
import kotlin.math.roundToInt

internal class EditorGlyphHeightCursorDrawable(private val editor: EditorEditText) : Drawable() {
    private val cursorPaint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG)
    private var cursorAlpha = 255
    private var cursorColorFilter: ColorFilter? = null

    override fun draw(canvas: Canvas) {
        val rect = editor.nativeCursorDrawRect() ?: return
        val textLayout = editor.layout ?: return
        val offset = editor.selectionEnd.coerceIn(0, textLayout.text.length)
        val line = textLayout.getLineForOffset(offset)
        val hasEditorBounds =
            bounds.top == textLayout.getLineTop(line) &&
                bounds.bottom == AndroidApiCompat.lineBottomWithoutSpacing(textLayout, line)
        val top = if (hasEditorBounds) rect.top else bounds.top.toFloat()
        val bottom = if (hasEditorBounds) rect.bottom else bounds.bottom.toFloat()
        cursorPaint.color = editor.caretColor
        cursorPaint.alpha = cursorAlpha
        cursorPaint.colorFilter = cursorColorFilter
        canvas.drawRect(bounds.left.toFloat(), top, bounds.right.toFloat(), bottom, cursorPaint)
    }

    override fun setAlpha(alpha: Int) {
        cursorAlpha = alpha
    }

    override fun setColorFilter(colorFilter: ColorFilter?) {
        cursorColorFilter = colorFilter
    }

    @Deprecated("Deprecated in Android")
    override fun getOpacity(): Int = PixelFormat.TRANSLUCENT

    override fun getIntrinsicWidth(): Int = editor.caretWidthPx.roundToInt()
}
