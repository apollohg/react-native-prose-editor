package com.apollohg.editor.viewer

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
class PreparedBlockSpacingTest {
    @Test
    fun `quote final direct paragraph drops only its bottom margin`() {
        val quote = ViewerContainerAncestor(1, "blockquote", 0, 0)
        val result = prepare(listOf(paragraph(quote)), """"paragraph":{"marginTop":3,"marginBottom":12,"paddingBottom":5,"borderBottomWidth":2},"blockquote":{"paddingTop":4,"paddingBottom":7,"marginBottom":9}""")
        val text = text(result, 0)
        val bounds = result.blocks.single().fragments.first { it.decorationBounds != null }.decorationBounds!!
        assertEquals(7, text.bounds.top)
        assertEquals(text.bounds.bottom + 5 + 2 + 7, bounds.bottom)
        assertEquals(bounds.bottom + 9, result.heightPx)
    }

    @Test
    fun `quote interior paragraph margins remain and collapse with their next sibling`() {
        val quote = ViewerContainerAncestor(1, "blockquote", 0, 1)
        val result = prepare(listOf(paragraph(quote), paragraph(quote)), """"paragraph":{"marginTop":8,"marginBottom":12,"paddingBottom":5},"blockquote":{"paddingTop":4,"paddingBottom":7}""")
        assertEquals(12, text(result, 0).bounds.top)
        assertEquals(5 + 12, text(result, 1).bounds.top - text(result, 0).bounds.bottom)
        val quoteBounds = result.blocks.last().fragments.first { it.decorationBounds != null }.decorationBounds!!
        assertEquals(text(result, 1).bounds.bottom + 5 + 7, quoteBounds.bottom)
    }

    @Test
    fun `quote does not trim a paragraph buried in its final list`() {
        val quote = ViewerContainerAncestor(1, "blockquote", 0, 1)
        val list = ViewerContainerAncestor(2, "bulletList", 1, 1)
        val item = ViewerContainerAncestor(3, "listItem", 1, 1)
        val result = prepare(listOf(paragraph(quote), paragraph(quote, list, item)), """"paragraph":{"marginBottom":12},"blockquote":{"paddingBottom":7},"bulletList":{"marginTop":5,"marginBottom":0},"listItem":{"marginBottom":0}""")
        assertEquals(12, text(result, 1).bounds.top - text(result, 0).bounds.bottom)
        val quoteBounds = result.blocks.last().fragments.first { it.decorationBounds != null }.decorationBounds!!
        assertEquals(text(result, 1).bounds.bottom + 12 + 7, quoteBounds.bottom)
    }

    @Test
    fun `root sibling margins collapse for positive mixed and negative values`() {
        for ((bottom, top, expected) in listOf(Triple(20, 8, 20), Triple(-9, 6, -3), Triple(6, -9, -3), Triple(-9, -4, -9))) {
            val result = prepare(listOf(paragraph(), paragraph()), """"paragraph":{"marginBottom":$bottom,"marginTop":$top,"paddingBottom":3,"paddingTop":2}""")
            assertEquals("bottom=$bottom top=$top", expected + 5, text(result, 1).bounds.top - text(result, 0).bounds.bottom)
        }
    }

    @Test
    fun `sibling container margins collapse while child margins and padding stay separate`() {
        val first = ViewerContainerAncestor(1, "blockquote", 0, 0)
        val second = ViewerContainerAncestor(2, "blockquote", 1, 1)
        val result = prepare(listOf(paragraph(first), paragraph(second)), """"paragraph":{"marginTop":3,"marginBottom":12,"paddingBottom":2},"blockquote":{"marginTop":10,"marginBottom":20,"paddingTop":7,"paddingBottom":5}""")
        assertEquals(2 + 5 + 20 + 7 + 3, text(result, 1).bounds.top - text(result, 0).bounds.bottom)
        val firstBounds = result.blocks.first().fragments.first { it.decorationBounds != null }.decorationBounds!!
        val secondBounds = result.blocks.last().fragments.first { it.decorationBounds != null }.decorationBounds!!
        assertEquals(20, secondBounds.top - firstBounds.bottom)
    }

    @Test
    fun `nested sibling containers collapse at their divergence inside the parent`() {
        val outer = ViewerContainerAncestor(1, "blockquote", 0, 1)
        val first = ViewerContainerAncestor(2, "blockquote", 0, 0)
        val second = ViewerContainerAncestor(3, "blockquote", 1, 1)
        val result = prepare(listOf(paragraph(outer, first), paragraph(outer, second)), """"paragraph":{"marginTop":3,"marginBottom":12},"blockquote":{"marginTop":10,"marginBottom":20,"paddingTop":7,"paddingBottom":5}""")
        assertEquals(10 + 7 + 10 + 7 + 3, text(result, 0).bounds.top)
        assertEquals(5 + 20 + 7 + 3, text(result, 1).bounds.top - text(result, 0).bounds.bottom)
        assertTrue(result.heightPx > text(result, 1).bounds.bottom)
    }

    @Test
    fun `leaf and container siblings collapse their own outer margins`() {
        val quote = ViewerContainerAncestor(1, "blockquote", 1, 1)
        val result = prepare(listOf(paragraph(), paragraph(quote), paragraph()), """"paragraph":{"marginTop":8,"marginBottom":30,"paddingTop":2,"paddingBottom":3},"blockquote":{"marginTop":20,"marginBottom":15,"paddingTop":7,"paddingBottom":5}""")
        assertEquals(3 + 30 + 7 + 8 + 2, text(result, 1).bounds.top - text(result, 0).bounds.bottom)
        assertEquals(3 + 5 + 15 + 2, text(result, 2).bounds.top - text(result, 1).bounds.bottom)
    }

    @Test
    fun `image siblings collapse margins while preserving image padding and borders`() {
        val image = ViewerBlock("image", 0, false, null, null, listOf(ViewerInline.Atom("image", 1, """{"src":"invalid-source","width":40,"height":20}""", "")))
        val result = prepare(listOf(paragraph(), image, paragraph()), """"paragraph":{"marginTop":8,"marginBottom":20,"paddingTop":2,"paddingBottom":3},"image":{"marginTop":10,"marginBottom":30,"paddingTop":4,"paddingBottom":6,"borderBottomWidth":2}""")
        val attachment = result.imageAttachments.single()
        assertEquals(3 + 20 + 4, attachment.bounds.top - text(result, 0).bounds.bottom)
        assertEquals(6 + 2 + 30 + 2, text(result, 2).bounds.top - attachment.bounds.bottom)
    }

    private fun paragraph(vararg containers: ViewerContainerAncestor) = ViewerBlock(
        "paragraph", containers.size, containers.any { it.nodeType == "blockquote" }, null, null,
        listOf(ViewerInline.Text("text", emptyList())), containers = containers.toList(),
    )

    private fun text(result: PreparedProseLayout, index: Int) = result.blocks[index].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }

    private fun prepare(blocks: List<ViewerBlock>, styles: String): PreparedProseLayout {
        val theme = PreparedProseTheme.resolve("""{"version":1,"styles":{"text":{"fontSize":17,"lineHeight":27},$styles}}""", 1f)
        val key = ProseLayoutKey("spacing", 300, "spacing", 0, 0, 0, 0, "spacing")
        return StaticLayoutAndroidProseLayoutEngine().prepare(ViewerDocument("spacing", blocks, false, 0), key, theme, 300, 1f, false)
    }
}
