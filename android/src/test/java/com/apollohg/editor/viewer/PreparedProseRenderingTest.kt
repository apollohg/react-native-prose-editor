package com.apollohg.editor.viewer

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StyleSpan
import android.text.style.UnderlineSpan
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerSource
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * Compiler-backed geometry fixtures for the complete Android prepared renderer.
 *
 * These tests intentionally exercise the same semantic source corpus as the
 * final iOS renderer. They are authored before the Android renderer and are
 * deferred, unexecuted, to the Task 10 validation milestone.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedProseRenderingTest {
    @Test
    fun `compiler backed structural fixtures preserve every block atom and inherited context`() {
        Fixture.structural.forEach { fixture ->
            val document = compile(fixture)
            val layout = prepare(document)

            assertTrue(fixture.name, fixture.expectedKinds.all { it in layout.fragmentKinds })
            assertTrue(fixture.name, fixture.assertDocument(document))
        }
    }

    @Test
    fun `every compiler fixture has deterministic contained exact geometry`() {
        Fixture.all.forEach { fixture ->
            val document = compile(fixture)
            val first = prepare(document)
            val second = prepare(document)

            assertPreparedLayoutsEqual(first, second, fixture.name)
            assertTrue(fixture.name, fixture.expectedKinds.all { it in first.fragmentKinds })
            assertTrue(fixture.name, fixture.assertDocument(document))
            assertGeometryContained(first, fixture.name)
            assertGeometryContained(second, fixture.name)
        }
    }

    @Test
    fun `multi block nested list items reserve one marker gutter and terminal spacing`() {
        val document = compile(Fixture.multiBlockList)
        val layout = prepare(document)
        val leavesByItem = document.blocks.withIndex()
            .filter { it.value.listItemBoundary != null }
            .groupBy { it.value.listItemBoundary!!.identity }

        assertEquals(3, leavesByItem.size)
        leavesByItem.values.forEach { leaves ->
            assertEquals(1, leaves.count { it.value.listItemBoundary!!.isFirstRenderableLeaf })
            assertEquals(1, leaves.count { it.value.listItemBoundary!!.isFinalRenderableLeaf })
            assertEquals(1, leaves.sumOf { layout.blocks[it.index].fragments.count { fragment -> fragment.kind == PreparedProseFragmentKind.MARKER } })

            val anchors = leaves.mapNotNull { leaf ->
                layout.blocks[leaf.index].fragments.firstOrNull {
                    it.kind == PreparedProseFragmentKind.TEXT || it.kind == PreparedProseFragmentKind.ATOM
                }?.let { fragment ->
                    fragment.bounds.left - if (leaf.value.nodeType == "codeBlock") Fixture.codePaddingPx else 0
                }
            }
            assertEquals(leaves.size, anchors.size)
            assertTrue(anchors.all { it == anchors.first() })
        }

        val outer = leavesByItem.values.first { leaves -> leaves.any { it.value.listContext?.index == 7 } }
        val nested = leavesByItem.values.first { leaves -> leaves.any { it.value.listContext?.index == 12 } }
        val empty = leavesByItem.values.first { leaves -> leaves.any { it.value.listContext?.index == 8 } }
        assertEquals("7.", layout.blocks[outer.first().index].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }.label)
        assertEquals("12.", layout.blocks[nested.first().index].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }.label)
        assertEquals("8.", layout.blocks[empty.first().index].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }.label)

        val outerBlocks = outer.map { layout.blocks[it.index] }.sortedBy { it.bounds.top }
        assertEquals(3, outerBlocks.size)
        assertEquals(outerBlocks[0].bounds.bottom, outerBlocks[1].bounds.top)
        assertEquals(outerBlocks[1].bounds.bottom, outerBlocks[2].bounds.top)
        assertEquals(Fixture.listItemSpacingPx, nested.minOf { layout.blocks[it.index].bounds.top } - outerBlocks[2].bounds.bottom)
    }

    @Test
    fun `prepared mark spans affect static layout and paint only geometry is explicit`() {
        val layout = prepare(compile(Fixture.marks))
        val text = layout.blocks.flatMap { it.fragments }.filter { it.kind == PreparedProseFragmentKind.TEXT }
        val spans = text.flatMap { (it.layout!!.text as android.text.Spanned).allSpans<Any>() }

        assertTrue(spans.any { it is StyleSpan && it.style == android.graphics.Typeface.BOLD })
        assertTrue(spans.any { it is UnderlineSpan })
        assertTrue(spans.any { it is ForegroundColorSpan })
        assertTrue(spans.any { it is BackgroundColorSpan })
        assertTrue(layout.blocks.flatMap { it.fragments }.any { it.kind == PreparedProseFragmentKind.STRIKE })
    }

    @Test
    fun `mention merges local paint and border into its immutable atom fragment`() {
        val layout = prepare(compile(Fixture.structural[2]))
        val atom = layout.blocks.flatMap { it.fragments }.single { it.kind == PreparedProseFragmentKind.ATOM }

        assertEquals(0xFF00FF00.toInt(), atom.color)
        assertEquals(0xFF0000FF.toInt(), atom.borderColor)
        assertEquals(2f, atom.strokeWidth)
        assertEquals(9f, atom.cornerRadius)
        assertNotNull(atom.labelLayout)
    }

    @Test
    fun `drawing is culling and paint only after preparation`() {
        val engine = CountingRendererEngine()
        val layout = engine.prepare(themed(compile(Fixture.unicode)), key(), Fixture.widthPx, 1f, false)
        val preparedStaticLayouts = engine.staticLayoutsBuilt
        val view = PreparedProseDrawingView(RuntimeEnvironment.getApplication())
        view.install(layout)
        view.layout(0, 0, Fixture.widthPx, layout.heightPx.coerceAtLeast(1))
        val canvas = Canvas(Bitmap.createBitmap(Fixture.widthPx, layout.heightPx.coerceAtLeast(1), Bitmap.Config.ARGB_8888))

        view.draw(canvas)
        view.draw(canvas)

        assertEquals(1, engine.prepareCount)
        assertEquals(preparedStaticLayouts, engine.staticLayoutsBuilt)
        var offscreenFragments = 0
        layout.forEachFragmentIntersecting(Rect(0, layout.heightPx + 1, Fixture.widthPx, layout.heightPx + 2)) {
            offscreenFragments += 1
        }
        assertEquals(0, offscreenFragments)
    }

    private fun compile(fixture: Fixture): ViewerDocument = compileWithRust(
        ProseViewerRequest(fixture.source, ProseViewerConfiguration(configJson = fixture.configJson, themeJson = Fixture.themeJson))
    )

    private fun themed(document: ViewerDocument): ViewerDocument = document.withPreparedTheme(PreparedProseTheme.resolve(Fixture.themeJson, 1f))

    private fun prepare(document: ViewerDocument): PreparedProseLayout =
        StaticLayoutAndroidProseLayoutEngine().prepare(themed(document), key(), Fixture.widthPx, 1f, false)

    private fun key() = ProseLayoutKey("fixture", Fixture.widthPx, "fixture", 0, 0, 0, "fixture")

    private fun assertGeometryContained(layout: PreparedProseLayout, name: String) {
        layout.blocks.forEach { block ->
            assertTrue("block escapes artifact: $name", block.bounds.top >= 0 && block.bounds.bottom <= layout.heightPx)
            block.fragments.forEach { fragment ->
                assertTrue("fragment escapes block: $name", block.bounds.contains(fragment.bounds))
            }
        }
    }

    private fun assertPreparedLayoutsEqual(first: PreparedProseLayout, second: PreparedProseLayout, name: String) {
        assertEquals("height: $name", first.heightPx, second.heightPx)
        assertEquals("block count: $name", first.blocks.size, second.blocks.size)
        first.blocks.zip(second.blocks).forEachIndexed { blockIndex, (left, right) ->
            assertEquals("block $blockIndex bounds: $name", left.bounds, right.bounds)
            assertEquals("block $blockIndex fragment count: $name", left.fragments.size, right.fragments.size)
            left.fragments.zip(right.fragments).forEachIndexed { fragmentIndex, (a, b) ->
                assertEquals("fragment $fragmentIndex kind: $name", a.kind, b.kind)
                assertEquals("fragment $fragmentIndex bounds: $name", a.bounds, b.bounds)
                assertEquals("fragment $fragmentIndex label: $name", a.label, b.label)
                assertEquals("fragment $fragmentIndex checked: $name", a.checked, b.checked)
                assertEquals("fragment $fragmentIndex text: $name", a.layout?.text?.toString(), b.layout?.text?.toString())
            }
        }
    }
}

private inline fun <reified T> android.text.Spanned.allSpans(): List<T> =
    getSpans(0, length, T::class.java).toList()

private class CountingRendererEngine : AndroidProseLayoutEngine {
    private val delegate = StaticLayoutAndroidProseLayoutEngine()
    var prepareCount = 0
    val staticLayoutsBuilt: Int get() = delegate.staticLayoutsBuilt

    override fun prepare(document: ViewerDocument, key: ProseLayoutKey, widthPx: Int, density: Float, collapsesWhenEmpty: Boolean): PreparedProseLayout {
        prepareCount += 1
        return delegate.prepare(document, key, widthPx, density, collapsesWhenEmpty)
    }
}

private data class Fixture(
    val name: String,
    val source: ProseViewerSource,
    val configJson: String,
    val expectedKinds: Set<PreparedProseFragmentKind>,
    val assertDocument: (ViewerDocument) -> Boolean,
) {
    companion object {
        const val widthPx = 640
        const val codePaddingPx = 12
        const val listItemSpacingPx = 4
        val themeJson = """{"mentions":{"textColor":"#102030","backgroundColor":"#DDEEFF","borderColor":"#445566","borderWidth":2,"borderRadius":7},"links":{"color":"#007AFF"}}"""
        private const val localConfig = """{"initialization":{"type":"localEmpty"}}"""
        private const val customConfig = """{"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"codeBlock","content":"text*","group":"block","role":"textBlock"},{"name":"blockquote","content":"block+","group":"block","role":"block"},{"name":"bulletList","content":"listItem+","group":"block","role":"list"},{"name":"orderedList","content":"listItem+","group":"block","role":"list","attrs":{"start":{"default":1}}},{"name":"taskList","content":"listItem+","group":"block","role":"list"},{"name":"listItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},{"name":"horizontal_rule","content":"","group":"block","role":"block","isVoid":true},{"name":"opaqueBlock","content":"","group":"block","role":"block","isVoid":true,"allowUndeclaredAttrs":true},{"name":"hardBreak","content":"","group":"inline","role":"hardBreak","isVoid":true},{"name":"mention","content":"","group":"inline","role":"inline","isVoid":true,"allowUndeclaredAttrs":true,"attrs":{"label":{"default":null}}},{"name":"opaque","content":"","group":"inline","role":"inline","isVoid":true,"allowUndeclaredAttrs":true},{"name":"text","group":"inline","role":"text"}],"marks":[{"name":"bold"},{"name":"italic"},{"name":"underline"},{"name":"strike"},{"name":"code"},{"name":"link","attrs":{"href":{}}},{"name":"textColor","attrs":{"color":{}}},{"name":"highlight","attrs":{"color":{}}},{"name":"textStyle","attrs":{"fontFamily":{},"fontSize":{}}}]},"initialization":{"type":"localEmpty"}}"""

        val structural = listOf(
            Fixture("nested JSON list and blockquote inheritance", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"blockquote","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"orderedList","attrs":{"start":12},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}]}]}]}]}]}"""), localConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BORDER)) { it.blocks.any { block -> block.inBlockquote && block.listContext?.index == 12 } },
            Fixture("HTML headings marks rules and hard breaks", ProseViewerSource.Html("<h1>Heading 1</h1><h2>Heading 2</h2><h3>Heading 3</h3><h4>Heading 4</h4><h5>Heading 5</h5><h6>Heading 6</h6><blockquote><p><strong>bold</strong><br>quote</p></blockquote><ol start=\"3\"><li>third</li></ol><hr>"), localConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.RULE)) { document -> (1..6).all { document.blocks.any { it.nodeType == "h$it" } } },
            Fixture("custom atoms task list and snake rule", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"label":"Ada","mentionTheme":{"textColor":"#FF0000","backgroundColor":"#00FF00","borderColor":"#0000FF","borderWidth":2,"borderRadius":9}}},{"type":"opaque","attrs":{"label":"opaque"}}]},{"type":"taskList","content":[{"type":"listItem","attrs":{"checked":true},"content":[{"type":"paragraph","content":[{"type":"text","text":"task"}]}]}]},{"type":"horizontal_rule"}]}"""), customConfig, setOf(PreparedProseFragmentKind.ATOM, PreparedProseFragmentKind.RULE)) { document -> document.blocks.any { it.listContext?.kind == "task" && it.listContext.checked } }
        )
        val marks = Fixture("all marks", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"bold","marks":[{"type":"bold"}]},{"type":"text","text":"italic","marks":[{"type":"italic"}]},{"type":"text","text":"under","marks":[{"type":"underline"}]},{"type":"text","text":"strike","marks":[{"type":"strike"}]},{"type":"text","text":"code","marks":[{"type":"code"}]},{"type":"text","text":"link","marks":[{"type":"link","attrs":{"href":"https://example.test"}}]},{"type":"text","text":"red","marks":[{"type":"textColor","attrs":{"color":"#FF0000"}}]},{"type":"text","text":"highlight","marks":[{"type":"highlight","attrs":{"color":"#FFF176"}}]},{"type":"text","text":"sized","marks":[{"type":"textStyle","attrs":{"fontFamily":"monospace","fontSize":19}}]},{"type":"text","text":"combo","marks":[{"type":"code"},{"type":"bold"},{"type":"italic"}]}]}]}"""), customConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.STRIKE)) { true }
        val multiBlockList = Fixture("multi block nested ordered list boundaries", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"blockquote","content":[{"type":"orderedList","attrs":{"start":7},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"codeBlock","content":[{"type":"text","text":"second"}]},{"type":"opaqueBlock","attrs":{"label":"third"}},{"type":"orderedList","attrs":{"start":12},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]},{"type":"listItem","content":[{"type":"paragraph"}]}]}]}]}"""), customConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.BACKGROUND, PreparedProseFragmentKind.ATOM)) { it.blocks.any { block -> block.listContext?.index == 12 } }
        val unicode = Fixture("unicode emoji bidi hard break and opaque atoms", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"שלום 🚀"},{"type":"hardBreak"},{"type":"opaque","attrs":{"label":"inline"}},{"type":"text","text":" café"}]},{"type":"opaqueBlock","attrs":{"label":"block"}}]}"""), customConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.ATOM)) { it.blocks.any { block -> block.nodeType == "opaqueBlock" } }
        val all = structural + listOf(marks, multiBlockList, unicode)
    }
}
