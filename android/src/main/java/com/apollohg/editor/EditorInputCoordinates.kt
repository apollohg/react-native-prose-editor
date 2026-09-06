package com.apollohg.editor

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.text.Selection
import android.text.Spanned
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper


internal fun EditorInputConnection.isCurrentInputSession(): Boolean =
    editorView.isInputConnectionCurrentForEditor(boundEditorId, boundGeneration)

internal fun EditorInputConnection.currentMapper(): ImeTextCoordinateMapper? =
    editorView.imeTextCoordinateMapperForEditor(boundMapperGeneration)

internal fun EditorInputConnection.imeTextSlice(
    mapper: ImeTextCoordinateMapper,
    start: Int,
    end: Int,
    flags: Int,
): CharSequence {
    val slice = mapper.visibleText.subSequence(start, end)
    return if ((flags and InputConnection.GET_TEXT_WITH_STYLES) != 0 && slice is Spanned) {
        slice
    } else {
        slice.toString()
    }
}

internal fun EditorInputConnection.rawRangeForIme(start: Int, end: Int): Pair<Int, Int>? {
    val mapper = currentMapper() ?: return null
    if (start == end) {
        val raw = mapper.imeToRaw(start, ImeTextCoordinateMapper.Affinity.AFTER)
        return raw to raw
    }
    return if (start < end) {
        mapper.imeToRaw(start, ImeTextCoordinateMapper.Affinity.AFTER) to
            mapper.imeToRaw(end, ImeTextCoordinateMapper.Affinity.BEFORE)
    } else {
        mapper.imeToRaw(start, ImeTextCoordinateMapper.Affinity.BEFORE) to
            mapper.imeToRaw(end, ImeTextCoordinateMapper.Affinity.AFTER)
    }
}

internal fun EditorInputConnection.nanosToMicros(nanos: Long): Long = nanos / 1_000L

internal fun EditorInputConnection.isCurrentInputSessionFor(event: String): Boolean {
    val isCurrent = isCurrentInputSession()
    if (!isCurrent) {
        editorView.recordImeTraceForTesting(
            "${event}Ignored",
            "reason=stale boundEditor=$boundEditorId boundGen=$boundGeneration"
        )
    }
    return isCurrent
}

internal fun EditorInputConnection.refreshComposingTextFromEditable() {
    val editable = editorView.text ?: return
    val visibleReplacementText = editorView.composingTextFromVisibleReplacementForEditor()
    if (visibleReplacementText != null) {
        editorView.setComposingTextForEditor(
            ImeTextCoordinateMapper.build(visibleReplacementText, boundMapperGeneration)
                .visibleText
                .toString()
        )
        return
    }
    val start = BaseInputConnection.getComposingSpanStart(editable)
    val end = BaseInputConnection.getComposingSpanEnd(editable)
    if (start < 0 || end < 0 || start > end || end > editable.length) {
        editorView.setComposingTextForEditor(null)
        return
    }
    val mapper = currentMapper()
    val composingText = if (mapper != null) {
        mapper.visibleText.subSequence(
            mapper.rawToIme(start),
            mapper.rawToIme(end),
        ).toString()
    } else {
        editable.subSequence(start, end).toString()
    }
    editorView.setComposingTextForEditor(composingText)
}

internal fun EditorInputConnection.deleteTransientTextAroundSelection(beforeLength: Int, afterLength: Int): Boolean {
    val editable = editorView.text ?: return false
    val rawStart = editorView.selectionStart
    val rawEnd = editorView.selectionEnd
    if (rawStart < 0 || rawEnd < 0) return false
    val selectionStart = rawStart.coerceIn(0, editable.length)
    val selectionEnd = rawEnd.coerceIn(0, editable.length)
    val normalizedStart = minOf(selectionStart, selectionEnd)
    val normalizedEnd = maxOf(selectionStart, selectionEnd)
    val deleteStart: Int
    val deleteEnd: Int
    if (normalizedStart != normalizedEnd) {
        deleteStart = normalizedStart
        deleteEnd = normalizedEnd
    } else {
        deleteStart = maxOf(0, normalizedStart - beforeLength.coerceAtLeast(0))
        deleteEnd = minOf(editable.length, normalizedEnd + afterLength.coerceAtLeast(0))
    }
    if (deleteStart >= deleteEnd) return false
    val (snappedStart, snappedEnd) = PositionBridge.snapRangeToScalarBoundaries(
        deleteStart,
        deleteEnd,
        editable.toString()
    )
    if (snappedStart >= snappedEnd) return false
    editable.delete(snappedStart, snappedEnd)
    Selection.setSelection(editable, snappedStart.coerceIn(0, editable.length))
    return true
}

internal fun EditorInputConnection.deleteTransientTextAroundSelectionInCodePoints(
    beforeLength: Int,
    afterLength: Int
): Boolean {
    val currentText = editorView.text?.toString() ?: return false
    val rawStart = editorView.selectionStart
    val rawEnd = editorView.selectionEnd
    if (rawStart < 0 || rawEnd < 0) return false
    val selectionStart = rawStart.coerceIn(0, currentText.length)
    val selectionEnd = rawEnd.coerceIn(0, currentText.length)
    val normalizedStart = minOf(selectionStart, selectionEnd)
    val normalizedEnd = maxOf(selectionStart, selectionEnd)
    if (normalizedStart != normalizedEnd) {
        return deleteTransientTextAroundSelection(0, 0)
    }
    val beforeUtf16Length = EditorInputConnection.codePointsToUtf16Length(
        text = currentText,
        fromUtf16Offset = normalizedStart,
        codePointCount = beforeLength,
        forward = false
    )
    val afterUtf16Length = EditorInputConnection.codePointsToUtf16Length(
        text = currentText,
        fromUtf16Offset = normalizedEnd,
        codePointCount = afterLength,
        forward = true
    )
    return deleteTransientTextAroundSelection(beforeUtf16Length, afterUtf16Length)
}

internal fun EditorInputConnection.currentComposingSpanText(): String? {
    val editable = editorView.text ?: return null
    val start = BaseInputConnection.getComposingSpanStart(editable)
    val end = BaseInputConnection.getComposingSpanEnd(editable)
    if (start < 0 || end < 0 || start > end || end > editable.length) {
        return null
    }
    return editable.subSequence(start, end).toString()
}

internal fun EditorInputConnection.currentComposingSpanRange(): Pair<Int, Int>? {
    if (!editorView.isCurrentTextAuthorizedForEditor()) return null
    val editable = editorView.text ?: return null
    val start = BaseInputConnection.getComposingSpanStart(editable)
    val end = BaseInputConnection.getComposingSpanEnd(editable)
    if (start < 0 || end < 0 || start > end || end > editable.length) {
        return null
    }
    return editorView.authorizedUtf16Range(start, end)
}

internal fun EditorInputConnection.currentComposingSpanRawRange(): Pair<Int, Int>? {
    val editable = editorView.text ?: return null
    val start = BaseInputConnection.getComposingSpanStart(editable)
    val end = BaseInputConnection.getComposingSpanEnd(editable)
    if (start < 0 || end < 0 || start > end || end > editable.length) {
        return null
    }
    return start to end
}

/**
 * Called for hardware keyboard key events.
 *
 * Intercepts DEL (backspace) and ENTER to route through Rust. Other key
 * events are passed through to the base connection.
 */
