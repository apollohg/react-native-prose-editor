package com.apollohg.editor

import android.graphics.Matrix
import android.graphics.Rect
import android.graphics.RectF
import android.text.Layout
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CursorAnchorInfo

internal fun EditorEditText.buildSurfaceCursorAnchorInfo(): CursorAnchorInfo {
    val mapper = imeTextCoordinateMapperForEditor()
        ?: ImeTextCoordinateMapper.build(editableText, inputConnectionGeneration)
    val raw = editableText
    val layout = layout
    val originX = totalPaddingLeft.toFloat() - scrollX
    val originY = totalPaddingTop.toFloat() - scrollY
    val start = selectionStart.coerceIn(0, raw.length)
    val end = selectionEnd.coerceIn(0, raw.length)
    val builder = CursorAnchorInfo.Builder()
        .setMatrix(cursorScreenMatrix())
        .setSelectionRange(mapper.rawToIme(start), mapper.rawToIme(end))
    val visible = Rect()
    getLocalVisibleRect(visible)
    visible.offset(-scrollX, -scrollY)
    fun flags(left: Float, top: Float, right: Float, bottom: Float, rtl: Boolean): Int {
        val overlaps = left <= visible.right && right >= visible.left && top < visible.bottom && bottom > visible.top
        var result = if (overlaps) CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION else 0
        if (left < visible.left || right > visible.right || top < visible.top || bottom > visible.bottom || !overlaps) {
            result = result or CursorAnchorInfo.FLAG_HAS_INVISIBLE_REGION
        }
        if (rtl) result = result or CursorAnchorInfo.FLAG_IS_RTL
        return result
    }
    fun top(line: Int): Float = originY + ((layout as? EditorDocumentLayout)?.textLineTop(line) ?: layout.getLineTop(line))
    fun bottom(line: Int): Float = originY + ((layout as? EditorDocumentLayout)?.textLineBottom(line) ?: layout.getLineBottom(line))
    val line = layout.getLineForOffset(end)
    val x = originX + layout.getPrimaryHorizontal(end)
    val top = top(line)
    val bottom = bottom(line)
    builder.setInsertionMarkerLocation(x, top, originY + layout.getLineBaseline(line), bottom,
        flags(x, top, x, bottom, layout.getParagraphDirection(line) == Layout.DIR_RIGHT_TO_LEFT))
    val composingStart = BaseInputConnection.getComposingSpanStart(raw)
    val composingEnd = BaseInputConnection.getComposingSpanEnd(raw)
    if (composingStart >= 0 && composingEnd >= composingStart) {
        val imeStart = mapper.rawToIme(composingStart)
        val imeEnd = mapper.rawToIme(composingEnd)
        builder.setComposingText(imeStart, mapper.visibleText.subSequence(imeStart, imeEnd))
        var offset = imeStart
        while (offset < imeEnd) {
            val next = (offset + Character.charCount(Character.codePointAt(mapper.visibleText, offset))).coerceAtMost(imeEnd)
            val rawStart = mapper.imeToRaw(offset, ImeTextCoordinateMapper.Affinity.AFTER)
            val rawEnd = mapper.imeToRaw(next, ImeTextCoordinateMapper.Affinity.BEFORE)
            val characterLine = layout.getLineForOffset(rawStart)
            val leading = originX + layout.getPrimaryHorizontal(rawStart)
            val trailing = originX + if (layout.getLineForOffset(rawEnd) == characterLine) layout.getPrimaryHorizontal(rawEnd) else layout.getLineRight(characterLine)
            val bounds = RectF(minOf(leading, trailing), top(characterLine), maxOf(leading, trailing), bottom(characterLine))
            val characterFlags = flags(bounds.left, bounds.top, bounds.right, bounds.bottom, layout.isRtlCharAt(rawStart))
            for (unit in offset until next) builder.addCharacterBounds(unit, bounds.left, bounds.top, bounds.right, bounds.bottom, characterFlags)
            offset = next
        }
    }
    return builder.build()
}

private fun View.cursorScreenMatrix(): Matrix {
    val result = Matrix()
    fun append(view: View) {
        (view.parent as? View)?.let { parent ->
            append(parent)
            result.preTranslate(-parent.scrollX.toFloat(), -parent.scrollY.toFloat())
        }
        result.preTranslate(view.left.toFloat(), view.top.toFloat())
        result.preConcat(view.matrix)
    }
    append(this)
    val origin = floatArrayOf(0f, 0f)
    result.mapPoints(origin)
    val screen = IntArray(2)
    getLocationOnScreen(screen)
    result.postTranslate(screen[0] - origin[0], screen[1] - origin[1])
    return result
}
