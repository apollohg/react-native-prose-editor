package com.apollohg.editor


import org.json.JSONObject
import org.json.JSONArray

/**
 * The v2 adapter.
 *
 * Owns one v2 editor session (decimal-string handle) and translates the
 * existing native view operations into typed v2 transactions/results. Every
 * mutation is one typed transaction against the tracked base document
 * revision. Transient IME/composing state never reaches the adapter — only
 * final commits do.
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
    internal val backend: EditorV2Backend,
    val editorId: String,
    internal val roomBound: Boolean,
    internal val collaborationWake: (String, CollaborationWakeReason) -> Unit,
) : EditorV2Driver {

    var onAutonomousError: ((EditorV2Error) -> Unit)? = null

    private data class AutonomousErrorOwner(
        val token: Long,
        val callback: (EditorV2Error) -> Unit,
        val onReleased: () -> Unit,
    )

    /** One live native-view binding exclusively owns autonomous errors. */
    private var autonomousErrorOwner: AutonomousErrorOwner? = null

    var baseDocumentRevision: ULong = 0uL
        internal set
    /** Paired with [baseDocumentRevision] by the same locked render read. */
    var stateRevision: ULong = 0uL
        internal set
    private var nextRequestId: ULong = 0uL
    internal var nativeOwnerId: String? = null
    private var nativeOwnerToken: Long? = null
    internal var positionEpoch: String? = null
    internal var lastRequestIdForTesting: ULong? = null
        private set
    internal var backendEnvelopeCallCountForTesting = 0
        private set
    internal var lastSyncedScalarSelection: IntArray? = null
    internal var cachedAuthoritativeScalarSelection: IntArray? = null
    internal var cachedScalarLength: Int? = null
    internal var cachedActiveState: JSONObject? = null
    internal var cachedHistoryState: JSONObject? = null
    internal var cachedViewUpdateJson: String? = null
    internal var cachedAtomicRenderJson: String? = null
    internal var cachedAtomicRenderDocumentRevision: ULong? = null
    internal var renderUpdateCallCountForTesting = 0
        internal set
    internal var destroyed = false

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
        fun attach(
            backend: EditorV2Backend,
            editorId: String,
            roomBound: Boolean,
            collaborationWake: (String, CollaborationWakeReason) -> Unit = { id, reason ->
                NativeCollaborationTransportRegistry.notifyOutboundAvailable(id, reason)
            },
        ): EditorV2Adapter? {
            if (!isCanonicalDecimalEditorId(editorId)) return null
            // Attachment establishes only that the handle is live. The first
            // render adopts the revision, scalar extent, selection, active,
            // and history caches as one locked snapshot before input resumes.
            if (backend.getState(editorId) !is EditorV2CallResult.Ok) return null
            return EditorV2Adapter(backend, editorId, roomBound, collaborationWake)
        }

        private fun isCanonicalDecimalEditorId(editorId: String): Boolean =
            editorId.isNotEmpty() &&
                editorId.all { it in '0'..'9' } &&
                (editorId == "0" || editorId.first() != '0') &&
                editorId.toULongOrNull() != null

        fun contractError(message: String): EditorV2Error =
            EditorV2Error(domain = "boundary", code = "FFI_RESULT_INVALID", message = message)

        internal fun destroyedError(): EditorV2Error =
            EditorV2Error(domain = "lifecycle", code = "ENGINE_DESTROYED", message = "editor session is destroyed")
    }

    fun destroy(): EditorV2Error? {
        if (destroyed) return null
        nativeOwnerId?.let { backend.releaseNativeBinding(editorId, it) }
        nativeOwnerId = null
        nativeOwnerToken = null
        positionEpoch = null
        destroyed = true
        val error = backend.destroy(editorId) ?: return null
        if (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING") return null
        return error
    }


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

    internal fun callWithEnvelope(
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

    internal fun selectionEnvelope(anchor: Int, head: Int, affinity: String): JSONObject =
        JSONObject().put(
            "selection",
            JSONObject()
                .put("type", "text")
                .put("anchor", positionEnvelope(anchor, affinity))
                .put("head", positionEnvelope(head, affinity)),
        )


    internal fun bindAutonomousErrorOwner(
        token: Long,
        callback: (EditorV2Error) -> Unit,
        onReleased: () -> Unit,
    ) {
        val displaced = synchronized(this) {
            autonomousErrorOwner.also {
                autonomousErrorOwner = AutonomousErrorOwner(token, callback, onReleased)
            }
        }
        claimNativeBinding(token, replaceExisting = true)
        displaced?.onReleased?.invoke()
    }

    internal fun claimNativeBindingIfUnowned(token: Long) {
        claimNativeBinding(token, replaceExisting = false)
    }

    internal fun isNativeBindingOwner(token: Long): Boolean = synchronized(this) {
        nativeOwnerToken == token
    }

    private fun claimNativeBinding(token: Long, replaceExisting: Boolean) {
        val releasedOwner = synchronized(this) {
            if (!replaceExisting && nativeOwnerToken != null) return
            if (nativeOwnerToken == token) return
            nativeOwnerId.also {
                nativeOwnerToken = token
                nativeOwnerId = token.toString()
                positionEpoch = null
            }
        }
        releasedOwner?.let { backend.releaseNativeBinding(editorId, it) }
    }

    internal fun releaseNativeBindingOwner(token: Long) {
        val releasedOwner = synchronized(this) {
            if (nativeOwnerToken != token) return
            nativeOwnerToken = null
            positionEpoch = null
            nativeOwnerId.also { nativeOwnerId = null }
        }
        releasedOwner?.let { backend.releaseNativeBinding(editorId, it) }
    }

    /** A stale view may clear only its own binding generation. */
    internal fun clearAutonomousErrorOwner(token: Long) {
        val released = synchronized(this) {
            if (autonomousErrorOwner?.token == token) {
                autonomousErrorOwner = null
                true
            } else {
                false
            }
        }
        if (released) releaseNativeBindingOwner(token)
    }

    /** Pair release invalidates the final owner even when no view remains registered. */
    internal fun releaseAutonomousErrorOwner() {
        val released = synchronized(this) {
            autonomousErrorOwner.also {
                autonomousErrorOwner = null
            }
        }
        released?.let { releaseNativeBindingOwner(it.token) }
        released?.onReleased?.invoke()
    }

    internal fun ownsAutonomousErrorOwner(token: Long): Boolean = synchronized(this) {
        autonomousErrorOwner?.token == token
    }

    internal fun emit(error: EditorV2Error) {
        debugNotes.add("emit ${error.domain}/${error.code}: ${error.message}")
        val callback = synchronized(this) {
            autonomousErrorOwner?.callback ?: onAutonomousError
        }
        callback?.invoke(error)
    }

    internal fun atomicRenderJson(matchingDocumentRevision: String): String? {
        val revision = matchingDocumentRevision.toULongOrNull() ?: return null
        if (cachedAtomicRenderDocumentRevision != revision) return null
        return cachedAtomicRenderJson
    }

    @Volatile
    internal var latestJSDrivenDocumentRevision: ULong = 0uL

    private fun parseExternalReset(resetJson: String): JSONObject? {
        val reset = try { JSONObject(resetJson) } catch (_: Exception) { null }
        if (reset == null || reset.optString("history") != "resetAndClear" ||
            reset.keys().asSequence().toSet() != setOf("history", "documentRevision", if (reset.has("setJson")) "setJson" else "setHtml") ||
            (reset.opt("setJson") !is JSONObject && reset.opt("setHtml") !is String) ||
            canonicalV2U64(reset.opt("documentRevision") as? String) == null
        ) {
            emit(contractError("external reset intent is malformed"))
            return null
        }
        return reset
    }

    internal fun validateExternalReset(resetJson: String): Boolean = parseExternalReset(resetJson) != null

    @Synchronized
    internal fun adoptExternalReset(renderJson: String, resetJson: String): String? {
        val reset = parseExternalReset(resetJson) ?: return null
        val current = refreshFromRustState(null) ?: return null
        if (parseAtomicRenderSnapshot(current)?.documentRevision == reset.getString("documentRevision").toULong()) {
            return adoptExternalRender(renderJson)
        }
        if (latestJSDrivenDocumentRevision > reset.getString("documentRevision").toULong()) return current
        when (val result = backend.getState(editorId)) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                return null
            }
            is EditorV2CallResult.Ok -> {
                val origin = try { JSONObject(result.value).getString("documentOrigin") } catch (_: Exception) { null }
                if (origin == null) {
                    emit(contractError("v2 reset state violates the frozen shape"))
                    return null
                }
                if (origin != "nativeView") return refreshFromRustState(null)
            }
        }
        reset.remove("documentRevision")
        return when (val result = callWithEnvelope(reset, includeBaseRevision = false) { requestJson ->
            backend.replaceDocument(editorId, requestJson)
        }) {
            is EditorV2CallResult.Err -> {
                emit(result.error)
                null
            }
            is EditorV2CallResult.Ok -> {
                val commit = try { JSONObject(result.value) } catch (_: Exception) { null }
                if (commit?.opt("changed") !is Boolean ||
                    (commit.opt("documentRevision") as? String)?.toULongOrNull() == null
                ) {
                    emit(contractError("v2 reset result violates the frozen shape"))
                    return null
                }
                val update = refreshFromRustState(null) ?: return null
                if (commit.getBoolean("changed")) {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                update
            }
        }
    }

    internal fun adoptExternalRender(renderJson: String): String? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        val snapshot = parseAtomicRenderSnapshot(renderJson)
        if (snapshot == null) {
            emit(contractError("v2 atomic render snapshot violates the frozen shape"))
            return null
        }
        val resolvedPositionEpoch = when {
            snapshot.positionEpoch != null -> snapshot.positionEpoch
            nativeOwnerId == null -> positionEpoch
            else -> pinPositionEpochCandidate(snapshot.documentRevision) ?: return null
        }
        val pinned = PinnedAtomicRenderSnapshot(snapshot, resolvedPositionEpoch)
        return adopt(
            pinned.snapshot,
            stripViewSelection = false,
            engineOwnedSelection = true,
            resolvedPositionEpoch = pinned.positionEpoch,
        )
    }

    internal fun validateExternalRender(renderJson: String): Boolean {
        if (destroyed) {
            emit(destroyedError())
            return false
        }
        if (parseAtomicRenderSnapshot(renderJson) != null) return true
        emit(contractError("v2 atomic render snapshot violates the frozen shape"))
        return false
    }

    override fun refreshFromRustState(mirrorSelection: IntArray?): String? =
        // This result crosses the controlled-prop boundary, which admits only
        // complete frozen atomic render snapshots. Its parse/adopt side effect
        // updates adapter caches, but its public result must retain every
        // validated wire field, including selection and scalarLength.
        refreshInternal(
            mirrorSelection,
            stripViewSelection = false,
            controlledPropSnapshot = true
        )

    internal fun recoverNativeRender(): String? {
        val ownerId = synchronized(this) {
            positionEpoch = null
            nativeOwnerId
        }
        ownerId?.let { backend.releaseNativeBinding(editorId, it) }
        return refreshInternal(null, stripViewSelection = false)
    }

    override fun currentStateJson(): String? =
        refreshInternal(cachedAuthoritativeScalarSelection?.copyOf())

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

    override fun historyCanUndo(): Boolean? =
        cachedHistoryState?.let { exactBool(it.opt("canUndo")) }

    override fun historyCanRedo(): Boolean? =
        cachedHistoryState?.let { exactBool(it.opt("canRedo")) }

    override fun selectionJson(): String? {
        val update = refreshInternal(cachedAuthoritativeScalarSelection?.copyOf()) ?: return null
        return try {
            JSONObject(update).getJSONObject("selection").toString()
        } catch (error: Exception) {
            null
        }
    }


    override fun syncSelection(anchor: Int, head: Int): EditorV2SelectionSync? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        if (nativeOwnerId != null) {
            val previousDocumentRevision = baseDocumentRevision
            val update = performNativeIntent(nativeIntent("setSelection", anchor, head)).updateJsonOrNull()
                ?: return null
            val mapping = textDocumentSelection(update) ?: return null
            publishCollaborationSelection(mapping[0], mapping[1])
            return EditorV2SelectionSync(
                mapping[0],
                mapping[1],
                update.takeIf { baseDocumentRevision != previousDocumentRevision },
            )
        }
        var refreshedUpdateJson: String? = null
        val mapping = when (val outcome = ensureSelection(anchor, head)) {
            is SelectionSyncOutcome.Ok -> resolveSelectionMapping(anchor, head)
            is SelectionSyncOutcome.Refreshed -> {
                refreshedUpdateJson = outcome.updateJson
                textDocumentSelection(outcome.updateJson)
            }
            is SelectionSyncOutcome.Failed -> return null
        }
        if (mapping == null) return null
        publishCollaborationSelection(mapping[0], mapping[1])
        return EditorV2SelectionSync(mapping[0], mapping[1], refreshedUpdateJson)
    }

    override fun syncSelectionQuiet(anchor: Int, head: Int): String? {
        if (destroyed) return null
        if (nativeOwnerId != null) {
            val previousDocumentRevision = baseDocumentRevision
            val update = performNativeIntent(nativeIntent("setSelection", anchor, head)).updateJsonOrNull()
                ?: return null
            val mapping = textDocumentSelection(update) ?: return update
            publishCollaborationSelection(mapping[0], mapping[1])
            return update.takeIf { baseDocumentRevision != previousDocumentRevision }
        }
        var refreshedUpdateJson: String? = null
        val mapping = when (val outcome = ensureSelection(anchor, head)) {
            is SelectionSyncOutcome.Ok -> {
                if (!roomBound) return null
                val selection = lastSyncedScalarSelection ?: return null
                resolveSelectionMapping(selection[0], selection[1])
            }
            is SelectionSyncOutcome.Refreshed -> {
                refreshedUpdateJson = outcome.updateJson
                textDocumentSelection(outcome.updateJson)
            }
            is SelectionSyncOutcome.Failed -> return null
        } ?: return refreshedUpdateJson
        publishCollaborationSelection(mapping[0], mapping[1])
        return refreshedUpdateJson
    }

    override fun scalarPositionForDoc(docPos: Int): Int? =
        mapPosition(backend.docToScalar(editorId, docPos), "scalar")

    override fun docPositionForScalar(scalar: Int): Int? =
        mapPosition(backend.scalarToDoc(editorId, scalar), "doc")

    override fun insertText(text: String, atScalarPos: Int): String? {
        if (text.isEmpty()) return currentStateJson()
        if (nativeOwnerId != null) {
            return performNativeIntent(
                nativeIntent("insertText", atScalarPos, atScalarPos).put("text", text)
            ).updateJsonOrNull()
        }
        val postCaret = atScalarPos + text.codePointCount(0, text.length)
        return performMutation(
            preSelection = intArrayOf(atScalarPos, atScalarPos),
            postSelectionMirror = intArrayOf(postCaret, postCaret),
            includeSelectionInUpdate = true,
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
        if (nativeOwnerId != null) {
            return performNativeIntent(
                nativeIntent("replaceSelectionText", scalarFrom, scalarTo).put("text", text)
            ).updateJsonOrNull()
        }
        val postCaret = scalarFrom + text.codePointCount(0, text.length)
        // A range-replacing commit (autocorrect, paste-over-selection, IME
        // commit over a composing range) is ONE typed ReplaceSelectionText
        // transaction: the planner's InsertText is collapsed-only.
        return performMutation(
            preSelection = intArrayOf(scalarFrom, scalarTo),
            postSelectionMirror = intArrayOf(postCaret, postCaret),
            includeSelectionInUpdate = true,
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

    override fun replaceTextRangeNative(
        scalarFrom: Int,
        scalarTo: Int,
        text: String,
    ): EditorV2NativeIntentResult {
        if (nativeOwnerId == null) return EditorV2NativeIntentResult.Rejected
        if (text.isEmpty()) {
            val clampedFrom = clampScalar(scalarFrom)
            val clampedTo = clampScalar(scalarTo)
            if (clampedFrom >= clampedTo) {
                return refreshUnchangedNativeOutcome(
                    performNativeIntent(
                        nativeIntent("setSelection", clampedFrom, clampedFrom),
                        reportPositionEpochInvalid = true,
                    )
                )
            }
            return refreshUnchangedNativeOutcome(
                performNativeIntent(
                    nativeIntent("deleteRange", clampedFrom, clampedTo),
                    reportPositionEpochInvalid = true,
                )
            )
        }
        return refreshUnchangedNativeOutcome(
            performNativeIntent(
                nativeIntent("replaceSelectionText", scalarFrom, scalarTo).put("text", text),
                reportPositionEpochInvalid = true,
            )
        )
    }

    override fun deleteScalarRangeNative(
        scalarFrom: Int,
        scalarTo: Int,
    ): EditorV2NativeIntentResult {
        if (nativeOwnerId == null) return EditorV2NativeIntentResult.Rejected
        val clampedFrom = clampScalar(scalarFrom)
        val clampedTo = clampScalar(scalarTo)
        val intent = if (clampedFrom >= clampedTo) {
            nativeIntent("setSelection", clampedFrom, clampedFrom)
        } else {
            nativeIntent("deleteRange", clampedFrom, clampedTo)
        }
        return refreshUnchangedNativeOutcome(
            performNativeIntent(intent, reportPositionEpochInvalid = true),
        )
    }

    override fun deleteScalarRange(scalarFrom: Int, scalarTo: Int): String? {
        val clampedFrom = clampScalar(scalarFrom)
        val clampedTo = clampScalar(scalarTo)
        if (clampedFrom >= clampedTo) return currentStateJson()
        if (nativeOwnerId != null) {
            return deleteScalarRangeNative(clampedFrom, clampedTo).updateJsonOrNull()
        }
        return performMutation(
            postSelectionMirror = intArrayOf(clampedFrom, clampedFrom),
            includeSelectionInUpdate = true,
        ) {
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
        if (nativeOwnerId != null) {
            return performNativeIntent(nativeIntent("deleteBackward", anchor, head)).updateJsonOrNull()
        }
        val postCaret = if (anchor == head) (anchor - 1).coerceAtLeast(0) else minOf(anchor, head)
        return performMutation(
            preSelection = intArrayOf(anchor, head),
            postSelectionMirror = intArrayOf(postCaret, postCaret),
            includeSelectionInUpdate = true,
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "deleteBackward"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }
    }

    override fun splitBlockAt(scalarPos: Int): EditorV2SplitRender? =
        if (nativeOwnerId != null) {
            when (val outcome = performNativeIntent(nativeIntent("splitBlock", scalarPos, scalarPos))) {
                is EditorV2NativeIntentResult.Applied ->
                    EditorV2SplitRender(outcome.render.updateJson, outcome.render.changed)
                is EditorV2NativeIntentResult.Recovered ->
                    EditorV2SplitRender(outcome.updateJson, false)
                EditorV2NativeIntentResult.Rejected -> null
            }
        } else performSplitMutation(
            preSelection = intArrayOf(scalarPos, scalarPos),
            postSelectionMirror = intArrayOf(scalarPos + 1, scalarPos + 1),
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "splitBlock"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }

    override fun deleteAndSplit(scalarFrom: Int, scalarTo: Int): EditorV2SplitRender? =
        if (nativeOwnerId != null) {
            when (val outcome = performNativeIntent(nativeIntent("deleteAndSplit", scalarFrom, scalarTo))) {
                is EditorV2NativeIntentResult.Applied ->
                    EditorV2SplitRender(outcome.render.updateJson, outcome.render.changed)
                is EditorV2NativeIntentResult.Recovered ->
                    EditorV2SplitRender(outcome.updateJson, false)
                EditorV2NativeIntentResult.Rejected -> null
            }
        } else performSplitMutation(
            preSelection = intArrayOf(scalarFrom, scalarTo),
            postSelectionMirror = intArrayOf(scalarFrom + 1, scalarFrom + 1),
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "deleteAndSplit"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }

    override fun moveSelection(anchor: Int, head: Int, destination: Int): String? {
        val from = minOf(anchor, head)
        val to = maxOf(anchor, head)
        return commandAtSelection(
            JSONObject()
                .put("type", "moveSelection")
                .put(
                    "range",
                    JSONObject()
                        .put("from", positionEnvelope(from))
                        .put("to", positionEnvelope(to)),
                )
                .put("at", positionEnvelope(destination)),
            anchor,
            head,
        )
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
        val itemType = EditorNodeTypes.listItemType(listType)
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

    override fun resizeImageAtDocPos(docPos: Int, width: Int, height: Int): String? {
        val scalar = scalarPositionForDoc(docPos) ?: return null
        return performMutation(adoptEngineSelection = true) {
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
