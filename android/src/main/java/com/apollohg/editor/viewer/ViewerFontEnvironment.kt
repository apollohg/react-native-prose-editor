package com.apollohg.editor.viewer

import android.content.res.Configuration
import android.util.Log

/** Tracks process font availability and system scale without touching a frozen artifact. */
internal class ViewerFontEnvironment {
    companion object {
        private val warningLock = Any()
        private val missingWarnings = mutableSetOf<String>()
        fun warnOnceForMissingFamily(family: String, semanticGeneration: String, revision: String) {
            val key = "$revision\u001f$semanticGeneration\u001f$family"
            val shouldWarn = synchronized(warningLock) {
                val inserted = missingWarnings.add(key)
                while (missingWarnings.size > 512) missingWarnings.minOrNull()?.let(missingWarnings::remove)
                inserted
            }
            if (shouldWarn) {
                Log.w("NativeEditorImage", "PreparedProseViewer: requested font family $family is unavailable; using system fallback")
            }
        }
    }
    private val lock = Any()
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

    private fun invalidate() {
        val next = synchronized(lock) {
            revision += 1
            revision
        }
        onInvalidated?.invoke(next)
    }
}
