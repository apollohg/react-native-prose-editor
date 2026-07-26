package com.apollohg.editor.viewer

import org.json.JSONObject

/** Benchmark-only counters. Release drawing paths return before allocating. */
internal object PreparedProseInstrumentation {
    enum class Owner { COMPILED, LAYOUT, IMAGE, SIDECARS }
    enum class InvalidationReason { CONTENT, WIDTH, ATTACHMENT, FONT, MEMORY_PRESSURE, CACHE_RESET, REUSE }

    private const val SAMPLE_LIMIT = 20_000
    private val lock = Any()
    @Volatile private var enabled = false
    private val compileNanos = mutableListOf<Long>(); private val layoutNanos = mutableListOf<Long>()
    private val lookupNanos = mutableListOf<Long>(); private val drawNanos = mutableListOf<Long>(); private val frameNanos = mutableListOf<Long>()
    private val invalidations = linkedMapOf<String, Int>(); private val retained = linkedMapOf<String, Long>()
    private var compileCount = 0; private var layoutCount = 0; private var cacheHits = 0; private var cacheMisses = 0; private var cacheWaits = 0; private var visibleBlocks = 0; private var duplicatePublications = 0

    fun beginBenchmark() = synchronized(lock) { enabled = true; resetLocked() }
    fun reset() = synchronized(lock) { resetLocked() }
    @JvmStatic fun exportJson(): String = synchronized(lock) {
        val owners = Owner.values().associate { owner -> owner.name.lowercase() to retained.filterKeys { it.startsWith(owner.name + ":") }.values.sum() }
        JSONObject().put("compileCount", compileCount).put("compileNanos", compileNanos).put("layoutCount", layoutCount).put("layoutNanos", layoutNanos).put("cacheHits", cacheHits).put("cacheMisses", cacheMisses).put("cacheWaits", cacheWaits).put("cacheLookupNanos", lookupNanos).put("drawNanos", drawNanos).put("frameNanos", frameNanos).put("visibleBlocksDrawn", visibleBlocks).put("invalidations", JSONObject(invalidations as Map<*, *>)).put("duplicatePublications", duplicatePublications).put("retainedBytes", JSONObject(owners)).toString()
    }
    fun now(): Long = if (enabled) System.nanoTime() else 0L
    fun compiled(start: Long) = record(start) { compileCount += 1; append(compileNanos, it) }
    fun laidOut(start: Long) = record(start) { layoutCount += 1; append(layoutNanos, it) }
    fun cacheLookup(start: Long, hit: Boolean, waited: Boolean = false) = record(start) { elapsed -> append(lookupNanos, elapsed); if (hit) cacheHits += 1 else cacheMisses += 1; if (waited) cacheWaits += 1 }
    fun drew(start: Long, blocks: Int) = record(start) { elapsed -> append(drawNanos, elapsed); append(frameNanos, elapsed); visibleBlocks += blocks }
    fun retained(owner: Owner, scope: String, bytes: Long) { if (!enabled) return; synchronized(lock) { retained[owner.name + ":" + scope] = bytes.coerceAtLeast(0L) } }
    fun invalidated(reason: InvalidationReason) { if (!enabled) return; synchronized(lock) { invalidations[reason.name] = invalidations.getOrDefault(reason.name, 0) + 1 } }
    fun duplicatePublication() { if (!enabled) return; synchronized(lock) { duplicatePublications += 1 } }
    private fun record(start: Long, block: (Long) -> Unit) { if (!enabled || start == 0L) return; synchronized(lock) { block(System.nanoTime() - start) } }
    private fun append(samples: MutableList<Long>, value: Long) { if (samples.size < SAMPLE_LIMIT) samples += value }
    private fun resetLocked() { compileNanos.clear(); layoutNanos.clear(); lookupNanos.clear(); drawNanos.clear(); frameNanos.clear(); invalidations.clear(); retained.clear(); compileCount = 0; layoutCount = 0; cacheHits = 0; cacheMisses = 0; cacheWaits = 0; visibleBlocks = 0; duplicatePublications = 0 }
}
