package com.apollohg.editor.prototype

internal interface PrototypeCursorReporter {
    fun requestCursorUpdates(mode: Int): Boolean
}
