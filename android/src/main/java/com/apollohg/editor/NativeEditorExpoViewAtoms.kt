package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Rect
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import kotlin.math.abs

internal fun NativeEditorExpoView.atomChildAtImpl(index: Int): View? = reactChildren.getOrNull(index)

internal fun NativeEditorExpoView.addAtomChildImpl(child: View, index: Int) {
    reactChildren.remove(child)
    reactChildren.add(index.coerceIn(0, reactChildren.size), child)
    val atomKey = atomKey(child)
    if (atomKey == null) {
        (child.parent as? ViewGroup)?.removeView(child)
        addNativeAtomView(child, childCount)
        return
    }
    (child.parent as? ViewGroup)?.removeView(child)
    addNativeAtomView(child, childCount)
    richTextView.mountAtomChild(child, atomKey)
}

internal fun NativeEditorExpoView.removeAtomChildAtImpl(index: Int) {
    reactChildren.getOrNull(index)?.let(::removeAtomChild)
}

internal fun NativeEditorExpoView.removeAtomChildImpl(child: View) {
    reactChildren.remove(child)
    if (!richTextView.unmountAtomChild(child)) {
        (child.parent as? ViewGroup)?.removeView(child)
    }
}

internal fun NativeEditorExpoView.drawAtomHostsIntoScrollView(canvas: Canvas, drawingTime: Long) {
    val scrollView = richTextView.editorScrollView
    val scrollBounds = Rect(0, 0, scrollView.width, scrollView.height)
    offsetDescendantRectToMyCoords(scrollView, scrollBounds)
    val saveCount = canvas.save()
    canvas.clipRect(
        scrollView.scrollX,
        scrollView.scrollY,
        scrollView.scrollX + scrollView.width,
        scrollView.scrollY + scrollView.height,
    )
    canvas.translate(-scrollBounds.left.toFloat(), -scrollBounds.top.toFloat())
    reactChildren.forEach { child ->
        if (
            child.parent === this &&
            atomKey(child) != null &&
            child.visibility == View.VISIBLE
        ) {
            drawNativeAtomView(canvas, child, drawingTime)
        }
    }
    canvas.restoreToCount(saveCount)
}

internal fun NativeEditorExpoView.atomChildAt(x: Float, y: Float): View? = reactChildren.lastOrNull { child ->
    atomKey(child) != null &&
        child.visibility == View.VISIBLE &&
        x >= child.x &&
        x < child.x + child.width &&
        y >= child.y &&
        y < child.y + child.height
}

internal fun NativeEditorExpoView.atomScrollMovedVerticallyBeyondSlop(event: MotionEvent): Boolean {
    val dx = abs(event.x - atomScrollDownX)
    val dy = abs(event.y - atomScrollDownY)
    return dy > atomScrollTouchSlopPx && dy > dx
}

internal fun NativeEditorExpoView.atomScrollMovedHorizontallyBeyondSlop(event: MotionEvent): Boolean {
    val dx = abs(event.x - atomScrollDownX)
    val dy = abs(event.y - atomScrollDownY)
    return dx > atomScrollTouchSlopPx && dx >= dy
}

internal fun NativeEditorExpoView.dispatchAtomScrollTouch(event: MotionEvent): Boolean {
    atomScrollTouchDispatchCountForTesting += 1
    val editorLocation = IntArray(2)
    val scrollLocation = IntArray(2)
    getLocationOnScreen(editorLocation)
    richTextView.editorScrollView.getLocationOnScreen(scrollLocation)
    val scrollEvent = MotionEvent.obtain(event)
    scrollEvent.offsetLocation(
        (editorLocation[0] - scrollLocation[0]).toFloat(),
        (editorLocation[1] - scrollLocation[1]).toFloat(),
    )
    val handled = richTextView.editorScrollView.onTouchEvent(scrollEvent)
    scrollEvent.recycle()
    return handled
}

internal fun NativeEditorExpoView.emitAtomLayout(widthPx: Float, positions: List<AtomLayoutPosition>) {
    val density = resources.displayMetrics.density.takeIf { it > 0f } ?: 1f
    val event = mapOf<String, Any>(
        "width" to widthPx / density,
        "positions" to positions.map { position ->
            mapOf(
                "key" to position.key,
                "x" to position.xPx / density,
                "y" to position.yPx / density,
                "height" to position.heightPx / density,
            )
        },
        "viewport" to mapOf(
            "y" to richTextView.editorScrollView.top / density,
            "height" to richTextView.editorScrollView.height / density,
        ),
        "editorId" to eventEditorId(richTextView.editorId)
    )
    onAtomLayoutForTesting?.invoke(event) ?: onAtomLayout(event)
}

internal fun NativeEditorExpoView.atomKey(child: View): String? {
    val nativeId = child.getTag(com.facebook.react.R.id.view_tag_native_id) as? String
    if (nativeId == null || !nativeId.startsWith(ATOM_NATIVE_ID_PREFIX)) return null
    return nativeId.removePrefix(ATOM_NATIVE_ID_PREFIX).takeIf(String::isNotEmpty)
}
