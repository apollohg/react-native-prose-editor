package com.apollohg.editor.viewer
import android.graphics.Paint
import android.graphics.Rect
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import android.text.TextDirectionHeuristics
import android.view.View
import android.view.ViewGroup
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import uniffi.editor_core.FfiViewerMark
import java.text.Bidi
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class PreparedProseAccessibilityBidiGeometryTest : PreparedProseAccessibilityTestFixture() {
    @Test
    fun `selection fragments merge edge-touching pieces on one visual line`() {
        assertEquals(
            listOf(Rect(2, 10, 18, 20)),
            mergeAdjacentSameLineSelectionFragments(
                listOf(Rect(2, 10, 10, 20), Rect(10, 10, 18, 20))
            )
        )
    }

    @Test
    fun `selection fragments preserve one pixel gaps and separate visual lines`() {
        assertEquals(
            listOf(Rect(2, 10, 10, 20), Rect(11, 10, 18, 20), Rect(0, 30, 8, 40)),
            mergeAdjacentSameLineSelectionFragments(
                listOf(Rect(11, 10, 18, 20), Rect(0, 30, 8, 40), Rect(2, 10, 10, 20))
            )
        )
    }

    @Test
    fun `logical caret affinity selects secondary at nested LTR level boundaries`() {
        val geometry = affinityGeometry(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            levels = listOf(0, 1, 2, 1, 0),
        )

        assertTrue(primaryIsTrailingPrevious(2, geometry))
        assertEquals(
            102f,
            fallbackHorizontalForLogicalCaret(
                geometry,
                2,
                FallbackLogicalCaretAffinity.LEADING_NEXT,
            ),
        )
        assertEquals(false, primaryIsTrailingPrevious(3, geometry))
        assertEquals(
            103f,
            fallbackHorizontalForLogicalCaret(
                geometry,
                3,
                FallbackLogicalCaretAffinity.TRAILING_PREVIOUS,
            ),
        )
    }

    @Test
    fun `logical caret affinity selects secondary for embedded parity transitions`() {
        val ltr = affinityGeometry(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            levels = listOf(0, 1, 0),
        )
        val rtl = affinityGeometry(
            paragraphDirection = Layout.DIR_RIGHT_TO_LEFT,
            levels = listOf(1, 2, 3, 2, 1),
        )

        for ((geometry, start, end) in listOf(Triple(ltr, 1, 2), Triple(rtl, 2, 3))) {
            assertTrue(primaryIsTrailingPrevious(start, geometry))
            assertEquals(
                100f + start,
                fallbackHorizontalForLogicalCaret(
                    geometry,
                    start,
                    FallbackLogicalCaretAffinity.LEADING_NEXT,
                ),
            )
            assertEquals(false, primaryIsTrailingPrevious(end, geometry))
            assertEquals(
                100f + end,
                fallbackHorizontalForLogicalCaret(
                    geometry,
                    end,
                    FallbackLogicalCaretAffinity.TRAILING_PREVIOUS,
                ),
            )
        }
    }

    @Test
    fun `logical caret affinity preserves coincident interior carets`() {
        val geometry = affinityGeometry(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            runs = listOf(FallbackLogicalBidiRun(0, 0, 3, 0)),
            text = "abc",
            primaryHorizontal = { 17f },
            secondaryHorizontal = { 17f },
        )

        assertEquals(false, primaryIsTrailingPrevious(1, geometry))
        assertEquals(
            17f,
            fallbackHorizontalForLogicalCaret(
                geometry,
                1,
                FallbackLogicalCaretAffinity.LEADING_NEXT,
            ),
        )
        assertEquals(
            17f,
            fallbackHorizontalForLogicalCaret(
                geometry,
                1,
                FallbackLogicalCaretAffinity.TRAILING_PREVIOUS,
            ),
        )
    }

    @Test
    fun `fallback uses line boundaries only for outer soft-wrap terminals`() {
        val ltr = affinityGeometry(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            levels = listOf(0),
            text = "xy",
            rawLineEnd = 1,
            nextLineStart = 1,
            outerLineBoundary = { edge -> if (edge == FallbackVisualEdge.LEFT) 0f else 40f },
            primaryHorizontal = { offset ->
                check(offset != 1) { "outer soft-wrap terminal must not query a public horizontal" }
                0f
            },
            secondaryHorizontal = { error("LTR outer terminal must not query secondary horizontal") },
        )
        val rtl = affinityGeometry(
            paragraphDirection = Layout.DIR_RIGHT_TO_LEFT,
            levels = listOf(1),
            text = "xy",
            rawLineEnd = 1,
            nextLineStart = 1,
            outerLineBoundary = { edge -> if (edge == FallbackVisualEdge.LEFT) 0f else 40f },
            primaryHorizontal = { offset ->
                check(offset != 1) { "outer soft-wrap terminal must not query a public horizontal" }
                40f
            },
            secondaryHorizontal = { error("RTL outer terminal must not query secondary horizontal") },
        )

        assertEquals(listOf(Rect(0, 0, 40, 20)), fallbackSelectionRectsForGeometry(ltr, 0, 1))
        assertEquals(listOf(Rect(0, 0, 40, 20)), fallbackSelectionRectsForGeometry(rtl, 0, 1))
    }

    @Test
    fun `fallback final and hard-break line ends do not leak into a next line`() {
        fun horizontal(offset: Int): Float {
            check(offset != 2) { "final and hard-break boundaries must not use the next-line offset" }
            return if (offset == 0) 0f else 10f
        }
        val finalLine = affinityGeometry(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            levels = listOf(0),
            text = "x",
            primaryHorizontal = ::horizontal,
            secondaryHorizontal = ::horizontal,
        )
        val hardBreak = affinityGeometry(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            levels = listOf(0),
            text = "x\nnext",
            rawLineEnd = 2,
            nextLineStart = 2,
            primaryHorizontal = ::horizontal,
            secondaryHorizontal = ::horizontal,
        )

        assertEquals(listOf(Rect(0, 0, 10, 20)), fallbackSelectionRectsForGeometry(finalLine, 0, 1))
        assertEquals(listOf(Rect(0, 0, 10, 20)), fallbackSelectionRectsForGeometry(hardBreak, 0, 1))
    }

    @Test
    fun `empty shaped selection fallback uses matching RTL boundary affinities`() {
        val text = "Latin \u05d0\u05d1\u05d2 Latin"
        val width = 300
        val layout = StaticLayout.Builder.obtain(
            text,
            0,
            text.length,
            TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 18f },
            width,
        ).build()
        val bidi = Bidi(text, Bidi.DIRECTION_LEFT_TO_RIGHT)
        val run = (0 until bidi.runCount).single { (bidi.getRunLevel(it) and 1) == 1 }
        val start = bidi.getRunStart(run)
        val end = bidi.getRunLimit(run)

        val rect = requireNotNull(
            fallbackSelectionRectForVisualRun(
                layout = layout,
                runStart = start,
                runEnd = end,
                runIsRtl = true,
                line = layout.getLineForOffset(start),
                width = width,
            )
        )

        val expectedRight = ceil(
            max(layout.getPrimaryHorizontal(start), layout.getSecondaryHorizontal(start))
        ).toInt().coerceIn(0, width)
        val expectedLeft = kotlin.math.floor(
            min(layout.getPrimaryHorizontal(end), layout.getSecondaryHorizontal(end))
        ).toInt().coerceIn(0, width)
        assertEquals(expectedLeft, rect.left)
        assertEquals(expectedRight, rect.right)
        assertEquals(layout.getLineTop(0), rect.top)
        assertEquals(layout.getLineBottom(0), rect.bottom)
    }

    @Test
    fun `fallback resolves LTR selection from complete RTL paragraph line`() {
        val text = "\u05d0\u05d1\u05d2 Latin \u05d3\u05d4\u05d5\nnext"
        val layout = layoutFor(text, width = 300)
        val start = text.indexOf("Latin")
        val end = start + "Latin ".length

        assertEquals(
            listOf(Rect(286, 0, 291, 39), Rect(291, 0, 296, 39)),
            fallbackSelectionRectsForLine(layout, start, end, line = 0, width = 300),
        )
    }

    @Test
    fun `fallback resolves RTL selection from complete LTR paragraph line`() {
        val text = "Latin \u05d0\u05d1\u05d2- next"
        val layout = layoutFor(text, width = 300)
        val start = text.indexOf('\u05d0')
        val end = text.indexOf("- ") + 2

        assertEquals(
            listOf(Rect(6, 0, 9, 41), Rect(9, 0, 15, 41)),
            fallbackSelectionRectsForLine(layout, start, end, line = 0, width = 300),
        )
    }

    @Test
    fun `fallback continuation Bidi fixture preserves inherited RTL paragraph direction`() {
        // Robolectric does not deterministically choose a soft-wrap point for
        // mixed-script prose. Exercise the exact fallback branch with a fixed
        // continuation-line Bidi fixture instead of searching text/widths.
        val text = "Latin \u05d0\u05d1\u05d2"
        val width = 300
        val layout = layoutFor(text, width, TextDirectionHeuristics.RTL)
        val visualRuns = expectedVisualRuns(Bidi(text, Bidi.DIRECTION_RIGHT_TO_LEFT), 0)
        val ltr = visualRuns.single { !it.isRtl }
        val rtl = visualRuns.single { it.isRtl }

        assertEquals(Layout.DIR_RIGHT_TO_LEFT, layout.getParagraphDirection(0))
        assertEquals(listOf(1, 0), visualRuns.map { it.logicalIndex })
        assertTrue(rtl.visualIndex < ltr.visualIndex)
        assertFallbackVisualRunRect(layout, ltr, width)
    }

    @Test
    fun `fallback keeps an exact soft-wrap LTR terminal on its current visual edge`() {
        val text = "terminal"
        val width = 300
        val layout = layoutFor(text, width, TextDirectionHeuristics.LTR)
        val terminal = expectedVisualRuns(Bidi(text, Bidi.DIRECTION_LEFT_TO_RIGHT), 0).single()

        val rect = requireNotNull(
            fallbackSelectionRectForVisualRun(
                layout = layout,
                runStart = terminal.documentStart,
                runEnd = terminal.documentEnd,
                runIsRtl = false,
                line = 0,
                width = width,
                // This pure visual-run fixture explicitly supplies the
                // ambiguous shared line-end and its current-line edge.
                softWrapLineEnd = terminal.documentEnd,
                softWrapTerminalBoundary = layout.getLineRight(0),
            )
        )

        assertEquals(layout.getLineTop(0), rect.top)
        assertEquals(layout.getLineBottom(0), rect.bottom)
        assertEquals(ceil(layout.getLineRight(0)).toInt().coerceIn(0, width), rect.right)
        assertTrue(rect.right > rect.left)
    }

    @Test
    fun `fallback keeps an internal LTR terminal in an RTL paragraph out of adjacent visual text`() {
        assertInternalMixedSoftWrapTerminalBoundary(
            // Do not put whitespace after the LTR isolate content: Java Bidi
            // resets trailing whitespace to the RTL paragraph direction.
            // The opposite-direction terminal must instead end at lineEnd.
            firstLine = "\u05d0\u05d1\u05d2 \u2066abc",
            continuation = "\u2069next",
            paragraphDirection = Layout.DIR_RIGHT_TO_LEFT,
            terminalRunIsRtl = false,
            expectedVisualLogicalOrder = listOf(1, 0),
        )
    }

    @Test
    fun `fallback keeps an internal RTL terminal in an LTR paragraph out of adjacent visual text`() {
        assertInternalMixedSoftWrapTerminalBoundary(
            // Likewise, omit trailing whitespace so Java Bidi leaves the RTL
            // content as the terminal run selected below.
            firstLine = "abc \u2067\u05d0\u05d1\u05d2",
            continuation = "\u2069next",
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            terminalRunIsRtl = true,
            expectedVisualLogicalOrder = listOf(0, 1),
        )
    }

    @Test
    fun `Bidi visual reordering reverses nested logical runs`() {
        val logical = arrayOf<Any>("base", "rtl-outer", "ltr-inner", "rtl-tail", "base-tail")
        Bidi.reorderVisually(byteArrayOf(0, 1, 2, 1, 0), 0, logical, 0, logical.size)

        assertEquals(
            listOf("base", "rtl-tail", "ltr-inner", "rtl-outer", "base-tail"),
            logical.toList(),
        )
    }

    @Test
    fun `host shaped bidi link preserves its nonempty contour in visual order`() {
        val document = ViewerDocument(
            semanticKey = "bidi-link-fixture",
            blocks = listOf(
                ViewerBlock(
                    nodeType = "paragraph",
                    depth = 0,
                    inBlockquote = false,
                    listContext = null,
                    listItemBoundary = null,
                    inlines = listOf(
                        ViewerInline.Text(
                            "Latin \u05e2\u05d1\u05e8\u05d9\u05ea Latin",
                            listOf(FfiViewerMark("link", "{\"href\":\"https://example.test/bidi\"}")),
                        ),
                    ),
                ),
            ),
            isEmpty = false,
            retainedBytes = 64,
        )

        val link = StaticLayoutAndroidProseLayoutEngine().prepare(
            document,
            key(document),
            PreparedProseTheme.resolve(null, 1f),
            300,
            1f,
            false,
        ).interactions.single { it.kind == PreparedProseInteraction.Kind.LINK }

        assertTrue(link.rects.isNotEmpty())
        assertEquals(link.rects.sortedWith(compareBy<android.graphics.Rect> { it.top }.thenBy { it.left }), link.rects)
    }
}
