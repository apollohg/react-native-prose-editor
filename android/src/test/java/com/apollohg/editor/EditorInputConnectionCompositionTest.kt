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
internal class EditorInputConnectionCompositionTest : EditorInputConnectionTestSupport() {
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
}
