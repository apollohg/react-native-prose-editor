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
internal class EditorInputConnectionLifecycleTest : EditorInputConnectionTestSupport() {
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
}
