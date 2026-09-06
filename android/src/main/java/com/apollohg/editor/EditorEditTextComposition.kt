package com.apollohg.editor

import android.graphics.Typeface
import android.text.Editable
import android.text.Spanned
import android.view.inputmethod.BaseInputConnection

internal fun EditorEditText.authorizedUtf16RangeImpl(start: Int, end: Int): Pair<Int, Int> {
    if (start == end) {
        val snapped = PositionBridge.snapToScalarBoundary(
            start,
            lastAuthorizedText,
            biasForward = true
        )
        return snapped to snapped
    }
    return PositionBridge.snapRangeToScalarBoundaries(start, end, lastAuthorizedText)
}

internal fun EditorEditText.isCurrentTextAuthorizedForEditorImpl(): Boolean =
    (text?.toString() ?: "") == lastAuthorizedText

internal fun EditorEditText.captureCompositionReplacementRangeIfNeededImpl() {
    if (didInvalidateCompositionReplacementRange) return
    if (compositionReplacementRange() != null) return
    val (start, end) = normalizedUtf16SelectionRange() ?: return
    setCompositionReplacementRange(start, end)
}

internal fun EditorEditText.setCompositionReplacementRangeImpl(start: Int, end: Int) {
    if (didInvalidateCompositionReplacementRange) return
    val replacementRange = authorizedUtf16Range(start, end)
    composingReplacementStartUtf16 = replacementRange.first
    composingReplacementEndUtf16 = replacementRange.second
    composingReplacementAuthorizedTextRevision = lastAuthorizedTextRevision
    didInvalidateCompositionReplacementRange = false
}

internal fun EditorEditText.compositionReplacementRangeImpl(): Pair<Int, Int>? {
    val start = composingReplacementStartUtf16 ?: return null
    val end = composingReplacementEndUtf16 ?: return null
    if (composingReplacementAuthorizedTextRevision != lastAuthorizedTextRevision) {
        clearCompositionTrackingForEditor()
        didInvalidateCompositionReplacementRange = true
        return null
    }
    return start to end
}

internal fun EditorEditText.authorizedSelectionForTransientInputRestore(
    currentStart: Int,
    currentEnd: Int
): Pair<Int, Int>? {
    compositionReplacementRange()?.let { return it }
    return if (
        currentStart >= 0 &&
        currentEnd >= 0 &&
        currentStart <= lastAuthorizedText.length &&
        currentEnd <= lastAuthorizedText.length
    ) {
        currentStart to currentEnd
    } else {
        null
    }
}

internal fun EditorEditText.consumeInvalidatedCompositionReplacementRangeForEditorImpl(): Boolean {
    val invalidated = didInvalidateCompositionReplacementRange
    didInvalidateCompositionReplacementRange = false
    return invalidated
}

internal fun EditorEditText.hasInvalidatedCompositionReplacementRangeForEditorImpl(): Boolean =
    didInvalidateCompositionReplacementRange

internal fun EditorEditText.setComposingTextForEditorImpl(text: String?) {
    composingText = text
}

internal fun EditorEditText.composingTextForEditorImpl(): String? = composingText

internal fun EditorEditText.applyTransientComposingTextStyleForEditorImpl() {
    val editable = text ?: return
    removeTransientComposingTextStyleSpans(editable)

    val start = BaseInputConnection.getComposingSpanStart(editable)
    val end = BaseInputConnection.getComposingSpanEnd(editable)
    if (start < 0 || end < 0 || start >= end || end > editable.length) return

    val textStyle = theme?.effectiveTextStyle("paragraph")
    val resolvedTextSize = textStyle?.fontSize?.times(resources.displayMetrics.density) ?: baseFontSize
    val resolvedTextColor = textStyle?.color ?: baseTextColor

    editable.setSpan(
        TransientComposingSizeSpan(resolvedTextSize.toInt()),
        start,
        end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )
    editable.setSpan(
        TransientComposingColorSpan(resolvedTextColor),
        start,
        end,
        Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
    )

    val typefaceStyle = textStyle?.typefaceStyle() ?: Typeface.NORMAL
    if (typefaceStyle != Typeface.NORMAL) {
        editable.setSpan(
            TransientComposingStyleSpan(typefaceStyle),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }

    val fontFamily = textStyle?.fontFamily?.takeIf { it.isNotBlank() }
    if (fontFamily != null) {
        editable.setSpan(
            TransientComposingTypefaceSpan(fontFamily),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    invalidateImeTextCoordinateMapperForEditor()
}

internal fun EditorEditText.removeTransientComposingTextStyleSpans(editable: Editable) {
    editable
        .getSpans(0, editable.length, TransientComposingTextStyleSpan::class.java)
        .forEach(editable::removeSpan)
    invalidateImeTextCoordinateMapperForEditor()
}

internal fun EditorEditText.composingTextFromVisibleReplacementForEditorImpl(): String? {
    val (start, end) = compositionReplacementRange() ?: return null
    val authorizedText = lastAuthorizedText
    val currentText = text?.toString() ?: return null
    if (start < 0 || end < start || end > authorizedText.length) return null

    val authorizedPrefix = authorizedText.substring(0, start)
    val authorizedSuffix = authorizedText.substring(end)
    if (!currentText.startsWith(authorizedPrefix)) return null
    if (!currentText.endsWith(authorizedSuffix)) return null

    val replacementEnd = currentText.length - authorizedSuffix.length
    if (replacementEnd < authorizedPrefix.length) return null
    return currentText.substring(authorizedPrefix.length, replacementEnd)
}

internal fun EditorEditText.clearCompositionTrackingForEditorImpl() {
    composingText = null
    composingReplacementStartUtf16 = null
    composingReplacementEndUtf16 = null
    composingReplacementAuthorizedTextRevision = null
}

internal fun EditorEditText.hasCompositionTrackingForEditor(): Boolean =
    composingText != null ||
        composingReplacementStartUtf16 != null ||
        composingReplacementEndUtf16 != null ||
        composingReplacementAuthorizedTextRevision != null

internal fun EditorEditText.retireInputConnectionForEditor() {
    recordImeTraceForTesting("retireInputConnection")
    activeInputConnection?.clearCompositionTrackingForEditor()
    invalidateInputConnectionsForEditor()
    clearCompositionTrackingForEditor()
    clearCompositionInvalidationForEditor()
    clearNativeComposingSpans()
}

internal fun EditorEditText.retireInputConnectionForHostDetachImpl() {
    retireInputConnectionForEditor()
}

internal fun EditorEditText.isEditorDestroyedForInputImpl(): Boolean =
    editorId != 0L && NativeEditorViewRegistry.isDestroyed(editorId)

internal fun EditorEditText.hasLiveEditor(): Boolean =
    editorId != 0L && !isEditorDestroyedForInput()

internal fun EditorEditText.discardTransientInputForDestroyedEditorIfNeeded(): Boolean {
    if (!isEditorDestroyedForInput()) return false
    cancelExternalTextCompositionForLifecycleIfNeeded()
    retireInputConnectionForEditor()
    clearNativeTextMutationAfterBlurWindow()
    clearNativeTextMutationAdoptionSuppression()
    return true
}

internal fun EditorEditText.discardTransientInputAndRestoreAuthorizedTextForEditor() {
    cancelExternalTextCompositionForLifecycleIfNeeded()
    retireInputConnectionForEditor()
    clearNativeTextMutationAfterBlurWindow()
    restoreAuthorizedTextSnapshotForEditor()
    suppressNativeTextMutationAdoptionForCurrentRevision()
}

internal fun EditorEditText.restoreAuthorizedTextSnapshotForEditor() {
    if ((text?.toString() ?: "") == lastAuthorizedText) return
    val authorizedSnapshot = lastAuthorizedRenderedText ?: lastAuthorizedText
    val wasApplyingRustState = isApplyingRustState
    isApplyingRustState = true
    beginBatchEdit()
    try {
        setText(authorizedSnapshot)
    } finally {
        endBatchEdit()
        isApplyingRustState = wasApplyingRustState
    }
}

internal fun EditorEditText.restartInputAfterCompositionInvalidationIfNeeded(shouldRestart: Boolean) {
    if (!shouldRestart) return
    restartInputForEditorIfFocused("focused")
}
