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
internal class EditorInputConnectionDeletionTest : EditorInputConnectionTestFixture() {
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
                    "seed=$seed step=$step offset=$offset\n" + harness.editText.imeTraceSnapshotForTesting().joinToString("\n"),
                    harness.expectedText(),
                    harness.editText.text.toString()
                )
            }
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
}
