package com.apollohg.editor

import com.apollohg.editor.EditorEditText.ApplyUpdateTrace
import android.text.SpannableStringBuilder
import org.json.JSONObject

/**
     * Apply a full render update from Rust to the EditText.
     *
     * Parses the update JSON, converts render elements to [android.text.SpannableStringBuilder]
     * via [RenderBridge], and replaces the EditText's content.
     *
     * @param updateJSON The JSON string from an [EditorV2Driver] transaction result.
     */
internal fun EditorEditText.applyUpdateJSONImpl(
    updateJSON: String,
    notifyListener: Boolean = true,
    refreshInputConnectionForExternalUpdate: Boolean = false
): Boolean {
    throwOnNextApplyUpdateForTesting?.let { error ->
        throwOnNextApplyUpdateForTesting = null
        throw error
    }
    val totalStartedAt = System.nanoTime()
    val previousVisibleText = text?.toString().orEmpty()
    val parseStartedAt = totalStartedAt
    val update = try {
        org.json.JSONObject(updateJSON)
    } catch (error: Exception) {
        recordImeTraceForTesting(
            "applyUpdateJSONNoop",
            "reason=parseError jsonLength=${updateJSON.length} error=${error.javaClass.simpleName}"
        )
        return false
    }
    deferredRustUpdateJSON?.let { deferredUpdateJSON ->
        if (deferredUpdateJSON != updateJSON) {
            advanceRenderBlocksThroughDeferredUpdate(deferredUpdateJSON)
        }
        cancelDeferredRustUpdateApplication(invalidateRenderBlocks = false)
    }
    val parseNanos = System.nanoTime() - parseStartedAt

    val resolveRenderBlocksStartedAt = System.nanoTime()
    val updateDocumentVersion = canonicalV2U64(update.opt("documentVersion") as? String)
    val renderElements = update.optJSONArray("renderElements")
    val renderBlocks = update.optJSONArray("renderBlocks")
    val renderPatch = parseRenderPatch(update.optJSONObject("renderPatch"))
    val resolvedRenderBlocks = renderBlocks
        ?: renderPatch
            ?.takeIf { patchMatchesCurrentRenderBlocks(it, updateDocumentVersion) }
            ?.let { patch ->
                currentRenderBlocksJson?.let { mergeRenderBlocks(it, patch) }
            }
    val resolveRenderBlocksNanos = System.nanoTime() - resolveRenderBlocksStartedAt
    if (
        renderBlocks == null &&
        renderElements == null &&
        renderPatch != null &&
        resolvedRenderBlocks == null
    ) {
        return recoverRenderPatchBaseMismatch(
            notifyListener,
            refreshInputConnectionForExternalUpdate,
        )
    }

    // The core is the authority on empty state; adopt it before anything
    // reconsiders the placeholder.
    setCoreReportedDocumentIsEmpty(
        if (update.has("documentIsEmpty")) update.optBoolean("documentIsEmpty") else null
    )
    val shouldSkipRender = !refreshInputConnectionForExternalUpdate &&
        !currentRenderBlocksNeedFullApply &&
        !authorizedVisibleTextNeedsRebuild &&
        resolvedRenderBlocks != null &&
        currentRenderBlocksJson?.let { current ->
            renderBlocksEqual(current, resolvedRenderBlocks)
        } == true &&
        text?.toString() == lastAuthorizedText &&
        lastAppliedRenderAppearanceRevision == renderAppearanceRevision
    val previousScrollX = scrollX
    val previousScrollY = scrollY

    explicitSelectedImageRange = null
    val buildRenderNanos: Long
    val applyRenderNanos: Long
    val patchTrace = if (
        !shouldSkipRender &&
        !currentRenderBlocksNeedFullApply &&
        renderPatch != null &&
        resolvedRenderBlocks != null &&
        lastAppliedRenderAppearanceRevision == renderAppearanceRevision
    ) {
        applyRenderPatchIfPossible(renderPatch, refreshInputConnectionForExternalUpdate)
    } else {
        null
    }
    val appliedPatch = patchTrace?.applied == true
    if (shouldSkipRender) {
        pendingOptimisticRenderText = null
        lastRenderAppliedPatchForTesting = false
        retainCurrentRenderBlocks(
            resolvedRenderBlocks,
            updateDocumentVersion,
            needFullApply = false,
        )
        clearNativeTextMutationAdoptionSuppression()
        clearNativeTextMutationAfterBlurWindow()
        buildRenderNanos = 0L
        applyRenderNanos = 0L
    } else if (appliedPatch) {
        pendingOptimisticRenderText = null
        retainCurrentRenderBlocks(
            resolvedRenderBlocks,
            updateDocumentVersion,
            needFullApply = false,
        )
        lastAppliedRenderAppearanceRevision = renderAppearanceRevision
        buildRenderNanos = patchTrace?.buildRenderNanos ?: 0L
        applyRenderNanos = patchTrace?.applyRenderNanos ?: 0L
    } else {
        cancelPendingImageLoads()
        val buildStartedAt = System.nanoTime()
        val fullSpannable = if (resolvedRenderBlocks != null) {
            RenderBridge.buildSpannableFromBlocks(
                resolvedRenderBlocks,
                baseFontSize = baseFontSize,
                textColor = baseTextColor,
                theme = theme,
                density = resources.displayMetrics.density,
                hostView = this,
                atomConfiguration = atomRenderConfiguration
            )
        } else if (renderElements != null) {
            RenderBridge.buildSpannableFromArray(
                renderElements,
                baseFontSize,
                baseTextColor,
                theme,
                resources.displayMetrics.density,
                this,
                atomRenderConfiguration
            )
        } else {
            recordImeTraceForTesting(
                "applyUpdateJSONNoop",
                "reason=noRenderPayload jsonLength=${updateJSON.length}"
            )
            return false
        }
        buildRenderNanos = System.nanoTime() - buildStartedAt
        retainCurrentRenderBlocks(
            resolvedRenderBlocks,
            updateDocumentVersion,
            needFullApply = false,
        )
        val applyStartedAt = System.nanoTime()
        val optimisticText = pendingOptimisticRenderText
        val canReuseOptimisticVisibleText =
                optimisticText != null &&
                text?.toString() == optimisticText &&
                fullSpannable.toString() == optimisticText &&
                !spannedContainsImageSpan(fullSpannable)
        if (canReuseOptimisticVisibleText) {
            authorizeVisibleTextForMatchedOptimisticRender(fullSpannable)
        } else {
            applyRenderedSpannable(
                fullSpannable,
                usedPatch = false,
                preserveInputConnectionForExternalUpdate = refreshInputConnectionForExternalUpdate
            )
        }
        pendingOptimisticRenderText = null
        applyRenderNanos = System.nanoTime() - applyStartedAt
        lastAppliedRenderAppearanceRevision = renderAppearanceRevision
    }

    // Apply the selection from the update.
    val selectionStartedAt = System.nanoTime()
    val selection = update.optJSONObject("selection")
    if (selection != null) {
        applySelectionFromJSON(
            selection,
            updateDocumentVersion,
        )
    } else {
        logicalSelectionSnapshot = null
    }
    lastAppliedDocumentVersion = updateDocumentVersion
    authorizedVisibleTextNeedsRebuild = false
    val selectionNanos = System.nanoTime() - selectionStartedAt

    val postApplyStartedAt = System.nanoTime()
    if (notifyListener) {
        editorListener?.onEditorUpdate(updateJSON)
    }
    if (!shouldSkipRender) {
        onContentSizeMayChange?.invoke()
    }
    onSelectionOrContentMayChange?.invoke()
    if (heightBehavior == EditorHeightBehavior.AUTO_GROW) {
        requestLayout()
    } else {
        preserveScrollPosition(previousScrollX, previousScrollY)
    }
    refreshInputConnectionAfterExternalTextReplacementIfNeeded(
        enabled = refreshInputConnectionForExternalUpdate,
        previousVisibleText = previousVisibleText
    )
    val postApplyNanos = System.nanoTime() - postApplyStartedAt

    val totalNanos = System.nanoTime() - totalStartedAt
    recordImeTraceForTesting(
        "applyUpdateJSON",
        "notify=$notifyListener skippedRender=$shouldSkipRender attemptedPatch=${renderPatch != null} jsonLength=${updateJSON.length} parseUs=${nanosToMicros(parseNanos)} resolveUs=${nanosToMicros(resolveRenderBlocksNanos)} buildUs=${nanosToMicros(buildRenderNanos)} applyUs=${nanosToMicros(applyRenderNanos)} selectionUs=${nanosToMicros(selectionNanos)} postUs=${nanosToMicros(postApplyNanos)} totalUs=${nanosToMicros(totalNanos)}"
    )

    if (captureApplyUpdateTraceForTesting) {
        lastApplyUpdateTraceForTesting = ApplyUpdateTrace(
            attemptedPatch = renderPatch != null,
            usedPatch = appliedPatch,
            skippedRender = shouldSkipRender,
            parseNanos = parseNanos,
            resolveRenderBlocksNanos = resolveRenderBlocksNanos,
            patchEligibilityNanos = patchTrace?.eligibilityNanos ?: 0L,
            buildRenderNanos = buildRenderNanos,
            applyRenderNanos = applyRenderNanos,
            selectionNanos = selectionNanos,
            postApplyNanos = postApplyNanos,
            totalNanos = totalNanos
        )
    }
    return !shouldSkipRender
}
