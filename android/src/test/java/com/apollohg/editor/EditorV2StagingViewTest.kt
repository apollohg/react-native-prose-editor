package com.apollohg.editor

import android.view.inputmethod.EditorInfo
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
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * v2 view integration tests: the views drive the v2 adapter (backed by the
 * fake v2 engine) for typing, composition, correction, selection, toolbar,
 * undo/redo, read-only, and lifecycle races — while transient composing text
 * provably never reaches the engine.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorV2StagingViewTest {

    private val localEmptyConfig = """{"initialization":{"type":"localEmpty"}}"""
    private lateinit var backend: FakeEditorV2Backend
    private lateinit var editText: EditorEditText
    private lateinit var adapter: EditorV2Adapter

    @Before
    fun setUp() {
        backend = FakeEditorV2Backend()
        adapter = attachCreatedEditor(localEmptyConfig)
        editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.editorId = 4242L
        editText.v2Driver = adapter
        adapter.setContentHtml("<p>Hello</p>")?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        assertEquals("Hello", editText.text.toString())
    }

    private fun attachCreatedEditor(configJson: String): EditorV2Adapter {
        val editorId = when (val created = backend.create(configJson, null)) {
            is EditorV2CallResult.Ok -> JSONObject(created.value).getString("editorId")
            is EditorV2CallResult.Err -> throw AssertionError("create failed: ${created.error}")
        }
        return EditorV2Adapter.attach(backend, editorId, roomBound = false)
            ?: throw AssertionError("created editor could not be attached")
    }

    private fun documentText(): String {
        val result = backend.getDocumentJson(adapter.editorId) as EditorV2CallResult.Ok
        return FakeEditorV2Backend.documentTextOf(JSONObject(result.value))
    }

    private fun adoptExternalRender(adapter: EditorV2Adapter, snapshot: String): String? =
        adapter.adoptExternalRender(snapshot)

    private fun atomicRenderSnapshot(text: String, revision: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                org.json.JSONArray().put(
                    org.json.JSONArray()
                        .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph").put("depth", 0))
                        .put(JSONObject().put("type", "textRun").put("text", text).put("marks", org.json.JSONArray()))
                        .put(JSONObject().put("type", "blockEnd"))
                )
            )
            .put("renderPatch", JSONObject.NULL)
            .put("selection", JSONObject().put("type", "text").put("anchor", 1).put("head", 1).put("anchorScalar", 0).put("headScalar", 0))
            .put(
                "activeState",
                JSONObject()
                    .put("marks", JSONObject())
                    .put("markAttrs", JSONObject())
                    .put("nodes", JSONObject().put("paragraph", true))
                    .put("commands", JSONObject())
                    .put("allowedMarks", org.json.JSONArray().put("bold"))
                    .put("insertableNodes", org.json.JSONArray().put("hardBreak"))
            )
            .put("historyState", JSONObject().put("canUndo", true).put("canRedo", false))
            .put("documentVersion", revision)
            .put("stateRevision", revision)
            .put("scalarLength", text.length)
            .toString()

    @Test
    fun `view mutations route through the v2 adapter`() {
        backend.calls.clear()
        editText.setSelection(5)
        editText.handleTextCommit("X")
        assertEquals("HelloX", documentText())
        assertEquals("HelloX", editText.text.toString())
        assertEquals(1, backend.calls.count { it == "applyInput" })
    }

    @Test
    fun `external N plus one render reaches the view before its first native key commits N plus two`() {
        val session = backend.sessions.getValue(adapter.editorId)
        session.text.insert(0, "EXT")
        session.revision += 1u
        val adopted = adoptExternalRender(adapter, atomicRenderSnapshot("EXTHello", session.revision.toString()))
        assertNotNull(adopted)
        editText.applyUpdateJSON(adopted!!, notifyListener = false)
        editText.setSelection(0)

        editText.handleTextCommit("K")

        assertEquals("KEXTHello", documentText())
        assertEquals(3uL, adapter.baseDocumentRevision)
    }

    @Test
    fun `composing text never reaches the engine and finish commits once`() {
        editText.setSelection(5)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        backend.calls.clear()

        inputConnection.setComposingText(" wor", 1)
        inputConnection.setComposingText(" worl", 1)
        assertTrue(
            "transient composing state must stay native-only: $backend.calls",
            backend.calls.none { it == "applyInput" || it == "applyCommand" },
        )
        val revisionBefore = adapter.baseDocumentRevision
        inputConnection.finishComposingText()
        assertEquals(revisionBefore + 1u, adapter.baseDocumentRevision)
        assertEquals(1, backend.calls.count { it == "applyInput" })
    }

    @Test
    fun `gboard style correction commits as one transaction`() {
        editText.setSelection(5)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        inputConnection.setComposingText(" teh", 1)
        backend.calls.clear()

        inputConnection.commitText(" the", 1)

        assertEquals(1, backend.calls.count { it == "applyInput" })
        assertEquals("Hello the", documentText())
    }

    @Test
    fun `delete and return route through the adapter`() {
        editText.setSelection(5)
        backend.calls.clear()
        editText.handleDelete(1, 0)
        assertEquals("Hell", documentText())

        editText.setSelection(4)
        editText.handleReturnKey()
        assertEquals("Hell\n", documentText())
    }

    @Test
    fun `undo and redo flow through toolbar path`() {
        editText.setSelection(5)
        editText.handleTextCommit("!")
        assertEquals("Hello!", documentText())

        editText.performToolbarUndo()
        assertEquals("Hello", documentText())
        editText.performToolbarRedo()
        assertEquals("Hello!", documentText())
    }

    @Test
    fun `read only rejects view edits atomically and keeps selection`() {
        val readOnly = attachCreatedEditor(
            """{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"""
        )
        val view = EditorEditText(RuntimeEnvironment.getApplication())
        view.editorId = 4343L
        view.v2Driver = readOnly
        readOnly.setContentHtml("<p>ab</p>")?.let { view.applyUpdateJSON(it, notifyListener = false) }
        val errors = mutableListOf<EditorV2Error>()
        readOnly.onAutonomousError = { errors.add(it) }

        view.setSelection(2)
        view.handleTextCommit("z")
        view.handleDelete(1, 0)

        assertEquals("ab", documentTextOf(readOnly))
        assertEquals("ab", view.text.toString())
        assertEquals("MUTATION_REJECTED", errors.last().code)
    }

    private fun documentTextOf(adapter: EditorV2Adapter): String {
        val result = backend.getDocumentJson(adapter.editorId) as EditorV2CallResult.Ok
        return FakeEditorV2Backend.documentTextOf(JSONObject(result.value))
    }

    @Test
    fun `destroy mid composition is a structured failure without partial commit`() {
        editText.setSelection(5)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        inputConnection.setComposingText("xyz", 1)
        val errors = mutableListOf<EditorV2Error>()
        adapter.onAutonomousError = { errors.add(it) }

        adapter.destroy()

        inputConnection.finishComposingText()

        assertEquals("ENGINE_DESTROYED", errors.last().code)
    }

    @Test
    fun `stale revision refreshes the view from rust state`() {
        // Sync the caret while fresh; a remote-side change then advances the
        // session behind the view's back, so the next commit goes stale.
        editText.setSelection(5)
        val session = backend.sessions.getValue(adapter.editorId)
        session.text.append(" REMOTE")
        session.revision += 1u

        editText.handleTextCommit("X")

        // The stale commit is never retried; the view refreshes from Rust.
        assertEquals("Hello REMOTE", documentText())
        assertEquals("Hello REMOTE", editText.text.toString())
    }

    @Test
    fun `unchanged refresh skips render on the existing bridge`() {
        editText.setSelection(5)
        editText.captureApplyUpdateTraceForTesting = true
        // A selection-only sync produces no document change; applying the
        // refreshed state must hit the render-skip path (no full re-render).
        adapter.refreshFromRustState(intArrayOf(2, 2))?.let {
            editText.applyUpdateJSON(it, notifyListener = false)
        }
        val trace = editText.lastApplyUpdateTrace()
        assertNotNull(trace)
        assertTrue("identical render blocks must skip the re-render", trace!!.skippedRender)
    }

    @Test
    fun `selection sync reports rust mapped doc positions`() {
        var reported: Pair<Int, Int>? = null
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) {
                reported = anchor to head
            }
            override fun onEditorUpdate(updateJson: String) {}
        }
        editText.setSelection(3)
        assertNotNull(reported)
        assertEquals(4, reported!!.first)
        assertEquals(4, reported!!.second)
    }
}
