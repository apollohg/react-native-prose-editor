package com.apollohg.editor

import com.apollohg.editor.EditorEditText.Companion.EMPTY_BLOCK_PLACEHOLDER

/**
     * Handle surrounding text deletion from the IME.
     *
     * Called by [EditorInputConnection.deleteSurroundingText].
     *
     * @param beforeLength Number of UTF-16 code units to delete before the cursor.
     * @param afterLength Number of UTF-16 code units to delete after the cursor.
     */
internal fun EditorEditText.handleDeleteImpl(beforeLength: Int, afterLength: Int) {
    if (!isEditable) return
    if (isApplyingRustState) return
    val selectionRange = normalizedUtf16SelectionRange()
    if (selectionRange != null && isCollapsedAtomBoundarySelection(selectionRange.first, selectionRange.second)) {
        return
    }
    if (editorId == 0L) {
        // Dev mode: direct editing.
        val editable = this.text ?: return
        val (selectionStart, selectionEnd) = selectionRange ?: return
        val delStart: Int
        val delEnd: Int
        if (selectionStart != selectionEnd) {
            delStart = selectionStart
            delEnd = selectionEnd
        } else {
            delStart = maxOf(0, selectionStart - beforeLength.coerceAtLeast(0))
            delEnd = minOf(editable.length, selectionStart + afterLength.coerceAtLeast(0))
        }
        editable.delete(delStart, delEnd)
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return

    val currentText = text?.toString() ?: ""
    val (selectionStart, selectionEnd) = selectionRange ?: return
    if (selectionStart != selectionEnd) {
        val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
        deleteRangeInRust(scalarStart, scalarEnd)
        return
    }
    val cursor = selectionStart
    if (beforeLength > 0 &&
        afterLength == 0 &&
        cursor > 0 &&
        currentText.getOrNull(cursor - 1) == EMPTY_BLOCK_PLACEHOLDER
    ) {
        val scalarCursor = PositionBridge.utf16ToScalar(cursor, currentText)
        deleteBackwardAtSelectionScalarInRust(scalarCursor, scalarCursor)
        return
    }
    val rawDelStart = maxOf(0, cursor - beforeLength.coerceAtLeast(0))
    val rawDelEnd = minOf(currentText.length, cursor + afterLength.coerceAtLeast(0))
    val (delStart, delEnd) = PositionBridge.snapRangeToScalarBoundaries(
        rawDelStart,
        rawDelEnd,
        currentText
    )

    val scalarStart = PositionBridge.utf16ToScalar(delStart, currentText)
    val scalarEnd = PositionBridge.utf16ToScalar(delEnd, currentText)

    if (scalarStart < scalarEnd) {
        deleteRangeInRust(scalarStart, scalarEnd)
    } else if (beforeLength > 0 && afterLength == 0) {
        deleteBackwardAtSelectionScalarInRust(scalarEnd, scalarEnd)
    }
}

    /**
     * Handle backspace key press (hardware keyboard or key event).
     *
     * If there's a range selection, deletes the range. Otherwise deletes
     * the grapheme cluster before the cursor.
     */
internal fun EditorEditText.handleBackspaceImpl() {
    if (!isEditable) return
    if (isApplyingRustState) return
    val selectionRange = normalizedUtf16SelectionRange() ?: return
    if (isCollapsedAtomBoundarySelection(selectionRange.first, selectionRange.second)) return
    if (editorId == 0L) {
        // Dev mode: direct editing.
        val editable = this.text ?: return
        val (start, end) = selectionRange
        if (start != end) {
            editable.delete(start, end)
        } else if (start > 0) {
            // Delete one grapheme cluster backward.
            val prevBoundary = PositionBridge.snapToGraphemeBoundary(start - 1, text?.toString() ?: "")
            val adjustedPrev = if (prevBoundary >= start) maxOf(0, start - 1) else prevBoundary
            editable.delete(adjustedPrev, start)
        }
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return

    val currentText = text?.toString() ?: ""
    val (start, end) = selectionRange
    val logicalSelection = currentLogicalScalarSelection()

    if (start != end) {
        // Range selection: delete the range.
        val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
        deleteRangeInRust(scalarStart, scalarEnd)
    } else if (logicalSelection != null && logicalSelection.first == logicalSelection.second) {
        deleteBackwardAtSelectionScalarInRust(logicalSelection.first, logicalSelection.second)
    } else if (start > 0) {
        if (currentText.getOrNull(start - 1) == EMPTY_BLOCK_PLACEHOLDER) {
            val scalarCursor = PositionBridge.utf16ToScalar(start, currentText)
            deleteBackwardAtSelectionScalarInRust(scalarCursor, scalarCursor)
            return
        }
        // Cursor: delete one grapheme cluster backward.
        // Find the previous grapheme boundary by snapping (start - 1).
        val breakIter = java.text.BreakIterator.getCharacterInstance()
        breakIter.setText(currentText)
        val prevBoundary = breakIter.preceding(start)
        val prevUtf16 = if (prevBoundary == java.text.BreakIterator.DONE) 0 else prevBoundary

        val scalarStart = PositionBridge.utf16ToScalar(prevUtf16, currentText)
        val scalarEnd = PositionBridge.utf16ToScalar(start, currentText)
        if (scalarStart < scalarEnd) {
            deleteRangeInRust(scalarStart, scalarEnd)
        } else {
            deleteBackwardAtSelectionScalarInRust(scalarEnd, scalarEnd)
        }
    } else {
        deleteBackwardAtSelectionScalarInRust(0, 0)
    }
}

internal fun EditorEditText.handleForwardDeleteImpl() {
    if (!isEditable) return
    if (isApplyingRustState) return
    val selectionRange = normalizedUtf16SelectionRange() ?: return
    if (isCollapsedAtomBoundarySelection(selectionRange.first, selectionRange.second)) return
    if (editorId == 0L) {
        val editable = this.text ?: return
        val (start, end) = selectionRange
        if (start != end) {
            editable.delete(start, end)
        } else if (start < editable.length) {
            val breakIter = java.text.BreakIterator.getCharacterInstance()
            breakIter.setText(editable.toString())
            val nextBoundary = breakIter.following(start)
            val nextUtf16 = if (nextBoundary == java.text.BreakIterator.DONE) {
                editable.length
            } else {
                nextBoundary
            }
            editable.delete(start, nextUtf16.coerceIn(start, editable.length))
        }
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return

    val currentText = text?.toString() ?: ""
    val (start, end) = selectionRange
    if (start != end) {
        val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
        deleteRangeInRust(scalarStart, scalarEnd)
    } else if (start < currentText.length) {
        val breakIter = java.text.BreakIterator.getCharacterInstance()
        breakIter.setText(currentText)
        val nextBoundary = breakIter.following(start)
        val nextUtf16 = if (nextBoundary == java.text.BreakIterator.DONE) {
            currentText.length
        } else {
            nextBoundary
        }
        val (utf16Start, utf16End) = PositionBridge.snapRangeToScalarBoundaries(
            start,
            nextUtf16.coerceIn(start, currentText.length),
            currentText
        )
        val scalarStart = PositionBridge.utf16ToScalar(utf16Start, currentText)
        val scalarEnd = PositionBridge.utf16ToScalar(utf16End, currentText)
        if (scalarStart < scalarEnd) {
            deleteRangeInRust(scalarStart, scalarEnd)
        }
    }
}

    /**
     * Handle return/enter key as a block split operation.
     */
internal fun EditorEditText.handleReturnKeyImpl() {
    if (!isEditable) return
    if (isApplyingRustState) return

    val currentText = text?.toString() ?: ""
    val (start, end) = normalizedUtf16SelectionRange() ?: return
    if (isCollapsedAtomBoundarySelection(start, end)) return

    if (editorId == 0L) {
        // Dev mode: insert newline directly.
        val editable = this.text ?: return
        editable.replace(start, end, "\n")
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return

    val (scalarStart, scalarEnd) = normalizedScalarSelectionRange(currentText) ?: return
    if (scalarStart != scalarEnd) {
        // Range selection: atomic delete-and-split via Rust.
        deleteAndSplitInRust(scalarStart, scalarEnd)
    } else {
        splitBlockInRust(scalarEnd)
    }
}

    /**
     * Handle Shift+Enter as an inline hard break insertion.
     */
internal fun EditorEditText.handleHardBreakImpl() {
    if (!isEditable) return
    if (isApplyingRustState) return
    if (isCollapsedAtomBoundarySelection(selectionStart, selectionEnd)) return

    if (editorId == 0L) {
        val editable = this.text ?: return
        val start = selectionStart
        val end = selectionEnd
        editable.replace(start, end, "\n")
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return

    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.insertNode(preferredHardBreakNodeType(), selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

    /**
     * Handle hardware Tab / Shift+Tab as list indent / outdent when the caret is in a list.
     */
internal fun EditorEditText.handleTabImpl(shiftPressed: Boolean): Boolean {
    if (!isEditable) return false
    if (isApplyingRustState) return false
    if (!hasLiveEditor()) return false
    if (!isSelectionInsideList()) return false
    val selection = currentScalarSelection() ?: return false

    v2Driver?.let { driver ->
        val update = if (shiftPressed) {
            driver.outdentListItem(selection.first, selection.second)
        } else {
            driver.indentListItem(selection.first, selection.second)
        }
        update?.let { applyUpdateJSON(it) }
    }
    return true
}
