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

internal abstract class PreparedProseAccessibilityTestFixture {
    protected fun key(document: ViewerDocument) = ProseLayoutKey(
        semanticKey = document.semanticKey,
        widthPx = 90,
        themeDigest = "fixture",
        nativeFontRevision = 0,
        fontEnvironmentRevision = 0,
        densityBits = 1f.toRawBits().toLong(),
        attachmentRevision = 0,
        generationIdentity = "fixture",
    )

    protected fun layoutFor(
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

    protected fun assertFallbackMatchesCompleteLineBidi(
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

    protected fun assertFallbackVisualRunRect(
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

    protected fun assertInternalMixedSoftWrapTerminalBoundary(
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
        assertTrue(text[lineEnd - 1].isLetter())
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

    protected data class FixedLogicalCaret(
        val run: FallbackLogicalBidiRun,
        val affinity: FallbackLogicalCaretAffinity,
    )

    protected fun affinityGeometry(
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

    protected data class ExpectedVisualRun(
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
    protected fun expectedVisualRuns(bidi: Bidi, documentOffset: Int): List<ExpectedVisualRun> {
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

    protected fun visualEdgeBoundary(
        layout: StaticLayout,
        offset: Int,
        edge: FallbackVisualEdge,
    ): Float {
        val primary = layout.getPrimaryHorizontal(offset)
        val secondary = layout.getSecondaryHorizontal(offset)
        return if (edge == FallbackVisualEdge.RIGHT) max(primary, secondary) else min(primary, secondary)
    }

    protected companion object {
        const val MIXED_SOFT_WRAP_CHARACTER_WIDTH_PX = 20
    }

    protected fun preparedArtifact(generation: String): PreparedProseLayout = PreparedProseLayout(
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

    protected fun interactiveArtifact(): PreparedProseLayout {
        val base = preparedArtifact("interactive")
        return base.copy(
            heightPx = 100,
            interactions = listOf(
                base.interactions.single(),
                PreparedProseInteraction(
                    kind = PreparedProseInteraction.Kind.MENTION,
                    rects = listOf(Rect(30, 40, 50, 60)),
                    visibleText = "@Ada",
                    docPos = 1,
                    label = "@Ada",
                    attrsJson = "{}",
                ),
            ),
            accessibilityNodes = listOf(
                base.accessibilityNodes.single(),
                PreparedProseAccessibilityNode(
                    interactionIndex = 1,
                    role = PreparedProseAccessibilityNode.Role.MENTION,
                    label = "@Ada",
                    bounds = Rect(30, 40, 50, 60),
                ),
            ),
        )
    }

    protected fun tap(view: PreparedProseDrawingView, x: Float, y: Float): Boolean {
        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, x, y, 0)
        val up = MotionEvent.obtain(0, 1, MotionEvent.ACTION_UP, x, y, 0)
        return try {
            val consumedDown = view.onTouchEvent(down)
            val consumedUp = view.onTouchEvent(up)
            consumedDown && consumedUp
        } finally {
            down.recycle()
            up.recycle()
        }
    }

    protected fun mountVisible(
        parent: CapturingAccessibilityParent,
        child: View,
        width: Int = 100,
        height: Int = 100,
    ) {
        parent.addView(child)
        (child as? PreparedProseDrawingView)?.accessibilityVisibilityForTesting = { true }
        parent.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
        )
        parent.layout(0, 0, width, height)
        child.layout(0, 0, width, height)
    }

    protected class CapturingAccessibilityParent(context: android.content.Context) : ViewGroup(context) {
        init {
            shadowOf(context.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        }

        val eventTypes = mutableListOf<Int>()
        private val changeTypes = mutableListOf<Int>()
        var onEvent: ((AccessibilityEvent) -> Unit)? = null

        override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            onEvent?.invoke(event)
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
