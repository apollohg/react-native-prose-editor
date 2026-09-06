package com.apollohg.editor

import android.content.Context
import android.text.TextUtils
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.view.inputmethod.SurroundingText
import android.view.inputmethod.TextSnapshot

internal fun EditorInputConnection.extractedTextForIme(request: ExtractedTextRequest?, flags: Int): ExtractedText? {
    val mapper = currentMapper() ?: return null
    if (request != null && flags and InputConnection.GET_EXTRACTED_TEXT_MONITOR != 0) {
        extractedTextRequest = ExtractedTextRequest().apply {
            token = request.token
            this.flags = request.flags
            hintMaxLines = request.hintMaxLines
            hintMaxChars = request.hintMaxChars
        }
    }
    return buildExtractedText(mapper, request?.flags ?: 0).also {
        if (flags and InputConnection.GET_EXTRACTED_TEXT_MONITOR != 0) lastPublishedExtractedText = it
    }
}

private fun EditorInputConnection.buildExtractedText(mapper: ImeTextCoordinateMapper, flags: Int): ExtractedText =
    ExtractedText().apply {
        text = imeTextSlice(mapper, 0, mapper.visibleText.length, flags)
        startOffset = 0
        partialStartOffset = -1
        partialEndOffset = -1
        selectionStart = mapper.rawToIme(editorView.selectionStart.coerceAtLeast(0))
        selectionEnd = mapper.rawToIme(editorView.selectionEnd.coerceAtLeast(0))
        this.flags = if (editorView.inputType and android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE == 0) ExtractedText.FLAG_SINGLE_LINE else 0
    }

internal fun EditorInputConnection.surroundingTextForIme(beforeLength: Int, afterLength: Int, flags: Int): SurroundingText? {
    if (beforeLength < 0 || afterLength < 0) return null
    val mapper = currentMapper() ?: return null
    if (editorView.selectionStart < 0 || editorView.selectionEnd < 0) return null
    val from = mapper.rawToIme(minOf(editorView.selectionStart, editorView.selectionEnd))
    val to = mapper.rawToIme(maxOf(editorView.selectionStart, editorView.selectionEnd))
    val start = (from.toLong() - beforeLength).coerceAtLeast(0).toInt()
    val end = (to.toLong() + afterLength).coerceAtMost(mapper.visibleText.length.toLong()).toInt()
    return SurroundingText(imeTextSlice(mapper, start, end, flags), from - start, to - start, start)
}

internal fun EditorInputConnection.snapshotForIme(): TextSnapshot? {
    val surrounding = surroundingTextForIme(1024, 1024, InputConnection.GET_TEXT_WITH_STYLES) ?: return null
    val mapper = currentMapper() ?: return null
    val composing = composingSelectionForIme(mapper)
    return TextSnapshot(surrounding, composing.first, composing.second,
        getCursorCapsMode(TextUtils.CAP_MODE_CHARACTERS or TextUtils.CAP_MODE_WORDS or TextUtils.CAP_MODE_SENTENCES))
}

private fun EditorInputConnection.composingSelectionForIme(mapper: ImeTextCoordinateMapper): Pair<Int, Int> {
    val editable = editorView.editableText
    val start = BaseInputConnection.getComposingSpanStart(editable)
    val end = BaseInputConnection.getComposingSpanEnd(editable)
    if (start < 0 || end < 0) return -1 to -1
    return mapper.rawToIme(minOf(start, end)) to mapper.rawToIme(maxOf(start, end))
}

internal fun EditorInputConnection.publishInputStateIfNeeded() {
    val mapper = currentMapper() ?: return
    val composing = composingSelectionForIme(mapper)
    val selection = listOf(
        mapper.rawToIme(editorView.selectionStart.coerceAtLeast(0)),
        mapper.rawToIme(editorView.selectionEnd.coerceAtLeast(0)), composing.first, composing.second)
    if (selection != lastPublishedSelection) {
        lastPublishedSelection = selection
        val manager = editorView.context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        manager?.updateSelection(editorView, selection[0], selection[1], selection[2], selection[3])
    }
    publishExtractedTextIfNeeded()
}

internal fun EditorInputConnection.publishExtractedTextIfNeeded() {
    val request = extractedTextRequest ?: return
    val mapper = currentMapper() ?: return
    val next = buildExtractedText(mapper, request.flags)
    val previous = lastPublishedExtractedText
    if (previous != null && previous.text == next.text && previous.selectionStart == next.selectionStart &&
        previous.selectionEnd == next.selectionEnd && previous.flags == next.flags) return
    lastPublishedExtractedText = next
    val manager = editorView.context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
    manager?.updateExtractedText(editorView, request.token, next)
}
