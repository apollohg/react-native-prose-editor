package com.apollohg.editor

import android.graphics.Color
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorStyleSheetTest {
    @Test
    fun `placeholder measures explicit line height and typography`() {
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        editor.placeholderText = "Placeholder"
        editor.applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"placeholder":{"fontSize":18,"lineHeight":40,"fontWeight":"700","textDecorationLine":"underline"}}}"""))
        val layout = requireNotNull(editor.buildPlaceholderLayout(200))
        assertEquals(40, layout.height)
        assertTrue(layout.paint.isUnderlineText)
    }

    @Test
    fun `horizontal rule keeps styled margins and borders in line metrics`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"horizontalRule":{"height":2,"marginTop":3,"marginBottom":5,"borderTopWidth":4}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"voidBlock","nodeType":"horizontalRule"}]""", 17f, Color.BLACK, theme, 1f)
        val box = rendered.getSpans(0, rendered.length, EditorBlockBoxSpan::class.java).singleOrNull()
        assertNotNull(box)
        assertEquals(7f, box!!.box.outerInset.top)
        assertEquals(5f, box.box.outerInset.bottom)
    }
    @Test
    fun `theme only changes preserve composing text and selection`() {
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        editor.applyRenderJSON("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"compose","marks":[]},{"type":"blockEnd"}]""")
        editor.setSelection(3)
        android.view.inputmethod.BaseInputConnection.setComposingSpans(editor.editableText)
        editor.applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"paragraph":{"color":"#ff0000ff"}}}"""))
        assertEquals("compose", editor.text.toString())
        assertEquals(3, editor.selectionStart)
        assertEquals(0, android.view.inputmethod.BaseInputConnection.getComposingSpanStart(editor.editableText))
    }
    @Test
    fun `container box starts after preceding paragraph separator`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"blockquote":{"backgroundColor":"#ff0000ff","paddingTop":8}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"before","marks":[]},{"type":"blockEnd"},{"type":"blockStart","nodeType":"blockquote","depth":0},{"type":"blockStart","nodeType":"paragraph","depth":1},{"type":"textRun","text":"quote","marks":[]},{"type":"blockEnd"},{"type":"blockEnd"}]""", 17f, Color.BLACK, theme, 1f)
        val box = rendered.getSpans(0, rendered.length, EditorBlockBoxSpan::class.java).single { it.box.backgroundColor == Color.RED }
        assertEquals(rendered.indexOf("quote"), rendered.getSpanStart(box))
    }
    @Test
    fun `theme updates reuse image ownership and clear existing boxes`() {
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        editor.applyRenderJSON("""[{"type":"voidBlock","nodeType":"image","attrs":{"src":"invalid-source","width":40,"height":20}}]""")
        val initial = editor.text!!.getSpans(0, 1, BlockImageSpan::class.java).single()
        editor.applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"image":{"borderTopLeftRadius":8}}}"""))
        assertSame(initial, editor.text!!.getSpans(0, 1, BlockImageSpan::class.java).single())
        assertEquals(8f, initial.imageStyle?.box?.corners?.topLeft)
        editor.applyTheme(null)
        assertNull(initial.imageStyle)
        initial.close()
    }
    @Test
    fun `task checkbox size reserves its actual marker width`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"taskCheckbox":{"size":28,"gap":9,"checked":{"backgroundColor":"#ff0000ff"}}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"taskItem","depth":0,"listContext":{"kind":"task","checked":true,"isFirst":true,"isLast":true}},{"type":"blockStart","nodeType":"paragraph","depth":1},{"type":"textRun","text":"task","marks":[]},{"type":"blockEnd"},{"type":"blockEnd"}]""", 17f, Color.BLACK, theme, 1f)
        val marker = rendered.getSpans(0, 1, android.text.style.ReplacementSpan::class.java).singleOrNull()
        assertNotNull(marker)
        assertEquals(28, marker!!.getSize(android.graphics.Paint(), rendered, 0, 1, null))
    }
    @Test
    fun `mention rich override keeps inherited sides while replacing explicit typography`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"mention":{"fontSize":22,"borderLeftWidth":4,"borderRightWidth":3,"color":"#ff0000ff"}},"mentions":{"node":{"style":{"fontSize":19,"borderRightWidth":0}}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"opaqueInlineAtom","nodeType":"mention","label":"@Ada","docPos":1},{"type":"blockEnd"}]""", 17f, Color.BLACK, theme, 1f)
        val span = rendered.getSpans(0, 4, android.text.style.ReplacementSpan::class.java).singleOrNull()
        assertNotNull(span)
        val paint = android.text.TextPaint().apply { textSize = 19f }
        assertTrue(span!!.getSize(paint, rendered, 0, 4, null) >= paint.measureText("@Ada") + 4)
        val metrics = android.graphics.Paint.FontMetricsInt()
        EditorMentionSpan(EditorElementStyle(EditorTextStyle(fontSize = 19f, lineHeight = 40f), EditorBoxStyle()), 1f).getSize(paint, "Ada", 0, 3, metrics)
        assertEquals(40, metrics.descent - metrics.ascent)
    }
    @Test
    fun `image border padding and margins reserve replacement geometry`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"image":{"paddingLeft":3,"paddingTop":2,"borderRightWidth":4,"marginBottom":5}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"voidBlock","nodeType":"image","attrs":{"src":"invalid-source","width":40,"height":20}}]""", 17f, Color.BLACK, theme, 1f)
        val span = rendered.getSpans(0, 1, BlockImageSpan::class.java).single()
        val metrics = android.graphics.Paint.FontMetricsInt()
        assertEquals(47, span.getSize(android.graphics.Paint(), rendered, 0, 1, metrics))
        assertEquals(27, metrics.descent - metrics.ascent)
        span.close()
    }

    @Test
    fun `content border participates in host padding and clearing`() {
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        editor.applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"content":{"borderLeftWidth":3,"paddingLeft":7,"paddingTop":5,"borderTopWidth":2}}}"""))
        val density = editor.resources.displayMetrics.density
        assertEquals((10 * density).toInt(), editor.paddingLeft)
        assertEquals((7 * density).toInt(), editor.paddingTop)
        editor.applyTheme(null)
        assertEquals(0, editor.paddingLeft)
    }
    @Test
    fun `inline explicit normal and none clear semantic marks in fixed order`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"bold":{"color":"#ff0000ff"},"link":{"fontWeight":"normal","textDecorationLine":"none","letterSpacing":2}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"link","marks":["bold",{"type":"link","href":"https://example.com"}]},{"type":"blockEnd"}]""", 17f, Color.BLACK, theme, 1f)
        val paint = android.text.TextPaint()
        rendered.getSpans(0, 4, android.text.style.CharacterStyle::class.java).forEach { it.updateDrawState(paint) }
        assertFalse(paint.typeface?.isBold == true)
        assertFalse(paint.isUnderlineText)
        assertEquals(2f / 17f, paint.letterSpacing, 0.001f)
    }

    @Test
    fun `paragraph vertical box space is included in layout`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"paragraph":{"paddingTop":8,"paddingBottom":10,"borderTopWidth":2,"marginTop":3,"marginBottom":5}}}""")!!
        fun height(value: EditorTheme?): Int {
            val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"box","marks":[]},{"type":"blockEnd"}]""", 17f, Color.BLACK, value, 1f)
            return android.text.StaticLayout.Builder.obtain(rendered, 0, rendered.length, android.text.TextPaint().apply { textSize = 17f }, 200).setIncludePad(false).build().height
        }
        assertEquals(28, height(theme) - height(EditorTheme.fromJson("""{"version":1,"styles":{}}""")))
    }
    @Test
    fun `versioned paragraph inherits base typography and portable alpha`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"text":{"fontSize":19,"lineHeight":28,"color":"#11223380"},"paragraph":{"marginBottom":12}}}""")!!
        val paragraph = theme.effectiveTextStyle("paragraph")
        assertEquals(19f, paragraph.fontSize)
        assertEquals(28f, paragraph.lineHeight)
        assertEquals(Color.argb(128, 17, 34, 51), paragraph.color)
    }

    @Test
    fun `heading semantic size precedes ancestor overrides`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"text":{"fontSize":19},"blockquote":{"fontSize":23},"h1":{"color":"#ff0000ff"}}}""")!!
        assertEquals(32f, theme.effectiveTextStyle("h1").fontSize)
        assertEquals(23f, theme.effectiveTextStyle("h1", true).fontSize)
    }

    @Test
    fun `unknown stylesheet versions are rejected`() {
        assertNull(EditorTheme.fromJson("""{"version":2,"styles":{}}"""))
    }
}
