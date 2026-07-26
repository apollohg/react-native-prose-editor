package com.apollohg.editor.viewer

import android.content.res.Configuration
import android.graphics.Typeface
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.lang.ref.WeakReference

/** Tracks font availability and system scale without mutating a frozen artifact. */
internal class ViewerFontEnvironment {
    companion object {
        private val warningLock = Any()
        private val missingWarningsBySemanticGeneration = LinkedHashMap<String, MutableSet<String>>(64, 0.75f, true)
        private const val MISSING_WARNING_GENERATION_LIMIT = 128
        private val familyLock = Any()
        private val registeredFamilies = mutableMapOf<String, Typeface>()
        private val demonstrablyMissingFamilies = mutableSetOf<String>()
        private val familyObservers = FamilyObservers()
        private val genericPlatformFamilies = setOf(
            "default", "sans", "sans-serif", "serif", "monospace", "cursive", "casual",
            "sans-serif-smallcaps", "sans-serif-condensed", "sans-serif-light", "sans-serif-medium",
            "sans-serif-black", "sans-serif-thin", "sans-serif-condensed-light",
        )
        private val platformFamilyAvailability = object : LinkedHashMap<String, Boolean>(64, 0.75f, true) {
            override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Boolean>?): Boolean = size > 128
        }
        /** Test seam models the platform family resolver, not Typeface equality. */
        private var platformResolverForTesting: ((String) -> Boolean)? = null

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
            // API 34 exposes the resolved system family name. That is a public
            // resolver, unlike the old DEFAULT equality heuristic; cache it
            // outside layout/draw hot paths. On older releases an unknown
            // family is not demonstrably absent, so retain fallback silently.
            val resolved = synchronized(familyLock) {
                Typeface.create(normalized, style)
            }
            val available = synchronized(familyLock) {
                platformResolverForTesting?.invoke(normalized)
                    ?: platformFamilyAvailability[normalized.lowercase()]
                    ?: resolvePlatformFamilyAvailability(normalized).also {
                        platformFamilyAvailability[normalized.lowercase()] = it
                    }
            }
            return ResolvedFamily(resolved, !available)
        }

        private fun resolvePlatformFamilyAvailability(family: String): Boolean {
            if (family.lowercase() in genericPlatformFamilies) return true
            if (Build.VERSION.SDK_INT < 34) return true
            val resolvedName = Typeface.create(family, Typeface.NORMAL).systemFontFamilyName
            return resolvedName.equals(family, ignoreCase = true)
        }

        internal fun setPlatformFamilyResolverForTesting(resolver: ((String) -> Boolean)?) {
            synchronized(familyLock) { platformResolverForTesting = resolver }
        }

        internal fun resetFamilyRegistryForTesting() {
            synchronized(familyLock) {
                registeredFamilies.clear()
                demonstrablyMissingFamilies.clear()
                platformResolverForTesting = null
                platformFamilyAvailability.clear()
            }
            familyObservers.resetForTesting()
        }

        fun warnOnceForMissingFamily(family: String, semanticGeneration: String): Boolean {
            val shouldWarn = synchronized(warningLock) {
                val families = missingWarningsBySemanticGeneration.getOrPut(semanticGeneration) { mutableSetOf() }
                val inserted = families.add(family)
                while (missingWarningsBySemanticGeneration.size > MISSING_WARNING_GENERATION_LIMIT) {
                    val oldest = missingWarningsBySemanticGeneration.entries.iterator().next().key
                    missingWarningsBySemanticGeneration.remove(oldest)
                }
                inserted
            }
            if (shouldWarn) Log.w("NativeEditorImage", "PreparedProseViewer: requested font family $family is unavailable; using system fallback")
            return shouldWarn
        }

        internal fun resetMissingWarningsForTesting() = synchronized(warningLock) {
            missingWarningsBySemanticGeneration.clear()
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
