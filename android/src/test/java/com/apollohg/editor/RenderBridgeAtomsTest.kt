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
internal class RenderBridgeAtomsTest : RenderBridgeTestFixture() {
    @Test
    fun `registered atom void block gets estimated spacer span`() {
        val result = RenderBridge.buildSpannable(
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]""",
            baseFontSize,
            textColor,
            atomConfiguration = AtomRenderConfiguration(
                registeredNodeTypes = setOf("counterCard"),
                estimatedHeightsDp = mapOf("counterCard" to 120f),
                measuredHeightsPx = emptyMap()
            )
        )

        val span = result.getSpans(0, result.length, AtomBlockSpan::class.java).single()
        assertEquals("counterCard:0", span.atomKey)
        assertEquals("counterCard", span.nodeType)
        assertEquals(1, span.docPos)
        assertEquals(120, span.reservedHeightPx)
    }

    @Test
    fun `registered atoms override built in void rendering`() {
        for (nodeType in listOf("image", "horizontalRule", "horizontal_rule")) {
            val result = RenderBridge.buildSpannable(
                """[{"type":"voidBlock","nodeType":"$nodeType","docPos":1,"atomId":"custom-1"}]""",
                baseFontSize,
                textColor,
                atomConfiguration = AtomRenderConfiguration(
                    registeredNodeTypes = setOf(nodeType),
                    estimatedHeightsDp = mapOf(nodeType to 120f),
                    measuredHeightsPx = emptyMap(),
                )
            )
            val span = result.getSpans(0, result.length, AtomBlockSpan::class.java).single()
            assertEquals("custom-1", span.atomKey)
            assertEquals(nodeType, span.nodeType)
            assertEquals(120, span.reservedHeightPx)
        }
    }

    @Test
    @Config(qualifiers = "xhdpi")
    fun `atom estimate converts dp to pixels at display density`() {
        val density = org.robolectric.RuntimeEnvironment
            .getApplication()
            .resources
            .displayMetrics
            .density
        val result = RenderBridge.buildSpannable(
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]""",
            baseFontSize,
            textColor,
            density = density,
            atomConfiguration = AtomRenderConfiguration(
                registeredNodeTypes = setOf("counterCard"),
                estimatedHeightsDp = mapOf("counterCard" to 120f),
                measuredHeightsPx = emptyMap()
            )
        )

        assertEquals(2f, density)
        assertEquals(
            240,
            result.getSpans(0, result.length, AtomBlockSpan::class.java)
                .single()
                .reservedHeightPx
        )
    }

    @Test
    fun `atom keys follow contract C4`() {
        val result = RenderBridge.buildSpannable(
            """
            [
              {"type":"voidBlock","nodeType":"counterCard","docPos":1},
              {"type":"voidBlock","nodeType":"counterCard","docPos":3},
              {"type":"voidBlock","nodeType":"counterCard","docPos":5,"atomId":"client-1:9"}
            ]
            """.trimIndent(),
            baseFontSize,
            textColor,
            atomConfiguration = AtomRenderConfiguration(
                registeredNodeTypes = setOf("counterCard"),
                estimatedHeightsDp = emptyMap(),
                measuredHeightsPx = emptyMap()
            )
        )

        assertEquals(
            listOf("counterCard:0", "counterCard:1", "client-1:9"),
            result.getSpans(0, result.length, AtomBlockSpan::class.java).map { it.atomKey }
        )
    }

    @Test
    fun `unregistered atom void block keeps bare replacement character`() {
        val result = RenderBridge.buildSpannable(
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]""",
            baseFontSize,
            textColor
        )

        assertEquals(LayoutConstants.OBJECT_REPLACEMENT_CHARACTER, result.toString())
        assertTrue(result.getSpans(0, result.length, AtomBlockSpan::class.java).isEmpty())
    }

    @Test
    fun `measured atom height overrides estimate`() {
        val result = RenderBridge.buildSpannable(
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]""",
            baseFontSize,
            textColor,
            atomConfiguration = AtomRenderConfiguration(
                registeredNodeTypes = setOf("counterCard"),
                estimatedHeightsDp = mapOf("counterCard" to 120f),
                measuredHeightsPx = mapOf("counterCard:0" to 260)
            )
        )

        assertEquals(
            260,
            result.getSpans(0, result.length, AtomBlockSpan::class.java)
                .single()
                .reservedHeightPx
        )
    }

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
                node = EditorMentionNodeTheme(
                    textColor = 0xff112233.toInt(),
                    backgroundColor = 0xffddeeff.toInt(),
                    fontWeight = "bold"
                )
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
                "mentionTheme": {"node": {"textColor": "#445566"}}
            },
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme(
            mentions = EditorMentionTheme(
                node = EditorMentionNodeTheme(
                    textColor = 0xff112233.toInt(),
                    backgroundColor = 0xffddeeff.toInt(),
                    fontWeight = "bold"
                )
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

    @Test
    fun `render - mention inline atom inherits surrounding block typography`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "h2", "depth": 0},
            {"type": "textRun", "text": "Hi ", "marks": []},
            {"type": "opaqueInlineAtom", "nodeType": "mention", "label": "@Alice", "docPos": 4},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "Hi ", "marks": []},
            {"type": "opaqueInlineAtom", "nodeType": "mention", "label": "@Bob", "docPos": 20},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "text": { "fontSize": 18, "fontFamily": "serif" },
              "headings": { "h2": { "fontSize": 28, "fontWeight": "700" } }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)

        val rendered = result.toString()
        val headingTextStart = rendered.indexOf("Hi ")
        val headingMentionStart = rendered.indexOf("@Alice")
        val paragraphMentionStart = rendered.indexOf("@Bob")
        assertTrue(
            "Expected heading text and both mentions in '$rendered'",
            headingTextStart >= 0 && headingMentionStart >= 0 && paragraphMentionStart >= 0
        )

        fun <T : Any> spansOver(start: Int, length: Int, type: Class<T>): List<T> = result
            .getSpans(start, start + length, type)
            .filter { result.getSpanStart(it) <= start && result.getSpanEnd(it) >= start + length }

        val headingTextSizes =
            spansOver(headingTextStart, 3, AbsoluteSizeSpan::class.java).map { it.size }
        val headingMentionSizes =
            spansOver(headingMentionStart, "@Alice".length, AbsoluteSizeSpan::class.java).map { it.size }
        val paragraphMentionSizes =
            spansOver(paragraphMentionStart, "@Bob".length, AbsoluteSizeSpan::class.java).map { it.size }
        val headingMentionFamilies =
            spansOver(headingMentionStart, "@Alice".length, TypefaceSpan::class.java).map { it.family }
        val headingMentionStyles =
            spansOver(headingMentionStart, "@Alice".length, StyleSpan::class.java).map { it.style }

        assertEquals(
            "Heading text should render at the themed heading size. Spans: $headingTextSizes",
            listOf(28),
            headingTextSizes
        )
        assertEquals(
            "Mention in an h2 should inherit the heading font size, not the view base size. Spans: $headingMentionSizes",
            listOf(28),
            headingMentionSizes
        )
        assertEquals(
            "Mention in a paragraph should inherit the themed body font size. Spans: $paragraphMentionSizes",
            listOf(18),
            paragraphMentionSizes
        )
        assertEquals(
            "Mention should inherit the themed font family. Spans: $headingMentionFamilies",
            listOf("serif"),
            headingMentionFamilies
        )
        assertEquals(
            "Mention should inherit the heading font weight. Spans: $headingMentionStyles",
            listOf(Typeface.BOLD),
            headingMentionStyles
        )
    }

    @Test
    fun `render - mention theme font weight overrides the surrounding block weight`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "h2", "depth": 0},
            {
                "type": "opaqueInlineAtom",
                "nodeType": "mention",
                "label": "@Alice",
                "docPos": 1,
                "mentionTheme": {"node": {"fontWeight": "400"}}
            },
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """
            {
              "headings": { "h2": { "fontSize": 28, "fontWeight": "700" } }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)

        val styleSpans = result.getSpans(0, result.length, StyleSpan::class.java).map { it.style }
        val sizeSpans = result.getSpans(0, result.length, AbsoluteSizeSpan::class.java).map { it.size }

        assertEquals(
            "A regular-weight mention theme should override the bold heading weight. Spans: $styleSpans",
            emptyList<Int>(),
            styleSpans
        )
        assertEquals(
            "Overriding the weight should leave the inherited heading size intact. Spans: $sizeSpans",
            listOf(28),
            sizeSpans
        )
    }
}
