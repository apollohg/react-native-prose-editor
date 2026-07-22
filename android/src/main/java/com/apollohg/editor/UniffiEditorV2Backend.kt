package com.apollohg.editor

import uniffi.editor_core.editorV2ApplyCommand
import uniffi.editor_core.editorV2ApplyInput
import uniffi.editor_core.editorV2ApplyLocalApi
import uniffi.editor_core.editorV2CollaborationTakeOutbound
import uniffi.editor_core.editorV2Create
import uniffi.editor_core.editorV2Destroy
import uniffi.editor_core.editorV2DocToScalar
import uniffi.editor_core.editorV2GetContentSnapshot
import uniffi.editor_core.editorV2GetDocumentHtml
import uniffi.editor_core.editorV2GetDocumentJson
import uniffi.editor_core.editorV2GetState
import uniffi.editor_core.editorV2Redo
import uniffi.editor_core.editorV2RenderUpdate
import uniffi.editor_core.editorV2ReplaceDocument
import uniffi.editor_core.editorV2ResolveScalarSelection
import uniffi.editor_core.editorV2ScalarToDoc
import uniffi.editor_core.editorV2SetSelection
import uniffi.editor_core.editorV2SnapshotExport
import uniffi.editor_core.editorV2Undo
import uniffi.editor_core.FfiBytesResult
import uniffi.editor_core.FfiError
import uniffi.editor_core.FfiJsonResult

/**
 * The production v2 backend over the real UniFFI bindings. The v2 verbs
 * call the `editorV2*` entries; render/selection/position derivation calls
 * the v2 render accessor (`editorV2RenderUpdate` and friends) against the
 * live v2 session.
 */
internal object UniffiEditorV2Backend : EditorV2Backend {

    private fun FfiError.toV2() =
        EditorV2Error(domain, code, message, requestId, operationIndex, limit, actual, detailsJson)

    private fun contractError(message: String): EditorV2Error =
        EditorV2Error(domain = "boundary", code = "FFI_RESULT_INVALID", message = message)

    private fun normalize(result: FfiJsonResult): EditorV2CallResult<String> {
        val value = result.value
        val error = result.error
        if (value != null && error == null) return EditorV2CallResult.Ok(value)
        if (value == null && error != null) return EditorV2CallResult.Err(error.toV2())
        return EditorV2CallResult.Err(contractError("v2 result must carry exactly one of value/error"))
    }

    private fun normalize(result: FfiBytesResult): EditorV2CallResult<ByteArray> {
        val value = result.value
        val error = result.error
        if (value != null && error == null) return EditorV2CallResult.Ok(value)
        if (value == null && error != null) return EditorV2CallResult.Err(error.toV2())
        return EditorV2CallResult.Err(contractError("v2 result must carry exactly one of value/error"))
    }

    override fun create(configJson: String, snapshotState: ByteArray?): EditorV2CallResult<String> =
        normalize(editorV2Create(configJson, snapshotState))

    override fun destroy(editorId: String): EditorV2Error? =
        editorV2Destroy(editorId).error?.toV2()

    override fun getState(editorId: String): EditorV2CallResult<String> =
        normalize(editorV2GetState(editorId))

    override fun getDocumentJson(editorId: String): EditorV2CallResult<String> =
        normalize(editorV2GetDocumentJson(editorId))

    override fun getDocumentHtml(editorId: String): EditorV2CallResult<String> =
        normalize(editorV2GetDocumentHtml(editorId))

    override fun getContentSnapshot(editorId: String): EditorV2CallResult<String> =
        normalize(editorV2GetContentSnapshot(editorId))

    override fun applyInput(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2ApplyInput(editorId, requestJson))

    override fun applyCommand(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2ApplyCommand(editorId, requestJson))

    override fun applyLocalApi(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2ApplyLocalApi(editorId, requestJson))

    override fun replaceDocument(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2ReplaceDocument(editorId, requestJson))

    override fun setSelection(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2SetSelection(editorId, requestJson))

    override fun undo(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2Undo(editorId, requestJson))

    override fun redo(editorId: String, requestJson: String): EditorV2CallResult<String> =
        normalize(editorV2Redo(editorId, requestJson))

    override fun collaborationTakeOutbound(editorId: String, generation: ULong): EditorV2CallResult<ByteArray> =
        normalize(editorV2CollaborationTakeOutbound(editorId, generation))

    override fun snapshotExport(editorId: String): EditorV2CallResult<Pair<String, ByteArray>> {
        val result = editorV2SnapshotExport(editorId)
        val value = result.value
        val error = result.error
        if (value != null && error == null) {
            return EditorV2CallResult.Ok(value.metadataJson to value.encodedState)
        }
        if (error != null) return EditorV2CallResult.Err(error.toV2())
        return EditorV2CallResult.Err(contractError("v2 result must carry exactly one of value/error"))
    }

    // ── v2 render/selection/position accessor ──

    override fun renderUpdate(
        editorId: String,
        mirrorAnchor: Int?,
        mirrorHead: Int?,
    ): EditorV2CallResult<String> =
        normalize(
            editorV2RenderUpdate(
                editorId,
                mirrorAnchor?.toUInt(),
                mirrorHead?.toUInt(),
            )
        )

    override fun resolveScalarSelection(editorId: String, anchor: Int, head: Int): EditorV2CallResult<String> =
        normalize(editorV2ResolveScalarSelection(editorId, anchor.toUInt(), head.toUInt()))

    override fun docToScalar(editorId: String, docPos: Int): EditorV2CallResult<String> =
        normalize(editorV2DocToScalar(editorId, docPos.toUInt()))

    override fun scalarToDoc(editorId: String, scalar: Int): EditorV2CallResult<String> =
        normalize(editorV2ScalarToDoc(editorId, scalar.toUInt()))
}
