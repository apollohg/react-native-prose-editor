package com.apollohg.editor.viewer

/** Debug/androidTest-only draw timing, inlined into [PreparedProseDrawingView]. */
internal inline fun recordPreparedProseDraw(draw: () -> Int) {
    val started = PreparedProseInstrumentation.now()
    PreparedProseInstrumentation.drew(started, draw())
}
