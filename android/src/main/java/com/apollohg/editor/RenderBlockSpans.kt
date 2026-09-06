package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.text.Annotation
import android.text.Layout
import android.text.Spanned
import android.text.style.LeadingMarginSpan
import android.text.style.LineBackgroundSpan
import android.text.style.ReplacementSpan

class BlockquoteSpan(
    private val baseIndentPx: Int,
    private val totalIndentPx: Int,
    private val stripeColor: Int,
    private val stripeWidthPx: Int,
    private val gapWidthPx: Int
    ) : LeadingMarginSpan {

    override fun getLeadingMargin(first: Boolean): Int = totalIndentPx

    override fun drawLeadingMargin(
        canvas: Canvas,
        paint: Paint,
        x: Int,
        dir: Int,
        top: Int,
        baseline: Int,
        bottom: Int,
        text: CharSequence,
        start: Int,
        end: Int,
        first: Boolean,
        layout: android.text.Layout?
    ) {
        if (!lineContainsQuotedContent(text, start, end)) {
            return
        }

        val savedColor = paint.color
        val savedStyle = paint.style

        paint.color = stripeColor
        paint.style = Paint.Style.FILL

        val stripeStart = x + (dir * baseIndentPx)
        val stripeLeft = if (dir > 0) stripeStart.toFloat() else (stripeStart - stripeWidthPx).toFloat()
        val stripeRight = if (dir > 0) stripeLeft + stripeWidthPx else stripeLeft + stripeWidthPx
        val stripeBottom = resolvedStripeBottom(
            text = text,
            start = start,
            end = end,
            baseline = baseline,
            bottom = bottom,
            layout = layout,
            paint = paint
        )
        canvas.drawRect(
            stripeLeft,
            top.toFloat(),
            stripeRight,
            stripeBottom,
            paint
        )

        paint.color = savedColor
        paint.style = savedStyle
    }

    private fun lineContainsQuotedContent(text: CharSequence, start: Int, end: Int): Boolean {
        if (start >= end || text !is Spanned) return true
        for (index in start until end.coerceAtMost(text.length)) {
            val ch = text[index]
            if (ch == '\n' || ch == '\r') continue
            val quoted = text.getSpans(index, index + 1, Annotation::class.java).any {
                it.key == RenderBridge.NATIVE_BLOCKQUOTE_ANNOTATION
            }
            if (quoted) {
                return true
            }
        }
        return false
    }

    internal fun resolvedStripeBottom(
        text: CharSequence,
        start: Int,
        end: Int,
        baseline: Int,
        bottom: Int,
        layout: android.text.Layout?,
        paint: Paint? = null
    ): Float {
        if (layout == null || text.isEmpty()) {
            return bottom.toFloat()
        }
        val lineIndex = safeLineForOffset(layout, start, text.length)
        val nextLine = lineIndex + 1
        if (nextLine >= layout.lineCount) {
            return trimmedTextBottom(baseline, layout, lineIndex, paint)
        }

        val nextLineStart = layout.getLineStart(nextLine)
        val nextLineEnd = layout.getLineEnd(nextLine)
        return if (lineContainsQuotedContent(text, nextLineStart, nextLineEnd)) {
            bottom.toFloat()
        } else {
            trimmedTextBottom(baseline, layout, lineIndex, paint)
        }
    }

    private fun trimmedTextBottom(
        baseline: Int,
        layout: Layout,
        lineIndex: Int,
        paint: Paint?
    ): Float {
        val fontDescent = paint?.fontMetrics?.descent
        return if (fontDescent != null) {
            baseline + fontDescent
        } else {
            (baseline + layout.getLineDescent(lineIndex)).toFloat()
        }
    }

    private fun safeLineForOffset(layout: Layout, offset: Int, textLength: Int): Int {
        if (textLength <= 0) return 0
        val safeStart = offset.coerceIn(0, textLength - 1)
        return layout.getLineForOffset(safeStart)
    }
}

class CodeBlockSpan(
    private val backgroundColor: Int,
    private val cornerRadiusPx: Float,
    private val paddingHorizontalPx: Int,
    private val paddingVerticalPx: Int
) : LeadingMarginSpan, LineBackgroundSpan {
    override fun getLeadingMargin(first: Boolean): Int = paddingHorizontalPx

    override fun drawLeadingMargin(
        canvas: Canvas,
        paint: Paint,
        x: Int,
        dir: Int,
        top: Int,
        baseline: Int,
        bottom: Int,
        text: CharSequence,
        start: Int,
        end: Int,
        first: Boolean,
        layout: Layout
    ) = Unit

    override fun drawBackground(
        canvas: Canvas,
        paint: Paint,
        left: Int,
        right: Int,
        top: Int,
        baseline: Int,
        bottom: Int,
        text: CharSequence,
        start: Int,
        end: Int,
        lineNumber: Int
    ) {
        val spanned = text as? Spanned ?: return
        val spanStart = spanned.getSpanStart(this)
        val spanEnd = spanned.getSpanEnd(this)
        if (spanStart < 0 || start >= spanEnd || end <= spanStart) return

        val isFirstLine = start <= spanStart
        val isLastLine = end >= spanEnd
        val rect = RectF(
            left.toFloat(),
            if (isFirstLine) top.toFloat() - paddingVerticalPx else top.toFloat(),
            (right - paddingHorizontalPx).toFloat(),
            if (isLastLine) bottom.toFloat() + paddingVerticalPx else bottom.toFloat()
        )

        val savedColor = paint.color
        val savedStyle = paint.style
        paint.color = backgroundColor
        paint.style = Paint.Style.FILL

        when {
            isFirstLine && isLastLine -> canvas.drawRoundRect(rect, cornerRadiusPx, cornerRadiusPx, paint)
            isFirstLine -> {
                canvas.drawRoundRect(rect, cornerRadiusPx, cornerRadiusPx, paint)
                canvas.drawRect(rect.left, rect.centerY(), rect.right, rect.bottom, paint)
            }
            isLastLine -> {
                canvas.drawRoundRect(rect, cornerRadiusPx, cornerRadiusPx, paint)
                canvas.drawRect(rect.left, rect.top, rect.right, rect.centerY(), paint)
            }
            else -> canvas.drawRect(rect, paint)
        }

        paint.color = savedColor
        paint.style = savedStyle
    }
}

class HorizontalRuleSpan(
    private val lineColor: Int,
    private val lineHeight: Float = LayoutConstants.HORIZONTAL_RULE_HEIGHT,
    private val verticalPadding: Float = LayoutConstants.HORIZONTAL_RULE_VERTICAL_PADDING
) : ReplacementSpan(), LeadingMarginSpan {

    override fun getLeadingMargin(first: Boolean): Int = 0

    override fun getSize(
        paint: Paint,
        text: CharSequence,
        start: Int,
        end: Int,
        fm: Paint.FontMetricsInt?
    ): Int {
        if (fm != null) {
            val totalHeight = kotlin.math.ceil(lineHeight + (verticalPadding * 2)).toInt()
            val halfHeight = totalHeight / 2
            fm.ascent = -halfHeight
            fm.top = fm.ascent
            fm.descent = totalHeight - halfHeight
            fm.bottom = fm.descent
        }
        // Keep the placeholder atom in the text model without reserving
        // visible glyph width, so Android does not paint a tofu/OBJ box.
        return 0
    }

    override fun drawLeadingMargin(
        canvas: Canvas,
        paint: Paint,
        x: Int,
        dir: Int,
        top: Int,
        baseline: Int,
        bottom: Int,
        text: CharSequence,
        start: Int,
        end: Int,
        first: Boolean,
        layout: android.text.Layout?
    ) {
        val savedColor = paint.color
        val savedStyle = paint.style

        paint.color = lineColor
        paint.style = Paint.Style.FILL

        val lineY = (top + bottom) / 2f
        val lineWidth = layout?.width?.toFloat() ?: canvas.width.toFloat()
        canvas.drawRect(
            x.toFloat(),
            lineY - lineHeight / 2f,
            lineWidth,
            lineY + lineHeight / 2f,
            paint
        )

        paint.color = savedColor
        paint.style = savedStyle
    }

    override fun draw(
        canvas: Canvas,
        text: CharSequence,
        start: Int,
        end: Int,
        x: Float,
        top: Int,
        y: Int,
        bottom: Int,
        paint: Paint
    ) {
        // Intentionally empty: drawLeadingMargin renders the separator line,
        // and ReplacementSpan suppresses drawing the underlying FFFC glyph.
    }
}
