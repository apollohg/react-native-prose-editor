package com.apollohg.editor.viewer

import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.Typeface
import android.text.Layout
import android.text.SpannableString
import android.text.StaticLayout
import android.text.TextPaint
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
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

/** Creates immutable StaticLayout fragments without depending on a mounted View. */
internal interface AndroidProseLayoutEngine {
    fun prepare(document: ViewerDocument, key: ProseLayoutKey, widthPx: Int, density: Float, collapsesWhenEmpty: Boolean): PreparedProseLayout
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

internal data class PreparedProseTheme(
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
) {
    companion object {
        fun resolve(themeJson: String?, density: Float): PreparedProseTheme {
            val theme = EditorTheme.fromJson(themeJson) ?: EditorTheme()
            fun px(value: Float, fallback: Float): Int = max(0, (value.takeIf { it.isFinite() } ?: fallback).times(density).toInt())
            fun typeface(style: EditorTextStyle?, fallback: Typeface): Typeface {
                val family = style?.fontFamily
                return Typeface.create(family ?: fallback.familyName(), style?.typefaceStyle() ?: fallback.style)
            }
            fun paint(style: EditorTextStyle?, fallback: PreparedTextPaint? = null): PreparedTextPaint {
                val base = fallback ?: PreparedTextPaint(Typeface.DEFAULT, 17f * density, 0xFF212121.toInt(), null, 0)
                return PreparedTextPaint(
                    typeface = typeface(style, base.typeface),
                    sizePx = ((style?.fontSize ?: (base.sizePx / density)) * density).takeIf { it.isFinite() && it > 0f } ?: base.sizePx,
                    color = style?.color ?: base.color,
                    lineHeightPx = style?.lineHeight?.let { px(it, 0f).takeIf { value -> value > 0 } } ?: base.lineHeightPx,
                    spacingAfterPx = style?.spacingAfter?.let { px(it, 0f) } ?: base.spacingAfterPx,
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
                text, paragraph, headings, quote, paint(theme.effectiveTextStyle("codeBlock"), codeFallback),
                px(theme.contentInsets?.top ?: 0f, 0f), px(theme.contentInsets?.right ?: 0f, 0f), px(theme.contentInsets?.bottom ?: 0f, 0f), px(theme.contentInsets?.left ?: 0f, 0f),
                px(theme.list?.indent ?: 28f, 28f), theme.list?.baseIndentMultiplier ?: 1f, px(theme.list?.itemSpacing ?: 4f, 4f), theme.list?.markerColor ?: text.color, theme.list?.markerScale ?: 1f,
                px(theme.blockquote?.indent ?: 16f, 16f), theme.blockquote?.borderColor ?: 0xFFC7C7CC.toInt(), px(theme.blockquote?.borderWidth ?: 3f, 3f), px(theme.blockquote?.markerGap ?: 10f, 10f),
                theme.codeBlock?.backgroundColor ?: 0xFFF2F2F7.toInt(), (theme.codeBlock?.borderRadius ?: 8f) * density, px(theme.codeBlock?.paddingHorizontal ?: 12f, 12f), px(theme.codeBlock?.paddingVertical ?: 8f, 8f),
                theme.horizontalRule?.color ?: 0xFFC7C7CC.toInt(), max(1, px(theme.horizontalRule?.thickness ?: 1f, 1f)), px(theme.horizontalRule?.verticalMargin ?: 12f, 12f),
                theme.links, theme.mentions,
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

private data class PreparedAtomAppearance(val paint: PreparedTextPaint, val background: Int, val borderColor: Int?, val borderWidth: Float, val radius: Float, val paddingHorizontal: Int = 6, val paddingVertical: Int = 4)
private data class PreparedAtomSpec(val start: Int, val nodeType: String, val label: String, val appearance: PreparedAtomAppearance, val widthPx: Int, val heightPx: Int)
private data class PreparedMarker(val layout: StaticLayout?, val label: String, val widthPx: Int, val heightPx: Int, val checked: Boolean)

private class AtomMetricSpan(
    private val widthPx: Int,
    private val heightPx: Int,
    private val descentPx: Int,
) : ReplacementSpan() {
    override fun getSize(paint: Paint, text: CharSequence?, start: Int, end: Int, fm: Paint.FontMetricsInt?): Int {
        fm?.let {
            it.ascent = -(heightPx - descentPx)
            it.top = it.ascent
            it.descent = descentPx
            it.bottom = descentPx
        }
        return widthPx
    }
    override fun draw(canvas: android.graphics.Canvas, text: CharSequence?, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) = Unit
}

/** Marker span is semantic-free and lets preparation derive explicit strike rectangles. */
private class StrikeMarkerSpan : android.text.style.CharacterStyle() {
    override fun updateDrawState(tp: TextPaint) = Unit
}

internal class StaticLayoutAndroidProseLayoutEngine : AndroidProseLayoutEngine {
    /** Test seam: drawing must never increment this prepared-layout counter. */
    internal var staticLayoutsBuilt: Int = 0
        private set

    override fun prepare(document: ViewerDocument, key: ProseLayoutKey, widthPx: Int, density: Float, collapsesWhenEmpty: Boolean): PreparedProseLayout {
        if (widthPx <= 0 || !density.isFinite() || density <= 0f) return PreparedProseLayout.error(key, 0, ProseViewerError.invalidWidth())
        if (document.isEmpty && collapsesWhenEmpty) return PreparedProseLayout(key, widthPx, 0, emptyList(), retainedBytes = document.retainedBytes)
        val theme = document.preparedTheme ?: PreparedProseTheme.resolve(null, density)
        val contentWidth = max(1, widthPx - theme.insetLeftPx - theme.insetRightPx)
        var cursorY = theme.insetTopPx
        var retained = document.retainedBytes + theme.retainedBytes
        val markers = mutableMapOf<Int, PreparedMarker>()
        document.blocks.forEach { block ->
            val boundary = block.listItemBoundary ?: return@forEach
            if (markers[boundary.identity] == null) block.listContext?.let { markers[boundary.identity] = markerFor(it, theme.paintFor(block), theme) }
        }
        val blocks = document.blocks.map { block ->
            val marker = block.listItemBoundary?.let { markers[it.identity] }
            val prepared = prepareBlock(block, marker, theme, contentWidth, cursorY)
            cursorY = prepared.nextY
            retained += prepared.block.retainedBytes + prepared.extraBytes
            prepared.block
        }
        val height = max(0, cursorY + theme.insetBottomPx)
        return PreparedProseLayout(key, widthPx, height, blocks, retainedBytes = retained)
    }

    private data class BlockResult(val block: PreparedProseBlock, val nextY: Int, val extraBytes: Long)

    private fun prepareBlock(block: ViewerBlock, measuredMarker: PreparedMarker?, theme: PreparedProseTheme, contentWidth: Int, cursorY: Int): BlockResult {
        val paint = theme.paintFor(block)
        val marker = if (block.listItemBoundary?.isFirstRenderableLeaf == true) measuredMarker else null
        val listDepth = block.listItemBoundary?.nestingDepth ?: max(0, block.depth - 1)
        val markerGutter = measuredMarker?.let { max(6, it.widthPx + 6) } ?: 0
        val listInset = if (block.listContext == null) 0 else max(0, (theme.listIndentPx * theme.listBaseIndentMultiplier).toInt()) + theme.listIndentPx * listDepth + markerGutter
        val quoteInset = if (block.inBlockquote) theme.quoteBorderWidthPx + theme.quoteMarkerGapPx + theme.quoteIndentPx else 0
        val codeInset = if (block.nodeType == "codeBlock") theme.codePaddingHorizontalPx else 0
        val textX = theme.insetLeftPx + listInset + quoteInset + codeInset
        val itemSpacing = if (block.listContext == null) paint.spacingAfterPx else if (block.listItemBoundary?.isFinalRenderableLeaf != false) theme.listItemSpacingPx else 0
        if (block.nodeType == "horizontalRule" || block.nodeType == "horizontal_rule") {
            val ruleTop = cursorY + theme.ruleMarginPx + (marker?.heightPx ?: 0) / 2
            val rule = Rect(theme.insetLeftPx + listInset + quoteInset, ruleTop, theme.insetLeftPx + contentWidth, ruleTop + theme.ruleThicknessPx)
            val fragments = mutableListOf(PreparedProseFragment(PreparedProseFragmentKind.RULE, rule, color = theme.ruleColor, strokeWidth = theme.ruleThicknessPx.toFloat()))
            val end = rule.bottom + theme.ruleMarginPx
            if (block.inBlockquote) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, end), color = theme.quoteBorderColor)
            marker?.let { fragments += markerFragment(it, textX, ruleTop + theme.ruleThicknessPx, markerGutter, theme.listMarkerColor) }
            return finishBlock(fragments, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, end), end, itemSpacing)
        }

        val availableWidth = max(1, contentWidth - listInset - quoteInset - codeInset * 2)
        val attributed = attributed(block.inlines, paint, theme)
        val textTop = cursorY + if (block.nodeType == "codeBlock") theme.codePaddingVerticalPx else 0
        val layout = staticLayout(attributed.text, paint, availableWidth)
        val textHeight = max(1, layout.height)
        val totalEnd = textTop + textHeight + if (block.nodeType == "codeBlock") theme.codePaddingVerticalPx else 0
        val fragments = mutableListOf<PreparedProseFragment>()
        if (block.nodeType == "codeBlock") fragments += PreparedProseFragment(PreparedProseFragmentKind.BACKGROUND, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, totalEnd), color = theme.codeBackground, cornerRadius = theme.codeRadiusPx)
        fragments += PreparedProseFragment(PreparedProseFragmentKind.TEXT, Rect(textX, textTop, textX + availableWidth, textTop + textHeight), layout, textX, textTop)
        fragments += strikeFragments(attributed.text, layout, textX, textTop, paint.color)
        attributed.atoms.forEach { atom ->
            val line = layout.getLineForOffset(atom.start)
            val atomX = textX + layout.getPrimaryHorizontal(atom.start).toInt()
            val baseline = textTop + layout.getLineBaseline(line)
            val atomTop = baseline - atom.heightPx + atom.appearance.paddingVertical
            val bounds = Rect(atomX, atomTop, atomX + atom.widthPx, atomTop + atom.heightPx)
            val labelWidth = max(1, atom.widthPx - atom.appearance.paddingHorizontal * 2)
            val labelLayout = staticLayout(SpannableString(atom.label), atom.appearance.paint, labelWidth)
            fragments += PreparedProseFragment(PreparedProseFragmentKind.ATOM, bounds, labelLayout = labelLayout, labelX = bounds.left + atom.appearance.paddingHorizontal, labelY = bounds.top + atom.appearance.paddingVertical, color = atom.appearance.background, borderColor = atom.appearance.borderColor, cornerRadius = atom.appearance.radius, strokeWidth = atom.appearance.borderWidth, label = atom.label)
        }
        if (block.inBlockquote) fragments += PreparedProseFragment(PreparedProseFragmentKind.BORDER, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + theme.quoteBorderWidthPx, totalEnd), color = theme.quoteBorderColor)
        marker?.let { fragments += markerFragment(it, textX, textTop + layout.getLineBaseline(0), markerGutter, theme.listMarkerColor) }
        return finishBlock(fragments, Rect(theme.insetLeftPx, cursorY, theme.insetLeftPx + contentWidth, totalEnd), totalEnd, itemSpacing, attributed.retainedBytes)
    }

    private fun finishBlock(fragments: List<PreparedProseFragment>, seed: Rect, end: Int, spacing: Int, extraBytes: Long = 0): BlockResult {
        val bounds = fragments.fold(seed) { acc, fragment -> Rect(acc).apply { union(fragment.bounds) } }
        return BlockResult(PreparedProseBlock(fragments.toList(), bounds), max(end, bounds.bottom) + spacing, extraBytes)
    }

    private data class AttributedBlock(val text: SpannableString, val atoms: List<PreparedAtomSpec>, val retainedBytes: Long)

    private fun attributed(inlines: List<ViewerInline>, base: PreparedTextPaint, theme: PreparedProseTheme): AttributedBlock {
        val source = StringBuilder()
        val spans = mutableListOf<(SpannableString) -> Unit>()
        val atoms = mutableListOf<PreparedAtomSpec>()
        inlines.forEach { inline -> when (inline) {
            is ViewerInline.Text -> {
                val start = source.length
                source.append(inline.text)
                val end = source.length
                val markSpans = markSpans(inline.marks, base, theme)
                spans += { value -> markSpans.forEach { value.setSpan(it, start, end, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE) } }
            }
            is ViewerInline.Atom -> {
                if (inline.nodeType == "hardBreak" || inline.nodeType == "hard_break") source.append('\n') else {
                    val appearance = atomAppearance(inline.nodeType, inline.attrsJson, base, theme)
                    val label = inline.label.ifEmpty { " " }
                    val labelPaint = appearance.paint.newTextPaint()
                    val width = max(base.sizePx.toInt(), ceil(labelPaint.measureText(label) + appearance.paddingHorizontal * 2).toInt())
                    val height = max(labelPaint.fontMetricsInt.descent - labelPaint.fontMetricsInt.ascent + appearance.paddingVertical * 2, base.sizePx.toInt())
                    val start = source.length
                    source.append('\uFFFC')
                    atoms += PreparedAtomSpec(start, inline.nodeType, label, appearance, width, height)
                    spans += { value -> value.setSpan(AtomMetricSpan(width, height, appearance.paddingVertical), start, start + 1, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE) }
                }
            }
        } }
        val text = SpannableString(if (source.isEmpty()) "\u200B" else source.toString())
        spans.forEach { it(text) }
        return AttributedBlock(text, atoms, 256L + text.length * 52L + atoms.sumOf { 256L + it.label.length * 2L })
    }

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
        var link = false
        marks.forEach { mark ->
            val attrs = runCatching { org.json.JSONObject(mark.attrsJson) }.getOrNull()
            when (mark.markType) {
                "bold", "strong" -> bold = true
                "italic", "em" -> italic = true
                "underline" -> underline = true
                "strike", "strikethrough" -> strike = true
                "code" -> monospace = true
                "link" -> { link = true; underline = underline || (theme.link?.underline ?: true); background = theme.link?.backgroundColor ?: background }
                "textColor", "color", "foregroundColor" -> explicitColor = parseColor(attrs?.optString("color", null) ?: attrs?.optString("textColor", null)) ?: explicitColor
                "highlight", "backgroundColor" -> background = parseColor(attrs?.optString("color", null) ?: attrs?.optString("backgroundColor", null)) ?: background
                "textStyle", "font" -> { family = attrs?.optString("fontFamily", null) ?: family; size = attrs?.optDouble("fontSize", Double.NaN)?.takeIf { it.isFinite() && it > 0 }?.toFloat() ?: size }
            }
        }
        val result = mutableListOf<Any>()
        val style = when { bold && italic -> Typeface.BOLD_ITALIC; bold -> Typeface.BOLD; italic -> Typeface.ITALIC; else -> Typeface.NORMAL }
        if (style != Typeface.NORMAL) result += StyleSpan(style)
        if (monospace) result += TypefaceSpan("monospace") else family?.takeIf { it.isNotBlank() }?.let { result += TypefaceSpan(it) }
        size?.let { result += AbsoluteSizeSpan(it.toInt().coerceAtLeast(1), true) }
        result += ForegroundColorSpan(explicitColor ?: if (link) theme.link?.color ?: 0xFF007AFF.toInt() else base.color)
        background?.let { result += BackgroundColorSpan(it) }
        if (underline) result += UnderlineSpan()
        if (strike) result += StrikeMarkerSpan()
        return result
    }

    private fun staticLayout(text: CharSequence, paint: PreparedTextPaint, width: Int): StaticLayout {
        staticLayoutsBuilt += 1
        val resolved = paint.newTextPaint()
        val natural = resolved.fontMetricsInt.descent - resolved.fontMetricsInt.ascent
        val extra = max(0, (paint.lineHeightPx ?: natural) - natural).toFloat()
        return StaticLayout.Builder.obtain(text, 0, text.length, resolved, width)
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(false)
            .setLineSpacing(extra, 1f)
            .setBreakStrategy(Layout.BREAK_STRATEGY_HIGH_QUALITY)
            .build()
    }

    private fun strikeFragments(text: SpannableString, layout: StaticLayout, x: Int, y: Int, color: Int): List<PreparedProseFragment> =
        text.getSpans(0, text.length, StrikeMarkerSpan::class.java).flatMap { span ->
            val start = text.getSpanStart(span)
            val end = text.getSpanEnd(span)
            (layout.getLineForOffset(start)..layout.getLineForOffset(max(start, end - 1))).mapNotNull { line ->
                val lineStart = max(start, layout.getLineStart(line))
                val lineEnd = minOf(end, layout.getLineEnd(line))
                val left = x + minOf(layout.getPrimaryHorizontal(lineStart), layout.getPrimaryHorizontal(lineEnd)).toInt()
                val right = x + maxOf(layout.getPrimaryHorizontal(lineStart), layout.getPrimaryHorizontal(lineEnd)).toInt()
                if (right <= left) null else {
                    val baseline = y + layout.getLineBaseline(line)
                    val thickness = max(1, abs(layout.getLineAscent(line)) / 12)
                    PreparedProseFragment(PreparedProseFragmentKind.STRIKE, Rect(left, baseline - abs(layout.getLineAscent(line)) / 3, right, baseline - abs(layout.getLineAscent(line)) / 3 + thickness), color = color, strokeWidth = thickness.toFloat())
                }
            }
        }

    private fun markerFor(context: ViewerListContext, textPaint: PreparedTextPaint, theme: PreparedProseTheme): PreparedMarker {
        val label = when { context.kind == "task" -> ""; context.ordered -> "${context.index}."; else -> "•" }
        val markerPaint = textPaint.copy(sizePx = max(1f, textPaint.sizePx * max(0.01f, theme.listMarkerScale)), color = theme.listMarkerColor)
        if (label.isEmpty()) {
            val side = max(markerPaint.sizePx.toInt(), markerPaint.newTextPaint().fontMetricsInt.descent - markerPaint.newTextPaint().fontMetricsInt.ascent)
            return PreparedMarker(null, label, side, side, context.checked)
        }
        val layout = staticLayout(SpannableString(label), markerPaint, max(1, ceil(markerPaint.newTextPaint().measureText(label)).toInt()))
        return PreparedMarker(layout, label, layout.width, layout.height, context.checked)
    }

    private fun markerFragment(marker: PreparedMarker, textX: Int, baseline: Int, gutter: Int, color: Int): PreparedProseFragment {
        val x = textX - gutter + (gutter - marker.widthPx)
        val y = baseline - marker.heightPx
        return PreparedProseFragment(PreparedProseFragmentKind.MARKER, Rect(x, y, x + marker.widthPx, y + marker.heightPx), marker.layout, x, y, color = color, label = marker.label, checked = marker.checked)
    }

    private fun atomAppearance(nodeType: String, attrsJson: String, base: PreparedTextPaint, theme: PreparedProseTheme): PreparedAtomAppearance {
        if (nodeType == "mention") {
            val values = runCatching { org.json.JSONObject(attrsJson) }.getOrNull()
            val local = EditorMentionTheme.fromJson(values?.optJSONObject("mentionTheme"))
            val mention = theme.mention?.mergedWith(local) ?: local
            val weighted = mention?.fontWeight?.let { EditorTextStyle(fontWeight = it).typefaceStyle() } ?: base.typeface.style
            return PreparedAtomAppearance(base.copy(typeface = Typeface.create(base.typeface, weighted), color = mention?.textColor ?: base.color), mention?.backgroundColor ?: 0x1F007AFF, mention?.borderColor, max(0f, mention?.borderWidth ?: 0f), max(0f, mention?.borderRadius ?: 6f))
        }
        return PreparedAtomAppearance(base, 0xFFF2F2F7.toInt(), null, 0f, 5f)
    }
}

private fun Typeface.familyName(): String? = when (this) { Typeface.MONOSPACE -> "monospace"; Typeface.SERIF -> "serif"; Typeface.SANS_SERIF -> "sans"; else -> null }
private fun parseColor(raw: String?): Int? = runCatching { raw?.takeIf { it.isNotBlank() }?.let(Color::parseColor) }.getOrNull()
