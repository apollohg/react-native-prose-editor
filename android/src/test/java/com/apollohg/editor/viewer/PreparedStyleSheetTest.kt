package com.apollohg.editor.viewer

import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedStyleSheetTest {
    @Test
    fun `inline explicit line height reaches viewer measurement`() {
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"text":{"lineHeight":20},"bold":{"lineHeight":40}}}""", 1f)
        val document = ViewerDocument("line", listOf(ViewerBlock("paragraph", 0, false, null, null, listOf(ViewerInline.Text("bold", listOf(uniffi.editor_core.FfiViewerMark("bold", "{}")))))), false, 0)
        val key = ProseLayoutKey("line", 200, "line", 0, 0, 0, 0, "line")
        val result = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, theme, 200, 1f, false)
        assertEquals(40, result.heightPx)
    }

    @Test
    fun `physical alignment remains left for RTL paragraphs`() {
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"paragraph":{"textAlign":"left"}}}""", 1f)
        val document = ViewerDocument("rtl", listOf(ViewerBlock("paragraph", 0, false, null, null, listOf(ViewerInline.Text("שלום", emptyList())))), false, 0)
        val key = ProseLayoutKey("rtl", 200, "rtl", 0, 0, 0, 0, "rtl")
        val result = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, theme, 200, 1f, false)
        val layout = result.blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.layout!!
        assertEquals(0f, layout.getLineLeft(0), .01f)
    }

    @Test
    fun `stylesheet retains document foreground background and font marks`() {
        val marks = listOf(
            uniffi.editor_core.FfiViewerMark("textColor", """{"color":"#ff0000"}"""),
            uniffi.editor_core.FfiViewerMark("highlight", """{"color":"#ffff00"}"""),
            uniffi.editor_core.FfiViewerMark("textStyle", """{"fontFamily":"monospace","fontSize":29}"""),
        )
        val document = ViewerDocument("marks", listOf(ViewerBlock("paragraph", 0, false, null, null, listOf(ViewerInline.Text("styled", marks)))), false, 0)
        val key = ProseLayoutKey("marks", 200, "marks", 0, 0, 0, 0, "marks")
        val result = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, PreparedProseTheme.resolve("""{"version":1,"styles":{"paragraph":{"marginBottom":4}}}""", 1f), 200, 1f, false)
        val text = result.blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.layout!!.text as android.text.Spanned
        val style = text.getSpans(0, text.length, com.apollohg.editor.EditorResolvedTextSpan::class.java).single().style
        assertEquals(android.graphics.Color.RED, style.color)
        assertEquals(android.graphics.Color.YELLOW, style.backgroundColor)
        assertEquals(29f, style.fontSize)
        assertEquals("monospace", style.fontFamily)
    }

    @Test
    fun `styled image preserves declared size within available width`() {
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"image":{"borderLeftWidth":4,"borderRightWidth":2,"paddingTop":3}}}""", 1f)
        val block = ViewerBlock("image", 0, false, null, null, listOf(ViewerInline.Atom("image", 1, """{"src":"image.png","width":40,"height":20}""", "")))
        val key = ProseLayoutKey("test", 200, "test", 0, 0, 0, 0, "test")
        val result = StaticLayoutAndroidProseLayoutEngine().prepare(ViewerDocument("test", listOf(block), false, 0), key, theme, 200, 1f, false)
        assertEquals(40, result.imageAttachments.single().bounds.width())
        assertEquals(20, result.imageAttachments.single().bounds.height())
        assertEquals(23, result.heightPx)
    }

    @Test
    fun `compiler retains separate nested quote and list containers`() {
        val source = """{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"blockquote","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}]}]}]}]}"""
        val document = compileWithRust(ProseViewerRequest(com.apollohg.editor.ProseViewerSource.Json(source), com.apollohg.editor.ProseViewerConfiguration(configJson = """{"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"blockquote","content":"block+","group":"block","role":"block"},{"name":"bulletList","content":"listItem+","group":"block","role":"list"},{"name":"listItem","content":"paragraph block*","role":"listItem"},{"name":"text","group":"inline","role":"text"}]},"initialization":{"type":"localEmpty"}}""")))
        assertEquals(listOf("blockquote", "blockquote", "bulletList", "listItem"), document.blocks.last().containers.map { it.nodeType })
        assertEquals(1, document.blocks.first().containers.single().lastLeaf)
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"blockquote":{"paddingTop":4,"paddingBottom":5,"backgroundColor":"#ff0000ff"}}}""", 1f)
        val key = ProseLayoutKey("test", 200, "test", 0, 0, 0, 0, "test")
        val result = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, theme, 200, 1f, false)
        val first = result.blocks.first().fragments.first { it.decorationBounds != null }
        val last = result.blocks.last().fragments.first { it.decorationBounds != null }
        assertEquals(first.decorationBounds, last.decorationBounds)
        assertTrue(first.bounds.bottom <= last.bounds.top)
    }
    @Test
    fun `paragraph box reserves asymmetric geometry in viewer`() {
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"paragraph":{"paddingTop":8,"paddingRight":19,"paddingBottom":10,"paddingLeft":7,"borderTopWidth":2,"borderRightWidth":3,"marginTop":3,"marginBottom":5}}}""", 1f)
        val document = ViewerDocument("test", listOf(ViewerBlock("paragraph", 0, false, null, null, listOf(ViewerInline.Text("box", emptyList())))), false, 0)
        val key = ProseLayoutKey("test", 200, "test", 0, 0, 0, 0, "test")
        val result = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, theme, 200, 1f, false)
        val text = result.blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.TEXT }
        assertEquals(7, text.bounds.left)
        assertEquals(13, text.bounds.top)
        assertEquals(171, text.layout!!.width)
        assertEquals(text.bounds.bottom + 15, result.heightPx)
    }
}
