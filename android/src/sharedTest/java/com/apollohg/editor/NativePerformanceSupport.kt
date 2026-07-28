package com.apollohg.editor

import android.graphics.Color
import android.app.Instrumentation
import android.content.Context
import android.os.Looper
import android.os.SystemClock
import android.view.ViewGroup
import android.view.Choreographer
import androidx.recyclerview.widget.RecyclerView
import androidx.recyclerview.widget.LinearLayoutManager
import com.apollohg.editor.viewer.PreparedProseInstrumentation
import com.apollohg.editor.viewer.PreparedProseLayoutRegistry
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.sqrt
import org.json.JSONArray
import org.json.JSONObject

/** Shared literal used by RecyclerView and FlatList benchmarks. It is the
 * same complete compiler configuration that the public viewer builder receives. */
internal data class PreparedProseBenchmarkConfiguration(
    val configJson: String,
    val imagePolicyJson: String,
) {
    companion object {
        fun load(context: Context): PreparedProseBenchmarkConfiguration {
            val root = context.assets.open("prepared-prose-benchmark-config.json")
                .bufferedReader()
                .use { JSONObject(it.readText()) }
            return PreparedProseBenchmarkConfiguration(
                configJson = root.getJSONObject("configuration").toString(),
                imagePolicyJson = root.getJSONObject("imageLoadingPolicy").toString(),
            )
        }
    }
}

internal data class TimingStats(
    val name: String,
    val samplesNanos: List<Long>
) {
    val averageMillis: Double = samplesNanos.average() / 1_000_000.0

    private val relativeStdDev: Double = run {
        if (samplesNanos.size <= 1) {
            0.0
        } else {
            val average = samplesNanos.average()
            val variance = samplesNanos
                .map { sample -> (sample - average) * (sample - average) }
                .average()
            if (average == 0.0) 0.0 else sqrt(variance) / average
        }
    }

    fun summaryString(tag: String = "NativePerformanceTest"): String {
        val formattedSamples = samplesNanos.joinToString(", ") { sample ->
            String.format(Locale.US, "%.3f", sample / 1_000_000.0)
        }
        return buildString {
            append("[")
            append(tag)
            append("] ")
            append(name)
            append(" avg=")
            append(String.format(Locale.US, "%.3fms", averageMillis))
            append(" rsd=")
            append(String.format(Locale.US, "%.3f%%", relativeStdDev * 100.0))
            append(" samplesMs=[")
            append(formattedSamples)
            append("]")
        }
    }
}

/** Pixel 7 release gate, evaluated only by the Task 14 device lane. */
internal object PreparedProsePerformanceGates {
    private const val NS_PER_MS = 1_000_000L
    fun assertPasses(
        exportJson: String,
        expectedDocuments: Int,
        expectedWindows: List<PreparedProseRecyclerHarness.WarmWindow>,
    ) {
        val export = JSONObject(exportJson)
        val phases = export.getJSONObject("phaseSamples")
        val cold = phases.getJSONObject("cold")
        val warm = phases.getJSONObject("warm")
        val imagesDisabled = phases.getJSONObject("imagesDisabled")
        val nominalFramePeriodNanos = export.getLong("nominalFramePeriodNanos")
        val singleTickToleranceNanos = export.getLong("singleTickToleranceNanos")
        val combined = cold.getJSONArray("combinedCompileLayoutNanos").longs()
        val lookup = cold.getJSONArray("cacheLookupNanos").longs()
        val draw = cold.getJSONArray("drawNanos").longs()
        requireNonEmpty(combined, "cold compile+layout")
        requireNonEmpty(lookup, "cold cache lookup")
        requireNonEmpty(draw, "cold draw")
        check(combined.size >= expectedDocuments) { "expected cold compile+layout samples for every corpus document" }
        check(percentile(combined, .95) < 4 * NS_PER_MS)
        check(percentile(lookup, .99) < 100_000L)
        check(percentile(draw, .95) < NS_PER_MS)
        check(export.getInt("schemaVersion") == 2)
        check(export.getString("percentileDefinition") == "nearest-rank: sorted[ceil(p*n)-1]")
        listOf(cold, warm, imagesDisabled).forEach { phase ->
            check(phase.getInt("drawCount") > 0) { "phase must contain actual viewer draw evidence" }
            val rawFrameDeltas = phase.getJSONArray("rawFrameDeltasNanos").longs()
            requireNonEmpty(rawFrameDeltas, "${phase} raw frame")
            check(
                rawFrameDeltas.count {
                    it <= Math.addExact(nominalFramePeriodNanos, singleTickToleranceNanos)
                }.toDouble() / rawFrameDeltas.size >= .99
            )
        }
        val warmViewerCausedIntervals = warm.getJSONArray("viewerCausedDelayedIntervals").objects()
        check(warmViewerCausedIntervals.all { it.getBoolean("viewerCaused") })
        check((warmViewerCausedIntervals.maxOfOrNull { it.getLong("rawDeltaNanos") } ?: 0L) <= 33_300_000L)
        check(imagesDisabled.getInt("imageRequestCount") == 0)
        check(imagesDisabled.getInt("imageMetadataCount") == 0)
        check(imagesDisabled.getInt("imageDecodeCount") == 0)
        val preReset = export.getJSONObject("preResetSnapshot")
        val postReset = export.getJSONObject("postResetSnapshot")
        check(preReset.getLong("unmountedHighWaterBytes") <= 32L * 1024L * 1024L)
        check(postReset.getLong("unmountedCurrentBytes") == 0L)
        check(postReset.getLong("unmountedCurrentResidentCount") == 0L)
        check(postReset.getLong("compiledCurrentBytes") == 0L)
        check(postReset.getLong("compiledCurrentResidentCount") == 0L)
        val evidence = export.getJSONArray("windowEvidence")
        assertExactWindowEvidence(evidence, expectedWindows)
        evidence.objects().filter { it.getString("phase") == "warm" }.forEach { window ->
            check(window.getInt("compileCount") == 0)
            check(window.getInt("layoutCount") == 0)
            check(window.getInt("cacheMisses") == 0)
            check(window.getInt("residentKeyCount") == window.getJSONArray("entryIds").length())
            check(window.getJSONObject("cache").getLong("unmountedHighWaterBytes") <= 32L * 1024L * 1024L)
        }
        check(export.optInt("duplicatePublications") == 0)
    }

    /**
     * The benchmark must prove its literal virtualized traversal rather than
     * merely some warm cache hits. Filtering keeps the requirement independent
     * of interleaved forward/reverse recording while preserving each phase's
     * exact corpus order.
     */
    fun assertExactWindowEvidence(
        evidence: JSONArray,
        expectedWindows: List<PreparedProseRecyclerHarness.WarmWindow>,
    ) {
        check(expectedWindows.size == 27) { "expected the 27 literal warm windows" }
        val records = evidence.objects()
        assertWindowSeries(
            records.filter { it.getString("phase") == "cold" },
            expectedWindows.map { it to it.primeIds },
            "cold",
        )
        assertWindowSeries(
            records.filter { it.getString("phase") == "warm" },
            expectedWindows.map { it to it.warmIds },
            "warm",
        )
        assertWindowSeries(
            records.filter { it.getString("phase") == "imagesDisabled" },
            expectedWindows.flatMap { window -> listOf(window to window.primeIds, window to window.warmIds) },
            "imagesDisabled",
        )
    }

    private fun assertWindowSeries(
        actual: List<JSONObject>,
        expected: List<Pair<PreparedProseRecyclerHarness.WarmWindow, List<String>>>,
        phase: String,
    ) {
        check(actual.size == expected.size) {
            "expected ${expected.size} $phase window records, found ${actual.size}"
        }
        actual.zip(expected).forEachIndexed { index, (record, expectedRecord) ->
            val (window, ids) = expectedRecord
            check(record.getString("windowId") == window.id) {
                "$phase record $index must be literal window ${window.id}"
            }
            check(record.getJSONArray("entryIds").strings() == ids) {
                "$phase record $index must retain ${window.id}'s literal entry ordering"
            }
        }
    }

    private fun JSONArray.longs() = List(length()) { getLong(it) }
    private fun JSONArray.objects() = List(length()) { getJSONObject(it) }
    private fun JSONArray.strings() = List(length()) { getString(it) }
    private fun requireNonEmpty(values: List<Long>, name: String) = check(values.isNotEmpty()) { "$name evidence must be nonempty" }
    /** Nearest rank shared with iOS: sorted[ceil(p * n) - 1]. */
    private fun percentile(values: List<Long>, percentile: Double): Long = values.sorted()[(kotlin.math.ceil(values.size * percentile).toInt() - 1).coerceAtLeast(0)]
}

/** Actual RecyclerView traversal: holders host the shipped ProseViewerView. */
internal class PreparedProseRecyclerHarness(
    context: Context,
    private val configuration: PreparedProseBenchmarkConfiguration,
) : RecyclerView(context) {
    data class Entry(val id: String, val contentJson: String)
    data class WarmWindow(val id: String, val primeIds: List<String>, val warmIds: List<String>)
    data class WindowPhaseResult(
        val residentKeyCount: Int,
        val residentKeyDigest: String,
        val compileCount: Int,
        val layoutCount: Int,
        val cacheMisses: Int,
    )
    data class WindowTraversalResult(
        val windowId: String,
        val prime: WindowPhaseResult,
        val warm: WindowPhaseResult,
        val initialLeadingHolderAttached: Boolean,
    )

    private enum class Direction { PRIME, WARM }
    private data class ActiveWindow(
        val window: WarmWindow,
        val phase: PreparedProseInstrumentation.TraversalPhase,
        val imagesEnabled: Boolean,
        val completion: (Result<WindowTraversalResult>) -> Unit,
        var direction: Direction = Direction.PRIME,
        var counters: Triple<Int, Int, Int> = Triple(0, 0, 0),
        var prime: WindowPhaseResult? = null,
        var initialLeadingHolderAttached: Boolean = false,
        var finishingDirection: Boolean = false,
        var evidenceActive: Boolean = false,
        val attachmentDeadlineUptimeMs: Long = SystemClock.uptimeMillis() + INITIAL_ATTACHMENT_TIMEOUT_MS,
    )

    private val benchmarkAdapter = object : RecyclerView.Adapter<Holder>() {
        var entries: List<Entry> = emptyList()
        var imagesEnabled = true
        override fun onCreateViewHolder(parent: ViewGroup, viewType: Int) = Holder(ProseViewerView(parent.context), configuration)
        override fun onBindViewHolder(holder: Holder, position: Int) { holder.bind(entries[position], imagesEnabled) }
        override fun onViewRecycled(holder: Holder) { holder.viewer.prepareForReuse(); super.onViewRecycled(holder) }
        override fun getItemCount() = entries.size
        fun submit(nextEntries: List<Entry>, nextImagesEnabled: Boolean) {
            entries = nextEntries
            imagesEnabled = nextImagesEnabled
            notifyDataSetChanged()
        }
    }
    private val frameCallback = object : Choreographer.FrameCallback {
        override fun doFrame(frameTimeNanos: Long) {
            PreparedProseInstrumentation.onDisplayFrame(frameTimeNanos)
            if (measuring) Choreographer.getInstance().postFrameCallback(this)
        }
    }
    private var measuring = false
    private var activeWindow: ActiveWindow? = null
    private val windowEntriesById = linkedMapOf<String, Entry>()

    init {
        layoutManager = LinearLayoutManager(context)
        adapter = benchmarkAdapter
        layoutParams = ViewGroup.LayoutParams(390, 844)
        addOnScrollListener(object : OnScrollListener() {
            override fun onScrollStateChanged(recyclerView: RecyclerView, newState: Int) {
                if (newState != SCROLL_STATE_IDLE) return
                completeDirectionIfIdleAndAttached()
            }
        })
    }

    /**
     * Each literal window is submitted once, then RecyclerView owns bind,
     * measure, draw, scrolling, and recycling through a forward prime and
     * immediate reverse warm revisit. The instrumentation thread only waits
     * for the lifecycle callback; it never drives per-item layout work.
     */
    fun traverseWindows(
        instrumentation: Instrumentation,
        windows: List<WarmWindow>,
        entriesById: Map<String, Entry>,
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Boolean,
    ): List<WindowTraversalResult> {
        require(phase == PreparedProseInstrumentation.TraversalPhase.COLD || phase == PreparedProseInstrumentation.TraversalPhase.IMAGES_DISABLED)
        windowEntriesById.clear()
        windowEntriesById.putAll(entriesById)
        return windows.map { window -> traverseWindow(instrumentation, window, phase, imagesEnabled) }
    }

    /** Robolectric entrypoint for the same attached RecyclerView lifecycle. */
    fun traverseWindowsForTesting(
        windows: List<WarmWindow>,
        entriesById: Map<String, Entry>,
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Boolean,
        completion: (Result<List<WindowTraversalResult>>) -> Unit,
    ) {
        check(Looper.myLooper() == Looper.getMainLooper()) { "Robolectric traversal must start on the main thread" }
        require(phase == PreparedProseInstrumentation.TraversalPhase.COLD || phase == PreparedProseInstrumentation.TraversalPhase.IMAGES_DISABLED)
        windowEntriesById.clear()
        windowEntriesById.putAll(entriesById)
        val results = mutableListOf<WindowTraversalResult>()
        fun start(index: Int) {
            if (index == windows.size) {
                completion(Result.success(results))
                return
            }
            startWindow(windows[index], phase, imagesEnabled) { result ->
                result.fold(
                    onSuccess = { traversal -> results += traversal; start(index + 1) },
                    onFailure = { error -> completion(Result.failure(error)) },
                )
            }
        }
        start(0)
    }

    private fun traverseWindow(
        instrumentation: Instrumentation,
        window: WarmWindow,
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Boolean,
    ): WindowTraversalResult {
        val completion = CountDownLatch(1)
        var result: Result<WindowTraversalResult>? = null
        instrumentation.runOnMainSync {
            check(isAttachedToWindow) { "RecyclerView must be attached before traversal" }
            check(activeWindow == null) { "a RecyclerView window traversal is already active" }
            startWindow(window, phase, imagesEnabled) { completed ->
                result = completed
                completion.countDown()
            }
        }
        check(completion.await(10, TimeUnit.SECONDS)) { "timed out waiting for RecyclerView window ${window.id}" }
        return requireNotNull(result).getOrThrow()
    }

    private fun startWindow(
        window: WarmWindow,
        phase: PreparedProseInstrumentation.TraversalPhase,
        imagesEnabled: Boolean,
        completion: (Result<WindowTraversalResult>) -> Unit,
    ) {
        check(isAttachedToWindow) { "RecyclerView must be attached before traversal" }
        check(activeWindow == null) { "a RecyclerView window traversal is already active" }
        val entries = window.primeIds.map { id ->
            requireNotNull(windowEntriesById[id]) { "window ${window.id} references unknown entry $id" }
        }
        activeWindow = ActiveWindow(window, phase, imagesEnabled, completion)
        // Prime evidence includes RecyclerView's initial post-submit binds,
        // not only the subsequent scroll-bound work.
        beginDirection()
        benchmarkAdapter.submit(entries, imagesEnabled)
        awaitInitialLeadingAttachment()
    }

    /** Wait for RecyclerView's ordinary post-submit bind/attachment before priming. */
    private fun awaitInitialLeadingAttachment() {
        val active = activeWindow ?: return
        if (findViewHolderForAdapterPosition(0) != null) {
            active.initialLeadingHolderAttached = true
            driveCurrentDirection()
            return
        }
        if (SystemClock.uptimeMillis() >= active.attachmentDeadlineUptimeMs) {
            failActiveWindow(IllegalStateException("initial leading holder was not attached for ${active.window.id}"))
            return
        }
        post { awaitInitialLeadingAttachment() }
    }

    private fun beginDirection() {
        val active = activeWindow ?: return
        active.finishingDirection = false
        PreparedProseLayoutRegistry.shared.beginBenchmarkResidentCensus()
        val evidencePhase = if (active.direction == Direction.WARM && active.phase == PreparedProseInstrumentation.TraversalPhase.COLD) {
            PreparedProseInstrumentation.TraversalPhase.WARM
        } else {
            active.phase
        }
        PreparedProseInstrumentation.beginPhase(evidencePhase)
        active.counters = PreparedProseInstrumentation.phaseCounters()
        active.evidenceActive = true
        measuring = true
        Choreographer.getInstance().postFrameCallback(frameCallback)
    }

    private fun driveCurrentDirection() {
        val active = activeWindow ?: return
        check(active.evidenceActive) { "direction evidence must begin before scroll" }
        post {
            val current = activeWindow ?: return@post
            if (current.direction == Direction.PRIME) {
                val lastIndex = benchmarkAdapter.entries.lastIndex
                smoothScrollToPosition(lastIndex)
            } else {
                smoothScrollToPosition(0)
            }
            // A one-item window and an already-attached destination can leave
            // RecyclerView idle without dispatching a scroll-state callback.
            completeDirectionIfIdleAndAttached()
        }
    }

    /** Completion always requires an idle RecyclerView and attached destination. */
    private fun completeDirectionIfIdleAndAttached() {
        val active = activeWindow ?: return
        if (scrollState != SCROLL_STATE_IDLE || active.finishingDirection) return
        val destination = if (active.direction == Direction.PRIME) benchmarkAdapter.entries.lastIndex else 0
        if (destination < 0 || findViewHolderForAdapterPosition(destination) == null) return
        active.finishingDirection = true
        finishDirection()
    }

    private fun finishDirection() {
        val active = activeWindow ?: return
        val census = PreparedProseLayoutRegistry.shared.endBenchmarkResidentCensus()
        val counters = PreparedProseInstrumentation.phaseCounters()
        val result = WindowPhaseResult(
            residentKeyCount = census.count,
            residentKeyDigest = census.digest,
            compileCount = counters.first - active.counters.first,
            layoutCount = counters.second - active.counters.second,
            cacheMisses = counters.third - active.counters.third,
        )
        val evidencePhase = if (active.direction == Direction.WARM && active.phase == PreparedProseInstrumentation.TraversalPhase.COLD) {
            PreparedProseInstrumentation.TraversalPhase.WARM
        } else {
            active.phase
        }
        PreparedProseInstrumentation.recordWindow(
            windowId = active.window.id,
            entryIds = if (active.direction == Direction.PRIME) active.window.primeIds else active.window.warmIds,
            phase = evidencePhase,
            residentKeyCount = result.residentKeyCount,
            residentKeyDigest = result.residentKeyDigest,
            cache = PreparedProseInstrumentation.snapshotCache(),
            counters = Triple(result.compileCount, result.layoutCount, result.cacheMisses),
        )
        measuring = false
        Choreographer.getInstance().removeFrameCallback(frameCallback)
        PreparedProseInstrumentation.endPhase()
        active.evidenceActive = false
        if (active.direction == Direction.PRIME) {
            active.prime = result
            active.direction = Direction.WARM
            beginDirection()
            driveCurrentDirection()
        } else {
            activeWindow = null
            active.completion(Result.success(WindowTraversalResult(active.window.id, requireNotNull(active.prime), result, active.initialLeadingHolderAttached)))
        }
    }

    private fun failActiveWindow(error: Throwable) {
        val active = activeWindow ?: return
        activeWindow = null
        if (active.evidenceActive) {
            PreparedProseLayoutRegistry.shared.endBenchmarkResidentCensus()
            measuring = false
            Choreographer.getInstance().removeFrameCallback(frameCallback)
            PreparedProseInstrumentation.endPhase()
            active.evidenceActive = false
        }
        active.completion(Result.failure(error))
    }

    fun exportBeforeReset(): String = PreparedProseInstrumentation.exportJson()

    fun resetCacheWhileMounted() {
        check((0 until childCount).any { index ->
            (getChildViewHolder(getChildAt(index)) as? Holder)?.viewer?.preparedLayoutForTesting != null
        }) { "a prepared viewer must remain mounted before the reset" }
        PreparedProseLayoutRegistry.shared.didReceiveMemoryWarning()
        check((0 until childCount).any { index ->
            (getChildViewHolder(getChildAt(index)) as? Holder)?.viewer?.preparedLayoutForTesting != null
        }) { "the mounted prepared viewer must remain usable after the reset" }
    }

    private class Holder(
        val viewer: ProseViewerView,
        private val configuration: PreparedProseBenchmarkConfiguration,
    ) : RecyclerView.ViewHolder(viewer) {
        fun bind(entry: Entry, imagesEnabled: Boolean) {
            viewer.apply(
                ProseViewerSource.Json(entry.contentJson),
                ProseViewerConfiguration(
                    configJson = configuration.configJson,
                    imagePolicyJson = configuration.imagePolicyJson,
                    imagesEnabled = imagesEnabled,
                    collapsesWhenEmpty = true,
                ),
            )
        }
    }

    private companion object {
        const val INITIAL_ATTACHMENT_TIMEOUT_MS = 5_000L
    }
}

internal data class ApplyUpdateTraceStats(
    val name: String,
    val traces: List<EditorEditText.ApplyUpdateTrace>
) {
    private fun average(selector: (EditorEditText.ApplyUpdateTrace) -> Long): Double =
        if (traces.isEmpty()) 0.0 else traces.map(selector).average() / 1_000_000.0

    fun summaryString(tag: String = "NativePerformanceTest"): String {
        return buildString {
            append("[")
            append(tag)
            append("] ")
            append(name)
            append(" avgMs={")
            append("parse=")
            append(String.format(Locale.US, "%.3f", average { it.parseNanos }))
            append(", resolveBlocks=")
            append(String.format(Locale.US, "%.3f", average { it.resolveRenderBlocksNanos }))
            append(", patchEligibility=")
            append(String.format(Locale.US, "%.3f", average { it.patchEligibilityNanos }))
            append(", buildRender=")
            append(String.format(Locale.US, "%.3f", average { it.buildRenderNanos }))
            append(", applyRender=")
            append(String.format(Locale.US, "%.3f", average { it.applyRenderNanos }))
            append(", selection=")
            append(String.format(Locale.US, "%.3f", average { it.selectionNanos }))
            append(", postApply=")
            append(String.format(Locale.US, "%.3f", average { it.postApplyNanos }))
            append(", total=")
            append(String.format(Locale.US, "%.3f", average { it.totalNanos }))
            append("} patchUsage=")
            append(traces.count { it.usedPatch })
            append("/")
            append(traces.size)
            append(" skippedRender=")
            append(traces.count { it.skippedRender })
            append("/")
            append(traces.size)
        }
    }
}

internal object NativePerformanceFixtureFactory {
    private const val blockCount = 96
    private const val paragraphCharacterCount = 180
    private const val patchInsertIndex = 24

    fun largeRenderJson(): String = largeRenderElements().toString()

    fun largeDocumentJson(): String = JSONObject()
        .put("type", "doc")
        .put("content", largeDocumentContent())
        .toString()

    fun largeUpdateJson(): String {
        val renderBlocks = largeRenderBlocks()
        return JSONObject()
            .put("renderBlocks", renderBlocks)
            .toString()
    }

    fun largePatchedUpdateJson(): String {
        val originalBlocks = largeRenderBlocks()
        val insertedBlock = emptyParagraphRenderBlock()
        val patchedBlocks = JSONArray()
        for (index in 0 until originalBlocks.length()) {
            if (index == patchInsertIndex) {
                patchedBlocks.put(insertedBlock)
            }
            patchedBlocks.put(cloneJsonArray(originalBlocks.optJSONArray(index) ?: JSONArray()))
        }
        if (patchInsertIndex >= originalBlocks.length()) {
            patchedBlocks.put(insertedBlock)
        }

        val startIndex = if (patchInsertIndex > 0) patchInsertIndex - 1 else 0
        val oldDeleteCount = when {
            originalBlocks.length() == 0 -> 0
            patchInsertIndex == 0 -> minOf(1, originalBlocks.length())
            patchInsertIndex >= originalBlocks.length() -> 1
            else -> 2
        }
        val patchBlocks = JSONArray()
        for (index in startIndex until minOf(patchedBlocks.length(), startIndex + oldDeleteCount + 1)) {
            patchBlocks.put(cloneJsonArray(patchedBlocks.optJSONArray(index) ?: JSONArray()))
        }

        return JSONObject()
            .put(
                "renderPatch",
                JSONObject()
                    .put("startIndex", startIndex)
                    .put("deleteCount", oldDeleteCount)
                    .put("renderBlocks", patchBlocks)
            )
            .toString()
    }

    /**
     * Load the large fixture document into a v2 session through its adapter;
     * returns the resulting update JSON (null when the apply fails).
     */
    fun loadLargeDocumentIntoEditor(adapter: EditorV2Adapter): String? =
        adapter.setContentJson(largeDocumentJson())

    fun remoteSelections(
        totalScalar: Int,
        peerCount: Int = 6,
        selectionWidth: Int = 0
    ): List<RemoteSelectionDecoration> {
        val upperBound = maxOf(1, totalScalar - 1)
        val colors = listOf(
            Color.parseColor("#007AFF"),
            Color.parseColor("#34C759"),
            Color.parseColor("#FF9500"),
            Color.parseColor("#FF2D55"),
            Color.parseColor("#AF52DE"),
            Color.parseColor("#30B0C7"),
        )

        return evenlySpacedValues(1, upperBound, peerCount)
            .mapIndexed { index, scalar ->
                val head = if (selectionWidth > 0 && index % 2 == 1) {
                    minOf(upperBound, scalar + selectionWidth)
                } else {
                    scalar
                }
                RemoteSelectionDecoration(
                    clientId = (index + 1).toString(),
                    anchor = scalar,
                    head = head,
                    color = colors[index % colors.size],
                    name = "Peer ${index + 1}",
                    isFocused = true
                )
            }
    }

    fun typingCursorOffset(renderedText: CharSequence): Int =
        selectionScrubOffsets(renderedText, points = 1).firstOrNull() ?: 0

    fun selectionScrubOffsets(renderedText: CharSequence, points: Int): List<Int> {
        val candidates = renderedText
            .mapIndexedNotNull { index, char ->
                when (char) {
                    '\uFFFC', '\u200B', '\n', '\r' -> null
                    else -> index
                }
            }

        if (candidates.isEmpty()) {
            return listOf(0)
        }

        return evenlySpacedValues(0, candidates.lastIndex, points).map { candidates[it] }
    }

    private fun largeDocumentContent(): JSONArray {
        val content = JSONArray()
        content.put(
            JSONObject()
                .put("type", "h1")
                .put("content", JSONArray().put(textNode(textFragment(seed = 10_000, minCharacterCount = 40))))
        )

        for (index in 0 until blockCount) {
            if (index > 0 && index % 18 == 0) {
                content.put(JSONObject().put("type", "horizontalRule"))
            }

            when {
                index % 12 == 5 -> {
                    content.put(
                        JSONObject()
                            .put("type", "blockquote")
                            .put(
                                "content",
                                JSONArray().put(
                                    JSONObject()
                                        .put("type", "paragraph")
                                        .put(
                                            "content",
                                            richInlineDocContent(
                                                seed = index,
                                                totalCharacters = paragraphCharacterCount
                                            )
                                        )
                                )
                            )
                    )
                }

                index % 9 == 3 -> {
                    content.put(
                        JSONObject()
                            .put("type", "h2")
                            .put(
                                "content",
                                JSONArray().put(
                                    textNode(
                                        textFragment(seed = index + 2_000, minCharacterCount = 72)
                                    )
                                )
                            )
                    )
                }

                else -> {
                    content.put(
                        JSONObject()
                            .put("type", "paragraph")
                            .put(
                                "content",
                                richInlineDocContent(
                                    seed = index,
                                    totalCharacters = paragraphCharacterCount
                                )
                            )
                    )
                }
            }
        }

        return content
    }

    private fun richInlineDocContent(seed: Int, totalCharacters: Int): JSONArray {
        val text = textFragment(seed = seed, minCharacterCount = totalCharacters)
        val cutA = text.length / 4
        val cutB = text.length / 2
        val cutC = (text.length * 3) / 4

        val content = JSONArray()
        appendTextNode(content, text.substring(0, cutA))
        appendTextNode(content, text.substring(cutA, cutB), JSONArray().put("bold"))
        appendTextNode(content, text.substring(cutB, cutC), JSONArray().put("italic"))
        appendTextNode(
            content,
            text.substring(cutC),
            JSONArray().put(
                JSONObject()
                    .put("type", "link")
                    .put(
                        "attrs",
                        JSONObject()
                            .put("href", "https://example.com/item/$seed")
                            .put("target", "_blank")
                            .put("rel", "noopener noreferrer nofollow")
                            .put("class", JSONObject.NULL)
                            .put("title", JSONObject.NULL)
                    )
            )
        )
        return content
    }

    private fun appendTextNode(content: JSONArray, text: String, marks: JSONArray? = null) {
        if (text.isEmpty()) return
        val node = textNode(text)
        if (marks != null && marks.length() > 0) {
            node.put("marks", marks)
        }
        content.put(node)
    }

    private fun textNode(text: String): JSONObject =
        JSONObject()
            .put("type", "text")
            .put("text", text)

    private fun evenlySpacedValues(start: Int, endInclusive: Int, count: Int): List<Int> {
        if (count <= 1 || endInclusive <= start) {
            return listOf(start.coerceAtMost(endInclusive))
        }

        return (0 until count).map { index ->
            start + (((endInclusive - start).toLong() * index.toLong()) / (count - 1).toLong()).toInt()
        }
    }

    private fun largeRenderElements(): JSONArray = flattenRenderBlocks(largeRenderBlocks())

    private fun largeRenderBlocks(): JSONArray {
        val blocks = JSONArray()

        blocks.put(
            JSONArray().apply {
                appendBlockStart(this, nodeType = "h1", depth = 0)
                appendTextRun(this, textFragment(seed = 10_000, minCharacterCount = 40))
                appendBlockEnd(this)
            }
        )

        for (index in 0 until blockCount) {
            if (index > 0 && index % 18 == 0) {
                blocks.put(
                    JSONArray().apply {
                        appendHorizontalRule(this)
                    }
                )
            }

            blocks.put(
                JSONArray().apply {
                    when {
                        index % 12 == 5 -> {
                            appendBlockStart(this, nodeType = "blockquote", depth = 0)
                            appendBlockStart(this, nodeType = "paragraph", depth = 1)
                            appendRichInlineContent(
                                this,
                                seed = index,
                                totalCharacters = paragraphCharacterCount
                            )
                            appendBlockEnd(this)
                            appendBlockEnd(this)
                        }

                        index % 9 == 3 -> {
                            appendBlockStart(this, nodeType = "h2", depth = 0)
                            appendTextRun(
                                this,
                                textFragment(seed = index + 2_000, minCharacterCount = 72)
                            )
                            appendBlockEnd(this)
                        }

                        else -> {
                            appendBlockStart(this, nodeType = "paragraph", depth = 0)
                            appendRichInlineContent(
                                this,
                                seed = index,
                                totalCharacters = paragraphCharacterCount
                            )
                            appendBlockEnd(this)
                        }
                    }
                }
            )
        }

        return blocks
    }

    private fun flattenRenderBlocks(blocks: JSONArray): JSONArray {
        val flattened = JSONArray()
        for (blockIndex in 0 until blocks.length()) {
            val block = blocks.optJSONArray(blockIndex) ?: continue
            for (elementIndex in 0 until block.length()) {
                flattened.put(block.optJSONObject(elementIndex))
            }
        }
        return flattened
    }

    private fun emptyParagraphRenderBlock(): JSONArray =
        JSONArray().apply {
            appendBlockStart(this, nodeType = "paragraph", depth = 0)
            appendTextRun(this, "\u200B")
            appendBlockEnd(this)
        }

    private fun cloneJsonArray(array: JSONArray): JSONArray = JSONArray(array.toString())

    private fun appendRichInlineContent(
        elements: JSONArray,
        seed: Int,
        totalCharacters: Int
    ) {
        val text = textFragment(seed = seed, minCharacterCount = totalCharacters)
        val cutA = text.length / 4
        val cutB = text.length / 2
        val cutC = (text.length * 3) / 4

        appendTextRun(elements, text.substring(0, cutA))
        appendTextRun(elements, text.substring(cutA, cutB), marks = JSONArray().put("bold"))
        appendTextRun(elements, text.substring(cutB, cutC), marks = JSONArray().put("italic"))
        appendTextRun(
            elements,
            text.substring(cutC),
            marks = JSONArray().put(
                JSONObject()
                    .put("type", "link")
                    .put("href", "https://example.com/item/$seed")
            )
        )
    }

    private fun appendBlockStart(elements: JSONArray, nodeType: String, depth: Int) {
        elements.put(
            JSONObject()
                .put("type", "blockStart")
                .put("nodeType", nodeType)
                .put("depth", depth)
        )
    }

    private fun appendBlockEnd(elements: JSONArray) {
        elements.put(JSONObject().put("type", "blockEnd"))
    }

    private fun appendHorizontalRule(elements: JSONArray) {
        elements.put(
            JSONObject()
                .put("type", "voidBlock")
                .put("nodeType", "horizontalRule")
                .put("docPos", 0)
        )
    }

    private fun appendTextRun(elements: JSONArray, text: String, marks: JSONArray = JSONArray()) {
        elements.put(
            JSONObject()
                .put("type", "textRun")
                .put("text", text)
                .put("marks", marks)
        )
    }

    private fun textFragment(seed: Int, minCharacterCount: Int): String {
        val words = listOf(
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo",
            "sierra", "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
        )

        val builder = StringBuilder()
        var cursor = 0
        while (builder.length < minCharacterCount) {
            if (builder.isNotEmpty()) {
                builder.append(' ')
            }
            builder.append(words[(seed + cursor) % words.size])
            cursor += 1
        }
        return builder.substring(0, minCharacterCount)
    }
}
