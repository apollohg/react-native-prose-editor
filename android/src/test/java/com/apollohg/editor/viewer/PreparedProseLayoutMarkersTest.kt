package com.apollohg.editor.viewer
import android.graphics.Canvas
import android.graphics.Bitmap
import android.graphics.Rect
import android.app.Activity
import android.os.Looper
import android.text.StaticLayout
import android.text.TextPaint
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import com.apollohg.editor.PreparedProseRecyclerHarness
import com.apollohg.editor.PreparedProseBenchmarkConfiguration
import com.apollohg.editor.PreparedProsePerformanceGates
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerErrorCode
import com.apollohg.editor.ProseViewerInteractionListenerAdapter
import com.apollohg.editor.ProseViewerMention
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ProseViewerView
import com.apollohg.editor.OrderedListMarkerSpan
import com.apollohg.editor.RenderBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class PreparedProseLayoutMarkersTest : PreparedProseLayoutTestFixture() {
    @Test
    fun `ordered markers use default schemes by semantic ancestor depth`() {
        val orderedContext = ViewerListContext(
            ordered = true,
            index = 1,
            kind = null,
            checked = false,
            isLast = true,
        )
        val bulletContext = orderedContext.copy(ordered = false)
        val checkedTaskContext = bulletContext.copy(kind = "task", checked = true)
        fun ancestor(
            identity: Int,
            context: ViewerListContext,
            storedDepth: Int,
            isMarkerOwner: Boolean,
        ) = ViewerListItemAncestor(
            identity = identity,
            context = context,
            nestingDepth = storedDepth,
            isFirstRenderableLeaf = isMarkerOwner,
            isFinalRenderableLeaf = isMarkerOwner,
        )
        val ancestorChains = listOf(
            listOf(ancestor(0, orderedContext, storedDepth = 2, isMarkerOwner = true)),
            listOf(
                ancestor(100, bulletContext, storedDepth = 8, isMarkerOwner = false),
                ancestor(1, orderedContext, storedDepth = 0, isMarkerOwner = true),
            ),
            listOf(
                ancestor(101, orderedContext, storedDepth = 12, isMarkerOwner = false),
                ancestor(102, bulletContext, storedDepth = 3, isMarkerOwner = false),
                ancestor(2, orderedContext, storedDepth = 0, isMarkerOwner = true),
            ),
            listOf(
                ancestor(103, bulletContext, storedDepth = 20, isMarkerOwner = true),
                ancestor(104, checkedTaskContext, storedDepth = 2, isMarkerOwner = true),
                ancestor(105, orderedContext, storedDepth = 0, isMarkerOwner = true),
                ancestor(3, orderedContext, storedDepth = 1, isMarkerOwner = true),
            ),
        )
        val blocks = ancestorChains.mapIndexed { index, ancestors ->
            val markerOwner = ancestors.last()
            ViewerBlock(
                nodeType = "paragraph",
                depth = 40 + index,
                inBlockquote = index == 1,
                listContext = markerOwner.context,
                listItemBoundary = ViewerListItemBoundary(
                    identity = markerOwner.identity,
                    nestingDepth = markerOwner.nestingDepth,
                    isFirstRenderableLeaf = true,
                    isFinalRenderableLeaf = true,
                ),
                inlines = listOf(ViewerInline.Text("item", emptyList())),
                listItemAncestors = ancestors,
            )
        }
        val document = ViewerDocument(
            semanticKey = "ordered-marker-theme",
            blocks = blocks,
            isEmpty = false,
            retainedBytes = 128,
        )
        val theme = PreparedProseTheme.resolve(null, density = 1f)

        val layout = StaticLayoutAndroidProseLayoutEngine().prepare(
            document = document,
            key = testLayoutKey("ordered-marker-theme"),
            theme = theme,
            widthPx = 320,
            density = 1f,
            collapsesWhenEmpty = false,
        )
        val markerFragments = layout.blocks
            .flatMap { it.fragments }
            .filter { it.kind == PreparedProseFragmentKind.MARKER }
        val markerLabels = markerFragments.mapNotNull { it.label }
        val nestedLeafMarkers = layout.blocks.last().fragments
            .filter { it.kind == PreparedProseFragmentKind.MARKER }

        assertEquals(listOf("1.", "a.", "i.", "•", "", "i.", "1."), markerLabels)
        assertEquals(listOf("•", "", "i.", "1."), nestedLeafMarkers.mapNotNull { it.label })
        assertEquals("•", nestedLeafMarkers[0].label)
        assertFalse(nestedLeafMarkers[0].checked)
        assertEquals("", nestedLeafMarkers[1].label)
        assertTrue(nestedLeafMarkers[1].checked)
        assertEquals(listOf("i.", "1."), nestedLeafMarkers.drop(2).mapNotNull { it.label })
    }

    @Test
    fun `ordered marker editor and viewer rendering conform for shared tuples`() {
        data class Fixture(val index: Long, val semanticDepth: Int, val expected: String)

        val fixtures = listOf(
            Fixture(index = 27, semanticDepth = 0, expected = "AA)"),
            Fixture(index = 3_999, semanticDepth = 1, expected = "MMMCMXCIX)"),
            Fixture(index = 42, semanticDepth = 2, expected = "42)"),
        )
        val themeJson =
            """{"list":{"orderedMarker":{"schemes":["upperAlpha","upperRoman","decimal"],"suffix":")"}}}"""
        val editorTheme = com.apollohg.editor.EditorTheme.fromJson(themeJson)
        val viewerTheme = PreparedProseTheme.resolve(themeJson, density = 1f)

        fixtures.forEach { fixture ->
            val renderElements = JSONArray()
            repeat(fixture.semanticDepth + 1) { depth ->
                val deepest = depth == fixture.semanticDepth
                renderElements.put(
                    JSONObject()
                        .put("type", "blockStart")
                        .put("nodeType", "listItem")
                        .put("depth", depth)
                        .put(
                            "listContext",
                            JSONObject()
                                .put("ordered", deepest)
                                .put("index", if (deepest) fixture.index else 1)
                                .put("isFirst", true)
                                .put("isLast", true),
                        ),
                )
            }
            renderElements.put(
                JSONObject()
                    .put("type", "blockStart")
                    .put("nodeType", "paragraph")
                    .put("depth", fixture.semanticDepth + 1),
            )
            renderElements.put(
                JSONObject()
                    .put("type", "textRun")
                    .put("text", "item")
                    .put("marks", JSONArray()),
            )
            renderElements.put(JSONObject().put("type", "blockEnd"))
            repeat(fixture.semanticDepth + 1) {
                renderElements.put(JSONObject().put("type", "blockEnd"))
            }

            val editor = RenderBridge.buildSpannable(
                renderElements.toString(),
                16f,
                0xFF000000.toInt(),
                editorTheme,
            )
            val editorLabel = editor.getSpans(
                0,
                editor.length,
                OrderedListMarkerSpan::class.java,
            ).single().label

            val orderedContext = ViewerListContext(
                ordered = true,
                index = fixture.index,
                kind = null,
                checked = false,
                isLast = true,
            )
            val ancestors = (0..fixture.semanticDepth).map { depth ->
                val deepest = depth == fixture.semanticDepth
                ViewerListItemAncestor(
                    identity = depth,
                    context = if (deepest) orderedContext else orderedContext.copy(ordered = false),
                    nestingDepth = 50 - depth,
                    isFirstRenderableLeaf = deepest,
                    isFinalRenderableLeaf = deepest,
                )
            }
            val block = ViewerBlock(
                nodeType = "paragraph",
                depth = 80 + fixture.semanticDepth,
                inBlockquote = false,
                listContext = orderedContext,
                listItemBoundary = ViewerListItemBoundary(
                    identity = ancestors.last().identity,
                    nestingDepth = 40 - fixture.semanticDepth,
                    isFirstRenderableLeaf = true,
                    isFinalRenderableLeaf = true,
                ),
                inlines = listOf(ViewerInline.Text("item", emptyList())),
                listItemAncestors = ancestors,
            )
            val viewer = StaticLayoutAndroidProseLayoutEngine().prepare(
                document = ViewerDocument(
                    semanticKey = "conformance-${fixture.semanticDepth}",
                    blocks = listOf(block),
                    isEmpty = false,
                    retainedBytes = 64,
                ),
                key = testLayoutKey("conformance-${fixture.semanticDepth}"),
                theme = viewerTheme,
                widthPx = 320,
                density = 1f,
                collapsesWhenEmpty = false,
            )
            val viewerLabel = viewer.blocks
                .flatMap { it.fragments }
                .single { it.kind == PreparedProseFragmentKind.MARKER }
                .label

            assertEquals(fixture.expected, editorLabel)
            assertEquals(fixture.expected, viewerLabel)
            assertEquals(editorLabel, viewerLabel)
        }
    }

    @Test
    fun `culling skips a large offscreen prefix and visits each visible block once`() {
        val blockLayout = StaticLayout.Builder
            .obtain("x", 0, 1, TextPaint().apply { textSize = 14f }, 10)
            .build()
        val artifact = PreparedProseLayout(
            key = ProseLayoutKey("culling", 10, "", 0, 0, 0, 0, "culling"),
            widthPx = 10,
            heightPx = 10_000,
            blocks = List(1_000) { index ->
                PreparedProseBlock(
                    fragments = listOf(
                        PreparedProseFragment(
                            PreparedProseFragmentKind.TEXT,
                            Rect(0, index * 10, 10, index * 10 + 10),
                            layout = blockLayout,
                        )
                    ),
                    bounds = Rect(0, index * 10, 10, index * 10 + 10),
                )
            },
            retainedBytes = 0,
        )
        val visited = mutableListOf<Int>()

        artifact.forEachBlockIntersecting(Rect(0, 9_000, 10, 9_030)) { block ->
            visited += block.topPx
        }

        assertEquals(listOf(9_000, 9_010, 9_020), visited)
        assertEquals(visited.size, visited.distinct().size)
    }
}
