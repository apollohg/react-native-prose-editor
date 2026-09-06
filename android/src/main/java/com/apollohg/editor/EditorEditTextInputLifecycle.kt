package com.apollohg.editor

import android.content.Context
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.InputMethodManager

internal fun EditorEditText.restartInputForEditorIfFocused(source: String) {
    if (!hasFocus()) return
    restartInputForEditor(source)
}

internal fun EditorEditText.restartInputForEditor(source: String = "explicit") {
    recordImeTraceForTesting("restartInput", "source=$source")
    val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
    imm?.restartInput(this)
    scheduleSelectionUpdateAfterRestartInput(source)
}

internal fun EditorEditText.scheduleSelectionUpdateAfterRestartInput(source: String) {
    val generation = ++restartInputSelectionUpdateGeneration
    post {
        if (generation != restartInputSelectionUpdateGeneration) return@post
        if (!hasFocus()) return@post
        val start = selectionStart
        val end = selectionEnd
        if (start < 0 || end < 0) {
            recordImeTraceForTesting(
                "updateSelectionAfterRestartSkipped",
                "source=$source reason=selection start=$start end=$end"
            )
            return@post
        }
        val mapper = imeTextCoordinateMapperForEditor() ?: return@post
        val imeStart = mapper.rawToIme(start)
        val imeEnd = mapper.rawToIme(end)
        val editable = text
        val rawComposingStart = editable?.let(BaseInputConnection::getComposingSpanStart) ?: -1
        val rawComposingEnd = editable?.let(BaseInputConnection::getComposingSpanEnd) ?: -1
        val imeComposingStart = if (rawComposingStart >= 0) {
            mapper.rawToIme(rawComposingStart)
        } else {
            -1
        }
        val imeComposingEnd = if (rawComposingEnd >= 0) {
            mapper.rawToIme(rawComposingEnd)
        } else {
            -1
        }
        val imm = context.getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
        imm?.updateSelection(
            this,
            imeStart,
            imeEnd,
            imeComposingStart,
            imeComposingEnd,
        )
        recordImeTraceForTesting(
            "updateSelectionAfterRestart",
            "source=$source sel=$imeStart..$imeEnd"
        )
    }
}

internal fun EditorEditText.scheduleLineBoundaryInputRefreshForEditor(source: String) {
    if (!hasFocus()) return
    val generation = ++lineBoundaryInputRefreshGeneration
    recordImeTraceForTesting(
        "lineBoundaryInputRefreshScheduled",
        "source=$source generation=$generation"
    )
    post {
        if (generation != lineBoundaryInputRefreshGeneration) return@post
        if (!hasFocus()) return@post
        if (!isCursorAtRenderedLineStartForSentenceCaps()) {
            recordImeTraceForTesting(
                "lineBoundaryInputRefreshSkipped",
                "source=$source reason=cursor"
            )
            return@post
        }
        restartInputForEditor("lineBoundary:$source")
    }
}

internal fun EditorEditText.clearCompositionInvalidationForEditor() {
    didInvalidateCompositionReplacementRange = false
}

internal fun EditorEditText.nextInputConnectionGenerationForEditor(): Long {
    return inputConnectionGeneration
}

internal fun EditorEditText.isInputConnectionCurrentForEditorImpl(
    boundEditorId: Long,
    boundGeneration: Long
): Boolean =
    editorId == boundEditorId &&
        inputConnectionGeneration == boundGeneration &&
        !isEditorDestroyedForInput()

internal fun EditorEditText.invalidateInputConnectionsForEditor() {
    inputConnectionGeneration += 1L
    cachedImeTextCoordinateMapper = null
    recordImeTraceForTesting("invalidateInputConnections", "nextGen=$inputConnectionGeneration")
    activeInputConnection = null
}

internal fun EditorEditText.imeTextCoordinateMapperForEditorImpl(
    boundGeneration: Long = inputConnectionGeneration,
): ImeTextCoordinateMapper? {
    if (boundGeneration != inputConnectionGeneration) return null
    val cached = cachedImeTextCoordinateMapper
    if (
        cached != null &&
        cached.generation == boundGeneration &&
        cachedImeTextCoordinateRevision == imeTextCoordinateRevision
    ) {
        return cached
    }
    return ImeTextCoordinateMapper.build(text ?: "", boundGeneration).also { mapper ->
        cachedImeTextCoordinateMapper = mapper
        cachedImeTextCoordinateRevision = imeTextCoordinateRevision
    }
}

internal fun EditorEditText.invalidateImeTextCoordinateMapperForEditor() {
    imeTextCoordinateRevision += 1L
    cachedImeTextCoordinateMapper = null
}

internal fun EditorEditText.clearNativeComposingSpans() {
    val editable = text ?: return
    BaseInputConnection.removeComposingSpans(editable)
    removeTransientComposingTextStyleSpans(editable)
}

internal fun EditorEditText.restoreAuthorizedTextIfNeededImpl() {
    if (!hasLiveEditor()) return
    if ((text?.toString() ?: "") == lastAuthorizedText) return
    recordImeTraceForTesting(
        "restoreAuthorizedText",
        "authorizedLength=${lastAuthorizedText.length}"
    )
    val stateJSON = v2Driver?.currentStateJson() ?: return
    applyUpdateJSON(stateJSON)
}

internal fun EditorEditText.discardTransientNativeInputForEditorRebindImpl() {
    cancelExternalTextCompositionForLifecycleIfNeeded()
    retireInputConnectionForEditor()
    nativeTextMutationAfterBlurWindow = null
    lastAllowedAtomCaretSelection = null
    clearNativeTextMutationAdoptionSuppression()
    clearImeTraceForTesting()
}

internal fun EditorEditText.discardTransientNativeInputForExternalRecoveryImpl() {
    cancelExternalTextCompositionForLifecycleIfNeeded()
    retireInputConnectionForEditor()
    nativeTextMutationAfterBlurWindow = null
    restoreAuthorizedTextIfNeeded()
    suppressNativeTextMutationAdoptionForCurrentRevision()
}

internal fun EditorEditText.discardTransientNativeInputForReadOnly() {
    discardTransientNativeInputForExternalRecovery()
}

internal fun EditorEditText.refreshInputConnectionAfterExternalTextReplacementIfNeeded(
    enabled: Boolean,
    previousVisibleText: String
) {
    if (!enabled || !hasFocus()) return
    val currentVisibleText = text?.toString().orEmpty()
    if (currentVisibleText == previousVisibleText) return
    clearInputStateForExternalReplacementPreservingConnection()
    restartInputForEditor("externalUpdate")
}

internal fun EditorEditText.clearInputStateForExternalReplacementPreservingConnection() {
    activeInputConnection?.clearCompositionTrackingForEditor()
    clearCompositionTrackingForEditor()
    clearCompositionInvalidationForEditor()
    clearNativeComposingSpans()
}
