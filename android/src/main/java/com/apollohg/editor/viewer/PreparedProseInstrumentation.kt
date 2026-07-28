package com.apollohg.editor.viewer

import com.apollohg.editor.BuildConfig
import org.json.JSONArray
import org.json.JSONObject
import java.lang.Math.addExact
import java.lang.Math.subtractExact
import java.lang.Math.toIntExact

/** Debug/device-only accounting, serialized at phase transitions and writes. */
internal object PreparedProseInstrumentation {
    enum class Owner { COMPILED, UNMOUNTED_LAYOUT, FABRIC_LEASE_HANDOFF, DIRECT_MOUNTED, IMAGE, SIDECARS, OTHER }
    enum class InvalidationReason { CONTENT, WIDTH, ATTACHMENT, FONT, MEMORY_PRESSURE, CACHE_RESET, REUSE }
    enum class TraversalPhase { COLD, WARM, IMAGES_DISABLED, RESET }
    enum class ViewerWorkKind { LAYOUT, DRAW }
    data class ViewerWorkSpan(val startNanos: Long, val endNanos: Long, val kind: ViewerWorkKind)
    data class FrameClassification(val nominalFrameCount: Int, val isDelayed: Boolean)
    private data class ClippedRange(val lower: Long, val upper: Long)
    data class CacheSnapshot(
        var unmountedCurrentBytes: Long = 0,
        var unmountedHighWaterBytes: Long = 0,
        var unmountedCurrentResidentCount: Long = 0,
        var unmountedHighWaterResidentCount: Long = 0,
        var compiledCurrentBytes: Long = 0,
        var compiledCurrentResidentCount: Long = 0,
    ) {
        fun json() = JSONObject()
            .put("unmountedCurrentBytes", unmountedCurrentBytes)
            .put("unmountedHighWaterBytes", unmountedHighWaterBytes)
            .put("unmountedCurrentResidentCount", unmountedCurrentResidentCount)
            .put("unmountedHighWaterResidentCount", unmountedHighWaterResidentCount)
            .put("compiledCurrentBytes", compiledCurrentBytes)
            .put("compiledCurrentResidentCount", compiledCurrentResidentCount)
    }
    private data class DelayedInterval(
        val startNanos: Long,
        val endNanos: Long,
        val rawDeltaNanos: Long,
        val viewerLayoutNanos: Long,
        val viewerDrawNanos: Long,
        val viewerCaused: Boolean,
    ) {
        fun json() = JSONObject()
            .put("startNanos", startNanos).put("endNanos", endNanos).put("rawDeltaNanos", rawDeltaNanos)
            .put("viewerLayoutNanos", viewerLayoutNanos).put("viewerDrawNanos", viewerDrawNanos).put("viewerCaused", viewerCaused)
    }
    private class PhaseSamples {
        val compileNanos = mutableListOf<Long>(); val layoutNanos = mutableListOf<Long>(); val combinedCompileLayoutNanos = mutableListOf<Long>()
        val cacheLookupNanos = mutableListOf<Long>(); val drawNanos = mutableListOf<Long>(); val rawFrameDeltasNanos = mutableListOf<Long>()
        var compileCount = 0; var layoutCount = 0; var cacheHits = 0; var cacheMisses = 0; var cacheWaits = 0; var drawCount = 0; var visibleBlocksDrawn = 0
        var imageRequestCount = 0; var imageMetadataCount = 0; var imageDecodeCount = 0
        var nominalFrameCount = 0; var delayedIntervalCount = 0; val viewerCausedDelayedIntervals = mutableListOf<DelayedInterval>(); val invalidations = linkedMapOf<String, Int>()
        fun json() = JSONObject()
            .put("compileNanos", JSONArray(compileNanos)).put("layoutNanos", JSONArray(layoutNanos)).put("combinedCompileLayoutNanos", JSONArray(combinedCompileLayoutNanos))
            .put("cacheLookupNanos", JSONArray(cacheLookupNanos)).put("drawNanos", JSONArray(drawNanos)).put("rawFrameDeltasNanos", JSONArray(rawFrameDeltasNanos))
            .put("compileCount", compileCount).put("layoutCount", layoutCount).put("cacheHits", cacheHits).put("cacheMisses", cacheMisses).put("cacheWaits", cacheWaits).put("drawCount", drawCount).put("visibleBlocksDrawn", visibleBlocksDrawn)
            .put("imageRequestCount", imageRequestCount).put("imageMetadataCount", imageMetadataCount).put("imageDecodeCount", imageDecodeCount)
            .put("nominalFrameCount", nominalFrameCount).put("delayedIntervalCount", delayedIntervalCount).put("viewerCausedDelayedIntervals", JSONArray(viewerCausedDelayedIntervals.map(DelayedInterval::json)))
            .put("invalidations", JSONObject(invalidations as Map<*, *>))
    }

    const val NOMINAL_FRAME_PERIOD_NANOS = 16_666_667L
    const val SINGLE_TICK_TOLERANCE_NANOS = 1_000_000L
    private const val SAMPLE_LIMIT = 20_000; private val lock = Any()
    @Volatile private var enabled = false; private var phase: TraversalPhase? = null
    private val phaseSamples = linkedMapOf<TraversalPhase, PhaseSamples>(); private val completedPhases = linkedSetOf<TraversalPhase>(); private val pendingCompileNanos = linkedMapOf<TraversalPhase, MutableMap<String, Long>>()
    private val viewerWorkSpans = linkedMapOf<TraversalPhase, MutableList<ViewerWorkSpan>>(); private var cacheSnapshot = CacheSnapshot(); private var preResetSnapshot = CacheSnapshot(); private var postResetSnapshot = CacheSnapshot()
    private var duplicatePublications = 0; private var previousFrameNanos = 0L; private var surfaceDrawnSinceFrame = false

    fun classifyFrame(rawDeltaNanos: Long, nominalFramePeriodNanos: Long, singleTickToleranceNanos: Long): FrameClassification {
        require(nominalFramePeriodNanos > 0)
        if (rawDeltaNanos <= addExact(nominalFramePeriodNanos, singleTickToleranceNanos)) return FrameClassification(1, false)
        return FrameClassification(toIntExact(addExact(rawDeltaNanos, nominalFramePeriodNanos - 1) / nominalFramePeriodNanos), true)
    }

    fun viewerCaused(start: Long, end: Long, spans: List<ViewerWorkSpan>, period: Long): Boolean {
        require(end >= start); require(period > 0)
        val duration = subtractExact(end, start)
        val lateness = if (duration > period) subtractExact(duration, period) else 0L
        val union = mergedRanges(start, end, spans)
        val work = union.fold(0L) { total, range -> addExact(total, subtractExact(range.upper, range.lower)) }
        return lateness > 0 && work >= lateness
    }

    fun beginBenchmark() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { enabled = true; resetLocked() } }
    fun reset() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { resetLocked() } }
    fun beginPhase(next: TraversalPhase) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (!enabled) return@synchronized; phase = next; completedPhases.remove(next); previousFrameNanos = 0; surfaceDrawnSinceFrame = false } }
    fun endPhase() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { phase?.let { completedPhases += it; viewerWorkSpans.remove(it) }; phase = null; previousFrameNanos = 0; surfaceDrawnSinceFrame = false } }
    fun beginTraversal(next: TraversalPhase) = beginPhase(next)
    fun endTraversal() = endPhase()

    fun onDisplayFrame(frameTimeNanos: Long) {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return
        synchronized(lock) {
            val active = phase ?: return
            val previous = previousFrameNanos
            previousFrameNanos = frameTimeNanos
            if (previous == 0L || !surfaceDrawnSinceFrame) return
            val rawDelta = subtractExact(frameTimeNanos, previous)
            if (rawDelta <= 0L) return
            val spans = viewerWorkSpans[active].orEmpty()
            val sample = samples(active)
            append(sample.rawFrameDeltasNanos, rawDelta)
            val classification = classifyFrame(rawDelta, NOMINAL_FRAME_PERIOD_NANOS, SINGLE_TICK_TOLERANCE_NANOS)
            sample.nominalFrameCount += classification.nominalFrameCount
            if (classification.isDelayed) {
                sample.delayedIntervalCount++
                val caused = viewerCaused(previous, frameTimeNanos, spans, NOMINAL_FRAME_PERIOD_NANOS)
                val interval = DelayedInterval(previous, frameTimeNanos, rawDelta, clippedWork(previous, frameTimeNanos, spans, ViewerWorkKind.LAYOUT), clippedWork(previous, frameTimeNanos, spans, ViewerWorkKind.DRAW), caused)
                if (caused) sample.viewerCausedDelayedIntervals += interval
            }
            viewerWorkSpans[active] = spans.filterTo(mutableListOf()) { it.endNanos > previous }
            surfaceDrawnSinceFrame = false
        }
    }

    fun recordViewerWork(startNanos: Long, endNanos: Long, kind: ViewerWorkKind) {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION || startNanos >= endNanos) return
        synchronized(lock) { if (enabled) phase?.let { viewerWorkSpans.getOrPut(it) { mutableListOf() } += ViewerWorkSpan(startNanos, endNanos, kind) } }
    }
    fun snapshotCache(): CacheSnapshot = synchronized(lock) { cacheSnapshot.copy() }
    fun capturePreResetSnapshot() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) preResetSnapshot = cacheSnapshot.copy() } }
    fun capturePostResetSnapshot() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) postResetSnapshot = cacheSnapshot.copy() } }
    fun cacheUpdated(unmountedBytes: Long? = null, unmountedResidentCount: Long? = null, compiledBytes: Long? = null, compiledResidentCount: Long? = null) {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return
        synchronized(lock) {
            if (!enabled) return
            unmountedBytes?.let { cacheSnapshot.unmountedCurrentBytes = it.coerceAtLeast(0); cacheSnapshot.unmountedHighWaterBytes = maxOf(cacheSnapshot.unmountedHighWaterBytes, cacheSnapshot.unmountedCurrentBytes) }
            unmountedResidentCount?.let { cacheSnapshot.unmountedCurrentResidentCount = it.coerceAtLeast(0); cacheSnapshot.unmountedHighWaterResidentCount = maxOf(cacheSnapshot.unmountedHighWaterResidentCount, cacheSnapshot.unmountedCurrentResidentCount) }
            compiledBytes?.let { cacheSnapshot.compiledCurrentBytes = it.coerceAtLeast(0) }
            compiledResidentCount?.let { cacheSnapshot.compiledCurrentResidentCount = it.coerceAtLeast(0) }
        }
    }

    @JvmStatic fun exportJson(): String {
        if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION) return "{}"
        return synchronized(lock) {
            val phases = JSONObject()
            listOf(TraversalPhase.COLD, TraversalPhase.WARM, TraversalPhase.IMAGES_DISABLED).forEach { phase -> phases.put(phase.jsonName(), samples(phase).json()) }
            JSONObject().put("schemaVersion", 2).put("percentileDefinition", "nearest-rank: sorted[ceil(p*n)-1]")
                .put("nominalFramePeriodNanos", NOMINAL_FRAME_PERIOD_NANOS).put("singleTickToleranceNanos", SINGLE_TICK_TOLERANCE_NANOS)
                .put("phaseSamples", phases).put("windowEvidence", JSONArray()).put("preResetSnapshot", preResetSnapshot.json()).put("postResetSnapshot", postResetSnapshot.json()).put("duplicatePublications", duplicatePublications).toString()
        }
    }
    fun now(): Long = if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION && enabled) System.nanoTime() else 0L
    fun compiled(start: Long, generation: String) = record(start) { active, elapsed, _ -> val sample = samples(active); sample.compileCount++; append(sample.compileNanos, elapsed); pendingCompileNanos.getOrPut(active) { linkedMapOf() }[generation] = elapsed }
    fun laidOut(start: Long, generation: String) = record(start) { active, elapsed, end -> val sample = samples(active); sample.layoutCount++; append(sample.layoutNanos, elapsed); pendingCompileNanos[active]?.remove(generation)?.let { append(sample.combinedCompileLayoutNanos, addExact(it, elapsed)) }; recordViewerWorkLocked(start, end, ViewerWorkKind.LAYOUT, active) }
    fun cacheLookup(start: Long, hit: Boolean, waited: Boolean = false) = record(start) { active, elapsed, _ -> val sample = samples(active); append(sample.cacheLookupNanos, elapsed); if (hit) sample.cacheHits++ else sample.cacheMisses++; if (waited) sample.cacheWaits++ }
    fun drew(start: Long, blocks: Int) = record(start) { active, elapsed, end -> val sample = samples(active); append(sample.drawNanos, elapsed); sample.drawCount++; sample.visibleBlocksDrawn += blocks; recordViewerWorkLocked(start, end, ViewerWorkKind.DRAW, active); surfaceDrawnSinceFrame = true }
    fun imageRequested() = incrementImageCounter { it.imageRequestCount++ }
    fun imageMetadataRead() = incrementImageCounter { it.imageMetadataCount++ }
    fun imageDecoded() = incrementImageCounter { it.imageDecodeCount++ }
    fun retained(owner: Owner, scope: String, bytes: Long) = Unit
    fun invalidated(reason: InvalidationReason) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) phase?.let { active -> val values = samples(active).invalidations; values[reason.name] = values.getOrDefault(reason.name, 0) + 1 } } }
    fun duplicatePublication() { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) duplicatePublications++ } }
    private fun record(start: Long, block: (TraversalPhase, Long, Long) -> Unit) { if (!BuildConfig.PREPARED_PROSE_INSTRUMENTATION || !enabled || start == 0L) return; synchronized(lock) { phase?.let { active -> val end = System.nanoTime(); block(active, subtractExact(end, start), end) } } }
    private fun incrementImageCounter(block: (PhaseSamples) -> Unit) { if (BuildConfig.PREPARED_PROSE_INSTRUMENTATION) synchronized(lock) { if (enabled) phase?.let { block(samples(it)) } } }
    private fun recordViewerWorkLocked(start: Long, end: Long, kind: ViewerWorkKind, active: TraversalPhase) { if (start < end) viewerWorkSpans.getOrPut(active) { mutableListOf() } += ViewerWorkSpan(start, end, kind) }
    private fun mergedRanges(start: Long, end: Long, spans: List<ViewerWorkSpan>): List<ClippedRange> = spans.mapNotNull { span -> val lower = maxOf(start, span.startNanos); val upper = minOf(end, span.endNanos); if (lower < upper) ClippedRange(lower, upper) else null }.sortedBy { it.lower }.fold(mutableListOf()) { merged, range -> if (merged.isNotEmpty() && range.lower <= merged.last().upper) { val last = merged.removeAt(merged.lastIndex); merged += ClippedRange(last.lower, maxOf(last.upper, range.upper)) } else merged += range; merged }
    private fun clippedWork(start: Long, end: Long, spans: List<ViewerWorkSpan>, kind: ViewerWorkKind): Long = mergedRanges(start, end, spans.filter { it.kind == kind }).fold(0L) { total, range -> addExact(total, subtractExact(range.upper, range.lower)) }
    private fun samples(phase: TraversalPhase) = phaseSamples.getOrPut(phase) { PhaseSamples() }
    private fun append(target: MutableList<Long>, value: Long) { if (target.size < SAMPLE_LIMIT) target += value }
    private fun TraversalPhase.jsonName() = when (this) { TraversalPhase.COLD -> "cold"; TraversalPhase.WARM -> "warm"; TraversalPhase.IMAGES_DISABLED -> "imagesDisabled"; TraversalPhase.RESET -> "reset" }
    private fun resetLocked() { phase = null; phaseSamples.clear(); completedPhases.clear(); pendingCompileNanos.clear(); viewerWorkSpans.clear(); cacheSnapshot = CacheSnapshot(); preResetSnapshot = CacheSnapshot(); postResetSnapshot = CacheSnapshot(); duplicatePublications = 0; previousFrameNanos = 0; surfaceDrawnSinceFrame = false }
}
