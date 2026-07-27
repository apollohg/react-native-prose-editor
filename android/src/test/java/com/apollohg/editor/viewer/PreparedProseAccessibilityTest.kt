package com.apollohg.editor.viewer

import android.graphics.Paint
import android.graphics.Rect
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import android.text.TextDirectionHeuristics
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import uniffi.editor_core.FfiViewerMark
import java.text.Bidi
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedProseAccessibilityTest {
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
            // Keep the visible break space within the LTR isolate. Otherwise
            // Bidi resets trailing whitespace to the RTL paragraph direction
            // and the whitespace, rather than the LTR content, becomes the
            // terminal run under test.
            firstLine = "\u05d0\u05d1\u05d2 \u2066abc ",
            continuation = "\u2069next",
            paragraphDirection = Layout.DIR_RIGHT_TO_LEFT,
            terminalRunIsRtl = false,
            expectedVisualLogicalOrder = listOf(2, 1, 0),
        )
    }

    @Test
    fun `fallback keeps an internal RTL terminal in an LTR paragraph out of adjacent visual text`() {
        assertInternalMixedSoftWrapTerminalBoundary(
            // Likewise, retain the visible break space in the RTL isolate so
            // the LTR paragraph's trailing whitespace cannot replace the RTL
            // terminal run selected below.
            firstLine = "abc \u2067\u05d0\u05d1\u05d2 ",
            continuation = "\u2069next",
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            terminalRunIsRtl = true,
            expectedVisualLogicalOrder = listOf(0, 1, 2),
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
    fun `Fabric-equivalent prepared replacement announces one subtree change and preserves focus clear`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context).apply { addView(view) }
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        view.install(null, announceAccessibilitySubtree = false)
        view.install(preparedArtifact("replacement"))

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )

        parent.clearEvents()
        val installedArtifact = view.preparedLayout
        view.linkInteractionsEnabled = false
        assertEquals(1, parent.subtreeChangeCount())
        assertTrue(view.preparedLayout === installedArtifact)

        parent.clearEvents()
        view.install(null)
        assertEquals(1, parent.subtreeChangeCount())
    }

    @Test
    fun `Fabric mount success lets final installation own one replacement subtree notification`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context).apply { addView(view) }
        val transaction = FabricReplacementAccessibilityTransaction()
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        transaction.clearReplacing(view)
        transaction.installMountedReplacement(view, preparedArtifact("replacement"))

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )
    }

    @Test
    fun `Fabric mount miss announces a removed subtree once and suppresses a later deferred install`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context).apply { addView(view) }
        val transaction = FabricReplacementAccessibilityTransaction()
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        transaction.clearReplacing(view)
        transaction.finishWithoutMountedReplacement(view)

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )

        parent.clearEvents()
        transaction.installMountedReplacement(view, preparedArtifact("deferred replacement"))
        assertEquals(0, parent.subtreeChangeCount())
        assertEquals(emptyList<Int>(), parent.eventTypes)
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

    @Test
    fun `host geometry freezes wrapped links and long-safe mentions in reading order`() {
        val document = ViewerDocument(
            semanticKey = "interaction-fixture",
            blocks = listOf(
                ViewerBlock(
                    nodeType = "paragraph",
                    depth = 0,
                    inBlockquote = false,
                    listContext = null,
                    listItemBoundary = null,
                    inlines = listOf(
                        ViewerInline.Text(
                            "linked ".repeat(12),
                            listOf(FfiViewerMark("link", "{\"href\":\"https://example.test/wrapped\"}")),
                        ),
                        ViewerInline.Atom("mention", UInt.MAX_VALUE.toLong(), "{}", "@Ada"),
                    ),
                ),
            ),
            isEmpty = false,
            retainedBytes = 64,
        )
        val layout = StaticLayoutAndroidProseLayoutEngine().prepare(
            document,
            key(document),
            PreparedProseTheme.resolve(null, 1f),
            90,
            1f,
            false,
        )

        assertEquals(listOf(PreparedProseInteraction.Kind.LINK, PreparedProseInteraction.Kind.MENTION), layout.interactions.map { it.kind })
        assertEquals("https://example.test/wrapped", layout.interactions.first().href)
        assertTrue(layout.interactions.first().rects.isNotEmpty())
        assertEquals(UInt.MAX_VALUE.toLong(), layout.interactions.last().docPos)
        assertEquals(listOf(PreparedProseAccessibilityNode.Role.LINK, PreparedProseAccessibilityNode.Role.MENTION), layout.accessibilityNodes.map { it.role })
        assertTrue(layout.retainedBytes > document.retainedBytes)
    }

    private fun key(document: ViewerDocument) = ProseLayoutKey(
        semanticKey = document.semanticKey,
        widthPx = 90,
        themeDigest = "fixture",
        nativeFontRevision = 0,
        fontEnvironmentRevision = 0,
        densityBits = 1f.toRawBits().toLong(),
        attachmentRevision = 0,
        generationIdentity = "fixture",
    )

    private fun layoutFor(
        text: String,
        width: Int,
        textDirection: android.text.TextDirectionHeuristic = TextDirectionHeuristics.FIRSTSTRONG_LTR,
    ): StaticLayout = StaticLayout.Builder.obtain(
        text,
        0,
        text.length,
        TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 18f },
        width,
    ).setTextDirection(textDirection).build()

    private fun assertFallbackMatchesCompleteLineBidi(
        layout: StaticLayout,
        start: Int,
        end: Int,
        line: Int,
        width: Int,
    ) {
        val lineStart = layout.getLineStart(line)
        val rawLineEnd = layout.getLineEnd(line)
        val lineEnd = if (rawLineEnd > lineStart && layout.text[rawLineEnd - 1] == '\n') rawLineEnd - 1 else rawLineEnd
        val direction = if (layout.getParagraphDirection(line) == Layout.DIR_RIGHT_TO_LEFT) {
            Bidi.DIRECTION_RIGHT_TO_LEFT
        } else {
            Bidi.DIRECTION_LEFT_TO_RIGHT
        }
        val bidi = Bidi(layout.text.subSequence(lineStart, lineEnd).toString(), direction)
        val expected = buildList {
            for (run in expectedVisualRuns(bidi, lineStart)) {
                val runStart = max(start, run.documentStart)
                val runEnd = min(end, run.documentEnd)
                fallbackSelectionRectForVisualRun(
                    layout = layout,
                    runStart = runStart,
                    runEnd = runEnd,
                    runIsRtl = run.isRtl,
                    line = line,
                    width = width,
                )?.let(::add)
            }
        }

        assertEquals(expected, fallbackSelectionRectsForLine(layout, start, end, line, width))
        assertTrue(expected.size >= 2)
        assertTrue(expected.all { it.left in 0..width && it.right in 0..width && it.left < it.right })
    }

    private fun assertFallbackVisualRunRect(
        layout: StaticLayout,
        run: ExpectedVisualRun,
        width: Int,
    ) {
        val rect = requireNotNull(
            fallbackSelectionRectForVisualRun(
                layout = layout,
                runStart = run.documentStart,
                runEnd = run.documentEnd,
                runIsRtl = run.isRtl,
                line = 0,
                width = width,
            )
        )
        val startEdge = if (run.isRtl) FallbackVisualEdge.RIGHT else FallbackVisualEdge.LEFT
        val endEdge = if (run.isRtl) FallbackVisualEdge.LEFT else FallbackVisualEdge.RIGHT
        val start = visualEdgeBoundary(layout, run.documentStart, startEdge)
        val end = visualEdgeBoundary(layout, run.documentEnd, endEdge)
        assertEquals(
            Rect(
                kotlin.math.floor(min(start, end)).toInt().coerceIn(0, width),
                layout.getLineTop(0),
                ceil(max(start, end)).toInt().coerceIn(0, width),
                layout.getLineBottom(0),
            ),
            rect,
        )
    }

    private fun assertInternalMixedSoftWrapTerminalBoundary(
        firstLine: String,
        continuation: String,
        paragraphDirection: Int,
        terminalRunIsRtl: Boolean,
        expectedVisualLogicalOrder: List<Int>,
    ) {
        val text = firstLine + continuation
        val lineEnd = firstLine.length
        assertEquals(firstLine.length, lineEnd)
        assertTrue(lineEnd < text.length)
        assertEquals(' ', text[lineEnd - 1])
        assertTrue(text[lineEnd - 1] != '\n')
        val bidiDirection = if (paragraphDirection == Layout.DIR_RIGHT_TO_LEFT) {
            Bidi.DIRECTION_RIGHT_TO_LEFT
        } else {
            Bidi.DIRECTION_LEFT_TO_RIGHT
        }
        val bidi = Bidi(text.subSequence(0, lineEnd).toString(), bidiDirection)
        assertEquals(
            "the fixture must retain the inherited paragraph direction",
            paragraphDirection == Layout.DIR_LEFT_TO_RIGHT,
            bidi.baseIsLeftToRight(),
        )
        val logicalRuns = List(bidi.runCount) { logicalIndex ->
                FallbackLogicalBidiRun(
                    logicalIndex = logicalIndex,
                    documentStart = bidi.getRunStart(logicalIndex),
                    documentEnd = bidi.getRunLimit(logicalIndex),
                    level = bidi.getRunLevel(logicalIndex).toByte(),
                )
            }
        val visualRuns = visualBidiRuns(logicalRuns)
        assertEquals(
            expectedVisualLogicalOrder,
            visualRuns.map { it.logicalRun.logicalIndex },
        )
        val terminal = visualRuns.single {
            it.documentEnd == lineEnd && it.isRtl == terminalRunIsRtl
        }
        assertTrue(text.subSequence(terminal.documentStart, terminal.documentEnd).any { it.isLetter() })
        val terminalEdge = if (terminal.isRtl) FallbackVisualEdge.LEFT else FallbackVisualEdge.RIGHT
        val neighbor = when (terminalEdge) {
            FallbackVisualEdge.LEFT -> visualRuns[terminal.visualIndex - 1]
            FallbackVisualEdge.RIGHT -> visualRuns[terminal.visualIndex + 1]
        }
        assertEquals(
            if (terminalEdge == FallbackVisualEdge.LEFT) terminal.visualIndex - 1 else terminal.visualIndex + 1,
            neighbor.visualIndex,
        )
        val neighborEdge = if (terminalEdge == FallbackVisualEdge.LEFT) {
            FallbackVisualEdge.RIGHT
        } else {
            FallbackVisualEdge.LEFT
        }
        val neighborOffset = when (neighborEdge) {
            FallbackVisualEdge.LEFT -> if (neighbor.isRtl) neighbor.documentEnd else neighbor.documentStart
            FallbackVisualEdge.RIGHT -> if (neighbor.isRtl) neighbor.documentStart else neighbor.documentEnd
        }
        // At this directional boundary the same logical offset has two cursor
        // positions. The adjacent visual run supplies the terminal edge by
        // the opposite affinity, not by its physical left/right label alone.
        assertEquals(terminal.documentStart, neighborOffset)
        val width = visualRuns.size * MIXED_SOFT_WRAP_CHARACTER_WIDTH_PX
        val logicalCaretPositions = mutableMapOf<FixedLogicalCaret, Float>()
        visualRuns.forEach { run ->
            val left = run.visualIndex * MIXED_SOFT_WRAP_CHARACTER_WIDTH_PX.toFloat()
            val right = left + MIXED_SOFT_WRAP_CHARACTER_WIDTH_PX
            logicalCaretPositions[FixedLogicalCaret(run.logicalRun, run.affinityAt(FallbackVisualEdge.LEFT))] = left
            logicalCaretPositions[FixedLogicalCaret(run.logicalRun, run.affinityAt(FallbackVisualEdge.RIGHT))] = right
        }
        fun positionAt(offset: Int, affinity: FallbackLogicalCaretAffinity): Float {
            val run = logicalRuns.single {
                when (affinity) {
                    FallbackLogicalCaretAffinity.LEADING_NEXT -> it.documentStart == offset
                    FallbackLogicalCaretAffinity.TRAILING_PREVIOUS -> it.documentEnd == offset
                }
            }
            return requireNotNull(logicalCaretPositions[FixedLogicalCaret(run, affinity)])
        }
        lateinit var geometry: FallbackLineGeometry
        geometry = FallbackLineGeometry(
            text = text,
            lineStart = 0,
            rawLineEnd = lineEnd,
            nextLineStart = lineEnd,
            paragraphDirection = paragraphDirection,
            top = 10,
            bottom = 30,
            width = width,
            logicalRuns = logicalRuns,
            outerLineBoundary = { edge -> if (edge == FallbackVisualEdge.LEFT) 0f else width.toFloat() },
            primaryHorizontal = { offset ->
                positionAt(
                    offset,
                    if (primaryIsTrailingPrevious(offset, geometry)) {
                        FallbackLogicalCaretAffinity.TRAILING_PREVIOUS
                    } else {
                        FallbackLogicalCaretAffinity.LEADING_NEXT
                    },
                )
            },
            secondaryHorizontal = { offset ->
                positionAt(
                    offset,
                    if (primaryIsTrailingPrevious(offset, geometry)) {
                        FallbackLogicalCaretAffinity.LEADING_NEXT
                    } else {
                        FallbackLogicalCaretAffinity.TRAILING_PREVIOUS
                    },
                )
            },
        )
        val rect = fallbackSelectionRectsForGeometry(
            geometry = geometry,
            start = terminal.documentStart,
            end = terminal.documentEnd,
        ).single()
        val startBoundary = requireNotNull(
            logicalCaretPositions[
                FixedLogicalCaret(terminal.logicalRun, FallbackLogicalCaretAffinity.LEADING_NEXT)
            ]
        )
        val terminalBoundary = requireNotNull(
            logicalCaretPositions[FixedLogicalCaret(neighbor.logicalRun, neighbor.affinityAt(neighborEdge))]
        )
        assertNotEquals(
            "shared logical offset must retain distinct terminal and adjacent-run caret positions",
            startBoundary,
            terminalBoundary,
        )
        assertEquals(
            Rect(
                kotlin.math.floor(min(startBoundary, terminalBoundary)).toInt().coerceIn(0, width),
                geometry.top,
                ceil(max(startBoundary, terminalBoundary)).toInt().coerceIn(0, width),
                geometry.bottom,
            ),
            rect,
        )
        assertTrue(rect.left < rect.right)
    }

    private data class FixedLogicalCaret(
        val run: FallbackLogicalBidiRun,
        val affinity: FallbackLogicalCaretAffinity,
    )

    private fun affinityGeometry(
        paragraphDirection: Int,
        levels: List<Int>? = null,
        runs: List<FallbackLogicalBidiRun>? = null,
        text: String = "x".repeat(levels?.size ?: runs!!.maxOf { it.documentEnd }),
        rawLineEnd: Int = levels?.size ?: runs!!.maxOf { it.documentEnd },
        nextLineStart: Int? = null,
        outerLineBoundary: (FallbackVisualEdge) -> Float = { edge ->
            if (edge == FallbackVisualEdge.LEFT) 0f else 200f
        },
        primaryHorizontal: (Int) -> Float = { offset -> 10f + offset },
        secondaryHorizontal: (Int) -> Float = { offset -> 100f + offset },
    ): FallbackLineGeometry {
        val logicalRuns = runs ?: requireNotNull(levels).mapIndexed { index, level ->
            FallbackLogicalBidiRun(index, index, index + 1, level.toByte())
        }
        return FallbackLineGeometry(
            text = text,
            lineStart = 0,
            rawLineEnd = rawLineEnd,
            nextLineStart = nextLineStart,
            paragraphDirection = paragraphDirection,
            top = 0,
            bottom = 20,
            width = 200,
            logicalRuns = logicalRuns,
            outerLineBoundary = outerLineBoundary,
            primaryHorizontal = primaryHorizontal,
            secondaryHorizontal = secondaryHorizontal,
        )
    }

    private data class ExpectedVisualRun(
        val logicalIndex: Int,
        val visualIndex: Int,
        val documentStart: Int,
        val documentEnd: Int,
        val isRtl: Boolean,
        val level: Byte,
    )


    /**
     * Test-only expected order deliberately starts from Java Bidi's logical
     * accessors and applies its public reordering API. The fixture never
     * equates a logical index with a visual neighbour.
     */
    private fun expectedVisualRuns(bidi: Bidi, documentOffset: Int): List<ExpectedVisualRun> {
        if (bidi.runCount == 0) return emptyList()
        val logical = List(bidi.runCount) { logicalIndex ->
            ExpectedVisualRun(
                logicalIndex = logicalIndex,
                visualIndex = -1,
                documentStart = documentOffset + bidi.getRunStart(logicalIndex),
                documentEnd = documentOffset + bidi.getRunLimit(logicalIndex),
                isRtl = (bidi.getRunLevel(logicalIndex) and 1) == 1,
                level = bidi.getRunLevel(logicalIndex).toByte(),
            )
        }
        val reordered: Array<Any> = Array(logical.size) { logical[it] }
        Bidi.reorderVisually(ByteArray(logical.size) { logical[it].level }, 0, reordered, 0, reordered.size)
        return reordered.mapIndexed { visualIndex, value ->
            (value as ExpectedVisualRun).copy(visualIndex = visualIndex)
        }
    }

    private fun visualEdgeBoundary(
        layout: StaticLayout,
        offset: Int,
        edge: FallbackVisualEdge,
    ): Float {
        val primary = layout.getPrimaryHorizontal(offset)
        val secondary = layout.getSecondaryHorizontal(offset)
        return if (edge == FallbackVisualEdge.RIGHT) max(primary, secondary) else min(primary, secondary)
    }

    private companion object {
        const val MIXED_SOFT_WRAP_CHARACTER_WIDTH_PX = 20
    }

    private fun preparedArtifact(generation: String): PreparedProseLayout = PreparedProseLayout(
        key = ProseLayoutKey(
            semanticKey = generation,
            widthPx = 100,
            themeDigest = "fixture",
            nativeFontRevision = 0,
            fontEnvironmentRevision = 0,
            densityBits = 1f.toRawBits().toLong(),
            attachmentRevision = 0,
            generationIdentity = generation,
        ),
        widthPx = 100,
        heightPx = 20,
        blocks = emptyList(),
        interactions = listOf(
            PreparedProseInteraction(
                kind = PreparedProseInteraction.Kind.LINK,
                rects = listOf(Rect(0, 0, 20, 20)),
                href = "https://example.test/$generation",
                visibleText = generation,
                label = generation,
            ),
        ),
        accessibilityNodes = listOf(
            PreparedProseAccessibilityNode(
                interactionIndex = 0,
                role = PreparedProseAccessibilityNode.Role.LINK,
                label = generation,
                bounds = Rect(0, 0, 20, 20),
            ),
        ),
        retainedBytes = 0,
    )

    private class CapturingAccessibilityParent(context: android.content.Context) : ViewGroup(context) {
        val eventTypes = mutableListOf<Int>()
        private val changeTypes = mutableListOf<Int>()

        override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            eventTypes += event.eventType
            changeTypes += event.contentChangeTypes
            return true
        }

        fun clearEvents() {
            eventTypes.clear()
            changeTypes.clear()
        }

        fun subtreeChangeCount(): Int = eventTypes.indices.count { index ->
            eventTypes[index] == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED &&
                changeTypes[index] == AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
        }

        override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) = Unit
    }
}
