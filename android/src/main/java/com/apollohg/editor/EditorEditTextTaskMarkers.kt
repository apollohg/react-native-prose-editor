package com.apollohg.editor

import com.apollohg.editor.EditorEditText.Companion.MARKER_TAP_HORIZONTAL_SLOP_DP
import android.text.Spanned
import android.view.MotionEvent

/**
     * Task-marker taps are recognized as a paired DOWN+UP on the same marker
     * within touch slop. DOWN is never consumed (so scrolls and selection
     * gestures that start on a checkbox keep working); only the matching UP
     * is consumed.
     */
internal fun EditorEditText.handleTaskListMarkerTap(event: MotionEvent): Boolean {
    when (event.actionMasked) {
        MotionEvent.ACTION_DOWN -> {
            pendingTaskMarkerDownScalar = taskListMarkerScalarHitAt(event.x, event.y)
            pendingTaskMarkerDownX = event.x
            pendingTaskMarkerDownY = event.y
            return false
        }
        MotionEvent.ACTION_MOVE -> {
            if (pendingTaskMarkerDownScalar != null && !withinTouchSlop(event)) {
                pendingTaskMarkerDownScalar = null
            }
            return false
        }
        MotionEvent.ACTION_UP -> {
            val downScalar = pendingTaskMarkerDownScalar ?: return false
            pendingTaskMarkerDownScalar = null
            if (!withinTouchSlop(event)) return false
            val upScalar = taskListMarkerScalarHitAt(event.x, event.y) ?: return false
            if (upScalar != downScalar) return false
            if (!commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
            val authoritativeScalar = taskListMarkerScalarHitAt(event.x, event.y) ?: return true
            requestFocus()
            toggleTaskItemCheckedAtSelectionScalarInRust(
                authoritativeScalar,
                authoritativeScalar
            )
            performClick()
            return true
        }
        MotionEvent.ACTION_CANCEL -> {
            pendingTaskMarkerDownScalar = null
            return false
        }
        else -> return false
    }
}

internal fun EditorEditText.withinTouchSlop(event: MotionEvent): Boolean {
    return kotlin.math.abs(event.x - pendingTaskMarkerDownX) <= touchSlopPx &&
        kotlin.math.abs(event.y - pendingTaskMarkerDownY) <= touchSlopPx
}

internal fun EditorEditText.taskListMarkerScalarHitAt(x: Float, y: Float): Int? {
    val spanned = text as? Spanned ?: return null
    val textLayout = layout ?: return null
    if (spanned.isEmpty()) return null

    val localX = x + scrollX - totalPaddingLeft
    val localY = y + scrollY - totalPaddingTop
    if (localY < 0) return null

    val line = textLayout.getLineForVertical(localY.toInt().coerceAtLeast(0))
    val lineTop = textLayout.editorTextLineTop(line).toFloat()
    val lineBottom = textLayout.editorTextLineBottom(line).toFloat()
    if (localY < lineTop || localY > lineBottom) {
        return null
    }
    val lineStart = textLayout.getLineStart(line).coerceIn(0, spanned.length)
    val lineEnd = textLayout.getLineEnd(line).coerceIn(lineStart, spanned.length)
    val markerEnd = renderedTaskListMarkerEnd(spanned, lineStart, lineEnd) ?: return null
    val markerRight = textLayout.getPrimaryHorizontal(markerEnd).coerceAtLeast(
        textLayout.getPrimaryHorizontal(lineStart)
    ) + MARKER_TAP_HORIZONTAL_SLOP_DP * resources.displayMetrics.density
    if (localX > markerRight) {
        return null
    }

    // Confirmed hit: only now pay for the String conversion the
    // PositionBridge scalar math requires.
    val currentText = spanned.toString()
    val snappedUtf16 = PositionBridge.snapToScalarBoundary(
        lineStart,
        currentText,
        biasForward = true
    )
    return PositionBridge.utf16ToScalar(snappedUtf16, currentText)
}
