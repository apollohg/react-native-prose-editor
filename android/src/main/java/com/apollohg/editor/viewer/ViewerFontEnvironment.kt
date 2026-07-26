package com.apollohg.editor.viewer

import android.content.res.Configuration
import android.graphics.Typeface
import android.util.Log

/** Tracks process font availability and system scale without touching a frozen artifact. */
internal class ViewerFontEnvironment {
    companion object {
        private val warningLock = Any()
        private val missingWarnings = mutableSetOf<String>()
        private val familyLock = Any()
        private val registeredFamilies = mutableMapOf<String, Typeface>()
        private val demonstrablyMissingFamilies = mutableSetOf<String>()

        internal data class ResolvedFamily(val typeface: Typeface, val isDemonstrablyMissing: Boolean)

        /**
         * Font loaders that know a custom family is available register the
         * actual Typeface here. Typeface.create silently falls back, so it is
         * not evidence that a custom family is missing.
         */
        @JvmStatic fun registerAvailableFamily(family: String, typeface: Typeface) {
            val normalized = family.trim()
            if (normalized.isEmpty()) return
            synchronized(familyLock) {
                registeredFamilies[normalized] = typeface
                demonstrablyMissingFamilies.remove(normalized)
            }
        }

        /** Loader failures may opt into an absence warning on the next layout. */
        @JvmStatic fun markFamilyUnavailable(family: String) {
            val normalized = family.trim()
            if (normalized.isEmpty()) return
            synchronized(familyLock) {
                registeredFamilies.remove(normalized)
                demonstrablyMissingFamilies += normalized
            }
        }

        internal fun resolveFamily(family: String?, style: Int, fallback: Typeface): ResolvedFamily {
            val normalized = family?.trim().orEmpty()
            if (normalized.isEmpty()) return ResolvedFamily(Typeface.create(fallback, style), false)
            synchronized(familyLock) {
                registeredFamilies[normalized]?.let { return ResolvedFamily(Typeface.create(it, style), false) }
                if (normalized in demonstrablyMissingFamilies) {
                    return ResolvedFamily(Typeface.create(fallback, style), true)
                }
            }
            // System and unknown custom families are both allowed through. A
            // fallback-looking Typeface is not proof of absence on Android.
            return ResolvedFamily(Typeface.create(normalized, style), false)
        }

        internal fun resetFamilyRegistryForTesting() = synchronized(familyLock) {
            registeredFamilies.clear()
            demonstrablyMissingFamilies.clear()
        }
        fun warnOnceForMissingFamily(family: String, semanticGeneration: String, revision: String): Boolean {
            val key = "$revision\u001f$semanticGeneration\u001f$family"
            val shouldWarn = synchronized(warningLock) {
                val inserted = missingWarnings.add(key)
                while (missingWarnings.size > 512) missingWarnings.minOrNull()?.let(missingWarnings::remove)
                inserted
            }
            if (shouldWarn) {
                Log.w("NativeEditorImage", "PreparedProseViewer: requested font family $family is unavailable; using system fallback")
            }
            return shouldWarn
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
