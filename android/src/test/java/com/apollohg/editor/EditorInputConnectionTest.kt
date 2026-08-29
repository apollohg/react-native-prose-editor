package com.apollohg.editor

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Bundle
import android.os.Looper
import android.provider.Settings
import android.text.Selection
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.InputType
import android.text.style.AbsoluteSizeSpan
import android.view.KeyEvent
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
import java.time.Duration

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorInputConnectionTest {
    @Test
    fun `backspace at paragraph boundary stays equal to Rust render`() {
        val harness = structuredDeleteHarness("<p>Alpha</p><p>Beta</p>")
        try {
            harness.editText.setSelection("Alpha\n".length)
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.deleteSurroundingText(1, 0))
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(harness.expectedText(), harness.editText.text.toString())
            assertEquals("AlphaBeta", harness.editText.text.toString())
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `ordinary character backspace keeps deferred optimistic path`() {
        val harness = structuredDeleteHarness("<p>Alpha</p>")
        try {
            harness.editText.setSelection(5)
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.deleteSurroundingText(1, 0))

            assertTrue(harness.editText.hasDeferredRustUpdateApplicationForTesting())
            assertEquals("Alph", harness.editText.text.toString())
            assertEquals("Alph", harness.editText.authorizedTextForTesting())
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `plain backspace does not authorize optimistic text before Rust accepts it`() {
        val harness = externalCompositionHarness("Alpha")
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(5)
            harness.backend.nextRenderUpdateResult = EditorV2CallResult.Err(
                EditorV2Error("render", "RENDER_FAILED", "transient"),
            )
            var authorizedDuringNativeIntent: String? = null
            harness.backend.onApplyNativeIntent = {
                authorizedDuringNativeIntent = harness.editText.authorizedTextForTesting()
            }
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.deleteSurroundingText(1, 0))

            assertEquals("Alph", harness.editText.text.toString())
            assertEquals("Alpha", authorizedDuringNativeIntent)
            assertEquals("Alph", harness.editText.authorizedTextForTesting())
            shadowOf(Looper.getMainLooper()).idle()
            assertEquals("Alph", harness.editText.text.toString())
            assertEquals("Alph", harness.editText.authorizedTextForTesting())
            assertEquals(4, harness.editText.selectionStart)
            assertEquals(4, harness.editText.selectionEnd)
            assertEquals(1, listener.receivedUpdates.size)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `plain backspace restores authoritative text when native intent is rejected`() {
        val harness = externalCompositionHarness("Alpha")
        try {
            harness.editText.setSelection(5)
            harness.backend.nextApplyNativeIntentResult = EditorV2CallResult.Err(
                EditorV2Error("operation", "MUTATION_REJECTED", "rejected"),
            )
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.deleteSurroundingText(1, 0))

            assertEquals("Alpha", harness.editText.text.toString())
            assertEquals("Alpha", harness.editText.authorizedTextForTesting())
            assertEquals(5, harness.editText.selectionStart)
            assertEquals(5, harness.editText.selectionEnd)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `plain backspace adopts Rust selection when position epoch is recovered`() {
        val harness = externalCompositionHarness("Alpha")
        try {
            harness.editText.setSelection(5)
            val session = harness.backend.sessions.getValue(harness.editorId)
            session.anchor = 2
            session.head = 2
            session.positionEpochs.clear()
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.deleteSurroundingText(1, 0))

            assertEquals("Alpha", harness.editText.text.toString())
            assertEquals("Alpha", harness.editText.authorizedTextForTesting())
            assertEquals(2, harness.editText.selectionStart)
            assertEquals(2, harness.editText.selectionEnd)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `plain input recovery falls back when an incremental render cannot apply`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyRenderJSON(
            JSONArray()
                .put(
                    JSONObject()
                        .put("type", "blockStart")
                        .put("nodeType", "paragraph")
                        .put("depth", 0),
                )
                .put(
                    JSONObject()
                        .put("type", "textRun")
                        .put("text", "Alpha")
                        .put("marks", JSONArray()),
                )
                .put(JSONObject().put("type", "blockEnd"))
                .toString(),
        )
        editText.setSelection(5)
        val authoritative = editText.captureAuthoritativeInputSnapshotForEditor()
        editText.runWithTransientInputMutationGuard {
            editText.text!!.delete(4, 5)
            true
        }
        val patchOnlyRecovery = JSONObject()
            .put(
                "renderPatch",
                JSONObject()
                    .put("startIndex", 0)
                    .put("deleteCount", 1)
                    .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Alpha"))),
            )
            .toString()

        editText.restoreAuthoritativeInputForEditor(authoritative, patchOnlyRecovery)

        assertEquals("Alpha", editText.text.toString())
        assertEquals("Alpha", editText.authorizedTextForTesting())
        assertEquals(5, editText.selectionStart)
        assertEquals(5, editText.selectionEnd)
    }

    @Test
    fun `ordinary backspace after earlier generated separators stays equal to Rust render`() {
        val harness = structuredDeleteHarness(
            "<p>First</p><p>Second</p><p>Type a native mention here</p>"
        )
        try {
            val target = "native"
            val cursor = harness.editText.text.toString().indexOf(target) + target.length
            harness.editText.setSelection(cursor)
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.deleteSurroundingText(1, 0))
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(harness.expectedText(), harness.editText.text.toString())
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `generated structural backspaces do not mutate native text optimistically`() {
        val listCases = listOf(
            """
            [
                {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}},
                {"type":"blockStart","nodeType":"paragraph","depth":1},
                {"type":"textRun","text":"Item","marks":[]},
                {"type":"blockEnd"},
                {"type":"blockEnd"}
            ]
            """.trimIndent(),
            """
            [
                {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":true,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}},
                {"type":"blockStart","nodeType":"paragraph","depth":1},
                {"type":"textRun","text":"Item","marks":[]},
                {"type":"blockEnd"},
                {"type":"blockEnd"}
            ]
            """.trimIndent(),
            """
            [
                {"type":"blockStart","nodeType":"taskItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true,"kind":"task","checked":false}},
                {"type":"blockStart","nodeType":"paragraph","depth":1},
                {"type":"textRun","text":"Item","marks":[]},
                {"type":"blockEnd"},
                {"type":"blockEnd"}
            ]
            """.trimIndent()
        )
        listCases.forEach { renderJSON ->
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyRenderJSON(renderJSON)
            val bodyStart = editText.text.toString().indexOf("Item")
            assertGeneratedBackspaceDoesNotMutateNative(editText, bodyStart)
        }

        val placeholderEditor = EditorEditText(RuntimeEnvironment.getApplication())
        placeholderEditor.applyRenderJSON(
            """
            [
                {"type":"blockStart","nodeType":"paragraph","depth":0},
                {"type":"textRun","text":"A","marks":[]},
                {"type":"voidInline","nodeType":"hardBreak","docPos":2},
                {"type":"blockEnd"}
            ]
            """.trimIndent()
        )
        assertGeneratedBackspaceDoesNotMutateNative(
            placeholderEditor,
            placeholderEditor.text!!.length
        )
    }

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
    fun `random collapsed backspaces keep native render equal to Rust`() {
        val initialHtml = buildString {
            append("<p><strong>Native Editor</strong> example app.</p>")
            append("<p>Use this screen to test focus, theme updates, lists, line breaks, toolbar behavior, and optional addons.</p>")
            append("<p>Enable mentions above, then type @ after a space, on a blank line, or after punctuation to show native mention suggestions in the toolbar.</p>")
            append("<blockquote><p>Blockquotes can wrap one or more blocks and inherit theme styling.</p></blockquote>")
            append("<ul><li><p>Try typing</p></li><li><p>Try list indenting</p><ul><li>Multiple levels are supported</li></ul></li></ul>")
            append("<p></p>")
        }

        val seed = 0
        val harness = structuredDeleteHarness(initialHtml)
        try {
            val random = kotlin.random.Random(seed)
            repeat(80) { step ->
                val offset = random.nextInt(harness.editText.text!!.length + 1)
                harness.editText.setSelection(offset)
                val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

                assertTrue(inputConnection.deleteSurroundingText(1, 0))
                shadowOf(Looper.getMainLooper()).idle()

                assertEquals(
                    "seed=$seed step=$step offset=$offset",
                    harness.expectedText(),
                    harness.editText.text.toString()
                )
            }
        } finally {
            harness.adapter.destroy()
        }
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
    fun `keyboard input commits external phrase before typing`() {
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
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        editText.beginExternalTextComposition("speech-1")
        editText.updateExternalTextComposition("speech-1", "O/A")

        assertTrue(inputConnection.commitText("!", 1))

        assertEquals("O/A!", editText.text.toString())
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
    fun `external composition empty final text deletes the selected range`() {
        val harness = externalCompositionHarness("arrival")
        harness.editText.setSelection(0, 7)

        harness.editText.beginExternalTextComposition("speech-1")
        harness.editText.updateExternalTextComposition("speech-1", "draft")
        val result = JSONObject(
            harness.editText.commitExternalTextComposition("speech-1", "")
        )

        assertEquals("committed", result.getString("outcome"))
        assertEquals("", harness.backend.sessions.getValue(harness.editorId).text.toString())
        assertEquals("", harness.editText.text.toString())
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
    fun `external composition task marker tap recomputes filtered scalar`() {
        val harness = realExternalCompositionHarness(
            initialText = "12",
            configJson = """
                {
                  "schema": {
                    "nodes": [
                      {"name":"doc","content":"block+","role":"doc"},
                      {"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},
                      {"name":"taskList","content":"taskItem+","group":"block","role":"list"},
                      {"name":"taskItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},
                      {"name":"text","group":"inline","role":"text"}
                    ],
                    "marks": []
                  },
                  "initialization": {"type":"localEmpty"},
                  "policy": {"inputFilter":"[0-9]"}
                }
            """.trimIndent()
        )
        try {
            val document = """
                {
                  "type": "doc",
                  "content": [
                    {"type":"paragraph","content":[{"type":"text","text":"12"}]},
                    {"type":"taskList","content":[
                      {"type":"taskItem","attrs":{"checked":false},"content":[
                        {"type":"paragraph","content":[{"type":"text","text":"Task item"}]}
                      ]}
                    ]}
                  ]
                }
            """.trimIndent()
            harness.adapter.setContentJson(document)
                ?.let { harness.editText.applyUpdateJSON(it, notifyListener = false) }
            harness.editText.setSelection(0, 2)
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            val toggles = mutableListOf<Pair<Int, Int>>()
            harness.editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { anchor, head ->
                listener.events.add("toggle")
                toggles.add(anchor to head)
            }
            harness.editText.beginExternalTextComposition("speech-filtered-task")
            harness.editText.updateExternalTextComposition("speech-filtered-task", "letters")
            harness.editText.layoutParams = android.view.ViewGroup.LayoutParams(600, 240)
            val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
            val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
            harness.editText.measure(widthSpec, heightSpec)
            harness.editText.layout(
                0,
                0,
                harness.editText.measuredWidth,
                harness.editText.measuredHeight
            )
            val textLayout = requireNotNull(harness.editText.layout)
            val markerIndex = harness.editText.text.toString()
                .indexOf(LayoutConstants.TASK_LIST_MARKER_UNCHECKED)
            assertTrue(markerIndex >= 0)
            val provisionalScalar = PositionBridge.utf16ToScalar(
                markerIndex,
                harness.editText.text.toString()
            )
            val markerLine = textLayout.getLineForOffset(markerIndex)
            val tapX = harness.editText.totalPaddingLeft + 1f
            val tapY = harness.editText.totalPaddingTop +
                ((textLayout.getLineTop(markerLine) + textLayout.getLineBottom(markerLine)) / 2f)

            val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
            harness.editText.onTouchEvent(down)
            down.recycle()
            val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
            harness.editText.onTouchEvent(up)
            up.recycle()

            val authoritativeText = harness.editText.text.toString()
            val authoritativeMarker = authoritativeText
                .indexOf(LayoutConstants.TASK_LIST_MARKER_UNCHECKED)
            val authoritativeScalar = PositionBridge.utf16ToScalar(
                authoritativeMarker,
                authoritativeText
            )
            assertTrue(provisionalScalar != authoritativeScalar)
            assertEquals(listOf(authoritativeScalar to authoritativeScalar), toggles)
            assertEquals(
                "interaction",
                JSONObject(listener.externalCompositionEnds.single()).getString("cause")
            )
            assertEquals(listOf("external", "toggle"), listener.events)
        } finally {
            harness.adapter.destroy()
        }
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
    fun `external composition delete and selection commit before interaction`() {
        val deleteHarness = externalCompositionHarness("arrival")
        val deleteListener = RecordingEditorListener()
        deleteHarness.editText.editorListener = deleteListener
        deleteHarness.editText.setSelection(0, 7)
        val deleteConnection = deleteHarness.editText.onCreateInputConnection(EditorInfo())!!
        deleteHarness.editText.beginExternalTextComposition("speech-delete")
        deleteHarness.editText.updateExternalTextComposition("speech-delete", "draft")

        assertTrue(deleteConnection.deleteSurroundingText(1, 0))

        assertEquals("draf", deleteHarness.editText.text.toString())
        assertEquals("interaction", JSONObject(deleteListener.externalCompositionEnds.single()).getString("cause"))

        val selectionHarness = externalCompositionHarness("arrival")
        val selectionListener = RecordingEditorListener()
        selectionHarness.editText.editorListener = selectionListener
        selectionHarness.editText.setSelection(0, 7)
        val selectionConnection = selectionHarness.editText.onCreateInputConnection(EditorInfo())!!
        selectionHarness.editText.beginExternalTextComposition("speech-selection")
        selectionHarness.editText.updateExternalTextComposition("speech-selection", "draft")

        assertTrue(selectionConnection.setSelection(0, 0))

        assertEquals("draft", selectionHarness.editText.text.toString())
        assertEquals(0, selectionHarness.editText.selectionStart)
        assertEquals("interaction", JSONObject(selectionListener.externalCompositionEnds.single()).getString("cause"))
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
    fun `external composition maximum length failure is atomic`() {
        assertRealExternalCompositionPolicyFailure(
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"maxLength":3}}""",
            initialText = "ab",
            finalText = "long"
        )
    }

    @Test
    fun `external composition input filter failure is atomic`() {
        assertRealExternalCompositionPolicyFailure(
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[unclosed"}}""",
            initialText = "12",
            finalText = "letters"
        )
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
    fun `editor input traits use rich text defaults`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        assertEquals(InputType.TYPE_CLASS_TEXT, editText.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_MULTI_LINE)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_AUTO_CORRECT)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
    }

    @Test
    fun `editor input traits apply React keyboard props`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        editText.setKeyboardType("email-address")
        editText.setAutoCapitalize("none")
        editText.setAutoCorrect(false)

        assertEquals(InputType.TYPE_CLASS_TEXT, editText.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_MULTI_LINE)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS)
        assertFalse(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_AUTO_CORRECT)
        assertFalse(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
    }

    @Test
    fun `private IME option changes restart focused input only when value changes`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        assertTrue(editText.requestFocus())
        fun restartCount() = editText.imeTraceSnapshotForTesting().count {
            it.contains("restartInput:source=privateImeOptions")
        }

        editText.setPrivateImeOptionsForEditor("nm")
        assertEquals(1, restartCount())

        editText.setPrivateImeOptionsForEditor("nm")
        assertEquals(1, restartCount())

        editText.setPrivateImeOptionsForEditor(null)
        assertEquals(2, restartCount())
    }

    @Test
    fun `cursor caps mode treats rendered empty block start as sentence start`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        assertEquals("Hello\n\u200B", editText.text.toString())
        assertTrue(
            editText.cursorCapsModeForEditor(
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES,
                baseCapsMode = 0
            ) hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )

        val editorInfo = EditorInfo()
        val inputConnection = editText.onCreateInputConnection(editorInfo)
        assertNotNull(inputConnection)
        assertTrue(editorInfo.initialCapsMode hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
        assertTrue(
            inputConnection!!.getCursorCapsMode(InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
                hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )
    }

    @Test
    fun `text before cursor hides synthetic empty block placeholder from IME context`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertEquals("\n", inputConnection!!.getTextBeforeCursor(1, 0).toString())
        assertEquals("Hello\n", inputConnection.getTextBeforeCursor(20, 0).toString())
    }

    @Test
    fun `all surrounding text queries use placeholder free coordinates and retain styles`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val raw = SpannableStringBuilder("\u200Ba\uD83D\uDE00\u200Bb").apply {
            setSpan(AbsoluteSizeSpan(30), 1, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        editText.setText(raw)
        editText.setSelection(5)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        val before = requireNotNull(
            inputConnection.getTextBeforeCursor(20, InputConnection.GET_TEXT_WITH_STYLES),
        )
        assertEquals("a\uD83D\uDE00", before.toString())
        assertTrue(before is Spanned)
        assertEquals(1, (before as Spanned).getSpans(0, before.length, AbsoluteSizeSpan::class.java).size)
        assertEquals("b", inputConnection.getTextAfterCursor(20, 0).toString())

        editText.setSelection(1, 6)
        assertEquals("a\uD83D\uDE00b", inputConnection.getSelectedText(0).toString())

        editText.setSelection(1)
        assertNull(inputConnection.getSelectedText(0))

        editText.setSelection(0, 1)
        assertNull(inputConnection.getSelectedText(0))
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
    fun `correction offsets map past synthetic placeholders`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("\u200Bteh"), notifyListener = false)
        editText.editorId = 1
        editText.setSelection(editText.text!!.length)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacement = Triple(from, to, text)
        }
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertEquals(Triple(1, 4, "the"), replacement)
    }

    @Test
    fun `explicit correction maps both ends around an interior synthetic placeholder`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("te\u200Bh"), notifyListener = false)
        editText.editorId = 1
        editText.setSelection(editText.text!!.length)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacement = Triple(from, to, text)
        }
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertEquals(Triple(0, 4, "the"), replacement)
    }

    @Test
    fun `inferred correction maps its visible token around an interior synthetic placeholder`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("te\u200Bh "), notifyListener = false)
        editText.editorId = 1
        editText.setSelection(editText.text!!.length)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacement = Triple(from, to, text)
        }
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(2, null, "the")))

        assertEquals(Triple(0, 4, "the"), replacement)
    }

    @Test
    fun `composition deletion uses visible coordinates around synthetic placeholders`() {
        listOf(false, true).forEach { deleteInCodePoints ->
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyUpdateJSON(renderUpdateJson("a\u200Bb"), notifyListener = false)
            editText.editorId = 1
            editText.setSelection(2)
            val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))
            assertTrue(inputConnection.setComposingRegion(0, 2))

            val deleted = if (deleteInCodePoints) {
                inputConnection.deleteSurroundingTextInCodePoints(1, 0)
            } else {
                inputConnection.deleteSurroundingText(1, 0)
            }

            assertTrue(deleted)
            assertEquals("\u200Bb", editText.text.toString())
            assertEquals(1, editText.selectionStart)
            assertEquals("b", editText.composingTextForEditor())
        }
    }

    @Test
    fun `surrounding deletion skips invisible placeholders`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("a\u200Bb")
        editText.setSelection(2)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.deleteSurroundingText(1, 0))

        assertEquals("\u200Bb", editText.text.toString())
        assertEquals(1, editText.selectionStart)
    }

    @Test
    fun `surrounding deletion preserves invisible placeholders inside the visible range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("a\u200Bb")
        editText.setSelection(3)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.deleteSurroundingText(2, 0))

        assertEquals("\u200B", editText.text.toString())
        assertEquals(1, editText.selectionStart)
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
    fun `selection updates after restart use IME visible coordinates`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = EditorEditText(activity)
        activity.setContentView(editText)
        editText.setText("\u200Ba")
        editText.setSelection(2)
        assertTrue(editText.requestFocus())

        editText.setPrivateImeOptionsForEditor("mapped-selection")
        shadowOf(Looper.getMainLooper()).idle()

        val trace = editText.imeTraceSnapshotForTesting()
        assertTrue(trace.toString(), trace.any {
            it.contains("updateSelectionAfterRestart:source=privateImeOptions sel=1..1")
        })
    }

    @Test
    fun `initial surrounding text removes synthetic placeholder for IME sentence caps`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        val editorInfo = EditorInfo()
        val inputConnection = editText.onCreateInputConnection(editorInfo)
        assertNotNull(inputConnection)

        assertEquals("Hello\n", editorInfo.getInitialTextBeforeCursor(20, 0).toString())
        assertEquals(editText.selectionStart - 1, editorInfo.initialSelStart)
        assertEquals(editText.selectionEnd - 1, editorInfo.initialSelEnd)
        assertFalse(editorInfo.getInitialTextBeforeCursor(20, 0).toString().contains("\u200B"))
        assertTrue(
            editorInfo.initialCapsMode hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )
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
    fun `cursor caps mode does not force sentence caps mid line`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        assertFalse(
            editText.cursorCapsModeForEditor(
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES,
                baseCapsMode = 0
            ) hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )
    }

    @Test
    fun `editor numeric keyboard type maps to numeric input class`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        editText.setKeyboardType("numeric")

        assertEquals(InputType.TYPE_CLASS_NUMBER, editText.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_NUMBER_FLAG_DECIMAL)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_NUMBER_FLAG_SIGNED)
    }

    @Test
    fun `input trait changes stale old connection and fresh connection accepts input`() {
        val traitChanges: List<(EditorEditText) -> Unit> = listOf(
            { it.setAutoCorrect(false) },
            { it.setKeyboardType("email-address") }
        )

        for (changeTrait in traitChanges) {
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
            editText.setSelection(3)
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

            changeTrait(editText)

            assertTrue(oldConnection!!.commitText("old", 1))
            assertNull(insertedText)

            val freshConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(freshConnection)
            assertTrue(freshConnection!!.commitText("fresh", 1))

            assertEquals("fresh", insertedText)
        }
    }

    @Test
    fun `external clear keeps same editor input connection accepting input`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("sent"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.applyUpdateJSON(
            renderUpdateJson("\u200B"),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertTrue(
            editText.imeTraceSnapshotForTesting().any {
                it.contains("restartInput:source=externalUpdate")
            }
        )

        assertTrue(inputConnection!!.commitText("fresh", 1))

        assertEquals("fresh", insertedText)
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
    fun `external clear invalidates rendered editor content`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("sent"), notifyListener = false)
        shadowOf(editText).clearWasInvalidated()

        editText.applyUpdateJSON(
            renderUpdateJson(""),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )

        assertEquals("", editText.text?.toString())
        assertTrue(shadowOf(editText).wasInvalidated())
    }

    @Test
    fun `external clear after deferred Rust update clears stale visible text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(0)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.runWithDeferredRustUpdateApplication {
            editText.runWithTransientInputMutationGuard {
                editText.text!!.insert(0, "second")
                editText.setSelection(6)
                true
            }
            editText.applyRustUpdateJSONForTesting(renderUpdateJson("second"))
        }

        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("second", editText.text?.toString())

        editText.applyUpdateJSON(
            renderUpdateJson(""),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("", editText.text?.toString())

        assertTrue(inputConnection!!.commitText("next", 1))
        assertEquals("next", insertedText)
    }

    @Test
    fun `external clear after preflight native mutation keeps same editor input connection accepting input`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(0)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var renderedText = ""
        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            renderedText = renderedText.substring(0, scalar.coerceIn(0, renderedText.length)) +
                text +
                renderedText.substring(scalar.coerceIn(0, renderedText.length))
            editText.applyUpdateJSON(renderUpdateJson(renderedText), notifyListener = false)
            editText.setSelection(renderedText.length)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(0, "second")
            editText.setSelection(6)
            true
        }

        assertTrue(editText.prepareForExternalEditorUpdate())
        assertEquals("second", insertedText)
        assertEquals("second", editText.text?.toString())

        renderedText = ""
        insertedText = null
        editText.applyUpdateJSON(
            renderUpdateJson("\u200B"),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertEquals("\u200B", editText.text?.toString())
        assertTrue(inputConnection!!.commitText("next", 1))
        assertEquals("next", insertedText)
    }

    @Test
    fun `destroyed editor input session consumes IME changes without Rust mutation`() {
        val editorId = 880001L
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = editorId

        var insertedText: String? = null
        var replacedText: Triple<Int, Int, String>? = null
        var deletedRange: Pair<Int, Int>? = null
        var deletedBackward: Pair<Int, Int>? = null
        var syncedSelection: Pair<Int, Int>? = null
        editText.onInsertTextInRustForTesting = { text, _ -> insertedText = text }
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacedText = Triple(from, to, text)
        }
        editText.onDeleteRangeInRustForTesting = { from, to -> deletedRange = from to to }
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            deletedBackward = anchor to head
        }
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        NativeEditorViewRegistry.markEditorCreated(editorId)
        try {
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)

            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)

            assertTrue(inputConnection!!.commitText("x", 1))
            assertTrue(inputConnection.commitCompletion(CompletionInfo(0, 0, "done")))
            assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "abc", "xyz")))
            assertTrue(inputConnection.setComposingText("z", 1))
            assertTrue(inputConnection.deleteSurroundingText(1, 0))
            editText.setSelection(0)

            assertNull(insertedText)
            assertNull(replacedText)
            assertNull(deletedRange)
            assertNull(deletedBackward)
            assertNull(syncedSelection)
        } finally {
            NativeEditorViewRegistry.markEditorCreated(editorId)
        }
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

    @Test
    fun `old input connection remains usable after framework recreation from render`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        val inserted = mutableListOf<Pair<String, Int>>()
        val rendered = StringBuilder()
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserted.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyUpdateJSON(renderUpdateJson(rendered.toString()), notifyListener = false)
            editText.setSelection(rendered.length)
        }

        val originalConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(originalConnection)
        assertTrue(originalConnection!!.commitText("a", 1))

        val recreatedConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(recreatedConnection)

        assertTrue(originalConnection.commitText("b", 1))

        assertEquals(listOf("a" to 0, "b" to 1), inserted)
        assertEquals("ab", editText.text?.toString())
    }

    @Test
    fun `input trait change suppresses stale direct native mutation adoption`() {
        val traitChanges: List<(EditorEditText) -> Unit> = listOf(
            { it.setAutoCorrect(false) },
            { it.setKeyboardType("email-address") }
        )

        for (changeTrait in traitChanges) {
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
            assertTrue(editText.requestFocus())
            editText.setSelection(6)
            editText.editorId = 1
            editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

            var insertedText: String? = null
            var replacement: Triple<Int, Int, String>? = null
            editText.onInsertTextInRustForTesting = { text, _ ->
                insertedText = text
            }
            editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
                replacement = Triple(scalarFrom, scalarTo, text)
            }

            val oldConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(oldConnection)
            assertTrue(oldConnection!!.setComposingText("brave ", 1))

            changeTrait(editText)
            assertEquals("Hello world", editText.text?.toString())

            editText.runWithTransientInputMutationGuard {
                editText.text!!.insert(6, "stale ")
                true
            }

            assertFalse(editText.prepareForExternalEditorUpdate())
            assertNull(insertedText)
            assertNull(replacement)
        }
    }

    @Test
    fun `native mutation adoption suppression clears after authorized render update`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
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

        editText.setAutoCorrect(false)
        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(6, "stale ")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(insertedText)
        assertNull(insertedScalar)

        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.text!!.insert(6, "fresh ")

        assertEquals("fresh ", insertedText)
        assertEquals(6, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native mutation adoption suppression clears after skipped authorized render update`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            captureApplyUpdateTraceForTesting = true
        }
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
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

        editText.setAutoCorrect(false)
        assertEquals("Hello world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.lastApplyUpdateTrace()?.skippedRender == true)

        editText.text!!.insert(6, "fresh ")

        assertEquals("fresh ", insertedText)
        assertEquals(6, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `code point delete length matches ascii backspace`() {
        val text = "Hello"
        val cursor = 5

        val beforeUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = text,
            fromUtf16Offset = cursor,
            codePointCount = 1,
            forward = false
        )

        assertEquals(1, beforeUtf16Length)
    }

    @Test
    fun `code point delete length counts surrogate pair as two utf16 code units`() {
        val text = "A😀B"
        val cursor = 3

        val beforeUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = text,
            fromUtf16Offset = cursor,
            codePointCount = 1,
            forward = false
        )

        assertEquals(2, beforeUtf16Length)
    }

    @Test
    fun `code point forward delete length counts surrogate pair as two utf16 code units`() {
        val text = "A😀B"
        val cursor = 1

        val afterUtf16Length = EditorInputConnection.codePointsToUtf16Length(
            text = text,
            fromUtf16Offset = cursor,
            codePointCount = 1,
            forward = true
        )

        assertEquals(2, afterUtf16Length)
    }

    @Test
    fun `read only composing text and region are consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1
        editText.isEditable = false

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingText("X", 1))
        assertTrue(inputConnection.setComposingRegion(0, 2))
        assertEquals("abc", editText.text?.toString())
        assertNull(editText.composingTextForEditor())
    }

    @Test
    fun `read only input connection mutations are consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.isEditable = false

        assertTrue(inputConnection!!.commitText("X", 1))
        assertTrue(inputConnection.deleteSurroundingText(1, 0))
        assertTrue(inputConnection.deleteSurroundingTextInCodePoints(1, 0))
        assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL)))
        assertEquals("abc", editText.text?.toString())
        assertEquals(3, editText.selectionStart)
        assertEquals(3, editText.selectionEnd)
    }

    @Test
    fun `composing text does not trigger reconciliation while edit text is transient`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val handled = inputConnection!!.setComposingText("abc", 1)

        assertTrue(handled)
        assertEquals("abc", editText.text?.toString())
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `commit text uses original authorized offset while composing text is visible`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        inputConnection!!.setComposingText("brave ", 1)
        assertEquals("Hello brave world", editText.text?.toString())

        val handled = inputConnection.commitText("brave ", 1)

        assertTrue(handled)
        assertEquals("brave ", insertedText)
        assertEquals(6, insertedScalar)
    }

    @Test
    fun `composing region after visible composing text preserves original authorized range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        assertTrue(inputConnection.setComposingRegion(0, 5))
        assertTrue(inputConnection.commitText("brave ", 1))

        assertEquals("brave ", insertedText)
        assertEquals(6, insertedScalar)
        assertEquals(null, replacement)
    }

    @Test
    fun `repeated composing region updates authorized replacement before visible composing text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abcde"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(0, 1))
        assertTrue(inputConnection.setComposingRegion(0, 5))
        assertTrue(inputConnection.commitText("ABCDE", 1))

        assertEquals(Triple(0, 5, "ABCDE"), replacement)
    }

    @Test
    fun `commit text replaces original authorized selection while composing text is visible`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        inputConnection!!.setComposingText("there", 1)
        assertEquals("Hello there", editText.text?.toString())

        val handled = inputConnection.commitText("there", 1)

        assertTrue(handled)
        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `delete during composition edits transient text without mutating rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var deleteCalled = false
        editText.onDeleteRangeInRustForTesting = { _, _ ->
            deleteCalled = true
        }
        editText.onInsertTextInRustForTesting = { _, _ -> }
        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        inputConnection!!.setComposingText("abc", 1)

        val deleteHandled = inputConnection.deleteSurroundingText(1, 0)
        val commitHandled = inputConnection.commitText("ab", 1)

        assertTrue(deleteHandled)
        assertTrue(commitHandled)
        assertFalse(deleteCalled)
        assertEquals("ab", insertedText)
        assertEquals(0, insertedScalar)
    }

    @Test
    fun `key event backspace during composition edits transient text without mutating rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var deleteCalled = false
        editText.onDeleteRangeInRustForTesting = { _, _ ->
            deleteCalled = true
        }
        editText.onInsertTextInRustForTesting = { _, _ -> }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        assertTrue(inputConnection.sendKeyEvent(android.view.KeyEvent(
            android.view.KeyEvent.ACTION_DOWN,
            android.view.KeyEvent.KEYCODE_DEL
        )))
        assertTrue(inputConnection.commitText("ab", 1))

        assertFalse(deleteCalled)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `duplicate composition key event across view and input connection edits transient text once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))
        assertTrue(inputConnection.sendKeyEvent(event))

        assertEquals("Hello ab", editText.text?.toString())
    }

    @Test
    fun `duplicate forward delete composition key event stays on transient composition path`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var deleteCalled = false
        editText.onDeleteRangeInRustForTesting = { _, _ ->
            deleteCalled = true
        }
        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))
        editText.setSelection(0)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))
        assertTrue(inputConnection.sendKeyEvent(event))

        assertFalse(deleteCalled)
        assertEquals("bc", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertEquals("bc", insertedText)
    }

    @Test
    fun `forward delete composition edit refreshes composing text before finish commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var deleteCalled = false
        editText.onDeleteRangeInRustForTesting = { _, _ ->
            deleteCalled = true
        }
        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))
        editText.setSelection(0)

        assertTrue(inputConnection.deleteSurroundingText(0, 1))
        assertTrue(inputConnection.finishComposingText())

        assertFalse(deleteCalled)
        assertEquals("bc", insertedText)
        assertEquals(0, insertedScalar)
    }

    @Test
    fun `hardware backspace composition fallback does not split emoji surrogate pair`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("😀", 1))
        editText.setSelection("😀".length)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals("", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertNull(insertedText)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `hardware forward delete composition fallback does not split emoji surrogate pair`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("😀", 1))
        editText.setSelection(0)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals("", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertNull(insertedText)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `hardware backspace inside composing emoji deletes whole surrogate pair`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("😀", 1))
        editText.setSelection(1)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals("", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertNull(insertedText)
    }

    @Test
    fun `hardware forward delete inside composing emoji deletes whole surrogate pair`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("😀", 1))
        editText.setSelection(1)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals("", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertNull(insertedText)
    }

    @Test
    fun `printable hardware key inside composing emoji replaces whole surrogate pair`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("😀", 1))
        editText.setSelection(1)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals("a", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertEquals("a", insertedText)
    }

    @Test
    fun `hardware backspace composition fallback deletes one code point from combining text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("e\u0301", 1))
        editText.setSelection("e\u0301".length)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals("e", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertEquals("e", insertedText)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `commit completion routes autocomplete text through rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("hel"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCompletion(CompletionInfo(1L, 0, "hello")))

        assertEquals(Triple(0, 3, "hello"), replacement)
    }

    @Test
    fun `commit correction routes corrected text through rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertEquals(Triple(0, 3, "the"), replacement)
    }

    @Test
    fun `commit correction replaces correction offset when caret is collapsed after word`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertEquals(Triple(0, 3, "the"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `stale commit correction range is consumed without inserting at caret`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("tah "), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertNull(replacement)
        assertNull(insertedText)
        assertEquals("tah ", editText.text?.toString())
    }

    @Test
    fun `commit correction with missing old text replaces word at offset`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(0, null, "the")))

        assertEquals(Triple(0, 3, "the"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text and invalid offset is consumed without inserting`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(-1, null, "the")))

        assertNull(replacement)
        assertNull(insertedText)
        assertEquals("teh ", editText.text?.toString())
    }

    @Test
    fun `commit correction with missing old text replaces word at sentence offset`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("say teh now"), notifyListener = false)
        editText.setSelection(7)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(4, null, "the")))

        assertEquals(Triple(4, 7, "the"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text replaces word containing offset`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("say teh now"), notifyListener = false)
        editText.setSelection(7)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(5, null, "the")))

        assertEquals(Triple(4, 7, "the"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text preserves trailing punctuation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh."), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(0, null, "the")))

        assertEquals(Triple(0, 3, "the"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text preserves punctuation inside sentence`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("say teh, now"), notifyListener = false)
        editText.setSelection(8)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(5, null, "the")))

        assertEquals(Triple(4, 7, "the"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text on punctuation is consumed without inserting`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh."), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(3, null, "the")))

        assertNull(replacement)
        assertNull(insertedText)
        assertEquals("teh.", editText.text?.toString())
    }

    @Test
    fun `commit correction with missing old text keeps internal hyphen in token`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("dont-stop "), notifyListener = false)
        editText.setSelection(10)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(4, null, "don't-stop")))

        assertEquals(Triple(0, 9, "don't-stop"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text keeps internal apostrophe in token`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("cant's "), notifyListener = false)
        editText.setSelection(7)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(4, null, "can't")))

        assertEquals(Triple(0, 6, "can't"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with missing old text on whitespace is consumed without inserting`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(3, null, "the")))

        assertNull(replacement)
        assertNull(insertedText)
        assertEquals("teh ", editText.text?.toString())
    }

    @Test
    fun `commit correction with missing old text does not split surrogate pair word`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("te😀h "), notifyListener = false)
        editText.setSelection(5)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(3, null, "term")))

        assertEquals(Triple(0, 4, "term"), replacement)
        assertNull(insertedText)
    }

    @Test
    fun `commit correction with old text and invalid offset is consumed without inserting`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(4)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCorrection(CorrectionInfo(-1, "teh", "the")))

        assertNull(replacement)
        assertNull(insertedText)
        assertEquals("teh ", editText.text?.toString())
    }

    @Test
    fun `commit correction during visible composition commits corrected composing text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("teh", 1))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertNull(insertedText)
        assertNull(insertedScalar)

        assertTrue(inputConnection.commitText("the", 1))

        assertEquals("the", insertedText)
        assertEquals(0, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `matching commit text after composition correction applies once so space can follow`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val rendered = StringBuilder()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("wouldnt", 1))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "wouldnt", "wouldn't")))

        assertTrue(inserts.isEmpty())
        assertEquals("wouldnt", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())

        assertTrue(inputConnection.commitText("wouldn't", 1))

        assertEquals(listOf("wouldn't" to 0), inserts)
        assertEquals("wouldn't", editText.text?.toString())
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())

        assertTrue(inputConnection.commitText(" ", 1))

        assertEquals(listOf("wouldn't" to 0, " " to 8), inserts)
        assertEquals("wouldn't ", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("wouldn't ", editText.text?.toString())
    }

    @Test
    fun `single letter composition correction followed by commit text keeps uppercase replacement`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val rendered = StringBuilder()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("i", 1))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "i", "I")))
        assertTrue(inputConnection.commitText("I", 1))

        assertEquals(listOf("I" to 0), inserts)
        assertEquals("I", editText.text?.toString())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("I", editText.text?.toString())
    }

    @Test
    fun `single letter composition correction applies when ime sends no follow up commit text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val rendered = StringBuilder()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("i", 1))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "i", "I")))

        assertTrue(inserts.isEmpty())
        assertEquals("i", editText.text?.toString())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(listOf("I" to 0), inserts)
        assertEquals("I", editText.text?.toString())
    }

    @Test
    fun `printable hardware key during composition stays transient until finish commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("b", 1))

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A, 0)
        assertTrue(editText.dispatchKeyEvent(event))

        assertEquals(0, editText.reconciliationCount)
        assertEquals("ba", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertEquals("ba", insertedText)
    }

    @Test
    fun `printable input connection key during composition stays transient until finish commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("b", 1))

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A, 0)
        assertTrue(inputConnection.sendKeyEvent(event))

        assertEquals(0, editText.reconciliationCount)
        assertEquals("ba", editText.text?.toString())
        assertTrue(inputConnection.finishComposingText())
        assertEquals("ba", insertedText)
    }

    @Test
    fun `read only completion and correction are consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.editorId = 1
        editText.isEditable = false

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitCompletion(CompletionInfo(1L, 0, "replacement")))
        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "abc", "replacement")))
        assertEquals("abc", editText.text?.toString())
    }

    @Test
    fun `key event enter during composition does not split rust before commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var deleteAndSplitCalled = false
        editText.onDeleteAndSplitScalarInRustForTesting = { _, _ ->
            deleteAndSplitCalled = true
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        assertTrue(inputConnection.sendKeyEvent(android.view.KeyEvent(
            android.view.KeyEvent.ACTION_DOWN,
            android.view.KeyEvent.KEYCODE_ENTER
        )))

        assertFalse(deleteAndSplitCalled)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `typing keeps every character when a remote update lands between keystrokes`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(backend, editorId, roomBound = true)!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>ab</p>")?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        activity.setContentView(editText)
        assertTrue(editText.requestFocus())
        editText.setSelection(2)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!

        assertTrue(inputConnection.commitText("X", 1))
        assertEquals("abX", editText.text.toString())

        val session = backend.sessions.getValue(editorId)
        session.text.append("R")
        session.revision += 1uL

        assertTrue(inputConnection.commitText("Y", 1))
        shadowOf(Looper.getMainLooper()).idle()

        assertTrue(
            "the typed character must survive a concurrent remote update, saw ${editText.text}",
            editText.text.toString().contains("Y")
        )
    }

    @Test
    fun `composition commit survives a remote update landing mid composition`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(backend, editorId, roomBound = true)!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>hello</p>")?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        activity.setContentView(editText)
        assertTrue(editText.requestFocus())
        editText.setSelection(0, 5)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        assertTrue(inputConnection.setComposingText("hello", 1))

        val session = backend.sessions.getValue(editorId)
        session.text.append(" REMOTE")
        session.revision += 1uL

        assertTrue(inputConnection.commitText("HELLO", 1))
        shadowOf(Looper.getMainLooper()).idle()

        assertTrue(
            "the committed word must survive a concurrent remote update, saw ${editText.text}",
            editText.text.toString().contains("HELLO")
        )
    }

    @Test
    fun `composition replacement return defers one refresh until the split render is applied`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>seed</p>")?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        activity.setContentView(editText)
        assertTrue(editText.requestFocus())
        assertTrue(editText.hasFocus())
        editText.setSelection(1, 3)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        editText.clearImeTraceForTesting()

        assertTrue(inputConnection.setComposingText("ee", 1))
        assertTrue(inputConnection.commitText("\n", 1))
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("s\nd", editText.text.toString())
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("lineBoundaryInputRefreshScheduled")
        })
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("restartInput:source=lineBoundary:deleteAndSplit")
        })
    }

    @Test
    fun `stale composition return follows after affinity and refreshes the line boundary`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>seed</p>")?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        activity.setContentView(editText)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        val session = backend.sessions.getValue(adapter.editorId)
        session.text.append(" REMOTE")
        session.revision += 1u
        editText.clearImeTraceForTesting()

        assertTrue(inputConnection.setComposingText("\n", 1))
        assertTrue(inputConnection.commitText("\n", 1))
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("seed REMOTE\n", editText.text.toString())
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("lineBoundaryInputRefreshScheduled")
        })
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("restartInput:source=lineBoundary:")
        })
    }

    @Test
    fun `refreshed input connection commits after composition return split`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>seed</p>")?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        activity.setContentView(editText)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)

        val initialConnection = editText.onCreateInputConnection(EditorInfo())!!
        val initialGeneration = editText.inputConnectionGenerationForTesting()
        editText.clearImeTraceForTesting()

        assertTrue(initialConnection.setComposingText("\n", 1))
        assertTrue(initialConnection.commitText("\n", 1))
        shadowOf(Looper.getMainLooper()).idle()

        // The fake backend models the trailing empty paragraph as a terminal newline. The
        // device test covers Rust's trailing-hard-break placeholder representation.
        assertEquals("seed\n", editText.text.toString())
        assertEquals(5, editText.selectionStart)
        assertEquals(initialGeneration, editText.inputConnectionGenerationForTesting())

        val refreshedEditorInfo = EditorInfo()
        val refreshedConnection = editText.onCreateInputConnection(refreshedEditorInfo)!!
        assertTrue(refreshedConnection !== initialConnection)
        assertEquals(initialGeneration, editText.inputConnectionGenerationForTesting())
        assertEquals(5, refreshedEditorInfo.initialSelStart)
        assertEquals(5, refreshedEditorInfo.initialSelEnd)
        assertEquals("seed\n", refreshedEditorInfo.getInitialTextBeforeCursor(20, 0).toString())
        assertTrue(refreshedEditorInfo.initialCapsMode hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("lineBoundaryInputRefreshScheduled")
        })
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("restartInput:source=lineBoundary:splitBlock")
        })
        assertEquals(1, editText.imeTraceSnapshotForTesting().count {
            it.startsWith("createInputConnection:boundEditor=1 boundGen=$initialGeneration")
        })
        assertTrue(editText.imeTraceSnapshotForTesting().any {
            it.startsWith("applySelectionFromJSON:doc=") && it.contains("scalar=5..5")
        })

        assertTrue(refreshedConnection.commitText("x", 1))
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("seed\nx", editText.text.toString())
        assertEquals(6, editText.selectionStart)
        assertEquals(6, editText.selectionEnd)
        val document = adapter.documentJson()?.let(::JSONObject) ?: error("missing document JSON")
        assertEquals(2, document.getJSONArray("content").length())
    }

    @Test
    fun `commit text after composing region replaces original authorized range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(0, 3))
        assertTrue(inputConnection.commitText("the", 1))

        assertEquals(Triple(0, 3, "the"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `multiline composition commits as structured content`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var insertedContent: Triple<Int, Int, String>? = null
        editText.onInsertContentJsonAtSelectionScalarForTesting = { scalarFrom, scalarTo, json ->
            insertedContent = Triple(scalarFrom, scalarTo, json)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(6, 11))
        assertTrue(inputConnection.commitText("one\ntwo", 1))

        val (scalarFrom, scalarTo, json) = insertedContent!!
        assertEquals(6, scalarFrom)
        assertEquals(11, scalarTo)
        val content = JSONObject(json).getJSONArray("content")
        assertEquals("one", content.getJSONObject(0).getJSONArray("content").getJSONObject(0).getString("text"))
        assertEquals("two", content.getJSONObject(1).getJSONArray("content").getJSONObject(0).getString("text"))
    }

    @Test
    fun `commit newline after composing region delete splits original authorized range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedAndSplitRange: Pair<Int, Int>? = null
        editText.onDeleteAndSplitScalarInRustForTesting = { scalarFrom, scalarTo ->
            deletedAndSplitRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(6, 11))
        assertTrue(inputConnection.commitText("\n", 1))

        assertEquals(6 to 11, deletedAndSplitRange)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `empty commit text after composing region deletes authorized text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(0, 3))
        assertTrue(inputConnection.commitText("", 1))

        assertEquals(null, replacement)
        assertEquals(0 to 3, deletedRange)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `finish composing text after unchanged composing region skips no-op replacement`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(0, 3))
        assertTrue(inputConnection.finishComposingText())

        assertEquals(null, replacement)
        assertNotNull(syncedSelection)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `unchanged newline composition is treated as no-op before split handling`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("\n"), notifyListener = false)
        editText.setSelection(0, 1)
        editText.editorId = 1

        var deleteAndSplitCalled = false
        editText.onDeleteAndSplitScalarInRustForTesting = { _, _ ->
            deleteAndSplitCalled = true
        }
        var selectedScalar: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            selectedScalar = anchor to head
        }

        editText.handleCompositionCommit("\n", 0, 1)

        assertFalse(deleteAndSplitCalled)
        assertEquals(1 to 1, selectedScalar)
        assertEquals("\n", editText.text?.toString())
    }

    @Test
    fun `finish composing text after unchanged composing region moves default cursor to range end`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var selectedScalar: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            selectedScalar = anchor to head
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(0, 3))
        assertTrue(inputConnection.commitText("abc", 1))

        assertEquals(3 to 3, selectedScalar)
    }

    @Test
    fun `finish composing text with empty composition restores and handles cancellation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingText("", 1))
        assertTrue(inputConnection.finishComposingText())

        assertEquals("", editText.text?.toString())
        assertEquals(null, insertedText)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `finish composing text with empty selected composition deletes replacement range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("", 1))

        assertTrue(inputConnection.finishComposingText())

        assertEquals(6 to 11, deletedRange)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `composition replacement range invalidates after authorized render changes`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.finishComposingText())
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `commit text after authorized render change is consumed without inserting stale composition`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.commitText("brave ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `commit correction after authorized render change is consumed without replacing matching text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello brave world"), notifyListener = false)

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(6, "brave ", "braver ")))
        assertEquals("Hello brave world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `composing text after authorized render change does not reauthorize stale commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.setComposingText("braver ", 1))
        assertTrue(inputConnection.commitText("braver ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `composing region after authorized render change does not reauthorize stale commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.setComposingRegion(6, 13))
        assertTrue(inputConnection.commitText("braver ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `delete surrounding text after authorized render change is consumed without deleting authorized text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var deleteRange: Pair<Int, Int>? = null
        var deleteBackward: Pair<Int, Int>? = null
        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deleteRange = scalarFrom to scalarTo
        }
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            deleteBackward = anchor to head
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.deleteSurroundingText(1, 0))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(deleteRange)
        assertNull(deleteBackward)
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `delete surrounding text in code points after authorized render change is consumed without mutation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var deleteRange: Pair<Int, Int>? = null
        var deleteBackward: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deleteRange = scalarFrom to scalarTo
        }
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            deleteBackward = anchor to head
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.deleteSurroundingTextInCodePoints(1, 0))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(deleteRange)
        assertNull(deleteBackward)
    }

    @Test
    fun `no-op delete after authorized render change does not allow stale commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.deleteSurroundingText(0, 0))
        assertTrue(inputConnection.commitText("braver ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `no-op code point delete after authorized render change does not allow stale commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.deleteSurroundingTextInCodePoints(0, 0))
        assertTrue(inputConnection.commitText("braver ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `delete key event after authorized render change is consumed without rust mutation`() {
        for (keyCode in listOf(KeyEvent.KEYCODE_DEL, KeyEvent.KEYCODE_FORWARD_DEL)) {
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
            editText.setSelection(6)
            editText.editorId = 1

            var deleteRange: Pair<Int, Int>? = null
            var deleteBackward: Pair<Int, Int>? = null
            editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
                deleteRange = scalarFrom to scalarTo
            }
            editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
                deleteBackward = anchor to head
            }

            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)
            assertTrue(inputConnection!!.setComposingText("brave ", 1))
            assertEquals("Hello brave world", editText.text?.toString())

            editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

            assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, keyCode)))
            assertEquals("Hello updated world", editText.text?.toString())
            assertNull(deleteRange)
            assertNull(deleteBackward)
        }
    }

    @Test
    fun `printable key event after authorized render change is consumed without inserting text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        val event = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A, 0)
        assertTrue(inputConnection.sendKeyEvent(event))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `fresh input connection after stale key up accepts new commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        val staleConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(staleConnection)
        assertTrue(staleConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(staleConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL)))
        assertEquals("Hello updated world", editText.text?.toString())

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val freshConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(freshConnection)
        assertTrue(freshConnection!!.commitText(" fresh", 1))

        assertEquals(" fresh", insertedText)
    }

    @Test
    fun `key up after authorized render change does not clear invalidation before stale commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL)))
        assertTrue(inputConnection.commitText("brave ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `fresh input connection after stale selection accepts new commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        val staleConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(staleConnection)
        assertTrue(staleConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        assertTrue(staleConnection.setSelection(6, 13))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(syncedSelection)

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val freshConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(freshConnection)
        assertTrue(freshConnection!!.commitText(" fresh", 1))

        assertEquals(" fresh", insertedText)
    }

    @Test
    fun `set selection after authorized render change does not reauthorize stale commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello updated world"), notifyListener = false)

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        assertTrue(inputConnection.setSelection(6, 13))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(syncedSelection)

        assertTrue(inputConnection.commitText("braver ", 1))
        assertEquals("Hello updated world", editText.text?.toString())
        assertNull(insertedText)
        assertNull(replacement)
    }

    @Test
    fun `set selection without invalidation delegates and syncs selection`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setSelection(6, 11))
        assertEquals(6 to 11, syncedSelection)
        assertEquals(6, editText.selectionStart)
        assertEquals(11, editText.selectionEnd)
    }

    @Test
    fun `stale input connection is consumed after editor rebind`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("first"), notifyListener = false)
        editText.setSelection(5)
        editText.editorId = 1

        var insertedText: String? = null
        var deleteRange: Pair<Int, Int>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deleteRange = scalarFrom to scalarTo
        }

        val staleConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(staleConnection)

        editText.discardTransientNativeInputForEditorRebind()
        editText.editorId = 0
        editText.applyUpdateJSON(renderUpdateJson("second"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 2

        assertTrue(staleConnection!!.commitText("X", 1))
        assertTrue(staleConnection.deleteSurroundingText(1, 0))

        assertEquals("second", editText.text?.toString())
        assertNull(insertedText)
        assertNull(deleteRange)
    }

    @Test
    fun `focused read only toggle restarts input and keeps stale connection blocked`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1
        assertTrue(editText.requestFocus())

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val staleConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(staleConnection)

        editText.isEditable = false
        editText.isEditable = true

        assertTrue(
            editText.imeTraceSnapshotForTesting().any {
                it.contains("restartInput:source=editable")
            }
        )

        assertTrue(staleConnection!!.commitText("X", 1))

        assertEquals("abc", editText.text?.toString())
        assertNull(insertedText)

        val freshConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(freshConnection)

        assertTrue(freshConnection!!.commitText("Y", 1))
        assertEquals("Y", insertedText)
    }

    @Test
    fun `command preflight flushes empty selected composition as deletion`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
            editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("", 1))

        assertTrue(editText.prepareForExternalEditorUpdate())

        assertEquals(6 to 11, deletedRange)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `commit text after input connection recreation uses persisted composition range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val firstConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(firstConnection)
        assertTrue(firstConnection!!.setComposingText("brave ", 1))
        assertEquals("Hello brave world", editText.text?.toString())

        val recreatedConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(recreatedConnection)
        assertTrue(recreatedConnection!!.commitText("brave ", 1))

        assertEquals("brave ", insertedText)
        assertEquals(6, insertedScalar)
    }

    @Test
    fun `composing text uses rendered paragraph font size before Samsung space commit`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val editText = EditorEditText(context)
        editText.setBaseStyle(24f * density, Color.BLACK, Color.WHITE)
        editText.applyTheme(
            EditorTheme.fromJson(
                """
                {
                  "text": { "fontSize": 12, "color": "#112233" }
                }
                """.trimIndent()
            )
        )
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("word", 1))

        val sizeSpans = editText.text!!.getSpans(0, 4, AbsoluteSizeSpan::class.java)
        assertTrue(sizeSpans.any { it.size == (12f * density).toInt() })
    }

    @Test
    fun `finish composing defers render so pending Samsung space commit uses same connection`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val updates = mutableListOf<String>()
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) {
                updates.add(updateJSON)
            }
        }

        val inserted = mutableListOf<Pair<String, Int>>()
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserted.add(text to scalar)
            val renderedText = when (text) {
                "word" -> "word"
                " " -> "word "
                else -> text
            }
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(renderedText))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("word", 1))

        assertTrue(inputConnection.finishComposingText())

        assertEquals(listOf("word" to 0), inserted)
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertTrue(updates.isEmpty())

        assertTrue(inputConnection.commitText(" ", 1))

        assertEquals(listOf("word" to 0, " " to 4), inserted)
        assertEquals("word ", editText.text?.toString())
        assertEquals(1, updates.size)

        shadowOf(Looper.getMainLooper()).idle()

        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("word ", editText.text?.toString())
        assertEquals(1, updates.size)
    }

    @Test
    fun `finish composing deferred render applies on next loop without pending commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val updates = mutableListOf<String>()
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) {
                updates.add(updateJSON)
            }
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(text))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("word", 1))

        assertTrue(inputConnection.finishComposingText())

        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertTrue(updates.isEmpty())

        shadowOf(Looper.getMainLooper()).idle()

        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("word", editText.text?.toString())
        assertEquals(1, updates.size)
    }

    @Test
    fun `composition commit defers render so Samsung autocorrect space commit survives`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.editorId = 1

        val updates = mutableListOf<String>()
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) {
                updates.add(updateJSON)
            }
        }

        val rendered = StringBuilder("teh")
        val replacements = mutableListOf<Triple<Int, Int, String>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacements.add(Triple(scalarFrom, scalarTo, text))
            rendered.replace(scalarFrom, scalarTo, text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingRegion(0, 3))

        assertTrue(inputConnection.commitText("the", 1))

        assertEquals(listOf(Triple(0, 3, "the")), replacements)
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertTrue(updates.isEmpty())

        assertTrue(inputConnection.commitText(" ", 1))

        assertEquals(listOf(" " to 3), inserts)
        assertEquals("the ", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("the ", editText.text?.toString())
    }

    @Test
    fun `composition commit uses composing span when tracked range is collapsed`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("wouldnt"), notifyListener = false)
        editText.setSelection(7)
        editText.editorId = 1

        val rendered = StringBuilder("wouldnt")
        val replacements = mutableListOf<Triple<Int, Int, String>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacements.add(Triple(scalarFrom, scalarTo, text))
            rendered.replace(scalarFrom, scalarTo, text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingRegion(0, 7))
        editText.setCompositionReplacementRange(7, 7)

        assertTrue(inputConnection.commitText("wouldn't", 1))

        assertEquals(listOf(Triple(0, 7, "wouldn't")), replacements)
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())

        assertTrue(inputConnection.commitText(" ", 1))

        assertEquals(listOf(" " to 8), inserts)
        assertEquals("wouldn't ", editText.text?.toString())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("wouldn't ", editText.text?.toString())
    }

    @Test
    fun `composition commit adopts already visible correction instead of inserting duplicate word`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("wouldnt"), notifyListener = false)
        editText.setSelection(7)
        editText.editorId = 1

        val rendered = StringBuilder("wouldnt")
        val replacements = mutableListOf<Triple<Int, Int, String>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacements.add(Triple(scalarFrom, scalarTo, text))
            rendered.replace(scalarFrom, scalarTo, text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        editText.setCompositionReplacementRange(7, 7)
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 7, "wouldn't")
            Selection.setSelection(editText.text!!, 8, 8)
            true
        }

        assertTrue(inputConnection!!.commitText("wouldn't", 1))

        assertTrue(replacements.isEmpty())
        assertEquals(listOf("'" to 6), inserts)
        assertEquals("wouldn't", editText.text?.toString())
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())

        assertTrue(inputConnection.commitText(" ", 1))

        assertEquals(listOf("'" to 6, " " to 8), inserts)
        assertEquals("wouldn't ", editText.text?.toString())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("wouldn't ", editText.text?.toString())
    }

    @Test
    fun `already visible multi typo correction uses visible replacement range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("woudlnt"), notifyListener = false)
        editText.setSelection(7)
        editText.editorId = 1

        val rendered = StringBuilder("woudlnt")
        val replacements = mutableListOf<Triple<Int, Int, String>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacements.add(Triple(scalarFrom, scalarTo, text))
            rendered.replace(scalarFrom, scalarTo, text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        editText.setCompositionReplacementRange(7, 7)
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 7, "wouldn't")
            Selection.setSelection(editText.text!!, 8, 8)
            true
        }

        assertTrue(inputConnection!!.commitText("wouldn't", 1))

        assertEquals(listOf(Triple(3, 6, "ldn'")), replacements)
        assertTrue(inserts.isEmpty())
        assertEquals("wouldn't", editText.text?.toString())
    }

    @Test
    fun `empty commit text over composing range deletes original range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(6, 11))
        assertTrue(inputConnection.commitText("", 1))

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `commit text honors requested cursor position`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var selectedScalar: Pair<Int, Int>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            selectedScalar = anchor to head
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitText("()", 0))

        assertEquals("()", insertedText)
        assertEquals(6 to 6, selectedScalar)
    }

    @Test
    fun `no-op composition commit honors requested cursor position`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var selectedScalar: Pair<Int, Int>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            selectedScalar = anchor to head
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(0, 3))
        assertTrue(inputConnection.commitText("abc", 0))

        assertNull(replacement)
        assertEquals(0 to 0, selectedScalar)
    }

    @Test
    fun `command preflight flushes visible composing text before toolbar commands`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
            editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        inputConnection!!.setComposingText("abc", 1)

        val ready = editText.prepareForExternalEditorUpdate()

        assertTrue(ready)
        assertEquals("abc", insertedText)
        assertEquals(0, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `command preflight blocks and restores cancelled empty composing text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("", 1))
        assertEquals("", editText.text?.toString())

        val ready = editText.prepareForExternalEditorUpdate()

        assertFalse(ready)
        assertEquals("", editText.text?.toString())
        assertEquals(null, insertedText)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native insertion mutation commits to rust instead of reconciliation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        editText.text!!.insert(6, "brave ")

        assertEquals("brave ", insertedText)
        assertEquals(6, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native multiline insertion uses structured content insertion`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var insertedContent: Triple<Int, Int, String>? = null
        editText.onInsertContentJsonAtSelectionScalarForTesting = { scalarFrom, scalarTo, json ->
            insertedContent = Triple(scalarFrom, scalarTo, json)
        }

        editText.text!!.insert(6, "one\ntwo")

        val (scalarFrom, scalarTo, json) = insertedContent!!
        assertEquals(6, scalarFrom)
        assertEquals(6, scalarTo)
        val content = JSONObject(json).getJSONArray("content")
        assertEquals("one", content.getJSONObject(0).getJSONArray("content").getJSONObject(0).getString("text"))
        assertEquals("two", content.getJSONObject(1).getJSONArray("content").getJSONObject(0).getString("text"))
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native replacement mutation commits to rust instead of reconciliation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.text!!.replace(6, 11, "there")

        assertEquals(Triple(6, 11, "there"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native deletion mutation commits to rust instead of reconciliation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        editText.text!!.delete(5, 6)

        assertEquals(5 to 6, deletedRange)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native emoji replacement snaps diff to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("😀 ok"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.text!!.replace(0, 2, "😁")

        assertEquals(Triple(0, 1, "😁"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native autocorrect immediately after blur commits during blur grace window`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.clearFocus()
        editText.text!!.replace(0, 3, "the")

        assertEquals(Triple(1, 3, "he"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native autocorrect after blur commits even when ime leaves composing span`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.clearFocus()
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "the")
            BaseInputConnection.setComposingSpans(editText.text!!)
            true
        }

        assertTrue(editText.prepareForExternalEditorUpdate())
        assertEquals(Triple(1, 3, "he"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native composing diff after blur is not adopted as final mutation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(6)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.setCompositionReplacementRange(6, 6)
        editText.setComposingTextForEditor("brave ")
        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(6, "braver ")
            BaseInputConnection.setComposingSpans(editText.text!!)
            true
        }
        editText.clearFocus()

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(insertedText)
        assertNull(replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `input trait change after blur suppresses stale direct native mutation adoption`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(6)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val oldConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(oldConnection)
        assertTrue(oldConnection!!.setComposingText("brave ", 1))

        editText.clearFocus()
        editText.setAutoCorrect(false)
        assertEquals("Hello world", editText.text?.toString())

        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(6, "stale ")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(insertedText)
        assertNull(replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native mutation after blur is only adopted once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.clearFocus()
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "the")
            true
        }

        assertTrue(editText.prepareForExternalEditorUpdate())
        assertEquals(Triple(1, 3, "he"), replacement)

        replacement = null
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "tha")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(replacement)
    }

    @Test
    fun `native mutation after blur grace window expires is not adopted`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.clearFocus()
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(800))
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "the")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(replacement)
    }

    @Test
    fun `native mutation after blur is only adopted once after applied update render`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
            editText.applyUpdateJSON(renderUpdateJson("the "), notifyListener = false)
        }
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        editText.clearFocus()
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "the")
            true
        }

        assertTrue(editText.prepareForExternalEditorUpdate())
        assertEquals(Triple(1, 3, "he"), replacement)
        assertEquals("the ", editText.text?.toString())

        replacement = null
        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "tha")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(replacement)
    }

    @Test
    fun `native mutation after blur window clears after skipped authorized render`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            captureApplyUpdateTraceForTesting = true
        }
        editText.applyUpdateJSON(renderUpdateJson("the "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        editText.clearFocus()
        editText.applyUpdateJSON(renderUpdateJson("the "), notifyListener = false)
        assertTrue(editText.lastApplyUpdateTrace()?.skippedRender == true)

        editText.runWithTransientInputMutationGuard {
            editText.text!!.replace(0, 3, "tha")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(replacement)
        assertNull(insertedText)
    }

    @Test
    fun `native autocorrect preserves final ime selection`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)
        editText.editorId = 1

        editText.onReplaceTextInRustForTesting = { _, _, _ -> }

        editText.text!!.replace(0, 3, "the")

        assertEquals(4, editText.selectionStart)
        assertEquals(4, editText.selectionEnd)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native autocorrect preserves backward selection direction`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }
        Selection.setSelection(editText.text, 11, 6)

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.text!!.replace(0, 5, "Hi")

        assertEquals(Triple(1, 5, "i"), replacement)
        assertTrue(editText.selectionStart > editText.selectionEnd)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native autocorrect with stray composing span commits when no composition is tracked`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        BaseInputConnection.setComposingSpans(editText.text!!)
        editText.text!!.replace(0, 3, "the")

        assertEquals(Triple(1, 3, "he"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native autocorrect with tracked composition commits instead of reconciliation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingRegion(0, 3))
        BaseInputConnection.setComposingSpans(editText.text!!)

        editText.text!!.replace(0, 3, "the")

        assertEquals(Triple(1, 3, "he"), replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native insertion at tracked composition boundary commits as final mutation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("word "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        editText.setCompositionReplacementRange(0, 4)
        editText.text!!.insert(4, "!")

        assertEquals("!", insertedText)
        assertEquals(4, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native insertion at collapsed tracked composition range commits at caret`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abcd"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        editText.setCompositionReplacementRange(2, 2)
        editText.text!!.insert(2, "X")

        assertEquals("X", insertedText)
        assertEquals(2, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native mutation outside tracked composition range is not adopted`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh word"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.setCompositionReplacementRange(0, 3)
        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(4, "!")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(insertedText)
        assertNull(replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `focused native composing diff with tracked composing text is not adopted as final mutation`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(6)
        editText.editorId = 1

        var insertedText: String? = null
        var replacement: Triple<Int, Int, String>? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.setCompositionReplacementRange(6, 6)
        editText.setComposingTextForEditor("brave ")
        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(6, "braver ")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(insertedText)
        assertNull(replacement)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native autocorrect retires old input connection before late commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        val replacements = mutableListOf<Triple<Int, Int, String>>()
        var insertedText: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacements.add(Triple(scalarFrom, scalarTo, text))
        }
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val oldConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(oldConnection)
        assertTrue(oldConnection!!.setComposingRegion(0, 3))

        editText.text!!.replace(0, 3, "the")

        assertEquals(listOf(Triple(1, 3, "he")), replacements)

        assertTrue(oldConnection.commitText("the", 1))
        assertTrue(oldConnection.finishComposingText())

        assertEquals(listOf(Triple(1, 3, "he")), replacements)
        assertNull(insertedText)
    }

    @Test
    fun `fresh input connection after native autocorrect accepts new commit`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
            editText.applyUpdateJSON(renderUpdateJson("the "), notifyListener = false)
        }

        val oldConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(oldConnection)

        editText.text!!.replace(0, 3, "the")

        assertEquals(Triple(1, 3, "he"), replacement)

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val freshConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(freshConnection)
        assertTrue(freshConnection!!.commitText("!", 1))

        assertEquals("!", insertedText)
    }

    @Test
    fun `text commit replaces normalized backward selection range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        Selection.setSelection(editText.text, 11, 6)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.handleTextCommit("there")

        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `text commit uses exact Rust selection when its Android projection is ambiguous`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)
            assertEquals(1, harness.editText.selectionStart)

            var insertion: Pair<String, Int>? = null
            harness.editText.onInsertTextInRustForTesting = { text, scalar ->
                insertion = text to scalar
            }

            harness.editText.handleTextCommit("x")

            assertEquals("x" to 13, insertion)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `optimistic commit advances Rust caret before a deferred backspace`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)
            harness.editText.onInsertTextInRustForTesting = { _, _ -> }

            harness.editText.handleTextCommit("x")
            assertEquals("ax", harness.editText.text.toString())

            var backwardSelection: Pair<Int, Int>? = null
            harness.editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
                backwardSelection = anchor to head
            }
            harness.editText.handleBackspace()

            assertEquals(14 to 14, backwardSelection)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `deferred surrounding delete uses the optimistic Rust caret`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)
            harness.editText.onInsertTextInRustForTesting = { _, _ -> }
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.commitText("x", 1))
            var deletedRange: Pair<Int, Int>? = null
            harness.editText.onDeleteRangeInRustForTesting = { from, to ->
                deletedRange = from to to
            }
            assertTrue(inputConnection.deleteSurroundingText(1, 0))

            assertEquals(13 to 14, deletedRange)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `explicit Android caret move replaces the previous Rust selection`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)

            harness.editText.setSelection(0)
            var insertion: Pair<String, Int>? = null
            harness.editText.onInsertTextInRustForTesting = { text, scalar ->
                insertion = text to scalar
            }
            harness.editText.handleTextCommit("x")

            assertEquals("x" to 0, insertion)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `text replacement commit does not optimistically mutate visible text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(0, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitText("the", 1))

        assertEquals(Triple(0, 3, "the"), replacement)
        assertEquals("teh ", editText.text?.toString())
        assertFalse(
            editText.imeTraceSnapshotForTesting().any {
                it.contains("optimisticVisibleTextCommit")
            }
        )
    }

    @Test
    fun `bulk surrounding delete defers render so autocorrect replacement commit survives`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1

        val rendered = StringBuilder("teh")
        val deletes = mutableListOf<Pair<Int, Int>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletes.add(scalarFrom to scalarTo)
            rendered.delete(scalarFrom, scalarTo)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(3, 0))

        assertEquals(listOf(0 to 3), deletes)
        assertEquals("", editText.text?.toString())
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())

        assertTrue(inputConnection.commitText("the", 1))

        assertEquals(listOf("the" to 0), inserts)
        assertEquals("the", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("the", editText.text?.toString())
    }

    @Test
    fun `single character surrounding delete defers render so case replacement commit survives`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("i"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1

        val updates = mutableListOf<String>()
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) {
                updates.add(updateJSON)
            }
        }

        val rendered = StringBuilder("i")
        val deletes = mutableListOf<Pair<Int, Int>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletes.add(scalarFrom to scalarTo)
            rendered.delete(scalarFrom, scalarTo)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(1, 0))

        assertEquals(listOf(0 to 1), deletes)
        assertEquals("", editText.text?.toString())
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertTrue(updates.isEmpty())

        assertTrue(inputConnection.commitText("I", 1))

        assertEquals(listOf("I" to 0), inserts)
        assertEquals("I", editText.text?.toString())
        assertEquals(1, updates.size)

        shadowOf(Looper.getMainLooper()).idle()

        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("I", editText.text?.toString())
        assertEquals(1, updates.size)
    }

    @Test
    fun `bulk surrounding delete no-op does not queue rust delete`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val deletes = mutableListOf<Pair<Int, Int>>()
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletes.add(scalarFrom to scalarTo)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(3, 0))

        assertTrue(deletes.isEmpty())
        assertEquals("abc", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
    }

    @Test
    fun `text commit snaps split surrogate selection to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.handleTextCommit("X")

        assertEquals(Triple(1, 2, "X"), replacement)
    }

    @Test
    fun `selection sync snaps split surrogate selection to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        editText.setSelection(2, 3)

        assertEquals(1 to 2, syncedSelection)
    }

    @Test
    fun `selection sync preserves backward anchor and head direction`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        Selection.setSelection(editText.text, 11, 6)

        assertEquals(11 to 6, syncedSelection)
    }

    @Test
    fun `collapsed composition range snaps split surrogate caret to insertion point`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)

        editText.setCompositionReplacementRange(2, 2)

        assertEquals(3 to 3, editText.compositionReplacementRange())
    }

    @Test
    fun `backspace deletes normalized backward selection range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        Selection.setSelection(editText.text, 11, 6)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        editText.handleBackspace()

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `backspace snaps split surrogate selection to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2, 3)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        editText.handleBackspace()

        assertEquals(1 to 2, deletedRange)
    }

    @Test
    fun `delete surrounding text deletes forward selected range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(1, 0))

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `delete surrounding text in code points deletes backward selected range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        Selection.setSelection(editText.text, 11, 6)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingTextInCodePoints(1, 0))

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `delete surrounding text snaps split surrogate ranges to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(0, 1))

        assertEquals(1 to 2, deletedRange)
    }

    @Test
    fun `plain paste replaces selected range`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "there"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `paste as plain text ignores html and routes plain text through rust`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedHtml: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertContentHtmlInRustForTesting = { html ->
            insertedHtml = html
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(
            ClipData.newHtmlText("html", "there", "<strong>there</strong>")
        )

        assertTrue(editText.onTextContextMenuItem(android.R.id.pasteAsPlainText))

        assertNull(insertedHtml)
        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `plain paste coerces non text clipboard item through rust`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val intent = Intent(Intent.ACTION_VIEW, Uri.parse("https://example.test/share"))
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newIntent("intent", intent))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(
            Triple(6, 11, intent.toUri(Intent.URI_INTENT_SCHEME)),
            replacement
        )
    }

    @Test
    fun `editable cut copies selection and deletes through rust`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        assertTrue(editText.onTextContextMenuItem(android.R.id.cut))

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        assertEquals("world", clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        assertEquals(6 to 11, deletedRange)
        assertEquals("Hello world", editText.text?.toString())
    }

    @Test
    fun `read only cut and paste as plain text are consumed without mutating text`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.isEditable = false

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.cut))
        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))
        assertTrue(editText.onTextContextMenuItem(android.R.id.pasteAsPlainText))
        assertEquals("abc", editText.text?.toString())
    }

    @Test
    fun `editable accessibility set text replaces full document through rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        val args = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "there"
            )
        }

        assertTrue(
            editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                args
            )
        )

        assertEquals(Triple(0, 11, "there"), replacement)
        assertEquals("Hello world", editText.text?.toString())
    }

    @Test
    fun `editable accessibility set text replaces full document when selection is collapsed`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        val args = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "replacement"
            )
        }

        assertTrue(
            editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                args
            )
        )

        assertEquals(Triple(0, 11, "replacement"), replacement)
    }

    @Test
    fun `read only accessibility text mutations are rejected without mutating text`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.isEditable = false

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))
        val setTextArgs = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "X"
            )
        }

        assertFalse(
            editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                setTextArgs
            )
        )
        assertFalse(editText.performAccessibilityAction(AccessibilityNodeInfo.ACTION_PASTE, null))
        assertFalse(editText.performAccessibilityAction(AccessibilityNodeInfo.ACTION_CUT, null))
        assertEquals("abc", editText.text?.toString())
    }

    @Test
    fun `read only input connection consumes printable and forward delete keys`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1
        editText.isEditable = false

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A)))
        assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_SPACE)))
        assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL)))
        assertEquals("abc", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `read only multiple character key events are consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1
        editText.isEditable = false

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        val multipleCharactersEvent = KeyEvent(100L, "é", 0, 0)

        assertTrue(editText.dispatchKeyEvent(multipleCharactersEvent))
        assertTrue(inputConnection!!.sendKeyEvent(multipleCharactersEvent))
        assertEquals("abc", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `plain paste snaps split surrogate selection to scalar boundaries`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(Triple(1, 2, "X"), replacement)
    }

    @Test
    fun `multiline plain paste inserts structured content`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var insertedContent: Triple<Int, Int, String>? = null
        editText.onInsertContentJsonAtSelectionScalarForTesting = { scalarFrom, scalarTo, json ->
            insertedContent = Triple(scalarFrom, scalarTo, json)
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "one\ntwo"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        val (scalarFrom, scalarTo, json) = insertedContent!!
        assertEquals(6, scalarFrom)
        assertEquals(11, scalarTo)
        val content = JSONObject(json).getJSONArray("content")
        assertEquals("one", content.getJSONObject(0).getJSONArray("content").getJSONObject(0).getString("text"))
        assertEquals("two", content.getJSONObject(1).getJSONArray("content").getJSONObject(0).getString("text"))
    }

    @Test
    fun `html paste syncs current selection before inserting html`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }
        var insertedHtml: String? = null
        editText.onInsertContentHtmlInRustForTesting = { html ->
            insertedHtml = html
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(
            ClipData.newHtmlText("html", "there", "<strong>there</strong>")
        )

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(6 to 11, syncedSelection)
        assertEquals("<strong>there</strong>", insertedHtml)
    }

    private fun renderUpdateJson(text: String): String =
        renderBlocksUpdateJson(text)

    private class StructuredDeleteHarness(
        val adapter: EditorV2Adapter,
        val editText: EditorEditText,
        initialBlocks: JSONArray
    ) {
        private var blocks = JSONArray(initialBlocks.toString())

        fun adopt(updateJSON: String) {
            val update = JSONObject(updateJSON)
            update.optJSONArray("renderBlocks")?.let { replacement ->
                blocks = JSONArray(replacement.toString())
                return
            }
            val patch = update.getJSONObject("renderPatch")
            val start = patch.getInt("startIndex")
            val deleteCount = patch.getInt("deleteCount")
            val replacement = patch.getJSONArray("renderBlocks")
            blocks = JSONArray().apply {
                for (index in 0 until start) put(blocks.get(index))
                for (index in 0 until replacement.length()) put(replacement.get(index))
                for (index in start + deleteCount until blocks.length()) {
                    put(blocks.get(index))
                }
            }
        }

        fun expectedText(): String = RenderBridge.buildSpannableFromBlocks(
            blocks,
            baseFontSize = 16f,
            textColor = Color.BLACK
        ).toString()
    }

    private fun structuredDeleteHarness(initialHtml: String): StructuredDeleteHarness {
        val created = UniffiEditorV2Backend.create(
            """{"initialization":{"type":"localEmpty"}}""",
            null
        ) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(
            UniffiEditorV2Backend,
            editorId,
            roomBound = false
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        val initialUpdate = JSONObject(adapter.setContentHtml(initialHtml)!!)
        val harness = StructuredDeleteHarness(
            adapter,
            editText,
            initialUpdate.getJSONArray("renderBlocks")
        )
        editText.applyUpdateJSON(initialUpdate.toString(), notifyListener = false)
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) = harness.adopt(updateJSON)
        }
        return harness
    }

    private fun assertGeneratedBackspaceDoesNotMutateNative(
        editText: EditorEditText,
        selection: Int
    ) {
        editText.editorId = 1
        var routedDeleteCount = 0
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { _, _ ->
            routedDeleteCount += 1
        }
        editText.onDeleteRangeInRustForTesting = { _, _ ->
            routedDeleteCount += 1
        }
        editText.setSelection(selection)
        val before = editText.text.toString()
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!

        assertTrue(inputConnection.deleteSurroundingText(1, 0))

        assertEquals(before, editText.text.toString())
        assertEquals(1, routedDeleteCount)
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
    }

    private fun renderBlocksUpdateJson(vararg texts: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().apply {
                    texts.forEach { put(paragraphRenderBlock(it)) }
                }
            )
            .toString()

    private fun paragraphRenderBlock(text: String): JSONArray =
        JSONArray()
            .put(
                JSONObject()
                    .put("type", "blockStart")
                    .put("nodeType", "paragraph")
                    .put("depth", 0)
            )
            .put(
                JSONObject()
                    .put("type", "textRun")
                    .put("text", text)
                    .put("marks", JSONArray())
            )
            .put(JSONObject().put("type", "blockEnd"))

    private data class ExternalCompositionHarness(
        val backend: FakeEditorV2Backend,
        val editorId: String,
        val adapter: EditorV2Adapter,
        val editText: EditorEditText
    )

    private fun externalCompositionHarness(
        initialText: String,
        configJson: String = """{"initialization":{"type":"localEmpty"}}""",
        roomBound: Boolean = false
    ): ExternalCompositionHarness {
        val backend = FakeEditorV2Backend()
        val created = backend.create(configJson, null) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(backend, editorId, roomBound = roomBound)!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>$initialText</p>")
            ?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        return ExternalCompositionHarness(backend, editorId, adapter, editText)
    }

    private data class RealExternalCompositionHarness(
        val editorId: String,
        val adapter: EditorV2Adapter,
        val editText: EditorEditText
    )

    private fun realExternalCompositionHarness(
        initialText: String,
        configJson: String = """{"initialization":{"type":"localEmpty"}}""",
        roomBound: Boolean = false,
        collaborationWake: (String, CollaborationWakeReason) -> Unit = { _, _ -> }
    ): RealExternalCompositionHarness {
        val created = UniffiEditorV2Backend.create(configJson, null) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(
            UniffiEditorV2Backend,
            editorId,
            roomBound = roomBound,
            collaborationWake = collaborationWake
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>$initialText</p>")
            ?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        return RealExternalCompositionHarness(editorId, adapter, editText)
    }

    private fun assertRealExternalCompositionPolicyFailure(
        configJson: String,
        initialText: String,
        finalText: String
    ) {
        val collaborationWakes = mutableListOf<CollaborationWakeReason>()
        val harness = realExternalCompositionHarness(
            initialText,
            configJson,
            roomBound = true,
            collaborationWake = { _, reason -> collaborationWakes.add(reason) }
        )
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(0, initialText.length)
            val revisionBefore = harness.adapter.baseDocumentRevision
            val canUndoBefore = harness.adapter.historyCanUndo()
            val canRedoBefore = harness.adapter.historyCanRedo()
            collaborationWakes.clear()

            harness.editText.beginExternalTextComposition("speech-policy")
            harness.editText.updateExternalTextComposition("speech-policy", finalText)
            assertTrue(collaborationWakes.isEmpty())
            val resultJson = harness.editText.commitExternalTextComposition(
                "speech-policy",
                finalText
            )
            val duplicate = harness.editText.commitExternalTextComposition(
                "speech-policy",
                "ignored"
            )
            val result = JSONObject(resultJson)

            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("EXTERNAL_COMPOSITION_COMMIT_FAILED", result.errorCode())
            assertExternalCompositionErrorShape(result)
            assertEquals("<p>$initialText</p>", harness.adapter.documentHtml())
            assertEquals(initialText, harness.editText.text.toString())
            assertEquals(revisionBefore, harness.adapter.baseDocumentRevision)
            assertEquals(canUndoBefore, harness.adapter.historyCanUndo())
            assertEquals(canRedoBefore, harness.adapter.historyCanRedo())
            assertTrue(collaborationWakes.isEmpty())
            assertEquals(resultJson, duplicate)
            assertEquals(listOf(resultJson), listener.externalCompositionEnds)
            assertTrue(listener.receivedUpdates.isEmpty())
        } finally {
            harness.adapter.destroy()
        }
    }

    private fun assertExternalCompositionErrorShape(result: JSONObject) {
        val error = result.getJSONObject("error")
        assertEquals(
            setOf(
                "domain",
                "code",
                "message",
                "requestId",
                "operationIndex",
                "limit",
                "actual",
                "details"
            ),
            error.keys().asSequence().toSet()
        )
        assertEquals("lifecycle", error.getString("domain"))
        assertTrue(error.getString("message").isNotEmpty())
        listOf("requestId", "operationIndex", "limit", "actual", "details").forEach {
            assertTrue(error.isNull(it))
        }
    }

    private class RecordingEditorListener : EditorEditText.EditorListener {
        val externalCompositionEnds = mutableListOf<String>()
        val receivedUpdates = mutableListOf<String>()
        val events = mutableListOf<String>()

        override fun onSelectionChanged(anchor: Int, head: Int) = Unit

        override fun onEditorUpdate(updateJSON: String) {
            receivedUpdates.add(updateJSON)
            events.add("update")
        }

        override fun onExternalTextCompositionEnded(resultJson: String) {
            externalCompositionEnds.add(resultJson)
            events.add("external")
        }
    }

    private fun JSONObject.errorCode(): String =
        getJSONObject("error").getString("code")

    private fun withDefaultInputMethod(context: Context, inputMethodId: String, block: () -> Unit) {
        val previous = Settings.Secure.getString(
            context.contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD
        )
        Settings.Secure.putString(
            context.contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD,
            inputMethodId
        )
        try {
            block()
        } finally {
            Settings.Secure.putString(
                context.contentResolver,
                Settings.Secure.DEFAULT_INPUT_METHOD,
                previous
            )
        }
    }

    private infix fun Int.hasInputFlag(flag: Int): Boolean = (this and flag) == flag
}
