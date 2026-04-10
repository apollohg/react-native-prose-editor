package com.apollohg.editor

import android.graphics.Color
import java.util.Locale
import kotlin.math.sqrt
import org.json.JSONArray
import org.json.JSONObject
import uniffi.editor_core.*

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

    fun loadLargeDocumentIntoEditor(editorId: ULong): String {
        editorSetJson(editorId, largeDocumentJson())
        return editorGetCurrentState(editorId)
    }

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
                    clientId = index + 1,
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
