package com.apollohg.editor.viewer

import com.apollohg.editor.applyPhysicalTextAlignment

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
import com.apollohg.editor.EditorBoxStyle
import com.apollohg.editor.EditorEdges
import com.apollohg.editor.EditorResolvedTextSpan
import com.apollohg.editor.EditorLinkTheme
import com.apollohg.editor.EditorMentionTheme
import com.apollohg.editor.EditorOrderedListMarkerTheme
import com.apollohg.editor.EditorTextStyle
import com.apollohg.editor.EditorTheme
import com.apollohg.editor.OrderedListMarkerFormatter
import com.apollohg.editor.ProseViewerError
import java.text.Bidi
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min


/** Creates immutable StaticLayout fragments without depending on a mounted View. */
internal interface AndroidProseLayoutEngine {
    fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout

    /** Registry callers always provide the immutable semantic warning scope. */
    fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
        semanticGenerationIdentity: String,
    ): PreparedProseLayout = prepare(document, key, theme, widthPx, density, collapsesWhenEmpty)
}


private data class PreparedAtomAppearance(val paint: PreparedTextPaint, val background: Int, val borderColor: Int?, val borderWidth: Float, val radius: Float, val paddingHorizontal: Int, val paddingVertical: Int, val box: EditorBoxStyle? = null, val inset: EditorEdges? = null)
private data class PreparedAtomSpec(
    val start: Int,
    val nodeType: String,
    val docPos: Long,
    val attrsJson: String,
    val label: String,
    val appearance: PreparedAtomAppearance,
    val widthPx: Int,
    val heightPx: Int,
    val labelLayout: StaticLayout,
    val labelBaselinePx: Int,
)
private data class PreparedMarker(val layout: StaticLayout?, val label: String, val widthPx: Int, val heightPx: Int, val ascentPx: Int, val baselinePx: Int, val checked: Boolean, val checkbox: com.apollohg.editor.EditorElementStyle? = null)

internal const val PREPARED_LIST_MARKER_GAP_DP = 6f

internal data class PreparedMarkerInk(val ascentPx: Int, val heightPx: Int)

internal fun preparedMarkerInk(glyphBounds: Rect, layoutAscentPx: Int, layoutDescentPx: Int): PreparedMarkerInk {
    if (glyphBounds.isEmpty) {
        val ascent = max(0, -layoutAscentPx)
        return PreparedMarkerInk(ascent, max(1, ascent + max(0, layoutDescentPx)))
    }
    return PreparedMarkerInk(-glyphBounds.top, max(1, glyphBounds.bottom - glyphBounds.top))
}

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

    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout = prepare(
        document,
        key,
        theme,
        widthPx,
        density,
        collapsesWhenEmpty,
        key.semanticGenerationIdentity,
    )

    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
        semanticGenerationIdentity: String,
    ): PreparedProseLayout {
        val warningSemanticGeneration = semanticGenerationIdentity
        if (widthPx <= 0 || !density.isFinite() || density <= 0f) return PreparedProseLayout.error(key, 0, ProseViewerError.invalidWidth())
        val visibleBlocks = if (collapsesWhenEmpty) {
            document.blocks.dropLast(document.trailingEmptyTextBlockCount.coerceAtMost(document.blocks.size))
        } else {
            document.blocks
        }
        if (visibleBlocks.isEmpty() && collapsesWhenEmpty) return PreparedProseLayout(key, widthPx, 0, emptyList(), retainedBytes = document.retainedBytes)
        val contentWidth = max(1, widthPx - theme.insetLeftPx - theme.insetRightPx)
        var cursorY = theme.insetTopPx
        var retained = document.retainedBytes + theme.retainedBytes
        val interactions = mutableListOf<PreparedProseInteraction>()
        val imageAttachments = mutableListOf<ViewerImageAttachment>()
        val viewerAtoms = mutableListOf<PreparedViewerAtom>()
        val highlightedCodeKeys = mutableSetOf<String>()
        val markers = mutableMapOf<Int, PreparedMarker>()
        visibleBlocks.forEach { block ->
            listItemAncestors(block).forEachIndexed { nestingDepth, ancestor ->
                if (markers[ancestor.identity] == null) {
                    markers[ancestor.identity] = markerFor(
                        ancestor.context,
                        nestingDepth,
                        theme.paintFor(block),
                        theme,
                    )
                }
            }
        }
        val containerBounds = mutableMapOf<Int, Rect>()
        val sheet = theme.sourceTheme?.styleSheet
        var blocks = visibleBlocks.mapIndexed { index, block ->
            val nextAncestorIdentities = visibleBlocks.getOrNull(index + 1)
                ?.let(::listItemAncestors)
                ?.mapTo(mutableSetOf()) { it.identity }
                .orEmpty()
            val disappearingListItemIdentities = listItemAncestors(block)
                .filter { it.identity !in nextAncestorIdentities }
                .mapTo(mutableSetOf()) { it.identity }
            val containers = if (sheet == null) emptyList() else block.containers
            var containerInset = EditorEdges()
            containers.forEach { ancestor ->
                val box = sheet!!.box(ancestor.nodeType).scaled(density)
                if (ancestor.firstLeaf == index) {
                    cursorY += box.margin.top.toInt()
                    containerBounds[ancestor.identity] = Rect(theme.insetLeftPx + containerInset.left.toInt() + box.margin.left.toInt(), cursorY, widthPx - theme.insetRightPx - containerInset.right.toInt() - box.margin.right.toInt(), cursorY)
                    cursorY += box.inset.top.toInt()
                }
                containerInset += box.outerInset.copy(top = 0f, bottom = 0f)
            }
            val box = sheet?.box(block.nodeType)?.scaled(density) ?: EditorBoxStyle()
            val outer = box.outerInset
            val leafTop = cursorY
            val leftInset = (containerInset.left + outer.left).toInt()
            val rightInset = (containerInset.right + outer.right).toInt()
            var prepared = prepareBlock(
                block,
                imageAttachments.size,
                markers,
                if (sheet == null) theme else theme.copy(insetLeftPx = theme.insetLeftPx + leftInset, listItemSpacingPx = 0, listSpacingAfterPx = 0),
                max(1, contentWidth - leftInset - rightInset),
                cursorY + outer.top.toInt(),
                disappearingListItemIdentities,
                warningSemanticGeneration,
            )
            if (sheet != null) {
                val end = prepared.nextY + outer.bottom.toInt()
                val bounds = Rect(theme.insetLeftPx + containerInset.left.toInt() + box.margin.left.toInt(), leafTop + box.margin.top.toInt(), widthPx - theme.insetRightPx - containerInset.right.toInt() - box.margin.right.toInt(), end - box.margin.bottom.toInt())
                prepared.attachment?.let { attachment ->
                    bounds.left = attachment.bounds.left - box.inset.left.toInt()
                    bounds.right = attachment.bounds.right + box.inset.right.toInt()
                }
                val decoration = PreparedProseFragment(PreparedProseFragmentKind.BACKGROUND, bounds, box = box)
                val seed = Rect(theme.insetLeftPx, leafTop, widthPx - theme.insetRightPx, end)
                prepared = prepared.copy(block = PreparedProseBlock(listOf(decoration) + prepared.block.fragments, seed), nextY = end)
            }
            cursorY = prepared.nextY
            containers.asReversed().forEach { ancestor ->
                if (ancestor.lastLeaf == index) {
                    val box = sheet!!.box(ancestor.nodeType).scaled(density)
                    cursorY += box.inset.bottom.toInt()
                    containerBounds[ancestor.identity]?.bottom = cursorY
                    cursorY += box.margin.bottom.toInt()
                }
            }
            retained += prepared.block.retainedBytes + prepared.extraBytes
            interactions += prepared.interactions
            prepared.highlightedCodeKey?.let(highlightedCodeKeys::add)
            prepared.attachment?.let(imageAttachments::add)
            prepared.viewerAtom?.let { viewerAtoms += it; retained += it.retainedBytes }
            prepared.block
        }
        if (sheet != null) {
            blocks = blocks.mapIndexed { index, prepared ->
                val decorations = visibleBlocks[index].containers.mapNotNull { ancestor ->
                    val bounds = containerBounds[ancestor.identity] ?: return@mapNotNull null
                    val clip = Rect(bounds.left, if (ancestor.firstLeaf == index) bounds.top else prepared.bounds.top, bounds.right, if (ancestor.lastLeaf == index) bounds.bottom else prepared.bounds.bottom)
                    PreparedProseFragment(PreparedProseFragmentKind.BACKGROUND, clip, box = sheet.box(ancestor.nodeType).scaled(density), decorationBounds = bounds)
                }
                prepared.copy(fragments = decorations + prepared.fragments, bounds = Rect(prepared.bounds).apply { decorations.forEach { union(it.bounds) } })
            }
        }
        val height = max(0, (if (sheet != null) max(cursorY, blocks.maxOfOrNull { it.bounds.bottom } ?: 0) else blocks.maxOfOrNull { it.bounds.bottom } ?: cursorY) + theme.insetBottomPx)
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
        val highlightBlocks = if (theme.codeHighlighting == null) emptyList() else visibleBlocks.mapIndexedNotNull { index, block ->
            if (block.nodeType != "codeBlock") null else com.apollohg.editor.CodeHighlightBlock(index, codeText(block), block.language)
        }
        retained += highlightBlocks.sumOf { 64L + it.text.length * 2L }
        // Mounted image-publication sidecars are runtime surface ownership,
        // not immutable artifact/cache ownership; account them at the host.
        return PreparedProseLayout(key, widthPx, height, blocks, interactions, nodes, imageAttachments, retained, viewerAtoms = viewerAtoms, contentBox = sheet?.box("content")?.scaled(density), codeHighlighting = theme.codeHighlighting, codeHighlightBlocks = highlightBlocks, highlightedCodeKeys = highlightedCodeKeys.toSet())
    }

    private data class BlockResult(val block: PreparedProseBlock, val interactions: List<PreparedProseInteraction>, val attachment: ViewerImageAttachment? = null, val nextY: Int, val extraBytes: Long, val viewerAtom: PreparedViewerAtom? = null, val highlightedCodeKey: String? = null)

    private fun prepareBlock(block: ViewerBlock, attachmentOrdinal: Int, measuredMarkers: Map<Int, PreparedMarker>, theme: PreparedProseTheme, contentWidth: Int, cursorY: Int, disappearingListItemIdentities: Set<Int>, warningSemanticGeneration: String): BlockResult {
        val paint = theme.paintFor(block)
        val ancestors = listItemAncestors(block)
        val ancestorMarkers = ancestors.mapNotNull { ancestor -> measuredMarkers[ancestor.identity]?.let { ancestor to it } }
        val firstMarkers = ancestorMarkers.filter { (ancestor, _) -> ancestor.isFirstRenderableLeaf }
        fun listStyle(ancestor: ViewerListItemAncestor) = theme.sourceTheme?.styleSheet?.get(if (ancestor.context.kind == "task") "taskList" else if (ancestor.context.ordered) "orderedList" else "bulletList")
        fun listIndent(ancestor: ViewerListItemAncestor) = listStyle(ancestor)?.indent?.times(theme.density)?.toInt() ?: theme.listIndentPx
        val baseListInset = if (ancestors.isEmpty()) 0 else max(0, (listIndent(ancestors.first()) * (listStyle(ancestors.first())?.baseIndentMultiplier ?: theme.listBaseIndentMultiplier)).toInt())
        val ancestorGutters = ancestorMarkers.associate { (ancestor, marker) -> ancestor.identity to (marker.widthPx + (marker.checkbox?.gap?.times(theme.density)?.toInt() ?: theme.listMarkerGapPx)) }
        // A nested leaf owns every outer list column too: each ancestor adds
        // its list indent and independently measured marker gutter.
        val listInset = baseListInset + ancestorMarkers.sumOf { (ancestor, _) -> listIndent(ancestor) + (ancestorGutters[ancestor.identity] ?: 0) }
        val quoteInset = if (block.inBlockquote && theme.sourceTheme?.styleSheet == null) theme.quoteBorderWidthPx + theme.quoteMarkerGapPx + theme.quoteIndentPx else 0
        val codeInset = if (block.nodeType == "codeBlock") theme.codePaddingHorizontalPx else 0
        val textX = theme.insetLeftPx + listInset + quoteInset + codeInset
        val itemSpacing = if (ancestors.isEmpty()) {
            paint.spacingAfterPx
        } else {
            ancestors.sumOf { ancestor ->
                when {
                    ancestor.identity in disappearingListItemIdentities && ancestor.context.isLast -> theme.listSpacingAfterPx
                    ancestor.identity in disappearingListItemIdentities -> theme.listItemSpacingPx
                    ancestor.isFinalRenderableLeaf -> theme.listItemSpacingPx
                    else -> 0
                }
            }
        }
        fun markerAnchor(ancestor: ViewerListItemAncestor): Int {
            var inset = baseListInset
            ancestorMarkers.forEach { (candidate, _) ->
                inset += listIndent(candidate) + (ancestorGutters[candidate.identity] ?: 0)
                if (candidate.identity == ancestor.identity) return theme.insetLeftPx + quoteInset + inset
            }
            return textX - codeInset
        }
        val customAtom = (block.inlines.singleOrNull() as? ViewerInline.Atom)?.takeIf {
            block.isBlockAtom && theme.viewerAtoms?.nodeTypes?.contains(it.nodeType) == true
        }
        if (customAtom != null) {
            val atomWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
            val atomHeight = requireNotNull(theme.viewerAtoms).heightPx(customAtom, atomWidth, theme.density)
            val bounds = Rect(textX, cursorY, textX + atomWidth, cursorY + atomHeight)
            val fragments = mutableListOf<PreparedProseFragment>()
            if (block.inBlockquote && theme.sourceTheme?.styleSheet == null) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, bounds.bottom), color = theme.quoteBorderColor)
            firstMarkers.forEach { (ancestor, marker) ->
                fragments += markerFragment(marker, markerAnchor(ancestor), cursorY, bounds.bottom, ancestorGutters.getValue(ancestor.identity), theme.listMarkerColor)
            }
            return finishBlock(fragments, emptyList(), Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, bounds.bottom), bounds.bottom, itemSpacing).copy(
                viewerAtom = PreparedViewerAtom(customAtom.nodeType, customAtom.docPos, customAtom.attrsJson, bounds),
            )
        }
        if (block.nodeType == "image") {
            val source = ViewerImageAttachment.sourceAndDeclaredSize(block)
            if (source != null) {
                val availableImageWidth = max(1, contentWidth - listInset - quoteInset)
                val imageWidth = if (theme.sourceTheme?.styleSheet != null && source.third != null) minOf(availableImageWidth, (source.third!!.first * theme.density).toInt().coerceAtLeast(1)) else availableImageWidth
                val resolved = source.third ?: ViewerImageIntrinsicStore.shared.size(source.first)
                val imageHeight = resolved?.let { imageWidth * it.second / max(1, it.first) } ?: max(44, minOf(240, (imageWidth * .56f).toInt()))
                val bounds = Rect(textX, cursorY, textX + imageWidth, cursorY + imageHeight)
                val attachment = ViewerImageAttachment(source.first, source.second, bounds, source.third, attachmentOrdinal)
                return BlockResult(PreparedProseBlock(listOf(PreparedProseFragment(PreparedProseFragmentKind.IMAGE, bounds, color = theme.sourceTheme?.styleSheet?.box("image")?.backgroundColor ?: 0xFFF2F2F7.toInt(), box = theme.sourceTheme?.styleSheet?.box("image")?.scaled(theme.density)?.let { it.copy(border = EditorEdges(), padding = EditorEdges(), corners = com.apollohg.editor.EditorBoxDrawing.innerCorners(it)) }, resizeMode = theme.sourceTheme?.styleSheet?.get("image")?.resizeMode ?: "contain")), bounds), emptyList(), attachment, bounds.bottom + itemSpacing, 192)
            }
        }
        if (block.nodeType == "horizontalRule" || block.nodeType == "horizontal_rule") {
            val ruleTop = cursorY + theme.ruleMarginPx
            val ruleLeft = theme.insetLeftPx + listInset + quoteInset
            val ruleRight = max(ruleLeft + 1, theme.insetLeftPx + contentWidth - listInset - quoteInset)
            val rule = Rect(ruleLeft, ruleTop, ruleRight, ruleTop + theme.ruleThicknessPx)
            val fragments = mutableListOf(PreparedProseFragment(PreparedProseFragmentKind.RULE, rule, color = theme.ruleColor, strokeWidth = theme.ruleThicknessPx.toFloat()))
            val end = rule.bottom + theme.ruleMarginPx
            if (block.inBlockquote && theme.sourceTheme?.styleSheet == null) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, end), color = theme.quoteBorderColor)
            firstMarkers.forEach { (ancestor, marker) ->
                fragments += markerFragment(marker, markerAnchor(ancestor), cursorY, end, ancestorGutters.getValue(ancestor.identity), theme.listMarkerColor)
            }
            return finishBlock(fragments, emptyList(), Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, end), end, itemSpacing)
        }

        val availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
        val attributed = attributed(block.inlines, paint, theme, warningSemanticGeneration)
        var highlightedCodeKey: String? = null
        if (block.nodeType == "codeBlock") theme.codeHighlighting?.let { config ->
            val code = com.apollohg.editor.CodeHighlightBlock(0, codeText(block), block.language)
            ViewerCodeHighlightCache.get(config, code)?.let { ranges ->
                highlightedCodeKey = ViewerCodeHighlightCache.key(config, code)
                ranges.forEach { range ->
                    if (range.start >= 0 && range.start + range.length <= attributed.text.length) attributed.text.setSpan(com.apollohg.editor.EditorCodeHighlightSpan(range), range.start, range.start + range.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
                }
            }
        }
        val layout = staticLayout(attributed.text, paint, availableWidth)
        val codeTopInset = if (block.nodeType == "codeBlock") theme.codePaddingVerticalPx else 0
        val firstLineHeight = layout.getLineBottom(0) - layout.getLineTop(0)
        val markerTopProtection = firstMarkers.maxOfOrNull { (_, marker) ->
            max(0, ceil((marker.heightPx - firstLineHeight) / 2f).toInt() - codeTopInset)
        } ?: 0
        val textTop = cursorY + codeTopInset + if (cursorY == theme.insetTopPx) markerTopProtection else 0
        val textHeight = max(1, layout.height)
        val totalEnd = textTop + textHeight + if (block.nodeType == "codeBlock") theme.codePaddingVerticalPx else 0
        val fragments = mutableListOf<PreparedProseFragment>()
        val interactionRects = MutableList(attributed.semanticRanges.size) { mutableListOf<Rect>() }
        if (block.nodeType == "codeBlock" && theme.sourceTheme?.styleSheet == null) fragments += PreparedProseFragment(PreparedProseFragmentKind.BACKGROUND, Rect(theme.insetLeftPx + listInset + quoteInset, cursorY, theme.insetLeftPx + contentWidth - listInset - quoteInset, totalEnd), color = theme.codeBackground, cornerRadius = theme.codeRadiusPx)
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
            val atomTop = baseline - (atom.appearance.inset?.top?.toInt() ?: atom.appearance.paddingVertical) - atom.labelBaselinePx
            val bounds = Rect(atomLeft, atomTop, max(atomLeft + 1, atomRight), atomTop + atom.heightPx)
            fragments += PreparedProseFragment(
                kind = PreparedProseFragmentKind.ATOM,
                bounds = bounds,
                labelLayout = atom.labelLayout,
                labelX = bounds.left + (atom.appearance.inset?.left?.toInt() ?: atom.appearance.paddingHorizontal),
                labelY = bounds.top + (atom.appearance.inset?.top?.toInt() ?: atom.appearance.paddingVertical),
                color = atom.appearance.background,
                borderColor = atom.appearance.borderColor,
                cornerRadius = atom.appearance.radius,
                strokeWidth = atom.appearance.borderWidth,
                label = atom.label,
                atomNodeType = atom.nodeType,
                atomDocPos = atom.docPos,
                atomAttrsJson = atom.attrsJson,
                box = atom.appearance.box,
            )
        }
        if (block.inBlockquote && theme.sourceTheme?.styleSheet == null) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, totalEnd), color = theme.quoteBorderColor)
        val firstLineTop = textTop + layout.getLineTop(0)
        val firstLineBottom = textTop + layout.getLineBottom(0)
        firstMarkers.forEach { (ancestor, marker) ->
            fragments += markerFragment(marker, markerAnchor(ancestor), firstLineTop, firstLineBottom, ancestorGutters.getValue(ancestor.identity), theme.listMarkerColor)
        }
        val interactions = attributed.semanticRanges.zip(interactionRects).mapNotNull { (semantic, rects) ->
            if (rects.isEmpty()) null else when (semantic) {
                is PreparedSemanticRange.Link -> PreparedProseInteraction(PreparedProseInteraction.Kind.LINK, rects, semantic.href, semantic.text, null, semantic.text, null)
                is PreparedSemanticRange.Mention -> PreparedProseInteraction(PreparedProseInteraction.Kind.MENTION, rects, null, semantic.label, semantic.docPos, semantic.label, semantic.attrsJson)
            }
        }
        return finishBlock(fragments, interactions, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, totalEnd), totalEnd, itemSpacing, attributed.retainedBytes).copy(highlightedCodeKey = highlightedCodeKey)
    }

    private fun codeText(block: ViewerBlock): String = block.inlines.joinToString("") {
        when (it) { is ViewerInline.Text -> it.text; is ViewerInline.Atom -> if (it.nodeType == "hardBreak" || it.nodeType == "hard_break") "\n" else "\uFFFC" }
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
        return BlockResult(PreparedProseBlock(fragments.toList(), bounds), interactions, null, end + spacing, extraBytes)
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
        if (result.isEmpty()) {
            result += fallbackSelectionRectsForLine(layout, start, end, line, width)
        }
        return result.sortedWith(compareBy<Rect> { it.top }.thenBy { it.left })
    }

    private data class AttributedBlock(val text: SpannableString, val atoms: List<PreparedAtomSpec>, val semanticRanges: List<PreparedSemanticRange>, val retainedBytes: Long)
    private sealed interface PreparedSemanticRange { val start: Int; val end: Int
        data class Link(override val start: Int, override val end: Int, val href: String, val text: String) : PreparedSemanticRange
        data class Mention(override val start: Int, override val end: Int, val docPos: Long, val label: String, val attrsJson: String) : PreparedSemanticRange
    }

    private fun attributed(inlines: List<ViewerInline>, base: PreparedTextPaint, theme: PreparedProseTheme, warningSemanticGeneration: String): AttributedBlock {
        val source = StringBuilder()
        val spans = mutableListOf<(SpannableString) -> Unit>()
        val atoms = mutableListOf<PreparedAtomSpec>()
        val semanticRanges = mutableListOf<PreparedSemanticRange>()
        inlines.forEach { inline -> when (inline) {
            is ViewerInline.Text -> {
                val start = source.length
                source.append(inline.text)
                val end = source.length
                val markSpans = markSpans(inline.marks, base, theme, warningSemanticGeneration)
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
                    val atomInset = appearance.inset ?: EditorEdges(appearance.paddingVertical.toFloat(), appearance.paddingHorizontal.toFloat(), appearance.paddingVertical.toFloat(), appearance.paddingHorizontal.toFloat())
                    val width = max(base.sizePx.toInt(), ceil(labelPaint.measureText(label) + atomInset.left + atomInset.right).toInt())
                    val labelText = SpannableString(label).apply {
                        if (theme.sourceTheme?.styleSheet != null) appearance.paint.resolvedStyle?.let { setSpan(EditorResolvedTextSpan(it.copy(backgroundColor = null), theme.fontDensity), 0, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE) }
                    }
                    val labelLayout = staticLayout(labelText, appearance.paint, max(1, width - (atomInset.left + atomInset.right).toInt()))
                    val labelMetrics = labelPaint.fontMetricsInt
                    val labelAscent = max(0, -labelMetrics.ascent)
                    val labelDescent = max(0, labelMetrics.descent)
                    val ascent = labelAscent + atomInset.top.toInt()
                    // Keep the label's descenders and any resolved line-height
                    // expansion in the outer replacement metrics.
                    val descent = labelDescent + atomInset.bottom.toInt()
                    val metricDescent = descent + max(0, labelLayout.height + (atomInset.top + atomInset.bottom).toInt() - ascent - descent)
                    val height = ascent + metricDescent
                    val start = source.length
                    source.append('\uFFFC')
                    atoms += PreparedAtomSpec(
                        start = start,
                        nodeType = inline.nodeType,
                        docPos = inline.docPos,
                        attrsJson = inline.attrsJson,
                        label = label,
                        appearance = appearance,
                        widthPx = width,
                        heightPx = height,
                        labelLayout = labelLayout,
                        labelBaselinePx = labelLayout.getLineBaseline(0),
                    )
                    spans += { value -> value.setSpan(AtomMetricSpan(width, ascent, metricDescent), start, start + 1, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE) }
                    if (inline.nodeType == "mention") semanticRanges += PreparedSemanticRange.Mention(start, start + 1, inline.docPos, label, inline.attrsJson)
                }
            }
        } }
        val text = SpannableString(if (source.isEmpty()) "\u200B" else source.toString())
        spans.forEach { it(text) }
        return AttributedBlock(text, atoms, semanticRanges, 256L + text.length * 52L + atoms.sumOf { 256L + it.label.length * 2L })
    }

    private fun href(marks: List<uniffi.editor_core.FfiViewerMark>): String? = marks.firstOrNull { it.markType == "link" }
        ?.let { runCatching { org.json.JSONObject(it.attrsJson).optString("href") }.getOrNull()?.takeIf(String::isNotEmpty) }

    private fun markSpans(marks: List<uniffi.editor_core.FfiViewerMark>, base: PreparedTextPaint, theme: PreparedProseTheme, warningSemanticGeneration: String): List<Any> {
        theme.sourceTheme?.styleSheet?.let { sheet ->
            var resolved = base.resolvedStyle ?: EditorTextStyle()
            val active = marks.map { com.apollohg.editor.canonicalMark(it.markType) }.toSet()
            listOf("inlineCode", "bold", "italic", "link", "underline", "strike").filter { it in active }.forEach {
                resolved = resolved.mergedWith(com.apollohg.editor.semanticText(it)).mergedWith(sheet[it]?.text)
            }
            marks.forEach { mark ->
                val attrs = runCatching { org.json.JSONObject(mark.attrsJson) }.getOrNull()
                val override = when (mark.markType) {
                    "textColor", "color", "foregroundColor" -> EditorTextStyle(color = parseColor(attrs?.optionalString("color") ?: attrs?.optionalString("textColor")))
                    "highlight", "backgroundColor" -> EditorTextStyle(backgroundColor = parseColor(attrs?.optionalString("color") ?: attrs?.optionalString("backgroundColor")))
                    "textStyle", "font" -> EditorTextStyle(fontFamily = attrs?.optionalString("fontFamily"), fontSize = attrs?.optDouble("fontSize", Double.NaN)?.takeIf { it.isFinite() && it > 0 }?.toFloat())
                    else -> null
                }
                resolved = resolved.mergedWith(override)
            }
            return listOf(EditorResolvedTextSpan(resolved, theme.fontDensity))
        }
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
                "textColor", "color", "foregroundColor" -> explicitColor = parseColor(attrs?.optionalString("color") ?: attrs?.optionalString("textColor")) ?: explicitColor
                "highlight", "backgroundColor" -> background = parseColor(attrs?.optionalString("color") ?: attrs?.optionalString("backgroundColor")) ?: background
                "textStyle", "font" -> { family = attrs?.optionalString("fontFamily") ?: family; size = attrs?.optDouble("fontSize", Double.NaN)?.takeIf { it.isFinite() && it > 0 }?.toFloat() ?: size }
            }
        }
        // Resolve link family/size/weight/style first, then combine explicit
        // mark traits into one immutable metric span before StaticLayout sees it.
        var resolved = link?.let { base.withStyle(it.asTextStyle(), theme.fontDensity, warningSemanticGeneration) } ?: base
        if (family != null || size != null) {
            resolved = resolved.withStyle(EditorTextStyle(fontFamily = family, fontSize = size), theme.fontDensity, warningSemanticGeneration)
        }
        if (monospace) resolved = resolved.withStyle(EditorTextStyle(fontFamily = "monospace"), theme.fontDensity, warningSemanticGeneration)
        if (bold || italic) {
            resolved = resolved.withStyle(
                EditorTextStyle(
                    fontWeight = if (bold) "bold" else null,
                    fontStyle = if (italic) "italic" else null,
                ),
                theme.fontDensity,
                warningSemanticGeneration,
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
        val preparedText = SpannableString(text).apply {
            // The full prepared range includes a single line and the final line
            // after a hard break; builder line spacing does not provide that
            // guarantee and would double-compensate these metrics.
            if (getSpans(0, length, EditorResolvedTextSpan::class.java).isNotEmpty()) {
                setSpan(com.apollohg.editor.EditorStyledLineMetricsSpan(), 0, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            } else paint.lineHeightPx?.let { setSpan(FixedLineHeightMetricSpan(it), 0, length, Spanned.SPAN_INCLUSIVE_INCLUSIVE) }
            paint.textAlign?.let { applyPhysicalTextAlignment(it) }
        }
        return StaticLayout.Builder.obtain(preparedText, 0, preparedText.length, resolved, width)
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .apply { if (android.os.Build.VERSION.SDK_INT >= 26) setJustificationMode(if (paint.textAlign == "justify") Layout.JUSTIFICATION_MODE_INTER_WORD else Layout.JUSTIFICATION_MODE_NONE) }
            .setIncludePad(false)
            .setBreakStrategy(Layout.BREAK_STRATEGY_HIGH_QUALITY)
            .build()
    }

    private fun markerFor(
        context: ViewerListContext,
        nestingDepth: Int,
        textPaint: PreparedTextPaint,
        theme: PreparedProseTheme,
    ): PreparedMarker {
        if (context.kind == "task" && theme.sourceTheme?.styleSheet != null) {
            val checkbox = com.apollohg.editor.resolvedCheckboxStyle(theme.sourceTheme.styleSheet, context.checked)
            val side = ((checkbox.size ?: 18f) * theme.density).toInt()
            return PreparedMarker(null, "", side, side, side / 2, 0, context.checked, checkbox.copy(box = checkbox.box.scaled(theme.density)))
        }
        val label = when {
            context.kind == "task" -> ""
            context.ordered -> OrderedListMarkerFormatter.label(
                context.index,
                nestingDepth,
                theme.orderedListMarker,
            )
            else -> "•"
        }
        val markerScale = if (!context.ordered && context.kind != "task") max(0.01f, theme.listMarkerScale) else 1f
        val markerPaint = textPaint.copy(sizePx = max(1f, textPaint.sizePx * markerScale), color = theme.listMarkerColor)
        if (label.isEmpty()) {
            val side = max(markerPaint.sizePx.toInt(), markerPaint.newTextPaint().fontMetricsInt.descent - markerPaint.newTextPaint().fontMetricsInt.ascent)
            return PreparedMarker(null, label, side, side, side / 2, 0, context.checked)
        }
        val text = markerPaint.newTextPaint()
        val layout = staticLayout(SpannableString(label), markerPaint, max(1, ceil(text.measureText(label)).toInt()))
        val glyphBounds = Rect()
        text.getTextBounds(label, 0, label.length, glyphBounds)
        val ink = preparedMarkerInk(glyphBounds, layout.getLineAscent(0), layout.getLineDescent(0))
        return PreparedMarker(layout, label, layout.width, ink.heightPx, ink.ascentPx, layout.getLineBaseline(0), context.checked)
    }

    private fun markerFragment(marker: PreparedMarker, textX: Int, verticalTop: Int, verticalBottom: Int, gutter: Int, color: Int): PreparedProseFragment {
        val x = textX - gutter
        val markerTop = verticalTop + (verticalBottom - verticalTop - marker.heightPx) / 2
        val layoutY = markerTop + marker.ascentPx - marker.baselinePx
        return PreparedProseFragment(PreparedProseFragmentKind.MARKER, Rect(x, markerTop, x + marker.widthPx, markerTop + marker.heightPx), marker.layout, x, layoutY, color = color, label = marker.label, checked = marker.checked, box = marker.checkbox?.box, borderColor = marker.checkbox?.checkColor)
    }

    private fun atomAppearance(nodeType: String, attrsJson: String, base: PreparedTextPaint, theme: PreparedProseTheme): PreparedAtomAppearance {
        if (nodeType == "mention") {
            val values = runCatching { org.json.JSONObject(attrsJson) }.getOrNull()
            val local = EditorMentionTheme.fromJson(values?.optJSONObject("mentionTheme"))
            if (theme.sourceTheme?.styleSheet != null) {
                val element = com.apollohg.editor.resolvedMentionStyle(base.resolvedStyle ?: EditorTextStyle(), theme.sourceTheme, local)
                val box = element.box.scaled(theme.density)
                return PreparedAtomAppearance(base.withStyle(element.text, theme.fontDensity, "mention"), box.backgroundColor ?: 0, null, 0f, 0f, 0, 0, box, box.inset)
            }
            val mention = (theme.mention?.mergedWith(local) ?: local)?.node
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

internal fun Typeface.familyName(): String? = when (this) { Typeface.MONOSPACE -> "monospace"; Typeface.SERIF -> "serif"; Typeface.SANS_SERIF -> "sans"; else -> null }
private fun parseColor(raw: String?): Int? = runCatching { raw?.takeIf { it.isNotBlank() }?.let(Color::parseColor) }.getOrNull()
