package com.apollohg.editor.viewer

import android.view.Choreographer
import com.apollohg.editor.BuildConfig
import org.json.JSONObject

/**
 * Debug/device-only prepared-viewer accounting. `PREPARED_PROSE_INSTRUMENTATION`
 * is a generated build-type constant (true only in debug); R8 removes every
 * callsite and this object's volatile reads from release drawing code.
 *
 * Percentiles on both platforms use nearest rank: sorted[ceil(p * n) - 1].
 */
internal object PreparedProseInstrumentation {
    enum class Owner { COMPILED, UNMOUNTED_LAYOUT, FABRIC_LEASE_HANDOFF, DIRECT_MOUNTED, IMAGE, SIDECARS, OTHER }
    enum class InvalidationReason { CONTENT, WIDTH, ATTACHMENT, FONT, MEMORY_PRESSURE, CACHE_RESET, REUSE }
    enum class TraversalPhase { COLD, WARM, IMAGES_DISABLED, RESET }

    private const val SAMPLE_LIMIT = 20_000
    private val lock = Any()
    @Volatile private var enabled = false
    private var phase: TraversalPhase? = null
    private val compileNanos = mutableListOf<Long>(); private val layoutNanos = mutableListOf<Long>()
    private val lookupNanos = mutableListOf<Long>(); private val drawNanos = mutableListOf<Long>()
    private val coldFrameNanos = mutableListOf<Long>(); private val warmFrameNanos = mutableListOf<Long>()
    private val imagesDisabledFrameNanos = mutableListOf<Long>(); private val warmViewerFrameNanos = mutableListOf<Long>()
    private val invalidations = linkedMapOf<String, Int>(); private val retained = linkedMapOf<String, Long>()
    private var compileCount = 0; private var layoutCount = 0; private var cacheHits = 0; private var cacheMisses = 0; private var cacheWaits = 0; private var visibleBlocks = 0; private var duplicatePublications = 0
    private var previousFrameNanos = 0L
    private var surfaceDrawnSinceFrame = false

    fun beginBenchmark() {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return
        synchronized(lock) { enabled = true; resetLocked() }
    }
    fun reset() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { resetLocked() } }
    fun beginTraversal(next: TraversalPhase) {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return
        synchronized(lock) { phase = next; previousFrameNanos = 0L; surfaceDrawnSinceFrame = false }
    }
    fun endTraversal() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { phase = null; previousFrameNanos = 0L; surfaceDrawnSinceFrame = false } }
    /** Invoked by a Choreographer callback only while a benchmark traversal is active. */
    fun onDisplayFrame(frameTimeNanos: Long) {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return
        synchronized(lock) {
            val active = phase ?: return
            val previous = previousFrameNanos
            previousFrameNanos = frameTimeNanos
            if (previous == 0L || !surfaceDrawnSinceFrame) return
            val elapsed = frameTimeNanos - previous
            if (elapsed <= 0L) return
            when (active) {
                TraversalPhase.COLD -> append(coldFrameNanos, elapsed)
                TraversalPhase.WARM -> { append(warmFrameNanos, elapsed); append(warmViewerFrameNanos, elapsed) }
                TraversalPhase.IMAGES_DISABLED -> append(imagesDisabledFrameNanos, elapsed)
                TraversalPhase.RESET -> Unit
            }
            surfaceDrawnSinceFrame = false
        }
    }
    @JvmStatic fun exportJson(): String {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return "{}"
        return synchronized(lock) {
            val owners = Owner.values().associate { owner -> owner.name.lowercase() to retained.filterKeys { it.startsWith(owner.name + ":") }.values.sum() }
            JSONObject().put("percentileDefinition", "nearest-rank: sorted[ceil(p*n)-1]")
                .put("compileCount", compileCount).put("compileNanos", compileNanos).put("layoutCount", layoutCount).put("layoutNanos", layoutNanos)
                .put("cacheHits", cacheHits).put("cacheMisses", cacheMisses).put("cacheWaits", cacheWaits).put("cacheLookupNanos", lookupNanos)
                .put("drawNanos", drawNanos).put("coldFrameNanos", coldFrameNanos).put("warmFrameNanos", warmFrameNanos)
                .put("imagesDisabledFrameNanos", imagesDisabledFrameNanos).put("warmViewerFrameNanos", warmViewerFrameNanos)
                .put("visibleBlocksDrawn", visibleBlocks).put("invalidations", JSONObject(invalidations as Map<*, *>))
                .put("duplicatePublications", duplicatePublications).put("retainedBytes", JSONObject(owners)).toString()
        }
    }
    fun now(): Long = if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) System.nanoTime() else 0L
    fun compiled(start: Long) = record(start) { compileCount += 1; append(compileNanos, it) }
    fun laidOut(start: Long) = record(start) { layoutCount += 1; append(layoutNanos, it) }
    fun cacheLookup(start: Long, hit: Boolean, waited: Boolean = false) = record(start) { elapsed -> append(lookupNanos, elapsed); if (hit) cacheHits += 1 else cacheMisses += 1; if (waited) cacheWaits += 1 }
    fun drew(start: Long, blocks: Int) = record(start) { elapsed -> append(drawNanos, elapsed); visibleBlocks += blocks; surfaceDrawnSinceFrame = true }
    fun retained(owner: Owner, scope: String, bytes: Long) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) synchronized(lock) { retained[owner.name + ":" + scope] = bytes.coerceAtLeast(0L) } }
    fun invalidated(reason: InvalidationReason) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) synchronized(lock) { invalidations[reason.name] = invalidations.getOrDefault(reason.name, 0) + 1 } }
    fun duplicatePublication() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) synchronized(lock) { duplicatePublications += 1 } }
    private fun record(start: Long, block: (Long) -> Unit) { if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION || !enabled || start == 0L) return; synchronized(lock) { block(System.nanoTime() - start) } }
    private fun append(samples: MutableList<Long>, value: Long) { if (samples.size < SAMPLE_LIMIT) samples += value }
    private fun resetLocked() { phase = null; compileNanos.clear(); layoutNanos.clear(); lookupNanos.clear(); drawNanos.clear(); coldFrameNanos.clear(); warmFrameNanos.clear(); imagesDisabledFrameNanos.clear(); warmViewerFrameNanos.clear(); invalidations.clear(); retained.clear(); compileCount = 0; layoutCount = 0; cacheHits = 0; cacheMisses = 0; cacheWaits = 0; visibleBlocks = 0; duplicatePublications = 0; previousFrameNanos = 0L; surfaceDrawnSinceFrame = false }
}
