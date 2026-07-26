package com.apollohg.editor

import org.json.JSONObject
import org.json.JSONArray

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
    private val collaborationWake: (String, CollaborationWakeReason) -> Unit,
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
        private set
    /** Paired with [baseDocumentRevision] by the same locked render read. */
    var stateRevision: ULong = 0uL
        private set
    private var nextRequestId: ULong = 0uL
    internal var lastRequestIdForTesting: ULong? = null
        private set
    internal var backendEnvelopeCallCountForTesting = 0
        private set
    private var lastSyncedScalarSelection: IntArray? = null
    private var cachedAuthoritativeScalarSelection: IntArray? = null
    private var cachedScalarLength: Int? = null
    private var cachedActiveState: JSONObject? = null
    private var cachedHistoryState: JSONObject? = null
    private var cachedViewUpdateJson: String? = null
    internal var renderUpdateCallCountForTesting = 0
        private set
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
        displaced?.onReleased?.invoke()
    }

    /** A stale view may clear only its own binding generation. */
    internal fun clearAutonomousErrorOwner(token: Long) {
        synchronized(this) {
            if (autonomousErrorOwner?.token == token) {
                autonomousErrorOwner = null
            }
        }
    }

    /** Pair release invalidates the final owner even when no view remains registered. */
    internal fun releaseAutonomousErrorOwner() {
        val released = synchronized(this) {
            autonomousErrorOwner.also { autonomousErrorOwner = null }
        }
        released?.onReleased?.invoke()
    }

    internal fun ownsAutonomousErrorOwner(token: Long): Boolean = synchronized(this) {
        autonomousErrorOwner?.token == token
    }

    private fun emit(error: EditorV2Error) {
        debugNotes.add("emit ${error.domain}/${error.code}: ${error.message}")
        val callback = synchronized(this) {
            autonomousErrorOwner?.callback ?: onAutonomousError
        }
        callback?.invoke(error)
    }

    private fun ulongField(object_: JSONObject, key: String): ULong? =
        canonicalV2U64(object_.opt(key) as? String)?.toULong()

    private fun scalarField(object_: JSONObject, key: String): Int? =
        exactV2ScalarInt(object_.opt(key) as? Number)

    private fun exactBool(value: Any?): Boolean? = value as? Boolean

    private fun exactKeys(object_: JSONObject, keys: Set<String>): Boolean {
        val actual = mutableSetOf<String>()
        val iterator = object_.keys()
        while (iterator.hasNext()) actual += iterator.next()
        return actual == keys
    }

    private fun onlyKeys(object_: JSONObject, keys: Set<String>): Boolean {
        val iterator = object_.keys()
        while (iterator.hasNext()) if (iterator.next() !in keys) return false
        return true
    }

    private fun validJsonValue(value: Any?): Boolean = when (value) {
        null, JSONObject.NULL, is String, is Boolean -> true
        is Number -> value.toDouble().isFinite()
        is JSONArray -> (0 until value.length()).all { validJsonValue(value.opt(it)) }
        is JSONObject -> {
            val iterator = value.keys()
            var valid = true
            while (iterator.hasNext()) {
                if (!validJsonValue(value.opt(iterator.next()))) valid = false
            }
            valid
        }
        else -> false
    }

    private fun validRenderMark(value: Any?): Boolean = when (value) {
        is String -> true
        is JSONObject -> value.opt("type") is String && validJsonValue(value)
        else -> false
    }

    private fun validListContext(value: Any?): Boolean {
        val object_ = value as? JSONObject ?: return false
        if (!onlyKeys(object_, setOf("ordered", "index", "total", "start", "isFirst", "isLast", "kind", "checked"))) return false
        if (exactBool(object_.opt("ordered")) == null || scalarField(object_, "index") == null ||
            scalarField(object_, "total") == null || scalarField(object_, "start") == null ||
            exactBool(object_.opt("isFirst")) == null || exactBool(object_.opt("isLast")) == null
        ) return false
        val kind = object_.opt("kind")
        if (kind != null && kind !== JSONObject.NULL && kind !is String) return false
        val checked = object_.opt("checked")
        return checked == null || checked === JSONObject.NULL || exactBool(checked) != null
    }

    private fun validMentionTheme(value: Any?): Boolean {
        val object_ = value as? JSONObject ?: return false
        val stringKeys = setOf("textColor", "backgroundColor", "borderColor", "popoverBackgroundColor", "popoverBorderColor", "popoverShadowColor", "optionTextColor", "optionSecondaryTextColor", "optionHighlightedBackgroundColor", "optionHighlightedTextColor")
        val numberKeys = setOf("borderWidth", "borderRadius", "popoverBorderWidth", "popoverBorderRadius")
        if (!onlyKeys(object_, stringKeys + numberKeys + "fontWeight")) return false
        if (stringKeys.any { object_.has(it) && object_.opt(it) !is String }) return false
        if (numberKeys.any { object_.has(it) && (object_.opt(it) !is Number || !(object_.opt(it) as Number).toDouble().isFinite()) }) return false
        val weight = object_.opt("fontWeight")
        return weight == null || weight in setOf("normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900")
    }

    private fun validRenderElement(value: Any?): Boolean {
        val object_ = value as? JSONObject ?: return false
        return when (object_.opt("type") as? String) {
            "textRun" -> exactKeys(object_, setOf("type", "text", "marks")) && object_.opt("text") is String &&
                (object_.opt("marks") as? JSONArray)?.let { marks -> (0 until marks.length()).all { validRenderMark(marks.opt(it)) } } == true
            "blockStart" -> onlyKeys(object_, setOf("type", "nodeType", "depth", "listContext")) && object_.opt("nodeType") is String &&
                scalarField(object_, "depth") != null && (!object_.has("listContext") || validListContext(object_.opt("listContext")))
            "blockEnd" -> exactKeys(object_, setOf("type"))
            "voidInline", "voidBlock" -> onlyKeys(object_, setOf("type", "nodeType", "docPos", "attrs")) && object_.opt("nodeType") is String &&
                scalarField(object_, "docPos") != null && (!object_.has("attrs") || object_.opt("attrs") is JSONObject)
            "opaqueInlineAtom" -> onlyKeys(object_, setOf("type", "nodeType", "label", "docPos", "mentionTheme")) &&
                object_.opt("nodeType") is String && object_.opt("label") is String && scalarField(object_, "docPos") != null &&
                (!object_.has("mentionTheme") || validMentionTheme(object_.opt("mentionTheme")))
            "opaqueBlockAtom" -> exactKeys(object_, setOf("type", "nodeType", "label", "docPos")) &&
                object_.opt("nodeType") is String && object_.opt("label") is String && scalarField(object_, "docPos") != null
            else -> false
        }
    }

    private fun validRenderBlocks(value: Any?): Boolean {
        val blocks = value as? JSONArray ?: return false
        return (0 until blocks.length()).all { blockIndex ->
            val block = blocks.opt(blockIndex) as? JSONArray ?: return@all false
            (0 until block.length()).all { validRenderElement(block.opt(it)) }
        }
    }

    private fun validRenderPatch(value: Any?): Boolean {
        if (value === JSONObject.NULL) return true
        val patch = value as? JSONObject ?: return false
        return exactKeys(patch, setOf("startIndex", "deleteCount", "renderBlocks")) &&
            scalarField(patch, "startIndex") != null && scalarField(patch, "deleteCount") != null &&
            validRenderBlocks(patch.opt("renderBlocks"))
    }

    private fun validBooleanRecord(value: Any?): Boolean {
        val object_ = value as? JSONObject ?: return false
        val iterator = object_.keys()
        while (iterator.hasNext()) if (exactBool(object_.opt(iterator.next())) == null) return false
        return true
    }

    private fun validStringArray(value: Any?): Boolean {
        val array = value as? JSONArray ?: return false
        return (0 until array.length()).all { array.opt(it) is String }
    }

    private fun validActiveState(value: Any?): Boolean {
        val object_ = value as? JSONObject ?: return false
        if (!exactKeys(object_, setOf("marks", "markAttrs", "nodes", "commands", "allowedMarks", "insertableNodes"))) return false
        val attrs = object_.opt("markAttrs") as? JSONObject ?: return false
        val attrsIterator = attrs.keys()
        while (attrsIterator.hasNext()) if (attrs.opt(attrsIterator.next()) !is JSONObject) return false
        return validBooleanRecord(object_.opt("marks")) && validBooleanRecord(object_.opt("nodes")) &&
            validBooleanRecord(object_.opt("commands")) && validStringArray(object_.opt("allowedMarks")) &&
            validStringArray(object_.opt("insertableNodes"))
    }

    private fun scalarSelection(value: Any?): IntArray? {
        val selection = value as? JSONObject ?: return null
        if (selection.opt("type") != "text" || !exactKeys(selection, setOf("type", "anchor", "head", "anchorScalar", "headScalar"))) return null
        if (scalarField(selection, "anchor") == null || scalarField(selection, "head") == null) return null
        return intArrayOf(scalarField(selection, "anchorScalar") ?: return null, scalarField(selection, "headScalar") ?: return null)
    }

    private fun validSelection(value: Any?): Boolean {
        val selection = value as? JSONObject ?: return false
        return when (selection.opt("type") as? String) {
            "text" -> scalarSelection(selection) != null
            "node" -> exactKeys(selection, setOf("type", "pos", "posScalar")) && scalarField(selection, "pos") != null && scalarField(selection, "posScalar") != null
            "all" -> exactKeys(selection, setOf("type"))
            else -> false
        }
    }

    private data class AtomicRenderSnapshot(
        /** Original validated wire payload for controlled-prop delivery. */
        val atomicRenderJson: String,
        val viewUpdateJson: String,
        val documentRevision: ULong,
        val stateRevision: ULong,
        val scalarLength: Int,
        val scalarSelection: IntArray?,
        val activeState: JSONObject,
        val historyState: JSONObject,
    )

    private fun parseAtomicRenderSnapshot(json: String): AtomicRenderSnapshot? {
        return try {
            val object_ = JSONObject(json)
            if (!exactKeys(object_, setOf("renderBlocks", "renderPatch", "selection", "activeState", "historyState", "documentVersion", "stateRevision", "scalarLength")) ||
                !validRenderBlocks(object_.opt("renderBlocks")) || !validRenderPatch(object_.opt("renderPatch")) ||
                !validSelection(object_.opt("selection")) || !validActiveState(object_.opt("activeState"))
            ) return null
            val history = object_.opt("historyState") as? JSONObject ?: return null
            if (!exactKeys(history, setOf("canUndo", "canRedo")) || exactBool(history.opt("canUndo")) == null || exactBool(history.opt("canRedo")) == null) return null
            val revision = ulongField(object_, "documentVersion") ?: return null
            val state = ulongField(object_, "stateRevision") ?: return null
            val scalarLength = scalarField(object_, "scalarLength") ?: return null
            val scalarSelection = scalarSelection(object_.opt("selection"))
            val atomicRenderJson = object_.toString()
            object_.remove("scalarLength")
            AtomicRenderSnapshot(
                atomicRenderJson,
                object_.toString(),
                revision,
                state,
                scalarLength,
                scalarSelection,
                JSONObject(object_.getJSONObject("activeState").toString()),
                JSONObject(history.toString())
            )
        } catch (_: Exception) {
            null
        }
    }

    private fun adopt(snapshot: AtomicRenderSnapshot, stripViewSelection: Boolean): String {
        val update = JSONObject(snapshot.viewUpdateJson)
        if (stripViewSelection) update.remove("selection")
        val updateJson = update.toString()
        baseDocumentRevision = snapshot.documentRevision
        stateRevision = snapshot.stateRevision
        cachedScalarLength = snapshot.scalarLength
        cachedAuthoritativeScalarSelection = snapshot.scalarSelection?.copyOf()
        // The frozen atomic snapshot is also the engine-authoritative answer
        // for local selection suppression. Keeping the old mirror here would
        // let an external selection be silently skipped by the next input.
        lastSyncedScalarSelection = snapshot.scalarSelection?.copyOf()
        cachedActiveState = snapshot.activeState
        cachedHistoryState = snapshot.historyState
        cachedViewUpdateJson = updateJson
        return updateJson
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
        return adopt(snapshot, stripViewSelection = false)
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

    private fun refreshInternal(
        mirrorSelection: IntArray?,
        stripViewSelection: Boolean = mirrorSelection == null,
        controlledPropSnapshot: Boolean = false,
    ): String? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        val derived = when (
            val result = backend.renderUpdate(
                editorId,
                mirrorSelection?.get(0),
                mirrorSelection?.get(1),
            )
        ) {
            is EditorV2CallResult.Err -> {
                // A render update that fails or violates the frozen shape is a
                // boundary failure like any other. Returning null without
                // reporting it leaves every caller — the paired view and the
                // stateless render probe alike — holding a bare null with no
                // cause to surface, so the engine's own error is what travels.
                emit(result.error)
                return null
            }
            is EditorV2CallResult.Ok -> result.value
        }
        renderUpdateCallCountForTesting += 1
        val snapshot = parseAtomicRenderSnapshot(derived)
        return if (snapshot == null) {
            emit(contractError("v2 render update violates the frozen shape"))
            null
        } else {
            // Preserve an IME-owned caret only after authoritative active and
            // history state has been adopted from the post-operation snapshot.
            val viewUpdateJson = adopt(snapshot, stripViewSelection = stripViewSelection)
            if (controlledPropSnapshot) snapshot.atomicRenderJson else viewUpdateJson
        }
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

    private fun invalidateCachedAtomicState(selection: IntArray?) {
        cachedAuthoritativeScalarSelection = selection?.copyOf()
        cachedScalarLength = null
        cachedActiveState = null
        cachedHistoryState = null
        cachedViewUpdateJson = null
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
                val synchronizedSelection = intArrayOf(clampedAnchor, clampedHead)
                lastSyncedScalarSelection = synchronizedSelection
                cachedAuthoritativeScalarSelection = synchronizedSelection.copyOf()
                // Selection changes can affect active state and state revision,
                // but retain the last atomic history result until a document
                // mutation supplies its next locked snapshot.
                cachedActiveState = null
                cachedViewUpdateJson = null
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
        val mapping = when (val outcome = ensureSelection(anchor, head)) {
            is SelectionSyncOutcome.Ok -> resolveSelectionMapping(anchor, head)
            is SelectionSyncOutcome.Refreshed -> textDocumentSelection(outcome.updateJson)
            is SelectionSyncOutcome.Failed -> return null
        }
        if (mapping == null) return null
        publishCollaborationSelection(mapping[0], mapping[1])
        return mapping
    }

    private fun textDocumentSelection(updateJson: String): IntArray? {
        return try {
            val selection = JSONObject(updateJson).getJSONObject("selection")
            if (selection.optString("type") != "text") return null
            intArrayOf(
                scalarField(selection, "anchor") ?: return null,
                scalarField(selection, "head") ?: return null,
            )
        } catch (error: Exception) {
            null
        }
    }

    private fun resolveSelectionMapping(anchor: Int, head: Int): IntArray? {
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
        val mapping = when (val outcome = ensureSelection(anchor, head)) {
            is SelectionSyncOutcome.Ok -> {
                if (!roomBound) return
                val selection = lastSyncedScalarSelection ?: return
                resolveSelectionMapping(selection[0], selection[1])
            }
            is SelectionSyncOutcome.Refreshed -> textDocumentSelection(outcome.updateJson)
            is SelectionSyncOutcome.Failed -> return
        } ?: return
        publishCollaborationSelection(mapping[0], mapping[1])
    }

    private fun publishCachedCollaborationSelection() {
        if (!roomBound) return
        val selection = cachedAuthoritativeScalarSelection ?: return
        val mapping = resolveSelectionMapping(selection[0], selection[1]) ?: return
        publishCollaborationSelection(mapping[0], mapping[1])
    }

    private fun publishCollaborationSelection(docAnchor: Int, docHead: Int) {
        if (!roomBound) return
        val selectionJson = JSONObject()
            .put("type", "text")
            .put("anchor", docAnchor)
            .put("head", docHead)
            .toString()
        when (
            val result = backend.collaborationSetAwarenessSelection(
                editorId,
                selectionJson,
            )
        ) {
            is EditorV2CallResult.Err -> emit(result.error)
            is EditorV2CallResult.Ok -> {
                val outboundChanged = try {
                    val value = JSONObject(result.value)
                    if (value.length() != 1 ||
                        !value.has("outboundChanged") ||
                        value.opt("outboundChanged") !is Boolean
                    ) {
                        null
                    } else {
                        value.getBoolean("outboundChanged")
                    }
                } catch (error: Exception) {
                    null
                }
                if (outboundChanged == null) {
                    emit(contractError("awareness selection result violates the frozen shape"))
                } else if (outboundChanged) {
                    collaborationWake(editorId, CollaborationWakeReason.AWARENESS)
                }
            }
        }
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
                val changed = when (outcome) {
                    is MutationOutcome.Transaction -> {
                        baseDocumentRevision = outcome.revision
                        if (postSelectionMirror != null) {
                            lastSyncedScalarSelection = postSelectionMirror
                        }
                        invalidateCachedAtomicState(postSelectionMirror ?: preSelection)
                        outcome.changed
                    }
                    is MutationOutcome.NotApplicable -> {
                        return refreshInternal(postSelectionMirror ?: preSelection)
                    }
                    is MutationOutcome.Replacement -> {
                        baseDocumentRevision = outcome.revision
                        // Whole-root replacement resets the engine-side selection.
                        lastSyncedScalarSelection = null
                        invalidateCachedAtomicState(null)
                        outcome.changed
                    }
                }
                val mirror = if (includeSelectionInUpdate) postSelectionMirror ?: preSelection else null
                val update = refreshInternal(mirror) ?: return null
                if (changed) {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                update
            }
        }
    }

    /**
     * Split commands need to distinguish a locally committed transaction from
     * a refresh recovered after a stale revision or a no-op outcome. Both
     * paths return a render, but only the former warrants IME-boundary work.
     */
    private fun performSplitMutation(
        preSelection: IntArray,
        postSelectionMirror: IntArray,
        call: () -> EditorV2CallResult<String>,
    ): EditorV2SplitRender? {
        if (destroyed) {
            emit(destroyedError())
            return null
        }
        when (val sync = ensureSelection(preSelection[0], preSelection[1])) {
            is SelectionSyncOutcome.Ok -> Unit
            is SelectionSyncOutcome.Refreshed -> return EditorV2SplitRender(sync.updateJson, committed = false)
            is SelectionSyncOutcome.Failed -> return null
        }
        return when (val result = call()) {
            is EditorV2CallResult.Err ->
                handleMutationError(result.error, postSelectionMirror)
                    ?.let { EditorV2SplitRender(it, committed = false) }
            is EditorV2CallResult.Ok -> {
                val outcome = parseMutationOutcome(result.value)
                if (outcome == null) {
                    emit(contractError("v2 mutation outcome violates the frozen shape"))
                    return null
                }
                when (outcome) {
                    is MutationOutcome.NotApplicable ->
                        refreshInternal(postSelectionMirror)
                            ?.let { EditorV2SplitRender(it, committed = false) }
                    is MutationOutcome.Transaction -> {
                        baseDocumentRevision = outcome.revision
                        lastSyncedScalarSelection = postSelectionMirror
                        invalidateCachedAtomicState(postSelectionMirror)
                        val update = refreshInternal(postSelectionMirror) ?: return null
                        if (outcome.changed) {
                            publishCachedCollaborationSelection()
                            notifyCollaborationMutation()
                        }
                        EditorV2SplitRender(update, committed = outcome.changed)
                    }
                    is MutationOutcome.Replacement -> {
                        baseDocumentRevision = outcome.revision
                        lastSyncedScalarSelection = null
                        invalidateCachedAtomicState(null)
                        val update = refreshInternal(postSelectionMirror) ?: return null
                        if (outcome.changed) {
                            publishCachedCollaborationSelection()
                            notifyCollaborationMutation()
                        }
                        EditorV2SplitRender(update, committed = outcome.changed)
                    }
                }
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
                if (changed) {
                    invalidateCachedAtomicState(null)
                    lastSyncedScalarSelection = null
                }
                val update = refreshInternal(null) ?: return null
                if (changed) {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                update
            }
        }
    }

    // ── Native transport wake ──

    private fun notifyCollaborationMutation() {
        if (!roomBound) return
        collaborationWake(
            editorId,
            CollaborationWakeReason.LOCAL_MUTATION,
        )
    }

    // ── Typed verbs ──

    override fun insertText(text: String, atScalarPos: Int): String? {
        if (text.isEmpty()) return currentStateJson()
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

    override fun splitBlockAt(scalarPos: Int): EditorV2SplitRender? =
        performSplitMutation(
            preSelection = intArrayOf(scalarPos, scalarPos),
            postSelectionMirror = intArrayOf(scalarPos + 1, scalarPos + 1),
        ) {
            callWithEnvelope(JSONObject().put("command", JSONObject().put("type", "splitBlock"))) { requestJson ->
                backend.applyCommand(editorId, requestJson)
            }
        }

    override fun deleteAndSplit(scalarFrom: Int, scalarTo: Int): EditorV2SplitRender? =
        performSplitMutation(
            preSelection = intArrayOf(scalarFrom, scalarTo),
            postSelectionMirror = intArrayOf(scalarFrom + 1, scalarFrom + 1),
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
