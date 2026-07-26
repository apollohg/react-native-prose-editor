package com.apollohg.editor.viewer

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.util.AttributeSet
import android.view.View

/** Rendering-only consumer of fully prepared StaticLayout and geometry fragments. */
internal class PreparedProseDrawingView @JvmOverloads constructor(context: Context, attrs: AttributeSet? = null) : View(context, attrs) {
    var preparedLayout: PreparedProseLayout? = null
        private set
    var onUsableMetricsChanged: (() -> Unit)? = null
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)

    fun install(layout: PreparedProseLayout?) {
        if (preparedLayout === layout) return
        preparedLayout = layout
        invalidate()
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val artifact = preparedLayout ?: return
        val visible = mutableListOf<PreparedProseFragment>()
        artifact.forEachBlockIntersecting(canvas.clipBounds) { visible += it.fragments }
        // Phases stay global across blocks: later code backgrounds cannot cover
        // an earlier quote border, and text/labels always remain foreground.
        visible.forEach { drawBackground(canvas, it) }
        visible.forEach { drawBorderOrRule(canvas, it) }
        visible.forEach { drawForeground(canvas, it) }
    }

    private fun drawBackground(canvas: Canvas, fragment: PreparedProseFragment) {
        if (fragment.kind != PreparedProseFragmentKind.BACKGROUND && fragment.kind != PreparedProseFragmentKind.ATOM) return
        paint.style = Paint.Style.FILL
        paint.color = fragment.color ?: return
        canvas.drawRoundRect(RectF(fragment.bounds), fragment.cornerRadius, fragment.cornerRadius, paint)
    }

    private fun drawBorderOrRule(canvas: Canvas, fragment: PreparedProseFragment) {
        when (fragment.kind) {
            PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.RULE -> {
                paint.style = Paint.Style.FILL
                paint.color = fragment.color ?: return
                canvas.drawRect(fragment.bounds, paint)
            }
            PreparedProseFragmentKind.ATOM -> if (fragment.strokeWidth > 0f) {
                paint.style = Paint.Style.STROKE
                paint.strokeWidth = fragment.strokeWidth
                paint.color = fragment.borderColor ?: fragment.color ?: return
                val inset = fragment.strokeWidth / 2f
                canvas.drawRoundRect(RectF(fragment.bounds).apply { inset(inset, inset) }, maxOf(0f, fragment.cornerRadius - inset), maxOf(0f, fragment.cornerRadius - inset), paint)
            }
            else -> Unit
        }
    }

    private fun drawForeground(canvas: Canvas, fragment: PreparedProseFragment) {
        when (fragment.kind) {
            PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER -> {
                fragment.layout?.let { layout ->
                    val saved = canvas.save()
                    canvas.translate(fragment.layoutX.toFloat(), fragment.layoutY.toFloat())
                    layout.draw(canvas)
                    canvas.restoreToCount(saved)
                } ?: if (fragment.kind == PreparedProseFragmentKind.MARKER) drawTaskMarker(canvas, fragment)
            }
            PreparedProseFragmentKind.ATOM -> fragment.labelLayout?.let { layout ->
                val saved = canvas.save()
                canvas.translate(fragment.labelX.toFloat(), fragment.labelY.toFloat())
                layout.draw(canvas)
                canvas.restoreToCount(saved)
            }
            PreparedProseFragmentKind.STRIKE -> {
                paint.style = Paint.Style.FILL
                paint.color = fragment.color ?: return
                canvas.drawRect(fragment.bounds, paint)
            }
            else -> Unit
        }
    }

    private fun drawTaskMarker(canvas: Canvas, fragment: PreparedProseFragment) {
        val bounds = RectF(fragment.bounds)
        val inset = maxOf(1f, bounds.height() * 0.2f)
        val box = RectF(bounds).apply { inset(inset, inset) }
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = maxOf(1f, box.width() * 0.1f)
        paint.color = fragment.color ?: return
        canvas.drawRoundRect(box, box.width() * 0.2f, box.width() * 0.2f, paint)
        if (!fragment.checked) return
        paint.style = Paint.Style.STROKE
        paint.strokeWidth = maxOf(1.4f, box.width() * 0.12f)
        paint.strokeCap = Paint.Cap.ROUND
        paint.strokeJoin = Paint.Join.ROUND
        val path = android.graphics.Path().apply {
            moveTo(box.left + box.width() * 0.2f, box.centerY())
            lineTo(box.left + box.width() * 0.43f, box.bottom - box.height() * 0.2f)
            lineTo(box.right - box.width() * 0.16f, box.top + box.height() * 0.2f)
        }
        canvas.drawPath(path, paint)
        paint.strokeCap = Paint.Cap.BUTT
        paint.strokeJoin = Paint.Join.MITER
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
