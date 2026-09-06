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
internal class RenderBridgeParsingTest : RenderBridgeTestFixture() {
    @Test
    fun `policy reload and reattach preserve live edits after initial render`() {
        RenderImageLoader.resetForTesting()
        val release = CountDownLatch(1)
        val decodeCount = AtomicInteger(0)
        val initialDecodeStarted = CountDownLatch(1)
        val policyDecodeStarted = CountDownLatch(1)
        val reattachedDecodeStarted = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            when (decodeCount.incrementAndGet()) {
                1 -> initialDecodeStarted.countDown()
                2 -> policyDecodeStarted.countDown()
                3 -> reattachedDecodeStarted.countDown()
            }
            release.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
        }
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        val json = """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"textRun","text":"initial","marks":[]},
              {"type":"blockEnd"},
              {"type":"voidBlock","nodeType":"image","docPos":9,"attrs":{"src":"https://example.com/live.png"}}
            ]
        """.trimIndent()
        try {
            editor.applyRenderJSON(json)
            assertTrue(initialDecodeStarted.await(2, TimeUnit.SECONDS))
            editor.editableText.insert(0, "live ")

            editor.setImageLoadingPolicyJson("""{"readTimeoutMs":123}""")
            assertTrue(policyDecodeStarted.await(2, TimeUnit.SECONDS))
            assertTrue(editor.text.toString().startsWith("live initial"))
            invokeLifecycle(editor, "onDetachedFromWindow")
            invokeLifecycle(editor, "onAttachedToWindow")

            assertTrue(reattachedDecodeStarted.await(2, TimeUnit.SECONDS))
            assertTrue(editor.text.toString().startsWith("live initial"))
        } finally {
            release.countDown()
            RenderImageLoader.resetForTesting()
        }
    }

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
    fun `render - ProseMirror void node names`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Above", "marks": []},
            {"type": "voidInline", "nodeType": "hard_break", "docPos": 6},
            {"type": "textRun", "text": "Below", "marks": []},
            {"type": "blockEnd"},
            {"type": "voidBlock", "nodeType": "horizontal_rule", "docPos": 13}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertTrue(result.toString().contains("Above\nBelow"))
        assertTrue(result.getSpans(0, result.length, HorizontalRuleSpan::class.java).isNotEmpty())
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
    fun `render - inter-block newline carries generated separator annotation`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Alpha", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Beta", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)
        val separator = result.indexOf('\n')
        val annotations = result.getSpans(separator, separator + 1, Annotation::class.java)
            .filter { it.key == RenderBridge.NATIVE_INTER_BLOCK_SEPARATOR_ANNOTATION }

        assertEquals(1, annotations.size)
        assertEquals(separator, result.getSpanStart(annotations.single()))
        assertEquals(separator + 1, result.getSpanEnd(annotations.single()))
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
}
