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
internal class RenderBridgeOrderedListsTest : RenderBridgeTestFixture() {
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

    @Test
    fun `render - ProseMirror ordered list item`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "list_item", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertTrue(result.toString().contains("1. Item"))
    }

    @Test
    fun `ordered list marker formatter cycles schemes and formats boundaries`() {
        val theme = EditorOrderedListMarkerTheme.fromJson(
            JSONObject("""{"schemes":["decimal","lowerAlpha","lowerRoman"],"suffix":")"}""")
        )

        assertEquals("1)", OrderedListMarkerFormatter.label(1, 0, theme))
        assertEquals("z)", OrderedListMarkerFormatter.label(26, 1, theme))
        assertEquals("aa)", OrderedListMarkerFormatter.label(27, 1, theme))
        assertEquals("ix)", OrderedListMarkerFormatter.label(9, 2, theme))
        assertEquals("2)", OrderedListMarkerFormatter.label(2, 3, theme))
        assertEquals(
            "4000.",
            OrderedListMarkerFormatter.label(
                4_000,
                2,
                EditorOrderedListMarkerTheme(
                    listOf(EditorOrderedListNumberingScheme.LOWER_ROMAN),
                    "."
                )
            )
        )
        assertEquals(
            "0.",
            OrderedListMarkerFormatter.label(
                0,
                0,
                EditorOrderedListMarkerTheme(
                    listOf(EditorOrderedListNumberingScheme.UPPER_ALPHA),
                    "."
                )
            )
        )
        assertEquals(
            "4294967296.",
            OrderedListMarkerFormatter.label(
                4_294_967_296,
                0,
                EditorOrderedListMarkerTheme(
                    listOf(EditorOrderedListNumberingScheme.LOWER_ALPHA),
                    "."
                )
            )
        )
        val normalized = EditorOrderedListMarkerTheme.fromJson(
            JSONObject("""{"schemes":["unknown"],"suffix":"!"}""")
        )
        assertEquals("27.", OrderedListMarkerFormatter.label(27, 0, normalized))

        val uppercase = EditorOrderedListMarkerTheme.fromJson(
            JSONObject("""{"schemes":["upperAlpha","upperRoman"],"suffix":")"}""")
        )
        assertEquals("AA)", OrderedListMarkerFormatter.label(27, 0, uppercase))
        assertEquals("IX)", OrderedListMarkerFormatter.label(9, 1, uppercase))
        assertEquals("MMMCMXCIX)", OrderedListMarkerFormatter.label(3_999, 1, uppercase))
    }

    @Test
    fun `ordered marker theme normalizes missing empty and mixed schemes`() {
        val missing = EditorOrderedListMarkerTheme.fromJson(JSONObject())
        val empty = EditorOrderedListMarkerTheme.fromJson(JSONObject("""{"schemes":[]}"""))
        val mixed = EditorOrderedListMarkerTheme.fromJson(
            JSONObject("""{"schemes":["lowerAlpha",7,null,"unknown","upperRoman"]}""")
        )
        val malformed = EditorOrderedListMarkerTheme.fromJson(
            JSONObject("""{"schemes":[7,null,"unknown"]}""")
        )

        val defaultSchemes = listOf(
            EditorOrderedListNumberingScheme.DECIMAL,
            EditorOrderedListNumberingScheme.LOWER_ALPHA,
            EditorOrderedListNumberingScheme.LOWER_ROMAN,
        )
        assertEquals(defaultSchemes, missing?.schemes)
        assertEquals(defaultSchemes, empty?.schemes)
        assertEquals(
            listOf(
                EditorOrderedListNumberingScheme.LOWER_ALPHA,
                EditorOrderedListNumberingScheme.UPPER_ROMAN,
            ),
            mixed?.schemes,
        )
        assertEquals(defaultSchemes, malformed?.schemes)
    }

    @Test
    fun `render - absent ordered marker cycles default schemes by semantic depth`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Depth zero", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Depth one", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 2,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 3},
            {"type": "textRun", "text": "Depth two", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 3,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 4},
            {"type": "textRun", "text": "Depth three", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val rendered = RenderBridge.buildSpannable(
            json,
            baseFontSize,
            textColor,
            EditorTheme.fromJson("""{"list":{}}"""),
        )

        assertEquals(
            listOf("1.", "a.", "i.", "1."),
            rendered.getSpans(0, rendered.length, OrderedListMarkerSpan::class.java)
                .sortedBy { rendered.getSpanStart(it) }
                .map { it.label },
        )
        assertTrue(rendered.toString().contains("1. Depth zero"))
        assertTrue(rendered.toString().contains("1. Depth one"))
        assertTrue(rendered.toString().contains("1. Depth two"))
        assertTrue(rendered.toString().contains("1. Depth three"))
    }

    @Test
    fun `render - nested ordered marker uses semantic list depth without changing canonical text`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Outer item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Nested item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """{"list":{"orderedMarker":{"schemes":["decimal","lowerAlpha"],"suffix":")"}}}"""
        )

        val rendered = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)

        assertTrue(rendered.toString().contains("1. "))
        assertEquals(
            "a)",
            rendered.getSpans(0, rendered.length, OrderedListMarkerSpan::class.java)
                .last()
                .label
        )
    }

    @Test
    fun `render - missing ordered index keeps canonical and visual defaults aligned`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": true, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()
        val theme = EditorTheme.fromJson(
            """{"list":{"orderedMarker":{"schemes":["lowerAlpha"],"suffix":")"}}}"""
        )

        val rendered = RenderBridge.buildSpannable(json, baseFontSize, textColor, theme)

        assertTrue(rendered.toString().startsWith("1. "))
        assertEquals(
            "a)",
            rendered.getSpans(0, rendered.length, OrderedListMarkerSpan::class.java)
                .single()
                .label
        )
    }

    @Test
    fun `render - ordered list preserves exact u32 marker index`() {
        val maxJson = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 1,
             "listContext": {"ordered": true, "index": 4294967295, "total": 1, "start": 4294967295, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "Last item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val rendered = RenderBridge.buildSpannable(maxJson, baseFontSize, textColor).toString()
        assertTrue("u32::MAX marker must remain exact. Got: '$rendered'", rendered.contains("4294967295. "))

        for (malformedIndex in listOf<Any>(-1, 1.5, org.json.JSONObject.NULL, "1", 4_294_967_296L)) {
            val context = org.json.JSONObject()
                .put("ordered", true)
                .put("index", malformedIndex)
            assertEquals(
                "present malformed ordered-list index must be rejected before signed narrowing",
                "",
                RenderBridge.listMarkerString(context)
            )
        }
    }

    @Test
    fun `list marker - ordered`() {
        val ctx = org.json.JSONObject("""{"ordered": true, "index": 3}""")
        val marker = RenderBridge.listMarkerString(ctx)
        assertEquals("Ordered list item 3 should produce '3. '", "3. ", marker)
    }

    @Test
    fun `render - list markerGap sizes the bullet gap and the ordered marker gap`() {
        fun json(ordered: Boolean) = """
        [
            {"type": "blockStart", "nodeType": "listItem", "depth": 0,
             "listContext": {"ordered": $ordered, "index": 1, "total": 1, "start": 1, "isFirst": true, "isLast": true}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "Item", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        listOf(4f, 20f).forEach { gap ->
            val theme = EditorTheme.fromJson("""{"list": {"markerGap": $gap}}""")

            val unordered = RenderBridge.buildSpannable(json(false), baseFontSize, textColor, theme, 2f)
            val bullet = unordered.getSpans(0, unordered.length, CenteredBulletSpan::class.java).single()
            assertEquals(gap * 2f, bullet.textSideGapPx(0f), 0.01f)

            val orderedResult = RenderBridge.buildSpannable(json(true), baseFontSize, textColor, theme, 2f)
            val gapSpan = orderedResult.getSpans(0, orderedResult.length, MarkerGapSpan::class.java).single()
            assertEquals(
                kotlin.math.ceil(gap * 2f).toInt(),
                gapSpan.getSize(TextPaint(), orderedResult, 0, 1, null),
            )
        }
    }
}
