package com.apollohg.editor

import android.graphics.Rect
import android.view.View

internal fun EditorEditText.resolveAutoGrowHeightImpl(): Int {
    val availableWidth = (measuredWidth - compoundPaddingLeft - compoundPaddingRight).coerceAtLeast(0)
    val placeholderHeight = resolvePlaceholderHeightForAvailableWidth(availableWidth)
    val laidOutTextHeight = if (isLaidOut) layout?.height else null
    if (laidOutTextHeight != null && laidOutTextHeight > 0) {
        return maxOf(
            laidOutTextHeight + compoundPaddingTop + compoundPaddingBottom,
            placeholderHeight ?: 0
        )
    }

    val currentText = text
    if (availableWidth > 0 && currentText != null) {
        val staticLayout = EditorDocumentLayout(currentText, paint, availableWidth, includeFontPadding)
        val textHeight = staticLayout.height.takeIf { it > 0 } ?: lineHeight
        return maxOf(
            textHeight + compoundPaddingTop + compoundPaddingBottom,
            placeholderHeight ?: 0
        )
    }

    val minimumHeight = editorSuggestedMinimumHeight.coerceAtLeast(minHeight)
    return maxOf(
        placeholderHeight ?: 0,
        (lineHeight + compoundPaddingTop + compoundPaddingBottom).coerceAtLeast(minimumHeight)
    )
}

internal fun EditorEditText.preserveScrollPosition(previousScrollX: Int, previousScrollY: Int) {
    val restore = {
        val maxScrollX = maxOf(0, editorHorizontalScrollRange() - width)
        val maxScrollY = maxOf(0, editorVerticalScrollRange() - height)
        scrollTo(
            previousScrollX.coerceIn(0, maxScrollX),
            previousScrollY.coerceIn(0, maxScrollY)
        )
    }

    restore()
    post { restore() }
}

internal fun EditorEditText.ensureSelectionVisible() {
    if (!hasFocus()) return
    if (!isLaidOut || width <= 0 || height <= 0) return
    if (selectionEnd < 0 || caretVisibilityRequestPosted) return

    caretVisibilityRequestPosted = true
    val posted = post {
        caretVisibilityRequestPosted = false
        if (!hasFocus() || !isLaidOut || layout == null) return@post
        val selectionOffset = selectionEnd.takeIf { it >= 0 } ?: return@post
        val viewportBottomClearance = resolveViewportBottomClearancePx()
        if (heightBehavior == EditorHeightBehavior.FIXED) {
            bringPointIntoView(selectionOffset)
        }

        val textLayout = layout ?: return@post
        val clampedOffset = selectionOffset.coerceAtMost(textLayout.text.length)
        val line = textLayout.getLineForOffset(clampedOffset)
        val caretLeft = textLayout.getPrimaryHorizontal(clampedOffset).toInt()
        val rect = Rect(
            caretLeft + totalPaddingLeft,
            textLayout.editorTextLineTop(line) + totalPaddingTop,
            caretLeft + totalPaddingLeft + 1,
            textLayout.editorTextLineBottom(line) + totalPaddingTop + viewportBottomClearance
        )
        requestRectangleOnScreen(rect)
    }
    if (!posted) {
        caretVisibilityRequestPosted = false
    }
}

internal fun EditorEditText.resolveViewportBottomClearancePx(): Int {
    val occlusionTop = viewportBottomOcclusionTopOnScreenPx ?: return viewportBottomInsetPx
    var ancestor = parent
    var foundScrollableAncestor = false
    var clearance = 0
    while (ancestor is View) {
        if (ancestor.canScrollVertically(-1) || ancestor.canScrollVertically(1)) {
            ancestor.getLocationOnScreen(caretVisibilityLocationOnScreen)
            foundScrollableAncestor = true
            clearance = maxOf(
                clearance,
                caretVisibilityLocationOnScreen[1] + ancestor.height - occlusionTop
            )
        }
        ancestor = (ancestor as View).parent
    }
    return if (foundScrollableAncestor) clearance.coerceAtLeast(0) else viewportBottomInsetPx
}
