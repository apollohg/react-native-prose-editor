package com.apollohg.editor

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.text.Selection
import android.text.Spanned
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper


internal data class SurroundingDeleteRange(
    val utf16Start: Int,
    val utf16End: Int,
    val scalarStart: Int,
    val scalarEnd: Int
)


internal fun EditorInputConnection.shouldDeferPlainSurroundingDelete(beforeLength: Int, afterLength: Int): Boolean =
    beforeLength.coerceAtLeast(0) + afterLength.coerceAtLeast(0) > 0

internal fun EditorInputConnection.performMappedCompositionSurroundingDelete(
    beforeLength: Int,
    afterLength: Int,
    deleteInCodePoints: Boolean,
): Boolean {
    val mapper = currentMapper() ?: return true
    val rawStart = editorView.selectionStart
    val rawEnd = editorView.selectionEnd
    if (rawStart < 0 || rawEnd < 0) return true
    val imeStart = mapper.rawToIme(minOf(rawStart, rawEnd))
    val imeEnd = mapper.rawToIme(maxOf(rawStart, rawEnd))
    val visibleText = mapper.visibleText.toString()
    val beforeUtf16Length = if (deleteInCodePoints) {
        EditorInputConnection.codePointsToUtf16Length(visibleText, imeStart, beforeLength, forward = false)
    } else {
        beforeLength
    }
    val afterUtf16Length = if (deleteInCodePoints) {
        EditorInputConnection.codePointsToUtf16Length(visibleText, imeEnd, afterLength, forward = true)
    } else {
        afterLength
    }
    val imeDeleteStart = if (imeStart != imeEnd) {
        imeStart
    } else {
        maxOf(0, imeStart - beforeUtf16Length.coerceAtLeast(0))
    }
    val imeDeleteEnd = if (imeStart != imeEnd) {
        imeEnd
    } else {
        minOf(visibleText.length, imeEnd + afterUtf16Length.coerceAtLeast(0))
    }
    val rawDeleteStart = mapper.imeToRaw(
        imeDeleteStart,
        ImeTextCoordinateMapper.Affinity.AFTER,
    )
    val rawDeleteEnd = mapper.imeToRaw(
        imeDeleteEnd,
        ImeTextCoordinateMapper.Affinity.BEFORE,
    )
    editorView.runWithTransientInputMutationGuard {
        deleteVisibleTextInRawRange(rawDeleteStart, rawDeleteEnd, imeDeleteStart)
    }
    refreshComposingTextFromEditable()
    return true
}

internal fun EditorInputConnection.performDeferredPlainSurroundingDelete(
    beforeLength: Int,
    afterLength: Int,
    deleteInCodePoints: Boolean
): Boolean {
    val beforeText = editorView.text?.toString() ?: return true
    val mapper = currentMapper() ?: return true
    val rawSelectionStart = editorView.selectionStart
    val rawSelectionEnd = editorView.selectionEnd
    if (rawSelectionStart < 0 || rawSelectionEnd < 0) return true
    val normalizedRawStart = minOf(rawSelectionStart, rawSelectionEnd)
        .coerceIn(0, beforeText.length)
    val normalizedRawEnd = maxOf(rawSelectionStart, rawSelectionEnd)
        .coerceIn(0, beforeText.length)
    val imeSelectionStart = mapper.rawToIme(normalizedRawStart)
    val imeSelectionEnd = mapper.rawToIme(normalizedRawEnd)
    val beforeUtf16Length: Int
    val afterUtf16Length: Int
    if (deleteInCodePoints) {
        val visibleText = mapper.visibleText.toString()
        beforeUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = visibleText,
            fromUtf16Offset = imeSelectionStart,
            codePointCount = beforeLength,
            forward = false
        )
        afterUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = visibleText,
            fromUtf16Offset = imeSelectionEnd,
            codePointCount = afterLength,
            forward = true
        )
    } else {
        beforeUtf16Length = beforeLength
        afterUtf16Length = afterLength
    }
    val imeDeleteStart: Int
    val imeDeleteEnd: Int
    if (imeSelectionStart != imeSelectionEnd) {
        imeDeleteStart = imeSelectionStart
        imeDeleteEnd = imeSelectionEnd
    } else {
        imeDeleteStart = maxOf(0, imeSelectionStart - beforeUtf16Length.coerceAtLeast(0))
        imeDeleteEnd = minOf(
            mapper.visibleText.length,
            imeSelectionEnd + afterUtf16Length.coerceAtLeast(0),
        )
    }
    val rawDeleteStart = mapper.imeToRaw(
        imeDeleteStart,
        ImeTextCoordinateMapper.Affinity.AFTER,
    )
    val rawDeleteEnd = mapper.imeToRaw(
        imeDeleteEnd,
        ImeTextCoordinateMapper.Affinity.BEFORE,
    )
    val deleteRange = surroundingDeleteRange(
        text = beforeText,
        rawDeleteStart = rawDeleteStart,
        rawDeleteEnd = rawDeleteEnd,
        selectionStart = normalizedRawStart,
        selectionEnd = normalizedRawEnd,
    )
    val isCollapsedBackwardDelete =
        beforeLength == 1 &&
            afterLength == 0 &&
            editorView.selectionStart == editorView.selectionEnd

    if (isCollapsedBackwardDelete) {
        val hiddenGapStart = mapper.imeToRaw(
            imeSelectionStart,
            ImeTextCoordinateMapper.Affinity.BEFORE,
        )
        if (
            hiddenGapStart < normalizedRawStart &&
            editorView.renderedRangeContainsGeneratedStructure(
                hiddenGapStart,
                normalizedRawStart,
            )
        ) {
            editorView.recordImeTraceForTesting(
                "structuralSurroundingDelete",
                "before=$beforeLength after=$afterLength codePoints=$deleteInCodePoints hiddenGap=true",
            )
            editorView.handleStructuralBackspace()
            return true
        }
    }

    if (
        deleteRange != null &&
        editorView.renderedRangeContainsGeneratedStructure(
            deleteRange.utf16Start,
            deleteRange.utf16End
        )
    ) {
        editorView.recordImeTraceForTesting(
            "structuralSurroundingDelete",
            "before=$beforeLength after=$afterLength codePoints=$deleteInCodePoints"
        )
        if (isCollapsedBackwardDelete) {
            editorView.handleStructuralBackspace()
        } else {
            editorView.handleStructuralDelete(
                deleteRange.utf16Start,
                deleteRange.utf16End,
                deleteRange.scalarStart,
                deleteRange.scalarEnd
            )
        }
        return true
    }

    editorView.recordImeTraceForTesting(
        "deferredSurroundingDeleteBegin",
        "before=$beforeLength after=$afterLength codePoints=$deleteInCodePoints utf16=$beforeUtf16Length,$afterUtf16Length scalar=${deleteRange?.scalarStart}..${deleteRange?.scalarEnd}"
    )

    val authoritative = editorView.captureAuthoritativeInputSnapshotForEditor()
    val didDeleteVisibleText = editorView.runWithTransientInputMutationGuard {
        deleteVisibleTextInRawRange(rawDeleteStart, rawDeleteEnd, imeDeleteStart)
    }
    if (didDeleteVisibleText && deleteRange != null) {
        when (
            val outcome = editorView.deleteScalarRangeForPendingImeOperationForEditor(
                deleteRange.scalarStart,
                deleteRange.scalarEnd,
            )
        ) {
            is EditorV2NativeIntentResult.Applied -> {
                editorView.runWithDeferredRustUpdateApplication {
                    editorView.promoteOptimisticInputForEditor(
                        outcome.render,
                        deleteRange.scalarStart,
                    )
                }
            }
            is EditorV2NativeIntentResult.Recovered -> {
                editorView.restoreAuthoritativeInputForEditor(
                    authoritative,
                    outcome.updateJson,
                )
            }
            EditorV2NativeIntentResult.Rejected -> {
                editorView.restoreAuthoritativeInputForEditor(authoritative)
            }
            null -> {
                editorView.authorizeCurrentVisibleTextForPendingImeOperationForEditor(
                    logicalCursorAfter = deleteRange.scalarStart,
                )
            }
        }
    }
    editorView.recordImeTraceForTesting(
        "deferredSurroundingDeleteEnd",
        "visibleDeleted=$didDeleteVisibleText visibleLength=${editorView.text?.length ?: -1}"
    )
    return true
}

internal fun EditorInputConnection.surroundingDeleteRange(
    text: String,
    rawDeleteStart: Int,
    rawDeleteEnd: Int,
    selectionStart: Int,
    selectionEnd: Int,
): SurroundingDeleteRange? {
    val (deleteStart, deleteEnd) = PositionBridge.snapRangeToScalarBoundaries(
        rawDeleteStart,
        rawDeleteEnd,
        text
    )
    val logicalSelection = editorView.currentLogicalScalarSelectionForInput()
    if (logicalSelection != null) {
        val logicalStart = minOf(logicalSelection.first, logicalSelection.second)
        val logicalEnd = maxOf(logicalSelection.first, logicalSelection.second)
        if (logicalStart != logicalEnd) {
            return SurroundingDeleteRange(deleteStart, deleteEnd, logicalStart, logicalEnd)
        }
        val deletedBefore = visibleCodePointCount(text, deleteStart, selectionStart)
        val deletedAfter = visibleCodePointCount(text, selectionEnd, deleteEnd)
        val scalarStart = (logicalStart - deletedBefore).coerceAtLeast(0)
        val scalarEnd = logicalEnd + deletedAfter
        if (scalarStart < scalarEnd) {
            return SurroundingDeleteRange(deleteStart, deleteEnd, scalarStart, scalarEnd)
        }
    }
    val scalarStart = PositionBridge.utf16ToScalar(deleteStart, text)
    val scalarEnd = PositionBridge.utf16ToScalar(deleteEnd, text)
    if (scalarStart >= scalarEnd) return null
    return SurroundingDeleteRange(deleteStart, deleteEnd, scalarStart, scalarEnd)
}

internal fun EditorInputConnection.visibleCodePointCount(text: String, start: Int, end: Int): Int {
    val visible = text
        .substring(minOf(start, end), maxOf(start, end))
        .replace(LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER, "")
    return visible.codePointCount(0, visible.length)
}

internal fun EditorInputConnection.deleteVisibleTextInRawRange(
    rawStart: Int,
    rawEnd: Int,
    imeCursorAfter: Int,
): Boolean {
    val editable = editorView.text ?: return false
    val start = rawStart.coerceIn(0, editable.length)
    val end = rawEnd.coerceIn(start, editable.length)
    var chunkEnd = end
    var index = end - 1
    var didDelete = false
    while (index >= start) {
        if (editable[index] == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]) {
            if (index + 1 < chunkEnd) {
                editable.delete(index + 1, chunkEnd)
                didDelete = true
            }
            chunkEnd = index
        }
        index -= 1
    }
    if (start < chunkEnd) {
        editable.delete(start, chunkEnd)
        didDelete = true
    }
    if (didDelete) {
        val updatedMapper = currentMapper()
        val rawCursor = updatedMapper?.imeToRaw(
            imeCursorAfter,
            ImeTextCoordinateMapper.Affinity.AFTER,
        ) ?: start.coerceIn(0, editable.length)
        Selection.setSelection(editable, rawCursor.coerceIn(0, editable.length))
    }
    return didDelete
}
