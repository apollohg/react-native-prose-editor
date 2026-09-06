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

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorV2AdapterCollaborationTest : EditorV2AdapterTestFixture() {
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
    fun `atomic external snapshot accepts atomId only on voidBlock`() {
        fun adopt(element: JSONObject): String? {
            val adapter = makeAdapter()
            val snapshot = JSONObject(atomicRenderSnapshot("base", "7", selectionScalar = 1))
                .put(
                    "renderBlocks",
                    org.json.JSONArray().put(org.json.JSONArray().put(element))
                )
                .toString()
            return adoptExternalRender(adapter, snapshot)
        }

        assertNotNull(
            adopt(
                JSONObject()
                    .put("type", "voidBlock")
                    .put("nodeType", "counterCard")
                    .put("docPos", 1)
                    .put("atomId", "y1-2")
            )
        )
        assertNull(
            adopt(
                JSONObject()
                    .put("type", "voidBlock")
                    .put("nodeType", "counterCard")
                    .put("docPos", 1)
                    .put("atomId", 7)
            )
        )
        assertNull(
            adopt(
                JSONObject()
                    .put("type", "voidInline")
                    .put("nodeType", "hardBreak")
                    .put("docPos", 1)
                    .put("atomId", "y1-2")
            )
        )
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
}
