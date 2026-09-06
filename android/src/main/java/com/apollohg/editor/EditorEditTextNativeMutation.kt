package com.apollohg.editor

import com.apollohg.editor.EditorEditText.Companion.NATIVE_TEXT_MUTATION_AFTER_BLUR_WINDOW_MS
import android.os.SystemClock

internal fun EditorEditText.nativeTextMutationFromAuthorizedDiff(currentText: String): NativeTextMutation? {
    val authorizedText = lastAuthorizedText
    if (currentText == authorizedText) return null

    var prefix = 0
    val sharedLength = minOf(authorizedText.length, currentText.length)
    while (
        prefix < sharedLength &&
        authorizedText[prefix] == currentText[prefix]
    ) {
        prefix++
    }
    prefix = minOf(
        PositionBridge.snapToScalarBoundary(prefix, authorizedText, biasForward = false),
        PositionBridge.snapToScalarBoundary(prefix, currentText, biasForward = false)
    )

    var authorizedEnd = authorizedText.length
    var currentEnd = currentText.length
    while (
        authorizedEnd > prefix &&
        currentEnd > prefix &&
        authorizedText[authorizedEnd - 1] == currentText[currentEnd - 1]
    ) {
        authorizedEnd--
        currentEnd--
    }
    authorizedEnd = PositionBridge.snapToScalarBoundary(
        authorizedEnd,
        authorizedText,
        biasForward = true
    )
    currentEnd = PositionBridge.snapToScalarBoundary(
        currentEnd,
        currentText,
        biasForward = true
    )

    val replacementText = currentText.substring(prefix, currentEnd)
    val rawSelectionStart = selectionStart
    val rawSelectionEnd = selectionEnd
    val selectionAnchorUtf16 = rawSelectionStart
        .takeIf { it >= 0 }
        ?.let { PositionBridge.snapToScalarBoundary(it, currentText, biasForward = true) }
    val selectionHeadUtf16 = rawSelectionEnd
        .takeIf { it >= 0 }
        ?.let { PositionBridge.snapToScalarBoundary(it, currentText, biasForward = true) }
    return NativeTextMutation(
        scalarFrom = PositionBridge.utf16ToScalar(prefix, authorizedText),
        scalarTo = PositionBridge.utf16ToScalar(authorizedEnd, authorizedText),
        replacementText = replacementText,
        resultingText = currentText,
        replacementStartUtf16 = prefix,
        replacementEndUtf16 = currentEnd,
        selectionScalarAnchor = selectionAnchorUtf16?.let {
            PositionBridge.utf16ToScalar(it, currentText)
        },
        selectionScalarHead = selectionHeadUtf16?.let {
            PositionBridge.utf16ToScalar(it, currentText)
        }
    )
}

internal fun EditorEditText.shouldAdoptNativeTextMutation(
    mutation: NativeTextMutation,
    allowAfterBlur: Boolean = false
): Boolean {
    if (!isEditable) return false
    if (isNativeTextMutationAdoptionSuppressedForCurrentRevision()) return false
    if (!hasFocus()) {
        return allowAfterBlur &&
            canAdoptNativeTextMutationAfterBlur() &&
            shouldAdoptFinalNativeTextMutation(mutation)
    }
    return shouldAdoptFinalNativeTextMutation(mutation)
}

internal fun EditorEditText.shouldAdoptFinalNativeTextMutation(mutation: NativeTextMutation): Boolean {
    if (composingTextForEditor() != null) return false
    val trackedRange = compositionReplacementRange() ?: return true
    val authorizedText = lastAuthorizedText
    val trackedStart = PositionBridge.utf16ToScalar(trackedRange.first, authorizedText)
    val trackedEnd = PositionBridge.utf16ToScalar(trackedRange.second, authorizedText)
    if (trackedStart == trackedEnd) {
        return mutation.scalarFrom == trackedStart &&
            mutation.scalarTo == trackedStart &&
            mutation.replacementText.isNotEmpty()
    }
    if (mutation.scalarFrom == mutation.scalarTo) {
        return mutation.replacementText.isNotEmpty() &&
            mutation.scalarFrom >= trackedStart &&
            mutation.scalarFrom <= trackedEnd
    }
    return mutation.scalarFrom < trackedEnd && mutation.scalarTo > trackedStart
}

internal fun EditorEditText.drainNativeTextMutationIfNeeded(
    allowAfterBlur: Boolean,
    preserveInputConnectionForExternalUpdate: Boolean = false
): Boolean {
    if (editorId == 0L) return true
    if (discardTransientInputForDestroyedEditorIfNeeded()) return false
    val editable = text
    val currentText = editable?.toString() ?: ""
    if (currentText == lastAuthorizedText) return true

    val mutation = nativeTextMutationFromAuthorizedDiff(currentText)
    if (mutation != null && shouldAdoptNativeTextMutation(mutation, allowAfterBlur)) {
        commitNativeTextMutation(
            mutation,
            preserveInputConnectionForExternalUpdate = preserveInputConnectionForExternalUpdate
        )
        return true
    }
    recordImeTraceForTesting(
        "nativeMutationNoop",
        "reason=${if (mutation == null) "noDiffRange" else "notAdoptable"} allowAfterBlur=$allowAfterBlur currentLength=${currentText.length} authorizedLength=${lastAuthorizedText.length}"
    )
    return false
}

internal fun EditorEditText.beginNativeTextMutationAfterBlurWindow() {
    if (!hasLiveEditor()) {
        clearNativeTextMutationAfterBlurWindow()
        return
    }
    nativeTextMutationAfterBlurWindow = NativeTextMutationAfterBlurWindow(
        editorId = editorId,
        authorizedTextRevision = lastAuthorizedTextRevision,
        deadlineMs = SystemClock.uptimeMillis() + NATIVE_TEXT_MUTATION_AFTER_BLUR_WINDOW_MS
    )
}

internal fun EditorEditText.clearNativeTextMutationAfterBlurWindow() {
    nativeTextMutationAfterBlurWindow = null
}

internal fun EditorEditText.suppressNativeTextMutationAdoptionForCurrentRevision() {
    if (!hasLiveEditor()) {
        clearNativeTextMutationAdoptionSuppression()
        return
    }
    nativeTextMutationAdoptionSuppression = NativeTextMutationAdoptionSuppression(
        editorId = editorId,
        authorizedTextRevision = lastAuthorizedTextRevision
    )
}

internal fun EditorEditText.clearNativeTextMutationAdoptionSuppression() {
    nativeTextMutationAdoptionSuppression = null
}

internal fun EditorEditText.isNativeTextMutationAdoptionSuppressedForCurrentRevision(): Boolean {
    val suppression = nativeTextMutationAdoptionSuppression ?: return false
    if (
        suppression.editorId != editorId ||
        suppression.authorizedTextRevision != lastAuthorizedTextRevision
    ) {
        nativeTextMutationAdoptionSuppression = null
        return false
    }
    return true
}

internal fun EditorEditText.canAdoptNativeTextMutationAfterBlur(): Boolean {
    val window = nativeTextMutationAfterBlurWindow ?: return false
    val now = SystemClock.uptimeMillis()
    if (now > window.deadlineMs ||
        window.editorId != editorId ||
        window.authorizedTextRevision != lastAuthorizedTextRevision ||
        window.didAdoptMutation
    ) {
        nativeTextMutationAfterBlurWindow = null
        return false
    }
    return true
}

internal fun EditorEditText.commitNativeTextMutation(
    mutation: NativeTextMutation,
    preserveInputConnectionForExternalUpdate: Boolean = false
) {
    if (!hasLiveEditor()) return
    val startedAt = System.nanoTime()
    if ((text?.toString() ?: "") != mutation.resultingText) {
        recordImeTraceForTesting(
            "nativeMutationNoop",
            "reason=staleResult range=${mutation.scalarFrom}..${mutation.scalarTo} replacementLength=${mutation.replacementText.length}"
        )
        return
    }
    val shouldRestartInput = hasFocus()
    if (preserveInputConnectionForExternalUpdate) {
        clearInputStateForExternalReplacementPreservingConnection()
    } else {
        retireInputConnectionForEditor()
    }
    nativeTextMutationAfterBlurWindow?.didAdoptMutation = true
    clearNativeTextMutationAfterBlurWindow()

    recordImeTraceForTesting(
        "nativeMutationApply",
        "range=${mutation.scalarFrom}..${mutation.scalarTo} replacementLength=${mutation.replacementText.length} restartInput=$shouldRestartInput preserveInputConnection=$preserveInputConnectionForExternalUpdate"
    )
    if (mutation.replacementText.isEmpty()) {
        deleteRangeInRust(mutation.scalarFrom, mutation.scalarTo)
    } else {
        insertPlainTextRangeInRust(
            mutation.scalarFrom,
            mutation.scalarTo,
            mutation.replacementText
        )
    }
    restoreSelectionAfterNativeTextMutation(mutation)
    if (shouldRestartInput) {
        restartInputForEditor(
            if (preserveInputConnectionForExternalUpdate) "externalUpdatePreflight" else "explicit"
        )
    }
    recordImeTraceForTesting(
        "nativeMutationApplyDone",
        "totalUs=${nanosToMicros(System.nanoTime() - startedAt)} restartInput=$shouldRestartInput"
    )
}

internal fun EditorEditText.restoreSelectionAfterNativeTextMutation(mutation: NativeTextMutation) {
    val selectionScalarAnchor = mutation.selectionScalarAnchor ?: return
    val selectionScalarHead = mutation.selectionScalarHead ?: return
    val currentText = text?.toString() ?: return
    val anchorUtf16 = PositionBridge.scalarToUtf16(selectionScalarAnchor, currentText)
    val headUtf16 = PositionBridge.scalarToUtf16(selectionScalarHead, currentText)
    val length = currentText.length
    setSelection(anchorUtf16.coerceIn(0, length), headUtf16.coerceIn(0, length))
}
