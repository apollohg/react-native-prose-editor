package com.apollohg.editor

import android.graphics.RectF
import android.text.Annotation
import android.text.Selection
import android.text.Spanned
import org.json.JSONObject

internal fun EditorEditText.caretRectImpl(): RectF? {
    if (isCollapsedAtomBoundarySelection(selectionStart, selectionEnd)) return null
    val textLayout = layout ?: return null
    val selectionOffset = selectionEnd.takeIf { it >= 0 } ?: return null
    val clampedOffset = selectionOffset.coerceIn(0, textLayout.text.length)
    val caretLeft = textLayout.getPrimaryHorizontal(clampedOffset)
    // Clip the caret to the rendered glyph height so a ParagraphSpacerSpan's
    // inflated descent does not stretch it into the inter-block gap.
    val bounds = CaretGeometry.verticalBounds(
        textLayout,
        clampedOffset,
        paint,
        textLayout.text,
    )
    val left = totalPaddingLeft + caretLeft - scrollX
    val top = totalPaddingTop + bounds.top - scrollY
    val bottom = totalPaddingTop + bounds.bottom - scrollY
    return RectF(left, top, left + 1f, bottom)
}

internal fun EditorEditText.isCollapsedAtomBoundarySelection(start: Int, end: Int): Boolean {
    if (start != end) return false
    val content = text as? Spanned ?: return false
    if (start < 0 || start > content.length) return false
    if (start < content.length) {
        val beginsWithAtom = content
            .getSpans(start, start + 1, AtomBlockSpan::class.java)
            .any { content.getSpanStart(it) == start }
        if (beginsWithAtom) return true
    }
    if (start > 0) {
        return content
            .getSpans(start - 1, start, AtomBlockSpan::class.java)
            .any { content.getSpanEnd(it) == start }
    }
    return false
}

internal fun EditorEditText.updateAtomBoundaryCursorVisibility() {
    val shouldShowCursor = !isCollapsedAtomBoundarySelection(selectionStart, selectionEnd)
    if (isCursorVisible != shouldShowCursor) {
        isCursorVisible = shouldShowCursor
        invalidate()
    }
}

internal fun EditorEditText.restoreSelectionFromAtomBoundaryIfNeeded(start: Int, end: Int): Boolean {
    if (!isCollapsedAtomBoundarySelection(start, end)) {
        if (start >= 0 && end >= 0) {
            lastAllowedAtomCaretSelection = start to end
        }
        updateAtomBoundaryCursorVisibility()
        return false
    }

    updateAtomBoundaryCursorVisibility()
    if (!isApplyingRustState) {
        val editable = text
        val allowed = lastAllowedAtomCaretSelection
        if (
            editable != null &&
            allowed != null &&
            allowed.first in 0..editable.length &&
            allowed.second in 0..editable.length &&
            !isCollapsedAtomBoundarySelection(allowed.first, allowed.second)
        ) {
            runWithTransientInputMutationGuard {
                Selection.setSelection(editable, allowed.first, allowed.second)
                true
            }
        }
    }
    return true
}

internal fun EditorEditText.canonicalListCaretOffset(selStart: Int, selEnd: Int): Int? {
    if (selStart != selEnd) return null
    val content = text as? Spanned ?: return null
    if (selStart !in 0 until content.length) return null
    return content
        .getSpans(selStart, selStart + 1, Annotation::class.java)
        .firstOrNull { annotation ->
            annotation.key == RenderBridge.NATIVE_LIST_MARKER_ANNOTATION &&
                content.getSpanStart(annotation) <= selStart &&
                content.getSpanEnd(annotation) > selStart
        }
        ?.let(content::getSpanEnd)
}

internal fun EditorEditText.syncCurrentSelectionToRust() {
    if (!hasLiveEditor()) return

    val currentText = text?.toString() ?: ""
    if (currentText != lastAuthorizedText) return
    val (scalarAnchor, scalarHead) = currentLogicalScalarSelection()
        ?: rawScalarSelection(currentText)
        ?: return

    v2Driver?.let { driver ->
        val sync = driver.syncSelection(scalarAnchor, scalarHead)
        if (sync != null) {
            sync.refreshedUpdateJson?.let { applyRustUpdateJSON(it) }
            editorListener?.onSelectionChanged(sync.docAnchor, sync.docHead)
        }
        return
    }
    onSetSelectionScalarInRustForTesting?.invoke(scalarAnchor, scalarHead)
}

internal fun EditorEditText.currentScalarSelectionImpl(): Pair<Int, Int>? {
    currentLogicalScalarSelection()?.let { return it }
    val currentText = text?.toString() ?: return null
    return normalizedScalarSelectionRange(currentText)
}

internal fun EditorEditText.currentLogicalScalarSelectionForInputImpl(): Pair<Int, Int>? =
    currentLogicalScalarSelection()

internal fun EditorEditText.renderedRangeContainsGeneratedStructureImpl(start: Int, endExclusive: Int): Boolean {
    if (start >= endExclusive) return false
    val content = text as? Spanned ?: return false
    return content.getSpans(start, endExclusive, Annotation::class.java).any { annotation ->
        isGeneratedStructureAnnotation(annotation)
    }
}

internal fun EditorEditText.compositionContentRangeForEditorImpl(start: Int, end: Int): Pair<Int, Int>? {
    val content = text as? Spanned ?: return null
    var contentStart = minOf(start, end).coerceIn(0, content.length)
    var contentEnd = maxOf(start, end).coerceIn(0, content.length)
    val generatedSpans = content
        .getSpans(0, content.length, Annotation::class.java)
        .filter(::isGeneratedStructureAnnotation)

    var changed: Boolean
    do {
        changed = false
        generatedSpans.forEach { annotation ->
            val spanStart = content.getSpanStart(annotation)
            val spanEnd = content.getSpanEnd(annotation)
            if (spanStart <= contentStart && spanEnd > contentStart && spanEnd <= contentEnd) {
                contentStart = spanEnd
                changed = true
            }
            if (spanStart >= contentStart && spanStart < contentEnd && spanEnd >= contentEnd) {
                contentEnd = spanStart
                changed = true
            }
        }
    } while (changed)

    val containsInteriorStructure = generatedSpans.any { annotation ->
        content.getSpanStart(annotation) < contentEnd &&
            content.getSpanEnd(annotation) > contentStart
    }
    if (containsInteriorStructure) return null
    return contentStart to contentEnd
}

internal fun EditorEditText.isGeneratedStructureAnnotation(annotation: Annotation): Boolean =
    annotation.key == RenderBridge.NATIVE_INTER_BLOCK_SEPARATOR_ANNOTATION ||
        annotation.key == RenderBridge.NATIVE_LIST_MARKER_ANNOTATION ||
        annotation.key == RenderBridge.NATIVE_SYNTHETIC_PLACEHOLDER_ANNOTATION

internal fun EditorEditText.normalizedUtf16SelectionRange(currentText: String): Pair<Int, Int>? {
    val start = selectionStart
    val end = selectionEnd
    if (start < 0 || end < 0) return null
    val clampedStart = start.coerceIn(0, currentText.length)
    val clampedEnd = end.coerceIn(0, currentText.length)
    return minOf(clampedStart, clampedEnd) to maxOf(clampedStart, clampedEnd)
}

internal fun EditorEditText.normalizedUtf16SelectionRange(): Pair<Int, Int>? {
    val currentText = text?.toString() ?: return null
    return normalizedUtf16SelectionRange(currentText)
}

internal fun EditorEditText.normalizedScalarSelectionRange(currentText: String): Pair<Int, Int>? {
    currentLogicalScalarSelection()?.let { (anchor, head) ->
        return minOf(anchor, head) to maxOf(anchor, head)
    }
    val (start, end) = normalizedUtf16SelectionRange(currentText) ?: return null
    val (snappedStart, snappedEnd) = if (start == end) {
        val snapped = PositionBridge.snapToScalarBoundary(
            start,
            currentText,
            biasForward = true
        )
        snapped to snapped
    } else {
        PositionBridge.snapRangeToScalarBoundaries(start, end, currentText)
    }
    return PositionBridge.utf16ToScalar(snappedStart, currentText) to
        PositionBridge.utf16ToScalar(snappedEnd, currentText)
}

internal fun EditorEditText.currentLogicalScalarSelection(): Pair<Int, Int>? {
    val snapshot = logicalSelectionSnapshot ?: return null
    if (selectionStart == snapshot.utf16Anchor && selectionEnd == snapshot.utf16Head) {
        return snapshot.scalarAnchor to snapshot.scalarHead
    }
    recordImeTraceForTesting(
        "logicalSelectionInvalidated",
        "scalar=${snapshot.scalarAnchor}..${snapshot.scalarHead} projected=${snapshot.utf16Anchor}..${snapshot.utf16Head} physical=$selectionStart..$selectionEnd revision=${snapshot.documentVersion ?: "pending"}"
    )
    logicalSelectionSnapshot = null
    return null
}

internal fun EditorEditText.rememberLogicalSelection(
    scalarAnchor: Int,
    scalarHead: Int,
    utf16Anchor: Int,
    utf16Head: Int,
    documentVersion: String? = logicalSelectionSnapshot?.documentVersion
) {
    logicalSelectionSnapshot = LogicalSelectionSnapshot(
        scalarAnchor = scalarAnchor,
        scalarHead = scalarHead,
        utf16Anchor = utf16Anchor,
        utf16Head = utf16Head,
        documentVersion = documentVersion
    )
}

internal fun EditorEditText.rawScalarSelection(currentText: String): Pair<Int, Int>? {
    val anchor = selectionStart
    val head = selectionEnd
    if (anchor < 0 || head < 0) return null
    val clampedAnchor = anchor.coerceIn(0, currentText.length)
    val clampedHead = head.coerceIn(0, currentText.length)
    if (clampedAnchor == clampedHead) {
        val snapped = PositionBridge.snapToScalarBoundary(
            clampedAnchor,
            currentText,
            biasForward = true
        )
        val scalar = PositionBridge.utf16ToScalar(snapped, currentText)
        return scalar to scalar
    }
    val (rangeStart, rangeEnd) = PositionBridge.snapRangeToScalarBoundaries(
        minOf(clampedAnchor, clampedHead),
        maxOf(clampedAnchor, clampedHead),
        currentText
    )
    val snappedAnchor = if (clampedAnchor < clampedHead) rangeStart else rangeEnd
    val snappedHead = if (clampedAnchor < clampedHead) rangeEnd else rangeStart
    return PositionBridge.utf16ToScalar(snappedAnchor, currentText) to
        PositionBridge.utf16ToScalar(snappedHead, currentText)
}

    /**
     * Apply a selection from a parsed JSON selection object.
     *
     * The selection JSON matches the format from `serialize_editor_update`:
     * ```json
     * {"type": "text", "anchor": 5, "head": 5}
     * {"type": "node", "pos": 10}
     * {"type": "all"}
     * ```
     *
     * anchor/head from Rust are **document positions** (include structural tokens).
     * We convert doc→scalar via the v2 driver ([EditorV2Driver.scalarPositionForDoc])
     * before converting to UTF-16.
     */
internal fun EditorEditText.applySelectionFromJSON(
    selection: org.json.JSONObject,
    documentVersion: String?
) {
    val type = selection.optString("type", "") ?: return
    if (isEditorDestroyedForInput()) {
        recordImeTraceForTesting("applySelectionFromJSONSkipped", "reason=destroyed type=$type")
        return
    }

    isApplyingRustState = true
    try {
        val currentText = text?.toString() ?: ""
        when (type) {
            "text" -> {
                val docAnchor = exactV2ScalarInt(selection.opt("anchor") as? Number) ?: return
                val docHead = exactV2ScalarInt(selection.opt("head") as? Number) ?: return
                // The frozen v2 update includes exact scalar positions alongside its
                // structural document positions. Prefer those values: a cursor inside
                // an empty trailing block sits at a document boundary that cannot be
                // reconstructed from rendered structural tokens alone.
                val selectionDriver = v2Driver ?: return
                val scalarAnchor = exactV2ScalarInt(selection.opt("anchorScalar") as? Number)
                    ?: selectionDriver.scalarPositionForDoc(docAnchor)
                    ?: docAnchor
                val scalarHead = exactV2ScalarInt(selection.opt("headScalar") as? Number)
                    ?: selectionDriver.scalarPositionForDoc(docHead)
                    ?: docHead
                val anchorUtf16 = PositionBridge.scalarToUtf16(scalarAnchor, currentText)
                val headUtf16 = PositionBridge.scalarToUtf16(scalarHead, currentText)
                val len = text?.length ?: 0
                recordImeTraceForTesting(
                    "applySelectionFromJSON",
                    "doc=$docAnchor..$docHead scalar=$scalarAnchor..$scalarHead utf16=$anchorUtf16..$headUtf16 len=$len"
                )
                setSelection(
                    anchorUtf16.coerceIn(0, len),
                    headUtf16.coerceIn(0, len)
                )
                rememberLogicalSelection(
                    scalarAnchor = scalarAnchor,
                    scalarHead = scalarHead,
                    utf16Anchor = selectionStart,
                    utf16Head = selectionEnd,
                    documentVersion = documentVersion
                )
            }
            "node" -> {
                logicalSelectionSnapshot = null
                val docPos = exactV2ScalarInt(selection.opt("pos") as? Number) ?: return
                // Convert doc position to scalar offset.
                val nodeSelectionDriver = v2Driver ?: return
                val scalarPos = nodeSelectionDriver.scalarPositionForDoc(docPos) ?: docPos
                val startUtf16 = PositionBridge.scalarToUtf16(scalarPos, currentText)
                val len = text?.length ?: 0
                val clamped = startUtf16.coerceIn(0, len)
                // Select one character (the void node placeholder).
                val endClamped = (clamped + 1).coerceAtMost(len)
                setSelection(clamped, endClamped)
            }
            "all" -> {
                logicalSelectionSnapshot = null
                selectAll()
            }
        }
    } finally {
        isApplyingRustState = false
    }
}
