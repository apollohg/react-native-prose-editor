package com.apollohg.editor

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import kotlin.math.max

internal class ImageResizeOverlayView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : View(context, attrs, defStyleAttr) {
    private enum class Corner {
        TOP_LEFT,
        TOP_RIGHT,
        BOTTOM_LEFT,
        BOTTOM_RIGHT
    }

    private data class DragState(
        val corner: Corner,
        val originalRect: RectF,
        val docPos: Int,
        val maximumWidthPx: Float,
        var previewRect: RectF
    )

    private var editorView: RichTextEditorView? = null
    private var currentGeometry: EditorEditText.SelectedImageGeometry? = null
    private var dragState: DragState? = null

    private val density = resources.displayMetrics.density
    private val handleRadiusPx = 10f * density
    private val minimumImageSizePx = 48f * density
    private val borderPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.parseColor("#0A84FF")
        style = Paint.Style.STROKE
        strokeWidth = max(2f, density)
    }
    private val handleFillPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.WHITE
        style = Paint.Style.FILL
    }
    private val handleStrokePaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.parseColor("#0A84FF")
        style = Paint.Style.STROKE
        strokeWidth = max(2f, density)
    }

    init {
        setWillNotDraw(false)
        visibility = INVISIBLE
    }

    fun bind(editorView: RichTextEditorView) {
        this.editorView = editorView
    }

    fun refresh() {
        currentGeometry = editorView?.selectedImageGeometry()
        visibility = if (currentGeometry == null) INVISIBLE else VISIBLE
        if (currentGeometry != null) {
            bringToFront()
        }
        invalidate()
    }

    fun visibleRectForTesting(): RectF? =
        currentGeometry?.rect?.let(::RectF)

    fun simulateResizeForTesting(widthPx: Float, heightPx: Float) {
        val geometry = currentGeometry ?: return
        editorView?.resizeImage(geometry.docPos, widthPx, heightPx)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val geometry = currentGeometry ?: return
        canvas.drawRoundRect(geometry.rect, 8f * density, 8f * density, borderPaint)
        for (corner in Corner.entries) {
            val center = handleCenter(corner, geometry.rect)
            canvas.drawCircle(center.x, center.y, handleRadiusPx, handleFillPaint)
            canvas.drawCircle(center.x, center.y, handleRadiusPx, handleStrokePaint)
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val geometry = currentGeometry ?: return false

        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val corner = cornerAt(event.x, event.y, geometry.rect) ?: return false
                dragState = DragState(
                    corner = corner,
                    originalRect = RectF(geometry.rect),
                    docPos = geometry.docPos,
                    maximumWidthPx = editorView?.maximumImageWidthPx() ?: geometry.rect.width(),
                    previewRect = RectF(geometry.rect)
                )
                parent?.requestDisallowInterceptTouchEvent(true)
                return true
            }

            MotionEvent.ACTION_MOVE -> {
                val state = dragState ?: return false
                val nextRect = resizedRect(
                    originalRect = state.originalRect,
                    corner = state.corner,
                    deltaX = event.x - handleCenter(state.corner, state.originalRect).x,
                    deltaY = event.y - handleCenter(state.corner, state.originalRect).y,
                    maximumWidthPx = state.maximumWidthPx
                )
                state.previewRect = RectF(nextRect)
                currentGeometry = EditorEditText.SelectedImageGeometry(state.docPos, nextRect)
                invalidate()
                return true
            }

            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                val state = dragState ?: return false
                if (event.actionMasked == MotionEvent.ACTION_UP) {
                    editorView?.resizeImage(
                        state.docPos,
                        state.previewRect.width(),
                        state.previewRect.height()
                    )
                }
                dragState = null
                parent?.requestDisallowInterceptTouchEvent(false)
                post { refresh() }
                return true
            }
        }

        return false
    }

    private fun cornerAt(x: Float, y: Float, rect: RectF): Corner? {
        return Corner.entries.firstOrNull { corner ->
            val center = handleCenter(corner, rect)
            val dx = x - center.x
            val dy = y - center.y
            (dx * dx) + (dy * dy) <= handleRadiusPx * handleRadiusPx * 2
        }
    }

    private fun handleCenter(corner: Corner, rect: RectF) = when (corner) {
        Corner.TOP_LEFT -> android.graphics.PointF(rect.left, rect.top)
        Corner.TOP_RIGHT -> android.graphics.PointF(rect.right, rect.top)
        Corner.BOTTOM_LEFT -> android.graphics.PointF(rect.left, rect.bottom)
        Corner.BOTTOM_RIGHT -> android.graphics.PointF(rect.right, rect.bottom)
    }

    private fun anchorPoint(corner: Corner, rect: RectF) = when (corner) {
        Corner.TOP_LEFT -> android.graphics.PointF(rect.right, rect.bottom)
        Corner.TOP_RIGHT -> android.graphics.PointF(rect.left, rect.bottom)
        Corner.BOTTOM_LEFT -> android.graphics.PointF(rect.right, rect.top)
        Corner.BOTTOM_RIGHT -> android.graphics.PointF(rect.left, rect.top)
    }

    private fun resizedRect(
        originalRect: RectF,
        corner: Corner,
        deltaX: Float,
        deltaY: Float,
        maximumWidthPx: Float?
    ): RectF {
        val aspectRatio = max(originalRect.width() / max(originalRect.height(), 1f), 0.1f)
        val signedDx = if (corner == Corner.TOP_RIGHT || corner == Corner.BOTTOM_RIGHT) deltaX else -deltaX
        val signedDy = if (corner == Corner.BOTTOM_LEFT || corner == Corner.BOTTOM_RIGHT) deltaY else -deltaY
        val widthScale = (originalRect.width() + signedDx) / max(originalRect.width(), 1f)
        val heightScale = (originalRect.height() + signedDy) / max(originalRect.height(), 1f)
        val scale = max(max(widthScale, heightScale), minimumImageSizePx / max(originalRect.width(), 1f))
        val unclampedWidth = max(minimumImageSizePx, originalRect.width() * scale)
        val unclampedHeight = max(minimumImageSizePx / aspectRatio, unclampedWidth / aspectRatio)
        val (width, height) = editorView?.let { boundEditor ->
            maximumWidthPx?.let { maxWidth ->
                boundEditor.clampImageSize(
                    widthPx = unclampedWidth,
                    heightPx = unclampedHeight,
                    maximumWidthPx = maxWidth
                )
            } ?: boundEditor.clampImageSize(unclampedWidth, unclampedHeight)
        } ?: (unclampedWidth to unclampedHeight)
        val anchor = anchorPoint(corner, originalRect)

        return when (corner) {
            Corner.TOP_LEFT -> RectF(anchor.x - width, anchor.y - height, anchor.x, anchor.y)
            Corner.TOP_RIGHT -> RectF(anchor.x, anchor.y - height, anchor.x + width, anchor.y)
            Corner.BOTTOM_LEFT -> RectF(anchor.x - width, anchor.y, anchor.x, anchor.y + height)
            Corner.BOTTOM_RIGHT -> RectF(anchor.x, anchor.y, anchor.x + width, anchor.y + height)
        }
    }
}
