package com.apollohg.editor.viewer

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.text.StaticLayout
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StrikethroughSpan
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
    fun `fixed density compiler edge fixture has exact nested bidi geometry`() {
        val document = compile(Fixture.finalAndroidEdge)
        val first = prepare(document)
        val second = prepare(document)
        val expected = requireNotNull(Fixture.finalAndroidEdge.expectedGeometry)

        // These constants are the supported API-34/Robolectric geometry contract
        // for this compiler-backed source at density 1. One pixel permits only
        // integer rounding at StaticLayout's visual replacement-slot boundary.
        assertTrue("artifact height", kotlin.math.abs(expected.heightPx - first.heightPx) <= expected.tolerancePx)
        expected.blockBounds.forEachIndexed { index, bounds ->
            assertRectEquals("block $index", bounds, first.blocks[index].bounds, expected.tolerancePx)
        }
        expected.fragmentBounds.forEach { expectedFragment ->
            val actual = first.blocks[expectedFragment.blockIndex].fragments
                .filter { it.kind == expectedFragment.kind }
                .elementAt(expectedFragment.ordinal)
            assertRectEquals(
                "block ${expectedFragment.blockIndex} ${expectedFragment.kind} ${expectedFragment.ordinal}",
                expectedFragment.bounds,
                actual.bounds,
                expected.tolerancePx,
            )
        }

        val outerMarker = first.blocks[0].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }
        val nestedMarker = first.blocks[3].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }
        val outerAnchor = first.blocks[0].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.bounds.left
        val nestedAnchor = first.blocks[3].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.bounds.left
        val atom = first.blocks[0].fragments.single { it.kind == PreparedProseFragmentKind.ATOM }
        val atomLayout = first.blocks[0].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.layout!!
        val atomOffset = atomLayout.text.indexOf('\uFFFC')
        val visualStart = atomLayout.getPrimaryHorizontal(atomOffset)
        val visualEnd = atomLayout.getPrimaryHorizontal(atomOffset + 1)

        assertEquals("4294967295.", outerMarker.label)
        assertEquals("•", nestedMarker.label)
        assertTrue("nested anchor must retain the outer marker gutter", nestedAnchor > outerAnchor)
        assertTrue("nested marker must not move left of the outer text anchor", nestedMarker.bounds.left >= outerAnchor)
        assertTrue("nested marker must stay outside the outer text column", nestedMarker.bounds.right <= nestedAnchor)
        assertEquals(kotlin.math.min(visualStart, visualEnd).toInt(), atom.bounds.left - first.blocks[0].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.bounds.left)
        assertEquals(kotlin.math.max(visualStart, visualEnd).toInt(), atom.bounds.right - first.blocks[0].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.bounds.left)
        assertPreparedLayoutsEqual(first, second, Fixture.finalAndroidEdge.name)
    }

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

        val outer = leavesByItem.values.first { leaves -> leaves.any { it.value.listContext?.index == 7L } }
        val nested = leavesByItem.values.first { leaves -> leaves.any { it.value.listContext?.index == 12L } }
        val empty = leavesByItem.values.first { leaves -> leaves.any { it.value.listContext?.index == 8L } }
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

        assertTrue(spans.any { it is ResolvedTextStyleSpan })
        assertTrue(spans.any { it is UnderlineSpan })
        assertTrue(spans.any { it is ForegroundColorSpan })
        assertTrue(spans.any { it is BackgroundColorSpan })
        assertTrue(spans.any { it is StrikethroughSpan })
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
    fun `Fabric pins compiler semantics while density one to two resolves fresh bounded theme paints`() {
        var compilations = 0
        val engine = CountingRendererEngine()
        val registry = PreparedProseLayoutRegistry(
            compiler = { request -> compilations += 1; compileWithRust(request) },
            layoutEngine = engine,
        )
        val fixture = Fixture.structural[2]
        val request = ProseViewerRequest(fixture.source, ProseViewerConfiguration(configJson = fixture.configJson, themeJson = Fixture.themeJson))
        val surface = FabricSurfaceToken(77, 701)

        val densityOne = registry.measure(request, Fixture.widthPx, 1f, surface)
        val densityTwo = registry.measure(request, Fixture.widthPx, 2f, surface)
        val oneAtom = densityOne.blocks.flatMap { it.fragments }.single { it.kind == PreparedProseFragmentKind.ATOM }
        val twoAtom = densityTwo.blocks.flatMap { it.fragments }.single { it.kind == PreparedProseFragmentKind.ATOM }

        assertEquals(1, compilations)
        assertEquals(2, engine.prepareCount)
        assertEquals(1, registry.fabricGenerationPinCountForTesting)
        assertEquals(2, registry.preparedThemeCountForTesting)
        assertEquals(1f.toRawBits().toLong(), densityOne.key.densityBits)
        assertEquals(2f.toRawBits().toLong(), densityTwo.key.densityBits)
        assertEquals(2f, oneAtom.strokeWidth)
        assertEquals(4f, twoAtom.strokeWidth)
        assertEquals(9f, oneAtom.cornerRadius)
        assertEquals(18f, twoAtom.cornerRadius)
    }

    @Test
    fun `oversized marker reserves a nonnegative top and shares the first text baseline`() {
        val document = compileSource(
            """{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"marker"}]}]}]}]}""",
            Fixture.structural[2].configJson,
        )
        val layout = prepare(document, PreparedProseTheme.resolve("""{"list":{"markerScale":4}}""", 1f))
        val block = layout.blocks.single()
        val marker = block.fragments.single { it.kind == PreparedProseFragmentKind.MARKER }
        val text = block.fragments.single { it.kind == PreparedProseFragmentKind.TEXT }

        assertTrue(marker.bounds.top >= 0)
        assertEquals(
            text.bounds.top + text.layout!!.getLineBaseline(0) - marker.layout!!.getLineBaseline(0),
            marker.bounds.top,
        )
        assertTrue(marker.bounds.bottom <= block.bounds.bottom)
    }

    @Test
    fun `nested list and quote rule right edge excludes both insets`() {
        val document = compileSource(
            """{"type":"doc","content":[{"type":"blockquote","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"lead"}]},{"type":"horizontal_rule"}]}]}]}]}""",
            Fixture.structural[2].configJson,
        )
        val rule = prepare(document).blocks.flatMap { it.fragments }.single { it.kind == PreparedProseFragmentKind.RULE }

        assertTrue(rule.bounds.left > 0)
        assertEquals(Fixture.widthPx - rule.bounds.left, rule.bounds.right)
    }

    @Test
    fun `atom descenders remain inside metric line and atom bounds`() {
        val document = compileSource(
            """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"opaque","attrs":{"label":"gy"}}]}]}""",
            Fixture.structural[2].configJson,
        )
        val atom = prepare(document).blocks.flatMap { it.fragments }.single { it.kind == PreparedProseFragmentKind.ATOM }

        assertTrue(atom.bounds.bottom >= atom.labelY + atom.labelLayout!!.height)
        assertTrue(atom.bounds.height() > atom.labelLayout!!.getLineBaseline(0))
    }

    @Test
    fun `compiler backed fixed line heights cover single final heading code list and density metrics`() {
        val document = compileHtml(
            "<p>single</p><p>wrapped final line needs enough words to wrap at this fixed width</p>" +
                "<h1>heading</h1><pre><code>code</code></pre><ul><li>list leaf</li></ul>",
        )
        val densityOneTheme = PreparedProseTheme.resolve(
            """{"paragraph":{"fontSize":16,"lineHeight":30},"headings":{"h1":{"lineHeight":46}},"codeBlock":{"fontSize":14,"lineHeight":26}}""",
            1f,
        )
        val densityOne = prepare(document, densityOneTheme, 160)
        val natural = prepare(
            document,
            PreparedProseTheme.resolve("""{"paragraph":{"fontSize":16},"codeBlock":{"fontSize":14}}""", 1f),
            160,
        )
        val paragraphs = document.blocks.withIndex().filter { it.value.nodeType == "paragraph" && it.value.listContext == null }
        val heading = document.blocks.indexOfFirst { it.nodeType == "h1" }
        val code = document.blocks.indexOfFirst { it.nodeType == "codeBlock" }
        val list = document.blocks.indexOfLast { it.listContext != null }

        assertEquals(2, paragraphs.size)
        val single = textLayout(densityOne, paragraphs[0].index)
        val naturalSingle = textLayout(natural, paragraphs[0].index)
        assertEquals(30, lineHeight(single, 0))
        assertEquals(
            (30 - lineHeight(naturalSingle, 0)) / 2,
            single.getLineBaseline(0) - naturalSingle.getLineBaseline(0),
        )
        assertEquals(1, (single.text as android.text.Spanned).getSpans(0, single.text.length, FixedLineHeightMetricSpan::class.java).size)
        val wrapped = textLayout(densityOne, paragraphs[1].index)
        assertTrue("wrapped fixture must have a final line", wrapped.lineCount >= 2)
        assertEquals(30, lineHeight(wrapped, wrapped.lineCount - 1))
        assertEquals(46, lineHeight(textLayout(densityOne, heading), 0))
        assertEquals(26, lineHeight(textLayout(densityOne, code), 0))
        assertEquals(30, lineHeight(textLayout(densityOne, list), 0))
        assertEquals(30, densityOne.blocks[list].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }.bounds.height())

        val densityTwo = prepare(
            document,
            PreparedProseTheme.resolve(
                """{"paragraph":{"fontSize":16,"lineHeight":30},"headings":{"h1":{"lineHeight":46}},"codeBlock":{"fontSize":14,"lineHeight":26}}""",
                2f,
            ),
            320,
        )
        assertEquals(60, lineHeight(textLayout(densityTwo, paragraphs[0].index), 0))
        assertEquals(92, lineHeight(textLayout(densityTwo, heading), 0))
        assertEquals(52, lineHeight(textLayout(densityTwo, code), 0))
        assertEquals(60, densityTwo.blocks[list].fragments.single { it.kind == PreparedProseFragmentKind.MARKER }.bounds.height())
    }

    @Test
    fun `reduced line height preserves natural extreme text and atom metrics without clipping`() {
        val document = compileSource(
            """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"large"},{"type":"opaque","attrs":{"label":"atom"}}]}]}""",
            Fixture.structural[2].configJson,
        )
        val natural = prepare(document, PreparedProseTheme.resolve("""{"paragraph":{"fontSize":64}}""", 1f), 320)
        val reduced = prepare(document, PreparedProseTheme.resolve("""{"paragraph":{"fontSize":64,"lineHeight":16}}""", 1f), 320)
        val naturalLayout = textLayout(natural, 0)
        val reducedLayout = textLayout(reduced, 0)
        val atom = reduced.blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.ATOM }

        assertEquals(lineHeight(naturalLayout, 0), lineHeight(reducedLayout, 0))
        assertTrue(atom.bounds.height() >= atom.labelLayout!!.height)
        assertTrue(reduced.blocks.single().bounds.contains(atom.bounds))
        assertTrue(reducedLayout.text is android.text.Spanned)
        assertEquals(1, (reducedLayout.text as android.text.Spanned).getSpans(0, reducedLayout.text.length, FixedLineHeightMetricSpan::class.java).size)
    }

    @Test
    fun `link typography resolves before mark traits and RTL strike keeps its run foreground`() {
        val themed = """{"links":{"fontFamily":"monospace","fontSize":23,"fontWeight":"700","fontStyle":"italic","color":"#13579B"}}"""
        val document = compileSource(
            """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"שלום","marks":[{"type":"link","attrs":{"href":"https://example.test"}},{"type":"strike"},{"type":"bold"}]}]}]}""",
            Fixture.structural[2].configJson,
        )
        val text = prepare(document, PreparedProseTheme.resolve(themed, 1f)).blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.layout!!
        val spans = text.text as android.text.Spanned
        val style = spans.getSpans(0, spans.length, ResolvedTextStyleSpan::class.java).single()
        val foreground = spans.getSpans(0, spans.length, ForegroundColorSpan::class.java).single()

        assertEquals(23f, style.sizePx)
        assertEquals(android.graphics.Typeface.BOLD_ITALIC, style.typeface.style)
        assertEquals(0xFF13579B.toInt(), foreground.foregroundColor)
        assertEquals(1, spans.getSpans(0, spans.length, StrikethroughSpan::class.java).size)
        assertEquals(android.text.Layout.DIR_RIGHT_TO_LEFT, text.getParagraphDirection(0))
    }

    @Test
    fun `compiler u32 maximum ordered index and semantic atom position never narrow or wrap`() {
        val document = compileSource(
            """{"type":"doc","content":[{"type":"orderedList","attrs":{"start":4294967295},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"max"}]}]}]}]}""",
            Fixture.structural[2].configJson,
        )
        val compilerIndex = document.blocks.single().listContext!!.index
        val semanticAtom = ViewerInline.Atom("opaque", 0xFFFF_FFFFL, "{}", "max")
        val atomDocument = ViewerDocument("u32-atom", listOf(ViewerBlock("paragraph", 0, false, null, null, listOf(semanticAtom))), false, 0)
        val marker = prepare(document).blocks.single().fragments.single { it.kind == PreparedProseFragmentKind.MARKER }

        assertEquals(0xFFFF_FFFFL, compilerIndex)
        assertEquals("4294967295.", marker.label)
        assertEquals(0xFFFF_FFFFL, (atomDocument.blocks.single().inlines.single() as ViewerInline.Atom).docPos)
        assertTrue(prepare(atomDocument).blocks.single().fragments.any { it.kind == PreparedProseFragmentKind.ATOM })
    }

    @Test
    fun `drawing is culling and paint only after preparation`() {
        val engine = CountingRendererEngine()
        val layout = engine.prepare(compile(Fixture.unicode), key(), theme(), Fixture.widthPx, 1f, false)
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

    private fun compileSource(source: String, configJson: String): ViewerDocument = compileWithRust(
        ProseViewerRequest(ProseViewerSource.Json(source), ProseViewerConfiguration(configJson = configJson, themeJson = Fixture.themeJson))
    )

    private fun compileHtml(source: String): ViewerDocument = compileWithRust(
        ProseViewerRequest(ProseViewerSource.Html(source), ProseViewerConfiguration(configJson = Fixture.structural[1].configJson, themeJson = Fixture.themeJson))
    )

    private fun theme(density: Float = 1f): PreparedProseTheme = PreparedProseTheme.resolve(Fixture.themeJson, density)

    private fun prepare(document: ViewerDocument): PreparedProseLayout =
        StaticLayoutAndroidProseLayoutEngine().prepare(document, key(), theme(), Fixture.widthPx, 1f, false)

    private fun prepare(document: ViewerDocument, theme: PreparedProseTheme): PreparedProseLayout =
        StaticLayoutAndroidProseLayoutEngine().prepare(document, key(), theme, Fixture.widthPx, theme.density, false)

    private fun prepare(document: ViewerDocument, theme: PreparedProseTheme, widthPx: Int): PreparedProseLayout =
        StaticLayoutAndroidProseLayoutEngine().prepare(document, key(), theme, widthPx, theme.density, false)

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

    private fun assertRectEquals(name: String, expected: Rect, actual: Rect, tolerance: Int) {
        assertTrue("$name left", kotlin.math.abs(expected.left - actual.left) <= tolerance)
        assertTrue("$name top", kotlin.math.abs(expected.top - actual.top) <= tolerance)
        assertTrue("$name right", kotlin.math.abs(expected.right - actual.right) <= tolerance)
        assertTrue("$name bottom", kotlin.math.abs(expected.bottom - actual.bottom) <= tolerance)
    }

    private fun textLayout(layout: PreparedProseLayout, blockIndex: Int): StaticLayout =
        layout.blocks[blockIndex].fragments.single { it.kind == PreparedProseFragmentKind.TEXT }.layout!!

    private fun lineHeight(layout: StaticLayout, line: Int): Int =
        layout.getLineBottom(line) - layout.getLineTop(line)
}

private inline fun <reified T> android.text.Spanned.allSpans(): List<T> =
    getSpans(0, length, T::class.java).toList()

private class CountingRendererEngine : AndroidProseLayoutEngine {
    private val delegate = StaticLayoutAndroidProseLayoutEngine()
    var prepareCount = 0
    val staticLayoutsBuilt: Int get() = delegate.staticLayoutsBuilt

    override fun prepare(document: ViewerDocument, key: ProseLayoutKey, theme: PreparedProseTheme, widthPx: Int, density: Float, collapsesWhenEmpty: Boolean): PreparedProseLayout {
        prepareCount += 1
        return delegate.prepare(document, key, theme, widthPx, density, collapsesWhenEmpty)
    }
}

private data class Fixture(
    val name: String,
    val source: ProseViewerSource,
    val configJson: String,
    val expectedKinds: Set<PreparedProseFragmentKind>,
    val assertDocument: (ViewerDocument) -> Boolean,
    val expectedGeometry: ExpectedGeometry? = null,
) {
    companion object {
        const val widthPx = 640
        const val codePaddingPx = 12
        const val listItemSpacingPx = 4
        val themeJson = """{"mentions":{"textColor":"#102030","backgroundColor":"#DDEEFF","borderColor":"#445566","borderWidth":2,"borderRadius":7},"links":{"color":"#007AFF"}}"""
        private const val localConfig = """{"initialization":{"type":"localEmpty"}}"""
        private const val customConfig = """{"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"codeBlock","content":"text*","group":"block","role":"textBlock"},{"name":"blockquote","content":"block+","group":"block","role":"block"},{"name":"bulletList","content":"listItem+","group":"block","role":"list"},{"name":"orderedList","content":"listItem+","group":"block","role":"list","attrs":{"start":{"default":1}}},{"name":"taskList","content":"listItem+","group":"block","role":"list"},{"name":"listItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},{"name":"horizontal_rule","content":"","group":"block","role":"block","isVoid":true},{"name":"opaqueBlock","content":"","group":"block","role":"block","isVoid":true,"allowUndeclaredAttrs":true},{"name":"hardBreak","content":"","group":"inline","role":"hardBreak","isVoid":true},{"name":"mention","content":"","group":"inline","role":"inline","isVoid":true,"allowUndeclaredAttrs":true,"attrs":{"label":{"default":null}}},{"name":"opaque","content":"","group":"inline","role":"inline","isVoid":true,"allowUndeclaredAttrs":true},{"name":"text","group":"inline","role":"text"}],"marks":[{"name":"bold"},{"name":"italic"},{"name":"underline"},{"name":"strike"},{"name":"code"},{"name":"link","attrs":{"href":{}}},{"name":"textColor","attrs":{"color":{}}},{"name":"highlight","attrs":{"color":{}}},{"name":"textStyle","attrs":{"fontFamily":{},"fontSize":{}}}]},"initialization":{"type":"localEmpty"}}"""

        val structural = listOf(
            Fixture("nested JSON list and blockquote inheritance", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"blockquote","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"outer"}]},{"type":"orderedList","attrs":{"start":12},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"inner"}]}]}]}]}]}]}]}"""), localConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BORDER)) { it.blocks.any { block -> block.inBlockquote && block.listContext?.index == 12L } },
            Fixture("HTML headings marks rules and hard breaks", ProseViewerSource.Html("<h1>Heading 1</h1><h2>Heading 2</h2><h3>Heading 3</h3><h4>Heading 4</h4><h5>Heading 5</h5><h6>Heading 6</h6><blockquote><p><strong>bold</strong><br>quote</p></blockquote><ol start=\"3\"><li>third</li></ol><hr>"), localConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.RULE)) { document -> (1..6).all { document.blocks.any { it.nodeType == "h$it" } } },
            Fixture("custom atoms task list and snake rule", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"paragraph","content":[{"type":"mention","attrs":{"label":"Ada","mentionTheme":{"textColor":"#FF0000","backgroundColor":"#00FF00","borderColor":"#0000FF","borderWidth":2,"borderRadius":9}}},{"type":"opaque","attrs":{"label":"opaque"}}]},{"type":"taskList","content":[{"type":"listItem","attrs":{"checked":true},"content":[{"type":"paragraph","content":[{"type":"text","text":"task"}]}]}]},{"type":"horizontal_rule"}]}"""), customConfig, setOf(PreparedProseFragmentKind.ATOM, PreparedProseFragmentKind.RULE)) { document -> document.blocks.any { it.listContext?.kind == "task" && it.listContext.checked } }
        )
        val marks = Fixture("all marks", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"bold","marks":[{"type":"bold"}]},{"type":"text","text":"italic","marks":[{"type":"italic"}]},{"type":"text","text":"under","marks":[{"type":"underline"}]},{"type":"text","text":"strike","marks":[{"type":"strike"}]},{"type":"text","text":"code","marks":[{"type":"code"}]},{"type":"text","text":"link","marks":[{"type":"link","attrs":{"href":"https://example.test"}}]},{"type":"text","text":"red","marks":[{"type":"textColor","attrs":{"color":"#FF0000"}}]},{"type":"text","text":"highlight","marks":[{"type":"highlight","attrs":{"color":"#FFF176"}}]},{"type":"text","text":"sized","marks":[{"type":"textStyle","attrs":{"fontFamily":"monospace","fontSize":19}}]},{"type":"text","text":"combo","marks":[{"type":"code"},{"type":"bold"},{"type":"italic"}]}]}]}"""), customConfig, setOf(PreparedProseFragmentKind.TEXT)) { true }
        val multiBlockList = Fixture("multi block nested ordered list boundaries", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"blockquote","content":[{"type":"orderedList","attrs":{"start":7},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"codeBlock","content":[{"type":"text","text":"second"}]},{"type":"opaqueBlock","attrs":{"label":"third"}},{"type":"orderedList","attrs":{"start":12},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]},{"type":"listItem","content":[{"type":"paragraph"}]}]}]}]}"""), customConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.BACKGROUND, PreparedProseFragmentKind.ATOM)) { it.blocks.any { block -> block.listContext?.index == 12L } }
        val unicode = Fixture("unicode emoji bidi hard break and opaque atoms", ProseViewerSource.Json("""{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"שלום 🚀"},{"type":"hardBreak"},{"type":"opaque","attrs":{"label":"inline"}},{"type":"text","text":" café"}]},{"type":"opaqueBlock","attrs":{"label":"block"}}]}"""), customConfig, setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.ATOM)) { it.blocks.any { block -> block.nodeType == "opaqueBlock" } }
        val finalAndroidEdge = Fixture(
            name = "fixed density max u32 nested quote code rule and RTL atom",
            source = ProseViewerSource.Json("""{"type":"doc","content":[{"type":"blockquote","content":[{"type":"orderedList","attrs":{"start":4294967295},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"אב"},{"type":"opaque","attrs":{"label":"atom"}},{"type":"text","text":" tail"}]},{"type":"codeBlock","content":[{"type":"text","text":"code"}]},{"type":"horizontal_rule"},{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"nested"}]}]}]}]}]}]}]}"""),
            configJson = customConfig,
            expectedKinds = setOf(PreparedProseFragmentKind.TEXT, PreparedProseFragmentKind.MARKER, PreparedProseFragmentKind.BACKGROUND, PreparedProseFragmentKind.BORDER, PreparedProseFragmentKind.RULE, PreparedProseFragmentKind.ATOM),
            assertDocument = { document ->
                document.blocks.size == 4
                    && document.blocks.last().listItemAncestors.size == 2
                    && document.blocks.first().listContext?.index == 0xFFFF_FFFFL
            },
            expectedGeometry = ExpectedGeometry(
                heightPx = 120,
                blockBounds = listOf(
                    Rect(0, 0, 640, 29),
                    Rect(0, 29, 640, 66),
                    Rect(0, 66, 640, 91),
                    Rect(0, 95, 640, 116),
                ),
                fragmentBounds = listOf(
                    ExpectedFragment(1, PreparedProseFragmentKind.BACKGROUND, 0, Rect(164, 29, 476, 66)),
                    ExpectedFragment(2, PreparedProseFragmentKind.RULE, 0, Rect(164, 78, 476, 79)),
                ),
                tolerancePx = 1,
            ),
        )
        val all = structural + listOf(marks, multiBlockList, unicode)
    }
}

private data class ExpectedGeometry(
    val heightPx: Int,
    val blockBounds: List<Rect>,
    val fragmentBounds: List<ExpectedFragment>,
    val tolerancePx: Int,
)

private data class ExpectedFragment(
    val blockIndex: Int,
    val kind: PreparedProseFragmentKind,
    val ordinal: Int,
    val bounds: Rect,
)
