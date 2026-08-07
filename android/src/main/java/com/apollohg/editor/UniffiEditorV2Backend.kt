package com.apollohg.editor

import uniffi.editor_core.editorV2ApplyCommand
import uniffi.editor_core.editorV2ApplyInput
import uniffi.editor_core.editorV2ApplyLocalApi
import uniffi.editor_core.editorV2CollaborationAckOutbound
import uniffi.editor_core.editorV2CollaborationDetach
import uniffi.editor_core.editorV2CollaborationDrive
import uniffi.editor_core.editorV2CollaborationLeaseOutbound
import uniffi.editor_core.editorV2CollaborationNackOutbound
import uniffi.editor_core.editorV2CollaborationPeers
import uniffi.editor_core.editorV2CollaborationReattach
import uniffi.editor_core.editorV2CollaborationReceive
import uniffi.editor_core.editorV2CollaborationSetAwarenessSelection
import uniffi.editor_core.editorV2CollaborationSocketClose
import uniffi.editor_core.editorV2CollaborationSocketOpen
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

    override fun collaborationDrive(editorId: String, nowMillis: String): EditorV2CallResult<String> =
        normalize(editorV2CollaborationDrive(editorId, nowMillis))

    override fun collaborationSocketOpen(
        editorId: String,
        generation: String,
        nowMillis: String,
    ): EditorV2CallResult<String> =
        normalize(editorV2CollaborationSocketOpen(editorId, generation, nowMillis))

    override fun collaborationReceive(
        editorId: String,
        generation: String,
        message: ByteArray,
        nowMillis: String,
    ): EditorV2CallResult<String> =
        normalize(editorV2CollaborationReceive(editorId, generation, message, nowMillis))

    override fun collaborationSocketClose(
        editorId: String,
        generation: String,
        code: UInt?,
        reason: String?,
        nowMillis: String,
    ): EditorV2CallResult<String> =
        normalize(editorV2CollaborationSocketClose(editorId, generation, code, reason, nowMillis))

    override fun collaborationLeaseOutbound(
        editorId: String,
        generation: String,
    ): EditorV2LeaseResult {
        val result = editorV2CollaborationLeaseOutbound(editorId, generation)
        val value = result.value
        val error = result.error
        return when {
            value != null && !result.empty && error == null ->
                EditorV2LeaseResult.Value(EditorV2OutboundLease(value.leaseId, value.frame))
            value == null && result.empty && error == null -> EditorV2LeaseResult.Empty
            value == null && !result.empty && error != null -> EditorV2LeaseResult.Err(error.toV2())
            else -> EditorV2LeaseResult.Err(contractError("v2 lease result violates the frozen shape"))
        }
    }

    override fun collaborationAckOutbound(
        editorId: String,
        generation: String,
        leaseId: String,
    ): EditorV2CallResult<String> =
        normalize(editorV2CollaborationAckOutbound(editorId, generation, leaseId))

    override fun collaborationNackOutbound(
        editorId: String,
        generation: String,
        leaseId: String,
    ): EditorV2CallResult<String> =
        normalize(editorV2CollaborationNackOutbound(editorId, generation, leaseId))

    override fun collaborationDetach(editorId: String): EditorV2Error? =
        editorV2CollaborationDetach(editorId).error?.toV2()

    override fun collaborationReattach(editorId: String): EditorV2Error? =
        editorV2CollaborationReattach(editorId).error?.toV2()

    override fun collaborationPeers(editorId: String): EditorV2CallResult<String> =
        normalize(editorV2CollaborationPeers(editorId))

    override fun collaborationSetAwarenessSelection(
        editorId: String,
        selectionJson: String,
    ): EditorV2CallResult<String> =
        normalize(editorV2CollaborationSetAwarenessSelection(editorId, selectionJson))

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


    override fun renderUpdate(
        editorId: String,
        mirrorAnchor: Int?,
        mirrorHead: Int?,
    ): EditorV2CallResult<String> {
        val anchor = mirrorAnchor?.let(::exactV2U32)
        val head = mirrorHead?.let(::exactV2U32)
        if ((mirrorAnchor != null && anchor == null) || (mirrorHead != null && head == null)) {
            return EditorV2CallResult.Err(contractError("render mirror is not an exact u32"))
        }
        return normalize(editorV2RenderUpdate(editorId, anchor, head))
    }

    override fun resolveScalarSelection(editorId: String, anchor: Int, head: Int): EditorV2CallResult<String> {
        val exactAnchor = exactV2U32(anchor)
        val exactHead = exactV2U32(head)
        if (exactAnchor == null || exactHead == null) {
            return EditorV2CallResult.Err(contractError("selection offset is not an exact u32"))
        }
        return normalize(editorV2ResolveScalarSelection(editorId, exactAnchor, exactHead))
    }

    override fun docToScalar(editorId: String, docPos: Int): EditorV2CallResult<String> {
        val exact = exactV2U32(docPos)
            ?: return EditorV2CallResult.Err(contractError("document position is not an exact u32"))
        return normalize(editorV2DocToScalar(editorId, exact))
    }

    override fun scalarToDoc(editorId: String, scalar: Int): EditorV2CallResult<String> {
        val exact = exactV2U32(scalar)
            ?: return EditorV2CallResult.Err(contractError("scalar position is not an exact u32"))
        return normalize(editorV2ScalarToDoc(editorId, exact))
    }
}
