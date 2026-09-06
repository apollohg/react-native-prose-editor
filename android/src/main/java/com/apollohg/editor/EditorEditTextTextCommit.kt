package com.apollohg.editor

import android.text.Editable
import android.text.Selection
import android.text.Spanned
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.UnderlineSpan

/**
     * Handle committed text from the IME (typed characters, autocomplete).
     *
     * Called by [EditorInputConnection.commitText]. Routes the text through
     * the Rust editor instead of directly inserting into the EditText.
     */
internal fun EditorEditText.handleTextCommitImpl(text: String, newCursorPosition: Int = 1) {
    val startedAt = System.nanoTime()
    if (!isEditable) {
        recordImeTraceForTesting("handleTextCommitNoop", "reason=notEditable textLength=${text.length}")
        return
    }
    if (isApplyingRustState) {
        recordImeTraceForTesting("handleTextCommitNoop", "reason=applyingRust textLength=${text.length}")
        return
    }
    val selectionRange = normalizedUtf16SelectionRange()
    if (selectionRange == null) {
        recordImeTraceForTesting("handleTextCommitNoop", "reason=noSelection textLength=${text.length}")
        return
    }
    if (isCollapsedAtomBoundarySelection(selectionRange.first, selectionRange.second)) {
        recordImeTraceForTesting("handleTextCommitNoop", "reason=atomBoundary textLength=${text.length}")
        return
    }
    if (editorId == 0L) {
        // No Rust editor bound — fall through to direct editing (dev mode).
        val editable = this.text ?: return
        val (start, end) = selectionRange
        editable.replace(start, end, text)
        recordImeTraceForTesting(
            "handleTextCommitDirect",
            "textLength=${text.length} utf16Sel=$start..$end totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) {
        recordImeTraceForTesting("handleTextCommitNoop", "reason=destroyedEditor textLength=${text.length}")
        return
    }

    // Handle Enter/Return as a block split operation.
    if (text == "\n") {
        recordImeTraceForTesting(
            "handleTextCommit",
            "route=return utf16Sel=${selectionRange.first}..${selectionRange.second}"
        )
        handleReturnKey()
        recordImeTraceForTesting(
            "handleTextCommitDone",
            "route=return totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
        return
    }

    val currentText = this.text?.toString() ?: ""
    val scalarSelectionRange = normalizedScalarSelectionRange(currentText)
    if (scalarSelectionRange == null) {
        recordImeTraceForTesting("handleTextCommitNoop", "reason=noScalarSelection textLength=${text.length}")
        return
    }
    val (scalarStart, scalarEnd) = scalarSelectionRange
    val requestedCursor = requestedCursorScalar(
        scalarStart,
        scalarEnd,
        currentText,
        text,
        newCursorPosition
    )
    recordImeTraceForTesting(
        "handleTextCommit",
        "textLength=${text.length} cursor=$newCursorPosition utf16Sel=${selectionRange.first}..${selectionRange.second} scalarSel=$scalarStart..$scalarEnd requestedCursor=$requestedCursor"
    )
    val didApplyOptimisticVisibleText = applyOptimisticPlainTextCommitIfPossible(
        startUtf16 = selectionRange.first,
        endUtf16 = selectionRange.second,
        committedText = text,
        newCursorPosition = newCursorPosition,
        logicalCursorAfter = requestedCursor
            ?: scalarStart + text.codePointCount(0, text.length)
    )
    if (didApplyOptimisticVisibleText) {
        recordImeTraceForTesting(
            "optimisticVisibleTextCommit",
            "textLength=${text.length} utf16Sel=${selectionRange.first}..${selectionRange.second}"
        )
    }
    insertPlainTextRangeInRust(
        scalarStart,
        scalarEnd,
        text,
        requestedCursorScalar = requestedCursor
    )
    recordImeTraceForTesting(
        "handleTextCommitDone",
        "textLength=${text.length} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
    )
}

internal fun EditorEditText.applyOptimisticPlainTextCommitIfPossible(
    startUtf16: Int,
    endUtf16: Int,
    committedText: String,
    newCursorPosition: Int,
    logicalCursorAfter: Int
): Boolean {
    if (newCursorPosition != 1) return false
    if (startUtf16 != endUtf16) return false
    if (committedText.isEmpty()) return false
    if (committedText.codePointCount(0, committedText.length) != 1) return false
    if (committedText.indexOf('\n') >= 0 || committedText.indexOf('\r') >= 0) return false
    if (hasCompositionTrackingForEditor()) return false
    val editable = text ?: return false
    val currentText = editable.toString()
    if (currentText != lastAuthorizedText) return false
    if (startUtf16 < 0 || endUtf16 < startUtf16 || endUtf16 > editable.length) return false
    val spanned = editable as? Spanned
    if (spanned != null && spannedRangeContainsImageSpan(spanned, startUtf16, endUtf16)) return false

    val inlineSpans = spanned?.let {
        optimisticInlineSpansForInsertion(it, startUtf16)
    }.orEmpty()
    var didApply = false
    runWithTransientInputMutationGuard {
        editable.replace(startUtf16, endUtf16, committedText)
        val insertedEnd = startUtf16 + committedText.length
        applyOptimisticInlineSpans(editable, startUtf16, insertedEnd, inlineSpans)
        rememberLogicalSelection(
            scalarAnchor = logicalCursorAfter,
            scalarHead = logicalCursorAfter,
            utf16Anchor = insertedEnd,
            utf16Head = insertedEnd
        )
        Selection.setSelection(editable, insertedEnd, insertedEnd)
        didApply = true
        true
    }
    if (didApply) {
        pendingOptimisticRenderText = editable.toString()
    }
    return didApply
}

internal fun EditorEditText.optimisticInlineSpansForInsertion(
    spanned: Spanned,
    insertionStart: Int
): List<OptimisticInlineSpan> {
    if (spanned.isEmpty()) return emptyList()
    val sourceIndex = when {
        insertionStart > 0 -> insertionStart - 1
        insertionStart < spanned.length -> insertionStart
        else -> return emptyList()
    }
    val queryStart = sourceIndex.coerceIn(0, spanned.length - 1)
    val queryEnd = (queryStart + 1).coerceAtMost(spanned.length)
    val spans = mutableListOf<OptimisticInlineSpan>()
    spanned.getSpans(queryStart, queryEnd, Any::class.java).forEach { span ->
        if (spanned.getSpanStart(span) > queryStart || spanned.getSpanEnd(span) <= queryStart) {
            return@forEach
        }
        cloneOptimisticInlineSpan(span)?.let { clone ->
            spans.add(OptimisticInlineSpan(clone, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE))
        }
    }
    return spans
}

internal fun EditorEditText.cloneOptimisticInlineSpan(span: Any): Any? =
    when (span) {
        is ForegroundColorSpan -> ForegroundColorSpan(span.foregroundColor)
        is BackgroundColorSpan -> BackgroundColorSpan(span.backgroundColor)
        is AbsoluteSizeSpan -> AbsoluteSizeSpan(span.size, span.dip)
        is StyleSpan -> StyleSpan(span.style)
        is UnderlineSpan -> UnderlineSpan()
        is StrikethroughSpan -> StrikethroughSpan()
        else -> null
    }

internal fun EditorEditText.applyOptimisticInlineSpans(
    editable: Editable,
    start: Int,
    end: Int,
    inlineSpans: List<OptimisticInlineSpan>
) {
    if (start >= end || end > editable.length) return
    var hasColor = false
    var hasSize = false
    inlineSpans.forEach { spec ->
        hasColor = hasColor || spec.span is ForegroundColorSpan
        hasSize = hasSize || spec.span is AbsoluteSizeSpan
        editable.setSpan(spec.span, start, end, spec.flags)
    }
    val textStyle = theme?.effectiveTextStyle("paragraph")
    if (!hasColor) {
        editable.setSpan(
            ForegroundColorSpan(textStyle?.color ?: baseTextColor),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
    if (!hasSize) {
        val resolvedTextSize = textStyle?.fontSize?.times(resources.displayMetrics.density) ?: baseFontSize
        editable.setSpan(
            AbsoluteSizeSpan(resolvedTextSize.toInt(), false),
            start,
            end,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
    }
}

internal fun EditorEditText.applyVisibleCompositionCommitForPendingImeOperationForEditorImpl(
    committedText: String,
    replacementStartUtf16: Int,
    replacementEndUtf16: Int,
    newCursorPosition: Int
): Boolean {
    val editable = text ?: return false
    val currentText = editable.toString()
    val (startUtf16, endUtf16) = PositionBridge.snapRangeToScalarBoundaries(
        replacementStartUtf16,
        replacementEndUtf16,
        currentText
    )
    if (startUtf16 > endUtf16 || endUtf16 > editable.length) return false
    var didApply = false
    runWithTransientInputMutationGuard {
        editable.replace(startUtf16, endUtf16, committedText)
        val insertedEnd = startUtf16 + committedText.length
        val requestedCursor = when {
            newCursorPosition > 0 -> insertedEnd + newCursorPosition - 1
            newCursorPosition < 0 -> startUtf16 + newCursorPosition
            else -> insertedEnd
        }.coerceIn(0, editable.length)
        Selection.setSelection(editable, requestedCursor, requestedCursor)
        didApply = true
        true
    }
    if (didApply) {
        pendingOptimisticRenderText = null
    }
    return didApply
}

internal fun EditorEditText.commitAlreadyVisibleCompositionMutationForPendingImeOperationForEditorImpl(
    committedText: String,
    newCursorPosition: Int
): Boolean {
    if (committedText.isEmpty()) return false
    val currentText = text?.toString() ?: return false
    val mutation = nativeTextMutationFromAuthorizedDiff(currentText) ?: return false
    val tokenRange = committedTokenRangeAroundMutation(
        currentText,
        mutation.replacementStartUtf16,
        mutation.replacementEndUtf16
    ) ?: run {
        recordImeTraceForTesting(
            "alreadyVisibleCompositionNoop",
            "reason=noToken committedLength=${committedText.length} visibleRange=${mutation.replacementStartUtf16}..${mutation.replacementEndUtf16}"
        )
        return false
    }
    val visibleToken = currentText.substring(tokenRange.first, tokenRange.second)
    if (visibleToken != committedText) {
        recordImeTraceForTesting(
            "alreadyVisibleCompositionNoop",
            "reason=tokenMismatch committedLength=${committedText.length} tokenLength=${visibleToken.length} visibleRange=${mutation.replacementStartUtf16}..${mutation.replacementEndUtf16}"
        )
        return false
    }

    val authorizedText = lastAuthorizedText
    val requestedCursor = requestedCursorScalar(
        mutation.scalarFrom,
        mutation.scalarTo,
        authorizedText,
        mutation.replacementText,
        newCursorPosition
    )
    recordImeTraceForTesting(
        "alreadyVisibleCompositionApply",
        "range=${mutation.scalarFrom}..${mutation.scalarTo} replacementLength=${mutation.replacementText.length} committedLength=${committedText.length} requestedCursor=$requestedCursor"
    )
    pendingOptimisticRenderText = null
    insertPlainTextRangeInRust(
        mutation.scalarFrom,
        mutation.scalarTo,
        mutation.replacementText,
        requestedCursorScalar = requestedCursor
    )
    return true
}
