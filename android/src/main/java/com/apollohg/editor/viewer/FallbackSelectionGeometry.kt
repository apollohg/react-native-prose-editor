package com.apollohg.editor.viewer

import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.graphics.Path
import android.graphics.PathMeasure
import android.graphics.Typeface
import android.text.Layout
import android.text.Spanned
import android.text.SpannableString
import android.text.StaticLayout
import android.text.TextPaint
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LineHeightSpan
import android.text.style.MetricAffectingSpan
import android.text.style.StrikethroughSpan
import android.text.style.UnderlineSpan
import android.text.style.ReplacementSpan
import com.apollohg.editor.EditorLinkTheme
import com.apollohg.editor.EditorMentionTheme
import com.apollohg.editor.EditorOrderedListMarkerTheme
import com.apollohg.editor.EditorTextStyle
import com.apollohg.editor.EditorTheme
import com.apollohg.editor.OrderedListMarkerFormatter
import com.apollohg.editor.ProseViewerError
import java.text.Bidi
import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.min

/**
 * StaticLayout can split one visual selection run into edge-touching contours.
 * Android [Rect.right] is exclusive, so only overlapping or edge-touching
 * contours with compatible vertical bounds belong to one hit region.
 */
internal fun mergeAdjacentSameLineSelectionFragments(fragments: List<Rect>): List<Rect> {
    val ordered = fragments.sortedWith(compareBy<Rect> { it.top }.thenBy { it.left })
    val merged = mutableListOf<Rect>()
    ordered.forEach { fragment ->
        val previous = merged.lastOrNull()
        if (
            previous != null &&
            abs(previous.top - fragment.top) <= SELECTION_FRAGMENT_PIXEL_TOLERANCE_PX &&
            abs(previous.bottom - fragment.bottom) <= SELECTION_FRAGMENT_PIXEL_TOLERANCE_PX &&
            fragment.left <= previous.right
        ) {
            merged[merged.lastIndex] = Rect(
                min(previous.left, fragment.left),
                min(previous.top, fragment.top),
                max(previous.right, fragment.right),
                max(previous.bottom, fragment.bottom),
            )
        } else {
            merged += Rect(fragment)
        }
    }
    return merged
}

private const val SELECTION_FRAGMENT_PIXEL_TOLERANCE_PX = 1

internal enum class FallbackVisualEdge { LEFT, RIGHT }

/**
 * Java [Bidi]'s run accessors are indexed in logical order. Keep that source
 * identity immutable, then explicitly project it into visual order before any
 * geometry asks about neighbours or line-edge ownership.
 */
internal data class FallbackLogicalBidiRun(
    val logicalIndex: Int,
    val documentStart: Int,
    val documentEnd: Int,
    val level: Byte,
) {
    val isRtl: Boolean get() = (level.toInt() and 1) == 1
}

internal data class FallbackVisualBidiRun(
    val visualIndex: Int,
    val logicalRun: FallbackLogicalBidiRun,
) {
    val documentStart: Int get() = logicalRun.documentStart
    val documentEnd: Int get() = logicalRun.documentEnd
    val isRtl: Boolean get() = logicalRun.isRtl

    fun offsetAt(edge: FallbackVisualEdge): Int = when (edge) {
        FallbackVisualEdge.LEFT -> if (isRtl) documentEnd else documentStart
        FallbackVisualEdge.RIGHT -> if (isRtl) documentStart else documentEnd
    }

    fun edgeForLogicalEnd(): FallbackVisualEdge =
        if (isRtl) FallbackVisualEdge.LEFT else FallbackVisualEdge.RIGHT
}

/**
 * [Bidi.getRunStart], [Bidi.getRunLimit], and [Bidi.getRunLevel] take a
 * logical run index. [Bidi.reorderVisually] is the public Unicode Bidi API
 * that turns those immutable logical records into left-to-right visual order.
 */
internal fun visualBidiRuns(logicalRuns: List<FallbackLogicalBidiRun>): List<FallbackVisualBidiRun> {
    if (logicalRuns.isEmpty()) return emptyList()

    val levels = ByteArray(logicalRuns.size) { logicalRuns[it].level }
    val reordered: Array<Any> = Array(logicalRuns.size) { logicalRuns[it] }
    Bidi.reorderVisually(levels, 0, reordered, 0, reordered.size)
    return reordered.mapIndexed { visualIndex, value ->
        FallbackVisualBidiRun(visualIndex, value as FallbackLogicalBidiRun)
    }
}

private fun fallbackHorizontalAtVisualEdge(
    layout: StaticLayout,
    offset: Int,
    edge: FallbackVisualEdge,
): Float {
    val primary = layout.getPrimaryHorizontal(offset)
    val secondary = layout.getSecondaryHorizontal(offset)
    return if (edge == FallbackVisualEdge.RIGHT) max(primary, secondary) else min(primary, secondary)
}

/**
 * The logical side of a cursor position. This mirrors Layout's internal
 * `getHorizontal(offset, trailing, ...)` calls: a selection starts at the
 * leading edge of its next logical character and ends at the trailing edge of
 * its previous logical character. It is deliberately not a visual left/right
 * concept.
 */
internal enum class FallbackLogicalCaretAffinity { LEADING_NEXT, TRAILING_PREVIOUS }

internal fun FallbackVisualBidiRun.affinityAt(edge: FallbackVisualEdge): FallbackLogicalCaretAffinity =
    if (offsetAt(edge) == documentStart) {
        FallbackLogicalCaretAffinity.LEADING_NEXT
    } else {
        FallbackLogicalCaretAffinity.TRAILING_PREVIOUS
    }

/**
 * Mirrors Layout's primary-caret decision from the adjacent logical embedding
 * levels. At a directional boundary the same offset has two valid positions;
 * which one is primary is determined by the transition, not by the current
 * run's direction or by the paragraph's base direction alone.
 */
internal fun primaryIsTrailingPrevious(offset: Int, geometry: FallbackLineGeometry): Boolean {
    val paragraphLevel = if (geometry.paragraphDirection == Layout.DIR_RIGHT_TO_LEFT) 1 else 0
    val current = geometry.logicalRuns.firstOrNull {
        offset in it.documentStart until it.documentEnd
    }
    // Inside a logical run the public primary cursor is the leading-next
    // position. Only a run boundary needs the preceding-level comparison.
    if (current != null && offset > current.documentStart) return false

    val levelAt = current?.level?.toInt() ?: paragraphLevel
    val levelBefore = when {
        offset == geometry.lineStart -> paragraphLevel
        else -> geometry.logicalRuns.firstOrNull {
            offset - 1 in it.documentStart until it.documentEnd
        }?.level?.toInt() ?: paragraphLevel
    }
    return levelBefore < levelAt
}

/** Resolves one public horizontal API result using Layout-equivalent affinity. */
internal fun fallbackHorizontalForLogicalCaret(
    geometry: FallbackLineGeometry,
    offset: Int,
    affinity: FallbackLogicalCaretAffinity,
): Float {
    val desiredTrailing = affinity == FallbackLogicalCaretAffinity.TRAILING_PREVIOUS
    return if (desiredTrailing == primaryIsTrailingPrevious(offset, geometry)) {
        geometry.primaryHorizontal(offset)
    } else {
        geometry.secondaryHorizontal(offset)
    }
}

/**
 * Emulates Layout's private current-line trailing lookup at a soft-wrap end.
 * The public horizontal APIs resolve that shared offset as the next line's
 * start, so an internal terminal run instead borrows the same visual boundary
 * from its adjacent run. Only a run which owns the outer visual edge may use
 * the current line's left/right extent.
 */
private fun softWrapTerminalBoundary(
    terminalRun: FallbackVisualBidiRun,
    visualRuns: List<FallbackVisualBidiRun>,
    softWrapLineEnd: Int,
    outerLineBoundary: (FallbackVisualEdge) -> Float,
    logicalCaretHorizontal: (offset: Int, affinity: FallbackLogicalCaretAffinity) -> Float,
): Float? {
    val terminalEdge = terminalRun.edgeForLogicalEnd()
    val isOuter = when (terminalEdge) {
        FallbackVisualEdge.LEFT -> terminalRun.visualIndex == 0
        FallbackVisualEdge.RIGHT -> terminalRun.visualIndex == visualRuns.lastIndex
    }
    if (isOuter) {
        return outerLineBoundary(terminalEdge)
    }

    val neighbor = when (terminalEdge) {
        FallbackVisualEdge.LEFT -> visualRuns.getOrNull(terminalRun.visualIndex - 1)
        FallbackVisualEdge.RIGHT -> visualRuns.getOrNull(terminalRun.visualIndex + 1)
    } ?: return null
    val neighborEdge = if (terminalEdge == FallbackVisualEdge.LEFT) {
        FallbackVisualEdge.RIGHT
    } else {
        FallbackVisualEdge.LEFT
    }
    val neighborOffset = neighbor.offsetAt(neighborEdge)
    // A distinct visual run cannot own the terminal line-end offset. Refuse a
    // malformed/unexpected Bidi result rather than resolving that offset on
    // the next line and expanding this hit rectangle across adjacent content.
    if (neighborOffset == softWrapLineEnd) return null
    return logicalCaretHorizontal(neighborOffset, neighbor.affinityAt(neighborEdge))
}

/**
 * Test support for direct visual-edge fixtures. Production selection uses
 * [fallbackSelectionRectsForLine], which resolves both endpoints by logical
 * caret affinity instead of collapsing them to physical left/right extremes.
 */
internal fun fallbackSelectionRectForVisualRun(
    layout: StaticLayout,
    runStart: Int,
    runEnd: Int,
    runIsRtl: Boolean,
    line: Int,
    width: Int,
    softWrapLineEnd: Int? = null,
    softWrapTerminalBoundary: Float? = null,
): Rect? {
    if (runStart >= runEnd) return null

    fun visualBoundary(offset: Int, logicalRunStart: Boolean): Float {
        val visualRightEdge = if (runIsRtl) logicalRunStart else !logicalRunStart
        // A soft-wrap terminal offset is also the following line's start.
        // Layout's horizontal lookup therefore resolves it on that following
        // line. The current line's matching visual extreme is the exact
        // trailing boundary for this Bidi run. Keep ordinary primary/secondary
        // affinity lookup for a current-line start and for the final document
        // boundary, neither of which has this ambiguous line ownership.
        if (!logicalRunStart && offset == softWrapLineEnd) {
            softWrapTerminalBoundary?.let { return it }
            return if (visualRightEdge) layout.getLineRight(line) else layout.getLineLeft(line)
        }
        return fallbackHorizontalAtVisualEdge(
            layout,
            offset,
            if (visualRightEdge) FallbackVisualEdge.RIGHT else FallbackVisualEdge.LEFT,
        )
    }

    val start = visualBoundary(runStart, logicalRunStart = true)
    val end = visualBoundary(runEnd, logicalRunStart = false)
    val left = kotlin.math.floor(min(start, end)).toInt().coerceIn(0, width)
    val right = ceil(max(start, end)).toInt().coerceIn(0, width)
    return Rect(left, layout.getLineTop(line), right, layout.getLineBottom(line)).takeIf { !it.isEmpty }
}

/**
 * The line-level input required by the Bidi fallback. Keeping it separate
 * from [StaticLayout] makes the cursor-affinity algorithm deterministic while
 * leaving layout extraction at the production adapter boundary.
 */
internal data class FallbackLineGeometry(
    val text: CharSequence,
    val lineStart: Int,
    val rawLineEnd: Int,
    val nextLineStart: Int?,
    val paragraphDirection: Int,
    val top: Int,
    val bottom: Int,
    val width: Int,
    /** Full drawable-line runs in logical order, including embedding levels. */
    val logicalRuns: List<FallbackLogicalBidiRun>,
    val outerLineBoundary: (FallbackVisualEdge) -> Float,
    val primaryHorizontal: (Int) -> Float,
    val secondaryHorizontal: (Int) -> Float,
)

/**
 * Derives fallback selection geometry from one already-resolved visual line.
 *
 * The [StaticLayout] adapter below supplies this geometry in production. The
 * algorithm intentionally owns the soft-wrap, selection-intersection, Bidi
 * ordering, and terminal-neighbour checks so those invariants do not depend
 * on host text shaping.
 */
internal fun fallbackSelectionRectsForGeometry(
    geometry: FallbackLineGeometry,
    start: Int,
    end: Int,
): List<Rect> {
    val lineStart = geometry.lineStart
    val rawLineEnd = geometry.rawLineEnd
    // A hard-break line includes its terminator in StaticLayout's end offset.
    // It has no drawable/cursor run, so exclude it before constructing Bidi.
    val lineEnd = if (
        rawLineEnd > lineStart &&
        rawLineEnd <= geometry.text.length &&
        geometry.text[rawLineEnd - 1] == '\n'
    ) rawLineEnd - 1 else rawLineEnd
    val softWrapLineEnd = rawLineEnd.takeIf {
        lineEnd == rawLineEnd &&
            rawLineEnd < geometry.text.length &&
            geometry.nextLineStart == rawLineEnd
    }
    val selectedStart = maxOf(start, lineStart).coerceAtMost(lineEnd)
    val selectedEnd = minOf(end, lineEnd).coerceAtLeast(lineStart)
    if (selectedStart >= selectedEnd || lineStart >= lineEnd) return emptyList()

    val visualRuns = visualBidiRuns(geometry.logicalRuns)
    val fragments = mutableListOf<Rect>()
    for (visualRun in visualRuns) {
        val intersectedStart = maxOf(selectedStart, visualRun.documentStart)
        val intersectedEnd = minOf(selectedEnd, visualRun.documentEnd)
        val resolvesSoftWrapTerminal = softWrapLineEnd != null &&
            visualRun.documentEnd == softWrapLineEnd &&
            intersectedEnd == softWrapLineEnd
        val terminalBoundary = if (resolvesSoftWrapTerminal) {
            softWrapTerminalBoundary(
                terminalRun = visualRun,
                visualRuns = visualRuns,
                softWrapLineEnd = softWrapLineEnd!!,
                outerLineBoundary = geometry.outerLineBoundary,
                logicalCaretHorizontal = { offset, affinity ->
                    fallbackHorizontalForLogicalCaret(geometry, offset, affinity)
                },
            )
        } else {
            null
        }
        // Do not allow an internal terminal boundary to fall through to the
        // generic whole-line shortcut: that would over-expand the rectangle.
        if (resolvesSoftWrapTerminal && terminalBoundary == null) continue
        if (intersectedStart >= intersectedEnd) continue
        fun visualBoundary(offset: Int, logicalRunStart: Boolean): Float {
            val visualRightEdge = if (visualRun.isRtl) logicalRunStart else !logicalRunStart
            val edge = if (visualRightEdge) FallbackVisualEdge.RIGHT else FallbackVisualEdge.LEFT
            if (!logicalRunStart && offset == softWrapLineEnd) {
                return terminalBoundary ?: geometry.outerLineBoundary(edge)
            }
            return fallbackHorizontalForLogicalCaret(
                geometry,
                offset,
                if (logicalRunStart) {
                    FallbackLogicalCaretAffinity.LEADING_NEXT
                } else {
                    FallbackLogicalCaretAffinity.TRAILING_PREVIOUS
                },
            )
        }
        val startBoundary = visualBoundary(intersectedStart, logicalRunStart = true)
        val endBoundary = visualBoundary(intersectedEnd, logicalRunStart = false)
        Rect(
            kotlin.math.floor(min(startBoundary, endBoundary)).toInt().coerceIn(0, geometry.width),
            geometry.top,
            ceil(max(startBoundary, endBoundary)).toInt().coerceIn(0, geometry.width),
            geometry.bottom,
        ).takeIf { !it.isEmpty }?.let(fragments::add)
    }
    return fragments
}

/**
 * Derives fallback selection geometry from the complete [StaticLayout] line.
 *
 * The line's Bidi resolution must use the same paragraph direction as the
 * layout that supplied its cursor positions. Resolving only the selected
 * substring can give neutral characters and embedded runs a different level.
 */
internal fun fallbackSelectionRectsForLine(
    layout: StaticLayout,
    start: Int,
    end: Int,
    line: Int,
    width: Int,
): List<Rect> = fallbackSelectionRectsForGeometry(
    FallbackLineGeometry(
        text = layout.text,
        lineStart = layout.getLineStart(line),
        rawLineEnd = layout.getLineEnd(line),
        nextLineStart = if (line + 1 < layout.lineCount) layout.getLineStart(line + 1) else null,
        paragraphDirection = layout.getParagraphDirection(line),
        top = layout.getLineTop(line),
        bottom = layout.getLineBottom(line),
        width = width,
        logicalRuns = fallbackLogicalBidiRunsForLine(layout, line),
        outerLineBoundary = { edge ->
            if (edge == FallbackVisualEdge.LEFT) layout.getLineLeft(line) else layout.getLineRight(line)
        },
        primaryHorizontal = layout::getPrimaryHorizontal,
        secondaryHorizontal = layout::getSecondaryHorizontal,
    ),
    start,
    end,
)

private fun fallbackLogicalBidiRunsForLine(
    layout: StaticLayout,
    line: Int,
): List<FallbackLogicalBidiRun> {
    val lineStart = layout.getLineStart(line)
    val rawLineEnd = layout.getLineEnd(line)
    val lineEnd = if (rawLineEnd > lineStart && layout.text[rawLineEnd - 1] == '\n') {
        rawLineEnd - 1
    } else {
        rawLineEnd
    }
    if (lineStart >= lineEnd) return emptyList()
    val direction = if (layout.getParagraphDirection(line) == Layout.DIR_RIGHT_TO_LEFT) {
        Bidi.DIRECTION_RIGHT_TO_LEFT
    } else {
        Bidi.DIRECTION_LEFT_TO_RIGHT
    }
    val bidi = Bidi(layout.text.subSequence(lineStart, lineEnd).toString(), direction)
    return List(bidi.runCount) { logicalIndex ->
        FallbackLogicalBidiRun(
            logicalIndex = logicalIndex,
            documentStart = lineStart + bidi.getRunStart(logicalIndex),
            documentEnd = lineStart + bidi.getRunLimit(logicalIndex),
            level = bidi.getRunLevel(logicalIndex).toByte(),
        )
    }
}
