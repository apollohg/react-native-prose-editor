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
internal class EditorInputConnectionLifecycleHardwareKeysTest : EditorInputConnectionTestSupport() {
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
}
