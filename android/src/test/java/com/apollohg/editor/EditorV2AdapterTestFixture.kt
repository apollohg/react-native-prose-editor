package com.apollohg.editor
import android.text.Spanned
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

internal abstract class EditorV2AdapterTestFixture {
    protected val localEmptyConfig = """{"initialization":{"type":"localEmpty"}}"""
    protected lateinit var backend: FakeEditorV2Backend
    protected val createdAdapters = mutableListOf<EditorV2Adapter>()

    @Before
    fun setUp() {
        backend = FakeEditorV2Backend()
    }

    protected fun makeAdapter(configJson: String = localEmptyConfig): EditorV2Adapter {
        val adapter = EditorV2Adapter.attach(
            backend,
            createEditorId(configJson),
            roomBound = false,
        ) ?: throw AssertionError("created editor could not be attached")
        createdAdapters.add(adapter)
        return adapter
    }

    protected fun createEditorId(configJson: String, snapshotState: ByteArray? = null): String = when (
        val created = backend.create(configJson, snapshotState)
    ) {
        is EditorV2CallResult.Ok -> JSONObject(created.value).getString("editorId")
        is EditorV2CallResult.Err -> throw AssertionError("create failed: ${created.error}")
    }

    protected fun makeRoomAdapter(
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

    protected fun renderedText(updateJson: String?): String {
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

    protected fun documentText(adapter: EditorV2Adapter): String {
        val result = backend.getDocumentJson(adapter.editorId) as EditorV2CallResult.Ok
        return FakeEditorV2Backend.documentTextOf(JSONObject(result.value))
    }

    protected fun sessionOf(adapter: EditorV2Adapter): FakeEditorV2Backend.FakeSession =
        backend.sessions.getValue(adapter.editorId)

    /** A frozen v2 atomic render snapshot, deliberately independent of the fake's legacy payload. */
    protected fun atomicRenderSnapshot(text: String, revision: String, selectionScalar: Int = 0): String =
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

    protected fun imageAtomicRenderSnapshot(revision: String, width: Int): String =
        JSONObject()
            .put(
                "renderBlocks",
                org.json.JSONArray()
                    .put(
                        org.json.JSONArray()
                            .put(
                                JSONObject()
                                    .put("type", "blockStart")
                                    .put("nodeType", "paragraph")
                                    .put("depth", 0)
                            )
                            .put(
                                JSONObject()
                                    .put("type", "textRun")
                                    .put("text", "Hello")
                                    .put("marks", org.json.JSONArray())
                            )
                            .put(JSONObject().put("type", "blockEnd"))
                    )
                    .put(
                        org.json.JSONArray().put(
                            JSONObject()
                                .put("type", "voidBlock")
                                .put("nodeType", "image")
                                .put("docPos", 7)
                                .put(
                                    "attrs",
                                    JSONObject()
                                        .put("src", "https://example.com/cat.png")
                                        .put("width", width)
                                        .put("height", 80)
                                )
                        )
                    )
            )
            .put("renderPatch", JSONObject.NULL)
            .put(
                "selection",
                JSONObject()
                    .put("type", "node")
                    .put("pos", 7)
                    .put("posScalar", 6)
            )
            .put(
                "activeState",
                JSONObject()
                    .put("marks", JSONObject())
                    .put("markAttrs", JSONObject())
                    .put("nodes", JSONObject().put("image", true))
                    .put("commands", JSONObject())
                    .put("allowedMarks", org.json.JSONArray())
                    .put("insertableNodes", org.json.JSONArray())
            )
            .put("historyState", JSONObject().put("canUndo", true).put("canRedo", false))
            .put("documentVersion", revision)
            .put("stateRevision", revision)
            .put("scalarLength", 7)
            .put("documentIsEmpty", false)
            .toString()

    protected fun adoptExternalRender(adapter: EditorV2Adapter, snapshot: String): String? =
        adapter.adoptExternalRender(snapshot)

    // MARK: construction

    // MARK: commit semantics

    // MARK: Task 16B render accessor (probe replacement)

    // MARK: read-only atomicity

    // MARK: stale revision recovery

    // MARK: undo/redo

    // MARK: lifecycle races

    // MARK: synthesized update contract
}
