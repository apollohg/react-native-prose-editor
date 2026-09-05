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
class EditorInputConnectionCompositionTest : EditorInputConnectionTestSupport() {
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
}
