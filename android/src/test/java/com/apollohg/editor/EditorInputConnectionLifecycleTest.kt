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
class EditorInputConnectionLifecycleTest : EditorInputConnectionTestSupport() {
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
    fun `replaced deferred patch rebuilds once and preserves the next patch base`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Alpha", "Beta"), notifyListener = false)

        fun patchUpdate(
            fullTexts: List<String>,
            startIndex: Int,
            replacementText: String
        ): String = JSONObject()
            .put(
                "renderElements",
                JSONArray().apply {
                    fullTexts.forEach { text ->
                        val block = paragraphRenderBlock(text)
                        for (index in 0 until block.length()) put(block.get(index))
                    }
                }
            )
            .put(
                "renderPatch",
                JSONObject()
                    .put("startIndex", startIndex)
                    .put("deleteCount", 1)
                    .put("renderBlocks", JSONArray().put(paragraphRenderBlock(replacementText)))
            )
            .toString()

        editText.runWithDeferredRustUpdateApplication {
            editText.applyRustUpdateJSONForTesting(
                patchUpdate(listOf("Alpha 1", "Beta"), 0, "Alpha 1")
            )
            editText.applyRustUpdateJSONForTesting(
                patchUpdate(listOf("Alpha 1", "Beta 2"), 1, "Beta 2")
            )
        }

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("Alpha 1\nBeta 2", editText.text.toString())
        assertFalse(editText.lastRenderAppliedPatch())

        editText.applyUpdateJSON(
            patchUpdate(listOf("Alpha 1!", "Beta 2"), 0, "Alpha 1!"),
            notifyListener = false,
        )

        assertEquals("Alpha 1!\nBeta 2", editText.text.toString())
        assertTrue(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `direct patch advances through a pending deferred patch before applying`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Alpha", "Beta"), notifyListener = false)

        editText.runWithDeferredRustUpdateApplication {
            editText.applyRustUpdateJSONForTesting(
                renderPatchUpdateJson(startIndex = 0, replacementText = "Alpha 1")
            )
        }

        assertTrue(
            editText.applyUpdateJSON(
                renderPatchUpdateJson(startIndex = 1, replacementText = "Beta 2"),
                notifyListener = false,
            )
        )
        assertEquals("Alpha 1\nBeta 2", editText.text.toString())
    }

    @Test
    fun `wrong render patch base recovers a full native snapshot`() {
        val created = UniffiEditorV2Backend.create(
            """{"initialization":{"type":"localEmpty"}}""",
            null,
        ) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(
            UniffiEditorV2Backend,
            editorId,
            roomBound = false,
        )!!
        try {
            adapter.claimNativeBindingIfUnowned(1L)
            val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
                this.editorId = 1L
                v2Driver = adapter
            }
            val initialUpdate = adapter.setContentHtml("<p>Alpha</p>")!!
            assertTrue(editText.applyUpdateJSON(initialUpdate, notifyListener = false))
            val revision = JSONObject(initialUpdate).getString("documentVersion")
            val wrongBase = if (revision == "0") "1" else "0"
            val stalePatch = JSONObject()
                .put("documentVersion", revision)
                .put(
                    "renderPatch",
                    JSONObject()
                        .put("baseDocumentVersion", wrongBase)
                        .put("startIndex", 0)
                        .put("deleteCount", 1)
                        .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Corrupt"))),
                )
                .toString()

            assertTrue(editText.applyUpdateJSON(stalePatch, notifyListener = false))
            assertEquals("Alpha", editText.text.toString())
            assertFalse(editText.lastRenderAppliedPatch())

            val nextUpdate = adapter.insertText("!", atScalarPos = 5)!!
            assertTrue(editText.applyUpdateJSON(nextUpdate, notifyListener = false))
            assertEquals("Alpha!", editText.text.toString())
        } finally {
            adapter.destroy()
        }
    }

    @Test
    fun `authorizing optimistic text cannot skip an authoritative rebuild`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Alpha"), notifyListener = false)
        editText.text!!.replace(0, editText.text!!.length, "Rejected")
        editText.authorizeCurrentVisibleTextForPendingImeOperationForEditor()

        assertTrue(editText.applyUpdateJSON(renderBlocksUpdateJson("Alpha"), notifyListener = false))
        assertEquals("Alpha", editText.text.toString())
        assertEquals("Alpha", editText.authorizedTextForTesting())
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
}
