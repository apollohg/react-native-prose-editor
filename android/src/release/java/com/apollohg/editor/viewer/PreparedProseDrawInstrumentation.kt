package com.apollohg.editor.viewer

/**
 * Release deliberately inlines only the drawing block. This keeps collector
 * calls and instrumentation branches out of the release drawing hot path.
 */
internal inline fun recordPreparedProseDraw(draw: () -> Int) {
    draw()
}
