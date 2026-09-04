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
class EditorInputConnectionTest : EditorInputConnectionTestSupport() {
    @Test
    fun `atom boundary does not expose a cursor`() {
        val activity = Robolectric.buildActivity(Activity::class.java)
            .setup()
            .visible()
            .windowFocusChanged(true)
            .get()
        val editText = terminalAtomEditText(activity)
        activity.setContentView(editText)
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.AT_MOST),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        assertTrue(editText.requestFocus())

        for (offset in listOf(0, editText.text!!.length)) {
            editText.setSelection(offset)
            assertNull(editText.nativeCursorDrawRect())
            assertFalse(editText.isCursorVisible)
        }
    }

    @Test
    fun `atom boundary selection restores last paragraph caret`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity, paragraphThenAtomRenderJson())
        activity.setContentView(editText)
        editText.setSelection(3)
        val atomOffset = editText.text!!.length - 1

        for (offset in listOf(atomOffset, atomOffset + 1)) {
            editText.setSelection(offset)
            assertEquals(3, editText.selectionStart)
            assertEquals(3, editText.selectionEnd)
        }
    }

    @Test
    fun `tap on atom line preserves paragraph caret`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity, paragraphThenAtomRenderJson())
        activity.setContentView(editText)
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.AT_MOST),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        val textLayout = requireNotNull(editText.layout)
        val atomOffset = editText.text!!.length - 1
        val atomLine = textLayout.getLineForOffset(atomOffset)
        val tapX = editText.totalPaddingLeft + textLayout.width - 4f
        val tapY = editText.totalPaddingTop +
            (textLayout.getLineTop(atomLine) + textLayout.getLineBottom(atomLine)) / 2f
        editText.setSelection(3)

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()
        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(3, editText.selectionStart)
        assertEquals(3, editText.selectionEnd)
    }

    @Test
    fun `terminal atom does not add auto grow height`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        activity.setContentView(editText)
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(500, View.MeasureSpec.AT_MOST),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        val textLayout = requireNotNull(editText.layout)
        val expectedHeight = textLayout.height +
            editText.compoundPaddingTop + editText.compoundPaddingBottom

        assertEquals(expectedHeight, editText.resolveAutoGrowHeight())
    }

    @Test
    fun `terminal atom boundary ignores committed text`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        editText.setSelection(editText.text!!.length)
        var insertion: Pair<String, Int>? = null
        editText.onInsertTextInRustForTesting = { text, scalar -> insertion = text to scalar }

        editText.handleTextCommit("x")

        assertNull(insertion)
    }

    @Test
    fun `terminal atom boundary ignores return`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        editText.setSelection(editText.text!!.length)
        var splitPosition: Int? = null
        editText.onSplitBlockInRustForTesting = { scalar -> splitPosition = scalar }

        editText.handleTextCommit("\n")

        assertNull(splitPosition)
    }

    @Test
    fun `terminal atom boundary ignores backspace`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        editText.setSelection(editText.text!!.length)
        var deletion: Pair<Int, Int>? = null
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            deletion = anchor to head
        }

        editText.handleBackspace()

        assertNull(deletion)
    }

    private fun terminalAtomEditText(
        activity: Activity,
        renderJson: String =
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1,"atomId":"counter-1"}]""",
    ): EditorEditText =
        EditorEditText(activity).apply {
            applyAtomRenderConfiguration(
                AtomRenderConfiguration(
                    registeredNodeTypes = setOf("counterCard"),
                    estimatedHeightsDp = mapOf("counterCard" to 72f),
                    measuredHeightsPx = emptyMap(),
                )
            )
            applyRenderJSON(renderJson)
            editorId = 9_001L
        }

    private fun paragraphThenAtomRenderJson(): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Before","marks":[]},
          {"type":"blockEnd"},
          {"type":"voidBlock","nodeType":"counterCard","docPos":8,"atomId":"counter-1"}
        ]
        """.trimIndent()

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

}
