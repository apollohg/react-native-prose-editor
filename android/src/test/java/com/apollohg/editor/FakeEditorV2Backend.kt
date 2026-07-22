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
        val commands = mutableListOf<JSONObject>()
    }

    val sessions = LinkedHashMap<String, FakeSession>()
    private var nextId = 0uL

    /** Backend call log ("applyInput", "getState", ...), for traffic assertions. */
    val calls = mutableListOf<String>()

    private fun liveSession(editorId: String): FakeSession? =
        sessions[editorId]?.takeUnless { it.destroyed }

    private fun destroyedError(): EditorV2Error =
        EditorV2Error(domain = "lifecycle", code = "ENGINE_DESTROYED", message = "editor session is not registered")

    private fun admitBase(session: FakeSession, request: JSONObject): EditorV2Error? {
        val base = request.getLong("baseDocumentRevision").toULong()
        if (base != session.revision) {
            return EditorV2Error(
                domain = "operation",
                code = "REVISION_MISMATCH",
                message = "document revision mismatch: expected $base, actual ${session.revision}",
                requestId = request.getLong("requestId").toString(),
                detailsJson = JSONObject()
                    .put("expectedRevision", base.toLong())
                    .put("actualRevision", session.revision.toLong())
                    .toString(),
            )
        }
        return null
    }

    private fun admitWritable(session: FakeSession, requestId: ULong): EditorV2Error? {
        if (!session.readOnly) return null
        return EditorV2Error(
            domain = "boundary",
            code = "MUTATION_REJECTED",
            message = "document is read-only; only selection and local-API requests are allowed",
            requestId = requestId.toString(),
        )
    }

    private fun admissionError(editorId: String, request: JSONObject, mutation: Boolean): Pair<FakeSession?, EditorV2Error?> {
        val session = liveSession(editorId) ?: return null to destroyedError()
        val version = request.getLong("version")
        if (version != 1L) {
            return null to EditorV2Error(
                domain = "boundary",
                code = "CONFIG_INVALID",
                message = "unsupported v2 envelope version $version",
                requestId = request.optLong("requestId").toString(),
            )
        }
        val requestId = request.getLong("requestId").toULong()
        if (mutation) {
            admitWritable(session, requestId)?.let { return null to it }
        }
        admitBase(session, request)?.let { return null to it }
        return session to null
    }

    private fun transactionOutcome(session: FakeSession, changed: Boolean): String =
        JSONObject()
            .put("type", "transaction")
            .put("changed", changed)
            .put("documentRevision", session.revision.toLong())
            .put("stateRevision", session.revision.toLong())
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

    private fun insertAtSelection(session: FakeSession, text: String) {
        val (from, to) = orderedSelection(session)
        pushUndo(session)
        session.text.replace(from, to, text)
        val caret = from + text.length
        session.anchor = caret
        session.head = caret
        session.revision += 1u
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
            readOnly = config.optBoolean("readOnly", false),
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
                .put("documentRevision", session.revision.toLong())
                .put("stateRevision", session.revision.toLong())
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
                    requestId = request.getLong("requestId").toString(),
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
                        requestId = request.getLong("requestId").toString(),
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
                .put("documentRevision", session.revision.toLong())
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
        val requestId = request.getLong("requestId").toULong()
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
        val requestId = request.getLong("requestId").toULong()
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

    override fun collaborationTakeOutbound(editorId: String, generation: ULong): EditorV2CallResult<ByteArray> {
        calls.add("takeOutbound")
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        return EditorV2CallResult.Ok(session.outbox.removeFirstOrNull() ?: ByteArray(0))
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
        val session = liveSession(editorId) ?: return EditorV2CallResult.Err(destroyedError())
        val text = session.text.toString()
        val blocks = JSONArray()
        val paragraphs = text.split('\n')
        paragraphs.forEachIndexed { index, paragraph ->
            val elements = JSONArray()
            if (paragraph.isNotEmpty()) {
                elements.put(
                    JSONObject()
                        .put("type", "textRun")
                        .put("text", paragraph)
                        .put("marks", JSONArray())
                        .put("topLevelChildIndex", index)
                )
            } else {
                elements.put(
                    JSONObject()
                        .put("type", "textRun")
                        .put("text", "")
                        .put("marks", JSONArray())
                        .put("topLevelChildIndex", index)
                )
            }
            blocks.put(elements)
        }
        val update = JSONObject().put("renderBlocks", blocks)
        if (mirrorAnchor != null && mirrorHead != null) {
            update.put(
                "selection",
                JSONObject()
                    .put("type", "text")
                    .put("anchor", docPositionForText(text, mirrorAnchor))
                    .put("head", docPositionForText(text, mirrorHead))
                    .put("anchorScalar", mirrorAnchor)
                    .put("headScalar", mirrorHead)
            )
        }
        update.put(
            "activeState",
            JSONObject()
                .put("marks", JSONArray())
                .put("nodes", JSONArray().put("paragraph"))
                .put("commands", JSONArray())
                .put("allowedMarks", JSONArray().put("bold"))
                .put("insertableNodes", JSONArray().put("hardBreak"))
        )
        // Deliberately wrong sentinel values: the adapter MUST override both
        // from the authoritative v2 outcome.
        update.put("historyState", JSONObject().put("canUndo", false).put("canRedo", true))
        update.put("documentVersion", 424242)
        update.put("scalarLength", text.length)
        return EditorV2CallResult.Ok(update.toString())
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
            if (paragraph !== paragraphs.last()) scalar += 0 // '\n' is structural, not a scalar
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
            for (line in text.split('\n')) {
                docOffset += 1
                if (remaining <= line.length) return docOffset + remaining
                docOffset += line.length + 1
                remaining -= line.length
            }
            return docOffset
        }
    }
}
