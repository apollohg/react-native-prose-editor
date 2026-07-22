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

    private fun makeRoomAdapter(): EditorV2Adapter {
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
        ) ?: throw AssertionError("created room editor could not be attached")
        createdAdapters.add(adapter)
        return adapter
    }

    private fun renderedText(updateJson: String?): String {
        assertNotNull(updateJson)
        val update = JSONObject(updateJson)
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
        val parsed = JSONObject(update)
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
        assertEquals(3, mapping!![0])
        assertEquals(3, mapping[1])

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
    fun `replacement commit is one transaction`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>teh</p>")
        val revisionBefore = adapter.baseDocumentRevision
        backend.calls.clear()

        val update = adapter.replaceTextRange(0, 3, "the")
        assertEquals("the", renderedText(update))
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
        assertEquals(4, mapping!![0])
        assertEquals(2, mapping[1])

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

        val split = adapter.splitBlockAt(1)
        assertNotNull(split)
        assertEquals("a\n", renderedText(split))

        adapter.setContentHtml("<p>abcd</p>")
        val update = adapter.deleteAndSplit(1, 3)
        assertEquals("a\nd", renderedText(update))
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
    fun `render update carries blocks active state and overridden engine facts`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        backend.calls.clear()
        val update = JSONObject(adapter.insertText("c", 2))
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
        // Native-originated edit: no selection key (the view keeps its caret).
        assertFalse(update.has("selection"))
        assertTrue(backend.calls.contains("renderUpdate"))
    }

    @Test
    fun `render update mirrors scalar selection to doc positions`() {
        val adapter = makeAdapter()
        val update = JSONObject(adapter.setContentHtml("<p>ab</p><p>cd</p>"))
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
        val update = JSONObject(adapter.currentStateJson())
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
        assertEquals(2, adapter.scalarPositionForDoc(5))
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
            "splitBlock" to { adapter.splitBlockAt(1) },
            "deleteAndSplit" to { adapter.deleteAndSplit(0, 1) },
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
        assertEquals(2, mapping!![0])

        // Controlled content still passes.
        val replaced = adapter.setContentJson(
            "{\"type\":\"doc\",\"content\":[{\"type\":\"paragraph\",\"content\":[{\"type\":\"text\",\"text\":\"api\"}]}]}"
        )
        assertEquals("api", renderedText(replaced))
    }

    // MARK: stale revision recovery

    @Test
    fun `revision mismatch refreshes from rust state and never retries`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>base</p>")

        // Sync the caret while fresh, then externally advance the same
        // session so the adapter's tracked base goes stale.
        adapter.syncSelection(0, 0)
        val session = sessionOf(adapter)
        session.text.insert(0, "EXT")
        session.revision += 1u

        backend.calls.clear()
        val update = adapter.insertText("NORETRY", 0)
        assertNotNull("a stale op resolves into a refresh update", update)
        assertEquals("the stale op must not be retried", 1L, backend.calls.count { it == "applyInput" }.toLong())
        assertTrue(backend.calls.contains("getState"))
        assertEquals("EXTbase", renderedText(update))
        assertEquals("EXTbase", documentText(adapter))

        val recovered = adapter.insertText("ok", 0)
        assertEquals("okEXTbase", renderedText(recovered))
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

    // MARK: drain ping

    @Test
    fun `local commit drives drain ping on room bound session`() {
        val adapter = makeRoomAdapter()
        val frames = mutableListOf<ByteArray>()

        adapter.outboundFrameSink = { frames.add(it) }
        adapter.insertText("x", 0)
        assertTrue("no live generation: the ping must not fire", frames.isEmpty())

        adapter.collaborationGeneration = "7"
        val update = adapter.insertText("x", 0)
        assertNotNull(update)
        assertTrue("a local commit on a live generation drains the outbox", frames.isNotEmpty())
        frames.forEach { assertTrue(it.isNotEmpty()) }
    }

    @Test
    fun `drain ping skipped for local only session`() {
        val adapter = makeAdapter()
        val frames = mutableListOf<ByteArray>()
        adapter.outboundFrameSink = { frames.add(it) }
        adapter.collaborationGeneration = "1"
        adapter.insertText("x", 0)
        assertTrue(frames.isEmpty())
    }

    // MARK: synthesized update contract

    @Test
    fun `synthesized update carries rust state not fabricated state`() {
        val adapter = makeAdapter()
        adapter.setContentHtml("<p>ab</p>")
        val update = JSONObject(adapter.insertText("c", 2))
        assertEquals(2, update.getInt("documentVersion"))
        val history = update.getJSONObject("historyState")
        assertTrue(history.getBoolean("canUndo"))
        assertFalse(history.getBoolean("canRedo"))

        val state = JSONObject(adapter.currentStateJson())
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
