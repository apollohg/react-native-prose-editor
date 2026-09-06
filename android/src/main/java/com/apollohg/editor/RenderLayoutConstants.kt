package com.apollohg.editor

import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.roundToInt

object LayoutConstants {
    /** Base indentation per depth level (pixels at base scale). */
    const val INDENT_PER_DEPTH: Float = 24f

    /** Width reserved for the list bullet/number (pixels at base scale). */
    const val LIST_MARKER_WIDTH: Float = 36f

    /** Gap between the list marker and the text that follows (pixels at base scale). */
    const val LIST_MARKER_TEXT_GAP: Float = 8f

    /** Height of the horizontal rule separator line (pixels). */
    const val HORIZONTAL_RULE_HEIGHT: Float = 1f

    /** Vertical padding above and below the horizontal rule (pixels). */
    const val HORIZONTAL_RULE_VERTICAL_PADDING: Float = 8f

    /** Total leading inset reserved for each blockquote depth. */
    const val BLOCKQUOTE_INDENT: Float = 18f

    /** Width of the rendered blockquote border bar (pixels at base scale). */
    const val BLOCKQUOTE_BORDER_WIDTH: Float = 3f

    /** Gap between the blockquote border bar and the text that follows. */
    const val BLOCKQUOTE_MARKER_GAP: Float = 8f

    /** Bullet character for unordered list items. */
    const val UNORDERED_LIST_BULLET: String = "\u2022 "

    /** Scale factor applied only to unordered list marker glyphs. */
    const val UNORDERED_LIST_MARKER_FONT_SCALE: Float = 2.0f

    /** Rendered marker text for task list items. Must stay in sync with the
     *  Rust core's task_list_marker_string (render/mod.rs) — the marker's
     *  scalar length is part of the position-mapping contract. */
    const val TASK_LIST_MARKER_CHECKED: String = "☑ "
    const val TASK_LIST_MARKER_UNCHECKED: String = "☐ "

    /** Scale factor applied to task checkbox marker glyphs. */
    const val TASK_LIST_MARKER_FONT_SCALE: Float = 1.55f

    /** Default visual treatment for link text when no explicit theme color exists. */
    const val DEFAULT_LINK_COLOR: Int = 0xFF1B73E8.toInt()

    /** Object replacement character used for void block elements. */
    const val OBJECT_REPLACEMENT_CHARACTER: String = "\uFFFC"

    /** Zero-width placeholder used to preserve trailing hard-break lines. */
    const val SYNTHETIC_PLACEHOLDER_CHARACTER: String = "\u200B"

    /** Background color for inline code spans (light gray). */
    const val CODE_BACKGROUND_COLOR: Int = 0x1A000000  // 10% black
}

data class BlockContext(
    val nodeType: String,
    val depth: Int,
    val listContext: JSONObject?,
    val topLevelChildIndex: Int? = null,
    var markerPending: Boolean = false,
    var renderStart: Int = 0,
    val language: String? = null
)

data class AtomRenderConfiguration(
    val registeredNodeTypes: Set<String>,
    val estimatedHeightsDp: Map<String, Float>,
    val measuredHeightsPx: Map<String, Int>
) {
    fun reservedHeightPx(atomKey: String, nodeType: String, density: Float): Int =
        measuredHeightsPx[atomKey]
            ?: ((estimatedHeightsDp[nodeType] ?: 0f) * density).roundToInt()

    companion object {
        fun fromJson(json: String?): AtomRenderConfiguration? {
            if (json == null) return null
            val raw = try {
                JSONObject(json)
            } catch (_: Exception) {
                return null
            }
            val nodeTypes = buildSet {
                val values = raw.optJSONArray("nodeTypes") ?: JSONArray()
                for (index in 0 until values.length()) {
                    (values.opt(index) as? String)?.let(::add)
                }
            }
            val estimatedHeights = buildMap {
                val values = raw.optJSONObject("estimatedHeights") ?: JSONObject()
                for (nodeType in values.keys()) {
                    val height = (values.opt(nodeType) as? Number)?.toFloat() ?: continue
                    if (height.isFinite() && height >= 0f) {
                        put(nodeType, height)
                    }
                }
            }
            return AtomRenderConfiguration(nodeTypes, estimatedHeights, emptyMap())
        }
    }
}

internal data class PendingLeadingMargin(
    val indentPx: Int,
    val restIndentPx: Int?,
    val blockquoteIndentPx: Int = 0,
    val blockquoteStripeColor: Int? = null,
    val blockquoteStripeWidthPx: Int = 0,
    val blockquoteGapWidthPx: Int = 0,
    val blockquoteBaseIndentPx: Int = 0
)

internal data class PendingCodeBlockSpan(
    val start: Int,
    val end: Int
)
