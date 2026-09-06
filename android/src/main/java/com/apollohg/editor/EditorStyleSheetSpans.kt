package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.os.Build
import android.text.Layout
import android.text.Spanned
import android.text.TextPaint
import android.text.style.LeadingMarginSpan
import android.text.style.LineHeightSpan
import android.text.style.MetricAffectingSpan

internal class EditorResolvedTextSpan(val style: EditorTextStyle, private val density: Float) : MetricAffectingSpan() {
    val lineHeightPx: Int? get() = style.lineHeight?.times(density)?.toInt()

    override fun updateMeasureState(paint: TextPaint) {
        style.fontSize?.let { paint.textSize = it * density }
        val family = style.fontFamily?.let { Typeface.create(it, Typeface.NORMAL) } ?: paint.typeface ?: Typeface.DEFAULT
        paint.typeface = if (Build.VERSION.SDK_INT >= 28) {
            Typeface.create(family, style.fontWeight?.toIntOrNull() ?: if (style.fontWeight == "bold") 700 else 400, style.fontStyle == "italic")
        } else Typeface.create(family, style.typefaceStyle())
        style.letterSpacing?.let { paint.letterSpacing = it * density / paint.textSize }
    }

    override fun updateDrawState(paint: TextPaint) {
        updateMeasureState(paint)
        style.color?.let { paint.color = it }
        style.backgroundColor?.let { paint.bgColor = it }
        style.textDecorationLine?.let {
            paint.isUnderlineText = it.contains("underline") && !style.hasCustomDecoration()
            paint.isStrikeThruText = it.contains("line-through") && !style.hasCustomDecoration()
        }
    }
}

internal class EditorBlockBoxSpan(
    val box: EditorBoxStyle,
    val ancestorInset: EditorEdges,
    val depth: Int,
) : LeadingMarginSpan {
    override fun getLeadingMargin(first: Boolean): Int = box.outerInset.left.toInt()
    override fun drawLeadingMargin(canvas: Canvas, paint: Paint, x: Int, dir: Int, top: Int, baseline: Int, bottom: Int, text: CharSequence, start: Int, end: Int, first: Boolean, layout: Layout?) = Unit
    fun chooseHeight(text: CharSequence, start: Int, end: Int, spanstartv: Int, v: Int, fm: Paint.FontMetricsInt) {
        val spanned = text as? Spanned ?: return
        if (start <= spanned.getSpanStart(this)) {
            fm.ascent -= box.outerInset.top.toInt()
            fm.top = fm.ascent
        }
        if (end >= spanned.getSpanEnd(this)) {
            fm.descent += box.outerInset.bottom.toInt()
            fm.bottom = fm.descent
        }
    }

    fun bounds(layout: Layout, text: Spanned): RectF? {
        if (layout is EditorDocumentLayout) return layout.boxBounds(this)
        val start = text.getSpanStart(this)
        val end = text.getSpanEnd(this)
        if (start < 0 || end <= start) return null
        val first = layout.getLineForOffset(start)
        val last = layout.getLineForOffset((end - 1).coerceAtMost(text.length - 1))
        val ancestors = text.getSpans(start, end, EditorBlockBoxSpan::class.java).filter { it.depth < depth && text.getSpanStart(it) <= start && text.getSpanEnd(it) >= end }
        val topInset = ancestors.filter { layout.getLineForOffset(text.getSpanStart(it)) == first }.sumOf { it.box.outerInset.top.toDouble() }.toFloat()
        val bottomInset = ancestors.filter { layout.getLineForOffset(text.getSpanEnd(it) - 1) == last }.sumOf { it.box.outerInset.bottom.toDouble() }.toFloat()
        return RectF(
            ancestorInset.left + box.margin.left,
            layout.getLineTop(first) + topInset + box.margin.top,
            layout.width - ancestorInset.right - box.margin.right,
            layout.getLineBottom(last) - bottomInset - box.margin.bottom,
        )
    }
}

internal class EditorStyledLineMetricsSpan : LineHeightSpan.WithDensity {
    override fun chooseHeight(text: CharSequence, start: Int, end: Int, spanstartv: Int, v: Int, fm: Paint.FontMetricsInt) = Unit

    override fun chooseHeight(text: CharSequence, start: Int, end: Int, spanstartv: Int, v: Int, fm: Paint.FontMetricsInt, paint: TextPaint) {
        val content = text as? Spanned ?: return
        var cursor = start
        var ascent = 0
        var descent = 0
        var targetHeight = 0
        while (cursor < end) {
            val runEnd = content.nextSpanTransition(cursor, end, MetricAffectingSpan::class.java)
            val runPaint = TextPaint(paint)
            val spans = content.getSpans(cursor, runEnd, MetricAffectingSpan::class.java)
            spans.forEach { it.updateMeasureState(runPaint) }
            val metrics = runPaint.fontMetricsInt
            spans.filterIsInstance<android.text.style.ReplacementSpan>().lastOrNull()?.getSize(runPaint, text, cursor, runEnd, metrics)
            ascent = minOf(ascent, metrics.ascent)
            descent = maxOf(descent, metrics.descent)
            spans.filterIsInstance<EditorResolvedTextSpan>().forEach { targetHeight = maxOf(targetHeight, it.lineHeightPx ?: 0) }
            cursor = runEnd
        }
        if (ascent == 0 && descent == 0) return
        val extra = (targetHeight - (descent - ascent)).coerceAtLeast(0)
        fm.ascent = ascent - extra / 2
        fm.descent = descent + extra - extra / 2
        fm.top = fm.ascent
        fm.bottom = fm.descent
        content.getSpans(start, end, EditorBlockBoxSpan::class.java).forEach { it.chooseHeight(text, start, end, spanstartv, v, fm) }
    }
}

internal fun android.text.SpannableStringBuilder.applyStyleSheetLineMetrics() {
    getSpans(0, length, Any::class.java).filter {
        (it is EditorBlockBoxSpan || it is EditorResolvedTextSpan || it is EditorParagraphAlignmentSpan) && getSpanStart(it) == getSpanEnd(it)
    }.forEach { setSpan(it, getSpanStart(it), getSpanEnd(it), Spanned.SPAN_INCLUSIVE_INCLUSIVE) }
    getSpans(0, length, FixedLineHeightSpan::class.java).forEach(::removeSpan)
    setSpan(EditorStyledLineMetricsSpan(), 0, length, if (isEmpty()) Spanned.SPAN_INCLUSIVE_INCLUSIVE else Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
}

internal fun android.text.Spannable.applyPhysicalTextAlignment(alignment: String, start: Int = 0, end: Int = length) {
    setSpan(EditorParagraphAlignmentSpan(alignment), start, end, if (start == end) Spanned.SPAN_MARK_MARK else Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
    var paragraphStart = start
    while (paragraphStart < end) {
        val nextBreak = android.text.TextUtils.indexOf(this, '\n', paragraphStart, end)
        val paragraphEnd = if (nextBreak < 0) end else nextBreak + 1
        val rtl = android.text.TextDirectionHeuristics.FIRSTSTRONG_LTR.isRtl(this, paragraphStart, paragraphEnd - paragraphStart)
        val resolved = when (alignment) {
            "center" -> Layout.Alignment.ALIGN_CENTER
            "left" -> if (rtl) Layout.Alignment.ALIGN_OPPOSITE else Layout.Alignment.ALIGN_NORMAL
            "right" -> if (rtl) Layout.Alignment.ALIGN_NORMAL else Layout.Alignment.ALIGN_OPPOSITE
            else -> Layout.Alignment.ALIGN_NORMAL
        }
        setSpan(android.text.style.AlignmentSpan.Standard(resolved), paragraphStart, paragraphEnd, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        paragraphStart = paragraphEnd
    }
}

internal fun EditorEditText.drawStyleSheetBoxes(canvas: Canvas) {
    val content = text as? Spanned ?: return
    val textLayout = layout ?: return
    if (textLayout is EditorDocumentLayout) return
    val saved = canvas.save()
    canvas.translate(compoundPaddingLeft.toFloat(), extendedPaddingTop.toFloat())
    content.getSpans(0, content.length, EditorBlockBoxSpan::class.java).sortedBy { it.depth }.forEach {
        it.bounds(textLayout, content)?.let { bounds -> EditorBoxDrawing.draw(canvas, bounds, it.box) }
    }
    canvas.restoreToCount(saved)
}
