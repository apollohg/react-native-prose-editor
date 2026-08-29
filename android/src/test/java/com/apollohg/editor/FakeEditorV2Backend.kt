package com.apollohg.editor

import org.json.JSONArray
import org.json.JSONObject

/**
 * Test double: an in-memory implementation of the v2
 * backend contract used by [EditorV2Adapter]. It enforces the frozen v2
 * envelope invariants (version, request id, base-revision admission,
 * read-only policy, exactly-one outcomes) so adapter tests exercise the same
 * failure shapes the real Rust boundary produces — without loading a native
 * library under Robolectric.
 */
internal class FakeEditorV2Backend : EditorV2Backend {

    internal class FakeSession(
        val editorId: String,
        val readOnly: Boolean,
        val roomBound: Boolean,
    ) {
        var text = StringBuilder("")
        var revision = 0uL
        var destroyed = false
        var anchor = 0
        var head = 0
        var canRedo = false
        val undoStack = ArrayDeque<Triple<String, Int, Int>>()
        val redoStack = ArrayDeque<Triple<String, Int, Int>>()
        val outbox = ArrayDeque<ByteArray>()
        var outboundLease: Pair<String, ByteArray>? = null
        var nextLeaseId = 1uL
        val commands = mutableListOf<JSONObject>()
        var nextPositionEpoch = 1uL
        val positionEpochs = mutableMapOf<String, String>()
        val positionEpochTexts = mutableMapOf<String, String>()
        val nativeOutcomes = mutableMapOf<String, LinkedHashMap<String, String>>()
    }

    val sessions = LinkedHashMap<String, FakeSession>()
    private var nextId = 0uL
    private var nextSplitCommandNotApplicableRemoteText: String? = null
    var advanceRevisionAfterNextRender = false
    var awarenessSelectionResult: EditorV2CallResult<String> =
        EditorV2CallResult.Ok("""{"outboundChanged":false}""")
    var lastAwarenessSelectionJson: String? = null
    var nextPinPositionEpochResult: EditorV2CallResult<String>? = null
    var nextApplyNativeIntentResult: EditorV2CallResult<String>? = null
    var nextRenderUpdateResult: EditorV2CallResult<String>? = null
    var onApplyNativeIntent: (() -> Unit)? = null
    var nextCollaborationDetachError: EditorV2Error? = null
    var nextCollaborationReattachError: EditorV2Error? = null
    var onCollaborationReattach: (() -> Unit)? = null

    /** Generation `collaborationDrive` asks the transport to open, if any. */
    var collaborationGenerationToOpen: String? = null

    /** Backend call log ("applyInput", "getState", ...), for traffic assertions. */
    val calls = mutableListOf<String>()

    /**
     * Makes the next split command return `notApplicable` after a remote peer
     * has advanced the session. The returned render must be adopted, but this
     * is not a local split commit.
     */
    fun forceNextSplitCommandNotApplicableWithRemoteText(text: String) {
        nextSplitCommandNotApplicableRemoteText = text
    }

    private fun liveSession(editorId: String): FakeSession? =
        sessions[editorId]?.takeUnless { it.destroyed }

    private fun destroyedError(): EditorV2Error =
        EditorV2Error(domain = "lifecycle", code = "ENGINE_DESTROYED", message = "editor session is not registered")

    private fun configInvalid(message: String, requestId: String? = null): EditorV2Error =
        EditorV2Error(
            domain = "boundary",
            code = "CONFIG_INVALID",
            message = message,
            requestId = requestId,
        )

    private fun canonicalRequestId(request: JSONObject): String? =
        canonicalV2U64(request.opt("requestId") as? String)

    private fun admitBase(
        session: FakeSession,
        request: JSONObject,
        requestId: String,
    ): EditorV2Error? {
        val base = canonicalV2U64(request.opt("baseDocumentRevision") as? String)
            ?: return configInvalid("baseDocumentRevision must be canonical decimal u64 text", requestId)
        if (base != session.revision.toString()) {
            return EditorV2Error(
                domain = "operation",
                code = "REVISION_MISMATCH",
                message = "document revision mismatch: expected $base, actual ${session.revision}",
                requestId = requestId,
                detailsJson = JSONObject()
                    .put("expectedRevision", base)
                    .put("actualRevision", session.revision.toString())
                    .toString(),
            )
        }
        return null
    }

    private fun admitWritable(session: FakeSession, requestId: String): EditorV2Error? {
        if (!session.readOnly) return null
        return EditorV2Error(
            domain = "boundary",
            code = "MUTATION_REJECTED",
            message = "document is read-only; only selection and local-API requests are allowed",
            requestId = requestId,
        )
    }

    private fun admissionError(editorId: String, request: JSONObject, mutation: Boolean): Pair<FakeSession?, EditorV2Error?> {
        val session = liveSession(editorId) ?: return null to destroyedError()
        val requestId = canonicalRequestId(request)
            ?: return null to configInvalid("requestId must be canonical decimal u64 text")
        val version = exactV2U32(request.opt("version") as? Number)
        if (version != 1u) {
            return null to configInvalid("unsupported v2 envelope version ${request.opt("version")}", requestId)
        }
        if (mutation) {
            admitWritable(session, requestId)?.let { return null to it }
        }
        admitBase(session, request, requestId)?.let { return null to it }
        return session to null
    }

    private fun transactionOutcome(
        session: FakeSession,
        changed: Boolean,
        documentChanged: Boolean = changed,
    ): String =
        JSONObject()
            .put("type", "transaction")
            .put("changed", changed)
            .put("documentChanged", documentChanged)
            .put("documentRevision", session.revision.toString())
            .put("stateRevision", session.revision.toString())
            .put("canUndo", session.undoStack.isNotEmpty())
            .put("canRedo", session.redoStack.isNotEmpty())
            .toString()

    private fun pushUndo(session: FakeSession) {
        session.undoStack.addLast(Triple(session.text.toString(), session.anchor, session.head))
        session.redoStack.clear()
        if (session.roomBound) {
            session.outbox.addLast("update-rev-${session.revision + 1u}".toByteArray())
        }
    }

    /** The selection as [from, to) offsets (collapsed when from == to). */
    private fun orderedSelection(session: FakeSession): Pair<Int, Int> {
        val from = minOf(session.anchor, session.head).coerceIn(0, session.text.length)
        val to = maxOf(session.anchor, session.head).coerceIn(0, session.text.length)
        return from to to
    }

    private fun insertAtSelection(session: FakeSession, text: String): Boolean {
        val (from, to) = orderedSelection(session)
        val changed = session.text.substring(from, to) != text
        if (!changed) {
            val caret = from + text.length
            session.anchor = caret
            session.head = caret
            return false
        }
        pushUndo(session)
        session.text.replace(from, to, text)
        val caret = from + text.length
        session.anchor = caret
        session.head = caret
        session.revision += 1u
        return true
    }

    override fun create(configJson: String, snapshotState: ByteArray?): EditorV2CallResult<String> {
        calls.add("create")
        val config = JSONObject(configJson)
        nextId += 1u
        val editorId = nextId.toString()
        val initialization = config.optJSONObject("initialization")
        val roomBound = initialization?.optString("type") == "room"
        val session = FakeSession(
            editorId = editorId,
            readOnly = config.optJSONObject("policy")?.optBoolean("readOnly", false) ?: false,
            roomBound = roomBound,
        )
        if (roomBound && initialization.has("snapshot")) {
            session.text.append("seed")
        }
        sessions[editorId] = session
        return EditorV2CallResult.Ok(JSONObject().put("editorId", editorId).toString())
    }

    override fun destroy(editorId: String): EditorV2Error? {
        calls.add("destroy")
        val session = sessions[editorId] ?: return destroyedError()
        session.destroyed = true
        return null
    }

    override fun getState(editorId: String): EditorV2CallResult<String> {
        calls.add("getState")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(
            JSONObject()
                .put("documentState", "LocalReady")
                .put("transportState", "Detached")
                .put("renderState", "Ready")
                .put("documentRevision", session.revision.toString())
                .put("stateRevision", session.revision.toString())
                .put("canUndo", session.undoStack.isNotEmpty())
                .put("canRedo", session.redoStack.isNotEmpty())
                .toString()
        )
    }

    override fun getDocumentJson(editorId: String): EditorV2CallResult<String> {
        calls.add("getDocumentJson")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(documentJsonFor(session.text.toString()))
    }

    override fun getDocumentHtml(editorId: String): EditorV2CallResult<String> {
        calls.add("getDocumentHtml")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val html = session.text.split('\n').joinToString("") { "<p>$it</p>" }
        return EditorV2CallResult.Ok(JSONObject().put("html", html).toString())
    }

    override fun applyInput(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("applyInput")
        val request = JSONObject(requestJson)
        val (admittedSession, error) = admissionError(editorId, request, mutation = true)
        if (error != null) return EditorV2CallResult.Err(error)
        val session = admittedSession!!
        val text = request.getString("text")
        if (text.isEmpty()) {
            return EditorV2CallResult.Err(
                EditorV2Error(
                    domain = "boundary",
                    code = "CONFIG_INVALID",
                    message = "input commits require non-empty text",
                    requestId = canonicalRequestId(request),
                )
            )
        }
        insertAtSelection(session, text)
        return EditorV2CallResult.Ok(transactionOutcome(session, changed = true))
    }

    override fun applyCommand(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("applyCommand")
        val request = JSONObject(requestJson)
        val (admittedSession, error) = admissionError(editorId, request, mutation = true)
        if (error != null) return EditorV2CallResult.Err(error)
        val session = admittedSession!!
        val command = request.getJSONObject("command")
        session.commands.add(command)
        if (command.getString("type") in setOf("splitBlock", "deleteAndSplit")) {
            nextSplitCommandNotApplicableRemoteText?.let { remoteText ->
                nextSplitCommandNotApplicableRemoteText = null
                session.text = StringBuilder(remoteText)
                session.anchor = remoteText.length
                session.head = remoteText.length
                session.revision += 1u
                return EditorV2CallResult.Ok(JSONObject().put("type", "notApplicable").toString())
            }
        }
        when (command.getString("type")) {
            "insertText" -> insertAtSelection(session, command.getString("text"))
            "replaceSelectionText" -> insertAtSelection(session, command.getString("text"))
            "deleteBackward" -> {
                val (from, to) = orderedSelection(session)
                if (from == to) {
                    if (from == 0) {
                        return EditorV2CallResult.Ok(JSONObject().put("type", "notApplicable").toString())
                    }
                    pushUndo(session)
                    session.text.deleteCharAt(from - 1)
                    session.anchor = from - 1
                    session.head = from - 1
                } else {
                    pushUndo(session)
                    session.text.delete(from, to)
                    session.anchor = from
                    session.head = from
                }
                session.revision += 1u
            }
            "deleteRange" -> {
                val range = command.getJSONObject("range")
                val from = range.getJSONObject("from").getInt("offset").coerceIn(0, session.text.length)
                val to = range.getJSONObject("to").getInt("offset").coerceIn(0, session.text.length)
                if (from >= to) {
                    return EditorV2CallResult.Ok(JSONObject().put("type", "notApplicable").toString())
                }
                pushUndo(session)
                session.text.delete(from, to)
                session.anchor = from
                session.head = from
                session.revision += 1u
            }
            "splitBlock" -> {
                val (from, to) = orderedSelection(session)
                pushUndo(session)
                session.text.replace(from, to, "\n")
                session.anchor = from + 1
                session.head = from + 1
                session.revision += 1u
            }
            "deleteAndSplit" -> {
                val (from, to) = orderedSelection(session)
                pushUndo(session)
                session.text.delete(from, to)
                session.text.insert(from, "\n")
                session.anchor = from + 1
                session.head = from + 1
                session.revision += 1u
            }
            "insertContentHtml" -> {
                val html = command.getString("html")
                val text = html.replace(Regex("<[^>]+>"), "")
                insertAtSelection(session, text)
            }
            "insertContentJson" -> {
                val fragment = command.getJSONObject("json")
                val fragmentText = documentTextOf(fragment)
                val (from, to) = orderedSelection(session)
                pushUndo(session)
                val before = session.text.substring(0, from)
                val after = session.text.substring(to)
                val newText = listOf(before, fragmentText, after)
                    .filter { it.isNotEmpty() }
                    .joinToString("\n")
                session.text = StringBuilder(newText)
                val caret = listOf(before, fragmentText)
                    .filter { it.isNotEmpty() }
                    .joinToString("\n")
                    .length
                session.anchor = caret
                session.head = caret
                session.revision += 1u
            }
            else -> {
                // Structural commands (marks, blocks, lists, nodes, resize):
                // recorded above; the fake just bumps the revision.
                pushUndo(session)
                session.revision += 1u
            }
        }
        return EditorV2CallResult.Ok(transactionOutcome(session, changed = true))
    }

    override fun applyLocalApi(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("applyLocalApi")
        val request = JSONObject(requestJson)
        val (admittedSession, error) = admissionError(editorId, request, mutation = false)
        if (error != null) return EditorV2CallResult.Err(error)
        val session = admittedSession!!
        session.undoStack.clear()
        session.redoStack.clear()
        when {
            request.has("setHtml") -> {
                val html = request.getString("setHtml")
                session.text = StringBuilder(html.replace(Regex("</p>\\s*<p>"), "\n").replace(Regex("<[^>]+>"), ""))
            }
            request.has("setJson") -> {
                session.text = StringBuilder(documentTextOf(request.getJSONObject("setJson")))
            }
            else -> {
                return EditorV2CallResult.Err(
                    EditorV2Error(
                        domain = "boundary",
                        code = "CONFIG_INVALID",
                        message = "local-API requests carry exactly one of setJson or setHtml",
                        requestId = canonicalRequestId(request),
                    )
                )
            }
        }
        session.anchor = 0
        session.head = 0
        session.revision += 1u
        return EditorV2CallResult.Ok(
            JSONObject()
                .put("type", "replacement")
                .put("changed", true)
                .put("documentRevision", session.revision.toString())
                .toString()
        )
    }

    override fun replaceDocument(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("replaceDocument")
        return applyLocalApi(editorId, requestJson)
    }

    override fun setSelection(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("setSelection")
        val request = JSONObject(requestJson)
        val (admittedSession, error) = admissionError(editorId, request, mutation = false)
        if (error != null) return EditorV2CallResult.Err(error)
        val session = admittedSession!!
        val selection = request.getJSONObject("selection")
        if (selection.getString("type") == "text") {
            session.anchor = selection.getJSONObject("anchor").getInt("offset")
            session.head = selection.getJSONObject("head").getInt("offset")
        }
        return EditorV2CallResult.Ok(transactionOutcome(session, changed = false))
    }

    override fun undo(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("undo")
        val request = JSONObject(requestJson)
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val requestId = canonicalRequestId(request)
            ?: return EditorV2CallResult.Err(configInvalid("requestId must be canonical decimal u64 text"))
        admitWritable(session, requestId)?.let { return EditorV2CallResult.Err(it) }
        val snapshot = session.undoStack.removeLastOrNull()
            ?: return EditorV2CallResult.Ok(JSONObject().put("changed", false).toString())
        session.redoStack.addLast(Triple(session.text.toString(), session.anchor, session.head))
        session.text = StringBuilder(snapshot.first)
        session.anchor = snapshot.second
        session.head = snapshot.third
        session.revision += 1u
        if (session.roomBound) session.outbox.addLast("update-rev-${session.revision}".toByteArray())
        return EditorV2CallResult.Ok(JSONObject().put("changed", true).toString())
    }

    override fun redo(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("redo")
        val request = JSONObject(requestJson)
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val requestId = canonicalRequestId(request)
            ?: return EditorV2CallResult.Err(configInvalid("requestId must be canonical decimal u64 text"))
        admitWritable(session, requestId)?.let { return EditorV2CallResult.Err(it) }
        val snapshot = session.redoStack.removeLastOrNull()
            ?: return EditorV2CallResult.Ok(JSONObject().put("changed", false).toString())
        session.undoStack.addLast(Triple(session.text.toString(), session.anchor, session.head))
        session.text = StringBuilder(snapshot.first)
        session.anchor = snapshot.second
        session.head = snapshot.third
        session.revision += 1u
        if (session.roomBound) session.outbox.addLast("update-rev-${session.revision}".toByteArray())
        return EditorV2CallResult.Ok(JSONObject().put("changed", true).toString())
    }

    private fun collaborationDirective(
        transportState: String,
        generationToOpen: String? = null,
    ): String = JSONObject()
        .put("transportState", transportState)
        .put("generationToOpen", generationToOpen ?: JSONObject.NULL)
        .put("nextDeadlineMillis", JSONObject.NULL)
        .put("remoteCommitApplied", false)
        .put("peersChanged", false)
        .put("renewedLocal", false)
        .put("expiredPeers", JSONArray())
        .toString()

    override fun collaborationDrive(editorId: String, nowMillis: String): EditorV2CallResult<String> {
        calls.add("collaborationDrive")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val generationToOpen = collaborationGenerationToOpen ?: return EditorV2CallResult.Ok(
            collaborationDirective("Detached")
        )
        collaborationGenerationToOpen = null
        return EditorV2CallResult.Ok(collaborationDirective("Connecting", generationToOpen))
    }

    override fun collaborationSocketOpen(
        editorId: String,
        generation: String,
        nowMillis: String,
    ): EditorV2CallResult<String> {
        calls.add("collaborationSocketOpen")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(collaborationDirective("Handshaking"))
    }

    override fun collaborationReceive(
        editorId: String,
        generation: String,
        message: ByteArray,
        nowMillis: String,
    ): EditorV2CallResult<String> {
        calls.add("collaborationReceive")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(collaborationDirective("Synchronized"))
    }

    override fun collaborationSocketClose(
        editorId: String,
        generation: String,
        code: UInt?,
        reason: String?,
        nowMillis: String,
    ): EditorV2CallResult<String> {
        calls.add("collaborationSocketClose")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(collaborationDirective("Disconnected"))
    }

    override fun collaborationLeaseOutbound(
        editorId: String,
        generation: String,
    ): EditorV2LeaseResult {
        calls.add("collaborationLeaseOutbound")
        val session = liveSession(editorId) ?: return EditorV2LeaseResult.Err(destroyedError())
        session.outboundLease?.let { (leaseId, frame) ->
            return EditorV2LeaseResult.Value(EditorV2OutboundLease(leaseId, frame))
        }
        val frame = session.outbox.removeFirstOrNull() ?: return EditorV2LeaseResult.Empty
        val leaseId = session.nextLeaseId.toString()
        session.nextLeaseId += 1u
        session.outboundLease = leaseId to frame
        return EditorV2LeaseResult.Value(EditorV2OutboundLease(leaseId, frame))
    }

    override fun collaborationAckOutbound(
        editorId: String,
        generation: String,
        leaseId: String,
    ): EditorV2CallResult<String> {
        calls.add("collaborationAckOutbound")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        session.outboundLease = null
        return EditorV2CallResult.Ok(collaborationDirective("Synchronized"))
    }

    override fun collaborationNackOutbound(
        editorId: String,
        generation: String,
        leaseId: String,
    ): EditorV2CallResult<String> {
        calls.add("collaborationNackOutbound")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(collaborationDirective("Disconnected"))
    }

    override fun collaborationDetach(editorId: String): EditorV2Error? {
        calls.add("collaborationDetach")
        nextCollaborationDetachError?.let {
            nextCollaborationDetachError = null
            return it
        }
        return if (liveSession(editorId) == null) destroyedError() else null
    }

    override fun collaborationReattach(editorId: String): EditorV2Error? {
        calls.add("collaborationReattach")
        onCollaborationReattach?.invoke()
        nextCollaborationReattachError?.let {
            nextCollaborationReattachError = null
            return it
        }
        return if (liveSession(editorId) == null) destroyedError() else null
    }

    override fun collaborationPeers(editorId: String): EditorV2CallResult<String> {
        calls.add("collaborationPeers")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(JSONObject().put("peers", JSONArray()).toString())
    }

    override fun collaborationSetAwarenessSelection(
        editorId: String,
        selectionJson: String,
    ): EditorV2CallResult<String> {
        calls.add("collaborationSetAwarenessSelection")
        liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        lastAwarenessSelectionJson = selectionJson
        return awarenessSelectionResult
    }

    override fun getContentSnapshot(editorId: String): EditorV2CallResult<String> {
        calls.add("getContentSnapshot")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val docJson = documentJsonFor(session.text.toString())
        val html = session.text.split('\n').joinToString("") { "<p>$it</p>" }
        return EditorV2CallResult.Ok(
            JSONObject().put("html", html).put("json", JSONObject(docJson)).toString()
        )
    }

    override fun renderUpdate(
        editorId: String,
        mirrorAnchor: Int?,
        mirrorHead: Int?,
    ): EditorV2CallResult<String> {
        calls.add("renderUpdate")
        nextRenderUpdateResult?.let { result ->
            nextRenderUpdateResult = null
            return result
        }
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val text = session.text.toString()
        val blocks = JSONArray()
        val paragraphs = text.split('\n')
        paragraphs.forEachIndexed { index, paragraph ->
            val elements = JSONArray()
            elements.put(
                JSONObject()
                    .put("type", "blockStart")
                    .put("nodeType", "paragraph")
                    .put("depth", 0)
            )
            if (paragraph.isNotEmpty()) {
                elements.put(
                    JSONObject()
                        .put("type", "textRun")
                        .put("text", paragraph)
                        .put("marks", JSONArray())
                )
            } else {
                elements.put(
                    JSONObject()
                        .put("type", "textRun")
                        .put("text", "")
                        .put("marks", JSONArray())
                )
            }
            elements.put(JSONObject().put("type", "blockEnd"))
            blocks.put(elements)
        }
        val anchor = mirrorAnchor ?: session.anchor.coerceIn(0, text.length)
        val head = mirrorHead ?: session.head.coerceIn(0, text.length)
        val update = JSONObject()
            .put("renderBlocks", blocks)
            .put("renderPatch", JSONObject.NULL)
            .put(
                "selection",
                JSONObject()
                    .put("type", "text")
                    .put("anchor", docPositionForText(text, anchor))
                    .put("head", docPositionForText(text, head))
                    .put("anchorScalar", anchor)
                    .put("headScalar", head)
            )
            .put(
                "activeState",
                JSONObject()
                    .put("marks", JSONObject())
                    .put("markAttrs", JSONObject())
                    .put("nodes", JSONObject().put("paragraph", true))
                    .put("commands", JSONObject())
                    .put("allowedMarks", JSONArray().put("bold"))
                    .put("insertableNodes", JSONArray().put("hardBreak"))
            )
            .put(
                "historyState",
                JSONObject().put("canUndo", session.undoStack.isNotEmpty()).put("canRedo", session.redoStack.isNotEmpty())
            )
            .put("documentVersion", session.revision.toString())
            .put("stateRevision", session.revision.toString())
            .put("scalarLength", text.codePointCount(0, text.length))
            .put("documentIsEmpty", text.isEmpty())
        val updateJson = update.toString()
        if (advanceRevisionAfterNextRender) {
            advanceRevisionAfterNextRender = false
            session.revision += 1u
        }
        return EditorV2CallResult.Ok(updateJson)
    }

    override fun renderNative(
        editorId: String,
        ownerId: String,
        mirrorAnchor: Int?,
        mirrorHead: Int?,
    ): EditorV2CallResult<String> {
        calls.add("renderNative")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val rendered = renderUpdate(editorId, mirrorAnchor, mirrorHead)
        if (rendered !is EditorV2CallResult.Ok) return rendered
        val epoch = session.nextPositionEpoch++.toString()
        session.positionEpochs[ownerId] = epoch
        session.positionEpochTexts[ownerId] = session.text.toString()
        return EditorV2CallResult.Ok(JSONObject(rendered.value).put("positionEpoch", epoch).toString())
    }

    override fun pinPositionEpoch(
        editorId: String,
        ownerId: String,
        documentRevision: String,
    ): EditorV2CallResult<String> {
        calls.add("pinPositionEpoch")
        nextPinPositionEpochResult?.let { result ->
            nextPinPositionEpochResult = null
            return result
        }
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        if (documentRevision != session.revision.toString()) {
            return EditorV2CallResult.Err(
                EditorV2Error("operation", "REVISION_MISMATCH", "document revision mismatch")
            )
        }
        val epoch = session.nextPositionEpoch++.toString()
        session.positionEpochs[ownerId] = epoch
        session.positionEpochTexts[ownerId] = session.text.toString()
        return EditorV2CallResult.Ok(JSONObject().put("positionEpoch", epoch).toString())
    }

    override fun applyNativeIntent(editorId: String, requestJson: String): EditorV2CallResult<String> {
        calls.add("applyNativeIntent")
        onApplyNativeIntent?.invoke()
        nextApplyNativeIntentResult?.let { result ->
            nextApplyNativeIntentResult = null
            return result
        }
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val request = JSONObject(requestJson)
        val requestId = canonicalRequestId(request)
            ?: return EditorV2CallResult.Err(configInvalid("requestId must be canonical decimal u64 text"))
        val ownerId = canonicalV2U64(request.optString("ownerId"))
            ?: return EditorV2CallResult.Err(configInvalid("ownerId must be canonical decimal u64 text", requestId))
        val cached = session.nativeOutcomes[ownerId]?.get(requestId)
        if (cached != null) return EditorV2CallResult.Ok(cached)
        if (session.positionEpochs[ownerId] != request.optString("positionEpoch")) {
            return EditorV2CallResult.Err(
                EditorV2Error("boundary", "POSITION_EPOCH_INVALID", "position epoch is not pinned", requestId)
            )
        }
        val intent = request.getJSONObject("intent")
        if (intent.getString("type") != "setSelection") {
            admitWritable(session, requestId)?.let { return EditorV2CallResult.Err(it) }
        }
        val epochText = session.positionEpochTexts.getValue(ownerId)
        val selectionBeforeAnchor = session.anchor
        val selectionBeforeHead = session.head
        val collapsed = intent.getInt("anchor") == intent.getInt("head")
        session.anchor = remapEpochOffset(
            intent.getInt("anchor"),
            epochText,
            session.text.toString(),
            affinityAfter = collapsed,
        )
        session.head = remapEpochOffset(
            intent.getInt("head"),
            epochText,
            session.text.toString(),
            affinityAfter = collapsed,
        )
        val outcome = when (intent.getString("type")) {
            "setSelection" -> transactionOutcome(
                session,
                changed = selectionBeforeAnchor != session.anchor || selectionBeforeHead != session.head,
                documentChanged = false,
            )
            "insertText" -> {
                val documentChanged = insertAtSelection(session, intent.getString("text"))
                transactionOutcome(
                    session,
                    changed = documentChanged ||
                        selectionBeforeAnchor != session.anchor || selectionBeforeHead != session.head,
                    documentChanged = documentChanged,
                )
            }
            "replaceSelectionText" -> {
                val documentChanged = insertAtSelection(session, intent.getString("text"))
                transactionOutcome(
                    session,
                    changed = documentChanged ||
                        selectionBeforeAnchor != session.anchor || selectionBeforeHead != session.head,
                    documentChanged = documentChanged,
                )
            }
            "deleteBackward" -> nativeDeleteRange(
                session,
                if (session.anchor == session.head) (session.anchor - 1).coerceAtLeast(0) else minOf(session.anchor, session.head),
                maxOf(session.anchor, session.head),
            )
            "deleteForward" -> nativeDeleteRange(
                session,
                minOf(session.anchor, session.head),
                if (session.anchor == session.head) (session.anchor + 1).coerceAtMost(session.text.length) else maxOf(session.anchor, session.head),
            )
            "deleteSurroundingText" -> nativeDeleteRange(
                session,
                minOf(session.anchor, session.head).minus(intent.getInt("before")).coerceAtLeast(0),
                maxOf(session.anchor, session.head).plus(intent.getInt("after")).coerceAtMost(session.text.length),
            )
            "deleteRange" -> nativeDeleteRange(session, minOf(session.anchor, session.head), maxOf(session.anchor, session.head))
            else -> {
                val command = if (intent.getString("type") == "command") {
                    intent.getJSONObject("command")
                } else {
                    JSONObject(intent.toString()).apply {
                        remove("anchor")
                        remove("head")
                    }
                }
                val result = applyCommand(
                    editorId,
                    JSONObject()
                        .put("version", 1)
                        .put("requestId", requestId)
                        .put("baseDocumentRevision", session.revision.toString())
                        .put("command", command)
                        .toString(),
                )
                if (result is EditorV2CallResult.Err) return result
                (result as EditorV2CallResult.Ok).value
            }
        }
        val retained = JSONObject(outcome).put("positionFallback", false).toString()
        session.nativeOutcomes.getOrPut(ownerId) { LinkedHashMap() }[requestId] = retained
        return EditorV2CallResult.Ok(retained)
    }

    override fun releaseNativeBinding(editorId: String, ownerId: String): EditorV2Error? {
        calls.add("releaseNativeBinding")
        val session = liveSession(editorId) ?: return destroyedError()
        session.positionEpochs.remove(ownerId)
        session.positionEpochTexts.remove(ownerId)
        session.nativeOutcomes.remove(ownerId)
        return null
    }

    private fun remapEpochOffset(
        offset: Int,
        oldText: String,
        newText: String,
        affinityAfter: Boolean,
    ): Int {
        val oldOffset = offset.coerceIn(0, oldText.length)
        var prefix = 0
        val sharedLimit = minOf(oldText.length, newText.length)
        while (prefix < sharedLimit && oldText[prefix] == newText[prefix]) prefix += 1
        var suffix = 0
        while (
            suffix < oldText.length - prefix &&
            suffix < newText.length - prefix &&
            oldText[oldText.length - suffix - 1] == newText[newText.length - suffix - 1]
        ) {
            suffix += 1
        }
        val oldEnd = oldText.length - suffix
        val newEnd = newText.length - suffix
        return when {
            oldOffset < prefix -> oldOffset
            oldOffset > oldEnd -> oldOffset + (newEnd - oldEnd)
            oldOffset == prefix && oldEnd == prefix && !affinityAfter -> prefix
            oldOffset == oldEnd -> newEnd
            affinityAfter -> newEnd
            else -> prefix
        }.coerceIn(0, newText.length)
    }

    private fun nativeDeleteRange(session: FakeSession, from: Int, to: Int): String {
        if (from >= to) return JSONObject().put("type", "notApplicable").toString()
        pushUndo(session)
        session.text.delete(from, to)
        session.anchor = from
        session.head = from
        session.revision += 1u
        return transactionOutcome(session, changed = true)
    }

    override fun resolveScalarSelection(editorId: String, anchor: Int, head: Int): EditorV2CallResult<String> {
        calls.add("resolveScalarSelection")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val text = session.text.toString()
        return EditorV2CallResult.Ok(
            JSONObject()
                .put("type", "text")
                .put("anchor", docPositionForText(text, anchor))
                .put("head", docPositionForText(text, head))
                .put("anchorScalar", anchor)
                .put("headScalar", head)
                .toString()
        )
    }

    override fun docToScalar(editorId: String, docPos: Int): EditorV2CallResult<String> {
        calls.add("docToScalar")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(
            JSONObject().put("scalar", scalarForDocPosition(session.text.toString(), docPos)).toString()
        )
    }

    override fun scalarToDoc(editorId: String, scalar: Int): EditorV2CallResult<String> {
        calls.add("scalarToDoc")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(
            JSONObject().put("doc", docPositionForText(session.text.toString(), scalar)).toString()
        )
    }

    override fun snapshotExport(editorId: String): EditorV2CallResult<Pair<String, ByteArray>> {
        calls.add("snapshotExport")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val metadata = JSONObject()
            .put("formatVersion", 1)
            .put("documentId", "doc-fake")
            .put("lineageId", "lineage-fake")
            .put("fragmentName", "prosemirror")
            .put("schemaFingerprint", "fp-fake")
            .toString()
        return EditorV2CallResult.Ok(metadata to "encoded-state".toByteArray())
    }

    /** Inverse of docPositionForText for the fake's paragraph model. */
    private fun scalarForDocPosition(text: String, docPos: Int): Int {
        var docOffset = 0
        var scalar = 0
        val paragraphs = text.split('\n')
        for (paragraph in paragraphs) {
            docOffset += 1 // open
            if (docPos < docOffset + paragraph.length) {
                return scalar + (docPos - docOffset)
            }
            docOffset += paragraph.length
            scalar += paragraph.length
            docOffset += 1 // close
            if (paragraph !== paragraphs.last()) scalar += 1 // rendered inter-block newline
        }
        return scalar
    }

    companion object {
        fun documentJsonFor(text: String): String {
            val content = JSONArray()
            for (line in text.split('\n')) {
                val paragraph = JSONObject().put("type", "paragraph")
                if (line.isNotEmpty()) {
                    paragraph.put(
                        "content",
                        JSONArray().put(JSONObject().put("type", "text").put("text", line))
                    )
                }
                content.put(paragraph)
            }
            return JSONObject().put("type", "doc").put("content", content).toString()
        }

        fun documentTextOf(doc: JSONObject): String {
            fun blockText(node: JSONObject): String {
                val sb = StringBuilder()
                fun walk(inner: JSONObject) {
                    if (inner.optString("type") == "text") {
                        sb.append(inner.optString("text"))
                    }
                    val content = inner.optJSONArray("content") ?: return
                    for (index in 0 until content.length()) {
                        walk(content.getJSONObject(index))
                    }
                }
                walk(node)
                return sb.toString()
            }
            val content = doc.optJSONArray("content") ?: return ""
            val lines = mutableListOf<String>()
            for (index in 0 until content.length()) {
                lines.add(blockText(content.getJSONObject(index)))
            }
            return lines.joinToString("\n")
        }

        /** ProseMirror-style document position for a scalar in the paragraph model. */
        fun docPositionForText(text: String, scalar: Int): Int {
            var remaining = scalar
            var docOffset = 0
            val lines = text.split('\n')
            lines.forEachIndexed { index, line ->
                docOffset += 1
                if (remaining <= line.length) return docOffset + remaining
                docOffset += line.length + 1
                // The fake's scalar model includes each rendered inter-block newline,
                // matching scalarForDocPosition above. Advance past that scalar as well
                // before entering the next block.
                if (index < lines.lastIndex) {
                    remaining -= line.length + 1
                }
            }
            return docOffset
        }
    }
}
