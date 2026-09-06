package com.apollohg.editor

import android.graphics.Color
import android.graphics.Paint
import android.text.TextPaint
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EditorBlockSpacingTest {
    @Test
    fun `quote without bottom padding ends at its final text line`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"paragraph":{"lineHeight":27,"marginBottom":12},"blockquote":{"borderLeftWidth":2,"paddingLeft":12}}}""")!!
        val text = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"blockquote","depth":0},{"type":"blockStart","nodeType":"paragraph","depth":1},{"type":"textRun","text":"Quoted text","marks":[]},{"type":"blockEnd"},{"type":"blockEnd"}]""", 17f, Color.BLACK, theme, 1f)
        val layout = EditorDocumentLayout(text, TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 17f }, 300)
        val quote = text.getSpans(0, text.length, EditorBlockBoxSpan::class.java).single { it.depth == 0 }
        assertEquals(layout.textLineBottom(0).toFloat(), layout.boxBounds(quote)!!.bottom, .01f)
    }

    @Test
    fun `quote keeps interior spacing and explicit bottom padding`() {
        val (text, layout) = render(quote(paragraph("one") + paragraph("two")), """"paragraph":{"lineHeight":27,"marginBottom":12,"paddingBottom":3},"blockquote":{"paddingBottom":7,"borderBottomWidth":2}""")
        val quote = text.getSpans(0, text.length, EditorBlockBoxSpan::class.java).single { it.depth == 0 }
        assertEquals(15, layout.textLineTop(1) - layout.textLineBottom(0))
        assertEquals(12f, layout.boxBounds(quote)!!.bottom - layout.textLineBottom(1), .01f)
    }

    @Test
    fun `standalone paragraph retains trailing margin`() {
        val (_, layout) = render(paragraph("plain"), """"paragraph":{"lineHeight":27,"marginBottom":12}""")
        assertEquals(12, layout.height - layout.textLineBottom(0))
    }

    @Test
    fun `adjoining sibling margins collapse for positive negative and mixed values`() {
        for ((bottom, top, expected) in listOf(Triple(12, 8, 12), Triple(-12, -8, -12), Triple(12, -8, 4))) {
            val (_, layout) = render(paragraph("one") + paragraph("two", "h2"), """"paragraph":{"lineHeight":27,"marginBottom":$bottom},"h2":{"lineHeight":27,"marginTop":$top,"marginBottom":0}""")
            assertEquals("margins $bottom and $top", expected, layout.textLineTop(1) - layout.textLineBottom(0))
        }
    }

    @Test
    fun `quote outer margin collapses with next sibling without consuming padding`() {
        val (_, layout) = render(quote(paragraph("quote")) + paragraph("after", "h2"), """"paragraph":{"lineHeight":27,"marginBottom":12},"blockquote":{"paddingBottom":7,"marginBottom":12},"h2":{"lineHeight":27,"marginTop":8,"marginBottom":0,"paddingTop":5}""")
        assertEquals(24, layout.textLineTop(1) - layout.textLineBottom(0))
    }

    @Test
    fun `sibling paragraph margins collapse inside quote`() {
        val (_, layout) = render(quote(paragraph("one") + paragraph("two")), """"paragraph":{"lineHeight":27,"marginTop":8,"marginBottom":12},"blockquote":{"paddingTop":3}""")
        assertEquals(11, layout.textLineTop(0))
        assertEquals(12, layout.textLineTop(1) - layout.textLineBottom(0))
    }

    @Test
    fun `image replacement margins collapse with adjacent paragraphs`() {
        val image = """{"type":"voidBlock","nodeType":"image","attrs":{"src":"invalid-source","width":40,"height":20}},"""
        val (text, layout) = render(paragraph("before") + image + paragraph("after"), """"paragraph":{"lineHeight":27,"marginTop":8,"marginBottom":12},"image":{"marginTop":8,"marginBottom":12}""")
        val span = text.getSpans(0, text.length, BlockImageSpan::class.java).single()
        try {
            val bounds = layout.imageBounds(span)!!
            assertEquals(12f, bounds.top - layout.textLineBottom(0), .01f)
            assertEquals(12f, layout.textLineTop(layout.lineCount - 1) - bounds.bottom, .01f)
        } finally {
            span.close()
        }
    }

    @Test
    fun `quote keeps final paragraph margin when a following image is present`() {
        val image = """{"type":"voidBlock","nodeType":"image","attrs":{"src":"invalid-source","width":40,"height":20}},"""
        val (text, layout) = render(quote(paragraph("before") + image), """"paragraph":{"lineHeight":27,"marginBottom":12},"image":{"marginTop":8,"marginBottom":5}""")
        val span = text.getSpans(0, text.length, BlockImageSpan::class.java).single()
        try {
            val bounds = layout.imageBounds(span)!!
            assertEquals(12f, bounds.top - layout.textLineBottom(0), .01f)
            assertEquals(5f, layout.height - bounds.bottom, .01f)
        } finally {
            span.close()
        }
    }

    private fun paragraph(value: String, type: String = "paragraph") = """{"type":"blockStart","nodeType":"$type"},{"type":"textRun","text":"$value","marks":[]},{"type":"blockEnd"},"""

    private fun quote(children: String) = """{"type":"blockStart","nodeType":"blockquote"},$children{"type":"blockEnd"},"""

    private fun render(elements: String, styles: String): Pair<android.text.SpannableStringBuilder, EditorDocumentLayout> {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{$styles}}""")!!
        val text = RenderBridge.buildSpannable("[${elements.trimEnd(',')}]", 17f, Color.BLACK, theme, 1f)
        return text to EditorDocumentLayout(text, TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 17f }, 300)
    }
}
