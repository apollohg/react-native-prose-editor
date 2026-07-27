package com.apollohg.editor.viewer

import android.graphics.Rect
import android.graphics.Paint
import android.text.Layout
import android.text.StaticLayout
import android.text.TextPaint
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import org.junit.Assert.assertEquals
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
    fun `empty shaped selection fallback uses matching RTL boundary affinities`() {
        val text = "Latin \u05d0\u05d1\u05d2 Latin"
        val width = 300
        val layout = StaticLayout.Builder.obtain(
            text,
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

        assertFallbackMatchesCompleteLineBidi(layout, start, end, line = 0, width = 300)
    }

    @Test
    fun `fallback resolves RTL selection from complete LTR paragraph line`() {
        val text = "Latin \u05d0\u05d1\u05d2- next"
        val layout = layoutFor(text, width = 300)
        val start = text.indexOf('\u05d0')
        val end = text.indexOf("- ") + 2

        assertFallbackMatchesCompleteLineBidi(layout, start, end, line = 0, width = 300)
    }

    @Test
    fun `fallback preserves paragraph direction for wrapped continuation with different first strong character`() {
        val text = "\u05d0\u05d1\u05d2\u05d3\u05d4\u05d5\u05d6\u05d7\u05d8\u05d9\u05db\u05dc\u05de\u05e0\u05e1\u05e2 Latin \u05e4\u05e6\u05e7\u05e8 trailing words"
        val layout = layoutFor(text, width = 120)
        val line = (1 until layout.lineCount).first { candidate ->
            val lineStart = layout.getLineStart(candidate)
            val lineEnd = layout.getLineEnd(candidate)
            val firstStrong = (lineStart until lineEnd).firstOrNull {
                layout.text[it] in 'A'..'Z' || layout.text[it] in 'a'..'z' || layout.text[it] in '\u0590'..'\u05ff'
            }
            val firstStrongIsLtr = firstStrong?.let {
                layout.text[it] in 'A'..'Z' || layout.text[it] in 'a'..'z'
            } == true
            val hasFollowingRtl = firstStrong?.let { firstStrongOffset ->
                (firstStrongOffset + 1 until lineEnd).any { layout.text[it] in '\u0590'..'\u05ff' }
            } == true
            firstStrongIsLtr && hasFollowingRtl
        }
        val lineStart = layout.getLineStart(line)
        val lineEnd = layout.getLineEnd(line).let { rawEnd ->
            if (rawEnd > lineStart && layout.text[rawEnd - 1] == '\n') rawEnd - 1 else rawEnd
        }
        val firstStrong = (lineStart until lineEnd).first { layout.text[it] in 'A'..'Z' || layout.text[it] in 'a'..'z' }
        val end = (firstStrong until lineEnd).first { layout.text[it] in '\u0590'..'\u05ff' } + 1

        assertEquals(Layout.DIR_RIGHT_TO_LEFT, layout.getParagraphDirection(line))
        assertFallbackMatchesCompleteLineBidi(layout, firstStrong, end, line, width = 120)
    }

    @Test
    fun `fallback keeps a soft-wrap terminal LTR selection on its current line`() {
        val width = 72
        val layout = layoutFor(
            text = "alphabetical words continue onto a second visual line",
            width = width,
        )
        val line = 0
        val lineStart = layout.getLineStart(line)
        val lineEnd = layout.getLineEnd(line)

        assertTrue(lineEnd < layout.text.length)
        assertEquals(lineEnd, layout.getLineStart(line + 1))

        val rect = fallbackSelectionRectsForLine(
            layout = layout,
            start = lineStart,
            end = lineEnd,
            line = line,
            width = width,
        ).single()

        assertEquals(layout.getLineTop(line), rect.top)
        assertEquals(layout.getLineBottom(line), rect.bottom)
        assertEquals(
            ceil(layout.getLineRight(line)).toInt().coerceIn(0, width),
            rect.right,
        )
        assertTrue(rect.right > rect.left)
        assertTrue(rect.bottom <= layout.getLineTop(line + 1))
    }

    @Test
    fun `fallback keeps an internal LTR terminal in an RTL paragraph out of adjacent visual text`() {
        assertInternalMixedSoftWrapTerminalBoundary(
            paragraphDirection = Layout.DIR_RIGHT_TO_LEFT,
            terminalRunIsRtl = false,
        )
    }

    @Test
    fun `fallback keeps an internal RTL terminal in an LTR paragraph out of adjacent visual text`() {
        assertInternalMixedSoftWrapTerminalBoundary(
            paragraphDirection = Layout.DIR_LEFT_TO_RIGHT,
            terminalRunIsRtl = true,
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
    fun `bidi link uses discontiguous shaped selection rects in visual order`() {
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

        assertTrue(link.rects.size >= 2)
        assertEquals(link.rects.sortedWith(compareBy<android.graphics.Rect> { it.top }.thenBy { it.left }), link.rects)
    }

    @Test
    fun `prepared geometry freezes wrapped links and long-safe mentions in reading order`() {
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
        assertTrue(layout.interactions.first().rects.size >= 2)
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

    private fun layoutFor(text: String, width: Int): StaticLayout = StaticLayout.Builder.obtain(
        text,
        TextPaint(Paint.ANTI_ALIAS_FLAG).apply { textSize = 18f },
        width,
    ).build()

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

    private fun assertInternalMixedSoftWrapTerminalBoundary(
        paragraphDirection: Int,
        terminalRunIsRtl: Boolean,
    ) {
        val fixture = findInternalMixedSoftWrapTerminalFixture(paragraphDirection, terminalRunIsRtl)
        val rect = fallbackSelectionRectsForLine(
            layout = fixture.layout,
            start = fixture.terminalStart,
            end = fixture.terminalEnd,
            line = fixture.line,
            width = fixture.width,
        ).single()

        val expectedStart = visualEdgeBoundary(
            fixture.layout,
            fixture.terminalStart,
            if (fixture.terminalRunIsRtl) FallbackVisualEdge.RIGHT else FallbackVisualEdge.LEFT,
        )
        val expectedTerminal = visualEdgeBoundary(
            fixture.layout,
            fixture.neighborBoundaryOffset,
            fixture.neighborBoundaryEdge,
        )
        val expected = Rect(
            kotlin.math.floor(min(expectedStart, expectedTerminal)).toInt().coerceIn(0, fixture.width),
            fixture.layout.getLineTop(fixture.line),
            ceil(max(expectedStart, expectedTerminal)).toInt().coerceIn(0, fixture.width),
            fixture.layout.getLineBottom(fixture.line),
        )

        assertEquals(expected, rect)
        if (fixture.terminalRunIsRtl) {
            assertTrue(rect.left > kotlin.math.floor(fixture.layout.getLineLeft(fixture.line)).toInt())
        } else {
            assertTrue(rect.right < ceil(fixture.layout.getLineRight(fixture.line)).toInt())
        }
    }

    private data class InternalMixedSoftWrapFixture(
        val layout: StaticLayout,
        val width: Int,
        val line: Int,
        val terminalStart: Int,
        val terminalEnd: Int,
        val terminalRunIsRtl: Boolean,
        val neighborBoundaryOffset: Int,
        val neighborBoundaryEdge: FallbackVisualEdge,
    )

    private fun findInternalMixedSoftWrapTerminalFixture(
        paragraphDirection: Int,
        terminalRunIsRtl: Boolean,
    ): InternalMixedSoftWrapFixture {
        val candidates = listOf(
            "\u05d0\u05d1\u05d2 Latin words \u05d3\u05d4\u05d5 more Latin words \u05d5\u05d6\u05d7 trailing",
            "Latin words \u05d0\u05d1\u05d2 more Latin \u05d3\u05d4\u05d5 words trailing",
            "\u05d0\u05d1\u05d2 abc \u05d3\u05d4\u05d5 def \u05d6\u05d7\u05d8 ghi \u05d9\u05db\u05dc mno",
            "abc \u05d0\u05d1\u05d2 def \u05d3\u05d4\u05d5 ghi \u05d6\u05d7\u05d8 jkl \u05d9\u05db\u05dc",
        )
        for (text in candidates) for (width in 42..132 step 3) {
            val layout = layoutFor(text, width)
            for (line in 0 until layout.lineCount - 1) {
                val lineStart = layout.getLineStart(line)
                val lineEnd = layout.getLineEnd(line)
                if (lineEnd >= text.length || layout.getLineStart(line + 1) != lineEnd) continue
                if (layout.getParagraphDirection(line) != paragraphDirection) continue
                val bidiDirection = if (paragraphDirection == Layout.DIR_RIGHT_TO_LEFT) {
                    Bidi.DIRECTION_RIGHT_TO_LEFT
                } else {
                    Bidi.DIRECTION_LEFT_TO_RIGHT
                }
                val bidi = Bidi(text.substring(lineStart, lineEnd), bidiDirection)
                val visualRuns = expectedVisualRuns(bidi, lineStart)
                if (visualRuns.size < 3 || visualRuns.all { it.logicalIndex == it.visualIndex }) continue
                for (run in visualRuns) {
                    val runStart = run.documentStart
                    val runEnd = run.documentEnd
                    val isRtl = run.isRtl
                    val terminalEdge = if (isRtl) FallbackVisualEdge.LEFT else FallbackVisualEdge.RIGHT
                    val isInternal = (terminalEdge == FallbackVisualEdge.LEFT && run.visualIndex > 0) ||
                        (terminalEdge == FallbackVisualEdge.RIGHT && run.visualIndex < visualRuns.lastIndex)
                    if (runEnd != lineEnd || isRtl != terminalRunIsRtl || !isInternal) continue
                    val neighbor = if (terminalEdge == FallbackVisualEdge.LEFT) {
                        visualRuns[run.visualIndex - 1]
                    } else {
                        visualRuns[run.visualIndex + 1]
                    }
                    val neighborEdge = if (terminalEdge == FallbackVisualEdge.LEFT) {
                        FallbackVisualEdge.RIGHT
                    } else {
                        FallbackVisualEdge.LEFT
                    }
                    val neighborOffset = when (neighborEdge) {
                        FallbackVisualEdge.LEFT -> if (neighbor.isRtl) neighbor.documentEnd else neighbor.documentStart
                        FallbackVisualEdge.RIGHT -> if (neighbor.isRtl) neighbor.documentStart else neighbor.documentEnd
                    }
                    if (neighborOffset == lineEnd) continue
                    return InternalMixedSoftWrapFixture(
                        layout,
                        width,
                        line,
                        runStart,
                        runEnd,
                        isRtl,
                        neighborOffset,
                        neighborEdge,
                    )
                }
            }
        }
        throw AssertionError("No internal mixed-Bidi soft-wrap terminal fixture was constructed")
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
