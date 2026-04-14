package com.apollohg.editor

import android.graphics.Bitmap
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
import org.junit.Assert.assertEquals
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

/**
 * Unit tests for [RenderBridge] — conversion of RenderElement JSON into
 * [SpannableStringBuilder] with appropriate spans.
 *
 * Uses Robolectric to provide Android framework classes (SpannableStringBuilder,
 * span types, etc.) in a JVM test environment.
 *
 * NOTE: Robolectric must be in the test dependencies for these to run:
 * ```gradle
 * testImplementation("org.robolectric:robolectric:4.11.1")
 * ```
 *
 * These tests mirror the iOS RenderBridgeTests.swift test suite.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class RenderBridgeTest {

    // ── Test Fixtures ───────────────────────────────────────────────────

    private val baseFontSize = 16f
    private val textColor = Color.BLACK

    // ── Plain Text Rendering ────────────────────────────────────────────

    /** A single paragraph with unstyled text should produce the text content. */
    @Test
    fun `render - plain paragraph`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello, world!", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals(
            "Plain paragraph should render as the text content",
            "Hello, world!", result.toString()
        )

        // Verify foreground color span is present.
        val colorSpans = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
        assertTrue(
            "Should have at least one ForegroundColorSpan",
            colorSpans.isNotEmpty()
        )
    }

    // ── Bold Text Rendering ─────────────────────────────────────────────

    /** Bold mark should produce a StyleSpan with Typeface.BOLD. */
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

    // ── Italic Text Rendering ───────────────────────────────────────────

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

    // ── Bold + Italic Combined ──────────────────────────────────────────

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

    // ── Underline ───────────────────────────────────────────────────────

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

    // ── Strikethrough ───────────────────────────────────────────────────

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

    // ── Code Mark (Monospace) ───────────────────────────────────────────

    @Test
    fun `render - code inline`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "code", "marks": ["code"]},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("code", result.toString())

        val typefaceSpans = result.getSpans(0, result.length, TypefaceSpan::class.java)
        val monoSpan = typefaceSpans.find { it.family == "monospace" }
        assertNotNull(
            "Code mark should produce monospace TypefaceSpan. " +
                    "Families found: ${typefaceSpans.map { it.family }}",
            monoSpan
        )

        val bgSpans = result.getSpans(0, result.length, BackgroundColorSpan::class.java)
        assertTrue(
            "Code mark should have a background color span",
            bgSpans.isNotEmpty()
        )
    }

    // ── Hard Break (Void Inline) ────────────────────────────────────────

    /** A hardBreak void inline should render as a newline character. */
    @Test
    fun `render - hard break`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Line 1", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 7},
            {"type": "textRun", "text": "Line 2", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals(
            "Hard break should render as newline. Got: '${result}'",
            "Line 1\nLine 2", result.toString()
        )
    }

    // ── Horizontal Rule (Void Block) ────────────────────────────────────

    /** A horizontalRule should render as FFFC with a HorizontalRuleSpan. */
    @Test
    fun `render - horizontal rule`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "voidBlock", "nodeType": "horizontalRule", "docPos": 7},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Below", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        val string = result.toString()
        assertTrue(
            "Horizontal rule should contain object replacement character. Got: '$string'",
            string.contains("\uFFFC")
        )

        val hrSpans = result.getSpans(0, result.length, HorizontalRuleSpan::class.java)
        assertTrue(
            "Should have a HorizontalRuleSpan",
            hrSpans.isNotEmpty()
        )

        val replacementMetrics = Paint.FontMetricsInt()
        val hrOffset = string.indexOf('\uFFFC')
        val hrSpan = hrSpans.single()
        assertEquals(
            "Horizontal rule should not reserve glyph width for the replacement character",
            0,
            hrSpan.getSize(TextPaint().apply { textSize = baseFontSize }, result, hrOffset, hrOffset + 1, replacementMetrics)
        )

        val layout = StaticLayout.Builder
            .obtain(result, 0, result.length, TextPaint().apply { textSize = baseFontSize }, 240)
            .build()
        val hrLine = layout.getLineForOffset(hrOffset)
        assertTrue(
            "Horizontal rule line should not report a visible replacement glyph width; actual width=${layout.getLineWidth(hrLine)}",
            layout.getLineWidth(hrLine) <= 1f
        )
    }

    @Test
    fun `render - image span honors preferred dimensions`() {
        val json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "https://example.com/cat.png",
                "width": 140,
                "height": 80
            }}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, density = 1f)

        assertTrue(
            "Image should contain object replacement character. Got: '$result'",
            result.toString().contains("\uFFFC")
        )

        val imageSpans = result.getSpans(0, result.length, BlockImageSpan::class.java)
        assertEquals("Should have one BlockImageSpan", 1, imageSpans.size)

        val (widthPx, heightPx) = imageSpans.single().currentSizePx()
        assertEquals(140, widthPx)
        assertEquals(80, heightPx)
    }

    @Test
    fun `render - oversized preferred image dimensions scale to host width`() {
        val hostView = TextView(org.robolectric.RuntimeEnvironment.getApplication()).apply {
            measure(
                View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
            )
            layout(0, 0, measuredWidth, measuredHeight)
        }
        val json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "https://example.com/cat.png",
                "width": 4000,
                "height": 2000
            }}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(
            json,
            baseFontSize,
            textColor,
            density = 1f,
            hostView = hostView
        )

        val imageSpan = result.getSpans(0, result.length, BlockImageSpan::class.java).single()
        val (widthPx, heightPx) = imageSpan.currentSizePx()
        assertTrue(widthPx <= hostView.width)
        assertTrue(abs(heightPx - (widthPx / 2)) <= 1)
    }

    @Test
    fun `render - data url decoder handles expo style payloads`() {
        val dataUrl =
            "data:image/gif;base64,R0lGODdhAQABAIAAAP///////ywAAAAAAQABAAACAkQBADs="

        val bitmap = RenderImageDecoder.decodeSource(dataUrl)

        assertNotNull("Standard base64 image data URLs should decode", bitmap)
        assertEquals(1, bitmap?.width)
        assertEquals(1, bitmap?.height)
    }

    @Test
    fun `render - data url decoder accepts url safe base64`() {
        val standardDataUrl =
            "data:image/gif;base64,R0lGODdhAQABAIAAAP///////ywAAAAAAQABAAACAkQBADs="
        val bytes = RenderImageDecoder.decodeDataUrlBytes(standardDataUrl)
        assertNotNull(bytes)

        val urlSafePayload = Base64.encodeToString(
            bytes,
            Base64.URL_SAFE or Base64.NO_WRAP
        )
        val bitmap = RenderImageDecoder.decodeSource("data:image/gif;base64,$urlSafePayload")

        assertNotNull("URL-safe base64 image data URLs should decode", bitmap)
        assertEquals(1, bitmap?.width)
        assertEquals(1, bitmap?.height)
    }

    @Test
    fun `render - data url image span is ready on first render`() {
        val dataUrl =
            "data:image/gif;base64,R0lGODdhAQABAIAAAP///////ywAAAAAAQABAAACAkQBADs="
        val hostView = TextView(org.robolectric.RuntimeEnvironment.getApplication()).apply {
            measure(
                View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
            )
            layout(0, 0, measuredWidth, measuredHeight)
        }
        val json = """
        [
            {"type": "voidBlock", "nodeType": "image", "docPos": 1, "attrs": {
                "src": "$dataUrl"
            }}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(
            json,
            baseFontSize,
            textColor,
            density = 1f,
            hostView = hostView
        )

        val imageSpan = result.getSpans(0, result.length, BlockImageSpan::class.java).single()
        val (widthPx, heightPx) = imageSpan.currentSizePx()
        assertEquals(1, widthPx)
        assertEquals(1, heightPx)
    }

    @Test
    fun `render - image loader deduplicates concurrent remote loads`() {
        RenderImageLoader.resetForTesting()
        val decodeCount = AtomicInteger(0)
        val decodeStarted = CountDownLatch(1)
        val releaseDecode = CountDownLatch(1)
        val callbacks = CountDownLatch(2)
        val bitmap = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        val loaded = mutableListOf<Bitmap?>()

        RenderImageLoader.decodeSourceOverride = {
            decodeCount.incrementAndGet()
            decodeStarted.countDown()
            assertTrue(releaseDecode.await(2, TimeUnit.SECONDS))
            bitmap
        }

        try {
            RenderImageLoader.load("https://example.com/cat.png") {
                synchronized(loaded) {
                    loaded += it
                }
                callbacks.countDown()
            }
            assertTrue(decodeStarted.await(2, TimeUnit.SECONDS))
            RenderImageLoader.load("https://example.com/cat.png") {
                synchronized(loaded) {
                    loaded += it
                }
                callbacks.countDown()
            }

            releaseDecode.countDown()
            assertTrue(callbacks.await(2, TimeUnit.SECONDS))
            assertEquals(1, decodeCount.get())
            assertEquals(2, loaded.size)
            assertTrue(loaded.all { loadedBitmap -> loadedBitmap === bitmap })
        } finally {
            releaseDecode.countDown()
            RenderImageLoader.resetForTesting()
        }
    }

    @Test
    fun `render - large images are downsampled for decode`() {
        assertEquals(1, RenderImageDecoder.calculateInSampleSize(width = 1024, height = 768))
        assertEquals(2, RenderImageDecoder.calculateInSampleSize(width = 4096, height = 2048))
        assertEquals(4, RenderImageDecoder.calculateInSampleSize(width = 8192, height = 4096))
    }

    // ── Multiple Paragraphs ─────────────────────────────────────────────

    /** Two consecutive paragraphs should be separated by a newline. */
    @Test
    fun `render - multiple paragraphs`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals(
            "Two paragraphs should be separated by a newline",
            "First\nSecond", result.toString()
        )
    }

    // ── Mixed Marks in Same Paragraph ───────────────────────────────────

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

    // ── Mark Aliases ────────────────────────────────────────────────────

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

    // ── All Marks Combined ──────────────────────────────────────────────

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

    // ── Ordered List ────────────────────────────────────────────────────

    @Test
    fun `render - ordered list item`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val string = result.toString()

        assertTrue(
            "Ordered list should contain '1. ' marker. Got: '$string'",
            string.contains("1. ")
        )
        assertTrue(
            "Ordered list should contain '2. ' marker. Got: '$string'",
            string.contains("2. ")
        )
        assertTrue("Should contain first item text", string.contains("First item"))
        assertTrue("Should contain second item text", string.contains("Second item"))
    }

    // ── Unordered List ──────────────────────────────────────────────────

    @Test
    fun `render - unordered list item`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val string = result.toString()

        assertTrue(
            "Unordered list should contain bullet character. Got: '$string'",
            string.contains("\u2022")
        )
        assertTrue("Should contain item text", string.contains("Bullet item"))
    }

    @Test
    fun `render - unordered list marker keeps body text font metrics`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Bullet item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val markerSpans = result.getSpans(0, 1, AbsoluteSizeSpan::class.java)
        val textSpans = result.getSpans(2, 3, AbsoluteSizeSpan::class.java)

        assertTrue("Marker should have a size span", markerSpans.isNotEmpty())
        assertTrue("Text should have a size span", textSpans.isNotEmpty())
        assertEquals(textSpans[0].size, markerSpans[0].size)
        assertEquals(baseFontSize.toInt(), textSpans[0].size)
    }

    // ── Opaque Atoms ────────────────────────────────────────────────────

    @Test
    fun `render - opaque inline atom`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "before ", "marks": []},
            {"type": "opaqueInlineAtom", "label": "widget", "docPos": 8},
            {"type": "textRun", "text": " after", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertTrue(
            "Opaque inline atom should render as '[widget]'. Got: '${result}'",
            result.toString().contains("[widget]")
        )
    }

    @Test
    fun `render - mention inline atom uses visible label and mention theme`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hello ", "marks": []},
            {"type": "opaqueInlineAtom", "nodeType": "mention", "label": "@Alice", "docPos": 7},
            {"type": "textRun", "text": "!", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme(
            mentions = EditorMentionTheme(
                textColor = 0xff112233.toInt(),
                backgroundColor = 0xffddeeff.toInt(),
                fontWeight = "bold"
            )
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)

        assertTrue(
            "Mention inline atom should render its visible label. Got: '${result}'",
            result.toString().contains("@Alice")
        )
        assertTrue(
            "Mention inline atom should not use generic opaque brackets. Got: '${result}'",
            !result.toString().contains("[@Alice]")
        )
    }

    @Test
    fun `render - mention inline atom merges element mention theme override`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {
                "type": "opaqueInlineAtom",
                "nodeType": "mention",
                "label": "@Alice",
                "docPos": 1,
                "mentionTheme": {"textColor": "#445566"}
            },
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme(
            mentions = EditorMentionTheme(
                textColor = 0xff112233.toInt(),
                backgroundColor = 0xffddeeff.toInt(),
                fontWeight = "bold"
            )
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)

        assertEquals("@Alice", result.toString())
        val foreground = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
            .firstOrNull()
        val background = result.getSpans(0, result.length, BackgroundColorSpan::class.java)
            .firstOrNull()
        val boldSpan = result.getSpans(0, result.length, StyleSpan::class.java)
            .firstOrNull { it.style == Typeface.BOLD }

        assertEquals(Color.parseColor("#445566"), foreground?.foregroundColor)
        assertEquals(0xffddeeff.toInt(), background?.backgroundColor)
        assertNotNull("Mention override should preserve global bold styling", boldSpan)
    }

    @Test
    fun `render - opaque block atom`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "blockEnd"},
            {"type": "opaqueBlockAtom", "label": "codeBlock", "docPos": 7}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertTrue(
            "Opaque block atom should render as '[codeBlock]'. Got: '${result}'",
            result.toString().contains("[codeBlock]")
        )
    }

    // ── Invalid / Edge Cases ────────────────────────────────────────────

    @Test
    fun `render - invalid JSON`() {
        val result = RenderBridge.buildSpannable("not valid json", baseFontSize, textColor)
        assertEquals(
            "Invalid JSON should produce empty SpannableStringBuilder",
            "", result.toString()
        )
    }

    @Test
    fun `render - empty array`() {
        val result = RenderBridge.buildSpannable("[]", baseFontSize, textColor)
        assertEquals(
            "Empty array should produce empty SpannableStringBuilder",
            "", result.toString()
        )
    }

    // ── List Marker Generation ──────────────────────────────────────────

    @Test
    fun `list marker - ordered`() {
        val ctx = org.json.JSONObject("""{"ordered": true, "index": 3}""")
        val marker = RenderBridge.listMarkerString(ctx)
        assertEquals("Ordered list item 3 should produce '3. '", "3. ", marker)
    }

    @Test
    fun `list marker - unordered`() {
        val ctx = org.json.JSONObject("""{"ordered": false, "index": 1}""")
        val marker = RenderBridge.listMarkerString(ctx)
        assertEquals(
            "Unordered list should produce bullet + space",
            "\u2022 ", marker
        )
    }

    // ── Link Mark ───────────────────────────────────────────────────────

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

    // ── Depth Indentation ───────────────────────────────────────────────

    @Test
    fun `render - nested block indentation`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "indented", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("indented", result.toString())

        // Check for LeadingMarginSpan with expected indent.
        val marginSpans = result.getSpans(0, result.length, LeadingMarginSpan.Standard::class.java)
        assertTrue(
            "Depth 2 paragraph should have LeadingMarginSpan",
            marginSpans.isNotEmpty()
        )
        val expectedIndent = (2 * LayoutConstants.INDENT_PER_DEPTH).toInt()
        val actualIndent = marginSpans[0].getLeadingMargin(true)
        assertEquals(
            "Depth 2 paragraph should have ${expectedIndent}px indent",
            expectedIndent, actualIndent
        )
    }

    @Test
    fun `render - blockquote applies quote span and blockquote text style`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Quoted", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "blockquote": {
                "indent": 20,
                "borderColor": "#aa5500",
                "borderWidth": 4,
                "markerGap": 10,
                "text": { "color": "#334455" }
              }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)

        val quoteSpans = result.getSpans(0, result.length, BlockquoteSpan::class.java)
        assertTrue("Quoted paragraph should receive BlockquoteSpan", quoteSpans.isNotEmpty())
        assertEquals(20, quoteSpans.single().getLeadingMargin(true))

        val colorSpans = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
        assertTrue(
            "Blockquote text style should override text color",
            colorSpans.any { it.foregroundColor == Color.parseColor("#334455") }
        )
    }

    @Test
    fun `render - blockquote does not insert extra leading paragraph break`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "World", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)

        assertEquals("Hello\nWorld", result.toString())
    }

    @Test
    fun `render - consecutive blockquote paragraphs share one quote span`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "World", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)

        assertEquals("Hello\nWorld", result.toString())
        val quoteSpans = result.getSpans(0, result.length, BlockquoteSpan::class.java)
        assertEquals(1, quoteSpans.size)
    }

    @Test
    fun `render - trailing hard break in blockquote preserves quote span into following paragraph`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 6},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Tail", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        assertEquals("Hello\n\u200B\nTail", result.toString())
        val quoteSpans = result.getSpans(0, result.length, BlockquoteSpan::class.java)
        assertEquals(1, quoteSpans.size)
    }

    @Test
    fun `render - trailing hard break in blockquote appends synthetic placeholder with quote styling`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "A", "marks": []},
            {"type": "voidInline", "nodeType": "hardBreak", "docPos": 2},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        assertEquals("A\n\u200B", result.toString())

        val placeholderIndex = result.length - 1
        val placeholderAnnotations =
            result.getSpans(placeholderIndex, placeholderIndex + 1, Annotation::class.java)
        assertTrue(
            "Trailing hard-break placeholder should be marked as synthetic",
            placeholderAnnotations.any { it.key == "nativeSyntheticPlaceholder" }
        )
        assertTrue(
            "Trailing hard-break placeholder should keep blockquote styling",
            placeholderAnnotations.any { it.key == "nativeBlockquote" }
        )
    }

    @Test
    fun `render - blockquote span ends before separator newline to plain paragraph`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Hello", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "World", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        assertEquals("Hello\nWorld", result.toString())

        val quoteSpans = result.getSpans(0, result.length, BlockquoteSpan::class.java)
        assertEquals(1, quoteSpans.size)
        assertEquals(
            "Blockquote span should end at the separator newline boundary to following plain content",
            6,
            result.getSpanEnd(quoteSpans.single())
        )
    }

    @Test
    fun `render - paragraph preceding blockquote does not inherit quote indent`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Intro", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Quote", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        assertEquals("Intro\nQuote", result.toString())

        val quoteSpans = result.getSpans(0, result.length, BlockquoteSpan::class.java)
        assertEquals(1, quoteSpans.size)
        assertEquals(
            "Blockquote span should start at the quoted paragraph, not the preceding plain paragraph",
            6,
            result.getSpanStart(quoteSpans.single())
        )
    }

    @Test
    fun `render - nested list item inside blockquote indents more than parent item`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1, "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Parent", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 2, "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 3},
            {"type": "textRun", "text": "Child", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)

        val parentIndex = result.indexOf("Parent")
        val childIndex = result.indexOf("Child")
        assertTrue(parentIndex >= 0)
        assertTrue(childIndex >= 0)
        val parentMargin = result
            .getSpans(parentIndex, parentIndex + 1, LeadingMarginSpan::class.java)
            .sumOf { it.getLeadingMargin(true) }
        val childMargin = result
            .getSpans(childIndex, childIndex + 1, LeadingMarginSpan::class.java)
            .sumOf { it.getLeadingMargin(true) }

        assertTrue(
            "nested list item inside a blockquote should indent more than its parent item",
            childMargin > parentMargin
        )
    }

    @Test
    fun `render - first level list inside blockquote keeps extra list indent`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "blockquote", "depth": 0},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1, "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Quoted item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val quotedIndex = result.indexOf("Quoted item")
        assertTrue(quotedIndex >= 0)

        val quotedMargins = result.getSpans(
            quotedIndex,
            quotedIndex + 1,
            LeadingMarginSpan::class.java
        )
        val totalMargin = quotedMargins.sumOf { it.getLeadingMargin(true) }

        assertEquals(
            "first-level list text inside a blockquote should keep its extra list indent",
            42,
            totalMargin
        )
    }

    @Test
    fun `blockquote span trims bottom on final quoted line before plain content`() {
        val text = SpannableStringBuilder("Quote\nPlain")
        text.setSpan(
            Annotation(RenderBridge.NATIVE_BLOCKQUOTE_ANNOTATION, "1"),
            0,
            5,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        text.setSpan(
            Annotation(RenderBridge.NATIVE_BLOCKQUOTE_ANNOTATION, "1"),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )

        val paint = TextPaint().apply { textSize = 16f }
        val layout = StaticLayout.Builder
            .obtain(text, 0, text.length, paint, 200)
            .build()
        val span = BlockquoteSpan(
            baseIndentPx = 0,
            totalIndentPx = 18,
            stripeColor = Color.BLACK,
            stripeWidthPx = 3,
            gapWidthPx = 8
        )
        val line = 0
        val bottom = span.resolvedStripeBottom(
            text = text,
            start = layout.getLineStart(line),
            end = layout.getLineEnd(line),
            baseline = layout.getLineBaseline(line),
            bottom = layout.getLineBottom(line),
            layout = layout,
            paint = paint
        )

        assertEquals(
            "Final quoted line before plain content should trim stripe to baseline + font descent",
            layout.getLineBaseline(line) + paint.fontMetrics.descent,
            bottom,
            0.01f
        )
    }

    @Test
    fun `blockquote span ignores paragraph spacer when trimming final quoted line`() {
        val text = SpannableStringBuilder("Quote\nPlain")
        text.setSpan(
            Annotation(RenderBridge.NATIVE_BLOCKQUOTE_ANNOTATION, "1"),
            0,
            5,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        text.setSpan(
            Annotation(RenderBridge.NATIVE_BLOCKQUOTE_ANNOTATION, "1"),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        text.setSpan(
            ParagraphSpacerSpan(
                spacingPx = 40,
                baseFontSize = 16,
                textColor = Color.BLACK
            ),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )

        val paint = TextPaint().apply { textSize = 16f }
        val layout = StaticLayout.Builder
            .obtain(text, 0, text.length, paint, 200)
            .build()
        val span = BlockquoteSpan(
            baseIndentPx = 0,
            totalIndentPx = 18,
            stripeColor = Color.BLACK,
            stripeWidthPx = 3,
            gapWidthPx = 8
        )
        val line = 0
        val bottom = span.resolvedStripeBottom(
            text = text,
            start = layout.getLineStart(line),
            end = layout.getLineEnd(line),
            baseline = layout.getLineBaseline(line),
            bottom = layout.getLineBottom(line),
            layout = layout,
            paint = paint
        )

        assertTrue("Paragraph spacer should inflate line metrics in this reproduction", layout.getLineDescent(line) > paint.fontMetrics.descent)
        assertEquals(
            "Final quoted line should trim to font descent even when paragraph spacing inflates layout descent",
            layout.getLineBaseline(line) + paint.fontMetrics.descent,
            bottom,
            0.01f
        )
    }

    @Test
    fun `render - theme overrides paragraph typography`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Styled", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "text": { "fontSize": 18, "color": "#112233" },
              "paragraph": { "lineHeight": 28, "spacingAfter": 14 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)

        val colorSpans = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
        val sizeSpans = result.getSpans(0, result.length, AbsoluteSizeSpan::class.java)
        val lineHeightSpans = result.getSpans(0, result.length, FixedLineHeightSpan::class.java)

        assertTrue(colorSpans.any { it.foregroundColor == Color.parseColor("#112233") })
        assertTrue(sizeSpans.any { it.size == 18 })
        assertTrue(lineHeightSpans.isNotEmpty())
    }

    @Test
    fun `render - theme overrides specific heading level typography`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "h2", "depth": 0},
            {"type": "textRun", "text": "Styled heading", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "text": { "fontSize": 16, "color": "#112233" },
              "headings": {
                "h2": { "fontSize": 28, "fontWeight": "700", "color": "#445566", "lineHeight": 34, "spacingAfter": 12 },
                "h4": { "fontSize": 18, "color": "#AA5500" }
              }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)

        val colorSpans = result.getSpans(0, result.length, ForegroundColorSpan::class.java)
        val sizeSpans = result.getSpans(0, result.length, AbsoluteSizeSpan::class.java)
        val lineHeightSpans = result.getSpans(0, result.length, FixedLineHeightSpan::class.java)
        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java)

        assertTrue(colorSpans.any { it.foregroundColor == Color.parseColor("#445566") })
        assertTrue(sizeSpans.any { it.size == 28 })
        assertTrue(lineHeightSpans.isNotEmpty())
        assertTrue(styleSpans.any { it.style == Typeface.BOLD })
    }

    @Test
    fun `render - paragraph does not inherit text line height when paragraph line height is unset`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Styled", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "text": { "fontSize": 18, "lineHeight": 28 },
              "paragraph": { "spacingAfter": 14 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val lineHeightSpans = result.getSpans(0, result.length, FixedLineHeightSpan::class.java)

        assertTrue(lineHeightSpans.isEmpty())
    }

    @Test
    fun `render - no spacer span when spacingAfter is unset`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First paragraph", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second paragraph", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val spacerSpans = result.getSpans(0, result.length, ParagraphSpacerSpan::class.java)

        assertTrue("No spacer spans when theme has no spacingAfter", spacerSpans.isEmpty())
    }

    @Test
    fun `render - paragraph spacing applied to inter-block newline via ParagraphSpacerSpan`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "First paragraph", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Second paragraph", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "paragraph": { "spacingAfter": 14 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val separatorIndex = result.toString().indexOf('\n')

        // Spacer span should be on the inter-block newline character.
        val spacerSpans = result.getSpans(separatorIndex, separatorIndex + 1, ParagraphSpacerSpan::class.java)
        assertTrue("Inter-block newline should have a ParagraphSpacerSpan", spacerSpans.isNotEmpty())

        // No spacer span on paragraph content.
        val firstParaSpans = result.getSpans(0, separatorIndex, ParagraphSpacerSpan::class.java)
        assertTrue("Paragraph content should not have spacer spans", firstParaSpans.isEmpty())

        val secondParaSpans = result.getSpans(separatorIndex + 1, result.length, ParagraphSpacerSpan::class.java)
        assertTrue("Second paragraph content should not have spacer spans", secondParaSpans.isEmpty())
    }

    @Test
    fun `render - list item spacing applies to sibling list item separator`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 2, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "list": { "itemSpacing": 14 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val separatorIndex = result.toString().indexOf('\n')

        assertTrue("Expected a separator newline between list items", separatorIndex >= 0)
        val spacerSpans = result.getSpans(separatorIndex, separatorIndex + 1, ParagraphSpacerSpan::class.java)
        assertTrue("List item separator should receive ParagraphSpacerSpan from itemSpacing", spacerSpans.isNotEmpty())
    }

    @Test
    fun `render - nested first list item does not inherit paragraph spacing when itemSpacing is zero`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "paragraph": { "spacingAfter": 14 },
              "list": { "itemSpacing": 0 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val separatorIndex = result.toString().indexOf('\n')

        assertTrue("Expected a separator newline before nested list item", separatorIndex >= 0)
        val spacerSpans = result.getSpans(separatorIndex, separatorIndex + 1, ParagraphSpacerSpan::class.java)
        assertTrue(
            "Nested list separator should not keep parent paragraph spacing when itemSpacing is zero",
            spacerSpans.isEmpty()
        )
    }

    @Test
    fun `render - theme overrides list indentation`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "list": { "indent": 32, "markerScale": 1.5, "markerColor": "#334455" }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val marginSpans = result.getSpans(0, result.length, LeadingMarginSpan.Standard::class.java)
        assertTrue(marginSpans.isNotEmpty())
        assertEquals(64, marginSpans[0].getLeadingMargin(true))
    }

    @Test
    fun `render - unordered marker scale does not widen list text gutter`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val baseTheme = EditorTheme.fromJson(
            """
            {
              "text": { "fontSize": 40 },
              "list": { "indent": 32, "markerScale": 1 }
            }
            """.trimIndent()
        )
        val scaledTheme = EditorTheme.fromJson(
            """
            {
              "text": { "fontSize": 40 },
              "list": { "indent": 32, "markerScale": 2 }
            }
            """.trimIndent()
        )

        val baseResult = RenderBridge.buildSpannable(json, baseFontSize, textColor, baseTheme, 1f)
        val scaledResult = RenderBridge.buildSpannable(json, baseFontSize, textColor, scaledTheme, 1f)
        val baseMargin = baseResult.getSpans(0, baseResult.length, LeadingMarginSpan.Standard::class.java).single()
        val scaledMargin = scaledResult.getSpans(0, scaledResult.length, LeadingMarginSpan.Standard::class.java).single()

        assertEquals(baseMargin.getLeadingMargin(false), scaledMargin.getLeadingMargin(false))
    }

    @Test
    fun `render - themed list marker receives line height span`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "paragraph": { "lineHeight": 28 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val markerLineHeightSpans = result.getSpans(0, 1, FixedLineHeightSpan::class.java)
        assertTrue(markerLineHeightSpans.isNotEmpty())
    }

    @Test
    fun `render - indented list item has larger leading margin than non-indented`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 2, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Indented item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val text = result.toString()
        val newlineIndex = text.indexOf('\n')

        val allMargins = result.getSpans(0, result.length, LeadingMarginSpan.Standard::class.java)
        assertTrue("List items should have LeadingMarginSpans", allMargins.isNotEmpty())

        val firstItemMargin = allMargins.firstOrNull { result.getSpanStart(it) == 0 }
        assertNotNull("First item should have a paragraph-scoped LeadingMarginSpan", firstItemMargin)

        val indentedItemMargin = allMargins.firstOrNull { result.getSpanStart(it) > newlineIndex }
        assertNotNull("Indented item should have its own paragraph-scoped LeadingMarginSpan", indentedItemMargin)

        val firstIndent = firstItemMargin!!.getLeadingMargin(true)
        val indentedIndent = indentedItemMargin!!.getLeadingMargin(true)

        assertTrue(
            "Indented item margin ($indentedIndent) should be greater than first item margin ($firstIndent)",
            indentedIndent > firstIndent
        )
    }

    @Test
    fun `render - list indentation uses paragraph span flags`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Indented item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val marginSpans = result.getSpans(0, result.length, LeadingMarginSpan.Standard::class.java)

        assertTrue("Indented list item should have a LeadingMarginSpan", marginSpans.isNotEmpty())
        assertEquals(
            "LeadingMarginSpan should be paragraph-scoped",
            Spanned.SPAN_PARAGRAPH,
            result.getSpanFlags(marginSpans[0])
        )
        assertEquals(
            "LeadingMarginSpan should start at the list paragraph start, including the marker",
            0,
            result.getSpanStart(marginSpans[0])
        )
    }

    @Test
    fun `render - list paragraph uses a single leading margin span across multiple text runs`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Alpha", "marks": []},
            {"type": "textRun", "text": " Beta", "marks": ["bold"]},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val marginSpans = result.getSpans(0, result.length, LeadingMarginSpan.Standard::class.java)
        val paragraphSpans = marginSpans.filter { result.getSpanStart(it) == 0 }

        assertEquals("Paragraph should have exactly one LeadingMarginSpan", 1, paragraphSpans.size)
    }

    @Test
    fun `layout - sibling list items at same depth share the same visual left offset`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 3, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 3, "start": 1, "isFirst": false, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 3, "total": 3, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Third", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val paint = TextPaint().apply { textSize = baseFontSize }
        val layout = StaticLayout.Builder
            .obtain(result, 0, result.length, paint, 400)
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(false)
            .build()

        assertEquals("Expected one line per list item", 3, layout.lineCount)

        val firstLeft = layout.getLineLeft(0)
        val secondLeft = layout.getLineLeft(1)
        val thirdLeft = layout.getLineLeft(2)

        assertEquals("First and second sibling items should align", firstLeft, secondLeft, 0.01f)
        assertEquals("Second and third sibling items should align", secondLeft, thirdLeft, 0.01f)
    }

    @Test
    fun `layout - nested middle item does not shift trailing outer sibling`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 3, "start": 1, "isFirst": true, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "First", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 2, "total": 3, "start": 1, "isFirst": false, "isLast": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Second", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 3, "total": 3, "start": 1, "isFirst": false, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Third", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val paint = TextPaint().apply { textSize = baseFontSize }
        val layout = StaticLayout.Builder
            .obtain(result, 0, result.length, paint, 400)
            .setAlignment(Layout.Alignment.ALIGN_NORMAL)
            .setIncludePad(false)
            .build()

        val text = result.toString()
        val firstOffset = text.indexOf("First")
        val secondOffset = text.indexOf("Second")
        val nestedOffset = text.indexOf("Nested")
        val thirdOffset = text.indexOf("Third")

        val firstLeft = layout.getPrimaryHorizontal(firstOffset)
        val secondLeft = layout.getPrimaryHorizontal(secondOffset)
        val nestedLeft = layout.getPrimaryHorizontal(nestedOffset)
        val thirdLeft = layout.getPrimaryHorizontal(thirdOffset)
        val marginSummary = result
            .getSpans(0, result.length, LeadingMarginSpan.Standard::class.java)
            .joinToString(" | ") {
                "start=${result.getSpanStart(it)} end=${result.getSpanEnd(it)} margin=${it.getLeadingMargin(true)}"
            }

        val outerAligned = kotlin.math.abs(firstLeft - secondLeft) <= 0.01f
        val nestedIndented = nestedLeft > secondLeft
        val trailingAligned = kotlin.math.abs(firstLeft - thirdLeft) <= 0.01f

        if (!outerAligned || !nestedIndented || !trailingAligned) {
            fail(
                "Unexpected nested list layout: first=$firstLeft second=$secondLeft " +
                    "nested=$nestedLeft third=$thirdLeft text=$text margins=$marginSummary"
            )
        }
    }

    @Test
    fun `render - unordered list marker uses centered bullet span`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, null, 1f)
        val bulletSpans = result.getSpans(0, 2, CenteredBulletSpan::class.java)

        assertTrue(bulletSpans.isNotEmpty())
    }

    @Test
    fun `FixedLineHeightSpan - pushes all extra space below baseline`() {
        val span = FixedLineHeightSpan(30)
        val fm = android.graphics.Paint.FontMetricsInt()
        fm.ascent = -14
        fm.top = -14
        fm.descent = 6
        fm.bottom = 6
        // currentHeight = 6 - (-14) = 20, extra = 30 - 20 = 10

        span.chooseHeight("x", 0, 1, 0, 0, fm)

        assertEquals("ascent unchanged", -14, fm.ascent)
        assertEquals("top unchanged", -14, fm.top)
        assertEquals("descent increased by extra", 16, fm.descent)
        assertEquals("bottom matches descent", 16, fm.bottom)
    }

    @Test
    fun `FixedLineHeightSpan - no change when height matches target`() {
        val span = FixedLineHeightSpan(20)
        val fm = android.graphics.Paint.FontMetricsInt()
        fm.ascent = -14
        fm.top = -14
        fm.descent = 6
        fm.bottom = 6

        span.chooseHeight("x", 0, 1, 0, 0, fm)

        assertEquals(-14, fm.ascent)
        assertEquals(-14, fm.top)
        assertEquals(6, fm.descent)
        assertEquals(6, fm.bottom)
    }

    @Test
    fun `CenteredBulletSpan - restores paint state after draw`() {
        val bulletRadius = 3f
        val markerWidth = 24f
        val bodyFontSize = 16f
        val markerFontSize = 32f
        val span = CenteredBulletSpan(
            Color.BLACK,
            markerWidth,
            bulletRadius,
            bodyFontSize,
            LayoutConstants.LIST_MARKER_TEXT_GAP
        )

        val paint = Paint()
        paint.textSize = markerFontSize
        paint.color = Color.RED
        paint.style = Paint.Style.STROKE

        val bitmap = android.graphics.Bitmap.createBitmap(100, 100, android.graphics.Bitmap.Config.ARGB_8888)
        val canvas = android.graphics.Canvas(bitmap)

        span.draw(canvas, "•", 0, 1, 0f, 0, 20, 40, paint)

        assertEquals("textSize should be restored", markerFontSize, paint.textSize)
        assertEquals("color should be restored", Color.RED, paint.color)
        assertEquals("style should be restored", Paint.Style.STROKE, paint.style)
    }

    @Test
    fun `CenteredBulletSpan - larger bullet preserves text side gap`() {
        val markerWidth = 24f
        val bodyFontSize = 16f
        val gapToText = LayoutConstants.LIST_MARKER_TEXT_GAP
        val normalSpan = CenteredBulletSpan(Color.BLACK, markerWidth, 3f, bodyFontSize, gapToText)
        val scaledSpan = CenteredBulletSpan(Color.BLACK, markerWidth, 6f, bodyFontSize, gapToText)

        assertEquals(normalSpan.textSideGapPx(0f), scaledSpan.textSideGapPx(0f), 0.01f)
        assertEquals(gapToText, scaledSpan.textSideGapPx(0f), 0.01f)
    }


}
