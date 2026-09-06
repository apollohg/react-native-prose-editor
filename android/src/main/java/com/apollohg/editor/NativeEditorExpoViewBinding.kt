package com.apollohg.editor

/** Public v2 handles are decimal strings; [RichTextEditorView] uses an opaque local token. */
internal fun NativeEditorExpoView.publicHandleForViewToken(viewToken: Long): String? =
    EditorV2Registry.handleForViewToken(viewToken)

    /** Events never expose a signed widget token as a v2 editor id. */
internal fun NativeEditorExpoView.eventEditorId(viewToken: Long): String =
    publicHandleForViewToken(viewToken) ?: "0"

internal fun NativeEditorExpoView.setEditorHandleImpl(handle: String?) {
    val viewToken = handle?.let(EditorV2Registry::viewTokenForHandle)
    setEditorId(viewToken ?: 0L)
}

    /**
     * Internal-only widget binding. This token is allocated by
     * [EditorV2Registry] and is never a public session identifier.
     */
internal fun NativeEditorExpoView.setEditorIdImpl(id: Long) {
    if (id != 0L && NativeEditorViewRegistry.isDestroyed(id)) {
        setEditorId(0L)
        return
    }
    val previousEditorId = richTextView.editorId
    if (previousEditorId != id) {
        cancelActiveExternalTextComposition("lifecycle")
        invalidateAutoGrowContentHeightEmission()
        drainPendingEditorUpdateEvents()
        clearEditorErrorBinding("editorRebind")
    }
    if (previousEditorId == id && richTextView.editorEditText.editorId == id) {
        if (id != 0L && isAttachedToNativeWindow) {
            if (!NativeEditorViewRegistry.register(id, this)) {
                handleEditorDestroyed(id)
                return
            }
            bindEditorErrorCallbackIfLive(id)
            applyPendingEditorResetUpdateIfNeeded()
            applyPendingEditorUpdateIfNeeded()
            applyPendingThemeIfNeeded()
            applyPendingAtomsIfNeeded()
            refreshReadyStateIfSettled()
            applyAutoFocusIfNeeded()
        } else if (id != 0L) {
            NativeEditorViewRegistry.unregister(
                id,
                this,
                blockCommandsUntilRegistered = true
            )
        }
        richTextView.emitAtomLayoutIfAvailable(force = true)
        return
    }
    if (previousEditorId != id) {
        NativeEditorViewRegistry.unregister(previousEditorId, this)
        lastDocumentVersion = null
        renderedDocumentRevision = null
        cancelPendingToolbarRefocus()
        cancelPendingEditorUpdateRetry()
        if (pendingEditorUpdateEditorId != null && pendingEditorUpdateEditorId != id) {
            clearPendingEditorUpdateState()
        }
        if (pendingEditorResetUpdateEditorId != null && pendingEditorResetUpdateEditorId != id) {
            clearPendingEditorResetUpdateState()
        }
        appliedEditorUpdateRevision = 0L
        appliedEditorResetUpdateRevision = 0L
        consumedEditorUpdateRevision = 0L
        consumedEditorUpdateEditorId = null
        consumedEditorResetUpdateRevision = 0L
        consumedEditorResetUpdateEditorId = null
        clearPendingViewCommandUpdateRetry()
        cancelPendingThemeRetry()
        if (hasPendingTheme) {
            pendingThemeRetry.bind(id)
        }
        cancelPendingAtomsRetry()
        if (hasPendingAtoms) {
            pendingAtomsRetry.bind(id)
        }
        cancelPendingBlurRetry()
        clearPendingNativeActionRetry()
        clearMentionQueryState(resetLastEvent = true)
        lastReadyEditorId = null
    }
    if (!isAttachedToNativeWindow) {
        richTextView.setEditorIdWhileDetached(id)
        if (id != 0L) {
            NativeEditorViewRegistry.unregister(
                id,
                this,
                blockCommandsUntilRegistered = true
            )
        } else {
            toolbarState = NativeToolbarState.empty
            keyboardToolbarView.applyState(toolbarState)
        }
        richTextView.emitAtomLayoutIfAvailable(force = true)
        return
    }

    if (hasPendingEditorResetUpdateForEditor(id) || hasPendingEditorUpdateForEditor(id)) {
        richTextView.setEditorIdWhileDetached(id)
        richTextView.rebindEditorIfNeeded(notifyListener = false)
    } else {
        richTextView.editorId = id
    }
    if (id != 0L) {
        if (!NativeEditorViewRegistry.register(id, this)) {
            handleEditorDestroyed(id)
            return
        }
        bindEditorErrorCallbackIfLive(id)
    } else {
        toolbarState = NativeToolbarState.empty
        keyboardToolbarView.applyState(toolbarState)
    }
    applyPendingEditorResetUpdateIfNeeded()
    applyPendingEditorUpdateIfNeeded()
    applyPendingThemeIfNeeded()
    applyPendingAtomsIfNeeded()
    refreshReadyStateIfSettled()
    applyAutoFocusIfNeeded()
    richTextView.emitAtomLayoutIfAvailable(force = true)
}
