package com.apollohg.editor

import android.text.Annotation
import android.text.Spanned
import android.text.SpannableStringBuilder

internal fun EditorEditText.applyFullRenderPreservingEditorState(
    spannable: CharSequence,
    restoreScrollAfterLayout: Boolean = false,
) {
    val previousSelectionStart = selectionStart
    val previousSelectionEnd = selectionEnd
    val previousScrollX = scrollX
    val previousScrollY = scrollY
    applyRenderedSpannable(spannable, usedPatch = false)
    if (previousSelectionStart >= 0 && previousSelectionEnd >= 0) {
        val length = text?.length ?: 0
        setSelection(
            previousSelectionStart.coerceIn(0, length),
            previousSelectionEnd.coerceIn(0, length),
        )
    }
    preserveScrollPosition(previousScrollX, previousScrollY)
    if (restoreScrollAfterLayout) {
        post { preserveScrollPosition(previousScrollX, previousScrollY) }
    }
}

internal fun EditorEditText.applyRenderedSpannable(
    spannable: CharSequence,
    replaceRange: RenderReplaceRange? = null,
    replacedTopLevelStartIndex: Int? = null,
    replacedTopLevelDeleteCount: Int = 0,
    usedPatch: Boolean,
    preserveInputConnectionForExternalUpdate: Boolean = false
) {
    onBeforeRenderRefresh?.invoke()
    val startedAt = System.nanoTime()
    val previousScrollX = scrollX
    val previousScrollY = scrollY
    val hadCompositionTracking = hasCompositionTrackingForEditor()
    var shouldRestartInput = false
    val mode = if (replaceRange != null) "replace" else "setText"
    val precedingParagraphSpans = replaceRange
        ?.let { paragraphSpansEndingAt(it.start) }
        .orEmpty()
    val followingParagraphSpans = replaceRange
        ?.let { paragraphSpansStartingAt(it.endExclusive) }
        .orEmpty()
    val replacedImageSpans = (text as? Spanned)?.let { current ->
        if (replaceRange == null) {
            current.getSpans(0, current.length, BlockImageSpan::class.java).toList()
        } else if (replaceRange.start < replaceRange.endExclusive) {
            current.getSpans(
                replaceRange.start,
                replaceRange.endExclusive,
                BlockImageSpan::class.java,
            ).filter { span ->
                current.getSpanStart(span) < replaceRange.endExclusive &&
                    current.getSpanEnd(span) > replaceRange.start
            }
        } else {
            emptyList()
        }
    }.orEmpty()
    replacedImageSpans.forEach(BlockImageSpan::close)
    isApplyingRustState = true
    beginBatchEdit()
    try {
        if (replaceRange != null) {
            if (replacedTopLevelStartIndex != null) {
                removeParagraphSpansOwnedByTopLevelRange(
                    replacedTopLevelStartIndex,
                    replacedTopLevelDeleteCount
                )
            }
            editableText.replace(replaceRange.start, replaceRange.endExclusive, spannable)
            precedingParagraphSpans.forEach { snapshot ->
                editableText.setSpan(
                    snapshot.span,
                    snapshot.start,
                    snapshot.end,
                    snapshot.flags
                )
            }
            val replacementDelta = spannable.length - (replaceRange.endExclusive - replaceRange.start)
            followingParagraphSpans.forEach { snapshot ->
                editableText.setSpan(
                    snapshot.span,
                    snapshot.start + replacementDelta,
                    snapshot.end + replacementDelta,
                    snapshot.flags
                )
            }
        } else {
            setText(spannable)
        }
        lastAuthorizedText = text?.toString().orEmpty()
        lastAuthorizedRenderedText = text?.let { SpannableStringBuilder(it) }
        lastAuthorizedTextRevision += 1L
        clearNativeTextMutationAdoptionSuppression()
        if (hadCompositionTracking && preserveInputConnectionForExternalUpdate) {
            clearInputStateForExternalReplacementPreservingConnection()
            shouldRestartInput = true
        } else if (hadCompositionTracking) {
            retireInputConnectionForEditor()
            shouldRestartInput = true
        } else {
            clearCompositionTrackingForEditor()
        }
        lastRenderAppliedPatchForTesting = usedPatch
        clearNativeTextMutationAfterBlurWindow()
    } finally {
        endBatchEdit()
        isApplyingRustState = false
    }
    recordImeTraceForTesting(
        "applyRenderedSpannable",
        "mode=$mode usedPatch=$usedPatch incomingLength=${spannable.length} replace=${replaceRange?.start}..${replaceRange?.endExclusive} hadComposition=$hadCompositionTracking restartInput=$shouldRestartInput applyUs=${nanosToMicros(System.nanoTime() - startedAt)} scroll=$previousScrollX,$previousScrollY->$scrollX,$scrollY layout=${layout != null}"
    )
    invalidateRenderedContent()
    restartInputAfterCompositionInvalidationIfNeeded(shouldRestartInput)
}

internal fun EditorEditText.paragraphSpansEndingAt(offset: Int): List<ParagraphSpanSnapshot> =
    editableText
        .getSpans(0, editableText.length, Any::class.java)
        .filter { span ->
            editableText.getSpanEnd(span) == offset &&
                editableText.getSpanFlags(span) and Spanned.SPAN_PARAGRAPH == Spanned.SPAN_PARAGRAPH
        }
        .map { span ->
            ParagraphSpanSnapshot(
                span = span,
                start = editableText.getSpanStart(span),
                end = editableText.getSpanEnd(span),
                flags = editableText.getSpanFlags(span)
            )
        }

internal fun EditorEditText.paragraphSpansStartingAt(offset: Int): List<ParagraphSpanSnapshot> =
    editableText
        .getSpans(0, editableText.length, Any::class.java)
        .filter { span ->
            editableText.getSpanStart(span) == offset &&
                editableText.getSpanFlags(span) and Spanned.SPAN_PARAGRAPH == Spanned.SPAN_PARAGRAPH
        }
        .map { span ->
            ParagraphSpanSnapshot(
                span = span,
                start = editableText.getSpanStart(span),
                end = editableText.getSpanEnd(span),
                flags = editableText.getSpanFlags(span)
            )
        }

internal fun EditorEditText.removeParagraphSpansOwnedByTopLevelRange(startIndex: Int, deleteCount: Int) {
    if (deleteCount <= 0) return
    val endIndex = startIndex + deleteCount
    val topLevelAnnotations = editableText
        .getSpans(0, editableText.length, Annotation::class.java)
        .asSequence()
        .filter { it.key == RenderBridge.NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION }
        .mapNotNull { annotation ->
            val index = annotation.value.toIntOrNull() ?: return@mapNotNull null
            Triple(
                editableText.getSpanStart(annotation),
                editableText.getSpanEnd(annotation),
                index
            )
        }
        .sortedBy { it.first }
        .toList()

    editableText
        .getSpans(0, editableText.length, Any::class.java)
        .filter { span ->
            editableText.getSpanFlags(span) and Spanned.SPAN_PARAGRAPH == Spanned.SPAN_PARAGRAPH
        }
        .filter { span ->
            val spanStart = editableText.getSpanStart(span)
            val spanEnd = editableText.getSpanEnd(span)
            val ownerIndex = topLevelAnnotations.firstOrNull { annotation ->
                annotation.first < spanEnd && annotation.second > spanStart
            }?.third
            ownerIndex != null && ownerIndex >= startIndex && ownerIndex < endIndex
        }
        .forEach(editableText::removeSpan)
}

internal fun EditorEditText.invalidateRenderedContent() {
    invalidate()
    postInvalidateOnAnimation()
}

internal fun EditorEditText.authorizeVisibleTextForMatchedOptimisticRender(spannable: CharSequence) {
    val startedAt = System.nanoTime()
    val visibleText = text?.toString().orEmpty()
    lastAuthorizedText = visibleText
    lastAuthorizedRenderedText = text?.let { SpannableStringBuilder(it) }
        ?: SpannableStringBuilder(spannable)
    lastAuthorizedTextRevision += 1L
    clearNativeTextMutationAdoptionSuppression()
    clearCompositionTrackingForEditor()
    lastRenderAppliedPatchForTesting = false
    clearNativeTextMutationAfterBlurWindow()
    recordImeTraceForTesting(
        "reuseOptimisticVisibleTextRender",
        "textLength=${visibleText.length} applyUs=${nanosToMicros(System.nanoTime() - startedAt)}"
    )
}

    /**
     * Apply a render JSON string (just render elements, no update wrapper).
     *
     * Used for initial content loading (set_html / set_json return render
     * elements directly, not wrapped in an EditorUpdate).
     *
     * @param renderJSON The JSON array string of render elements.
     */
internal fun EditorEditText.applyRenderJSONImpl(renderJSON: String) {
    cancelPendingImageLoads()
    restartImageLoadsOnAttach = false
    val startedAt = System.nanoTime()
    val spannable = RenderBridge.buildSpannable(
        renderJSON,
        baseFontSize,
        baseTextColor,
        theme,
        resources.displayMetrics.density,
        this,
        atomRenderConfiguration
    )

    val previousScrollX = scrollX
    val previousScrollY = scrollY

    explicitSelectedImageRange = null
    invalidateCurrentRenderBlocks()
    pendingOptimisticRenderText = null
    applyRenderedSpannable(spannable, usedPatch = false)
    onContentSizeMayChange?.invoke()
    onSelectionOrContentMayChange?.invoke()
    if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
        requestLayout()
    } else {
        preserveScrollPosition(previousScrollX, previousScrollY)
    }
    recordImeTraceForTesting(
        "applyRenderJSON",
        "jsonLength=${renderJSON.length} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
    )
}
