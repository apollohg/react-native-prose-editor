package com.apollohg.editor

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.TextPaint
import android.text.style.ReplacementSpan
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EditorDocumentLayoutTest {
    private fun paint() = TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 20f }
    private fun box(text: SpannableStringBuilder, start: Int, end: Int, edges: EditorEdges, depth: Int = 0): EditorBlockBoxSpan {
        return EditorBlockBoxSpan(EditorBoxStyle(padding = edges), EditorEdges(), depth).also {
            text.setSpan(it, start, end, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
    }

    @Test
    fun `empty styled paragraph retains box and text metrics without inserting characters`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"paragraph":{"fontSize":31,"lineHeight":50,"paddingTop":7,"paddingBottom":11,"paddingLeft":13,"paddingRight":41}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"blockEnd"}]""", 20f, android.graphics.Color.BLACK, theme, 1f)
        assertEquals("", rendered.toString())
        val layout = EditorDocumentLayout(rendered, paint(), 240)
        assertEquals(7, layout.textLineTop(0))
        assertEquals(50, layout.textLineBottom(0) - layout.textLineTop(0))
        assertEquals(68, layout.height)
        assertEquals(13f, layout.getPrimaryHorizontal(0), 0.01f)
        assertEquals(199f, layout.contentRight(0), 0.01f)
        val caret = CaretGeometry.verticalBounds(layout, 0, paint(), rendered)
        assertTrue(caret.bottom - caret.top > 30f)
        rendered.insert(0, "a")
        val composing = EditorDocumentLayout(rendered, paint(), 240)
        assertEquals(50, composing.textLineBottom(0) - composing.textLineTop(0))
        assertEquals(68, composing.height)
        val composedCaret = CaretGeometry.verticalBounds(composing, 1, paint(), rendered)
        assertEquals(caret.top, composedCaret.top, 0.01f)
        assertEquals(caret.bottom, composedCaret.bottom, 0.01f)
    }

    @Test
    fun `empty block styles do not leak into the following paragraph`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"h1":{"fontSize":31,"lineHeight":50,"paddingLeft":13,"paddingTop":7},"paragraph":{"fontSize":20}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"h1","depth":0},{"type":"blockEnd"},{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"body","marks":[]},{"type":"blockEnd"}]""", 20f, android.graphics.Color.BLACK, theme, 1f)
        assertEquals("\nbody", rendered.toString())
        val layout = EditorDocumentLayout(rendered, paint(), 240)
        assertEquals(13f, layout.getPrimaryHorizontal(0), 0.01f)
        assertEquals(0f, layout.getPrimaryHorizontal(1), 0.01f)
        assertEquals(50, layout.textLineBottom(0) - layout.textLineTop(0))
        assertEquals(layout.textLineBottom(0) + 10, layout.textLineTop(1))
    }

    @Test
    fun `empty paragraph placeholder uses paragraph content width`() {
        val editor = EditorEditText(org.robolectric.RuntimeEnvironment.getApplication())
        editor.placeholderText = "A placeholder that wraps inside the paragraph"
        editor.applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"paragraph":{"paddingTop":7,"paddingBottom":11,"paddingLeft":13,"paddingRight":41},"placeholder":{"fontSize":20}}}"""))
        editor.applyRenderJSON("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"blockEnd"}]""")
        val placeholder = editor.buildPlaceholderLayout(240)!!
        assertEquals(186, placeholder.width)
        assertEquals(placeholder.height + 18 + editor.compoundPaddingTop + editor.compoundPaddingBottom, editor.resolvePlaceholderHeightForAvailableWidth(240))
    }

    @Test
    fun `negative vertical margins hit the last painted overlapping line`() {
        val text = SpannableStringBuilder("a\nb\nc")
        text.setSpan(EditorBlockBoxSpan(EditorBoxStyle(margin = EditorEdges(top = -35f)), EditorEdges(), 0), 4, 5, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val layout = EditorDocumentLayout(text, paint(), 240)
        assertTrue(layout.textLineTop(2) < layout.textLineTop(1))
        assertEquals(2, layout.getLineForVertical(layout.textLineTop(2) + 2))
    }

    @Test
    fun `negative horizontal margin paints inside available host padding`() {
        val text = SpannableStringBuilder("x")
        text.setSpan(EditorBlockBoxSpan(EditorBoxStyle(margin = EditorEdges(left = -10f)), EditorEdges(), 0), 0, 1, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        text.setSpan(object : ReplacementSpan() {
            override fun getSize(paint: Paint, text: CharSequence, start: Int, end: Int, fm: Paint.FontMetricsInt?) = 10
            override fun draw(canvas: Canvas, text: CharSequence, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) {
                canvas.drawRect(x, top.toFloat(), x + 10, bottom.toFloat(), Paint().apply { color = android.graphics.Color.RED })
            }
        }, 0, 1, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val layout = EditorDocumentLayout(text, paint(), 200)
        val bitmap = android.graphics.Bitmap.createBitmap(240, 80, android.graphics.Bitmap.Config.ARGB_8888)
        Canvas(bitmap).apply { translate(20f, 0f); layout.draw(this) }
        assertEquals(android.graphics.Color.RED, bitmap.getPixel(15, layout.textLineTop(0) + 3))
    }

    @Test
    fun `list container styles surround the group and cascade into child text`() {
        val theme = EditorTheme.fromJson("""{"version":1,"styles":{"bulletList":{"paddingLeft":13,"paddingRight":41,"paddingTop":7,"paddingBottom":11,"fontSize":29,"backgroundColor":"#ff0000ff"},"paragraph":{"lineHeight":36}}}""")!!
        val rendered = RenderBridge.buildSpannable("""[{"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":false,"isFirst":true,"isLast":false}},{"type":"blockStart","nodeType":"paragraph","depth":1},{"type":"textRun","text":"first","marks":[]},{"type":"blockEnd"},{"type":"blockEnd"},{"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":false,"isFirst":false,"isLast":true}},{"type":"blockStart","nodeType":"paragraph","depth":1},{"type":"textRun","text":"second","marks":[]},{"type":"blockEnd"},{"type":"blockEnd"}]""", 20f, android.graphics.Color.BLACK, theme, 1f)
        val group = rendered.getSpans(0, rendered.length, EditorBlockBoxSpan::class.java).singleOrNull { it.box.backgroundColor == android.graphics.Color.RED }
        assertNotNull("The Rust render sequence omits explicit list container nodes", group)
        val first = rendered.indexOf("first")
        val styled = rendered.getSpans(first, first + 1, EditorResolvedTextSpan::class.java).last()
        assertEquals(29f, styled.style.fontSize!!, 0.01f)
        val layout = EditorDocumentLayout(rendered, paint(), 300)
        assertEquals(7, layout.textLineTop(0))
        assertEquals(0f, layout.boxBounds(group!!)!!.top, 0.01f)
        assertEquals(layout.height.toFloat(), layout.boxBounds(group)!!.bottom, 0.01f)
        assertEquals(layout.textLineBottom(layout.lineCount - 1) + 11 + theme.styleSheet!!.box("listItem").outerInset.bottom.toInt(), layout.height)
    }

    @Test
    fun `asymmetric physical insets wrap only their own paragraph`() {
        val text = SpannableStringBuilder("one two three four five six seven eight nine\nplain")
        box(text, 0, text.indexOf('\n'), EditorEdges(left = 17f, right = 91f, top = 11f, bottom = 13f))
        val layout = EditorDocumentLayout(text, paint(), 240)
        val last = layout.getLineForOffset(text.indexOf('\n') - 1)
        assertTrue(last >= 2)
        for (line in 0..last) {
            assertEquals(17f, layout.contentLeft(line), 0.01f)
            assertEquals(149f, layout.contentRight(line), 0.01f)
            assertTrue(layout.getLineRight(line) <= 149.1f)
        }
        assertEquals(11, layout.textLineTop(0))
        assertEquals(0f, layout.getPrimaryHorizontal(text.indexOf("plain")), 0.01f)
        assertEquals(layout.textLineBottom(last) + 13, layout.textLineTop(last + 1))
        assertEquals(text.toString(), layout.text.toString())
    }

    @Test
    fun `nested container padding is applied once around code newlines`() {
        val text = SpannableStringBuilder("a\nb\nc\nafter")
        val outer = box(text, 0, 5, EditorEdges(7f, 19f, 11f, 13f))
        val inner = box(text, 0, 5, EditorEdges(3f, 5f, 17f, 2f), 1)
        val layout = EditorDocumentLayout(text, paint(), 200)
        assertEquals(10, layout.textLineTop(0))
        assertEquals(layout.textLineBottom(0), layout.textLineTop(1))
        assertEquals(layout.textLineBottom(1), layout.textLineTop(2))
        assertEquals(layout.textLineBottom(2) + 28, layout.textLineTop(3))
        assertEquals(15f, layout.getPrimaryHorizontal(2), 0.01f)
        assertEquals(RectF(0f, 0f, 200f, layout.textLineTop(3).toFloat()), layout.boxBounds(outer))
        assertEquals(7f, layout.boxBounds(inner)!!.top, 0.01f)
    }

    @Test
    fun `global caret selection and hit testing follow fragments and bidi`() {
        val text = SpannableStringBuilder("abc\nאבג דהו\nend")
        box(text, 4, 11, EditorEdges(left = 23f, right = 61f))
        val layout = EditorDocumentLayout(text, paint(), 240)
        val line = layout.getLineForOffset(4)
        assertEquals(-1, layout.getParagraphDirection(line))
        assertEquals(179f, layout.getPrimaryHorizontal(4), 0.01f)
        for (offset in listOf(0, 1, 4, 5, 6, 12, 13)) {
            val current = layout.getLineForOffset(offset)
            assertEquals(offset, layout.getOffsetForHorizontal(current, layout.getPrimaryHorizontal(offset)))
            assertEquals(current, layout.getLineForVertical(layout.textLineTop(current)))
        }
        val path = Path()
        layout.getSelectionPath(4, 7, path)
        val bounds = RectF()
        path.computeBounds(bounds, true)
        assertEquals(layout.textLineTop(line).toFloat(), bounds.top, 0.01f)
        assertTrue(bounds.right <= 179.1f)
        val caret = Path()
        layout.getCursorPath(5, caret, text)
        caret.computeBounds(bounds, true)
        assertEquals(layout.getPrimaryHorizontal(5), bounds.left, 1f)
    }

    @Test
    fun `replacement span height and shifted paragraph offsets remain authoritative`() {
        val text = SpannableStringBuilder("x\n\uFFFC\nlast")
        val atom = object : ReplacementSpan() {
            override fun getSize(paint: Paint, text: CharSequence, start: Int, end: Int, fm: Paint.FontMetricsInt?): Int {
                fm?.apply { ascent = -83; top = -83; descent = 0; bottom = 0 }
                return 60
            }
            override fun draw(canvas: Canvas, text: CharSequence, start: Int, end: Int, x: Float, top: Int, y: Int, bottom: Int, paint: Paint) = Unit
        }
        text.setSpan(atom, 2, 3, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val first = EditorDocumentLayout(text, paint(), 200)
        assertEquals(83, first.textLineBottom(1) - first.textLineTop(1))
        text.insert(0, "prefix ")
        val second = EditorDocumentLayout(text, paint(), 200, previous = first)
        assertEquals(9, second.getLineStart(1))
        assertEquals(83, second.textLineBottom(1) - second.textLineTop(1))
        assertEquals(11, second.getLineStart(2))
        assertEquals(2, first.getLineStart(1))
        assertTrue(second.reusedFragmentCount >= 2)
    }

    @Test
    fun `mutable atom measurements invalidate cached shaping`() {
        val text = SpannableStringBuilder("\uFFFC")
        val atom = AtomBlockSpan("atom", "card", 0, 43, true, true)
        text.setSpan(atom, 0, 1, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val first = EditorDocumentLayout(text, paint(), 200)
        atom.reservedHeightPx = 97
        val second = EditorDocumentLayout(text, paint(), 200, previous = first)
        assertEquals(97, second.textLineBottom(0) - second.textLineTop(0))
        assertEquals(0, second.reusedFragmentCount)
    }

    @Test
    fun `physical alignment and justification respect both sides`() {
        val text = SpannableStringBuilder("אבג\nleft right centered\none two three four five six seven eight")
        box(text, 0, 3, EditorEdges(left = 21f, right = 63f))
        text.applyPhysicalTextAlignment("left", 0, 3)
        text.applyPhysicalTextAlignment("center", 4, 23)
        text.applyPhysicalTextAlignment("justify", 24, text.length)
        val layout = EditorDocumentLayout(text, paint(), 240)
        assertEquals(21f, layout.getLineLeft(0), 0.01f)
        assertEquals(120f, (layout.getLineLeft(1) + layout.getLineRight(1)) / 2, 0.01f)
        val justified = layout.getLineForOffset(24)
        assertEquals(240f, layout.getLineRight(justified), 0.1f)
    }

    @Test
    fun `unmounted measurement uses the same physical wrapping as document layout`() {
        val themeJson = """{"version":1,"styles":{"text":{"fontSize":20},"paragraph":{"paddingLeft":13,"paddingRight":109,"paddingTop":7,"paddingBottom":11}}}"""
        val json = """[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"one two three four five six seven eight nine ten eleven twelve","marks":[]},{"type":"blockEnd"}]"""
        val text = RenderBridge.buildSpannable(json, 20f, android.graphics.Color.BLACK, EditorTheme.fromJson(themeJson), 1f)
        val layout = EditorDocumentLayout(text, paint(), 240, includeFontPadding = true)
        assertEquals(layout.height.toFloat(), RenderBridge.measureHeight(json, themeJson, 240f, 1f), 0.01f)
    }

    @Test
    fun `font padding and line spacing match native layout once across paragraphs`() {
        val text = "first\nsecond\nthird"
        val native = android.text.StaticLayout.Builder.obtain(text, 0, text.length, paint(), 240)
            .setIncludePad(true).setLineSpacing(3f, 1.1f).build()
        val layout = EditorDocumentLayout(text, paint(), 240, includeFontPadding = true, spacingMultiplier = 1.1f, spacingAdd = 3f)
        assertEquals(native.height, layout.height)
        for (line in 0 until native.lineCount) {
            assertEquals(native.getLineBaseline(line), layout.getLineBaseline(line))
        }
    }

    @Test
    fun `selection excludes font padding removed between paragraphs`() {
        val layout = EditorDocumentLayout("first\nsecond\nthird", paint(), 240, includeFontPadding = true)
        val path = Path()
        layout.getSelectionPath(6, 12, path)
        val bounds = RectF()
        path.computeBounds(bounds, true)
        assertEquals(layout.textLineTop(1).toFloat(), bounds.top, 0.01f)
        assertEquals(layout.textLineBottom(1).toFloat(), bounds.bottom, 0.01f)
    }

    @Test
    fun `large document edits reuse shifted paragraphs with equivalent geometry`() {
        val text = SpannableStringBuilder((0 until 500).joinToString("\n") { "Paragraph $it has enough words to wrap at this bounded width." })
        val start = System.nanoTime()
        val first = EditorDocumentLayout(text, paint(), 260)
        val cold = System.nanoTime() - start
        text.insert(0, "Added ")
        val editedStart = System.nanoTime()
        val edited = EditorDocumentLayout(text, paint(), 260, previous = first)
        val incremental = System.nanoTime() - editedStart
        val fresh = EditorDocumentLayout(text, paint(), 260)
        assertEquals(499, edited.reusedFragmentCount)
        assertEquals(fresh.height, edited.height)
        assertEquals(fresh.lineCount, edited.lineCount)
        for (line in 0 until fresh.lineCount) {
            assertEquals(fresh.getLineStart(line), edited.getLineStart(line))
            assertEquals(fresh.getLineBaseline(line), edited.getLineBaseline(line))
        }
        println("EditorDocumentLayout 500 paragraphs/${text.length} UTF16: cold=${cold / 1_000_000.0}ms, edited=${incremental / 1_000_000.0}ms, reused=${edited.reusedFragmentCount}")
    }

    @Test
    @Config(sdk = [24])
    fun `empty paragraphs retain independent line positions on minimum SDK`() {
        val layout = EditorDocumentLayout("\n\n", paint(), 200)
        assertEquals(3, layout.lineCount)
        assertEquals(0, layout.getLineStart(0))
        assertEquals(1, layout.getLineStart(1))
        assertEquals(2, layout.getLineStart(2))
        assertTrue(layout.height > 0)
        assertTrue(layout.getLineBaseline(1) > layout.getLineBaseline(0))
    }
}
