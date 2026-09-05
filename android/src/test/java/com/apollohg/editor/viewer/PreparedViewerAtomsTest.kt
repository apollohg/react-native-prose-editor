package com.apollohg.editor.viewer

import org.json.JSONArray
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedViewerAtomsTest {
    private fun prepare(measurements: String = "{}", width: Int = 600, isBlockAtom: Boolean = true, registeredType: String = "card", quoted: Boolean = false, listed: Boolean = false): PreparedProseLayout {
        val theme = PreparedProseTheme.resolve("""{"viewerAtoms":{"generation":"g1","revision":"r2","nodeTypes":["$registeredType"],"estimatedHeights":{"card":40},"measurements":$measurements}}""", 2f)
        val document = ViewerDocument("atoms", listOf(ViewerBlock("card", 0, quoted, if (listed) ViewerListContext(false, 1, null, false, true) else null, if (listed) ViewerListItemBoundary(1, 0, true, true) else null, listOf(ViewerInline.Atom("card", 7, "{\"id\":1}", "fallback")), isBlockAtom = isBlockAtom)), false, 128)
        val key = ProseLayoutKey("atoms", width, "theme", 0, 0, 0, 0, "g")
        return StaticLayoutAndroidProseLayoutEngine().prepare(document, key, theme, width, 2f, false)
    }

    @Test fun `registered blocks reserve full width and estimated height without native fallback`() {
        val layout = prepare()
        assertEquals(80, layout.heightPx)
        assertTrue(layout.blocks.flatMap { it.fragments }.isEmpty())
        assertTrue(layout.accessibilityNodes.isEmpty())
    }

    @Test fun `measurements only apply at matching logical width and allow zero height`() {
        assertEquals(144, prepare("""{"7":{"width":300,"height":72}}""").heightPx)
        assertEquals(80, prepare("""{"7":{"width":299,"height":72}}""").heightPx)
        assertEquals(0, prepare("""{"7":{"width":300,"height":0}}""").heightPx)
    }
    @Test fun `published geometry converts pixels and content origins to logical units`() {
        val layout = prepare()
        val atom = JSONArray(layout.atomsJson(2f, 12, 20)).getJSONObject(0)
        assertEquals(6.0, atom.getDouble("x"), 0.0)
        assertEquals(10.0, atom.getDouble("y"), 0.0)
        assertEquals(300.0, atom.getDouble("width"), 0.0)
        assertEquals(40.0, atom.getDouble("height"), 0.0)
        assertEquals(7L, atom.getLong("docPos"))
        assertEquals("{\"id\":1}", atom.getString("attrsJson"))
        assertTrue(layout.retainedBytes >= layout.viewerAtoms.sumOf { it.retainedBytes })
    }

    @Test fun `unregistered blocks and inline atoms retain native fallback`() {
        listOf(prepare(registeredType = "other"), prepare(isBlockAtom = false)).forEach { layout ->
            assertTrue(layout.viewerAtoms.isEmpty())
            assertTrue(layout.fragmentKinds.contains(PreparedProseFragmentKind.ATOM))
        }
    }

    @Test fun `quoted atom slots preserve the border and reduced content width`() {
        val layout = prepare(quoted = true)
        assertTrue(layout.fragmentKinds.contains(PreparedProseFragmentKind.BORDER))
        assertEquals(542, layout.viewerAtoms.single().bounds.width())
        assertEquals(58, layout.viewerAtoms.single().bounds.left)
        assertFalse(layout.fragmentKinds.contains(PreparedProseFragmentKind.ATOM))
    }

    @Test fun `list atom slots preserve marker gutters`() {
        val layout = prepare(listed = true)
        assertTrue(layout.fragmentKinds.contains(PreparedProseFragmentKind.MARKER))
        val bounds = layout.viewerAtoms.single().bounds
        assertTrue(bounds.left > 0)
        assertEquals(600, bounds.right)
        assertEquals(80, bounds.height())
    }

    @Test fun `empty atom geometry is published as an empty JSON array`() {
        assertEquals("[]", prepare(registeredType = "other").atomsJson(2f, 0, 0))
    }

}
