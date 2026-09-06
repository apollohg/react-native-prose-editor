package com.apollohg.editor

import android.text.Annotation
import android.text.Spanned
import android.view.DragEvent

internal fun EditorEditText.localTextDragFor(event: DragEvent): LocalTextDrag? {
    if (!isEditable || editorId == 0L || !isDragFromThisEditor(event.localState) || !hasFocus()) {
        return null
    }
    val currentText = text?.toString() ?: return null
    val selection = normalizedUtf16SelectionRange(currentText) ?: return null
    val (start, end) = PositionBridge.snapRangeToScalarBoundaries(
        selection.first,
        selection.second,
        currentText,
    )
    if (start >= end || containsInterBlockBoundary(start, end)) return null
    return LocalTextDrag(
        PositionBridge.utf16ToScalar(start, currentText),
        PositionBridge.utf16ToScalar(end, currentText),
        lastAppliedDocumentVersion,
        editorId,
    )
}

internal fun EditorEditText.isDragFromThisEditor(localState: Any?): Boolean = localState === this

internal fun EditorEditText.containsInterBlockBoundary(start: Int, end: Int): Boolean {
    val content = text as? Spanned ?: return false
    return content.getSpans(start, end, Annotation::class.java).any {
        it.key == RenderBridge.NATIVE_INTER_BLOCK_SEPARATOR_ANNOTATION
    }
}

internal fun EditorEditText.performLocalSelectionDrop(drag: LocalTextDrag, destination: Int): Boolean {
    if (destination in drag.scalarFrom..drag.scalarTo) return false
    if (drag.editorId != editorId || drag.documentVersion == null) return false
    if (lastAppliedDocumentVersion == null || lastAppliedDocumentVersion != drag.documentVersion) return false
    if (!prepareForExternalInteractionMutation()) return false
    if (lastAppliedDocumentVersion != drag.documentVersion) return false
    onMoveSelectionScalarForTesting?.let { callback ->
        callback(drag.scalarFrom, drag.scalarTo, destination)
        return true
    }
    val driver = v2Driver ?: return false
    val updateJSON = driver.moveSelection(drag.scalarFrom, drag.scalarTo, destination)
    applyNonOptimisticRustUpdate(driver, updateJSON)
    return updateJSON != null
}

internal fun EditorEditText.performLocalSelectionDropForTestingImpl(
    scalarFrom: Int,
    scalarTo: Int,
    destination: Int,
    documentVersion: String?,
): Boolean = performLocalSelectionDrop(
    LocalTextDrag(scalarFrom, scalarTo, documentVersion, editorId),
    destination,
)
