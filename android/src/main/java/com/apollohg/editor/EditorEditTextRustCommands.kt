package com.apollohg.editor

import org.json.JSONObject

/**
     * Insert text at a scalar position via the Rust editor.
     */
internal fun EditorEditText.insertTextInRust(text: String, atScalarPos: Int) {
    if (!hasLiveEditor()) return
    onInsertTextInRustForTesting?.let { callback ->
        callback(text, atScalarPos)
        return
    }
    v2Driver?.let { driver ->
        driver.insertText(text, atScalarPos)?.let { applyRustUpdateJSON(it) }
    }
}

internal fun EditorEditText.replaceTextRangeInRust(scalarFrom: Int, scalarTo: Int, text: String) {
    if (!hasLiveEditor()) return
    onReplaceTextInRustForTesting?.let { callback ->
        callback(scalarFrom, scalarTo, text)
        return
    }
    v2Driver?.let { driver ->
        driver.replaceTextRange(scalarFrom, scalarTo, text)?.let { applyRustUpdateJSON(it) }
    }
}

internal fun EditorEditText.insertPlainTextRangeInRust(
    scalarFrom: Int,
    scalarTo: Int,
    text: String,
    requestedCursorScalar: Int? = null
) {
    if (!hasLiveEditor()) return
    recordImeTraceForTesting(
        "rustPlainTextRoute",
        "range=$scalarFrom..$scalarTo textLength=${text.length} requestedCursor=$requestedCursorScalar"
    )
    if (text.isEmpty()) {
        if (scalarFrom != scalarTo) {
            deleteRangeInRust(scalarFrom, scalarTo)
        }
        applyRequestedCursorScalar(requestedCursorScalar)
        return
    }
    if (text.indexOf('\n') >= 0 || text.indexOf('\r') >= 0) {
        val docJson = plainTextDocumentFragmentJson(text)
        onInsertContentJsonAtSelectionScalarForTesting?.let { callback ->
            callback(scalarFrom, scalarTo, docJson)
            applyRequestedCursorScalar(requestedCursorScalar)
            return
        }
        v2Driver?.let { driver ->
            driver.insertContentJsonAtSelection(docJson, scalarFrom, scalarTo)?.let { applyRustUpdateJSON(it) }
        }
        applyRequestedCursorScalar(requestedCursorScalar)
        return
    }

    if (scalarFrom != scalarTo) {
        replaceTextRangeInRust(scalarFrom, scalarTo, text)
    } else {
        insertTextInRust(text, scalarFrom)
    }
    applyRequestedCursorScalar(requestedCursorScalar)
}

internal fun EditorEditText.requestedCursorScalar(
    scalarFrom: Int,
    scalarTo: Int,
    currentText: String,
    insertedText: String,
    newCursorPosition: Int
): Int? {
    if (newCursorPosition == 1) return null
    val insertedScalarLength = insertedText.codePointCount(0, insertedText.length)
    val currentScalarLength = currentText.codePointCount(0, currentText.length)
    val nextScalarLength =
        (currentScalarLength - (scalarTo - scalarFrom) + insertedScalarLength).coerceAtLeast(0)
    val requested = if (newCursorPosition > 0) {
        scalarFrom + insertedScalarLength + newCursorPosition - 1
    } else {
        scalarFrom + newCursorPosition
    }
    return requested.coerceIn(0, nextScalarLength)
}

internal fun EditorEditText.applyRequestedCursorScalar(requestedCursorScalar: Int?) {
    val requested = requestedCursorScalar ?: return
    if (!hasLiveEditor()) return
    val safeScalar = requested.coerceAtLeast(0)
    val cursorDriver = v2Driver
    if (cursorDriver != null) {
        cursorDriver.syncSelectionQuiet(safeScalar, safeScalar)?.let(::applyRustUpdateJSON)
    } else {
        onSetSelectionScalarInRustForTesting?.let { callback ->
            callback(safeScalar, safeScalar)
        }
    }
    val currentText = text?.toString().orEmpty()
    val localScalar = safeScalar.coerceIn(0, currentText.codePointCount(0, currentText.length))
    val safeUtf16 = PositionBridge.scalarToUtf16(localScalar, currentText)
        .coerceIn(0, currentText.length)
    if (selectionStart != safeUtf16 || selectionEnd != safeUtf16) {
        setSelection(safeUtf16, safeUtf16)
    }
}

internal fun EditorEditText.plainTextDocumentFragmentJson(text: String): String {
    val normalizedText = text.replace("\r\n", "\n").replace('\r', '\n')
    val content = org.json.JSONArray()
    for (line in normalizedText.split('\n')) {
        val paragraph = org.json.JSONObject().put("type", "paragraph")
        if (line.isNotEmpty()) {
            paragraph.put(
                "content",
                org.json.JSONArray().put(
                    org.json.JSONObject()
                        .put("type", "text")
                        .put("text", line)
                )
            )
        }
        content.put(paragraph)
    }
    return org.json.JSONObject()
        .put("type", "doc")
        .put("content", content)
        .toString()
}

    /**
     * Delete a scalar range via the Rust editor.
     *
     * @param scalarFrom Start scalar offset (inclusive).
     * @param scalarTo End scalar offset (exclusive).
     */
internal fun EditorEditText.deleteRangeInRust(scalarFrom: Int, scalarTo: Int) {
    if (!hasLiveEditor()) return
    if (scalarFrom >= scalarTo) return
    onDeleteRangeInRustForTesting?.let { callback ->
        callback(scalarFrom, scalarTo)
        return
    }
    v2Driver?.let { driver ->
        driver.deleteScalarRange(scalarFrom, scalarTo)?.let { applyRustUpdateJSON(it) }
    }
}

internal fun EditorEditText.deleteBackwardAtSelectionScalarInRust(scalarAnchor: Int, scalarHead: Int) {
    if (!hasLiveEditor()) return
    onDeleteBackwardAtSelectionScalarInRustForTesting?.let { callback ->
        callback(scalarAnchor, scalarHead)
        return
    }
    v2Driver?.let { driver ->
        driver.deleteBackwardAtSelection(scalarAnchor, scalarHead)?.let { applyRustUpdateJSON(it) }
    }
}

internal fun EditorEditText.toggleTaskItemCheckedAtSelectionScalarInRust(scalarAnchor: Int, scalarHead: Int) {
    if (!hasLiveEditor()) return
    onToggleTaskItemCheckedAtSelectionScalarInRustForTesting?.let { callback ->
        callback(scalarAnchor, scalarHead)
        return
    }
    v2Driver?.let { driver ->
        driver.toggleTaskItemCheckedAtSelection(scalarAnchor, scalarHead)?.let { applyRustUpdateJSON(it) }
    }
}

    /**
     * Split a block at a scalar position via the Rust editor.
     */
internal fun EditorEditText.splitBlockInRust(atScalarPos: Int) {
    if (!hasLiveEditor()) return
    onSplitBlockInRustForTesting?.let { callback ->
        callback(atScalarPos)
        return
    }
    v2Driver?.let { driver ->
        driver.splitBlockAt(atScalarPos)?.let { result ->
            applyRustUpdateJSON(
                result.updateJson,
                lineBoundaryRefreshSource = if (result.committed) "splitBlock" else null,
            )
        }
    }
}

internal fun EditorEditText.deleteAndSplitInRust(scalarFrom: Int, scalarTo: Int) {
    if (!hasLiveEditor()) return
    onDeleteAndSplitScalarInRustForTesting?.let { callback ->
        callback(scalarFrom, scalarTo)
        return
    }
    v2Driver?.let { driver ->
        driver.deleteAndSplit(scalarFrom, scalarTo)?.let { result ->
            applyRustUpdateJSON(
                result.updateJson,
                lineBoundaryRefreshSource = if (result.committed) "deleteAndSplit" else null,
            )
        }
    }
}

internal fun EditorEditText.isSelectionInsideList(): Boolean {
    if (!hasLiveEditor()) return false

    return try {
        val stateJson = v2Driver?.currentStateJson() ?: return false
        val state = org.json.JSONObject(stateJson)
        val nodes = state.optJSONObject("activeState")?.optJSONObject("nodes")
        nodes?.keys()?.asSequence()?.any { nodeType ->
            EditorNodeTypes.isListContainer(nodeType) && nodes.optBoolean(nodeType, false)
        } == true
    } catch (_: Exception) {
        false
    }
}

internal fun EditorEditText.preferredHardBreakNodeType(): String {
    return try {
        val stateJson = v2Driver?.currentStateJson() ?: return "hardBreak"
        val insertableNodes = org.json.JSONObject(stateJson)
            .optJSONObject("activeState")
            ?.optJSONArray("insertableNodes")
        val names = buildSet {
            if (insertableNodes != null) {
                for (index in 0 until insertableNodes.length()) {
                    insertableNodes.optString(index, null)?.let(::add)
                }
            }
        }
        EditorNodeTypes.preferredHardBreak(names)
    } catch (_: Exception) {
        "hardBreak"
    }
}

    /**
     * Paste HTML content through Rust.
     */
internal fun EditorEditText.pasteHTML(html: String) {
    if (!hasLiveEditor()) return
    syncCurrentSelectionToRust()
    onInsertContentHtmlInRustForTesting?.let { callback ->
        callback(html)
        return
    }
    v2Driver?.let { driver ->
        val selection = currentScalarSelection()
        val update = if (selection != null) {
            driver.insertContentHtmlAtSelection(html, selection.first, selection.second)
        } else {
            null
        }
        update?.let { applyUpdateJSON(it) }
    }
}

    /**
     * Paste plain text through Rust.
     */
internal fun EditorEditText.pastePlainText(text: String) {
    val (scalarStart, scalarEnd) = currentScalarSelection() ?: return
    insertPlainTextRangeInRust(scalarStart, scalarEnd, text)
}
