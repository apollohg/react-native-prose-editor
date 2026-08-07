package com.apollohg.editor

/**
 * Render returned by a block-split request. A refresh-only render keeps the
 * native view coherent but must not be treated as a locally committed split.
 */
internal data class EditorV2SplitRender(
    val updateJson: String,
    val committed: Boolean,
)

/**
 * The v2 driver: the ONLY engine path for views.
 *
 * The views call this interface for every engine interaction; the legacy
 * UniFFI free functions are gone. `EditorV2Adapter` supplies the
 * implementation, whose typed v2 transactions/results are the only engine
 * traffic for paired editors.
 */
internal interface EditorV2Driver {
    fun insertText(text: String, atScalarPos: Int): String?
    fun replaceTextRange(scalarFrom: Int, scalarTo: Int, text: String): String?
    fun deleteScalarRange(scalarFrom: Int, scalarTo: Int): String?
    fun deleteBackwardAtSelection(anchor: Int, head: Int): String?
    fun splitBlockAt(scalarPos: Int): EditorV2SplitRender?
    fun deleteAndSplit(scalarFrom: Int, scalarTo: Int): EditorV2SplitRender?

    fun insertNode(nodeType: String, anchor: Int, head: Int): String?
    fun insertContentHtmlAtSelection(html: String, anchor: Int, head: Int): String?
    fun insertContentJsonAtSelection(json: String, anchor: Int, head: Int): String?
    fun toggleMark(markName: String, anchor: Int, head: Int): String?
    fun setMark(markName: String, attrsJson: String, anchor: Int, head: Int): String?
    fun unsetMark(markName: String, anchor: Int, head: Int): String?
    fun toggleHeading(level: Int, anchor: Int, head: Int): String?
    fun toggleCodeBlock(anchor: Int, head: Int): String?
    fun toggleBlockquote(anchor: Int, head: Int): String?
    fun wrapInList(listType: String, anchor: Int, head: Int): String?
    fun unwrapFromList(anchor: Int, head: Int): String?
    fun indentListItem(anchor: Int, head: Int): String?
    fun outdentListItem(anchor: Int, head: Int): String?
    fun toggleTaskItemCheckedAtSelection(anchor: Int, head: Int): String?
    fun resizeImageAtDocPos(docPos: Int, width: Int, height: Int): String?

    fun undo(): String?
    fun redo(): String?

    /** Sync the scalar selection and return [docAnchor, docHead], or null on structured failure. */
    fun syncSelection(anchor: Int, head: Int): IntArray?
    /** Selection sync where no doc mapping is consumed. */
    fun syncSelectionQuiet(anchor: Int, head: Int)
    fun scalarPositionForDoc(docPos: Int): Int?
    fun docPositionForScalar(scalar: Int): Int?

    fun currentStateJson(): String?
    fun documentHtml(): String?
    fun documentJson(): String?
    fun contentSnapshotJson(): String?
    fun historyCanUndo(): Boolean?
    fun historyCanRedo(): Boolean?
    fun selectionJson(): String?

    fun setContentHtml(html: String): String?
    fun setContentJson(json: String): String?

    /** Stale-revision recovery: refresh from Rust state, never retry the failed op. */
    fun refreshFromRustState(mirrorSelection: IntArray?): String?

}
