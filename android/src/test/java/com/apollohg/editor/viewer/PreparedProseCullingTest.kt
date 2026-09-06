package com.apollohg.editor.viewer

import android.graphics.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PreparedProseCullingTest {
    @Test
    fun `nonmonotonic tops retain later overlapping blocks in paint order`() {
        val layout = withBounds(Rect(0, 0, 100, 27), Rect(0, 100, 100, 127), Rect(0, 10, 100, 37))
        assertEquals(listOf(0, 2), visited(layout, Rect(0, 0, 100, 20)))
    }

    @Test
    fun `nonmonotonic bottoms retain earlier tall blocks`() {
        val layout = withBounds(Rect(0, 100, 100, 200), Rect(0, 0, 100, 27))
        assertEquals(listOf(0), visited(layout, Rect(0, 150, 100, 180)))
    }

    @Test
    fun `negative top margins keep overlapping text inside culling bounds`() {
        val layout = prepared(""""h2":{"fontSize":17,"marginTop":100,"marginBottom":0},"h3":{"fontSize":17,"marginTop":-130,"marginBottom":0}""")
        val text = layout.blocks.map { block -> block.fragments.single { it.kind == PreparedProseFragmentKind.TEXT } }
        assertTrue(text[2].bounds.top < text[0].bounds.bottom)
        assertVisibleText(layout, Rect(0, 0, 300, text[0].bounds.bottom))
    }

    @Test
    fun `negative bottom margins do not cull the preceding visible text`() {
        val layout = prepared(""""paragraph":{"marginBottom":-80},"h2":{"fontSize":17,"marginTop":0,"marginBottom":0},"h3":{"fontSize":17,"marginTop":0,"marginBottom":0}""")
        val firstText = layout.blocks.first().fragments.single { it.kind == PreparedProseFragmentKind.TEXT }
        assertVisibleText(layout, Rect(0, firstText.bounds.top, 300, firstText.bounds.bottom))
    }

    private fun assertVisibleText(layout: PreparedProseLayout, clip: Rect) {
        val expected = layout.blocks.flatMap { it.fragments }.filter { it.kind == PreparedProseFragmentKind.TEXT && Rect.intersects(it.bounds, clip) }
        val actual = mutableListOf<PreparedProseFragment>()
        layout.forEachFragmentIntersecting(clip) { if (it.kind == PreparedProseFragmentKind.TEXT) actual += it }
        assertTrue(expected.isNotEmpty())
        assertEquals(expected, actual)
    }

    private fun visited(layout: PreparedProseLayout, clip: Rect): List<Int> = buildList {
        layout.forEachBlockIntersecting(clip) { add(layout.blocks.indexOf(it)) }
    }

    private fun withBounds(vararg bounds: Rect) = PreparedProseLayout(
        key(), 300, 250, bounds.map { PreparedProseBlock(emptyList(), it) }, retainedBytes = 0,
    )

    private fun prepared(styles: String): PreparedProseLayout {
        val blocks = listOf("paragraph", "h2", "h3").map { ViewerBlock(it, 0, false, null, null, listOf(ViewerInline.Text("text", emptyList()))) }
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"text":{"fontSize":17,"lineHeight":27},$styles}}""", 1f)
        return StaticLayoutAndroidProseLayoutEngine().prepare(ViewerDocument("overlap", blocks, false, 0), key(), theme, 300, 1f, false)
    }

    private fun key() = ProseLayoutKey("overlap", 300, "overlap", 0, 0, 0, 0, "overlap")
}
