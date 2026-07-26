package com.apollohg.editor.viewer

import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Path
import android.graphics.PathMeasure
import android.graphics.Typeface
import android.text.Layout
import android.text.Spanned
import android.text.SpannableString
import android.text.StaticLayout
import android.text.TextPaint
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LineHeightSpan
import android.text.style.MetricAffectingSpan
import android.text.style.StrikethroughSpan
import android.text.style.UnderlineSpan
import android.text.style.ReplacementSpan
import com.apollohg.editor.EditorLinkTheme
import com.apollohg.editor.EditorMentionTheme
import com.apollohg.editor.EditorTextStyle
import com.apollohg.editor.EditorTheme
import com.apollohg.editor.ProseViewerError
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

/**
 * StaticLayout can split one visual selection run into edge-touching contours.
 * Android [Rect.right] is exclusive, so only overlapping or edge-touching
 * contours with compatible vertical bounds belong to one hit region.
 */
internal fun mergeAdjacentSameLineSelectionFragments(fragments: List<Rect>): List<Rect> {
    val ordered = fragments.sortedWith(compareBy<Rect> { it.top }.thenBy { it.left })
    val merged = mutableListOf<Rect>()
    ordered.forEach { fragment ->
        val previous = merged.lastOrNull()
        if (
            previous != null &&
            abs(previous.top - fragment.top) <= SELECTION_FRAGMENT_PIXEL_TOLERANCE_PX &&
            abs(previous.bottom - fragment.bottom) <= SELECTION_FRAGMENT_PIXEL_TOLERANCE_PX &&
            fragment.left <= previous.right
        ) {
            merged[merged.lastIndex] = Rect(
                min(previous.left, fragment.left),
                min(previous.top, fragment.top),
                max(previous.right, fragment.right),
                max(previous.bottom, fragment.bottom),
            )
        } else {
            merged += Rect(fragment)
        }
    }
    return merged
}

private const val SELECTION_FRAGMENT_PIXEL_TOLERANCE_PX = 1

/** Creates immutable StaticLayout fragments without depending on a mounted View. */
internal interface AndroidProseLayoutEngine {
    fun prepare(document: ViewerDocument, key: ProseLayoutKey, theme: PreparedProseTheme, widthPx: Int, density: Float, collapsesWhenEmpty: Boolean): PreparedProseLayout
}

/** Immutable, density-resolved inputs. No TextPaint is shared with drawing. */
internal data class PreparedTextPaint(
    val typeface: Typeface,
    val sizePx: Float,
    val color: Int,
    val lineHeightPx: Int?,
    val spacingAfterPx: Int,
) {
    fun newTextPaint(): TextPaint = TextPaint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        typeface = this@PreparedTextPaint.typeface
        textSize = sizePx
        color = this@PreparedTextPaint.color
    }
}

private fun PreparedTextPaint.withStyle(style: EditorTextStyle, density: Float): PreparedTextPaint {
    val currentBold = typeface.style == Typeface.BOLD || typeface.style == Typeface.BOLD_ITALIC
    val currentItalic = typeface.style == Typeface.ITALIC || typeface.style == Typeface.BOLD_ITALIC
    val bold = style.fontWeight?.let { it == "bold" || it.toIntOrNull()?.let { value -> value >= 600 } == true } ?: currentBold
    val italic = style.fontStyle?.let { it == "italic" } ?: currentItalic
    val resolvedStyle = when {
        bold && italic -> Typeface.BOLD_ITALIC
        bold -> Typeface.BOLD
        italic -> Typeface.ITALIC
        else -> Typeface.NORMAL
    }
    val resolvedTypeface = ViewerFontEnvironment.resolveFamily(style.fontFamily, resolvedStyle, typeface).typeface
    return copy(
        typeface = resolvedTypeface,
        sizePx = style.fontSize?.times(density)?.takeIf { it.isFinite() && it > 0f } ?: sizePx,
        color = style.color ?: color,
    )
}

internal data class PreparedProseTheme(
    val density: Float,
    val fontDensity: Float,
    val text: PreparedTextPaint,
    val paragraph: PreparedTextPaint,
    val headings: Map<String, PreparedTextPaint>,
    val blockquote: PreparedTextPaint,
    val code: PreparedTextPaint,
    val insetTopPx: Int,
    val insetRightPx: Int,
    val insetBottomPx: Int,
    val insetLeftPx: Int,
    val listIndentPx: Int,
    val listBaseIndentMultiplier: Float,
    val listItemSpacingPx: Int,
    val listMarkerColor: Int,
    val listMarkerScale: Float,
    val quoteIndentPx: Int,
    val quoteBorderColor: Int,
    val quoteBorderWidthPx: Int,
    val quoteMarkerGapPx: Int,
    val codeBackground: Int,
    val codeRadiusPx: Float,
    val codePaddingHorizontalPx: Int,
    val codePaddingVerticalPx: Int,
    val ruleColor: Int,
    val ruleThicknessPx: Int,
    val ruleMarginPx: Int,
    val link: EditorLinkTheme?,
    val mention: EditorMentionTheme?,
    val atomPaddingHorizontalPx: Int,
    val atomPaddingVerticalPx: Int,
) {
    companion object {
        fun resolve(themeJson: String?, density: Float, fontScale: Float = 1f): PreparedProseTheme {
            val theme = EditorTheme.fromJson(themeJson) ?: EditorTheme()
            val resolvedFontScale = fontScale.takeIf { it.isFinite() && it > 0f } ?: 1f
            val scaledDensity = density * resolvedFontScale
            fun px(value: Float, fallback: Float): Int = max(0, (value.takeIf { it.isFinite() } ?: fallback).times(density).toInt())
            fun fontPx(value: Float, fallback: Float): Float = ((value.takeIf { it.isFinite() } ?: fallback) * scaledDensity)
            fun typeface(style: EditorTextStyle?, fallback: Typeface): Typeface {
                return ViewerFontEnvironment.resolveFamily(
                    style?.fontFamily,
                    style?.typefaceStyle() ?: fallback.style,
                    fallback,
                ).typeface
            }
            fun paint(style: EditorTextStyle?, fallback: PreparedTextPaint? = null): PreparedTextPaint {
                val base = fallback ?: PreparedTextPaint(Typeface.DEFAULT, 17f * scaledDensity, 0xFF212121.toInt(), null, 0)
                return PreparedTextPaint(
                    typeface = typeface(style, base.typeface),
                    sizePx = style?.fontSize?.let { fontPx(it, 17f) } ?: base.sizePx,
                    color = style?.color ?: base.color,
                    lineHeightPx = style?.lineHeight?.let { fontPx(it, 0f).toInt().takeIf { value -> value > 0 } } ?: base.lineHeightPx,
                    spacingAfterPx = style?.spacingAfter?.let { fontPx(it, 0f).toInt() } ?: base.spacingAfterPx,
                )
            }
            val text = paint(theme.text)
            val paragraph = paint(theme.effectiveTextStyle("paragraph"), text)
            val quote = paint(theme.effectiveTextStyle("paragraph", true), paragraph)
            val codeFallback = PreparedTextPaint(Typeface.MONOSPACE, text.sizePx, text.color, text.lineHeightPx, text.spacingAfterPx)
            val headings = listOf("h1" to 32f, "h2" to 28f, "h3" to 24f, "h4" to 21f, "h5" to 19f, "h6" to 17f).associate { (name, size) ->
                name to paint(EditorTextStyle(fontSize = size, fontWeight = "700", spacingAfter = 10f).mergedWith(theme.headings[name]), paragraph)
            }
            return PreparedProseTheme(
                density, scaledDensity, text, paragraph, headings, quote, paint(theme.effectiveTextStyle("codeBlock"), codeFallback),
                px(theme.contentInsets?.top ?: 0f, 0f), px(theme.contentInsets?.right ?: 0f, 0f), px(theme.contentInsets?.bottom ?: 0f, 0f), px(theme.contentInsets?.left ?: 0f, 0f),
                px(theme.list?.indent ?: 28f, 28f), theme.list?.baseIndentMultiplier ?: 1f, px(theme.list?.itemSpacing ?: 4f, 4f), theme.list?.markerColor ?: text.color, theme.list?.markerScale ?: 1f,
                px(theme.blockquote?.indent ?: 16f, 16f), theme.blockquote?.borderColor ?: 0xFFC7C7CC.toInt(), px(theme.blockquote?.borderWidth ?: 3f, 3f), px(theme.blockquote?.markerGap ?: 10f, 10f),
                theme.codeBlock?.backgroundColor ?: 0xFFF2F2F7.toInt(), (theme.codeBlock?.borderRadius ?: 8f) * density, px(theme.codeBlock?.paddingHorizontal ?: 12f, 12f), px(theme.codeBlock?.paddingVertical ?: 8f, 8f),
                theme.horizontalRule?.color ?: 0xFFC7C7CC.toInt(), max(1, px(theme.horizontalRule?.thickness ?: 1f, 1f)), px(theme.horizontalRule?.verticalMargin ?: 12f, 12f),
                theme.links, theme.mentions, px(6f, 6f), px(4f, 4f),
            )
        }
    }

    fun paintFor(block: ViewerBlock): PreparedTextPaint = when {
        block.nodeType == "codeBlock" -> code
        headings.containsKey(block.nodeType) -> headings.getValue(block.nodeType)
        block.inBlockquote -> blockquote
        else -> paragraph
    }

    val retainedBytes: Long get() = 3_072L + headings.size * 384L
}

private data class PreparedAtomAppearance(val paint: PreparedTextPaint, val background: Int, val borderColor: Int?, val borderWidth: Float, val radius: Float, val paddingHorizontal: Int, val paddingVertical: Int)
private data class PreparedAtomSpec(val start: Int, val nodeType: String, val label: String, val appearance: PreparedAtomAppearance, val widthPx: Int, val heightPx: Int, val labelLayout: StaticLayout, val labelBaselinePx: Int)
private data class PreparedMarker(val layout: StaticLayout?, val label: String, val widthPx: Int, val heightPx: Int, val ascentPx: Int, val descentPx: Int, val baselinePx: Int, val checked: Boolean)

private class AtomMetricSpan(
    private val widthPx: Int,
    private val ascentPx: Int,
    private val descentPx: Int,
) : ReplacementSpan() {
    override fun getSize(paint: Paint, text: CharSequence?, start: Int, end: Int, fm: Paint.FontMetricsInt?): Int {
        fm?.let {
            it.ascent = -ascentPx
            it.top = it.ascent
            it.descent = descentPx
            it.bottom = descentPx
        }
        return widthPx
    }
    override fun draw(canvas: android.graphics.Canvas, text: CharSequence?, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) = Unit
}

/**
 * Viewer-local fixed line metrics, deliberately independent of the legacy
 * editable-render bridge. The metric follows Core Text parity: never shrink
 * the shaped run/atom below its natural height, and split added leading around
 * the baseline with the odd pixel assigned below it.
 */
internal class FixedLineHeightMetricSpan(
    private val lineHeightPx: Int,
) : LineHeightSpan {
    override fun chooseHeight(
        text: CharSequence,
        start: Int,
        end: Int,
        spanstartv: Int,
        v: Int,
        fm: Paint.FontMetricsInt,
    ) {
        val naturalHeight = fm.descent - fm.ascent
        val targetHeight = max(naturalHeight, lineHeightPx)
        val extraLeading = targetHeight - naturalHeight
        if (naturalHeight <= 0 || extraLeading <= 0) return

        val leadingAboveBaseline = extraLeading / 2
        fm.ascent -= leadingAboveBaseline
        fm.top = fm.ascent
        fm.descent += extraLeading - leadingAboveBaseline
        fm.bottom = fm.descent
    }
}

/** Immutable before StaticLayout construction, including full link typography. */
internal class ResolvedTextStyleSpan(
    internal val typeface: Typeface,
    internal val sizePx: Float,
) : MetricAffectingSpan() {
    override fun updateMeasureState(textPaint: TextPaint) {
        textPaint.typeface = typeface
        textPaint.textSize = sizePx
    }

    override fun updateDrawState(textPaint: TextPaint) = updateMeasureState(textPaint)
}

internal class StaticLayoutAndroidProseLayoutEngine : AndroidProseLayoutEngine {
    /** Test seam: drawing must never increment this prepared-layout counter. */
    internal var staticLayoutsBuilt: Int = 0
        private set
    private var semanticGeneration = ""
    private var fontRevision = "0\u001f0"

    override fun prepare(document: ViewerDocument, key: ProseLayoutKey, theme: PreparedProseTheme, widthPx: Int, density: Float, collapsesWhenEmpty: Boolean): PreparedProseLayout {
        semanticGeneration = key.generationIdentity
        fontRevision = "${key.nativeFontRevision}\u001f${key.fontEnvironmentRevision}"
        if (widthPx <= 0 || !density.isFinite() || density <= 0f) return PreparedProseLayout.error(key, 0, ProseViewerError.invalidWidth())
        if (document.isEmpty && collapsesWhenEmpty) return PreparedProseLayout(key, widthPx, 0, emptyList(), retainedBytes = document.retainedBytes)
        val contentWidth = max(1, widthPx - theme.insetLeftPx - theme.insetRightPx)
        var cursorY = theme.insetTopPx
        var retained = document.retainedBytes + theme.retainedBytes
        val interactions = mutableListOf<PreparedProseInteraction>()
        val imageAttachments = mutableListOf<ViewerImageAttachment>()
        val markers = mutableMapOf<Int, PreparedMarker>()
        document.blocks.forEach { block ->
            listItemAncestors(block).forEach { ancestor ->
                if (markers[ancestor.identity] == null) {
                    markers[ancestor.identity] = markerFor(ancestor.context, theme.paintFor(block), theme)
                }
            }
        }
        val blocks = document.blocks.map { block ->
            val prepared = prepareBlock(block, imageAttachments.size, markers, theme, contentWidth, cursorY)
            cursorY = prepared.nextY
            retained += prepared.block.retainedBytes + prepared.extraBytes
            interactions += prepared.interactions
            prepared.attachment?.let(imageAttachments::add)
            prepared.block
        }
        val height = max(0, cursorY + theme.insetBottomPx)
        interactions.sortWith(compareBy<PreparedProseInteraction> { it.rects.firstOrNull()?.top ?: Int.MAX_VALUE }.thenBy { it.rects.firstOrNull()?.left ?: Int.MAX_VALUE })
        val nodes = interactions.mapIndexed { index, interaction ->
            PreparedProseAccessibilityNode(
                index,
                if (interaction.kind == PreparedProseInteraction.Kind.LINK) PreparedProseAccessibilityNode.Role.LINK else PreparedProseAccessibilityNode.Role.MENTION,
                if (interaction.kind == PreparedProseInteraction.Kind.LINK) interaction.visibleText else interaction.label,
                interaction.rects.fold(Rect()) { bounds, rect -> if (bounds.isEmpty) Rect(rect) else Rect(bounds).apply { union(rect) } },
            )
        }
        retained += interactions.sumOf { it.retainedBytes } + nodes.sumOf { it.retainedBytes }
        // Mounted image-publication sidecars are runtime surface ownership,
        // not immutable artifact/cache ownership; account them at the host.
        return PreparedProseLayout(key, widthPx, height, blocks, interactions, nodes, imageAttachments, retained)
    }

    private data class BlockResult(val block: PreparedProseBlock, val interactions: List<PreparedProseInteraction>, val attachment: ViewerImageAttachment? = null, val nextY: Int, val extraBytes: Long)

    private fun prepareBlock(block: ViewerBlock, attachmentOrdinal: Int, measuredMarkers: Map<Int, PreparedMarker>, theme: PreparedProseTheme, contentWidth: Int, cursorY: Int): BlockResult {
        val paint = theme.paintFor(block)
        val ancestors = listItemAncestors(block)
        val ancestorMarkers = ancestors.mapNotNull { ancestor -> measuredMarkers[ancestor.identity]?.let { ancestor to it } }
        val firstMarkers = ancestorMarkers.filter { (ancestor, _) -> ancestor.isFirstRenderableLeaf }
        val markerTopInset = firstMarkers.maxOfOrNull { (_, marker) -> max(0, marker.baselinePx - paint.newTextPaint().fontMetricsInt.run { -ascent }) } ?: 0
        val baseListInset = if (ancestors.isEmpty()) 0 else max(0, (theme.listIndentPx * theme.listBaseIndentMultiplier).toInt())
        val ancestorGutters = ancestorMarkers.associate { (ancestor, marker) -> ancestor.identity to max(6, marker.widthPx + 6) }
        // A nested leaf owns every outer list column too: each ancestor adds
        // its list indent and independently measured marker gutter.
        val listInset = baseListInset + ancestorMarkers.sumOf { (ancestor, _) -> theme.listIndentPx + (ancestorGutters[ancestor.identity] ?: 0) }
        val quoteInset = if (block.inBlockquote) theme.quoteBorderWidthPx + theme.quoteMarkerGapPx + theme.quoteIndentPx else 0
        val codeInset = if (block.nodeType == "codeBlock") theme.codePaddingHorizontalPx else 0
        val textX = theme.insetLeftPx + listInset + quoteInset + codeInset
        val itemSpacing = if (ancestors.isEmpty()) paint.spacingAfterPx else ancestors.count { it.isFinalRenderableLeaf } * theme.listItemSpacingPx
        if (block.nodeType == "image") {
            val source = ViewerImageAttachment.sourceAndDeclaredSize(block)
            if (source != null) {
                val imageWidth = max(1, contentWidth - listInset - quoteInset)
                val resolved = source.third ?: ViewerImageIntrinsicStore.shared.size(source.first)
                val imageHeight = resolved?.let { imageWidth * it.second / max(1, it.first) } ?: max(44, minOf(240, (imageWidth * .56f).toInt()))
                val bounds = Rect(textX, cursorY, textX + imageWidth, cursorY + imageHeight)
                val attachment = ViewerImageAttachment(source.first, source.second, bounds, source.third, attachmentOrdinal)
                return BlockResult(PreparedProseBlock(listOf(PreparedProseFragment(PreparedProseFragmentKind.IMAGE, bounds, color = 0xFFF2F2F7.toInt())), bounds), emptyList(), attachment, bounds.bottom + itemSpacing, 192)
            }
        }
        fun markerAnchor(ancestor: ViewerListItemAncestor): Int {
            var inset = baseListInset
            ancestorMarkers.forEach { (candidate, _) ->
                inset += theme.listIndentPx + (ancestorGutters[candidate.identity] ?: 0)
                if (candidate.identity == ancestor.identity) return theme.insetLeftPx + quoteInset + inset
            }
            return textX - codeInset
        }
        if (block.nodeType == "horizontalRule" || block.nodeType == "horizontal_rule") {
            val ruleTop = cursorY + markerTopInset + theme.ruleMarginPx
            val ruleLeft = theme.insetLeftPx + listInset + quoteInset
            val ruleRight = max(ruleLeft + 1, theme.insetLeftPx + contentWidth - listInset - quoteInset)
            val rule = Rect(ruleLeft, ruleTop, ruleRight, ruleTop + theme.ruleThicknessPx)
            val fragments = mutableListOf(PreparedProseFragment(PreparedProseFragmentKind.RULE, rule, color = theme.ruleColor, strokeWidth = theme.ruleThicknessPx.toFloat()))
            val end = rule.bottom + theme.ruleMarginPx
            if (block.inBlockquote) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, end), color = theme.quoteBorderColor)
            firstMarkers.forEach { (ancestor, marker) ->
                fragments += markerFragment(marker, markerAnchor(ancestor), max(cursorY + marker.baselinePx, ruleTop + theme.ruleThicknessPx), ancestorGutters.getValue(ancestor.identity), theme.listMarkerColor)
            }
            return finishBlock(fragments, emptyList(), Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, end), end, itemSpacing)
        }

        val availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
        val attributed = attributed(block.inlines, paint, theme)
        val layout = staticLayout(attributed.text, paint, availableWidth)
        val firstBaseline = layout.getLineBaseline(0)
        // A marker can be taller than the first text line. Reserve its excess
        // ascent before publishing geometry so no marker has a negative top.
        val markerTextInset = firstMarkers.maxOfOrNull { (_, marker) -> max(0, marker.baselinePx - firstBaseline) } ?: 0
        val textTop = cursorY + markerTextInset + if (block.nodeType == "codeBlock") theme.codePaddingVerticalPx else 0
        val textHeight = max(1, layout.height)
        val totalEnd = textTop + textHeight + if (block.nodeType == "codeBlock") theme.codePaddingVerticalPx else 0
        val fragments = mutableListOf<PreparedProseFragment>()
        val interactionRects = MutableList(attributed.semanticRanges.size) { mutableListOf<Rect>() }
        if (block.nodeType == "codeBlock") fragments += PreparedProseFragment(PreparedProseFragmentKind.BACKGROUND, Rect(theme.insetLeftPx + listInset + quoteInset, cursorY, theme.insetLeftPx + contentWidth - listInset - quoteInset, totalEnd), color = theme.codeBackground, cornerRadius = theme.codeRadiusPx)
        fragments += PreparedProseFragment(PreparedProseFragmentKind.TEXT, Rect(textX, textTop, textX + availableWidth, textTop + textHeight), layout, textX, textTop)
        attributed.semanticRanges.forEachIndexed { index, semantic ->
            val firstLine = layout.getLineForOffset(semantic.start)
            val lastLine = layout.getLineForOffset((semantic.end - 1).coerceAtLeast(semantic.start))
            for (line in firstLine..lastLine) {
                val lineStart = layout.getLineStart(line)
                val lineEnd = layout.getLineEnd(line)
                val start = maxOf(semantic.start, lineStart)
                val end = minOf(semantic.end, lineEnd)
                if (start >= end) continue
                // StaticLayout's selection path follows its shaped visual runs.
                // Clipping it to this line preserves discontiguous bidi pieces
                // rather than collapsing a logical range to endpoint geometry.
                mergeAdjacentSameLineSelectionFragments(
                    selectionRectsForLine(layout, start, end, line, availableWidth)
                ).forEach { rect ->
                    rect.offset(textX, textTop)
                    // This semantic target's touching contours share one visual
                    // run. Gapped bidi contours remain separate hit regions.
                    interactionRects[index] += rect
                }
            }
        }
        attributed.atoms.forEach { atom ->
            val line = layout.getLineForOffset(atom.start)
            // Replacement spans consume a visual slot that can run right-to-left.
            // The two endpoints, rather than a logical-start-plus-width guess,
            // are the only hit/draw geometry published for the atom.
            val visualStart = layout.getPrimaryHorizontal(atom.start)
            val visualEnd = layout.getPrimaryHorizontal(atom.start + 1)
            val atomLeft = textX + min(visualStart, visualEnd).toInt()
            val atomRight = textX + max(visualStart, visualEnd).toInt()
            val baseline = textTop + layout.getLineBaseline(line)
            val atomTop = baseline - atom.appearance.paddingVertical - atom.labelBaselinePx
            val bounds = Rect(atomLeft, atomTop, max(atomLeft + 1, atomRight), atomTop + atom.heightPx)
            fragments += PreparedProseFragment(PreparedProseFragmentKind.ATOM, bounds, labelLayout = atom.labelLayout, labelX = bounds.left + atom.appearance.paddingHorizontal, labelY = bounds.top + atom.appearance.paddingVertical, color = atom.appearance.background, borderColor = atom.appearance.borderColor, cornerRadius = atom.appearance.radius, strokeWidth = atom.appearance.borderWidth, label = atom.label)
        }
        if (block.inBlockquote) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, totalEnd), color = theme.quoteBorderColor)
        firstMarkers.forEach { (ancestor, marker) ->
            fragments += markerFragment(marker, markerAnchor(ancestor), textTop + firstBaseline, ancestorGutters.getValue(ancestor.identity), theme.listMarkerColor)
        }
        val interactions = attributed.semanticRanges.zip(interactionRects).mapNotNull { (semantic, rects) ->
            if (rects.isEmpty()) null else when (semantic) {
                is PreparedSemanticRange.Link -> PreparedProseInteraction(PreparedProseInteraction.Kind.LINK, rects, semantic.href, semantic.text, null, semantic.text)
                is PreparedSemanticRange.Mention -> PreparedProseInteraction(PreparedProseInteraction.Kind.MENTION, rects, null, semantic.label, semantic.docPos, semantic.label)
            }
        }
        return finishBlock(fragments, interactions, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, totalEnd), totalEnd, itemSpacing, attributed.retainedBytes)
    }

    private fun listItemAncestors(block: ViewerBlock): List<ViewerListItemAncestor> =
        block.listItemAncestors.ifEmpty {
            val boundary = block.listItemBoundary
            val context = block.listContext
            if (boundary == null || context == null) emptyList() else listOf(
                ViewerListItemAncestor(
                    boundary.identity,
                    context,
                    boundary.nestingDepth,
                    boundary.isFirstRenderableLeaf,
                    boundary.isFinalRenderableLeaf,
                )
            )
        }

    private fun finishBlock(fragments: List<PreparedProseFragment>, interactions: List<PreparedProseInteraction>, seed: Rect, end: Int, spacing: Int, extraBytes: Long = 0): BlockResult {
        val bounds = fragments.fold(seed) { acc, fragment -> Rect(acc).apply { union(fragment.bounds) } }
        return BlockResult(PreparedProseBlock(fragments.toList(), bounds), interactions, null, max(end, bounds.bottom) + spacing, extraBytes)
    }

    private fun selectionRectsForLine(
        layout: StaticLayout,
        start: Int,
        end: Int,
        line: Int,
        width: Int,
    ): List<Rect> {
        val path = Path()
        layout.getSelectionPath(start, end, path)
        val lineClip = Rect(0, layout.getLineTop(line), width, layout.getLineBottom(line))
        val measure = PathMeasure(path, false)
        val result = mutableListOf<Rect>()
        do {
            if (measure.length == 0f) continue
            val contour = Path()
            measure.getSegment(0f, measure.length, contour, true)
            val bounds = RectF()
            contour.computeBounds(bounds, true)
            val piece = Rect(
                bounds.left.toInt(),
                bounds.top.toInt(),
                ceil(bounds.right).toInt(),
                ceil(bounds.bottom).toInt(),
            )
            if (piece.intersect(lineClip) && !piece.isEmpty) result += piece
        } while (measure.nextContour())
        return result.sortedWith(compareBy<Rect> { it.top }.thenBy { it.left })
    }

    private data class AttributedBlock(val text: SpannableString, val atoms: List<PreparedAtomSpec>, val semanticRanges: List<PreparedSemanticRange>, val retainedBytes: Long)
    private sealed interface PreparedSemanticRange { val start: Int; val end: Int
        data class Link(override val start: Int, override val end: Int, val href: String, val text: String) : PreparedSemanticRange
        data class Mention(override val start: Int, override val end: Int, val docPos: Long, val label: String) : PreparedSemanticRange
    }

    private fun attributed(inlines: List<ViewerInline>, base: PreparedTextPaint, theme: PreparedProseTheme): AttributedBlock {
        val source = StringBuilder()
        val spans = mutableListOf<(SpannableString) -> Unit>()
        val atoms = mutableListOf<PreparedAtomSpec>()
        val semanticRanges = mutableListOf<PreparedSemanticRange>()
        inlines.forEach { inline -> when (inline) {
            is ViewerInline.Text -> {
                val start = source.length
                source.append(inline.text)
                val end = source.length
                val markSpans = markSpans(inline.marks, base, theme)
                spans += { value -> markSpans.forEach { value.setSpan(it, start, end, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE) } }
                href(inline.marks)?.let { href ->
                    val previous = semanticRanges.lastOrNull() as? PreparedSemanticRange.Link
                    if (previous != null && previous.href == href && previous.end == start) {
                        semanticRanges[semanticRanges.lastIndex] = previous.copy(end = end, text = previous.text + inline.text)
                    } else semanticRanges += PreparedSemanticRange.Link(start, end, href, inline.text)
                }
            }
            is ViewerInline.Atom -> {
                if (inline.nodeType == "hardBreak" || inline.nodeType == "hard_break") source.append('\n') else {
                    val appearance = atomAppearance(inline.nodeType, inline.attrsJson, base, theme)
                    val label = inline.label.ifEmpty { " " }
                    val labelPaint = appearance.paint.newTextPaint()
                    val width = max(base.sizePx.toInt(), ceil(labelPaint.measureText(label) + appearance.paddingHorizontal * 2).toInt())
                    val labelLayout = staticLayout(SpannableString(label), appearance.paint, max(1, width - appearance.paddingHorizontal * 2))
                    val labelMetrics = labelPaint.fontMetricsInt
                    val labelAscent = max(0, -labelMetrics.ascent)
                    val labelDescent = max(0, labelMetrics.descent)
                    val ascent = labelAscent + appearance.paddingVertical
                    // Keep the label's descenders and any resolved line-height
                    // expansion in the outer replacement metrics.
                    val descent = labelDescent + appearance.paddingVertical
                    val metricDescent = descent + max(0, labelLayout.height + appearance.paddingVertical * 2 - ascent - descent)
                    val height = ascent + metricDescent
                    val start = source.length
                    source.append('\uFFFC')
                    atoms += PreparedAtomSpec(start, inline.nodeType, label, appearance, width, height, labelLayout, labelLayout.getLineBaseline(0))
                    spans += { value -> value.setSpan(AtomMetricSpan(width, ascent, metricDescent), start, start + 1, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE) }
                    if (inline.nodeType == "mention") semanticRanges += PreparedSemanticRange.Mention(start, start + 1, inline.docPos, label)
                }
            }
        } }
        val text = SpannableString(if (source.isEmpty()) "\u200B" else source.toString())
        spans.forEach { it(text) }
        return AttributedBlock(text, atoms, semanticRanges, 256L + text.length * 52L + atoms.sumOf { 256L + it.label.length * 2L })
    }

    private fun href(marks: List<uniffi.editor_core.FfiViewerMark>): String? = marks.firstOrNull { it.markType == "link" }
        ?.let { runCatching { org.json.JSONObject(it.attrsJson).optString("href") }.getOrNull()?.takeIf(String::isNotEmpty) }

    private fun markSpans(marks: List<uniffi.editor_core.FfiViewerMark>, base: PreparedTextPaint, theme: PreparedProseTheme): List<Any> {
        var explicitColor: Int? = null
        var background: Int? = null
        var underline = false
        var bold = false
        var italic = false
        var monospace = false
        var strike = false
        var size: Float? = null
        var family: String? = null
        var hasLink = false
        var link: EditorLinkTheme? = null
        marks.forEach { mark ->
            val attrs = runCatching { org.json.JSONObject(mark.attrsJson) }.getOrNull()
            when (mark.markType) {
                "bold", "strong" -> bold = true
                "italic", "em" -> italic = true
                "underline" -> underline = true
                "strike", "strikethrough" -> strike = true
                "code" -> monospace = true
                "link" -> {
                    hasLink = true
                    link = theme.link
                    underline = underline || (theme.link?.underline ?: true)
                    background = theme.link?.backgroundColor ?: background
                }
                "textColor", "color", "foregroundColor" -> explicitColor = parseColor(attrs?.optString("color", null) ?: attrs?.optString("textColor", null)) ?: explicitColor
                "highlight", "backgroundColor" -> background = parseColor(attrs?.optString("color", null) ?: attrs?.optString("backgroundColor", null)) ?: background
                "textStyle", "font" -> { family = attrs?.optString("fontFamily", null) ?: family; size = attrs?.optDouble("fontSize", Double.NaN)?.takeIf { it.isFinite() && it > 0 }?.toFloat() ?: size }
            }
        }
        // Resolve link family/size/weight/style first, then combine explicit
        // mark traits into one immutable metric span before StaticLayout sees it.
        var resolved = link?.let { base.withStyle(it.asTextStyle(), theme.fontDensity) } ?: base
        if (family != null || size != null) {
            family?.let { requested ->
                if (ViewerFontEnvironment.resolveFamily(requested, Typeface.NORMAL, base.typeface).isDemonstrablyMissing) {
                    ViewerFontEnvironment.warnOnceForMissingFamily(requested, semanticGeneration, fontRevision)
                }
            }
            resolved = resolved.withStyle(EditorTextStyle(fontFamily = family, fontSize = size), theme.fontDensity)
        }
        if (monospace) resolved = resolved.withStyle(EditorTextStyle(fontFamily = "monospace"), theme.fontDensity)
        if (bold || italic) {
            resolved = resolved.withStyle(
                EditorTextStyle(
                    fontWeight = if (bold) "bold" else null,
                    fontStyle = if (italic) "italic" else null,
                ),
                theme.fontDensity,
            )
        }
        val result = mutableListOf<Any>(ResolvedTextStyleSpan(resolved.typeface, resolved.sizePx))
        result += ForegroundColorSpan(explicitColor ?: if (hasLink) link?.color ?: 0xFF007AFF.toInt() else base.color)
        background?.let { result += BackgroundColorSpan(it) }
        if (underline) result += UnderlineSpan()
        // Android shapes this with the run's resolved foreground and visual bidi
        // order, unlike manually derived logical-horizontal strike rectangles.
        if (strike) result += StrikethroughSpan()
        return result
    }

    private fun staticLayout(text: CharSequence, paint: PreparedTextPaint, width: Int): StaticLayout {
        staticLayoutsBuilt += 1
        val resolved = paint.newTextPaint()
        val preparedText = if (paint.lineHeightPx == null) text else SpannableString(text).apply {
            // The full prepared range includes a single line and the final line
            // after a hard break; builder line spacing does not provide that
            // guarantee and would double-compensate these metrics.
            setSpan(FixedLineHeightMetricSpan(paint.lineHeightPx), 0, length, Spanned.SPAN_INCLUSIVE_INCLUSIVE)
        }
        return StaticLayout.Builder.obtain(preparedText, 0, preparedText.length, resolved, width)
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(false)
            .setBreakStrategy(Layout.BREAK_STRATEGY_HIGH_QUALITY)
            .build()
    }

    private fun markerFor(context: ViewerListContext, textPaint: PreparedTextPaint, theme: PreparedProseTheme): PreparedMarker {
        val label = when { context.kind == "task" -> ""; context.ordered -> "${context.index}."; else -> "•" }
        val markerPaint = textPaint.copy(sizePx = max(1f, textPaint.sizePx * max(0.01f, theme.listMarkerScale)), color = theme.listMarkerColor)
        if (label.isEmpty()) {
            val side = max(markerPaint.sizePx.toInt(), markerPaint.newTextPaint().fontMetricsInt.descent - markerPaint.newTextPaint().fontMetricsInt.ascent)
            return PreparedMarker(null, label, side, side, side, 0, side, context.checked)
        }
        val layout = staticLayout(SpannableString(label), markerPaint, max(1, ceil(markerPaint.newTextPaint().measureText(label)).toInt()))
        val ascent = min(layout.height, max(0, -layout.getLineAscent(0)))
        val descent = min(max(0, layout.height - ascent), max(0, layout.getLineDescent(0)))
        val baseline = layout.getLineBaseline(0).coerceIn(ascent, max(ascent, layout.height - descent))
        return PreparedMarker(layout, label, layout.width, layout.height, ascent, descent, baseline, context.checked)
    }

    private fun markerFragment(marker: PreparedMarker, textX: Int, baseline: Int, gutter: Int, color: Int): PreparedProseFragment {
        val x = textX - gutter + (gutter - marker.widthPx)
        val y = baseline - marker.baselinePx
        return PreparedProseFragment(PreparedProseFragmentKind.MARKER, Rect(x, y, x + marker.widthPx, y + marker.heightPx), marker.layout, x, y, color = color, label = marker.label, checked = marker.checked)
    }

    private fun atomAppearance(nodeType: String, attrsJson: String, base: PreparedTextPaint, theme: PreparedProseTheme): PreparedAtomAppearance {
        if (nodeType == "mention") {
            val values = runCatching { org.json.JSONObject(attrsJson) }.getOrNull()
            val local = EditorMentionTheme.fromJson(values?.optJSONObject("mentionTheme"))
            val mention = theme.mention?.mergedWith(local) ?: local
            val weighted = mention?.fontWeight?.let { EditorTextStyle(fontWeight = it).typefaceStyle() } ?: base.typeface.style
            return PreparedAtomAppearance(
                base.copy(typeface = Typeface.create(base.typeface, weighted), color = mention?.textColor ?: base.color),
                mention?.backgroundColor ?: 0x1F007AFF,
                mention?.borderColor,
                max(0f, (mention?.borderWidth ?: 0f) * theme.density),
                max(0f, (mention?.borderRadius ?: 6f) * theme.density),
                theme.atomPaddingHorizontalPx,
                theme.atomPaddingVerticalPx,
            )
        }
        return PreparedAtomAppearance(base, 0xFFF2F2F7.toInt(), null, 0f, 5f * theme.density, theme.atomPaddingHorizontalPx, theme.atomPaddingVerticalPx)
    }
}

private fun Typeface.familyName(): String? = when (this) { Typeface.MONOSPACE -> "monospace"; Typeface.SERIF -> "serif"; Typeface.SANS_SERIF -> "sans"; else -> null }
private fun parseColor(raw: String?): Int? = runCatching { raw?.takeIf { it.isNotBlank() }?.let(Color::parseColor) }.getOrNull()
