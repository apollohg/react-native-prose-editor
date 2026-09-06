package com.apollohg.editor
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.text.Annotation
import android.text.Layout
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.util.Base64
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import kotlin.math.abs
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LeadingMarginSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
import android.text.style.URLSpan
import android.text.style.UnderlineSpan
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RenderBridgeInlineMarksTest : RenderBridgeTestFixture() {
    @Test
    fun `render - bold text`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "bold text", "marks": ["bold"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("bold text", result.toString())

        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        assertTrue("Should have a StyleSpan", styleSpans.isNotEmpty())

        val boldSpan = styleSpans.find { it.style == Typeface.BOLD }
        assertNotNull(
            "Should have a BOLD StyleSpan. Styles found: ${styleSpans.map { it.style }}",
            boldSpan
        )
    }

    @Test
    fun `render - italic text`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "italic text", "marks": ["italic"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("italic text", result.toString())

        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        val italicSpan = styleSpans.find { it.style == Typeface.ITALIC }
        assertNotNull(
            "Should have an ITALIC StyleSpan. Styles found: ${styleSpans.map { it.style }}",
            italicSpan
        )
    }

    @Test
    fun `render - bold italic`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "bold italic", "marks": ["bold", "italic"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        val boldItalicSpan = styleSpans.find { it.style == Typeface.BOLD_ITALIC }
        assertNotNull(
            "Should have a BOLD_ITALIC StyleSpan. Styles found: ${styleSpans.map { it.style }}",
            boldItalicSpan
        )
    }

    @Test
    fun `render - underline`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "underlined", "marks": ["underline"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("underlined", result.toString())

        val underlineSpans = result.getSpans(0, result.length, UnderlineSpan::class.java)
        assertTrue(
            "Should have an UnderlineSpan",
            underlineSpans.isNotEmpty()
        )
    }

    @Test
    fun `render - strikethrough`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "struck", "marks": ["strike"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("struck", result.toString())

        val strikeSpans = result.getSpans(0, result.length, StrikethroughSpan::class.java)
        assertTrue(
            "Should have a StrikethroughSpan",
            strikeSpans.isNotEmpty()
        )
    }

    @Test
    fun `render - mixed marks in paragraph`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "normal ", "marks": []},
            {"type": "textRun", "text": "bold", "marks": ["bold"]},
            {"type": "textRun", "text": " end", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("normal bold end", result.toString())

        // Check "normal " (offset 0-7) has no bold StyleSpan.
        val normalStyleSpans = result.getSpans(0, 7, StyleSpan::class.java)
        val normalBold = normalStyleSpans.find { it.style == Typeface.BOLD }
        // The bold span should NOT cover the "normal " range.
        if (normalBold != null) {
            val spanStart = result.getSpanStart(normalBold)
            assertTrue(
                "'normal' range should not overlap with bold span (span starts at $spanStart)",
                spanStart >= 7
            )
        }

        // Check "bold" (offset 7-11) has bold StyleSpan.
        val boldStyleSpans = result.getSpans(7, 11, StyleSpan::class.java)
        val boldSpan = boldStyleSpans.find { it.style == Typeface.BOLD }
        assertNotNull("'bold' should have BOLD StyleSpan", boldSpan)
    }

    @Test
    fun `render - strong alias for bold`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "strong", "marks": ["strong"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        val boldSpan = styleSpans.find { it.style == Typeface.BOLD }
        assertNotNull("'strong' should produce BOLD StyleSpan", boldSpan)
    }

    @Test
    fun `render - em alias for italic`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "emphasis", "marks": ["em"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        val italicSpan = styleSpans.find { it.style == Typeface.ITALIC }
        assertNotNull("'em' should produce ITALIC StyleSpan", italicSpan)
    }

    @Test
    fun `render - strikethrough alias for strike`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "deleted", "marks": ["strikethrough"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val strikeSpans = result.getSpans(0, result.length, StrikethroughSpan::class.java)
        assertTrue("'strikethrough' should produce StrikethroughSpan", strikeSpans.isNotEmpty())
    }

    @Test
    fun `render - all marks combined`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "everything", "marks": ["bold", "italic", "underline", "strike"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        val boldItalicSpan = styleSpans.find { it.style == Typeface.BOLD_ITALIC }
        assertNotNull("Should have BOLD_ITALIC", boldItalicSpan)

        val underlineSpans = result.getSpans(0, result.length, UnderlineSpan::class.java)
        assertTrue("Should have underline", underlineSpans.isNotEmpty())

        val strikeSpans = result.getSpans(0, result.length, StrikethroughSpan::class.java)
        assertTrue("Should have strikethrough", strikeSpans.isNotEmpty())
    }

    @Test
    fun `render - link mark`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "click here", "marks": [{"type":"link","href":"https://example.com"}]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        assertEquals("click here", result.toString())
        val underlineSpans = result.getSpans(0, result.length, UnderlineSpan::class.java)
        val colorSpans = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
        val urlSpans = result.getSpans(0, result.length, URLSpan::class.java)
        val hrefAnnotations = result.getSpans(0, result.length, Annotation::class.java)
            .filter { it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION }

        assertTrue("Link text should be underlined", underlineSpans.isNotEmpty())
        assertTrue(
            "Link text should use link color",
            colorSpans.any { it.foregroundColor == Color.parseColor("#1B73E8") }
        )
        assertTrue("Editor render should not expose clickable URL spans", urlSpans.isEmpty())
        assertEquals(1, hrefAnnotations.size)
        assertEquals("https://example.com", hrefAnnotations.first().value)
    }

    @Test
    fun `render - themed link mark`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "click here", "marks": [{"type":"link","href":"https://example.com"}]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "links": {
                "color": "#445566",
                "backgroundColor": "#eef6ff",
                "fontSize": 18,
                "fontWeight": "700",
                "fontStyle": "italic",
                "underline": false
              }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)
        assertEquals("click here", result.toString())
        val underlineSpans = result.getSpans(0, result.length, UnderlineSpan::class.java)
        val colorSpans = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
        val backgroundSpans = result.getSpans(0, result.length, BackgroundColorSpan::class.java)
        val sizeSpans = result.getSpans(0, result.length, AbsoluteSizeSpan::class.java)
        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)
        val hrefAnnotations = result.getSpans(0, result.length, Annotation::class.java)
            .filter { it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION }

        assertTrue("Link underline should be disabled by theme", underlineSpans.isEmpty())
        assertTrue(colorSpans.any { it.foregroundColor == Color.parseColor("#445566") })
        assertTrue(backgroundSpans.any { it.backgroundColor == Color.parseColor("#eef6ff") })
        assertTrue(sizeSpans.any { it.size == 18 })
        assertTrue(styleSpans.any { it.style == Typeface.BOLD_ITALIC })
        assertEquals(1, hrefAnnotations.size)
        assertEquals("https://example.com", hrefAnnotations.first().value)
    }
}
