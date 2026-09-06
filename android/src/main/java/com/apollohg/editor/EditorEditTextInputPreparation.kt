package com.apollohg.editor

import com.apollohg.editor.EditorEditText.ExternalEditorUpdatePreparation
import com.apollohg.editor.EditorEditText.CommandPreparation

internal fun EditorEditText.prepareForExternalEditorUpdateImpl(): Boolean =
    prepareForExternalEditorUpdateInternal().ready

    /**
     * Performs external-update preflight while retaining a mutation snapshot
     * for the caller that will apply it. This prevents a second state render
     * after a composing commit has already produced and adopted one.
     */
internal fun EditorEditText.hasPendingCompositionForExternalRefreshImpl(): Boolean =
    externalTextComposition != null || activeInputConnection?.hasPendingComposition() == true

internal fun EditorEditText.prepareForExternalEditorUpdateWithResultImpl(): ExternalEditorUpdatePreparation {
    externalUpdatePreparationCaptureDepth += 1
    if (externalUpdatePreparationCaptureDepth == 1) {
        capturedExternalUpdatePreparationJSON = null
    }
    return try {
        val preparation = prepareForExternalEditorUpdateInternal()
        ExternalEditorUpdatePreparation(
            ready = preparation.ready,
            adoptedUpdateJSON = capturedExternalUpdatePreparationJSON
        )
    } finally {
        externalUpdatePreparationCaptureDepth -= 1
        if (externalUpdatePreparationCaptureDepth == 0) {
            capturedExternalUpdatePreparationJSON = null
        }
    }
}

internal fun EditorEditText.prepareForExternalEditorUpdateInternal(): ExternalEditorUpdatePreparation {
    if (blockExternalEditorUpdatePreparationForTesting) {
        return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
    }
    if (discardTransientInputForDestroyedEditorIfNeeded()) {
        return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
    }
    if (!commitExternalTextCompositionForDocumentChangeIfNeeded()) {
        return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
    }
    val inputConnection = activeInputConnection
    if (inputConnection?.flushPendingCompositionForExternalMutation() == false) {
        return ExternalEditorUpdatePreparation(ready = false, adoptedUpdateJSON = null)
    }
    return ExternalEditorUpdatePreparation(
        ready = drainNativeTextMutationIfNeeded(
            allowAfterBlur = true,
            preserveInputConnectionForExternalUpdate = true
        ),
        adoptedUpdateJSON = null
    )
}

internal fun EditorEditText.prepareForExternalEditorCommandImpl(): CommandPreparation {
    if (blockExternalEditorCommandPreparationForTesting) {
        return CommandPreparation(ready = false, updateJSON = null)
    }
    val previousAuthorizedText = lastAuthorizedText
    val preflight = prepareForExternalEditorUpdateWithResult()
    if (!preflight.ready) {
        return CommandPreparation(ready = false, updateJSON = null)
    }
    if (preflight.adoptedUpdateJSON != null) {
        return CommandPreparation(ready = true, updateJSON = preflight.adoptedUpdateJSON)
    }
    if (!hasLiveEditor() || lastAuthorizedText == previousAuthorizedText) {
        return CommandPreparation(ready = true, updateJSON = null)
    }
    val commandStateJSON = v2Driver?.currentStateJson()
    return CommandPreparation(
        ready = true,
        updateJSON = commandStateJSON
    )
}
