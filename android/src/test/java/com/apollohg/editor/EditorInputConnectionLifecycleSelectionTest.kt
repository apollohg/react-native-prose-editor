package com.apollohg.editor
import android.graphics.Color
import android.os.Looper
import android.text.Selection
import android.text.style.AbsoluteSizeSpan
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.robolectric.Shadows.shadowOf
import org.robolectric.RuntimeEnvironment
import java.time.Duration
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputConnectionLifecycleSelectionTest : EditorInputConnectionTestSupport() {
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
    fun `local selection drop routes a move through Rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val update = JSONObject(renderUpdateJson("abcd"))
            .put("documentVersion", "1")
            .toString()
        editText.applyUpdateJSON(update, notifyListener = false)
        editText.editorId = 1
        var moved: Triple<Int, Int, Int>? = null
        editText.onMoveSelectionScalarForTesting = { from, to, destination ->
            moved = Triple(from, to, destination)
        }

        assertTrue(editText.performLocalSelectionDropForTesting(0, 2, 4, "1"))
        assertEquals(Triple(0, 2, 4), moved)
    }

    @Test
    fun `local selection drop rejects a stale document revision`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val update = JSONObject(renderUpdateJson("abcd"))
            .put("documentVersion", "2")
            .toString()
        editText.applyUpdateJSON(update, notifyListener = false)
        editText.editorId = 1
        var moved = false
        editText.onMoveSelectionScalarForTesting = { _, _, _ -> moved = true }

        assertFalse(editText.performLocalSelectionDropForTesting(0, 2, 4, "1"))
        assertFalse(moved)
    }

    @Test
    fun `local selection drop rejects a missing document revision`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abcd"), notifyListener = false)
        editText.editorId = 1
        editText.onMoveSelectionScalarForTesting = { _, _, _ -> error("must not move") }

        assertFalse(editText.performLocalSelectionDropForTesting(0, 2, 4, null))
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
}
