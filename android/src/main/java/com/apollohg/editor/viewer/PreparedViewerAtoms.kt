package com.apollohg.editor.viewer

import android.graphics.Rect
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.ceil

internal data class PreparedViewerAtom(val nodeType: String, val docPos: Long, val attrsJson: String, val bounds: Rect) {
    val retainedBytes: Long get() = 96L + (nodeType.length + attrsJson.length) * 2L
}

internal data class ViewerAtomConfiguration(
    val generation: String,
    val revision: String,
    val nodeTypes: Set<String>,
    val estimatedHeights: Map<String, Double>,
    val measurements: Map<String, Pair<Double, Double>>,
) {
    val retainedBytes: Long get() = 192L + (generation.length + revision.length) * 2L +
        nodeTypes.sumOf { 32L + it.length * 2L } + estimatedHeights.keys.sumOf { 48L + it.length * 2L } +
        measurements.keys.sumOf { 64L + it.length * 2L }

    fun heightPx(atom: ViewerInline.Atom, widthPx: Int, density: Float): Int {
        val measured = measurements[atom.docPos.toString()]?.takeIf { abs(it.first - widthPx / density.toDouble()) < 0.01 }
        return ceil((measured?.second ?: estimatedHeights[atom.nodeType] ?: 32.0) * density).toInt().coerceAtLeast(0)
    }

    companion object {
        fun parse(themeJson: String?): ViewerAtomConfiguration? = runCatching {
            val value = JSONObject(themeJson ?: return null).optJSONObject("viewerAtoms") ?: return null
            val types = value.optJSONArray("nodeTypes") ?: JSONArray()
            val estimates = value.optJSONObject("estimatedHeights") ?: JSONObject()
            val measurements = value.optJSONObject("measurements") ?: JSONObject()
            ViewerAtomConfiguration(
                value.getString("generation"), value.getString("revision"),
                (0 until types.length()).map { types.getString(it) }.toSet(),
                estimates.keys().asSequence().mapNotNull { key -> estimates.optDouble(key).takeIf { it.isFinite() && it >= 0 }?.let { key to it } }.toMap(),
                measurements.keys().asSequence().mapNotNull { key ->
                    val size = measurements.optJSONObject(key) ?: return@mapNotNull null
                    val width = size.optDouble("width")
                    val height = size.optDouble("height")
                    if (width.isFinite() && width >= 0 && height.isFinite() && height >= 0) key to (width to height) else null
                }.toMap(),
            )
        }.getOrNull()
    }
}

internal fun PreparedProseLayout.atomsJson(density: Float, originX: Int, originY: Int): String = JSONArray().apply {
    viewerAtoms.forEach { atom ->
        put(JSONObject().apply {
            put("nodeType", atom.nodeType)
            put("docPos", atom.docPos)
            put("attrsJson", atom.attrsJson)
            put("x", (atom.bounds.left.toDouble() + originX) / density)
            put("y", (atom.bounds.top.toDouble() + originY) / density)
            put("width", atom.bounds.width().toDouble() / density)
            put("height", atom.bounds.height().toDouble() / density)
        })
    }
}.toString()
