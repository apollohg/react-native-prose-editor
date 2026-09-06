package com.apollohg.editor.viewer

import com.apollohg.editor.CodeHighlightBlock
import com.apollohg.editor.CodeHighlightRange
import com.apollohg.editor.EditorCodeHighlightSpan
import com.apollohg.editor.NativeCodeHighlightingConfig
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedCodeHighlightingTest {
    @Test
    fun `remount of cached plain artifact requests presentation rebuild`() {
        val config = NativeCodeHighlightingConfig("remount-test", "one")
        val code = CodeHighlightBlock(0, "remount", "rust")
        val key = ProseLayoutKey("remount", 200, "remount", 0, 0, 0, 0, "remount")
        val plain = PreparedProseLayout(key, 200, 30, emptyList(), retainedBytes = 0, codeHighlighting = config, codeHighlightBlocks = listOf(code))
        ViewerCodeHighlightCache.store(config, code, listOf(CodeHighlightRange(0, 7, 0xff0000ff, 1)))
        val controller = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java).setup()
        val view = PreparedProseDrawingView(controller.get())
        var rebuilds = 0
        view.onCodeHighlightsReady = { rebuilds++ }
        view.install(plain)
        controller.get().setContentView(view)
        assertEquals(1, rebuilds)
        view.install(plain)
        assertEquals(1, rebuilds)
        controller.pause().stop().destroy()
    }

    @Test
    fun `cached syntax rebuilds a new artifact and respects UTF16 and theme identity`() {
        val config = NativeCodeHighlightingConfig("prepared-test", "one")
        val code = CodeHighlightBlock(0, "😀x", "rust")
        val block = ViewerBlock("codeBlock", 0, false, null, null, listOf(ViewerInline.Text(code.text, emptyList())), language = code.language)
        val document = ViewerDocument("syntax", listOf(block), false, 0)
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{}}""", 1f).copy(codeHighlighting = config)
        val key = ProseLayoutKey("syntax", 200, "syntax", 0, 0, 0, 0, "syntax")
        fun prepare(theme: PreparedProseTheme) = StaticLayoutAndroidProseLayoutEngine().prepare(document, key, theme, 200, 1f, false)
        fun text(layout: PreparedProseLayout) = layout.blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.layout!!.text as android.text.Spanned
        val plain = text(prepare(theme))
        ViewerCodeHighlightCache.store(config, code, listOf(CodeHighlightRange(2, 1, 0xff0000ff, 1)))
        val highlighted = text(prepare(theme))
        assertTrue(plain.getSpans(0, plain.length, EditorCodeHighlightSpan::class.java).isEmpty())
        val syntax = highlighted.getSpans(0, highlighted.length, EditorCodeHighlightSpan::class.java).single()
        assertEquals(2, highlighted.getSpanStart(syntax))
        assertTrue(text(prepare(theme.copy(codeHighlighting = config.copy(theme = "two")))).getSpans(0, 3, EditorCodeHighlightSpan::class.java).isEmpty())
    }
}
