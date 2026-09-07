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
import org.robolectric.annotation.GraphicsMode
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RenderBridgeListsTest : RenderBridgeTestFixture() {
    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun `render - paragraph padding does not change the list marker width`() {
        val json = """[
            {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":true,"index":1}},
            {"type":"blockStart","nodeType":"paragraph","depth":1},
            {"type":"textRun","text":"word word word word word word word word word word","marks":[]},
            {"type":"blockEnd"},
            {"type":"blockStart","nodeType":"paragraph","depth":1},
            {"type":"textRun","text":"Another paragraph","marks":[]},
            {"type":"blockEnd"},{"type":"blockEnd"}
        ]"""
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"paragraph":{"paddingLeft":10}}}""")
        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)
        val paint = TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = baseFontSize }
        val layout = EditorDocumentLayout(result, paint, 150)
        val textStart = result.toString().indexOf("word")
        assertTrue(layout.getLineStart(1) < result.toString().indexOf('\n'))
        assertEquals(layout.getPrimaryHorizontal(textStart),
            layout.getPrimaryHorizontal(layout.getLineStart(1)), 0.01f)
    }

    @Test
    @GraphicsMode(GraphicsMode.Mode.NATIVE)
    fun `render - wrapped list text aligns with first line`() {
        for (context in listOf(
            "{\"ordered\":false}",
            "{\"ordered\":true,\"index\":1}",
            "{\"ordered\":true,\"index\":123}",
            "{\"kind\":\"task\",\"checked\":false}"
        )) {
            val json = """[
                {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":$context},
                {"type":"blockStart","nodeType":"paragraph","depth":1},
                {"type":"textRun","text":"word word word word word word word word word word","marks":[]},
                {"type":"blockEnd"},{"type":"blockEnd"}
            ]"""
            val themes = listOf(null, EditorTheme.fromJson("""{
                "version":1,"styles":{
                    "text":{"fontSize":17},
                    "listMarker":{"gap":8,"ordered":{"schemes":["upperRoman"]}},
                    "checkbox":{"size":19,"gap":7}
                }
            }"""))
            for (theme in themes) for (density in listOf(1f, 2.625f)) {
                val result = RenderBridge.buildSpannable(json, baseFontSize * density, textColor, theme, density)
                val paint = TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = baseFontSize * density }
                val width = (150 * density).toInt()
                val layouts = listOf(
                    StaticLayout.Builder.obtain(result, 0, result.length, paint, width).build(),
                    EditorDocumentLayout(result, paint, width)
                )
                val textStart = result.toString().indexOf("word")
                for (layout in layouts) {
                    assertTrue(layout.lineCount > 1)
                    for (line in 1 until layout.lineCount) {
                        assertEquals("$context density=$density", layout.getPrimaryHorizontal(textStart),
                            layout.getPrimaryHorizontal(layout.getLineStart(line)), 0.01f)
                    }
                }
            }
        }
    }

    @Test
    fun `render - scalar positions follow canonical marker while replacement paints longer label`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": true, "index": 27, "total": 1, "start": 27, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """{"list":{"orderedMarker":{"schemes":["upperRoman"],"suffix":")"}}}""",
        )

        val rendered = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)
        val backingText = rendered.toString()
        val markerSpan = rendered.getSpans(
            0,
            rendered.length,
            OrderedListMarkerSpan::class.java,
        ).single()
        val canonicalTextStart = 4

        assertEquals("27. Item", backingText)
        assertEquals("XXVII)", markerSpan.label)
        assertTrue(markerSpan.label.length != rendered.getSpanEnd(markerSpan) - rendered.getSpanStart(markerSpan))
        assertEquals(0, rendered.getSpanStart(markerSpan))
        assertEquals(3, rendered.getSpanEnd(markerSpan))
        assertEquals(canonicalTextStart, PositionBridge.utf16ToScalar(canonicalTextStart, backingText))
        assertEquals(canonicalTextStart, PositionBridge.scalarToUtf16(canonicalTextStart, backingText))
    }

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

    @Test
    fun `every list marker carries the generated marker annotation`() {
        val cases = listOf(
            "listItem" to
                """{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}""",
            "listItem" to
                """{"ordered":true,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}""",
            "taskItem" to
                """{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true,"kind":"task","checked":false}"""
        )

        cases.forEach { (nodeType, listContext) ->
            val result = RenderBridge.buildSpannable(
                """
                [
                    {"type":"blockStart","nodeType":"$nodeType","depth":0,"listContext":$listContext},
                    {"type":"blockStart","nodeType":"paragraph","depth":1},
                    {"type":"textRun","text":"Item","marks":[]},
                    {"type":"blockEnd"},
                    {"type":"blockEnd"}
                ]
                """.trimIndent(),
                baseFontSize,
                textColor
            )
            val bodyStart = result.indexOf("Item")
            val annotations = result.getSpans(0, bodyStart, Annotation::class.java)
                .filter { it.key == RenderBridge.NATIVE_LIST_MARKER_ANNOTATION }

            assertEquals("$nodeType marker should be generated structure", 1, annotations.size)
            assertEquals(0, result.getSpanStart(annotations.single()))
            assertEquals(bodyStart, result.getSpanEnd(annotations.single()))
        }
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
    fun `render - list spacingAfter applies whenever a list ends`() {
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
            {"type": "textRun", "text": "Parent item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "After nested", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "After list", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """{"list":{"itemSpacing":6,"spacingAfter":20}}"""
        )
        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)

        fun spacingAt(separatorIndex: Int): Int {
            val span = result.getSpans(
                separatorIndex,
                separatorIndex + 1,
                ParagraphSpacerSpan::class.java,
            ).single()
            val paint = Paint().apply { textSize = baseFontSize }
            val natural = paint.fontMetricsInt
            val spaced = Paint.FontMetricsInt()
            span.getSize(paint, result, separatorIndex, separatorIndex + 1, spaced)
            return spaced.descent - natural.descent
        }

        fun separatorAfter(text: String): Int = result.indexOf('\n', result.indexOf(text) + text.length)

        assertEquals(6, spacingAt(separatorAfter("First item")))
        assertEquals(20, spacingAt(separatorAfter("Nested item")))
        assertEquals(20, spacingAt(separatorAfter("After nested")))
    }

    @Test
    fun `render - nested and outer list spacingAfter stack at a shared ending`() {
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
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "After list", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """{"list":{"itemSpacing":6,"spacingAfter":20}}"""
        )
        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val separatorIndex = result.indexOf('\n', result.indexOf("Nested item") + "Nested item".length)
        val span = result.getSpans(
            separatorIndex,
            separatorIndex + 1,
            ParagraphSpacerSpan::class.java,
        ).single()
        val paint = Paint().apply { textSize = baseFontSize }
        val natural = paint.fontMetricsInt
        val spaced = Paint.FontMetricsInt()
        span.getSize(paint, result, separatorIndex, separatorIndex + 1, spaced)

        assertEquals(40, spaced.descent - natural.descent)
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
    fun `render - list close clears paragraph spacing when list spacing is unset`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 0},
            {"type": "textRun", "text": "After", "marks": []},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """{"paragraph":{"spacingAfter":14}}"""
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val separatorIndex = result.indexOf('\n', result.indexOf("Item") + "Item".length)
        val spacerSpans = result.getSpans(
            separatorIndex,
            separatorIndex + 1,
            ParagraphSpacerSpan::class.java,
        )

        assertTrue(spacerSpans.isEmpty())
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
    fun `render - list base indent multiplier can collapse top-level list indent`() {
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
              "list": { "indent": 32, "baseIndentMultiplier": 0 }
            }
            """.trimIndent()
        )

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme, 1f)
        val marginSpan = result.getSpans(0, result.length, LeadingMarginSpan.Standard::class.java).single()

        assertEquals(0, marginSpan.getLeadingMargin(true))
        assertEquals(LayoutConstants.LIST_MARKER_WIDTH.toInt(), marginSpan.getLeadingMargin(false))
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
