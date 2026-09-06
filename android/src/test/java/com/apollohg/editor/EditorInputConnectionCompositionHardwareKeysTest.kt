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
internal class EditorInputConnectionCompositionHardwareKeysTest : EditorInputConnectionTestSupport() {
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
}
