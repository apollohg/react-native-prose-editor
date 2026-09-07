package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Matrix
import android.graphics.Path
import android.graphics.Rect
import android.graphics.RectF
import android.os.Build
import android.text.Layout
import android.text.NoCopySpan
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.SpannedString
import android.text.StaticLayout
import android.text.TextDirectionHeuristics
import android.text.TextPaint
import android.text.style.AlignmentSpan
import android.text.style.ReplacementSpan
import android.text.style.MetricAffectingSpan
import kotlin.math.ceil

internal class EditorDocumentLayout(
    text: CharSequence,
    paint: TextPaint,
    width: Int,
    private val includeFontPadding: Boolean = false,
    spacingMultiplier: Float = 1f,
    spacingAdd: Float = 0f,
    previous: EditorDocumentLayout? = null,
) : Layout(SpannedString(text), TextPaint(paint), width.coerceAtLeast(1), Alignment.ALIGN_NORMAL, spacingMultiplier, spacingAdd) {
    private data class Paragraph(val start: Int, val end: Int)
    private data class Box(val span: Any, val start: Int, val end: Int, val style: EditorBoxStyle, val depth: Int, val bounds: RectF = RectF())
    private data class SpanKey(val span: Any, val start: Int, val end: Int, val flags: Int, val measured: List<Int>?)
    private data class Key(val text: String, val width: Int, val alignment: Alignment, val justify: Boolean, val spans: List<SpanKey>)
    private data class Fragment(val start: Int, val end: Int, val x: Float, val y: Int, val firstLine: Int, val layout: StaticLayout, val key: Key, val topTrim: Int, val bottomTrim: Int, val afterSpacing: Int)
    private data class Line(val fragment: Fragment, val local: Int)

    private val fragmentSpacingMultiplier = spacingMultiplier
    private val fragmentSpacingAdd = spacingAdd
    private val content = this.text as Spanned
    private val boxes: List<Box>
    private val fragments: List<Fragment>
    private val lines: List<Line>
    private val monotonicLineTops: Boolean
    private val documentHeight: Int
    private val paintKey = listOf(paint.textSize, paint.textScaleX, paint.textSkewX, paint.letterSpacing, paint.flags, paint.typeface, paint.color, paint.bgColor, paint.baselineShift, paint.density, paint.textLocale, paint.fontFeatureSettings)
    internal val reusedFragmentCount: Int

    init {
        val paragraphs = mutableListOf<Paragraph>()
        var start = 0
        for (index in content.indices) {
            if (content[index] == '\n') {
                paragraphs += Paragraph(start, index)
                start = index + 1
            }
        }
        paragraphs += Paragraph(start, content.length)
        fun paragraphAt(offset: Int): Int {
            var low = 0
            var high = paragraphs.lastIndex
            while (low < high) {
                val middle = (low + high + 1) / 2
                if (paragraphs[middle].start <= offset) low = middle else high = middle - 1
            }
            return low
        }
        boxes = content.resolvedBlockSpacing().map { (span, style) ->
            Box(span, content.getSpanStart(span), content.getSpanEnd(span), style, span.depth)
        } + content.getSpans(0, content.length, CodeBlockSpan::class.java).map {
            Box(it, content.getSpanStart(it), content.getSpanEnd(it), it.documentBox, Int.MAX_VALUE)
        }
        val opens = Array(paragraphs.size) { mutableListOf<Box>() }
        val closes = Array(paragraphs.size) { mutableListOf<Box>() }
        val trailingCodeLines = boxes.filter {
            ((it.span as? EditorBlockBoxSpan)?.nodeType == "codeBlock" || it.span is CodeBlockSpan) &&
                it.start >= 0 && it.end > it.start && content[it.end - 1] == '\n'
        }.map { it.end }.toSet()
        for (box in boxes) {
            if (box.start < 0 || box.end < box.start) continue
            opens[paragraphAt(box.start)] += box
            val lastOffset = if (box.end in trailingCodeLines) box.end else maxOf(box.start, box.end - 1)
            closes[paragraphAt(lastOffset)] += box
        }
        val reusable = if (previous != null && previous.paintKey == paintKey && previous.includeFontPadding == includeFontPadding && previous.fragmentSpacingMultiplier == spacingMultiplier && previous.fragmentSpacingAdd == spacingAdd) {
            previous.fragments.groupBy { it.key }.mapValues { it.value.toMutableList() }.toMutableMap()
        } else mutableMapOf()
        val built = mutableListOf<Fragment>()
        val builtLines = mutableListOf<Line>()
        var left = 0f
        var right = 0f
        var y = 0
        var reused = 0
        for ((index, paragraph) in paragraphs.withIndex()) {
            opens[index].sortBy { it.depth }
            for (box in opens[index]) {
                box.bounds.left = left + box.style.margin.left
                box.bounds.right = this.width - right - box.style.margin.right
                box.bounds.top = y + box.style.margin.top
                left += box.style.outerInset.left
                right += box.style.outerInset.right
                y += ceil(box.style.outerInset.top).toInt()
            }
            val local = SpannableStringBuilder(content, paragraph.start, paragraph.end)
            if (local.isEmpty() && paragraph.start in trailingCodeLines) {
                content.getSpans(paragraph.start - 1, paragraph.start, MetricAffectingSpan::class.java)
                    .filter { content.getSpanStart(it) < paragraph.start && content.getSpanEnd(it) >= paragraph.start }
                    .forEach { local.setSpan(it, 0, 0, Spanned.SPAN_MARK_MARK) }
            }
            local.getSpans(0, local.length, Any::class.java).filter {
                it is EditorBlockBoxSpan || it is CodeBlockSpan || it is NoCopySpan
            }.forEach(local::removeSpan)
            local.setSpan(EditorOwnedBlockGeometrySpan, 0, local.length, Spanned.SPAN_INCLUSIVE_INCLUSIVE)
            val requestedAlignment = local.getSpans(0, local.length, EditorParagraphAlignmentSpan::class.java).lastOrNull()?.value
            val rtl = TextDirectionHeuristics.FIRSTSTRONG_LTR.isRtl(local, 0, local.length)
            val alignment = when (requestedAlignment) {
                "left" -> if (rtl) Alignment.ALIGN_OPPOSITE else Alignment.ALIGN_NORMAL
                "right" -> if (rtl) Alignment.ALIGN_NORMAL else Alignment.ALIGN_OPPOSITE
                "center" -> Alignment.ALIGN_CENTER
                else -> Alignment.ALIGN_NORMAL
            }
            if (requestedAlignment != null) local.getSpans(0, local.length, AlignmentSpan::class.java).forEach(local::removeSpan)
            val available = (this.width - ceil(left).toInt() - ceil(right).toInt()).coerceAtLeast(1)
            val spanKeys = local.getSpans(0, local.length, Any::class.java).map { span ->
                val spanStart = local.getSpanStart(span)
                val spanEnd = local.getSpanEnd(span)
                val measured = if (span is ReplacementSpan) {
                    val fm = Paint.FontMetricsInt()
                    val size = span.getSize(TextPaint(this.paint), local, spanStart, spanEnd, fm)
                    listOf(size, fm.ascent, fm.descent, fm.top, fm.bottom)
                } else null
                SpanKey(span, spanStart, spanEnd, local.getSpanFlags(span), measured)
            }
            val key = Key(local.toString(), available, alignment, requestedAlignment == "justify", spanKeys)
            val cached = reusable[key]?.removeFirstOrNull()
            val fragmentPaint = TextPaint(this.paint)
            val emptyStyles = if (local.isEmpty()) local.getSpans(0, 0, EditorResolvedTextSpan::class.java).toList() else emptyList()
            if (local.isEmpty()) local.getSpans(0, 0, MetricAffectingSpan::class.java).forEach { it.updateMeasureState(fragmentPaint) }
            val shaped = cached?.layout ?: StaticLayout.Builder.obtain(SpannedString(local), 0, local.length, fragmentPaint, available)
                .setIncludePad(includeFontPadding)
                .setAlignment(alignment)
                .setTextDirection(TextDirectionHeuristics.FIRSTSTRONG_LTR)
                .setLineSpacing(spacingAdd, spacingMultiplier)
                .apply { if (Build.VERSION.SDK_INT >= 26) setJustificationMode(if (key.justify) JUSTIFICATION_MODE_INTER_WORD else JUSTIFICATION_MODE_NONE) }
                .build()
            if (cached != null) reused++
            val emptyExtra = ((emptyStyles.maxOfOrNull { it.lineHeightPx ?: 0 } ?: 0) - shaped.height).coerceAtLeast(0)
            val topTrim = (if (includeFontPadding && index > 0) -shaped.topPadding else 0) - emptyExtra / 2
            val bottomTrim = (if (includeFontPadding && index < paragraphs.lastIndex) shaped.bottomPadding else 0) - (emptyExtra - emptyExtra / 2)
            val lastLine = shaped.lineCount - 1
            val naturalHeight = shaped.getLineDescent(lastLine) - bottomTrim - shaped.getLineAscent(lastLine) - if (lastLine == 0) topTrim else 0
            val afterSpacing = if (index < paragraphs.lastIndex) kotlin.math.round(naturalHeight * (spacingMultiplier - 1) + spacingAdd).toInt() else 0
            val fragment = Fragment(paragraph.start, paragraph.end, ceil(left), y - topTrim, builtLines.size, shaped, key, topTrim, bottomTrim, afterSpacing)
            built += fragment
            repeat(shaped.lineCount) { builtLines += Line(fragment, it) }
            y += shaped.height - topTrim - bottomTrim + afterSpacing
            closes[index].sortByDescending { it.depth }
            for (box in closes[index]) {
                box.bounds.bottom = y + box.style.inset.bottom
                y += ceil(box.style.outerInset.bottom).toInt()
                left -= box.style.outerInset.left
                right -= box.style.outerInset.right
            }
            if (paragraph.end < content.length) {
                y += content.getSpans(paragraph.end, paragraph.end + 1, ParagraphSpacerSpan::class.java).maxOfOrNull { it.spacingPx } ?: 0
            }
        }
        fragments = built
        lines = builtLines
        monotonicLineTops = (1 until lines.size).all { textLineTop(it - 1) <= textLineTop(it) }
        documentHeight = y
        reusedFragmentCount = reused
    }

    internal fun emptyLinePaint(offset: Int): TextPaint? = lines[getLineForOffset(offset)].fragment.let {
        if (it.start == it.end) it.layout.paint else null
    }

    internal fun contentLeft(line: Int): Float = lines[line].let { it.fragment.x + it.fragment.layout.getParagraphLeft(it.local) }
    internal fun contentRight(line: Int): Float = lines[line].let { it.fragment.x + it.fragment.layout.getParagraphRight(it.local) }
    internal fun textLineTop(line: Int): Int = lines[line].let { it.fragment.y + it.fragment.layout.getLineTop(it.local) + if (it.local == 0) it.fragment.topTrim else 0 }
    internal fun textLineBottom(line: Int): Int = lines[line].let { it.fragment.y + it.fragment.layout.getLineBottom(it.local) + if (it.local == it.fragment.layout.lineCount - 1) it.fragment.afterSpacing - it.fragment.bottomTrim else 0 }
    internal fun boxBounds(span: EditorBlockBoxSpan): RectF? = boxes.firstOrNull { it.span === span }?.bounds?.let(::RectF)
    internal fun imageBounds(span: BlockImageSpan): RectF? {
        val start = content.getSpanStart(span)
        val end = content.getSpanEnd(span)
        if (start < 0 || end <= start) return null
        return span.boxRect(minOf(getPrimaryHorizontal(start), getPrimaryHorizontal(end)), getLineBaseline(getLineForOffset(start)).toFloat())
    }
    override fun getHeight(): Int = documentHeight
    override fun getLineCount(): Int = lines.size
    override fun getLineTop(line: Int): Int = if (line == lines.size) documentHeight else textLineTop(line)
    override fun getLineDescent(line: Int): Int = lines[line].let { getLineTop(line + 1) - it.fragment.y - it.fragment.layout.getLineBaseline(it.local) }
    override fun getLineStart(line: Int): Int = if (line == lines.size) content.length else lines[line].let { it.fragment.start + it.fragment.layout.getLineStart(it.local) }
    override fun getParagraphDirection(line: Int): Int = lines[line].let { it.fragment.layout.getParagraphDirection(it.local) }
    override fun getLineContainsTab(line: Int): Boolean = lines[line].let { it.fragment.layout.getLineContainsTab(it.local) }
    override fun getLineDirections(line: Int): Directions = lines[line].let { it.fragment.layout.getLineDirections(it.local) }
    override fun getTopPadding(): Int = fragments.first().layout.topPadding
    override fun getBottomPadding(): Int = fragments.last().layout.bottomPadding
    override fun getEllipsisStart(line: Int): Int = 0
    override fun getEllipsisCount(line: Int): Int = 0
    override fun getLineLeft(line: Int): Float = lines[line].let { it.fragment.x + it.fragment.layout.getLineLeft(it.local) }
    override fun getLineRight(line: Int): Float = lines[line].let { it.fragment.x + it.fragment.layout.getLineRight(it.local) }
    override fun getLineMax(line: Int): Float = lines[line].let { it.fragment.layout.getLineMax(it.local) }
    override fun getLineWidth(line: Int): Float = lines[line].let { it.fragment.layout.getLineWidth(it.local) }
    override fun getLineVisibleEnd(line: Int): Int = lines[line].let { it.fragment.start + it.fragment.layout.getLineVisibleEnd(it.local) }
    override fun getLineBottom(line: Int, includeLineSpacing: Boolean): Int = textLineBottom(line)

    override fun getLineBounds(line: Int, bounds: Rect): Int {
        bounds.set(contentLeft(line).toInt(), textLineTop(line), contentRight(line).toInt(), textLineBottom(line))
        return getLineBaseline(line)
    }

    override fun getLineForVertical(vertical: Int): Int {
        if (monotonicLineTops) return findLine { textLineTop(it) <= vertical }
        val paintedLines = lines.indices.reversed()
        return paintedLines.firstOrNull { vertical >= textLineTop(it) && vertical < textLineBottom(it) }
            ?: paintedLines.minBy { maxOf(textLineTop(it) - vertical, vertical - textLineBottom(it), 0) }
    }
    override fun getLineForOffset(offset: Int): Int = findLine { getLineStart(it) <= offset.coerceIn(0, content.length) }
    private inline fun findLine(before: (Int) -> Boolean): Int {
        var low = 0
        var high = lines.lastIndex
        while (low < high) {
            val middle = (low + high + 1) / 2
            if (before(middle)) low = middle else high = middle - 1
        }
        return low
    }

    override fun getPrimaryHorizontal(offset: Int): Float = horizontal(offset, false)
    override fun getSecondaryHorizontal(offset: Int): Float = horizontal(offset, true)
    private fun horizontal(offset: Int, secondary: Boolean): Float {
        val fragment = lines[getLineForOffset(offset)].fragment
        val local = (offset - fragment.start).coerceIn(0, fragment.end - fragment.start)
        return fragment.x + if (secondary) fragment.layout.getSecondaryHorizontal(local) else fragment.layout.getPrimaryHorizontal(local)
    }
    override fun getOffsetForHorizontal(line: Int, horiz: Float): Int = lines[line].let {
        it.fragment.start + it.fragment.layout.getOffsetForHorizontal(it.local, horiz - it.fragment.x)
    }
    override fun isRtlCharAt(offset: Int): Boolean = lines[getLineForOffset(offset)].fragment.let {
        it.layout.isRtlCharAt((offset - it.start).coerceIn(0, it.end - it.start))
    }
    override fun getOffsetToLeftOf(offset: Int): Int = adjacentOffset(offset, true)
    override fun getOffsetToRightOf(offset: Int): Int = adjacentOffset(offset, false)
    private fun adjacentOffset(offset: Int, left: Boolean): Int {
        val line = getLineForOffset(offset)
        val fragment = lines[line].fragment
        val local = (offset - fragment.start).coerceIn(0, fragment.end - fragment.start)
        val next = if (left) fragment.layout.getOffsetToLeftOf(local) else fragment.layout.getOffsetToRightOf(local)
        if (next != local) return fragment.start + next
        val backward = left == (getParagraphDirection(line) > 0)
        return (offset + if (backward) -1 else 1).coerceIn(0, content.length)
    }

    override fun getCursorPath(point: Int, dest: Path, editingBuffer: CharSequence) {
        dest.reset()
        val line = getLineForOffset(point)
        val x = getPrimaryHorizontal(point)
        dest.moveTo(x, textLineTop(line).toFloat())
        dest.lineTo(x, textLineBottom(line).toFloat())
    }

    override fun getSelectionPath(start: Int, end: Int, dest: Path) {
        dest.reset()
        val from = minOf(start, end).coerceIn(0, content.length)
        val to = maxOf(start, end).coerceIn(from, content.length)
        if (from == to) return
        for (fragment in fragments) {
            if (from > fragment.end || to <= fragment.start) continue
            val localFrom = (from - fragment.start).coerceAtLeast(0)
            val localTo = (to - fragment.start).coerceAtMost(fragment.end - fragment.start)
            for (localLine in fragment.layout.getLineForOffset(localFrom)..fragment.layout.getLineForOffset(localTo)) {
                val lineFrom = maxOf(localFrom, fragment.layout.getLineStart(localLine))
                val lineTo = minOf(localTo, fragment.layout.getLineEnd(localLine))
                if (lineFrom >= lineTo) continue
                val localPath = Path()
                fragment.layout.getSelectionPath(lineFrom, lineTo, localPath)
                val line = fragment.firstLine + localLine
                val matrix = Matrix()
                matrix.setRectToRect(
                    RectF(0f, fragment.layout.getLineTop(localLine).toFloat(), 1f, fragment.layout.getLineBottom(localLine).toFloat()),
                    RectF(fragment.x, textLineTop(line).toFloat(), fragment.x + 1, textLineBottom(line).toFloat()),
                    Matrix.ScaleToFit.FILL,
                )
                localPath.transform(matrix)
                dest.addPath(localPath)
            }
            if (to > fragment.end && fragment.end < content.length) {
                val line = fragment.firstLine + fragment.layout.lineCount - 1
                val x = getPrimaryHorizontal(fragment.end)
                val edge = if (getParagraphDirection(line) < 0) contentLeft(line) else contentRight(line)
                dest.addRect(minOf(x, edge), textLineTop(line).toFloat(), maxOf(x + 1, edge), textLineBottom(line).toFloat(), Path.Direction.CW)
            }
        }
    }

    override fun draw(canvas: Canvas) = draw(canvas, null, null, 0)
    override fun draw(canvas: Canvas, highlight: Path?, highlightPaint: Paint?, cursorOffsetVertical: Int) {
        boxes.sortedBy { it.depth }.forEach { EditorBoxDrawing.draw(canvas, it.bounds, it.style) }
        for (fragment in fragments) {
            val saved = canvas.save()
            canvas.translate(fragment.x, fragment.y.toFloat())
            val localHighlight = highlight?.let {
                Path(it).apply {
                    offset(-fragment.x, -fragment.y.toFloat())
                    val bounds = RectF()
                    computeBounds(bounds, true)
                    bounds.top = fragment.topTrim.toFloat()
                    bounds.bottom = (fragment.layout.height - fragment.bottomTrim + fragment.afterSpacing).toFloat()
                    op(Path().apply { addRect(bounds, Path.Direction.CW) }, Path.Op.INTERSECT)
                }
            }
            if (Build.VERSION.SDK_INT >= 34) {
                // Android 14+ skips selection drawing when both highlight lists are null.
                fragment.layout.draw(canvas, emptyList(), emptyList(), localHighlight, highlightPaint, cursorOffsetVertical)
            } else {
                fragment.layout.draw(canvas, localHighlight, highlightPaint, cursorOffsetVertical)
            }
            canvas.restoreToCount(saved)
        }
    }
}

internal object EditorOwnedBlockGeometrySpan
internal class EditorParagraphAlignmentSpan(val value: String) : android.text.style.ParagraphStyle

internal fun Layout.editorTextLineTop(line: Int): Int = (this as? EditorDocumentLayout)?.textLineTop(line) ?: getLineTop(line)
internal fun Layout.editorTextLineBottom(line: Int): Int = (this as? EditorDocumentLayout)?.textLineBottom(line) ?: getLineBottom(line)
