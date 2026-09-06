package com.apollohg.editor

import android.os.Handler
import android.text.Editable
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.InputConnection

internal class EditorSurfaceInputConnection(private val surface: EditorTextSurface) : BaseInputConnection(surface, true) {
    private var closed = false
    private var batchDepth = 0

    override fun getEditable(): Editable? = if (closed) null else surface.editableText

    override fun beginBatchEdit(): Boolean {
        if (closed) return false
        batchDepth++
        return surface.beginBatchEdit()
    }

    override fun endBatchEdit(): Boolean {
        if (batchDepth == 0) return false
        batchDepth--
        surface.endBatchEdit()
        return batchDepth > 0
    }

    override fun sendKeyEvent(event: KeyEvent): Boolean = !closed && surface.dispatchKeyEvent(event)

    override fun performContextMenuAction(id: Int): Boolean = !closed && surface.onTextContextMenuItem(id)

    override fun commitCorrection(correctionInfo: CorrectionInfo?): Boolean = !closed

    override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean {
        val modes = InputConnection.CURSOR_UPDATE_IMMEDIATE or InputConnection.CURSOR_UPDATE_MONITOR
        return requestCursorUpdates(cursorUpdateMode and modes, cursorUpdateMode and modes.inv())
    }

    override fun requestCursorUpdates(cursorUpdateMode: Int, cursorUpdateFilter: Int): Boolean {
        val supported = InputConnection.CURSOR_UPDATE_FILTER_INSERTION_MARKER or InputConnection.CURSOR_UPDATE_FILTER_CHARACTER_BOUNDS
        if (closed || cursorUpdateFilter and supported.inv() != 0) return false
        return surface.requestSurfaceCursorUpdates(cursorUpdateMode)
    }

    override fun getHandler(): Handler? = surface.handler

    override fun closeConnection() {
        if (closed) return
        closed = true
        while (batchDepth > 0) endBatchEdit()
        super.closeConnection()
    }
}
