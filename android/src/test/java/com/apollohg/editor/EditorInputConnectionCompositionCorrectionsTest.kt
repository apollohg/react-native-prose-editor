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
internal class EditorInputConnectionCompositionCorrectionsTest : EditorInputConnectionTestSupport() {
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
}
