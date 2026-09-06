package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.ColorFilter
import android.graphics.PixelFormat
import android.graphics.RectF
import android.graphics.drawable.Drawable

internal class EditorBoxDrawable(private val box: EditorBoxStyle) : Drawable() {
    override fun draw(canvas: Canvas) = EditorBoxDrawing.draw(canvas, RectF(bounds), box)
    override fun setAlpha(alpha: Int) = Unit
    override fun setColorFilter(colorFilter: ColorFilter?) = Unit
    @Deprecated("Drawable opacity is no longer used")
    override fun getOpacity(): Int = PixelFormat.TRANSLUCENT
}
