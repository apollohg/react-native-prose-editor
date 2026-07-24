package com.apollohg.editor

/**
 * The v2 backend contract: the v2 engine verbs plus the v2
 * render/selection/position accessor. The production implementation
 * (`UniffiEditorV2Backend`) calls the real `editorV2*` UniFFI entries;
 * tests substitute `FakeEditorV2Backend`.
 */
internal data class EditorV2Error(
    val domain: String,
    val code: String,
    val message: String,
    val requestId: String? = null,
    val operationIndex: String? = null,
    val limit: String? = null,
    val actual: String? = null,
    val detailsJson: String? = null,
)

internal sealed interface EditorV2CallResult<out T> {
    data class Ok<T>(val value: T) : EditorV2CallResult<T>
    data class Err(val error: EditorV2Error) : EditorV2CallResult<Nothing>
}

internal data class EditorV2OutboundLease(val leaseId: String, val frame: ByteArray)

internal sealed interface EditorV2LeaseResult {
    data class Value(val lease: EditorV2OutboundLease) : EditorV2LeaseResult
    data object Empty : EditorV2LeaseResult
    data class Err(val error: EditorV2Error) : EditorV2LeaseResult
}

internal interface EditorV2Backend {
    fun create(configJson: String, snapshotState: ByteArray?): EditorV2CallResult<String>
    fun destroy(editorId: String): EditorV2Error?
    fun getState(editorId: String): EditorV2CallResult<String>
    fun getDocumentJson(editorId: String): EditorV2CallResult<String>
    fun getDocumentHtml(editorId: String): EditorV2CallResult<String>
    fun getContentSnapshot(editorId: String): EditorV2CallResult<String>
    fun applyInput(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun applyCommand(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun applyLocalApi(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun replaceDocument(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun setSelection(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun undo(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun redo(editorId: String, requestJson: String): EditorV2CallResult<String>
    fun collaborationDrive(editorId: String, nowMillis: String): EditorV2CallResult<String>
    fun collaborationSocketOpen(
        editorId: String,
        generation: String,
        nowMillis: String,
    ): EditorV2CallResult<String>
    fun collaborationReceive(
        editorId: String,
        generation: String,
        message: ByteArray,
        nowMillis: String,
    ): EditorV2CallResult<String>
    fun collaborationSocketClose(
        editorId: String,
        generation: String,
        code: UInt?,
        reason: String?,
        nowMillis: String,
    ): EditorV2CallResult<String>
    fun collaborationLeaseOutbound(editorId: String, generation: String): EditorV2LeaseResult
    fun collaborationAckOutbound(
        editorId: String,
        generation: String,
        leaseId: String,
    ): EditorV2CallResult<String>
    fun collaborationNackOutbound(
        editorId: String,
        generation: String,
        leaseId: String,
    ): EditorV2CallResult<String>
    fun collaborationDetach(editorId: String): EditorV2Error?
    fun collaborationReattach(editorId: String): EditorV2Error?
    fun collaborationPeers(editorId: String): EditorV2CallResult<String>
    fun snapshotExport(editorId: String): EditorV2CallResult<Pair<String, ByteArray>>

    /**
     * The v2 render accessor: from the LIVE v2 session produce the
     * complete locked atomic snapshot: render blocks/patch, selection,
     * active/history state, canonical document/state revisions, and scalar
     * extent. The adapter must parse and adopt this one result as-is; it
     * must never stitch it to a separate `getState` response.
     */
    fun renderUpdate(editorId: String, mirrorAnchor: Int?, mirrorHead: Int?): EditorV2CallResult<String>

    /** Engine-authoritative scalar→doc selection resolution for one session. */
    fun resolveScalarSelection(editorId: String, anchor: Int, head: Int): EditorV2CallResult<String>

    /** Lenient doc→scalar mapping (clamps at the document extent). */
    fun docToScalar(editorId: String, docPos: Int): EditorV2CallResult<String>

    /** Lenient scalar→doc mapping (clamps at the document extent). */
    fun scalarToDoc(editorId: String, scalar: Int): EditorV2CallResult<String>
}
