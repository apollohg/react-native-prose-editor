package com.apollohg.editor

import org.json.JSONObject

/**
 * The v2 adapter.
 *
 * Owns one v2 editor session (decimal-string handle) and translates the
 * existing native view operations into typed v2 transactions/results. Every
 * mutation is one typed transaction against the tracked base document
 * revision; a `REVISION_MISMATCH` refreshes from Rust state and is NEVER
 * retried against guessed positions. Transient IME/composing state never
 * reaches the adapter — only final commits do.
 *
 * Render derivation: the v2 render accessor
 * ([EditorV2Backend.renderUpdate] / [EditorV2Backend.resolveScalarSelection]
 * / [EditorV2Backend.docToScalar] / [EditorV2Backend.scalarToDoc]) returns
 * everything the view needs — full render blocks, toolbar active state, the
 * mirrored scalar selection resolved to doc positions, and the lenient
 * doc↔scalar mapping (including the document's scalar extent) — derived
 * directly from the live v2 session.
 */
internal class EditorV2Adapter private constructor(
    private val backend: EditorV2Backend,
    val editorId: String,
    private val roomBound: Boolean,
) : EditorV2Driver {

    var onAutonomousError: ((EditorV2Error) -> Unit)? = null
    var outboundFrameSink: ((ByteArray) -> Unit)? = null
    var collaborationGeneration: String? = null

    var baseDocumentRevision: ULong = 0uL
        private set
    private var nextRequestId: ULong = 0uL
    internal var lastRequestIdForTesting: ULong? = null
        private set
    internal var backendEnvelopeCallCountForTesting = 0
        private set
    private var lastSyncedScalarSelection: IntArray? = null
    private var cachedScalarLength: Int? = null
    private var destroyed = false

    /** Diagnostics for handled paths that never surface as error events. */
    val debugNotes = mutableListOf<String>()

    companion object {
        /**
         * Attach to an existing v2 session created through the module's
         * JS-facing `editorV2Create` entry. The session is NOT re-created;
         * the adapter routes the bound view's interactions through the
         * shared session (the TS document handle and collaboration
         * controller drive the same session over the module surface).
         */
        fun attach(backend: EditorV2Backend, editorId: String, roomBound: Boolean): EditorV2Adapter? {
            if (!isCanonicalDecimalEditorId(editorId)) return null
            val state = backend.getState(editorId) as? EditorV2CallResult.Ok ?: return null
            val documentRevision = try {
                canonicalV2U64(JSONObject(state.value).opt("documentRevision") as? String)
                    ?.toULong()
            } catch (_: Exception) { null } ?: return null
            return EditorV2Adapter(backend, editorId, roomBound).also {
                it.baseDocumentRevision = documentRevision
            }
        }

        private fun isCanonicalDecimalEditorId(editorId: String): Boolean =
            editorId.isNotEmpty() &&
                editorId.all { it in '0'..'9' } &&
                (editorId == "0" || editorId.first() != '0') &&
                editorId.toULongOrNull() != null

        fun contractError(message: String): EditorV2Error =
            EditorV2Error(domain = "boundary", code = "FFI_RESULT_INVALID", message = message)

        private fun destroyedError(): EditorV2Error =
            EditorV2Error(domain = "lifecycle", code = "ENGINE_DESTROYED", message = "editor session is destroyed")
    }

    fun destroy(): EditorV2Error? {
        if (destroyed) return null
        destroyed = true
        val error = backend.destroy(editorId) ?: return null
        if (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING") return null
        return error
    }

    // ── Envelopes ──

    private fun requestIdExhaustedError(): EditorV2Error =
        EditorV2Error(
            domain = "boundary",
            code = "CONFIG_INVALID",
            message = "v2 request id counter exhausted",
            requestId = nextRequestId.toString(),
            limit = ULong.MAX_VALUE.toString(),
        )

    private fun buildEnvelope(
        payload: JSONObject,
        includeBaseRevision: Boolean = true,
    ): EditorV2CallResult<String> {
        if (nextRequestId == ULong.MAX_VALUE) {
            return EditorV2CallResult.Err(requestIdExhaustedError())
        }
        nextRequestId += 1u
        lastRequestIdForTesting = nextRequestId
        val parts = mutableListOf(
            "\"version\":1",
            "\"requestId\":${JSONObject.quote(nextRequestId.toString())}",
        )
        if (includeBaseRevision) {
            parts.add("\"baseDocumentRevision\":${JSONObject.quote(baseDocumentRevision.toString())}")
        }
        val payloadJson = payload.toString()
        if (payloadJson.length > 2) {
            parts.add(payloadJson.substring(1, payloadJson.length - 1))
        }
        return EditorV2CallResult.Ok(parts.joinToString(separator = ",", prefix = "{", postfix = "}"))
    }

    private fun callWithEnvelope(
        payload: JSONObject,
        includeBaseRevision: Boolean = true,
        call: (String) -> EditorV2CallResult<String>,
    ): EditorV2CallResult<String> =
        when (val envelope = buildEnvelope(payload, includeBaseRevision)) {
            is EditorV2CallResult.Err -> envelope
            is EditorV2CallResult.Ok -> {
                backendEnvelopeCallCountForTesting += 1
                call(envelope.value)
            }
        }

    internal fun setNextRequestIdForTesting(requestId: ULong) {
        nextRequestId = requestId
    }

    private fun positionEnvelope(scalar: Int, affinity: String? = null): JSONObject {
        val envelope = JSONObject().put("offset", scalar).put("kind", "scalar")
        if (affinity != null) envelope.put("affinity", affinity)
        return envelope
    }

    private fun selectionEnvelope(anchor: Int, head: Int, affinity: String): JSONObject =
        JSONObject().put(
            "selection",
            JSONObject()
                .put("type", "text")
                .put("anchor", positionEnvelope(anchor, affinity))
                .put("head", positionEnvelope(head, affinity)),
        )

    // ── Structured reads ──

    private data class V2State(
        val documentRevision: ULong,
        val canUndo: Boolean,
        val canRedo: Boolean,
    )

    private fun emit(error: EditorV2Error) {
        debugNotes.add("emit ${error.domain}/${error.code}: ${error.message}")
        onAutonomousError?.invoke(error)
    }

    private fun ulongField(object_: JSONObject, key: String): ULong? =
        canonicalV2U64(object_.opt(key) as? String)?.toULong()

    private fun scalarField(object_: JSONObject, key: String): Int? =
        exactV2ScalarInt(object_.opt(key) as? Number)

    private fun fetchState(): V2State? {
        return when (val result = backend.getState(editorId)) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> {
                try {
                    val json = JSONObject(result.value)
                    val revision = ulongField(json, "documentRevision")
                    if (revision == null) {
                        emit(contractError("v2 getState value violates the frozen shape"))
                        null
                    } else {
                        V2State(
                            documentRevision = revision,
                            canUndo = json.getBoolean("canUndo"),
                            canRedo = json.getBoolean("canRedo"),
                        )
                    }
                } catch (error: Exception) {
                    emit(contractError("v2 getState value violates the frozen shape"))
                    null
                }
            }
        }
    }

    private fun fetchDocumentJson(): String? {
        return when (val result = backend.getDocumentJson(editorId)) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> result.value
        }
    }

    // ── Render derivation (v2 render accessor) ──

    private fun refreshInternal(mirrorSelection: IntArray?): String? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        val state = fetchState() ?: return null
        baseDocumentRevision = state.documentRevision
        val derived = when (
            val result = backend.renderUpdate(
                editorId,
                mirrorSelection?.get(0),
                mirrorSelection?.get(1),
            )
        ) {
            is EditorV2CallResult.Err -> {
                debugNotes.add("renderUpdate ${result.error.domain}/${result.error.code}")
                return null
            }
            is EditorV2CallResult.Ok -> result.value
        }
        return try {
            val update = JSONObject(derived)
            cachedScalarLength = scalarField(update, "scalarLength")
                ?: return null
            // The scalar extent feeds the adapter's IME clamp only; the
            // view-facing update keeps the exact legacy update JSON shape.
            update.remove("scalarLength")
            // History and version are v2-engine facts, re-stamped from the
            // same getState read that drives revision tracking.
            update.put(
                "historyState",
                JSONObject().put("canUndo", state.canUndo).put("canRedo", state.canRedo),
            )
            update.put("documentVersion", state.documentRevision.toString())
            if (mirrorSelection == null) {
                // Native-originated edits keep the native caret.
                update.remove("selection")
            }
            update.toString()
        } catch (error: Exception) {
            debugNotes.add("renderUpdate parse failed")
            null
        }
    }

    override fun refreshFromRustState(mirrorSelection: IntArray?): String? =
        refreshInternal(mirrorSelection)

    override fun currentStateJson(): String? =
        refreshInternal(lastSyncedScalarSelection)

    override fun documentHtml(): String? {
        if (destroyed) return null
        return when (val result = backend.getDocumentHtml(editorId)) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> try {
                JSONObject(result.value).getString("html")
            } catch (error: Exception) {
                emit(contractError("v2 getDocumentHtml value violates the frozen shape"))
                null
            }
        }
    }

    override fun documentJson(): String? {
        if (destroyed) return null
        return fetchDocumentJson()
    }

    override fun contentSnapshotJson(): String? {
        if (destroyed) return null
        return when (val result = backend.getContentSnapshot(editorId)) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> result.value
        }
    }

    override fun historyCanUndo(): Boolean? = fetchState()?.canUndo
    override fun historyCanRedo(): Boolean? = fetchState()?.canRedo

    override fun selectionJson(): String? {
        val update = refreshInternal(lastSyncedScalarSelection) ?: return null
        return try {
            JSONObject(update).getJSONObject("selection").toString()
        } catch (error: Exception) {
            null
        }
    }

    // ── Selection sync and position mapping ──

    private sealed interface SelectionSyncOutcome {
        object Ok : SelectionSyncOutcome
        data class Refreshed(val updateJson: String) : SelectionSyncOutcome
        object Failed : SelectionSyncOutcome
    }

    private fun clampScalar(scalar: Int): Int {
        val extent = cachedScalarLength ?: return scalar
        return scalar.coerceIn(0, extent)
    }

    private fun ensureSelection(anchor: Int, head: Int): SelectionSyncOutcome {
        val clampedAnchor = clampScalar(anchor)
        val clampedHead = clampScalar(head)
        val last = lastSyncedScalarSelection
        if (last != null && last[0] == clampedAnchor && last[1] == clampedHead) {
            return SelectionSyncOutcome.Ok
        }
        // Affinity policy mirrors the engine's own cursor resolution: a
        // collapsed caret prefers After with a deterministic Before fallback
        // at text-boundary positions; a range uses Before. The fallback
        // changes only the stickiness of the SAME position.
        val collapsed = clampedAnchor == clampedHead
        var result = callWithEnvelope(
            selectionEnvelope(clampedAnchor, clampedHead, if (collapsed) "after" else "before"),
        ) { requestJson ->
            backend.setSelection(editorId, requestJson)
        }
        if (collapsed &&
            result is EditorV2CallResult.Err &&
            result.error.code == "POSITION_INVALID"
        ) {
            result = callWithEnvelope(selectionEnvelope(clampedAnchor, clampedHead, "before")) { requestJson ->
                backend.setSelection(editorId, requestJson)
            }
        }
        return when (result) {
            is EditorV2CallResult.Ok -> {
                parseMutationOutcome(result.value)?.let { outcome ->
                    if (outcome is MutationOutcome.Transaction) {
                        baseDocumentRevision = outcome.revision
                    }
                }
                lastSyncedScalarSelection = intArrayOf(clampedAnchor, clampedHead)
                SelectionSyncOutcome.Ok
            }
            is EditorV2CallResult.Err -> {
                if (result.error.code == "REVISION_MISMATCH") {
                    val update = refreshInternal(intArrayOf(clampedAnchor, clampedHead))
                    if (update != null) {
                        SelectionSyncOutcome.Refreshed(update)
                    } else {
                        SelectionSyncOutcome.Failed
                    }
                } else {
                    emit(result.error)
                    SelectionSyncOutcome.Failed
                }
            }
        }
    }

    override fun syncSelection(anchor: Int, head: Int): IntArray? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        when (val outcome = ensureSelection(anchor, head)) {
            is SelectionSyncOutcome.Ok -> Unit
            is SelectionSyncOutcome.Refreshed -> {
                return try {
                    val selection = JSONObject(outcome.updateJson).getJSONObject("selection")
                    intArrayOf(
                        scalarField(selection, "anchor") ?: return null,
                        scalarField(selection, "head") ?: return null,
                    )
                } catch (error: Exception) {
                    null
                }
            }
            is SelectionSyncOutcome.Failed -> return null
        }
        // Engine-authoritative scalar→doc selection mapping for the delegate
        // callback's doc positions (v2 accessor).
        val resolved = when (val result = backend.resolveScalarSelection(editorId, anchor, head)) {
            is EditorV2CallResult.Err -> {
                debugNotes.add("resolveScalarSelection ${result.error.domain}/${result.error.code}")
                return null
            }
            is EditorV2CallResult.Ok -> result.value
        }
        return try {
            val selection = JSONObject(resolved)
            intArrayOf(
                scalarField(selection, "anchor") ?: return null,
                scalarField(selection, "head") ?: return null,
            )
        } catch (error: Exception) {
            null
        }
    }

    override fun syncSelectionQuiet(anchor: Int, head: Int) {
        if (destroyed) return
        ensureSelection(anchor, head)
    }

    override fun scalarPositionForDoc(docPos: Int): Int? =
        mapPosition(backend.docToScalar(editorId, docPos), "scalar")

    override fun docPositionForScalar(scalar: Int): Int? =
        mapPosition(backend.scalarToDoc(editorId, scalar), "doc")

    private fun mapPosition(result: EditorV2CallResult<String>, key: String): Int? =
        when (result) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> try {
                scalarField(JSONObject(result.value), key)
            } catch (error: Exception) { null }
        }

    // ── Mutation driver ──

    private sealed interface MutationOutcome {
        data class Transaction(val changed: Boolean, val revision: ULong) : MutationOutcome
        object NotApplicable : MutationOutcome
        data class Replacement(val changed: Boolean, val revision: ULong) : MutationOutcome
    }

    private fun parseMutationOutcome(json: String): MutationOutcome? {
        return try {
            val outcome = JSONObject(json)
            when (outcome.getString("type")) {
                "transaction" -> MutationOutcome.Transaction(
                    changed = outcome.getBoolean("changed"),
                    revision = ulongField(outcome, "documentRevision") ?: return null,
                )
                "notApplicable" -> MutationOutcome.NotApplicable
                "replacement" -> MutationOutcome.Replacement(
                    changed = outcome.getBoolean("changed"),
                    revision = ulongField(outcome, "documentRevision") ?: return null,
                )
                else -> null
            }
        } catch (error: Exception) {
            null
        }
    }

    private fun handleMutationError(error: EditorV2Error, mirror: IntArray?): String? {
        if (error.code == "REVISION_MISMATCH") {
            // Refresh from Rust state; NEVER retry against guessed positions.
            val update = refreshInternal(mirror)
            debugNotes.add("mismatch-refresh ${if (update == null) "nil" else "ok"}")
            return update
        }
        emit(error)
        return null
    }

    private fun performMutation(
        preSelection: IntArray? = null,
        postSelectionMirror: IntArray? = null,
        includeSelectionInUpdate: Boolean = false,
        call: () -> EditorV2CallResult<String>,
    ): String? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        if (preSelection != null) {
            when (val sync = ensureSelection(preSelection[0], preSelection[1])) {
                is SelectionSyncOutcome.Ok -> Unit
                is SelectionSyncOutcome.Refreshed -> return sync.updateJson
                is SelectionSyncOutcome.Failed -> return null
            }
        }
        return when (val result = call()) {
            is EditorV2CallResult.Err -> handleMutationError(result.error, postSelectionMirror ?: preSelection)
            is EditorV2CallResult.Ok -> {
                val outcome = parseMutationOutcome(result.value)
                if (outcome == null) {
                    emit(contractError("v2 mutation outcome violates the frozen shape"))
                    return null
                }
                when (outcome) {
                    is MutationOutcome.Transaction -> {
                        baseDocumentRevision = outcome.revision
                        if (postSelectionMirror != null) {
                            lastSyncedScalarSelection = postSelectionMirror
                        }
                    }
                    is MutationOutcome.NotApplicable -> {
                        return refreshInternal(postSelectionMirror ?: preSelection)
                    }
                    is MutationOutcome.Replacement -> {
                        baseDocumentRevision = outcome.revision
                        // Whole-root replacement resets the engine-side selection.
                        lastSyncedScalarSelection = null
                    }
                }
                val mirror = if (includeSelectionInUpdate) postSelectionMirror ?: preSelection else null
                val update = refreshInternal(mirror) ?: return null
                drainOutboundIfNeeded()
                update
            }
        }
    }

    private fun performHistoryMutation(call: (String) -> EditorV2CallResult<String>): String? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        return when (val result = callWithEnvelope(JSONObject(), includeBaseRevision = false, call = call)) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> {
                val changed = try {
                    JSONObject(result.value).getBoolean("changed")
                } catch (error: Exception) {
                    emit(contractError("v2 history outcome violates the frozen shape"))
                    return null
                }
                val update = refreshInternal(null) ?: return null
                if (changed) {
                    drainOutboundIfNeeded()
                }
                update
            }
        }
    }

    // ── Drain ping (mirrors the TS controller's onLocalDocumentCommit) ──

    override fun driveCollaborationDrainPing() {
        drainOutboundIfNeeded()
    }

    private fun drainOutboundIfNeeded() {
        val generation = collaborationGeneration ?: return
        val sink = outboundFrameSink ?: return
        if (!roomBound) return
        while (true) {
            when (val result = backend.collaborationTakeOutbound(editorId, generation)) {
                is EditorV2CallResult.Err -> {
                    emit(result.error)
                    return
                }
                is EditorV2CallResult.Ok -> {
                    if (result.value.isEmpty()) return
                    sink(result.value)
                }
            }
        }
    }

    // ── Typed verbs ──

    override fun insertText(text: String, atScalarPos: Int): String? {
        if (text.isEmpty()) return currentStateJson()
        val postCaret = atScalarPos + text.codePointCount(0, text.length)
        return performMutation(
            preSelection = intArrayOf(atScalarPos, atScalarPos),
            postSelectionMirror = intArrayOf(postCaret, postCaret),
        ) {
            callWithEnvelope(JSONObject().put("text", text)) { requestJson ->
                backend.applyInput(editorId, requestJson)
            }
        }
    }

    override fun replaceTextRange(scalarFrom: Int, scalarTo: Int, text: String): String? {
        if (text.isEmpty()) {
            return deleteScalarRange(scalarFrom, scalarTo)
        }
        val postCaret = scalarFrom + text.codePointCount(0, text.length)
        // A range-replacing commit (autocorrect, paste-over-selection, IME
        // commit over a composing range) is ONE typed ReplaceSelectionText
        // transaction: the planner's InsertText is collapsed-only.
        return performMutation(
            preSelection = intArrayOf(scalarFrom, scalarTo),
            postSelectionMirror = intArrayOf(postCaret, postCaret),
        ) {
            callWithEnvelope(
                JSONObject().put(
                    "command",
                    JSONObject().put("type", "replaceSelectionText").put("text", text),
                ),
            ) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }
    }

    override fun deleteScalarRange(scalarFrom: Int, scalarTo: Int): String? {
        val clampedFrom = clampScalar(scalarFrom)
        val clampedTo = clampScalar(scalarTo)
        if (clampedFrom >= clampedTo) return currentStateJson()
        return performMutation(postSelectionMirror = intArrayOf(clampedFrom, clampedFrom)) {
            callWithEnvelope(
                JSONObject().put(
                    "command",
                    JSONObject()
                        .put("type", "deleteRange")
                        .put(
                            "range",
                            JSONObject()
                                .put("from", positionEnvelope(clampedFrom))
                                .put("to", positionEnvelope(clampedTo)),
                        ),
                ),
            ) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }
    }

    override fun deleteBackwardAtSelection(anchor: Int, head: Int): String? {
        val postCaret = if (anchor == head) (anchor - 1).coerceAtLeast(0) else minOf(anchor, head)
        return performMutation(
            preSelection = intArrayOf(anchor, head),
            postSelectionMirror = intArrayOf(postCaret, postCaret),
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "deleteBackward"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }
    }

    override fun splitBlockAt(scalarPos: Int): String? =
        performMutation(
            preSelection = intArrayOf(scalarPos, scalarPos),
            postSelectionMirror = intArrayOf(scalarPos, scalarPos),
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "splitBlock"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }

    override fun deleteAndSplit(scalarFrom: Int, scalarTo: Int): String? =
        performMutation(
            preSelection = intArrayOf(scalarFrom, scalarTo),
            postSelectionMirror = intArrayOf(scalarFrom, scalarFrom),
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "deleteAndSplit"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }

    override fun insertNode(nodeType: String, anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "insertNode").put("nodeType", nodeType), anchor, head)

    override fun insertContentHtmlAtSelection(html: String, anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "insertContentHtml").put("html", html), anchor, head)

    override fun insertContentJsonAtSelection(json: String, anchor: Int, head: Int): String? {
        val fragment = try {
            JSONObject(json)
        } catch (error: Exception) {
            emit(contractError("insertContentJson fragment is not valid JSON"))
            return null
        }
        return commandAtSelection(JSONObject().put("type", "insertContentJson").put("json", fragment), anchor, head)
    }

    override fun toggleMark(markName: String, anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "toggleMark").put("markType", markName), anchor, head)

    override fun setMark(markName: String, attrsJson: String, anchor: Int, head: Int): String? {
        val attrs = try {
            JSONObject(attrsJson)
        } catch (error: Exception) {
            emit(contractError("setMark attrs are not valid JSON"))
            return null
        }
        return commandAtSelection(
            JSONObject().put("type", "setMark").put("markType", markName).put("attrs", attrs),
            anchor,
            head,
        )
    }

    override fun unsetMark(markName: String, anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "unsetMark").put("markType", markName), anchor, head)

    override fun toggleHeading(level: Int, anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "toggleHeading").put("level", level), anchor, head)

    override fun toggleCodeBlock(anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "toggleCodeBlock"), anchor, head)

    override fun toggleBlockquote(anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "toggleBlockquote"), anchor, head)

    override fun wrapInList(listType: String, anchor: Int, head: Int): String? {
        val itemType = if (listType == "taskList") "taskItem" else "listItem"
        return commandAtSelection(
            JSONObject().put("type", "wrapInList").put("listType", listType).put("itemType", itemType),
            anchor,
            head,
        )
    }

    override fun unwrapFromList(anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "unwrapFromList"), anchor, head)

    override fun indentListItem(anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "indentListItem"), anchor, head)

    override fun outdentListItem(anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "outdentListItem"), anchor, head)

    override fun toggleTaskItemCheckedAtSelection(anchor: Int, head: Int): String? =
        commandAtSelection(JSONObject().put("type", "toggleTaskItemChecked"), anchor, head)

    private fun commandAtSelection(command: JSONObject, anchor: Int, head: Int): String? =
        performMutation(
            preSelection = intArrayOf(anchor, head),
            postSelectionMirror = intArrayOf(anchor, head),
        ) {
            callWithEnvelope(JSONObject().put("command", command)) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }

    override fun resizeImageAtDocPos(docPos: Int, width: Int, height: Int): String? {
        val scalar = scalarPositionForDoc(docPos) ?: return null
        return performMutation {
            callWithEnvelope(
                JSONObject().put(
                    "command",
                    JSONObject()
                        .put("type", "resizeImage")
                        .put("at", positionEnvelope(scalar))
                        .put("width", width)
                        .put("height", height),
                ),
            ) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }
    }

    override fun undo(): String? =
        performHistoryMutation { requestJson -> backend.undo(editorId, requestJson) }

    override fun redo(): String? =
        performHistoryMutation { requestJson -> backend.redo(editorId, requestJson) }

    // ── Controlled content ──

    override fun setContentHtml(html: String): String? =
        performMutation(postSelectionMirror = intArrayOf(0, 0), includeSelectionInUpdate = true) {
            callWithEnvelope(JSONObject().put("setHtml", html).put("history", "resetAndClear")) { requestJson ->
                backend.applyLocalApi(editorId, requestJson)
            }
        }

    override fun setContentJson(json: String): String? {
        val document = try {
            JSONObject(json)
        } catch (error: Exception) {
            emit(contractError("setContentJson document is not valid JSON"))
            return null
        }
        return performMutation(postSelectionMirror = intArrayOf(0, 0), includeSelectionInUpdate = true) {
            callWithEnvelope(JSONObject().put("setJson", document).put("history", "resetAndClear")) { requestJson ->
                backend.applyLocalApi(editorId, requestJson)
            }
        }
    }
}
