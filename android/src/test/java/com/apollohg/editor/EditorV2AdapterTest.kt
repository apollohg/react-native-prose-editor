package com.apollohg.editor

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Contract tests for the Android v2 editor adapter.
 *
 * The adapter must translate every native interaction into the typed v2
 * transactions/results. The fake backend enforces the frozen envelope
 * invariants (version, base-revision admission, read-only policy, exactly-one
 * outcomes) so these tests prove the adapter's envelope construction,
 * revision tracking, recovery, and drain-ping semantics without a native
 * library.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorV2AdapterTest {

    private val localEmptyConfig = """{"initialization":{"type":"localEmpty"}}"""
    private lateinit var backend: FakeEditorV2Backend
    private val createdAdapters = mutableListOf<EditorV2Adapter>()

    @Before
    fun setUp() {
        backend = FakeEditorV2Backend()
    }

    private fun makeAdapter(configJson: String = localEmptyConfig): EditorV2Adapter {
        val adapter = EditorV2Adapter.attach(
            backend,
            createEditorId(configJson),
            roomBound = false,
        ) ?: throw AssertionError("created editor could not be attached")
        createdAdapters.add(adapter)
        return adapter
    }

    private fun createEditorId(configJson: String, snapshotState: ByteArray? = null): String = when (
        val created = backend.create(configJson, snapshotState)
    ) {
        is EditorV2CallResult.Ok -> JSONObject(created.value).getString("editorId")
        is EditorV2CallResult.Err -> throw AssertionError("create failed: ${created.error}")
    }

    @Test
    fun `attach rejects non canonical and unknown editor ids`() {
        assertNull(EditorV2Adapter.attach(backend, "01", roomBound = false))
        assertNull(EditorV2Adapter.attach(backend, "not-an-editor", roomBound = false))
        assertNull(EditorV2Adapter.attach(backend, "999999", roomBound = false))
    }

    private fun makeRoomAdapter(
        collaborationWake: (String, CollaborationWakeReason) -> Unit = { editorId, reason ->
            NativeCollaborationTransportRegistry.notifyOutboundAvailable(editorId, reason)
        },
    ): EditorV2Adapter {
        val seed = makeAdapter()
        seed.setContentHtml("<p>seed</p>")
        val snapshot = (backend.snapshotExport(seed.editorId) as EditorV2CallResult.Ok).value
        val adapter = EditorV2Adapter.attach(
            backend,
            createEditorId(
                JSONObject()
                    .put(
                        "initialization",
                        JSONObject()
                            .put("type", "room")
                            .put("documentId", "doc")
                            .put("lineageId", "lineage")
                            .put("snapshot", JSONObject(snapshot.first)),
                    )
                    .toString(),
                snapshot.second,
            ),
            roomBound = true,
            collaborationWake = collaborationWake,
        ) ?: throw AssertionError("created room editor could not be attached")
        createdAdapters.add(adapter)
        return adapter
    }

    private fun renderedText(updateJson: String?): String {
        val update = JSONObject(requireNotNull(updateJson))
        val blocks = update.getJSONArray("renderBlocks")
        val text = StringBuilder()
        for (blockIndex in 0 until blocks.length()) {
            val block = blocks.getJSONArray(blockIndex)
            if (blockIndex > 0) text.append('\n')
            for (elementIndex in 0 until block.length()) {
                val element = block.getJSONObject(elementIndex)
                if (element.optString("type") == "textRun") {
                    text.append(element.optString("text"))
                }
            }
        }
        return text.toString()
    }

    private fun documentText(adapter: EditorV2Adapter): String {
        val result = backend.getDocumentJson(adapter.editorId) as EditorV2CallResult.Ok
        return FakeEditorV2Backend.documentTextOf(JSONObject(result.value))
    }

    private fun sessionOf(adapter: EditorV2Adapter): FakeEditorV2Backend.FakeSession =
        backend.sessions.getValue(adapter.editorId)

    /** A frozen v2 atomic render snapshot, deliberately independent of the fake's legacy payload. */
    private fun atomicRenderSnapshot(text: String, revision: String, selectionScalar: Int = 0): String =
        JSONObject()
            .put(
                "renderBlocks",
                org.json.JSONArray().put(
                    org.json.JSONArray().put(
                        JSONObject()
                            .put("type", "textRun")
                            .put("text", text)
                            .put("marks", org.json.JSONArray())
                    )
                )
            )
            .put("renderPatch", JSONObject.NULL)
            .put(
                "selection",
                JSONObject()
                    .put("type", "text")
                    .put("anchor", selectionScalar)
                    .put("head", selectionScalar)
                    .put("anchorScalar", selectionScalar)
                    .put("headScalar", selectionScalar)
            )
            .put(
                "activeState",
                JSONObject()
                    .put("marks", JSONObject().put("bold", selectionScalar > 0))
                    .put("markAttrs", JSONObject())
                    .put("nodes", JSONObject().put("paragraph", true))
                    .put("commands", JSONObject().put("toggleBold", true))
                    .put("allowedMarks", org.json.JSONArray().put("bold"))
                    .put("insertableNodes", org.json.JSONArray().put("hardBreak"))
            )
            .put("historyState", JSONObject().put("canUndo", true).put("canRedo", false))
            .put("documentVersion", revision)
            .put("stateRevision", revision)
            .put("scalarLength", text.codePointCount(0, text.length))
            .put("documentIsEmpty", text.isEmpty())
            .toString()

    private fun adoptExternalRender(adapter: EditorV2Adapter, snapshot: String): String? =
        adapter.adoptExternalRender(snapshot)

    @Test
    fun `atomic render validation accepts an exclusive render patch`() {
        val adapter = makeAdapter()
        val snapshot = JSONObject(atomicRenderSnapshot("base", "1"))
        val blocks = snapshot.getJSONArray("renderBlocks")
        snapshot.put("renderBlocks", JSONObject.NULL)
        snapshot.put(
            "renderPatch",
            JSONObject()
                .put("startIndex", 0)
                .put("deleteCount", 0)
                .put("renderBlocks", blocks),
        )

        assertNotNull(adoptExternalRender(adapter, snapshot.toString()))
    }

    // MARK: construction

    @Test
    fun `create yields decimal handle and detached local state`() {
        val adapter = makeAdapter(
            """{"initialization":{"type":"localEmpty"},"policy":{"readOnly":false}}"""
        )
        assertTrue(adapter.editorId.toULongOrNull() != null)
        assertEquals(0uL, adapter.baseDocumentRevision)
        val state = JSONObject((backend.getState(adapter.editorId) as EditorV2CallResult.Ok).value)
        assertEquals("LocalReady", state.getString("documentState"))
        assertEquals("Detached", state.getString("transportState"))
    }

    @Test
    fun `setContentHtml uses local API with reset history and renders derived blocks`() {
        val adapter = makeAdapter()
        val update = adapter.setContentHtml("<p>Hello</p>")
        assertEquals("Hello", renderedText(update))
        assertEquals(1uL, adapter.baseDocumentRevision)
        val parsed = JSONObject(requireNotNull(update))
        assertEquals(1, parsed.getInt("documentVersion"))
        assertFalse(parsed.getJSONObject("historyState").getBoolean("canUndo"))
        assertEquals("Hello", documentText(adapter))
    }

    // MARK: commit semantics

    @Test
    fun `typing commit is exactly one local input transaction`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        backend.calls.clear()
        val revisionBefore = adapter.baseDocumentRevision

        val mapping = adapter.syncSelection(2, 2)
        assertNotNull(mapping)
        assertEquals(3, mapping!!.docAnchor)
        assertEquals(3, mapping.docHead)

        backend.calls.clear()
        val update = adapter.insertText("X", 2)
        assertEquals("abX", renderedText(update))
        assertEquals(revisionBefore + 1uL, adapter.baseDocumentRevision)
        assertEquals(1, backend.calls.count { it == "applyInput" })
        assertEquals("abX", documentText(adapter))

        val undone = adapter.undo()
        assertEquals("ab", renderedText(undone))
    }

    @Test
    fun `room typing survives a remote revision advance between keystrokes`() {
        val adapter = makeRoomAdapter()
        adapter.claimNativeBindingIfUnowned(1L)
        assertEquals("seedX", renderedText(adapter.insertText("X", 4)))

        val session = backend.sessions.getValue(adapter.editorId)
        session.text.append("R")
        session.revision += 1uL

        val update = adapter.insertText("Y", 5)

        assertEquals(
            "a keystroke concurrent with a remote update must still be typed",
            "seedXRY",
            documentText(adapter)
        )
        assertEquals("seedXRY", renderedText(update))

        assertEquals("seedXRYZ", documentText(adapter.also { it.insertText("Z", 7) }))
    }

    @Test
    fun `room typing publishes post mutation awareness before transport wake`() {
        val adapter = makeRoomAdapter { _, reason ->
            backend.calls.add("wake:${reason.wireValue}")
        }
        backend.awarenessSelectionResult =
            EditorV2CallResult.Ok("""{"outboundChanged":true}""")
        backend.calls.clear()

        val update = adapter.insertText("X", 4)

        assertEquals("seedX", renderedText(update))
        assertEquals(
            listOf(
                "collaborationSetAwarenessSelection",
                "wake:awareness",
                "wake:localMutation",
            ),
            backend.calls.filter {
                it == "collaborationSetAwarenessSelection" || it.startsWith("wake:")
            },
        )
        val awarenessSelection = JSONObject(backend.lastAwarenessSelectionJson!!)
        assertEquals(3, awarenessSelection.length())
        assertEquals("text", awarenessSelection.getString("type"))
        assertEquals(6, awarenessSelection.getInt("anchor"))
        assertEquals(6, awarenessSelection.getInt("head"))
    }

    @Test
    fun `room standalone selection publishes awareness without document wake`() {
        val adapter = makeRoomAdapter { _, reason ->
            backend.calls.add("wake:${reason.wireValue}")
        }
        backend.awarenessSelectionResult =
            EditorV2CallResult.Ok("""{"outboundChanged":true}""")
        backend.calls.clear()

        val mapping = adapter.syncSelection(1, 1)

        assertNotNull(mapping)
        assertEquals(
            listOf("collaborationSetAwarenessSelection", "wake:awareness"),
            backend.calls.filter {
                it == "collaborationSetAwarenessSelection" || it.startsWith("wake:")
            },
        )
    }

    @Test
    fun `refresh reports a failed engine render update`() {
        // A render update that fails used to return null in silence, so no
        // caller — the paired view or the stateless render probe — could name
        // the cause.
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }

        // Destroy the session behind the adapter's back: the next render
        // update reaches a handle the backend no longer knows.
        backend.destroy(adapter.editorId)

        assertNull(adapter.refreshFromRustState(mirrorSelection = null))
        assertEquals(1, errors.size)
        assertEquals("lifecycle", errors.single().domain)
    }

    @Test
    fun `awareness publication failure does not roll back committed typing`() {
        val adapter = makeRoomAdapter { _, _ -> }
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }
        backend.awarenessSelectionResult = EditorV2CallResult.Err(
            EditorV2Error(
                domain = "transport",
                code = "TRANSPORT_RESOURCE_EXHAUSTED",
                message = "awareness outbox is full",
            ),
        )

        val update = adapter.insertText("X", 4)

        assertEquals("seedX", renderedText(update))
        assertEquals("seedX", documentText(adapter))
        assertEquals("TRANSPORT_RESOURCE_EXHAUSTED", errors.last().code)
    }

    @Test
    fun `replacement commit is one transaction`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>teh</p>")
        val revisionBefore = adapter.baseDocumentRevision
        backend.calls.clear()

        val update = adapter.replaceTextRange(0, 3, "the")
        assertEquals("the", renderedText(update))
        val selection = JSONObject(requireNotNull(update)).getJSONObject("selection")
        assertEquals(3, selection.getInt("anchorScalar"))
        assertEquals(3, selection.getInt("headScalar"))
        assertEquals(revisionBefore + 1uL, adapter.baseDocumentRevision)
        assertEquals(1, backend.calls.count { it == "applyCommand" })
        assertEquals(0, backend.calls.count { it == "applyInput" })
        assertEquals("the", documentText(adapter))
    }

    @Test
    fun `backward selection position mapping is exact`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>abcd</p>")
        val mapping = adapter.syncSelection(3, 1)
        assertNotNull(mapping)
        assertEquals(4, mapping!!.docAnchor)
        assertEquals(2, mapping.docHead)

        val update = adapter.replaceTextRange(1, 3, "X")
        assertEquals("aXd", renderedText(update))
        assertEquals("aXd", documentText(adapter))
    }

    @Test
    fun `delete backward return and delete-and-split route typed commands`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")

        val deleted = adapter.deleteBackwardAtSelection(2, 2)
        assertEquals("a", renderedText(deleted))
        val backwardSelection = JSONObject(requireNotNull(deleted)).getJSONObject("selection")
        assertEquals(1, backwardSelection.getInt("anchorScalar"))
        assertEquals(1, backwardSelection.getInt("headScalar"))

        val split = adapter.splitBlockAt(1)
        assertNotNull(split)
        assertTrue(split!!.committed)
        assertEquals("a\n", renderedText(split.updateJson))

        adapter.setContentHtml("<p>abcd</p>")
        val update = adapter.deleteAndSplit(1, 3)
        assertNotNull(update)
        assertTrue(update!!.committed)
        assertEquals("a\nd", renderedText(update.updateJson))
    }

    @Test
    fun `range deletion render carries post-delete caret`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>abcd</p>")

        val deleted = adapter.deleteScalarRange(1, 3)

        assertEquals("ad", renderedText(deleted))
        val selection = JSONObject(requireNotNull(deleted)).getJSONObject("selection")
        assertEquals(1, selection.getInt("anchorScalar"))
        assertEquals(1, selection.getInt("headScalar"))
    }

    @Test
    fun `commands route as typed command envelopes at the synced selection`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        backend.calls.clear()

        assertNotNull(adapter.toggleMark("bold", 0, 2))
        assertNotNull(adapter.toggleHeading(2, 1, 1))
        assertNotNull(adapter.wrapInList("bulletList", 1, 1))
        assertNotNull(adapter.insertNode("hardBreak", 1, 1))
        assertNotNull(adapter.toggleBlockquote(1, 1))
        assertNotNull(adapter.toggleCodeBlock(1, 1))
        assertNotNull(adapter.indentListItem(1, 1))
        assertNotNull(adapter.outdentListItem(1, 1))
        assertNotNull(adapter.toggleTaskItemCheckedAtSelection(1, 1))

        val commands = sessionOf(adapter).commands
        assertEquals("toggleMark", commands[0].getString("type"))
        assertEquals("bold", commands[0].getString("markType"))
        assertEquals("toggleHeading", commands[1].getString("type"))
        assertEquals(2, commands[1].getInt("level"))
        assertEquals("wrapInList", commands[2].getString("type"))
        assertEquals("bulletList", commands[2].getString("listType"))
        assertEquals("listItem", commands[2].getString("itemType"))
        assertEquals("insertNode", commands[3].getString("type"))
        assertEquals("hardBreak", commands[3].getString("nodeType"))
        assertEquals("toggleBlockquote", commands[4].getString("type"))
        assertEquals("toggleCodeBlock", commands[5].getString("type"))
        assertEquals("indentListItem", commands[6].getString("type"))
        assertEquals("outdentListItem", commands[7].getString("type"))
        assertEquals("toggleTaskItemChecked", commands[8].getString("type"))
        assertEquals(9, commands.size)
    }

    @Test
    fun `ProseMirror list command uses snake case list item`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        backend.calls.clear()

        assertNotNull(adapter.wrapInList("bullet_list", 1, 1))

        val command = sessionOf(adapter).commands.single()
        assertEquals("bullet_list", command.getString("listType"))
        assertEquals("list_item", command.getString("itemType"))
    }

    @Test
    fun `paste routes typed content commands`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")

        val html = adapter.insertContentHtmlAtSelection("<strong>CD</strong>", 2, 2)
        assertEquals("abCD", renderedText(html))

        val fragment = "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"X\"}]}]}"
        val json = adapter.insertContentJsonAtSelection(fragment, 4, 4)
        assertNotNull(json)
        assertEquals("abCD\nX", documentText(adapter))
    }

    @Test
    fun `resize image converts doc position through rust mapping`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        assertNotNull(adapter.resizeImageAtDocPos(1, 120, 80))
        val command = sessionOf(adapter).commands.last()
        assertEquals("resizeImage", command.getString("type"))
        assertEquals(120, command.getInt("width"))
        assertEquals(80, command.getInt("height"))
        assertTrue(backend.calls.contains("docToScalar"))
    }

    // MARK: Task 16B render accessor (probe replacement)

    @Test
    fun `render update carries blocks active state and native input post caret`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        backend.calls.clear()
        val update = JSONObject(requireNotNull(adapter.insertText("c", 2)))
        assertTrue(update.has("renderBlocks"))
        assertTrue(update.has("activeState"))
        // The scalar extent rides the accessor payload but stays
        // adapter-internal (the view-facing update keeps the legacy shape).
        assertFalse(update.has("scalarLength"))
        // History/version are the v2 engine's facts, never the fake's
        // deliberately wrong sentinels.
        assertEquals(2, update.getInt("documentVersion"))
        val history = update.getJSONObject("historyState")
        assertTrue(history.getBoolean("canUndo"))
        assertFalse(history.getBoolean("canRedo"))
        // A full render replacement resets Android's selection. The native input update must
        // therefore carry the adapter's authoritative post-input scalar caret.
        val selection = update.getJSONObject("selection")
        assertEquals("text", selection.getString("type"))
        assertEquals(3, selection.getInt("anchorScalar"))
        assertEquals(3, selection.getInt("headScalar"))
        assertTrue(backend.calls.contains("renderUpdate"))
    }

    @Test
    fun `render update mirrors scalar selection to doc positions`() {
        val adapter = makeAdapter()
        val update = JSONObject(
            requireNotNull(adapter.setContentHtml("<p>ab</p><p>cd</p>"))
        )
        val selection = update.getJSONObject("selection")
        assertEquals("text", selection.getString("type"))
        assertEquals(0, selection.getInt("anchorScalar"))
        assertEquals(0, selection.getInt("headScalar"))
        assertEquals(1, selection.getInt("anchor"))
        assertEquals(1, selection.getInt("head"))
    }

    @Test
    fun `empty document refresh carries blocks and no selection`() {
        val adapter = makeAdapter()
        val update = JSONObject(requireNotNull(adapter.currentStateJson()))
        assertTrue(update.has("renderBlocks"))
        assertTrue(update.has("activeState"))
        assertFalse(update.has("scalarLength"))
        assertFalse(update.has("selection"))
        assertEquals(0, update.getInt("documentVersion"))
    }

    @Test
    fun `position mapping routes through the v2 accessor`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p><p>cd</p>")
        backend.calls.clear()
        // The structural separator between rendered blocks occupies one scalar.
        assertEquals(3, adapter.scalarPositionForDoc(5))
        assertEquals(3, adapter.docPositionForScalar(2))
        assertTrue(backend.calls.contains("docToScalar"))
        assertTrue(backend.calls.contains("scalarToDoc"))
        assertFalse(backend.calls.contains("deriveRenderUpdate"))
        assertFalse(backend.calls.contains("scalarLengthForDoc"))
    }

    // MARK: read-only atomicity

    @Test
    fun `read only rejects every mutation path atomically`() {
        val adapter = makeAdapter(
            """{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"""
        )
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }

        // Controlled local content passes under read-only (Source::Api parity).
        val seed = adapter.setContentHtml("<p>seed</p>")
        assertEquals("seed", renderedText(seed))
        assertTrue(errors.isEmpty())
        val revisionAfterSeed = adapter.baseDocumentRevision

        val mutations: List<Pair<String, () -> String?>> = listOf(
            "insertText" to { adapter.insertText("x", 0) },
            "replaceTextRange" to { adapter.replaceTextRange(0, 1, "x") },
            "deleteBackward" to { adapter.deleteBackwardAtSelection(1, 1) },
            "deleteScalarRange" to { adapter.deleteScalarRange(0, 1) },
            "splitBlock" to { adapter.splitBlockAt(1)?.updateJson },
            "deleteAndSplit" to { adapter.deleteAndSplit(0, 1)?.updateJson },
            "insertNode" to { adapter.insertNode("hardBreak", 1, 1) },
            "insertContentHtml" to { adapter.insertContentHtmlAtSelection("<p>x</p>", 1, 1) },
            "insertContentJson" to { adapter.insertContentJsonAtSelection("{\"type\":\"paragraph\"}", 1, 1) },
            "toggleMark" to { adapter.toggleMark("bold", 0, 1) },
            "setMark" to { adapter.setMark("link", "{\"href\":\"https://example.com\"}", 0, 1) },
            "unsetMark" to { adapter.unsetMark("bold", 0, 1) },
            "toggleHeading" to { adapter.toggleHeading(2, 1, 1) },
            "toggleCodeBlock" to { adapter.toggleCodeBlock(1, 1) },
            "toggleBlockquote" to { adapter.toggleBlockquote(1, 1) },
            "wrapInList" to { adapter.wrapInList("bulletList", 1, 1) },
            "unwrapFromList" to { adapter.unwrapFromList(1, 1) },
            "indentListItem" to { adapter.indentListItem(1, 1) },
            "outdentListItem" to { adapter.outdentListItem(1, 1) },
            "toggleTaskItemChecked" to { adapter.toggleTaskItemCheckedAtSelection(1, 1) },
            "resizeImage" to { adapter.resizeImageAtDocPos(0, 10, 10) },
            "undo" to { adapter.undo() },
            "redo" to { adapter.redo() },
        )
        for ((name, mutate) in mutations) {
            assertNull("read-only $name must be rejected", mutate())
            assertEquals("$name domain", "boundary", errors.last().domain)
            assertEquals("$name code", "MUTATION_REJECTED", errors.last().code)
            assertNotNull("$name request id", errors.last().requestId)
        }

        assertEquals("seed", documentText(adapter))
        assertEquals(revisionAfterSeed, adapter.baseDocumentRevision)

        // Selection/navigation remains allowed.
        val mapping = adapter.syncSelection(1, 1)
        assertNotNull(mapping)
        assertEquals(2, mapping!!.docAnchor)

        // Controlled content still passes.
        val replaced = adapter.setContentJson(
            "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"api\"}]}]}"
        )
        assertEquals("api", renderedText(replaced))
    }

    // MARK: stale revision recovery

    @Test
    fun `pre sync mismatch refuses selection relative input without replay`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.append("R")
        session.revision += 1u
        backend.calls.clear()

        val update = adapter.insertText("X", 2)

        assertEquals("baseR", documentText(adapter))
        assertEquals("baseR", renderedText(update))
        assertEquals(1, backend.calls.count { it == "setSelection" })
        assertEquals(0, backend.calls.count { it == "applyInput" })
    }

    @Test
    fun `pre sync mismatch refuses selection replacement without replay`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.append("R")
        session.revision += 1u

        val update = adapter.replaceTextRange(1, 3, "Q")

        assertEquals("baseR", documentText(adapter))
        assertEquals("baseR", renderedText(update))
    }

    @Test
    fun `pre sync mismatch refuses split without replay`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.append("R")
        session.revision += 1u
        backend.calls.clear()

        val split = adapter.splitBlockAt(2)

        assertNotNull(split)
        assertFalse(split!!.committed)
        assertEquals("baseR", renderedText(split.updateJson))
        assertEquals(0, backend.calls.count { it == "applyCommand" })
    }

    @Test
    fun `a second mismatch after recovery returns a refresh without another retry`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.append("R")
        session.revision += 1u
        backend.advanceRevisionAfterNextRender = true
        backend.calls.clear()

        val update = adapter.insertText("X", 2)

        assertNotNull(update)
        assertEquals("baseR", documentText(adapter))
        assertEquals(0, backend.calls.count { it == "applyInput" })
        assertEquals(1, backend.calls.count { it == "renderUpdate" })
    }

    @Test
    fun `revision mismatch refuses caret relative input without replay`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")

        // Sync the caret while fresh, then externally advance the same
        // session so the adapter's tracked base goes stale.
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.insert(0, "EXT")
        session.revision += 1u

        backend.calls.clear()
        val update = adapter.insertText("REBASED", 0)
        assertNotNull("a stale input returns the authoritative refresh", update)
        assertEquals("the keystroke is attempted once", 1L, backend.calls.count { it == "applyInput" }.toLong())
        assertEquals("a race refreshes exclusively through atomic renders", 0, backend.calls.count { it == "getState" })
        assertEquals("one render recovers the race", 1, backend.calls.count { it == "renderUpdate" })
        assertEquals("EXTbase", renderedText(update))
        assertEquals("EXTbase", documentText(adapter))

        val recovered = adapter.insertText("ok", 0)
        assertEquals("okEXTbase", renderedText(recovered))
    }

    @Test
    fun `a toolbar state read between keystrokes does not defeat the rebase`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>seed</p>")
        adapter.syncSelection(4, 4)
        adapter.currentStateJson()
        val session = sessionOf(adapter)
        session.text.append("R")
        session.revision += 1u

        backend.calls.clear()
        adapter.insertText("X", 4)

        assertEquals("seedR", documentText(adapter))
        assertFalse(backend.calls.any { it == "applyInput" })
    }

    @Test
    fun `a mirrored refresh does not cache its mirror as the synced selection`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>seed</p>")
        adapter.syncSelection(4, 4)
        val session = sessionOf(adapter)
        session.text.append("R")
        session.revision += 1u

        adapter.deleteScalarRange(1, 3)
        backend.calls.clear()

        adapter.insertText("Q", 1)

        assertEquals(
            "the next keystroke must re-sync rather than trust the mirror",
            1L,
            backend.calls.count { it == "setSelection" }.toLong()
        )
        assertEquals("sQeedR", documentText(adapter))
    }

    @Test
    fun `revision mismatch never rebases a positioned mutation`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.insert(0, "EXT")
        session.revision += 1u

        backend.calls.clear()
        val update = adapter.deleteScalarRange(0, 4)

        assertNotNull(update)
        assertEquals(
            "a mutation carrying explicit positions must never be replayed",
            1L,
            backend.calls.count { it == "applyCommand" }.toLong()
        )
        assertEquals("EXTbase", documentText(adapter))
    }

    @Test
    fun `split renders refuse a stale split and mark not applicable uncommitted`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.insert(0, "REMOTE ")
        session.revision += 1u

        backend.calls.clear()
        val stale = adapter.splitBlockAt(0)

        assertNotNull(stale)
        assertFalse(stale!!.committed)
        assertEquals("REMOTE base", renderedText(stale.updateJson))
        assertEquals(1, backend.calls.count { it == "applyCommand" })
        assertEquals(1, backend.calls.count { it == "renderUpdate" })

        adapter.setContentHtml("<p>next</p>")
        adapter.syncSelection(0, 1)
        backend.forceNextSplitCommandNotApplicableWithRemoteText("REMOTE")
        backend.calls.clear()
        val notApplicable = adapter.deleteAndSplit(0, 1)

        assertNotNull(notApplicable)
        assertFalse(notApplicable!!.committed)
        assertEquals("REMOTE", renderedText(notApplicable.updateJson))
        assertEquals(1, backend.calls.count { it == "applyCommand" })
        assertEquals(1, backend.calls.count { it == "renderUpdate" })
    }

    @Test
    fun `adopting external N plus one snapshot commits first native key at N plus two`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")
        val session = sessionOf(adapter)
        session.text.insert(0, "EXT")
        session.revision += 1u
        val snapshot = atomicRenderSnapshot("EXTbase", session.revision.toString(), selectionScalar = 0)

        backend.calls.clear()
        val adopted = adoptExternalRender(adapter, snapshot)
        assertEquals("EXTbase", renderedText(adopted))
        assertEquals(2uL, adapter.baseDocumentRevision)

        val committed = adapter.insertText("K", 0)
        assertEquals("KEXTbase", renderedText(committed))
        assertEquals(3uL, adapter.baseDocumentRevision)
        assertEquals("KEXTbase", documentText(adapter))
        assertEquals(0, backend.calls.count { it == "getState" })
    }

    @Test
    fun `external adoption keeps the previous snapshot when epoch pinning fails`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>a</p>")
        adapter.claimNativeBindingIfUnowned(99L)
        val session = sessionOf(adapter)
        val snapshotA = atomicRenderSnapshot("a", session.revision.toString(), selectionScalar = 0)
        assertNotNull(adoptExternalRender(adapter, snapshotA))
        val revisionA = adapter.baseDocumentRevision
        val stateRevisionA = adapter.stateRevision
        val epochA = session.positionEpochs.getValue("99")
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = errors::add
        val snapshotB = JSONObject(
            atomicRenderSnapshot("bb", (revisionA + 1u).toString(), selectionScalar = 1),
        )
            .put("historyState", JSONObject().put("canUndo", false).put("canRedo", true))
            .toString()
        backend.nextPinPositionEpochResult = EditorV2CallResult.Err(
            EditorV2Error("operation", "REVISION_MISMATCH", "stale"),
        )

        assertNull(adoptExternalRender(adapter, snapshotB))

        assertEquals(revisionA, adapter.baseDocumentRevision)
        assertEquals(stateRevisionA, adapter.stateRevision)
        val cachedA = requireNotNull(adapter.atomicRenderJson(revisionA.toString()))
        assertEquals("a", renderedText(cachedA))
        assertEquals(0, JSONObject(cachedA).getJSONObject("selection").getInt("anchorScalar"))
        assertFalse(JSONObject(cachedA).getJSONObject("activeState").getJSONObject("marks").getBoolean("bold"))
        assertEquals(true, adapter.historyCanUndo())
        assertEquals(false, adapter.historyCanRedo())
        assertNull(adapter.atomicRenderJson((revisionA + 1u).toString()))
        assertEquals(epochA, session.positionEpochs.getValue("99"))
        assertEquals("REVISION_MISMATCH", errors.single().code)
    }

    @Test
    fun `external adoption keeps the previous snapshot when epoch pin result is malformed`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>a</p>")
        adapter.claimNativeBindingIfUnowned(99L)
        val session = sessionOf(adapter)
        val snapshotA = atomicRenderSnapshot("a", session.revision.toString(), selectionScalar = 0)
        assertNotNull(adoptExternalRender(adapter, snapshotA))
        val revisionA = adapter.baseDocumentRevision
        val epochA = session.positionEpochs.getValue("99")
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = errors::add
        backend.nextPinPositionEpochResult = EditorV2CallResult.Ok("{}")

        assertNull(
            adoptExternalRender(
                adapter,
                atomicRenderSnapshot("bb", (revisionA + 1u).toString(), selectionScalar = 1),
            ),
        )

        assertEquals(revisionA, adapter.baseDocumentRevision)
        assertEquals("a", renderedText(adapter.atomicRenderJson(revisionA.toString())))
        assertEquals(epochA, session.positionEpochs.getValue("99"))
        assertEquals("FFI_RESULT_INVALID", errors.single().code)
    }

    @Test
    fun `atomic external snapshot rejects non canonical revision without changing adopted state`() {
        val adapter = makeAdapter()
        val valid = atomicRenderSnapshot("base", "7", selectionScalar = 1)
        assertNotNull(adoptExternalRender(adapter, valid))
        val revisionBefore = adapter.baseDocumentRevision
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors += it }

        val malformed = JSONObject(valid).put("documentVersion", "07").toString()
        assertNull(adoptExternalRender(adapter, malformed))
        assertEquals(revisionBefore, adapter.baseDocumentRevision)
        assertEquals("FFI_RESULT_INVALID", errors.single().code)
    }

    @Test
    fun `atomic external snapshot accepts an inserted mention carrying node attrs`() {
        // Rust emits `attrs` on every void/opaque element, so an inserted
        // mention must survive validation on its way back to the view.
        val adapter = makeAdapter()
        val mention = JSONObject()
            .put("type", "opaqueInlineAtom")
            .put("nodeType", "mention")
            .put("label", "@Alice Chen")
            .put("docPos", 1)
            .put(
                "attrs",
                JSONObject()
                    .put("id", "user-alice")
                    .put("label", "Alice Chen")
                    .put("mentionSuggestionChar", "@")
                    .put("type", "user")
            )
            .put(
                "mentionTheme",
                JSONObject().put("node", JSONObject().put("textColor", "#336EC1"))
            )
        val snapshot = JSONObject(atomicRenderSnapshot("base", "7", selectionScalar = 1))
            .put(
                "renderBlocks",
                org.json.JSONArray().put(org.json.JSONArray().put(mention))
            )
            .toString()

        assertNotNull(adoptExternalRender(adapter, snapshot))
    }

    @Test
    fun `atomic external snapshot rejects an opaque atom with non object attrs`() {
        val adapter = makeAdapter()
        val mention = JSONObject()
            .put("type", "opaqueInlineAtom")
            .put("nodeType", "mention")
            .put("label", "Ada")
            .put("docPos", 1)
            .put("attrs", org.json.JSONArray())
        val snapshot = JSONObject(atomicRenderSnapshot("base", "7", selectionScalar = 1))
            .put(
                "renderBlocks",
                org.json.JSONArray().put(org.json.JSONArray().put(mention))
            )
            .toString()

        assertNull(adoptExternalRender(adapter, snapshot))
    }

    @Test
    fun `external snapshot keeps active state derived from its authoritative selection`() {
        val adapter = makeAdapter()
        val adopted = JSONObject(
            requireNotNull(
                adoptExternalRender(adapter, atomicRenderSnapshot("ab", "4", selectionScalar = 1))
            )
        )

        assertTrue(adopted.getJSONObject("activeState").getJSONObject("marks").getBoolean("bold"))
        assertEquals(4uL, adapter.baseDocumentRevision)
    }

    @Test
    fun `external atomic adoption serves authoritative selection and history without split state reads`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        val session = sessionOf(adapter)
        val snapshot = JSONObject(atomicRenderSnapshot("ab", session.revision.toString(), selectionScalar = 2))
            .put("historyState", JSONObject().put("canUndo", false).put("canRedo", true))
            .toString()

        assertNotNull(adoptExternalRender(adapter, snapshot))
        backend.calls.clear()

        assertEquals(false, adapter.historyCanUndo())
        assertEquals(true, adapter.historyCanRedo())
        val state = JSONObject(requireNotNull(adapter.currentStateJson()))
        assertEquals(2, state.getJSONObject("selection").getInt("anchorScalar"))
        assertEquals(
            2,
            JSONObject(requireNotNull(adapter.selectionJson())).getInt("anchorScalar")
        )
        assertEquals(0, backend.calls.count { it == "getState" })
    }

    @Test
    fun `local selection and mutation replace adopted authoritative caches coherently`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        assertNotNull(adapter.syncSelection(1, 1))
        val session = sessionOf(adapter)
        session.anchor = 2
        session.head = 2
        session.revision += 1u
        val externalSnapshot = atomicRenderSnapshot("ab", session.revision.toString(), selectionScalar = 2)
        assertNotNull(adoptExternalRender(adapter, externalSnapshot))

        backend.calls.clear()
        assertNotNull(adapter.syncSelection(1, 1))
        assertTrue(backend.calls.contains("setSelection"))

        backend.calls.clear()
        val updated = adapter.insertText("x", 1)
        assertEquals("axb", renderedText(updated))
        assertEquals(true, adapter.historyCanUndo())
        assertEquals(false, adapter.historyCanRedo())
        assertEquals(0, backend.calls.count { it == "getState" })
        assertEquals(
            2,
            JSONObject(requireNotNull(adapter.selectionJson())).getInt("anchorScalar")
        )
    }

    // MARK: undo/redo

    @Test
    fun `undo redo round trip`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        adapter.insertText("c", 2)
        assertEquals("abc", documentText(adapter))

        val undone = adapter.undo()
        assertEquals("ab", renderedText(undone))
        val redone = adapter.redo()
        assertEquals("abc", renderedText(redone))
    }

    // MARK: lifecycle races

    @Test
    fun `destroy mid operations yields structured failure without crash`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }

        adapter.destroy()

        assertNull(adapter.insertText("x", 0))
        assertEquals("lifecycle", errors.last().domain)
        assertEquals("ENGINE_DESTROYED", errors.last().code)

        assertNull(adapter.refreshFromRustState(null))
        assertEquals("ENGINE_DESTROYED", errors.last().code)

        // Repeated destroy is safe.
        adapter.destroy()
        adapter.destroy()
    }

    @Test
    fun `stale autonomous error owner cannot clear newer owner`() {
        val adapter = makeAdapter()
        val firstErrors = mutableListOf<EditorV2Error>()
        val secondErrors = mutableListOf<EditorV2Error>()

        adapter.bindAutonomousErrorOwner(101L, { firstErrors += it }) {}
        adapter.bindAutonomousErrorOwner(202L, { secondErrors += it }) {}
        adapter.clearAutonomousErrorOwner(101L)
        adapter.destroy()

        assertNull(adapter.insertText("x", 0))
        assertTrue(firstErrors.isEmpty())
        assertEquals(1, secondErrors.size)
        assertEquals("ENGINE_DESTROYED", secondErrors.single().code)
    }

    // MARK: synthesized update contract

    @Test
    fun `synthesized update carries rust state not fabricated state`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        val update = JSONObject(requireNotNull(adapter.insertText("c", 2)))
        assertEquals(2, update.getInt("documentVersion"))
        val history = update.getJSONObject("historyState")
        assertTrue(history.getBoolean("canUndo"))
        assertFalse(history.getBoolean("canRedo"))

        val state = JSONObject(requireNotNull(adapter.currentStateJson()))
        assertTrue(state.has("activeState"))
        assertEquals(2, state.getInt("documentVersion"))
    }

    @Test
    fun `structured error envelope fields`() {
        val adapter = makeAdapter(
            """{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"""
        )
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }
        assertNull(adapter.insertText("x", 0))
        val error = errors.last()
        assertEquals("boundary", error.domain)
        assertEquals("MUTATION_REJECTED", error.code)
        assertTrue(error.message.isNotEmpty())
        assertNotNull(error.requestId)
        assertTrue(error.requestId!!.toULongOrNull() != null)
        assertNull(error.operationIndex)
        assertNull(error.limit)
        assertNull(error.actual)
    }

    @Test
    fun `request envelopes carry version request id and base revision`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        backend.calls.clear()
        adapter.insertText("X", 2)
        // The fake asserts the envelope on admission; this test pins the
        // exact revision arithmetic: base revision is the pre-commit one.
        val session = sessionOf(adapter)
        assertEquals(2uL, session.revision)
        assertEquals(2uL, adapter.baseDocumentRevision)
    }

    @Test
    fun `request id exhaustion emits max once then rejects locally`() {
        val adapter = makeAdapter()
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }
        adapter.setNextRequestIdForTesting(ULong.MAX_VALUE - 1u)

        val backendCallsBefore = adapter.backendEnvelopeCallCountForTesting
        assertNotNull(adapter.setContentHtml("<p>max</p>"))
        assertEquals(ULong.MAX_VALUE, adapter.lastRequestIdForTesting)
        assertEquals(backendCallsBefore + 1, adapter.backendEnvelopeCallCountForTesting)

        assertNull(adapter.setContentHtml("<p>must not reach backend</p>"))
        assertEquals(ULong.MAX_VALUE, adapter.lastRequestIdForTesting)
        assertEquals(backendCallsBefore + 1, adapter.backendEnvelopeCallCountForTesting)
        assertEquals("boundary", errors.last().domain)
        assertEquals("CONFIG_INVALID", errors.last().code)
        assertEquals(ULong.MAX_VALUE.toString(), errors.last().requestId)
        assertEquals("max", documentText(adapter))
    }

}
