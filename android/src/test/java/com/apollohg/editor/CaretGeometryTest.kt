package com.apollohg.editor

import android.graphics.Color
import android.graphics.Typeface
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.StaticLayout
import android.text.TextPaint
import android.text.style.AbsoluteSizeSpan
import android.text.style.StyleSpan
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Unit tests for [CaretGeometry] — the caret must clip to the rendered glyph
 * height and never extend into the paragraph gap that [ParagraphSpacerSpan]
 * adds below the line via inflated descent.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class CaretGeometryTest {

    private fun layoutFor(text: CharSequence, paint: TextPaint): StaticLayout =
        StaticLayout.Builder.obtain(text, 0, text.length, paint, WIDTH_PX).build()

    @Test
    fun `caret bottom clips paragraph spacer descent to the glyph height`() {
        val text = SpannableStringBuilder("Hello\nWorld")
        // Inter-block spacer inflates the descent of line 0 (the "Hello\n" line),
        // exactly as RenderBridge applies it between blocks.
        text.setSpan(
            ParagraphSpacerSpan(spacingPx = SPACER_PX, baseFontSize = FONT_SIZE_PX, textColor = Color.BLACK),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        val paint = TextPaint().apply { textSize = FONT_SIZE_PX.toFloat() }
        val layout = layoutFor(text, paint)
        val line = 0

        // Reproduction guard: the spacer really did inflate the layout descent.
        assertTrue(
            "ParagraphSpacerSpan should inflate layout descent in this reproduction",
            layout.getLineDescent(line) > paint.fontMetrics.descent
        )

        val bounds = CaretGeometry.verticalBounds(layout, offset = 5, paint = paint)

        assertEquals(
            "caret top should be the line top",
            layout.getLineTop(line).toFloat(),
            bounds.top,
            TOLERANCE_PX
        )
        assertEquals(
            "caret bottom should be baseline + raw font descent, not the inflated getLineBottom",
            layout.getLineBaseline(line) + paint.fontMetrics.descent,
            bounds.bottom,
            TOLERANCE_PX
        )
        assertTrue(
            "caret must not extend into the paragraph gap below the line",
            bounds.bottom < layout.getLineBottom(line)
        )
    }

    @Test
    fun `caret height is identical with and without the paragraph spacer`() {
        val paint = TextPaint().apply { textSize = FONT_SIZE_PX.toFloat() }

        val plain = layoutFor(SpannableStringBuilder("Hello\nWorld"), paint)

        val spaced = SpannableStringBuilder("Hello\nWorld")
        spaced.setSpan(
            ParagraphSpacerSpan(spacingPx = SPACER_PX, baseFontSize = FONT_SIZE_PX, textColor = Color.BLACK),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        val spacedLayout = layoutFor(spaced, paint)

        // Sanity: the spacer changed the layout (line 1 sits lower).
        assertTrue(
            "spacer should push the following line down",
            spacedLayout.getLineTop(1) > plain.getLineTop(1)
        )

        val plainBounds = CaretGeometry.verticalBounds(plain, offset = 5, paint = paint)
        val spacedBounds = CaretGeometry.verticalBounds(spacedLayout, offset = 5, paint = paint)

        assertEquals(
            "caret top must not depend on the paragraph spacer",
            plainBounds.top,
            spacedBounds.top,
            TOLERANCE_PX
        )
        assertEquals(
            "caret height must be the rendered glyph height regardless of paragraph spacing",
            plainBounds.bottom,
            spacedBounds.bottom,
            TOLERANCE_PX
        )
    }

    @Test
    fun `caret uses active metric spans without inheriting paragraph spacer descent`() {
        val text = SpannableStringBuilder("Large\nNext")
        text.setSpan(AbsoluteSizeSpan(32), 0, 5, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        text.setSpan(StyleSpan(Typeface.BOLD), 0, 5, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        text.setSpan(
            ParagraphSpacerSpan(
                spacingPx = SPACER_PX,
                baseFontSize = FONT_SIZE_PX,
                textColor = Color.BLACK,
            ),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE,
        )
        val fallbackPaint = TextPaint().apply { textSize = FONT_SIZE_PX.toFloat() }
        val layout = layoutFor(text, fallbackPaint)
        val expectedPaint = TextPaint(fallbackPaint).apply {
            textSize = 32f
            typeface = Typeface.create(typeface, Typeface.BOLD)
        }

        val bounds = CaretGeometry.verticalBounds(
            layout,
            offset = 5,
            fallbackPaint = fallbackPaint,
            text = text,
        )

        assertEquals(
            layout.getLineBaseline(0) + expectedPaint.fontMetrics.descent,
            bounds.bottom,
            TOLERANCE_PX,
        )
        assertTrue(bounds.bottom < layout.getLineBottom(0))
    }

    @Test
    fun `caret at a metric run boundary keeps the preceding run metrics`() {
        val text = SpannableStringBuilder("Bigsmall")
        text.setSpan(AbsoluteSizeSpan(32), 0, 3, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        text.setSpan(AbsoluteSizeSpan(12), 3, text.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val fallbackPaint = TextPaint().apply { textSize = FONT_SIZE_PX.toFloat() }
        val layout = layoutFor(text, fallbackPaint)
        val precedingPaint = TextPaint(fallbackPaint).apply { textSize = 32f }

        val bounds = CaretGeometry.verticalBounds(
            layout,
            offset = 3,
            fallbackPaint = fallbackPaint,
            text = text,
        )

        assertEquals(
            layout.getLineBaseline(0) + precedingPaint.fontMetrics.descent,
            bounds.bottom,
            TOLERANCE_PX,
        )
    }

    @Test
    fun `caret at a line start uses the following run metrics`() {
        val text = SpannableStringBuilder("First\nLarge")
        text.setSpan(AbsoluteSizeSpan(32), 6, text.length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val fallbackPaint = TextPaint().apply { textSize = FONT_SIZE_PX.toFloat() }
        val layout = layoutFor(text, fallbackPaint)
        val followingPaint = TextPaint(fallbackPaint).apply { textSize = 32f }

        val bounds = CaretGeometry.verticalBounds(
            layout,
            offset = 6,
            fallbackPaint = fallbackPaint,
            text = text,
        )

        assertEquals(
            layout.getLineBaseline(1) + followingPaint.fontMetrics.descent,
            bounds.bottom,
            TOLERANCE_PX,
        )
    }

    @Test
    fun `caret renders only when focused, window-focused, and selection collapsed`() {
        assertTrue(
            "collapsed caret in a focused field should render",
            CaretGeometry.shouldRender(focused = true, windowFocused = true, selectionStart = 3, selectionEnd = 3)
        )
        assertFalse(
            "no caret when the field is not focused",
            CaretGeometry.shouldRender(focused = false, windowFocused = true, selectionStart = 3, selectionEnd = 3)
        )
        assertFalse(
            "no caret when the window is not focused",
            CaretGeometry.shouldRender(focused = true, windowFocused = false, selectionStart = 3, selectionEnd = 3)
        )
        assertFalse(
            "no caret for a range selection (highlight shown instead)",
            CaretGeometry.shouldRender(focused = true, windowFocused = true, selectionStart = 3, selectionEnd = 5)
        )
        assertFalse(
            "no caret when there is no selection",
            CaretGeometry.shouldRender(focused = true, windowFocused = true, selectionStart = -1, selectionEnd = -1)
        )
    }

    private companion object {
        const val WIDTH_PX = 200
        const val FONT_SIZE_PX = 16
        const val SPACER_PX = 40
        const val TOLERANCE_PX = 0.01f
    }
}
