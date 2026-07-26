package com.apollohg.editor.viewer

import android.content.res.Configuration
import android.util.Log

/** Tracks process font availability and system scale without touching a frozen artifact. */
internal class ViewerFontEnvironment {
    companion object {
        private val warningLock = Any()
        private val missingWarnings = mutableSetOf<String>()
        fun warnOnceForMissingFamily(family: String, semanticGeneration: String, revision: Long) {
            val key = "$revision\u001f$semanticGeneration\u001f$family"
            if (synchronized(warningLock) { missingWarnings.add(key) }) {
                Log.w("NativeEditorImage", "PreparedProseViewer: requested font family $family is unavailable; using system fallback")
            }
        }
    }
    private val lock = Any()
    private val warned = mutableSetOf<String>()
    private var fontScale = Float.NaN
    var revision: Long = 0
        private set
    var onInvalidated: ((Long) -> Unit)? = null

    fun invalidateRegisteredFonts() = invalidate()

    fun onConfigurationChanged(configuration: Configuration) {
        val next = configuration.fontScale
        synchronized(lock) {
            if (next.isFinite() && next > 0f && next == fontScale) return
            fontScale = next
        }
        invalidate()
    }

    fun shouldWarnForMissingFamily(family: String, semanticGeneration: String): Boolean = synchronized(lock) {
        warned.add("$revision\u001f$semanticGeneration\u001f$family")
    }

    private fun invalidate() {
        val next = synchronized(lock) {
            revision += 1
            warned.clear()
            revision
        }
        onInvalidated?.invoke(next)
    }
}
