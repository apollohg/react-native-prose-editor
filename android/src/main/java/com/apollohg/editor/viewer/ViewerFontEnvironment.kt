package com.apollohg.editor.viewer

import android.content.res.Configuration
import android.graphics.Typeface
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.lang.ref.WeakReference

/** Tracks font availability and system scale without mutating a frozen artifact. */
internal class ViewerFontEnvironment {
    companion object {
        private val warningLock = Any()
        private val missingWarnings = mutableSetOf<String>()
        private val familyLock = Any()
        private val registeredFamilies = mutableMapOf<String, Typeface>()
        private val demonstrablyMissingFamilies = mutableSetOf<String>()
        private val familyObservers = FamilyObservers()
        private val platformFamilyNames = setOf(
            "default", "sans", "sans-serif", "serif", "monospace", "cursive", "casual",
            "sans-serif-smallcaps", "sans-serif-condensed", "sans-serif-light", "sans-serif-medium",
            "sans-serif-black", "sans-serif-thin", "sans-serif-condensed-light",
        )
        private var platformResolverForTesting: ((String, Int) -> Typeface)? = null

        internal data class ResolvedFamily(val typeface: Typeface, val isDemonstrablyMissing: Boolean)

        /**
         * Custom-family loaders report the actual Typeface. A real registry
         * change is delivered once on the main thread to every active direct
         * or Fabric environment; weak registrations prevent host retention.
         */
        @JvmStatic fun registerAvailableFamily(family: String, typeface: Typeface) {
            val normalized = family.trim()
            if (normalized.isEmpty()) return
            val changed = synchronized(familyLock) {
                val previous = registeredFamilies.put(normalized, typeface)
                val removedFailure = demonstrablyMissingFamilies.remove(normalized)
                previous !== typeface || removedFailure
            }
            if (changed) familyObservers.publish()
        }

        /** Explicit loader failure also replaces mounted fallback layouts once. */
        @JvmStatic fun markFamilyUnavailable(family: String) {
            val normalized = family.trim()
            if (normalized.isEmpty()) return
            val changed = synchronized(familyLock) {
                val removed = registeredFamilies.remove(normalized) != null
                val added = demonstrablyMissingFamilies.add(normalized)
                removed || added
            }
            if (changed) familyObservers.publish()
        }

        internal fun resolveFamily(family: String?, style: Int, fallback: Typeface): ResolvedFamily {
            val normalized = family?.trim().orEmpty()
            if (normalized.isEmpty()) return ResolvedFamily(Typeface.create(fallback, style), false)
            synchronized(familyLock) {
                registeredFamilies[normalized]?.let { return ResolvedFamily(Typeface.create(it, style), false) }
                if (normalized in demonstrablyMissingFamilies) return ResolvedFamily(Typeface.create(fallback, style), true)
            }
            // Typeface.create silently returns the platform default for an
            // unknown family. Compare that result to the same styled default;
            // known Android generic family aliases are valid even when they
            // intentionally resolve to that default. Registrations above win
            // before this comparison so custom families never false-warn.
            val resolved = synchronized(familyLock) {
                platformResolverForTesting?.invoke(normalized, style)
            } ?: Typeface.create(normalized, style)
            val platformFallback = Typeface.create(Typeface.DEFAULT, style)
            val isKnownPlatformFamily = normalized.lowercase() in platformFamilyNames
            return ResolvedFamily(resolved, !isKnownPlatformFamily && resolved == platformFallback)
        }

        internal fun setPlatformFamilyResolverForTesting(resolver: ((String, Int) -> Typeface)?) {
            synchronized(familyLock) { platformResolverForTesting = resolver }
        }

        internal fun resetFamilyRegistryForTesting() {
            synchronized(familyLock) {
                registeredFamilies.clear()
                demonstrablyMissingFamilies.clear()
                platformResolverForTesting = null
            }
            familyObservers.resetForTesting()
        }

        fun warnOnceForMissingFamily(family: String, semanticGeneration: String, revision: String): Boolean {
            val key = "$revision\u001f$semanticGeneration\u001f$family"
            val shouldWarn = synchronized(warningLock) {
                val inserted = missingWarnings.add(key)
                while (missingWarnings.size > 512) missingWarnings.minOrNull()?.let(missingWarnings::remove)
                inserted
            }
            if (shouldWarn) Log.w("NativeEditorImage", "PreparedProseViewer: requested font family $family is unavailable; using system fallback")
            return shouldWarn
        }
    }

    private val lock = Any()
    private var fontScale = Float.NaN
    private var active = false
    private var lastFamilyRevision = 0L
    var revision: Long = 0
        private set
    var onInvalidated: ((Long) -> Unit)? = null

    /** Mount-time registration; repeated activation is harmless. */
    fun activate(deliverPending: Boolean = false) {
        val familyRevision = familyObservers.register(this)
        val shouldDeliver = synchronized(lock) {
            active = true
            if (deliverPending && familyRevision > lastFamilyRevision) {
                lastFamilyRevision = familyRevision
                true
            } else {
                lastFamilyRevision = maxOf(lastFamilyRevision, familyRevision)
                false
            }
        }
        if (shouldDeliver) invalidate()
    }

    /** Detach/recycle teardown removes the weak subscriber deterministically. */
    fun deactivate() {
        synchronized(lock) { active = false }
        familyObservers.unregister(this)
    }

    fun invalidateRegisteredFonts() = invalidate()

    fun onConfigurationChanged(configuration: Configuration) {
        val next = configuration.fontScale
        synchronized(lock) {
            if (next.isFinite() && next > 0f && next == fontScale) return
            fontScale = next
        }
        invalidate()
    }

    private fun deliverFamilyRevision(nextFamilyRevision: Long) {
        val shouldDeliver = synchronized(lock) {
            if (!active || nextFamilyRevision <= lastFamilyRevision) false
            else {
                lastFamilyRevision = nextFamilyRevision
                true
            }
        }
        if (shouldDeliver) invalidate()
    }

    private fun invalidate() {
        val next = synchronized(lock) {
            revision += 1
            revision
        }
        onInvalidated?.invoke(next)
    }

    /** Lifecycle-safe weak observer set with main-thread, revision-deduped delivery. */
    private class FamilyObservers {
        private val lock = Any()
        private val mainHandler = Handler(Looper.getMainLooper())
        private val observers = mutableListOf<WeakReference<ViewerFontEnvironment>>()
        private var revision = 0L

        fun register(environment: ViewerFontEnvironment): Long = synchronized(lock) {
            pruneLocked()
            if (observers.none { it.get() === environment }) observers += WeakReference(environment)
            revision
        }

        fun unregister(environment: ViewerFontEnvironment) = synchronized(lock) {
            observers.removeAll { it.get() == null || it.get() === environment }
        }

        fun publish() {
            val delivery = synchronized(lock) {
                revision += 1
                pruneLocked()
                revision to observers.mapNotNull { it.get() }
            }
            val run = Runnable { delivery.second.forEach { it.deliverFamilyRevision(delivery.first) } }
            if (Looper.myLooper() == Looper.getMainLooper()) run.run() else mainHandler.post(run)
        }

        fun resetForTesting() = synchronized(lock) {
            observers.clear()
            revision = 0
        }

        private fun pruneLocked() { observers.removeAll { it.get() == null } }
    }
}
