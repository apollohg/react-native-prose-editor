package com.apollohg.editor
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.os.Looper
import android.provider.Settings
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.InputType
import android.text.style.AbsoluteSizeSpan
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.RobolectricTestRunner
import org.robolectric.Robolectric
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputConnectionExternalCompositionTest : EditorInputConnectionTestFixture() {
    @Test
    fun `spellcheck composition excludes a generated list marker`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyRenderJSON(
            """
            [
                {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}},
                {"type":"blockStart","nodeType":"paragraph","depth":1},
                {"type":"textRun","text":"Try typing","marks":[]},
                {"type":"blockEnd"},
                {"type":"blockEnd"}
            ]
            """.trimIndent()
        )
        editText.editorId = 1
        val bodyStart = editText.text.toString().indexOf("Try typing")
        editText.setSelection(bodyStart + 2)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        val originalText = editText.text.toString()
        val spellcheckText = originalText.substring(0, bodyStart + 3)

        assertTrue(inputConnection.setComposingRegion(0, bodyStart + 3))
        assertTrue(inputConnection.setComposingText(spellcheckText, 0))

        assertEquals(originalText, editText.text.toString())
        assertTrue(editText.renderedRangeContainsGeneratedStructure(0, bodyStart))
        assertEquals(bodyStart to bodyStart + 3, editText.compositionReplacementRange())
        assertEquals(bodyStart, BaseInputConnection.getComposingSpanStart(editText.text!!))
        assertEquals(bodyStart + 3, BaseInputConnection.getComposingSpanEnd(editText.text!!))

        assertTrue(inputConnection.commitText("Dry", 1))

        assertEquals(Triple(bodyStart, bodyStart + 3, "Dry"), replacement)
        assertTrue(editText.renderedRangeContainsGeneratedStructure(0, bodyStart))
    }

    @Test
    fun `external composition updates visible text without mutating Rust`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(backend, editorId, roomBound = false)!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>arrival</p>")
            ?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        editText.setSelection(0, 7)

        editText.beginExternalTextComposition("speech-1")
        editText.updateExternalTextComposition("speech-1", "on arrival")
        editText.updateExternalTextComposition("speech-1", "O/A")

        assertEquals("O/A", editText.text.toString())
        assertEquals("arrival", backend.sessions.getValue(editorId).text.toString())
    }

    @Test
    fun `IME finish composing does not end external composition`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")

        assertTrue(inputConnection.finishComposingText())
        assertTrue(listener.externalCompositionEnds.isEmpty())
        assertEquals("arrival", harness.backend.sessions.getValue(harness.editorId).text.toString())

        val update = JSONObject(
            harness.editText.updateExternalTextComposition("speech-1", "final draft")
        )
        assertEquals("active", update.getString("type"))
        assertEquals("final draft", harness.editText.text.toString())
    }

    @Test
    fun `external composition updates placeholder from provisional text`() {
        val harness = externalCompositionHarness("")
        harness.editText.placeholderText = "Type here"

        assertTrue(harness.editText.shouldDisplayPlaceholderForTesting())

        harness.editText.beginExternalTextComposition("placeholder")
        harness.editText.updateExternalTextComposition("placeholder", "draft")
        assertFalse(harness.editText.shouldDisplayPlaceholderForTesting())

        harness.editText.updateExternalTextComposition("placeholder", "")
        assertTrue(harness.editText.shouldDisplayPlaceholderForTesting())

        harness.editText.updateExternalTextComposition("placeholder", "draft")
        assertFalse(harness.editText.shouldDisplayPlaceholderForTesting())

        harness.editText.cancelExternalTextComposition("placeholder", "consumer")
        assertTrue(harness.editText.shouldDisplayPlaceholderForTesting())
    }

    @Test
    fun `external composition commit is exact once and uses final text`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        harness.backend.calls.clear()

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")
        val resultJson = harness.editText.commitExternalTextComposition("speech-1", "O/A")
        val duplicate = harness.editText.commitExternalTextComposition("speech-1", "ignored")
        val lateUpdate = JSONObject(
            harness.editText.updateExternalTextComposition("speech-1", "ignored")
        )

        val result = JSONObject(resultJson)
        assertEquals("O/A", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals("committed", result.getString("outcome"))
        assertEquals("consumer", result.getString("cause"))
        assertEquals("O/A", result.getString("text"))
        assertEquals(resultJson, duplicate)
        assertEquals("EXTERNAL_COMPOSITION_ENDED", lateUpdate.errorCode())
        assertEquals(1, harness.backend.calls.count { it == "applyNativeIntent" })
        assertEquals(listOf(resultJson), listener.externalCompositionEnds)
        assertEquals(listOf("update", "external"), listener.events)
    }

    @Test
    fun `external composition cancel restores authorized text and selection once`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        harness.backend.calls.clear()

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "O/A")
        val resultJson = harness.editText.cancelExternalTextComposition("speech-1", "consumer")
        val duplicate = harness.editText.cancelExternalTextComposition("speech-1", "consumer")

        val result = JSONObject(resultJson)
        assertEquals("arrival", harness.editText.text.toString())
        assertEquals(0, harness.editText.selectionStart)
        assertEquals(7, harness.editText.selectionEnd)
        assertEquals("arrival", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals("cancelled", result.getString("outcome"))
        assertEquals("consumer", result.getString("cause"))
        assertEquals(resultJson, duplicate)
        assertEquals(0, harness.backend.calls.count { it == "applyCommand" })
        assertEquals(listOf(resultJson), listener.externalCompositionEnds)
    }

    @Test
    fun `external composition no op final text does not add undo state`() {
        val harness = realExternalCompositionHarness("arrival")
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(0, 7)
            val revisionBefore = harness.adapter.baseDocumentRevision
            val canUndoBefore = harness.adapter.historyCanUndo()
            harness.editText.beginExternalTextComposition("speech-1")
            harness.editText.updateExternalTextComposition("speech-1", "draft")
            val requestIdBefore = harness.adapter.lastRequestIdForTesting

            val resultJson = harness.editText.commitExternalTextComposition("speech-1", "arrival")
            val result = JSONObject(resultJson)
            val duplicate = harness.editText.commitExternalTextComposition("speech-1", "ignored")

            assertEquals("committed", result.getString("outcome"))
            assertEquals(revisionBefore, harness.adapter.baseDocumentRevision)
            assertEquals(canUndoBefore, harness.adapter.historyCanUndo())
            assertEquals("arrival", harness.editText.text.toString())
            assertEquals(resultJson, duplicate)
            assertEquals(requestIdBefore?.plus(1u), harness.adapter.lastRequestIdForTesting)
            assertTrue(listener.receivedUpdates.isEmpty())
            assertEquals(listOf("external"), listener.events)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `external composition replacement preserves Unicode ranges`() {
        data class Case(val source: String, val start: Int, val end: Int, val expected: String)
        val cases = listOf(
            Case("Cafe\u0301", 3, 5, "CafZ"),
            Case("abc אבג def", 4, 7, "abc Z def")
        )

        val emoji = realExternalCompositionHarness("A🙂B")
        try {
            emoji.editText.setSelection(1, 3)
            emoji.editText.beginExternalTextComposition("speech-emoji")
            emoji.editText.updateExternalTextComposition("speech-emoji", "draft")
            emoji.editText.commitExternalTextComposition("speech-emoji", "Z")
            assertEquals("AZB", emoji.editText.text.toString())
            assertEquals("<p>AZB</p>", emoji.adapter.documentHtml())
        } finally {
            emoji.adapter.destroy()
        }

        cases.forEachIndexed { index, case ->
            val harness = externalCompositionHarness(case.source)
            harness.editText.setSelection(case.start, case.end)
            harness.editText.beginExternalTextComposition("speech-$index")
            harness.editText.updateExternalTextComposition("speech-$index", "draft 🙂")
            harness.editText.commitExternalTextComposition("speech-$index", "Z")

            assertEquals(case.expected, harness.editText.text.toString())
            assertEquals(case.expected, harness.backend.sessions.getValue(harness.editorId).text.toString())
        }
    }

    @Test
    fun `external composition rejects unavailable and read only editors`() {
        val unavailable = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            editorId = 1
            setText("arrival")
            setSelection(0, 7)
        }
        val readOnly = externalCompositionHarness(
            initialText = "arrival",
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"readOnly":true}}"""
        )
        readOnly.editText.setSelection(0, 7)
        readOnly.editText.isEditable = false

        val unavailableResult = JSONObject(unavailable.beginExternalTextComposition("speech-1"))
        val readOnlyResult = JSONObject(readOnly.editText.beginExternalTextComposition("speech-2"))

        assertEquals("EXTERNAL_COMPOSITION_UNAVAILABLE", unavailableResult.errorCode())
        assertEquals("EXTERNAL_COMPOSITION_UNAVAILABLE", readOnlyResult.errorCode())
        assertEquals("arrival", unavailable.text.toString())
        assertEquals("arrival", readOnly.editText.text.toString())
    }

    @Test
    fun `external composition rejects non text selection without touching the view`() {
        val harness = realExternalCompositionHarness("arrival")
        try {
            val selectionResult = UniffiEditorV2Backend.setSelection(
                harness.editorId,
                JSONObject()
                    .put("version", 1)
                    .put("requestId", "991101")
                    .put("baseDocumentRevision", harness.adapter.baseDocumentRevision.toString())
                    .put("selection", JSONObject().put("type", "all"))
                    .toString()
            )
            assertTrue(selectionResult is EditorV2CallResult.Ok)
            harness.adapter.refreshFromRustState(null)
                ?.let { harness.editText.applyUpdateJSON(it, notifyListener = false) }
            val before = harness.editText.text.toString()

            val result = JSONObject(
                harness.editText.beginExternalTextComposition("speech-1")
            )

            assertEquals("EXTERNAL_COMPOSITION_SELECTION_INCOMPATIBLE", result.errorCode())
            assertEquals(before, harness.editText.text.toString())
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `external composition second session commits first`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")
        val second = JSONObject(harness.editText.beginExternalTextComposition("speech-2"))

        assertEquals("active", second.getString("type"))
        assertEquals("draft", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals(1, listener.externalCompositionEnds.size)
        assertEquals("consumer", JSONObject(listener.externalCompositionEnds.single()).getString("cause"))
    }

    @Test
    fun `external composition repeated active session id cannot mutate twice`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        harness.backend.calls.clear()

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")
        val repeated = JSONObject(
            harness.editText.beginExternalTextComposition("speech-1")
        )
        val duplicate = harness.editText.commitExternalTextComposition("speech-1", "ignored")

        assertEquals("EXTERNAL_COMPOSITION_ENDED", repeated.errorCode())
        assertEquals("draft", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals(listener.externalCompositionEnds.single(), duplicate)
        assertEquals(1, harness.backend.calls.count { it == "applyNativeIntent" })
        assertEquals(1, listener.externalCompositionEnds.size)
    }

    @Test
    fun `external composition toolbar command commits before interaction`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        harness.backend.calls.clear()

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")
        harness.editText.performToolbarToggleMark("bold")

        assertEquals("draft", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals("interaction", JSONObject(listener.externalCompositionEnds.single()).getString("cause"))
        assertEquals(2, harness.backend.calls.count { it == "applyNativeIntent" })
        assertEquals(1, harness.backend.calls.count { it == "applyCommand" })
    }

    @Test
    fun `external composition paste commits before interaction route`() {
        val context = RuntimeEnvironment.getApplication()
        val harness = externalCompositionHarness("Hello")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(5)
        var insertion: Pair<String, Int>? = null
        harness.editText.onInsertTextInRustForTesting = { text, scalar ->
            listener.events.add("paste")
            insertion = text to scalar
        }
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))
        harness.editText.beginExternalTextComposition("speech-paste")
        harness.editText.updateExternalTextComposition("speech-paste", "!")

        assertTrue(harness.editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals("X" to 6, insertion)
        assertEquals(
            "interaction",
            JSONObject(listener.externalCompositionEnds.single()).getString("cause")
        )
        assertEquals(listOf("update", "external", "paste"), listener.events)
    }

    @Test
    fun `external composition cut commits before interaction route`() {
        val context = RuntimeEnvironment.getApplication()
        val harness = realExternalCompositionHarness(
            initialText = "Hello",
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[0-9]"}}"""
        )
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(0, 5)
            var deletion: Pair<Int, Int>? = null
            harness.editText.onDeleteRangeInRustForTesting = { from, to ->
                listener.events.add("cut")
                deletion = from to to
            }
            harness.editText.beginExternalTextComposition("speech-cut")
            harness.editText.updateExternalTextComposition("speech-cut", "letters")

            assertTrue(harness.editText.onTextContextMenuItem(android.R.id.cut))

            assertEquals(0 to 5, deletion)
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            assertEquals("Hello", clipboard.primaryClip?.getItemAt(0)?.text?.toString())
            assertEquals(
                "interaction",
                JSONObject(listener.externalCompositionEnds.single()).getString("cause")
            )
            assertEquals(listOf("external", "cut"), listener.events)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `external composition accessibility set text commits before interaction route`() {
        val harness = externalCompositionHarness("Hello")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(5)
        var replacement: Triple<Int, Int, String>? = null
        harness.editText.onReplaceTextInRustForTesting = { from, to, text ->
            listener.events.add("setText")
            replacement = Triple(from, to, text)
        }
        val arguments = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "replacement"
            )
        }
        harness.editText.beginExternalTextComposition("speech-set-text")
        harness.editText.updateExternalTextComposition("speech-set-text", "!")

        assertTrue(
            harness.editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                arguments
            )
        )

        assertEquals(Triple(0, 6, "replacement"), replacement)
        assertEquals(
            "interaction",
            JSONObject(listener.externalCompositionEnds.single()).getString("cause")
        )
        assertEquals(listOf("update", "external", "setText"), listener.events)
    }

    @Test
    fun `external composition external update commits for document change`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")

        assertTrue(harness.editText.prepareForExternalEditorUpdate())

        assertEquals("draft", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals("documentChange", JSONObject(listener.externalCompositionEnds.single()).getString("cause"))
    }

    @Test
    fun `external composition remote update applies after document change commit`() {
        val harness = externalCompositionHarness("arrival", roomBound = true)
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")

        assertTrue(harness.editText.prepareForExternalEditorUpdate())
        val session = harness.backend.sessions.getValue(harness.editorId)
        session.text.append(" remote")
        session.anchor = session.text.length
        session.head = session.text.length
        session.revision += 1u
        harness.adapter.currentStateJson()
            ?.let { harness.editText.applyUpdateJSON(it, notifyListener = false) }

        assertEquals("draft remote", harness.editText.text.toString())
        assertEquals(1, listener.externalCompositionEnds.size)
        assertEquals("documentChange", JSONObject(listener.externalCompositionEnds.single()).getString("cause"))
    }

    @Test
    fun `external composition lifecycle discard cancels without mutation`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        val session = harness.backend.sessions.getValue(harness.editorId)
        val revisionBefore = session.revision
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")

        harness.editText.discardTransientNativeInputForEditorRebind()

        assertEquals("arrival", harness.editText.text.toString())
        assertEquals(0, harness.editText.selectionStart)
        assertEquals(7, harness.editText.selectionEnd)
        assertEquals("arrival", session.text.toString())
        assertEquals(revisionBefore, session.revision)
        assertEquals("lifecycle", JSONObject(listener.externalCompositionEnds.single()).getString("cause"))
    }

    @Test
    fun `external composition cancel adopts current authorized driver state`() {
        val harness = externalCompositionHarness("abc", roomBound = true)
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(1, 2)
        val session = harness.backend.sessions.getValue(harness.editorId)
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "X")
        session.text.insert(0, "Z")
        session.anchor = 1
        session.head = 1
        session.revision += 1u
        val revisionBeforeCancel = session.revision
        val undoBefore = session.undoStack.size
        val outboxBefore = session.outbox.size
        harness.backend.calls.clear()

        val resultJson = harness.editText.cancelExternalTextComposition("speech-1", "consumer")
        val result = JSONObject(resultJson)

        assertEquals("cancelled", result.getString("outcome"))
        assertEquals("Zabc", harness.editText.text.toString())
        assertEquals("Zabc", session.text.toString())
        assertEquals(revisionBeforeCancel, session.revision)
        assertEquals(undoBefore, session.undoStack.size)
        assertEquals(outboxBefore, session.outbox.size)
        assertEquals(0, harness.backend.calls.count { it == "applyNativeIntent" })
        assertEquals(0, harness.backend.calls.count { it == "applyCommand" })
        assertTrue(listener.receivedUpdates.isEmpty())
        assertEquals(listOf(resultJson), listener.externalCompositionEnds)
    }

    @Test
    fun `external composition input trait discard cancels for lifecycle`() {
        val harness = externalCompositionHarness("arrival")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(0, 7)
        val session = harness.backend.sessions.getValue(harness.editorId)
        val revisionBefore = session.revision
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")

        harness.editText.setKeyboardType("email-address")

        assertEquals("arrival", harness.editText.text.toString())
        assertEquals(0, harness.editText.selectionStart)
        assertEquals(7, harness.editText.selectionEnd)
        assertEquals("arrival", session.text.toString())
        assertEquals(revisionBefore, session.revision)
        assertEquals("lifecycle", JSONObject(listener.externalCompositionEnds.single()).getString("cause"))
    }

    @Test
    fun `external composition invalid position epoch cancels without local mutation`() {
        val harness = externalCompositionHarness("abc", roomBound = true)
        val listener = RecordingEditorListener()
        val adapterErrors = mutableListOf<EditorV2Error>()
        harness.adapter.onAutonomousError = { adapterErrors += it }
        harness.editText.editorListener = listener
        harness.editText.setSelection(1, 2)
        val session = harness.backend.sessions.getValue(harness.editorId)
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "X")
        session.positionEpochs.clear()
        val revisionBeforeCommit = session.revision
        val undoBefore = session.undoStack.size
        val outboxBefore = session.outbox.size

        val resultJson = harness.editText.commitExternalTextComposition("speech-1", "Y")
        val result = JSONObject(resultJson)

        assertEquals("cancelled", result.getString("outcome"))
        assertEquals("EXTERNAL_COMPOSITION_COMMIT_FAILED", result.errorCode())
        assertEquals("abc", harness.editText.text.toString())
        assertEquals("abc", session.text.toString())
        assertEquals(revisionBeforeCommit, session.revision)
        assertEquals(undoBefore, session.undoStack.size)
        assertEquals(outboxBefore, session.outbox.size)
        assertTrue(listener.receivedUpdates.isEmpty())
        assertEquals(listOf(resultJson), listener.externalCompositionEnds)
        assertEquals(1, adapterErrors.size)
        assertEquals("POSITION_EPOCH_INVALID", adapterErrors.single().code)
    }

    @Test
    fun `external composition remains committed after post mutation render recovery`() {
        val harness = externalCompositionHarness("abc")
        val listener = RecordingEditorListener()
        harness.editText.editorListener = listener
        harness.editText.setSelection(1, 2)
        harness.editText.beginExternalTextComposition("speech-render-recovery")
        harness.editText.updateExternalTextComposition("speech-render-recovery", "Y")
        harness.backend.nextRenderUpdateResult = EditorV2CallResult.Err(
            EditorV2Error("render", "RENDER_FAILED", "transient"),
        )

        val resultJson = harness.editText.commitExternalTextComposition(
            "speech-render-recovery",
            "Y",
        )

        assertEquals("committed", JSONObject(resultJson).getString("outcome"))
        assertEquals("aYc", harness.editText.text.toString())
        assertEquals("aYc", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals(1, listener.receivedUpdates.size)
        assertEquals(listOf(resultJson), listener.externalCompositionEnds)
    }

    @Test
    fun `external composition valid input filter commits accepted partial text`() {
        val harness = realExternalCompositionHarness(
            initialText = "ab",
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[0-9]"}}"""
        )
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(0, 2)
            harness.editText.beginExternalTextComposition("speech-filter-partial")
            harness.editText.updateExternalTextComposition("speech-filter-partial", "a1b2")

            val result = JSONObject(
                harness.editText.commitExternalTextComposition(
                    "speech-filter-partial",
                    "a1b2"
                )
            )

            assertEquals("committed", result.getString("outcome"))
            assertEquals("12", harness.editText.text.toString())
            assertEquals("<p>12</p>", harness.adapter.documentHtml())
            assertEquals(1, listener.receivedUpdates.size)
            assertEquals(1, listener.externalCompositionEnds.size)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `external composition valid input filter commits fully filtered no op`() {
        val harness = realExternalCompositionHarness(
            initialText = "12",
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[0-9]"}}"""
        )
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(0, 2)
            val revisionBefore = harness.adapter.baseDocumentRevision
            val canUndoBefore = harness.adapter.historyCanUndo()
            harness.editText.beginExternalTextComposition("speech-filter-full")
            harness.editText.updateExternalTextComposition("speech-filter-full", "letters")

            val result = JSONObject(
                harness.editText.commitExternalTextComposition(
                    "speech-filter-full",
                    "letters"
                )
            )

            assertEquals("committed", result.getString("outcome"))
            assertEquals("12", harness.editText.text.toString())
            assertEquals("<p>12</p>", harness.adapter.documentHtml())
            assertEquals(revisionBefore, harness.adapter.baseDocumentRevision)
            assertEquals(canUndoBefore, harness.adapter.historyCanUndo())
            assertTrue(listener.receivedUpdates.isEmpty())
            assertEquals(1, listener.externalCompositionEnds.size)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `external composition invalid cancel cause keeps session active`() {
        val harness = externalCompositionHarness("arrival")
        harness.editText.setSelection(0, 7)
        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")

        val invalid = JSONObject(
            harness.editText.cancelExternalTextComposition("speech-1", "interaction")
        )
        val committed = JSONObject(
            harness.editText.commitExternalTextComposition("speech-1", "final")
        )

        assertEquals("EXTERNAL_COMPOSITION_CANCEL_CAUSE_INVALID", invalid.errorCode())
        assertEquals("committed", committed.getString("outcome"))
        assertEquals("final", harness.editText.text.toString())
    }

    @Test
    fun `IME selection and composition ranges map around invisible placeholders`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("\u200Bab\u200Bcd")
        editText.setSelection(1)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.setSelection(0, 2))
        assertEquals(1, editText.selectionStart)
        assertEquals(3, editText.selectionEnd)

        assertTrue(inputConnection.setSelection(4, 2))
        assertEquals(6, editText.selectionStart)
        assertEquals(4, editText.selectionEnd)

        assertTrue(inputConnection.setComposingRegion(0, 2))
        assertEquals(1, BaseInputConnection.getComposingSpanStart(editText.text!!))
        assertEquals(3, BaseInputConnection.getComposingSpanEnd(editText.text!!))
    }

    @Test
    fun `styled IME queries rebuild after composing spans change`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(0, 3)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))
        BaseInputConnection.setComposingSpans(editText.text!!)

        editText.applyTransientComposingTextStyleForEditor()

        val selected = requireNotNull(
            inputConnection.getSelectedText(InputConnection.GET_TEXT_WITH_STYLES),
        ) as Spanned
        assertEquals(1, selected.getSpans(0, selected.length, AbsoluteSizeSpan::class.java).size)
    }

    @Test
    fun `Samsung composing text at rendered line start is sentence capitalized`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)
        editText.editorId = 1

        withDefaultInputMethod(context, "com.samsung.android.honeyboard/.service.HoneyBoardService") {
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)

            assertTrue(inputConnection!!.setComposingText("test", 1))

            assertEquals("Test", editText.composingTextForEditor())
            assertTrue(
                editText.imeTraceSnapshotForTesting().any {
                    it.contains("samsungSentenceCapsFallback")
                }
            )
        }
    }

    @Test
    fun `external clear keeps same editor input connection accepting composition`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("sent"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.applyUpdateJSON(
            renderUpdateJson("\u200B"),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertTrue(inputConnection!!.setComposingText("f", 1))
        assertTrue(editText.text?.toString()?.contains("f") == true)
    }

    @Test
    fun `input trait change during active composition restores authorized text before fresh input`() {
        val traitChanges: List<(EditorEditText) -> Unit> = listOf(
            { it.setAutoCorrect(false) },
            { it.setKeyboardType("email-address") }
        )

        for (changeTrait in traitChanges) {
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
            editText.setSelection(6)
            editText.editorId = 1
            editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

            var insertedText: String? = null
            var insertedScalar: Int? = null
            editText.onInsertTextInRustForTesting = { text, scalar ->
                insertedText = text
                insertedScalar = scalar
            }

            val oldConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(oldConnection)
            assertTrue(oldConnection!!.setComposingText("brave ", 1))
            assertEquals("Hello brave world", editText.text?.toString())

            changeTrait(editText)

            assertEquals("Hello world", editText.text?.toString())
            assertEquals(6, editText.selectionStart)
            assertEquals(6, editText.selectionEnd)

            assertTrue(oldConnection.commitText("brave ", 1))
            assertNull(insertedText)
            assertNull(insertedScalar)

            val freshConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(freshConnection)
            assertTrue(freshConnection!!.commitText("fresh", 1))

            assertEquals("fresh", insertedText)
            assertEquals(6, insertedScalar)
        }
    }
}
