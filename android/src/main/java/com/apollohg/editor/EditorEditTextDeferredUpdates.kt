package com.apollohg.editor

import com.apollohg.editor.EditorEditText.AuthoritativeInputSnapshot
import android.os.Handler
import android.os.Looper
import android.text.SpannableStringBuilder
import org.json.JSONObject

// Samsung Keyboard may call finishComposingText() and then commitText(" ")
    // for one space tap. Defer the render from finishComposingText() by one
    // loop so setText() does not restart input before the pending space arrives.
internal fun EditorEditText.runWithDeferredRustUpdateApplicationImpl(block: () -> Unit) {
    recordImeTraceForTesting(
        "deferRustUpdateBegin",
        "depth=$deferredRustUpdateApplicationDepth pending=${deferredRustUpdateJSON != null}"
    )
    deferredRustUpdateApplicationDepth += 1
    try {
        block()
    } finally {
        deferredRustUpdateApplicationDepth -= 1
        recordImeTraceForTesting(
            "deferRustUpdateEnd",
            "depth=$deferredRustUpdateApplicationDepth pending=${deferredRustUpdateJSON != null}"
        )
        if (deferredRustUpdateApplicationDepth == 0) {
            scheduleDeferredRustUpdateApplication()
        }
    }
}

internal fun EditorEditText.applyRustUpdateJSON(
    updateJSON: String,
    lineBoundaryRefreshSource: String? = null
) {
    if (externalUpdatePreparationCaptureDepth > 0) {
        capturedExternalUpdatePreparationJSON = updateJSON
        // Keep the visible IME commit authorized until the external view
        // consumes this exact adapter-adopted snapshot below.
        authorizeCurrentVisibleTextForDeferredRustUpdate()
        recordImeTraceForTesting(
            "rustUpdateCapturedForExternalPreflight",
            "jsonLength=${updateJSON.length} depth=$externalUpdatePreparationCaptureDepth"
        )
        return
    }
    if (deferredRustUpdateApplicationDepth > 0) {
        if (deferredRustUpdateJSON != null && deferredRustUpdateJSON != updateJSON) {
            advanceRenderBlocksThroughDeferredUpdate(deferredRustUpdateJSON!!)
        }
        deferredRustUpdateJSON = updateJSON
        deferredRustUpdateLineBoundaryRefreshSource = lineBoundaryRefreshSource
        recordImeTraceForTesting(
            "rustUpdateDeferred",
            "jsonLength=${updateJSON.length} depth=$deferredRustUpdateApplicationDepth"
        )
        authorizeCurrentVisibleTextForDeferredRustUpdate()
        return
    }
    recordImeTraceForTesting(
        "rustUpdateApply",
        "mode=immediate jsonLength=${updateJSON.length}"
    )
    if (applyUpdateJSON(updateJSON)) {
        lineBoundaryRefreshSource?.let(::scheduleLineBoundaryInputRefreshForEditor)
    }
}

internal fun EditorEditText.authorizeCurrentVisibleTextForDeferredRustUpdate() {
    val visibleText = text?.toString().orEmpty()
    if (visibleText != lastAuthorizedText) {
        authorizedVisibleTextNeedsRebuild = true
    }
    lastAuthorizedText = visibleText
    lastAuthorizedRenderedText = text?.let { SpannableStringBuilder(it) }
    lastAuthorizedTextRevision += 1L
    clearNativeTextMutationAdoptionSuppression()
    clearNativeTextMutationAfterBlurWindow()
}

internal fun EditorEditText.authorizeCurrentVisibleTextForPendingImeOperationForEditorImpl(
    logicalCursorAfter: Int? = null
) {
    pendingOptimisticRenderText = null
    authorizeCurrentVisibleTextForDeferredRustUpdate()
    if (logicalCursorAfter != null) {
        rememberLogicalSelection(
            scalarAnchor = logicalCursorAfter,
            scalarHead = logicalCursorAfter,
            utf16Anchor = selectionStart,
            utf16Head = selectionEnd,
            documentVersion = null
        )
    }
    recordImeTraceForTesting(
        "authorizePendingImeVisibleText",
        "textLength=${lastAuthorizedText.length}"
    )
}

internal fun EditorEditText.captureAuthoritativeInputSnapshotForEditorImpl(): AuthoritativeInputSnapshot =
    AuthoritativeInputSnapshot(
        renderedText = SpannableStringBuilder(lastAuthorizedRenderedText ?: lastAuthorizedText),
        selectionStart = selectionStart.coerceAtLeast(0),
        selectionEnd = selectionEnd.coerceAtLeast(0),
    )

internal fun EditorEditText.deleteScalarRangeForPendingImeOperationForEditorImpl(
    scalarFrom: Int,
    scalarTo: Int,
): EditorV2NativeIntentResult? {
    onDeleteRangeInRustForTesting?.let { callback ->
        runWithDeferredRustUpdateApplication {
            callback(scalarFrom, scalarTo)
        }
        return null
    }
    return v2Driver?.deleteScalarRangeNative(scalarFrom, scalarTo)
}

internal fun EditorEditText.promoteOptimisticInputForEditorImpl(
    render: EditorV2NativeMutationRender,
    logicalCursorAfter: Int,
) {
    authorizeCurrentVisibleTextForPendingImeOperationForEditor(logicalCursorAfter)
    pendingOptimisticRenderText = text?.toString()
    applyRustUpdateJSON(render.updateJson)
}

internal fun EditorEditText.restoreAuthoritativeInputForEditorImpl(
    snapshot: AuthoritativeInputSnapshot,
    recoveryUpdateJson: String? = null,
) {
    pendingOptimisticRenderText = null
    cancelDeferredRustUpdateApplication()
    if (recoveryUpdateJson != null) {
        if (applyUpdateJSON(recoveryUpdateJson, notifyListener = false)) return
    }
    val wasApplyingRustState = isApplyingRustState
    isApplyingRustState = true
    beginBatchEdit()
    try {
        setText(snapshot.renderedText)
        val length = text?.length ?: 0
        setSelection(
            snapshot.selectionStart.coerceIn(0, length),
            snapshot.selectionEnd.coerceIn(0, length),
        )
    } finally {
        endBatchEdit()
        isApplyingRustState = wasApplyingRustState
    }
}

internal fun EditorEditText.handleStructuralBackspaceImpl() {
    if (!isEditable || isApplyingRustState) return
    if (editorId == 0L) {
        handleBackspace()
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return
    val currentText = text?.toString() ?: return
    val (anchor, head) = currentLogicalScalarSelection()
        ?: normalizedScalarSelectionRange(currentText)
        ?: return
    onDeleteBackwardAtSelectionScalarInRustForTesting?.let { callback ->
        callback(anchor, head)
        return
    }
    v2Driver?.let { driver ->
        if (selectAtomBeforeEmptyTrailingParagraph(driver)) return
        val updateJSON = driver.deleteBackwardAtSelection(anchor, head)
        applyNonOptimisticRustUpdate(driver, updateJSON)
    }
}

internal fun EditorEditText.selectAtomBeforeEmptyTrailingParagraph(driver: EditorV2Driver): Boolean {
    val adapter = driver as? EditorV2Adapter ?: return false
    val content = text ?: return false
    if (selectionStart != selectionEnd) return false
    val raw = content.toString()
    val paragraphStart = raw.lastIndexOf('\n') + 1
    if (paragraphStart < 2 || selectionEnd < paragraphStart) return false
    val trailingText = raw.substring(paragraphStart)
    if (trailingText.isNotEmpty() && trailingText != LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER) return false
    val atom = content.getSpans(paragraphStart - 2, paragraphStart - 1, AtomBlockSpan::class.java)
        .firstOrNull { content.getSpanEnd(it) == paragraphStart - 1 } ?: return false
    deferredRustUpdateJSON?.let { applyUpdateJSON(it) }
    adapter.selectAtomNode(atom.docPos)?.let {
        applyUpdateJSON(it, notifyListener = false)
        editorListener?.onSelectionChanged(atom.docPos, atom.docPos + 1)
    }
    return true
}

internal fun EditorEditText.handleStructuralDeleteImpl(
    utf16From: Int,
    utf16To: Int,
    scalarFrom: Int,
    scalarTo: Int
) {
    if (!isEditable || isApplyingRustState || scalarFrom >= scalarTo) return
    if (editorId == 0L) {
        text?.delete(utf16From, utf16To)
        return
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) return
    onDeleteRangeInRustForTesting?.let { callback ->
        callback(scalarFrom, scalarTo)
        return
    }
    v2Driver?.let { driver ->
        val updateJSON = driver.deleteScalarRange(scalarFrom, scalarTo)
        applyNonOptimisticRustUpdate(driver, updateJSON)
    }
}

internal fun EditorEditText.applyNonOptimisticRustUpdate(driver: EditorV2Driver, updateJSON: String?) {
    if (driver is EditorV2Adapter) {
        driver.recoverNativeRender()?.let { applyRustUpdateJSON(it) }
        return
    }
    updateJSON?.let { applyRustUpdateJSON(it) }
}

internal fun EditorEditText.committedTokenRangeAroundMutation(
    currentText: String,
    replacementStartUtf16: Int,
    replacementEndUtf16: Int
): Pair<Int, Int>? {
    if (currentText.isEmpty()) return null
    val start = replacementStartUtf16.coerceIn(0, currentText.length)
    val end = replacementEndUtf16.coerceIn(start, currentText.length)
    val probe = when {
        start < end -> start
        start < currentText.length -> start
        start > 0 -> Character.offsetByCodePoints(currentText, start, -1)
        else -> return null
    }
    val tokenRange = missingOldTextCorrectionTokenRangeForEditor(currentText, probe) ?: return null
    return if (start < end) {
        tokenRange.takeIf { it.first <= start && it.second >= end }
    } else {
        tokenRange.takeIf { start >= it.first && start <= it.second }
    }
}

internal fun EditorEditText.scheduleDeferredRustUpdateApplication() {
    val pendingUpdateJSON = deferredRustUpdateJSON ?: return
    val pendingLineBoundaryRefreshSource = deferredRustUpdateLineBoundaryRefreshSource
    val generation = ++deferredRustUpdateGeneration
    recordImeTraceForTesting(
        "rustUpdateDeferredScheduled",
        "generation=$generation jsonLength=${pendingUpdateJSON.length}"
    )
    Handler(Looper.getMainLooper()).post {
        if (generation != deferredRustUpdateGeneration) {
            recordImeTraceForTesting(
                "rustUpdateDeferredSkip",
                "reason=generation generation=$generation current=$deferredRustUpdateGeneration"
            )
            return@post
        }
        if (deferredRustUpdateJSON != pendingUpdateJSON) {
            recordImeTraceForTesting("rustUpdateDeferredSkip", "reason=replaced generation=$generation")
            return@post
        }
        deferredRustUpdateJSON = null
        deferredRustUpdateLineBoundaryRefreshSource = null
        recordImeTraceForTesting(
            "rustUpdateApply",
            "mode=deferred generation=$generation jsonLength=${pendingUpdateJSON.length}"
        )
        if (applyUpdateJSON(pendingUpdateJSON)) {
            pendingLineBoundaryRefreshSource?.let(::scheduleLineBoundaryInputRefreshForEditor)
        }
    }
}

internal fun EditorEditText.advanceRenderBlocksThroughDeferredUpdate(updateJSON: String) {
    val update = try {
        org.json.JSONObject(updateJSON)
    } catch (_: Exception) {
        invalidateCurrentRenderBlocks()
        return
    }
    val updateDocumentVersion = canonicalV2U64(update.opt("documentVersion") as? String)
    val renderBlocks = update.optJSONArray("renderBlocks")
    val patch = parseRenderPatch(update.optJSONObject("renderPatch"))
    val resolved = renderBlocks
        ?: patch?.takeIf { patchMatchesCurrentRenderBlocks(it, updateDocumentVersion) }?.let {
            currentRenderBlocksJson?.let { current -> mergeRenderBlocks(current, it) }
        }
    retainCurrentRenderBlocks(
        resolved,
        updateDocumentVersion,
        needFullApply = resolved != null,
    )
}

internal fun EditorEditText.cancelDeferredRustUpdateApplication(invalidateRenderBlocks: Boolean = true) {
    if (deferredRustUpdateJSON == null) return
    recordImeTraceForTesting(
        "rustUpdateDeferredCancel",
        "generation=$deferredRustUpdateGeneration"
    )
    deferredRustUpdateJSON = null
    deferredRustUpdateLineBoundaryRefreshSource = null
    deferredRustUpdateGeneration += 1L
    if (invalidateRenderBlocks) {
        invalidateCurrentRenderBlocks()
    }
}
