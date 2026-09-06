package com.apollohg.editor

import com.apollohg.editor.EditorEditText.SelectedImageGeometry
import android.graphics.Color
import android.graphics.RectF
import android.text.Layout
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.view.MotionEvent
import kotlin.math.roundToInt

internal fun EditorEditText.cancelPendingImageLoads() {
    imageLoadGeneration += 1L
    val handles = synchronized(imageLoadHandles) {
        imageLoadHandles.toList().also { imageLoadHandles.clear() }
    }
    handles.forEach { it.cancel() }
}

internal fun EditorEditText.rebuildLatestRenderForImages(): Boolean {
    val currentContent = text as? Spanned ?: return false
    val imageSpans = currentContent.getSpans(
        0,
        currentContent.length,
        BlockImageSpan::class.java
    )
    if (imageSpans.isEmpty()) return false

    cancelPendingImageLoads()
    val spannable = SpannableStringBuilder(currentContent)
    imageSpans.forEach { span ->
        val start = currentContent.getSpanStart(span)
        val end = currentContent.getSpanEnd(span)
        val flags = currentContent.getSpanFlags(span)
        span.close()
        spannable.removeSpan(span)
        if (start >= 0 && end > start) {
            spannable.setSpan(span.reloadedFor(this), start, end, flags)
        }
    }
    applyFullRenderPreservingEditorState(spannable, restoreScrollAfterLayout = true)
    requestLayout()
    invalidate()
    onContentSizeMayChange?.invoke()
    onSelectionOrContentMayChange?.invoke()
    return true
}

internal fun EditorEditText.hasRenderedImageSpans(): Boolean {
    val content = text as? Spanned ?: return false
    return content.getSpans(0, content.length, BlockImageSpan::class.java).isNotEmpty()
}

internal fun EditorEditText.selectedImageGeometryImpl(): SelectedImageGeometry? {
    if (!imageResizingEnabled) return null
    val spannable = text as? Spanned ?: return null
    val selection = resolvedSelectedImageRange(spannable) ?: return null
    val start = selection.start
    val end = selection.end
    val imageSpan = spannable
        .getSpans(start, end, BlockImageSpan::class.java)
        .firstOrNull() ?: return null
    val spanStart = spannable.getSpanStart(imageSpan)
    val spanEnd = spannable.getSpanEnd(imageSpan)
    if (spanStart != start || spanEnd != end) return null

    val textLayout = layout ?: return null
    val currentText = text?.toString() ?: return null
    val scalarPos = PositionBridge.utf16ToScalar(spanStart, currentText)
    val docPos = v2Driver?.docPositionForScalar(scalarPos) ?: scalarPos
    val line = textLayout.getLineForOffset(spanStart.coerceAtMost(maxOf(spannable.length - 1, 0)))
    val rect = resolvedImageRect(textLayout, imageSpan, spanStart, spanEnd)
    return SelectedImageGeometry(
        docPos = docPos,
        rect = rect
    )
}

internal fun EditorEditText.resizeImageAtDocPosImpl(docPos: Int, widthPx: Float, heightPx: Float) {
    if (!hasLiveEditor()) return
    val density = resources.displayMetrics.density
    val widthDp = maxOf(48, (widthPx / density).roundToInt())
    val heightDp = maxOf(48, (heightPx / density).roundToInt())
    onResizeImageAtDocPosForTesting?.let { callback ->
        callback(docPos, widthDp, heightDp)
        return
    }
    v2Driver?.let { driver ->
        driver.resizeImageAtDocPos(docPos, widthDp, heightDp)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.handleImageTap(event: MotionEvent): Boolean {
    if (!imageResizingEnabled) {
        pendingImageGesture = null
        return false
    }
    when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
            val hit = if (event.pointerCount == 1) imageSpanHitAt(event.x, event.y) else null
            pendingImageGesture = hit?.let {
                ImageGesture(
                    target = it.span,
                    pointerId = event.getPointerId(event.actionIndex),
                    downX = event.x,
                    downY = event.y
                )
            }
            if (hit != null) requestFocus()
            return hit != null
        }
        MotionEvent.ACTION_MOVE -> {
            val gesture = pendingImageGesture ?: return false
            if (
                event.pointerCount != 1 ||
                event.findPointerIndex(gesture.pointerId) < 0 ||
                movedBeyondImageTouchSlop(gesture, event)
            ) {
                pendingImageGesture = null
                return false
            }
            return true
        }
        MotionEvent.ACTION_POINTER_DOWN,
        MotionEvent.ACTION_POINTER_UP,
        MotionEvent.ACTION_CANCEL -> {
            pendingImageGesture = null
            return false
        }
        MotionEvent.ACTION_UP -> {
            val gesture = pendingImageGesture
            pendingImageGesture = null
            if (
                gesture == null ||
                event.pointerCount != 1 ||
                event.getPointerId(event.actionIndex) != gesture.pointerId ||
                movedBeyondImageTouchSlop(gesture, event)
            ) {
                return false
            }
            val hit = imageSpanHitAt(event.x, event.y) ?: return false
            if (hit.span !== gesture.target) return false
            requestFocus()
            selectExplicitImageRange(hit.start, hit.end)
            performClick()
            return true
        }
        else -> return false
    }
}

internal fun EditorEditText.movedBeyondImageTouchSlop(gesture: ImageGesture, event: MotionEvent): Boolean {
    val deltaX = event.x - gesture.downX
    val deltaY = event.y - gesture.downY
    return deltaX * deltaX + deltaY * deltaY > touchSlopPx * touchSlopPx
}

internal fun EditorEditText.imageSpanHitAt(x: Float, y: Float): ImageSpanHit? {
    val spannable = text as? Spanned ?: return null
    imageSpanRangeNearTouchOffset(spannable, x, y)?.let { return it }
    val textLayout = layout ?: return null
    return imageSpanRectHit(spannable, textLayout, x, y)
}

internal fun EditorEditText.imageSpanRectHit(
    spannable: Spanned,
    textLayout: Layout,
    x: Float,
    y: Float
): ImageSpanHit? {
    val candidateSpans = spannable.getSpans(0, spannable.length, BlockImageSpan::class.java)
    for (span in candidateSpans) {
        val spanStart = spannable.getSpanStart(span)
        val spanEnd = spannable.getSpanEnd(span)
        if (spanStart < 0 || spanEnd <= spanStart) continue
        val rect = resolvedImageRect(textLayout, span, spanStart, spanEnd)
        if (rect.contains(x, y)) {
            return ImageSpanHit(span, spanStart, spanEnd)
        }
    }
    return null
}

internal fun EditorEditText.selectExplicitImageRange(start: Int, end: Int) {
    explicitSelectedImageRange = ImageSelectionRange(start, end)
    if (selectionStart == start && selectionEnd == end) {
        onSelectionOrContentMayChange?.invoke()
        return
    }
    setSelection(start, end)
}

internal fun EditorEditText.clearExplicitSelectedImageRange() {
    if (explicitSelectedImageRange == null) return
    explicitSelectedImageRange = null
    onSelectionOrContentMayChange?.invoke()
}

internal fun EditorEditText.updateImageSelectionHighlightAppearance(
    start: Int = selectionStart,
    end: Int = selectionEnd,
    focused: Boolean = isFocused,
) {
    val content = text as? Spanned
    val shouldSuppress = focused &&
        imageResizingEnabled &&
        content != null &&
        isExactImageSpanRange(content, start, end)
    if (shouldSuppress) {
        if (suppressedImageSelectionHighlightColor == null) {
            suppressedImageSelectionHighlightColor = highlightColor
            highlightColor = Color.TRANSPARENT
        }
        return
    }

    val restoredColor = suppressedImageSelectionHighlightColor ?: return
    suppressedImageSelectionHighlightColor = null
    highlightColor = restoredColor
}

internal fun EditorEditText.resolvedSelectedImageRange(spannable: Spanned): ImageSelectionRange? {
    explicitSelectedImageRange?.let { explicit ->
        if (isExactImageSpanRange(spannable, explicit.start, explicit.end)) {
            return explicit
        }
        explicitSelectedImageRange = null
    }

    val start = selectionStart
    val end = selectionEnd
    if (!isExactImageSpanRange(spannable, start, end)) return null
    return ImageSelectionRange(start, end)
}

internal fun EditorEditText.isExactImageSpanRange(spannable: Spanned, start: Int, end: Int): Boolean {
    if (start < 0 || end != start + 1) return false
    val imageSpan = spannable
        .getSpans(start, end, BlockImageSpan::class.java)
        .firstOrNull() ?: return false
    return spannable.getSpanStart(imageSpan) == start && spannable.getSpanEnd(imageSpan) == end
}

internal fun EditorEditText.imageSpanRangeNearTouchOffset(
    spannable: Spanned,
    x: Float,
    y: Float
): ImageSpanHit? {
    val safeOffset = runCatching { getOffsetForPosition(x, y) }.getOrNull() ?: return null
    val nearbyOffsets = linkedSetOf(
        safeOffset,
        (safeOffset - 1).coerceAtLeast(0),
        (safeOffset + 1).coerceAtMost(spannable.length)
    )
    for (offset in nearbyOffsets) {
        val searchStart = (offset - 1).coerceAtLeast(0)
        val searchEnd = (offset + 1).coerceAtMost(spannable.length)
        val imageSpan = spannable
            .getSpans(searchStart, searchEnd, BlockImageSpan::class.java)
            .firstOrNull() ?: continue
        val spanStart = spannable.getSpanStart(imageSpan)
        val spanEnd = spannable.getSpanEnd(imageSpan)
        if (spanStart >= 0 && spanEnd > spanStart) {
            return ImageSpanHit(imageSpan, spanStart, spanEnd)
        }
    }
    return null
}

internal fun EditorEditText.resolvedImageRect(
    textLayout: Layout,
    imageSpan: BlockImageSpan,
    spanStart: Int,
    spanEnd: Int
): RectF {
    (textLayout as? EditorDocumentLayout)?.imageBounds(imageSpan)?.let {
        it.offset(compoundPaddingLeft.toFloat(), extendedPaddingTop.toFloat())
        return it
    }
    imageSpan.currentDrawRect()?.let { drawnRect ->
        return drawnRect
    }

    val safeOffset = spanStart.coerceAtMost(maxOf((text?.length ?: 0) - 1, 0))
    val line = textLayout.getLineForOffset(safeOffset)
    val startHorizontal = textLayout.getPrimaryHorizontal(spanStart)
    val endHorizontal = textLayout.getPrimaryHorizontal(spanEnd)
    val (widthPx, heightPx) = imageSpan.currentSizePx()
    val left = compoundPaddingLeft + minOf(startHorizontal, endHorizontal)
    val right = compoundPaddingLeft + maxOf(
        maxOf(startHorizontal, endHorizontal),
        minOf(startHorizontal, endHorizontal) + widthPx
    )
    val top = extendedPaddingTop + textLayout.editorTextLineBottom(line) - heightPx
    return RectF(left, top.toFloat(), right, top + heightPx.toFloat())
}
