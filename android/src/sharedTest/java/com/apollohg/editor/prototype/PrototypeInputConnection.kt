package com.apollohg.editor.prototype

import android.text.Editable
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.TextAttribute

internal class PrototypeInputConnection(private val targetView: View, private val session: PrototypeDocumentSession) : BaseInputConnection(targetView, true) {
    private val generation = session.acquireConnection()
    internal val isActive: Boolean get() = session.isCurrent(generation)
    private val active: Boolean get() = isActive

    override fun getEditable(): Editable? = if (active) session.editable else null
    override fun beginBatchEdit(): Boolean = active && session.beginBatch()
    override fun endBatchEdit(): Boolean = active && session.endBatch()

    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!active || text == null || !validUtf16(text)) return false
        return super.commitText(text, newCursorPosition) && session.changed(commit = true)
    }

    override fun replaceText(start: Int, end: Int, text: CharSequence, newCursorPosition: Int, textAttribute: TextAttribute?): Boolean {
        if (!active || start < 0 || end < 0 || !validUtf16(text)) return false
        val from = session.boundary(minOf(start, end))
        val to = session.boundary(maxOf(start, end), forward = true)
        return super.replaceText(from, to, text, newCursorPosition, textAttribute) && session.changed(commit = true)
    }

    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!active || text == null || !validUtf16(text)) return false
        return super.setComposingText(text, newCursorPosition) && session.changed(commit = false)
    }

    override fun setComposingRegion(start: Int, end: Int): Boolean {
        if (!active) return false
        return super.setComposingRegion(session.boundary(start), session.boundary(end, forward = true)) && session.changed(commit = false)
    }

    override fun finishComposingText(): Boolean = active && super.finishComposingText() && session.changed(commit = true)

    override fun setSelection(start: Int, end: Int): Boolean {
        if (!active || start !in 0..session.editable.length || end !in 0..session.editable.length) return false
        session.setSelection(start, end)
        return true
    }

    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (!active || beforeLength < 0 || afterLength < 0) return false
        val composingStart = getComposingSpanStart(session.editable)
        val composingEnd = getComposingSpanEnd(session.editable)
        val start = minOf(session.selectionStart, session.selectionEnd, if (composingStart < 0) Int.MAX_VALUE else composingStart)
        val end = maxOf(session.selectionStart, session.selectionEnd, composingEnd)
        val before = start - session.boundary((start.toLong() - beforeLength).coerceAtLeast(0).toInt())
        val after = session.boundary((end.toLong() + afterLength).coerceAtMost(session.editable.length.toLong()).toInt(), forward = true) - end
        return super.deleteSurroundingText(before, after) && session.changed(commit = true)
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
        if (!active || beforeLength < 0 || afterLength < 0) return false
        return super.deleteSurroundingTextInCodePoints(beforeLength, afterLength) && session.changed(commit = true)
    }

    override fun commitCorrection(correctionInfo: CorrectionInfo?): Boolean {
        // The IME commits corrected text separately; this only acknowledges it.
        return active && correctionInfo != null
    }

    override fun getExtractedText(request: ExtractedTextRequest?, flags: Int): ExtractedText? {
        if (!active) return null
        return ExtractedText().apply {
            text = session.editable.toString()
            startOffset = 0
            partialStartOffset = -1
            partialEndOffset = -1
            selectionStart = session.selectionStart
            selectionEnd = session.selectionEnd
        }
    }

    override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean = active && (targetView as? PrototypeCursorReporter)?.requestCursorUpdates(cursorUpdateMode) == true
    override fun sendKeyEvent(event: KeyEvent?): Boolean = active && event != null && targetView.dispatchKeyEvent(event)
    override fun closeConnection() = session.retireConnection(generation)

    private fun validUtf16(text: CharSequence): Boolean {
        var index = 0
        while (index < text.length) {
            val char = text[index++]
            if (Character.isHighSurrogate(char)) {
                if (index == text.length || !Character.isLowSurrogate(text[index++])) return false
            } else if (Character.isLowSurrogate(char)) return false
        }
        return true
    }
}
