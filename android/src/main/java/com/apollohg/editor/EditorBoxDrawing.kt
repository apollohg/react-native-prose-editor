package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.DashPathEffect
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF

internal object EditorBoxDrawing {
    fun path(bounds: RectF, corners: EditorCorners): Path {
        val sums = listOf(corners.topLeft + corners.topRight, corners.bottomLeft + corners.bottomRight, corners.topLeft + corners.bottomLeft, corners.topRight + corners.bottomRight)
        val scale = minOf(1f, *sums.mapIndexed { index, sum -> if (sum <= 0) 1f else (if (index < 2) bounds.width() else bounds.height()) / sum }.toFloatArray()).coerceAtLeast(0f)
        val radii = listOf(corners.topLeft, corners.topRight, corners.bottomRight, corners.bottomLeft).flatMap { listOf(it * scale, it * scale) }.toFloatArray()
        return Path().apply { addRoundRect(bounds, radii, Path.Direction.CW) }
    }

    fun draw(canvas: Canvas, bounds: RectF, box: EditorBoxStyle) {
        if (bounds.isEmpty) return
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        val outer = path(bounds, box.corners)
        box.backgroundColor?.let { paint.color = it; canvas.drawPath(outer, paint) }
        val innerBounds = RectF(bounds.left + box.border.left, bounds.top + box.border.top, bounds.right - box.border.right, bounds.bottom - box.border.bottom)
        val inner = path(innerBounds, innerCorners(box))
        val ring = Path(outer).apply { op(inner, Path.Op.DIFFERENCE) }
        if (box.borderStyle == "solid" && box.borderColors.distinct().size == 1) {
            paint.color = box.borderColors.first()
            canvas.drawPath(ring, paint)
            return
        }
        val widths = listOf(box.border.top, box.border.right, box.border.bottom, box.border.left)
        // Extend side clips through the rounded inner corners.
        val horizontal = box.border.left + box.border.right
        val vertical = box.border.top + box.border.bottom
        val reach = minOf(if (horizontal > 0) bounds.width() / horizontal else Float.POSITIVE_INFINITY, if (vertical > 0) bounds.height() / vertical else Float.POSITIVE_INFINITY)
        if (!reach.isFinite()) return
        val join = RectF(bounds.left + box.border.left * reach, bounds.top + box.border.top * reach, bounds.right - box.border.right * reach, bounds.bottom - box.border.bottom * reach)
        val vertices = listOf(
            floatArrayOf(bounds.left, bounds.top, bounds.right, bounds.top, join.right, join.top, join.left, join.top),
            floatArrayOf(bounds.right, bounds.top, bounds.right, bounds.bottom, join.right, join.bottom, join.right, join.top),
            floatArrayOf(bounds.right, bounds.bottom, bounds.left, bounds.bottom, join.left, join.bottom, join.right, join.bottom),
            floatArrayOf(bounds.left, bounds.bottom, bounds.left, bounds.top, join.left, join.top, join.left, join.bottom),
        )
        widths.forEachIndexed { index, width ->
            if (width <= 0) return@forEachIndexed
            val saved = canvas.save()
            canvas.clipPath(ring)
            val points = vertices[index]
            val wedge = Path().apply { moveTo(points[0], points[1]); for (i in 2..6 step 2) lineTo(points[i], points[i + 1]); close() }
            canvas.clipPath(wedge)
            paint.color = box.borderColors[index]
            if (box.borderStyle == "solid") {
                paint.style = Paint.Style.FILL
                canvas.drawPath(outer, paint)
            } else {
                paint.style = Paint.Style.STROKE
                paint.strokeWidth = width * 2f
                paint.pathEffect = DashPathEffect(if (box.borderStyle == "dotted") floatArrayOf(width, width) else floatArrayOf(width * 3f, width * 2f), 0f)
                canvas.drawPath(outer, paint)
                paint.pathEffect = null
            }
            canvas.restoreToCount(saved)
        }
    }

    fun innerCorners(box: EditorBoxStyle) = EditorCorners(
        (box.corners.topLeft - maxOf(box.border.top, box.border.left)).coerceAtLeast(0f),
        (box.corners.topRight - maxOf(box.border.top, box.border.right)).coerceAtLeast(0f),
        (box.corners.bottomRight - maxOf(box.border.bottom, box.border.right)).coerceAtLeast(0f),
        (box.corners.bottomLeft - maxOf(box.border.bottom, box.border.left)).coerceAtLeast(0f),
    )

    fun drawImage(canvas: Canvas, bitmap: Bitmap, bounds: RectF, box: EditorBoxStyle, resizeMode: String) {
        val inner = RectF(bounds.left + box.inset.left, bounds.top + box.inset.top, bounds.right - box.inset.right, bounds.bottom - box.inset.bottom)
        val saved = canvas.save()
        canvas.clipPath(path(inner, innerCorners(box)))
        val target = RectF(inner)
        if (resizeMode != "stretch") {
            val scale = if (resizeMode == "cover") maxOf(inner.width() / bitmap.width, inner.height() / bitmap.height) else minOf(inner.width() / bitmap.width, inner.height() / bitmap.height)
            val width = bitmap.width * scale
            val height = bitmap.height * scale
            target.set(inner.centerX() - width / 2, inner.centerY() - height / 2, inner.centerX() + width / 2, inner.centerY() + height / 2)
        }
        canvas.drawBitmap(bitmap, null, target, Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG))
        canvas.restoreToCount(saved)
    }
}
