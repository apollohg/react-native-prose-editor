package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.RectF
import android.os.Build
import android.util.TypedValue

internal fun EditorEditText.setEditorAccessibilityHintImpl(hint: CharSequence?) {
    editorAccessibilityHint = hint
}

internal fun EditorEditText.nativeCursorDrawRectImpl(): RectF? {
    if (isCollapsedAtomBoundarySelection(selectionStart, selectionEnd)) {
        isCursorVisible = false
        return null
    }
    val textLayout = layout ?: return null
    val offset = selectionEnd.coerceIn(0, textLayout.text.length)
    val bounds = CaretGeometry.verticalBounds(textLayout, offset, paint, textLayout.text)
    val left = textLayout.getPrimaryHorizontal(offset)
    return RectF(left, bounds.top, left + caretWidthPx, bounds.bottom)
}

    /**
     * The native caret is tinted by the theme's `colorControlActivated`; resolve
     * the same value so the replacement keeps the platform appearance, falling
     * back to the text color when the attribute is not a color.
     */
internal fun EditorEditText.resolveCaretColor(): Int {
    val resolved = TypedValue()
    val found = context.theme.resolveAttribute(
        android.R.attr.colorControlActivated,
        resolved,
        true
    )
    val isColor = resolved.type >= TypedValue.TYPE_FIRST_COLOR_INT &&
        resolved.type <= TypedValue.TYPE_LAST_COLOR_INT
    return if (found && isColor) resolved.data else currentTextColor
}

internal fun EditorEditText.clipLegacyNativeCursorTail(canvas: Canvas) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) return
    if (!CaretGeometry.shouldRender(isFocused, hasWindowFocus(), selectionStart, selectionEnd)) return
    val textLayout = layout ?: return
    val offset = selectionEnd.coerceIn(0, textLayout.text.length)
    val bounds = CaretGeometry.verticalBounds(textLayout, offset, paint, textLayout.text)
    val lineBottom = textLayout.editorTextLineBottom(textLayout.getLineForOffset(offset)).toFloat()
    if (bounds.bottom >= lineBottom) return

    val centerX = totalPaddingLeft + textLayout.getPrimaryHorizontal(offset)
    val halfWidth = maxOf(caretWidthPx, 2f * resources.displayMetrics.density)
    legacyCursorClipPaint.color = baseBackgroundColor
    canvas.drawRect(
        centerX - halfWidth,
        totalPaddingTop + bounds.bottom,
        centerX + halfWidth,
        totalPaddingTop + lineBottom,
        legacyCursorClipPaint
    )
}
