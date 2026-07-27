package com.apollohg.editor.viewer

import com.apollohg.editor.BuildConfig
import org.json.JSONArray
import org.json.JSONObject

/** Debug/device-only accounting, serialized at phase transitions and writes. */
internal object PreparedProseInstrumentation {
    enum class Owner { COMPILED, UNMOUNTED_LAYOUT, FABRIC_LEASE_HANDOFF, DIRECT_MOUNTED, IMAGE, SIDECARS, OTHER }
    enum class InvalidationReason { CONTENT, WIDTH, ATTACHMENT, FONT, MEMORY_PRESSURE, CACHE_RESET, REUSE }
    enum class TraversalPhase { COLD, WARM, IMAGES_DISABLED, RESET }
    private class PhaseSamples {
        val compileNanos = mutableListOf<Long>(); val layoutNanos = mutableListOf<Long>(); val combinedCompileLayoutNanos = mutableListOf<Long>()
        val cacheLookupNanos = mutableListOf<Long>(); val drawNanos = mutableListOf<Long>(); val frameNanos = mutableListOf<Long>(); val viewerFrameNanos = mutableListOf<Long>()
        var compileCount = 0; var layoutCount = 0; var cacheHits = 0; var cacheMisses = 0; var cacheWaits = 0; var drawCount = 0; var visibleBlocksDrawn = 0
        val invalidations = linkedMapOf<String, Int>()
        fun json() = JSONObject().put("compileNanos", JSONArray(compileNanos)).put("layoutNanos", JSONArray(layoutNanos)).put("combinedCompileLayoutNanos", JSONArray(combinedCompileLayoutNanos)).put("cacheLookupNanos", JSONArray(cacheLookupNanos)).put("drawNanos", JSONArray(drawNanos)).put("frameNanos", JSONArray(frameNanos)).put("viewerFrameNanos", JSONArray(viewerFrameNanos)).put("compileCount", compileCount).put("layoutCount", layoutCount).put("cacheHits", cacheHits).put("cacheMisses", cacheMisses).put("cacheWaits", cacheWaits).put("drawCount", drawCount).put("visibleBlocksDrawn", visibleBlocksDrawn).put("invalidations", JSONObject(invalidations as Map<*, *>))
    }
    private const val SAMPLE_LIMIT = 20_000; private val lock = Any()
    @Volatile private var enabled = false; private var phase: TraversalPhase? = null
    private val phaseSamples = linkedMapOf<TraversalPhase, PhaseSamples>(); private val completedPhases = linkedSetOf<TraversalPhase>(); private val pendingCompileNanos = linkedMapOf<TraversalPhase, MutableMap<String, Long>>()
    private val retained = linkedMapOf<String, Long>(); private var duplicatePublications = 0; private var previousFrameNanos = 0L; private var surfaceDrawnSinceFrame = false
    fun beginBenchmark() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { enabled = true; resetLocked() } }
    fun reset() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { resetLocked() } }
    fun beginPhase(next: TraversalPhase) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (!enabled) return@synchronized; phase = next; completedPhases.remove(next); previousFrameNanos = 0; surfaceDrawnSinceFrame = false } }
    fun endPhase() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { phase?.let(completedPhases::add); phase = null; previousFrameNanos = 0; surfaceDrawnSinceFrame = false } }
    /** Compatibility for device harnesses; bridge callers use begin/end phase. */
    fun beginTraversal(next: TraversalPhase) = beginPhase(next)
    fun endTraversal() = endPhase()
    fun onDisplayFrame(frameTimeNanos: Long) { if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return; synchronized(lock) { val active = phase ?: return; val previous = previousFrameNanos; previousFrameNanos = frameTimeNanos; if (previous == 0L || !surfaceDrawnSinceFrame) return; val elapsed = frameTimeNanos - previous; if (elapsed <= 0) return; val samples = samples(active); append(samples.frameNanos, elapsed); if (active == TraversalPhase.WARM) append(samples.viewerFrameNanos, elapsed); surfaceDrawnSinceFrame = false } }
    @JvmStatic fun exportJson(): String { if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return "{}"; return synchronized(lock) { val phases = JSONObject(); phaseSamples.filterKeys(completedPhases::contains).forEach { (phase, samples) -> phases.put(phase.name.lowercase(), samples.json()) }; val owners = Owner.values().associate { owner -> owner.name.lowercase() to retained.filterKeys { it.startsWith(owner.name + ":") }.values.sum() }; JSONObject().put("percentileDefinition", "nearest-rank: sorted[ceil(p*n)-1]").put("phaseSamples", phases).put("duplicatePublications", duplicatePublications).put("retainedBytes", JSONObject(owners)).toString() } }
    fun now(): Long = if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) System.nanoTime() else 0L
    fun compiled(start: Long, generation: String) = record(start) { active, elapsed -> val samples = samples(active); samples.compileCount++; append(samples.compileNanos, elapsed); pendingCompileNanos.getOrPut(active) { linkedMapOf() }[generation] = elapsed }
    fun laidOut(start: Long, generation: String) = record(start) { active, elapsed -> val samples = samples(active); samples.layoutCount++; append(samples.layoutNanos, elapsed); pendingCompileNanos[active]?.remove(generation)?.let { append(samples.combinedCompileLayoutNanos, it + elapsed) } }
    fun cacheLookup(start: Long, hit: Boolean, waited: Boolean = false) = record(start) { active, elapsed -> val samples = samples(active); append(samples.cacheLookupNanos, elapsed); if (hit) samples.cacheHits++ else samples.cacheMisses++; if (waited) samples.cacheWaits++ }
    fun drew(start: Long, blocks: Int) = record(start) { active, elapsed -> val samples = samples(active); append(samples.drawNanos, elapsed); samples.drawCount++; samples.visibleBlocksDrawn += blocks; surfaceDrawnSinceFrame = true }
    fun retained(owner: Owner, scope: String, bytes: Long) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) synchronized(lock) { retained[owner.name + ":" + scope] = bytes.coerceAtLeast(0L) } }
    fun invalidated(reason: InvalidationReason) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) phase?.let { active -> val values = samples(active).invalidations; values[reason.name] = values.getOrDefault(reason.name, 0) + 1 } } }
    fun duplicatePublication() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) duplicatePublications++ } }
    private fun record(start: Long, block: (TraversalPhase, Long) -> Unit) { if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION || !enabled || start == 0L) return; synchronized(lock) { phase?.let { block(it, System.nanoTime() - start) } } }
    private fun samples(phase: TraversalPhase) = phaseSamples.getOrPut(phase) { PhaseSamples() }
    private fun append(target: MutableList<Long>, value: Long) { if (target.size < SAMPLE_LIMIT) target += value }
    private fun resetLocked() { phase = null; phaseSamples.clear(); completedPhases.clear(); pendingCompileNanos.clear(); retained.clear(); duplicatePublications = 0; previousFrameNanos = 0; surfaceDrawnSinceFrame = false }
}
