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
internal class RenderBridgeLayoutTest : RenderBridgeTestFixture() {
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
            placeholderAnnotations.any {
                it.key == RenderBridge.NATIVE_SYNTHETIC_PLACEHOLDER_ANNOTATION
            }
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
    fun `layout - paragraph spacing remains additive with fixed line height`() {
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

        fun secondParagraphBaseline(spacingAfter: Int): Int {
            val theme = EditorTheme.fromJson(
                """
                {
                  "paragraph": { "lineHeight": 28, "spacingAfter": $spacingAfter }
                }
                """.trimIndent()
            )
            val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
            val layout = StaticLayout.Builder
                .obtain(result, 0, result.length, TextPaint().apply { textSize = baseFontSize }, 400)
                .setIncludePad(false)
                .build()
            val secondParagraphLine = layout.getLineForOffset(result.indexOf("Second paragraph"))
            return layout.getLineBaseline(secondParagraphLine)
        }

        val baselineWithoutSpacing = secondParagraphBaseline(spacingAfter = 0)
        val baselineWithSpacing = secondParagraphBaseline(spacingAfter = 14)

        assertEquals(14, baselineWithSpacing - baselineWithoutSpacing)
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
    fun `measureHeight returns positive height for single paragraph`() {
        val renderJSON = """[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"Hello world"},{"type":"blockEnd"}]"""
        val height = RenderBridge.measureHeight(
            json = renderJSON,
            themeJson = null,
            width = 375f,
            density = 1f
        )
        assertTrue("Single paragraph should have positive height, got $height", height > 0f)
    }

    @Test
    fun `measureHeight returns zero for empty content`() {
        val height = RenderBridge.measureHeight(
            json = "[]",
            themeJson = null,
            width = 375f,
            density = 1f
        )
        assertEquals("Empty content should have zero height", 0f, height)
    }

    @Test
    fun `measureHeight adds content insets`() {
        val renderJSON = """[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"Hello world"},{"type":"blockEnd"}]"""
        val noInsetHeight = RenderBridge.measureHeight(
            json = renderJSON,
            themeJson = null,
            width = 375f,
            density = 1f
        )
        val insetHeight = RenderBridge.measureHeight(
            json = renderJSON,
            themeJson = """{"contentInsets":{"top":20,"bottom":20}}""",
            width = 375f,
            density = 1f
        )
        assertEquals(
            "Content insets should add 40 to height",
            noInsetHeight + 40f,
            insetHeight,
            1f
        )
    }

    @Test
    fun `render - code block followed by paragraph does not crash and carries CodeBlockSpan`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
            {"type": "textRun", "text": "let x = 1", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "after", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertEquals("let x = 1\nafter", result.toString())
        val spans = result.getSpans(0, result.length, CodeBlockSpan::class.java)
        assertEquals("Exactly one CodeBlockSpan expected", 1, spans.size)
        assertEquals(0, result.getSpanStart(spans[0]))
        assertEquals("let x = 1".length, result.getSpanEnd(spans[0]))
    }

    @Test
    fun `code block span survives splice into a larger editable at the right offsets`() {
        // Build a fragment containing only the codeBlock (as the incremental
        // path does via buildSpannable), splice it into a builder
        // that already has "intro\n" via replace(), and assert
        // getSpanStart/getSpanEnd reflect the spliced position — this is what
        // drawBackground must consume.
        val fragment = RenderBridge.buildSpannable(
            """
            [
                {"type": "blockStart", "nodeType": "codeBlock", "depth": 0},
                {"type": "textRun", "text": "code", "marks": []},
                {"type": "blockEnd"}
            ]
            """.trimIndent(),
            baseFontSize, textColor
        )
        val host = android.text.SpannableStringBuilder("intro\n")
        host.replace(host.length, host.length, fragment)

        val spans = host.getSpans(0, host.length, CodeBlockSpan::class.java)
        assertEquals(1, spans.size)
        assertEquals("intro\n".length, host.getSpanStart(spans[0]))
        assertEquals(host.length, host.getSpanEnd(spans[0]))
    }
}
