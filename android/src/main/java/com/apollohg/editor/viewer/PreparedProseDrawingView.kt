package com.apollohg.editor.viewer

import android.content.Context
import android.graphics.Canvas
import android.util.AttributeSet
import android.view.View

/** Draws only precomputed StaticLayout blocks intersecting the dirty rectangle. */
internal class PreparedProseDrawingView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
) : View(context, attrs) {
    var preparedLayout: PreparedProseLayout? = null
        private set
    var onUsableMetricsChanged: (() -> Unit)? = null

    fun install(layout: PreparedProseLayout?) {
        if (preparedLayout === layout) return
        preparedLayout = layout
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val artifact = preparedLayout ?: return
        val clip = canvas.clipBounds
        artifact.blocks.forEach { block ->
            if (!block.intersects(clip)) return@forEach
            val save = canvas.save()
            canvas.translate(0f, block.topPx.toFloat())
            block.layout.draw(canvas)
            canvas.restoreToCount(save)
        }
    }

    override fun onSizeChanged(width: Int, height: Int, oldWidth: Int, oldHeight: Int) {
        super.onSizeChanged(width, height, oldWidth, oldHeight)
        if (width > 0) onUsableMetricsChanged?.invoke()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        if (width > 0) onUsableMetricsChanged?.invoke()
    }
}
