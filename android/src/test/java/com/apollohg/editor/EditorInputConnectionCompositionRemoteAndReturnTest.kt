package com.apollohg.editor
import android.app.Activity
import android.os.Looper
import android.text.InputType
import android.view.KeyEvent
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.robolectric.Shadows.shadowOf
import org.robolectric.Robolectric
import org.robolectric.RuntimeEnvironment
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputConnectionCompositionRemoteAndReturnTest : EditorInputConnectionTestSupport() {
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
    fun `multiline composition commits its original range atomically`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var insertedContent: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            insertedContent = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.setComposingRegion(6, 11))
        assertTrue(inputConnection.commitText("one\ntwo", 1))

        val (scalarFrom, scalarTo, text) = insertedContent!!
        assertEquals(6, scalarFrom)
        assertEquals(11, scalarTo)
        assertEquals("one\ntwo", text)
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
}
