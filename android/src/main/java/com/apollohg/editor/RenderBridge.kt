package com.apollohg.editor

import android.graphics.Color
import android.text.Layout
import android.text.SpannableStringBuilder
import android.view.View
import org.json.JSONArray
import org.json.JSONObject

object RenderBridge {
    internal const val NATIVE_BLOCKQUOTE_ANNOTATION = "nativeBlockquote"
    internal const val NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION = "nativeTopLevelChildIndex"
    internal const val NATIVE_LINK_HREF_ANNOTATION = "nativeLinkHref"
    internal const val NATIVE_TASK_LIST_MARKER_ANNOTATION = "nativeTaskListMarker"
    internal const val NATIVE_INTER_BLOCK_SEPARATOR_ANNOTATION = "nativeInterBlockSeparator"
    internal const val NATIVE_LIST_MARKER_ANNOTATION = "nativeListMarker"
    internal const val NATIVE_SYNTHETIC_PLACEHOLDER_ANNOTATION = "nativeSyntheticPlaceholder"

    internal data class RenderBuildState(
        val result: SpannableStringBuilder = SpannableStringBuilder(),
        val blockStack: MutableList<BlockContext> = mutableListOf(),
        val pendingLeadingMargins: MutableMap<Int, PendingLeadingMargin> = linkedMapOf(),
        val pendingCodeBlockSpans: MutableList<PendingCodeBlockSpan> = mutableListOf(),
        val atomOccurrences: MutableMap<String, Int> = mutableMapOf(),
        val reusableImages: MutableList<BlockImageSpan> = mutableListOf(),
        var isFirstBlock: Boolean = true,
        var nextBlockSpacingBefore: Float? = null,
        var pendingListBoundarySpacing: Float? = null,
    ) {
        fun replaceNextBlockSpacing(spacing: Float?) {
            nextBlockSpacingBefore = spacing
            pendingListBoundarySpacing = null
        }

        fun addListBoundarySpacing(spacing: Float?) {
            if (spacing == null) {
                if (pendingListBoundarySpacing == null) {
                    nextBlockSpacingBefore = null
                }
                return
            }
            pendingListBoundarySpacing = (pendingListBoundarySpacing ?: 0f) + spacing
            nextBlockSpacingBefore = pendingListBoundarySpacing
        }
    }

    fun buildSpannable(
        json: String,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f,
        hostView: View? = null,
        atomConfiguration: AtomRenderConfiguration? = null
    ): SpannableStringBuilder {
        val elements = try {
            JSONArray(json)
        } catch (_: Exception) {
            return SpannableStringBuilder()
        }

        return buildSpannableFromArray(
            elements,
            baseFontSize,
            textColor,
            theme,
            density,
            hostView,
            atomConfiguration
        )
    }

    fun buildSpannableFromArray(
        elements: JSONArray,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f,
        hostView: View? = null,
        atomConfiguration: AtomRenderConfiguration? = null
    ): SpannableStringBuilder {
        val state = RenderBuildState(reusableImages = reusableImages(hostView))
        appendElements(
            state = state,
            elements = elements,
            baseFontSize = baseFontSize,
            textColor = textColor,
            theme = theme,
            density = density,
            hostView = hostView,
            atomConfiguration = atomConfiguration
        )
        applyPendingLeadingMargins(state.result, state.pendingLeadingMargins)
        applyPendingCodeBlockSpans(state.result, state.pendingCodeBlockSpans, theme, density)
        if (theme?.styleSheet != null) state.result.applyStyleSheetLineMetrics()
        return state.result
    }

    fun buildSpannableFromBlocks(
        blocks: JSONArray,
        startIndex: Int = 0,
        includeTrailingInterBlockSeparator: Boolean = false,
        baseFontSize: Float,
        textColor: Int,
        theme: EditorTheme? = null,
        density: Float = 1f,
        hostView: View? = null,
        atomConfiguration: AtomRenderConfiguration? = null,
        reuseImages: Boolean = true,
    ): SpannableStringBuilder {
        val state = RenderBuildState(reusableImages = if (reuseImages) reusableImages(hostView) else mutableListOf())
        for (blockOffset in 0 until blocks.length()) {
            val blockElements = blocks.optJSONArray(blockOffset) ?: continue
            appendElements(
                state = state,
                elements = blockElements,
                baseFontSize = baseFontSize,
                textColor = textColor,
                theme = theme,
                density = density,
                hostView = hostView,
                atomConfiguration = atomConfiguration,
                topLevelChildIndex = startIndex + blockOffset
            )
        }
        if (includeTrailingInterBlockSeparator && !state.isFirstBlock) {
            val spacingPx = ((state.nextBlockSpacingBefore ?: 0f) * density).toInt()
            appendInterBlockNewline(
                state.result,
                baseFontSize,
                textColor,
                spacingPx,
                topLevelChildIndex = startIndex + blocks.length()
            )
        }
        applyPendingLeadingMargins(state.result, state.pendingLeadingMargins)
        applyPendingCodeBlockSpans(state.result, state.pendingCodeBlockSpans, theme, density)
        if (theme?.styleSheet != null) state.result.applyStyleSheetLineMetrics()
        return state.result
    }

    private fun reusableImages(host: View?): MutableList<BlockImageSpan> =
        if (host is EditorEditText) {
            host.text.getSpans(0, host.length(), BlockImageSpan::class.java).filter { it.canReuseFor(host) }.toMutableList()
        } else mutableListOf()

    fun measureHeight(
        json: String,
        themeJson: String?,
        width: Float,
        density: Float
    ): Float {
        if (width <= 0) return 0f

        val theme = EditorTheme.fromJson(themeJson)
        val baseFontSize = theme?.text?.fontSize
            ?: theme?.paragraph?.fontSize
            ?: 16f

        val spannable = buildSpannable(
            json = json,
            baseFontSize = baseFontSize,
            textColor = android.graphics.Color.BLACK,
            theme = theme,
            density = density,
            hostView = null
        )

        if (spannable.isEmpty() && spannable.getSpans(0, 0, EditorBlockBoxSpan::class.java).isEmpty()) return 0f

        val contentInsets = theme?.contentInsets
        val topInset = ((contentInsets?.top ?: 0f) * density).toInt()
        val bottomInset = ((contentInsets?.bottom ?: 0f) * density).toInt()
        val leftInset = ((contentInsets?.left ?: 0f) * density).toInt()
        val rightInset = ((contentInsets?.right ?: 0f) * density).toInt()

        val paint = android.text.TextPaint().apply {
            textSize = baseFontSize * density
            isAntiAlias = true
        }

        val availableWidth = (width - leftInset - rightInset).coerceAtLeast(0f).toInt()

        val staticLayout = EditorDocumentLayout(spannable, paint, availableWidth, includeFontPadding = true)

        val height = staticLayout.height + topInset + bottomInset
        return height.toFloat()
    }

    fun listMarkerString(listContext: JSONObject): String {
        if (listContext.optString("kind", "") == "task") {
            return if (listContext.optBoolean("checked", false)) {
                LayoutConstants.TASK_LIST_MARKER_CHECKED
            } else {
                LayoutConstants.TASK_LIST_MARKER_UNCHECKED
            }
        }
        val ordered = listContext.optBoolean("ordered", false)
        return if (ordered) {
            val index = if (!listContext.has("index")) {
                1L
            } else {
                exactV2U32(listContext.opt("index") as? Number)?.toLong() ?: return ""
            }
            "$index. "
        } else {
            LayoutConstants.UNORDERED_LIST_BULLET
        }
    }

}

internal fun JSONObject.optPositiveFiniteFloat(key: String): Float? {
    if (!has(key) || isNull(key)) return null
    val value = optDouble(key, Double.NaN)
    if (!value.isFinite() || value <= 0.0 || value > Int.MAX_VALUE.toDouble()) return null
    return value.toFloat().takeIf { it.isFinite() && it > 0f }
}
