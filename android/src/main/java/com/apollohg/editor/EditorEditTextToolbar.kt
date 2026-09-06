package com.apollohg.editor

internal fun EditorEditText.prepareForToolbarCommandWithExternalOwner(): Boolean {
    if (!isEditable || isApplyingRustState || !hasLiveEditor()) return false
    if (externalTextComposition == null) return true
    return commitExternalTextCompositionBeforeInteractionIfNeeded() &&
        prepareForExternalEditorUpdate()
}

internal fun EditorEditText.performToolbarToggleMarkImpl(markName: String) {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.toggleMark(markName, selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarToggleListImpl(listType: String, isActive: Boolean) {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        val update = if (isActive) {
            driver.unwrapFromList(selection.first, selection.second)
        } else {
            driver.wrapInList(listType, selection.first, selection.second)
        }
        update?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarToggleBlockquoteImpl() {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.toggleBlockquote(selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarToggleHeadingImpl(level: Int) {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    if (level !in 1..6) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.toggleHeading(level, selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarIndentListItemImpl() {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.indentListItem(selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarOutdentListItemImpl() {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.outdentListItem(selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarInsertNodeImpl(nodeType: String) {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    val selection = currentScalarSelection() ?: return
    v2Driver?.let { driver ->
        driver.insertNode(nodeType, selection.first, selection.second)?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarUndoImpl() {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    v2Driver?.let { driver ->
        driver.undo()?.let { applyUpdateJSON(it) }
    }
}

internal fun EditorEditText.performToolbarRedoImpl() {
    if (!prepareForToolbarCommandWithExternalOwner()) return
    v2Driver?.let { driver ->
        driver.redo()?.let { applyUpdateJSON(it) }
    }
}
