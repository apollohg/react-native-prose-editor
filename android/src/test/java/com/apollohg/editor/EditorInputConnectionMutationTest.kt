package com.apollohg.editor

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Looper
import android.text.Selection
import android.view.KeyEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.robolectric.Shadows.shadowOf
import org.robolectric.RuntimeEnvironment
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorInputConnectionMutationTest : EditorInputConnectionTestSupport() {
    @Test
    fun `text commit replaces normalized backward selection range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        Selection.setSelection(editText.text, 11, 6)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.handleTextCommit("there")

        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `text commit uses exact Rust selection when its Android projection is ambiguous`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)
            assertEquals(1, harness.editText.selectionStart)

            var insertion: Pair<String, Int>? = null
            harness.editText.onInsertTextInRustForTesting = { text, scalar ->
                insertion = text to scalar
            }

            harness.editText.handleTextCommit("x")

            assertEquals("x" to 13, insertion)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `optimistic commit advances Rust caret before a deferred backspace`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)
            harness.editText.onInsertTextInRustForTesting = { _, _ -> }

            harness.editText.handleTextCommit("x")
            assertEquals("ax", harness.editText.text.toString())

            var backwardSelection: Pair<Int, Int>? = null
            harness.editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
                backwardSelection = anchor to head
            }
            harness.editText.handleBackspace()

            assertEquals(14 to 14, backwardSelection)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `deferred surrounding delete uses the optimistic Rust caret`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)
            harness.editText.onInsertTextInRustForTesting = { _, _ -> }
            val inputConnection = harness.editText.onCreateInputConnection(EditorInfo())!!

            assertTrue(inputConnection.commitText("x", 1))
            var deletedRange: Pair<Int, Int>? = null
            harness.editText.onDeleteRangeInRustForTesting = { from, to ->
                deletedRange = from to to
            }
            assertTrue(inputConnection.deleteSurroundingText(1, 0))

            assertEquals(13 to 14, deletedRange)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `explicit Android caret move replaces the previous Rust selection`() {
        val harness = externalCompositionHarness("a")
        try {
            val update = JSONObject(renderUpdateJson("a"))
                .put("scalarLength", 13)
                .put(
                    "selection",
                    JSONObject()
                        .put("type", "text")
                        .put("anchor", 14)
                        .put("head", 14)
                        .put("anchorScalar", 13)
                        .put("headScalar", 13)
                )
            harness.editText.applyUpdateJSON(update.toString(), notifyListener = false)

            harness.editText.setSelection(0)
            var insertion: Pair<String, Int>? = null
            harness.editText.onInsertTextInRustForTesting = { text, scalar ->
                insertion = text to scalar
            }
            harness.editText.handleTextCommit("x")

            assertEquals("x" to 0, insertion)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `text replacement commit does not optimistically mutate visible text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh "), notifyListener = false)
        editText.setSelection(0, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.commitText("the", 1))

        assertEquals(Triple(0, 3, "the"), replacement)
        assertEquals("teh ", editText.text?.toString())
        assertFalse(
            editText.imeTraceSnapshotForTesting().any {
                it.contains("optimisticVisibleTextCommit")
            }
        )
    }

    @Test
    fun `bulk surrounding delete defers render so autocorrect replacement commit survives`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("teh"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = 1

        val rendered = StringBuilder("teh")
        val deletes = mutableListOf<Pair<Int, Int>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletes.add(scalarFrom to scalarTo)
            rendered.delete(scalarFrom, scalarTo)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(3, 0))

        assertEquals(listOf(0 to 3), deletes)
        assertEquals("", editText.text?.toString())
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())

        assertTrue(inputConnection.commitText("the", 1))

        assertEquals(listOf("the" to 0), inserts)
        assertEquals("the", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("the", editText.text?.toString())
    }

    @Test
    fun `single character surrounding delete defers render so case replacement commit survives`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("i"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1

        val updates = mutableListOf<String>()
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) {
                updates.add(updateJSON)
            }
        }

        val rendered = StringBuilder("i")
        val deletes = mutableListOf<Pair<Int, Int>>()
        val inserts = mutableListOf<Pair<String, Int>>()
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletes.add(scalarFrom to scalarTo)
            rendered.delete(scalarFrom, scalarTo)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserts.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyRustUpdateJSONForTesting(renderUpdateJson(rendered.toString()))
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(1, 0))

        assertEquals(listOf(0 to 1), deletes)
        assertEquals("", editText.text?.toString())
        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertTrue(updates.isEmpty())

        assertTrue(inputConnection.commitText("I", 1))

        assertEquals(listOf("I" to 0), inserts)
        assertEquals("I", editText.text?.toString())
        assertEquals(1, updates.size)

        shadowOf(Looper.getMainLooper()).idle()

        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("I", editText.text?.toString())
        assertEquals(1, updates.size)
    }

    @Test
    fun `bulk surrounding delete no-op does not queue rust delete`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1

        val deletes = mutableListOf<Pair<Int, Int>>()
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletes.add(scalarFrom to scalarTo)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(3, 0))

        assertTrue(deletes.isEmpty())
        assertEquals("abc", editText.text?.toString())
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
    }

    @Test
    fun `text commit snaps split surrogate selection to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        editText.handleTextCommit("X")

        assertEquals(Triple(1, 2, "X"), replacement)
    }

    @Test
    fun `selection sync snaps split surrogate selection to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        editText.setSelection(2, 3)

        assertEquals(1 to 2, syncedSelection)
    }

    @Test
    fun `selection sync preserves backward anchor and head direction`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        Selection.setSelection(editText.text, 11, 6)

        assertEquals(11 to 6, syncedSelection)
    }

    @Test
    fun `collapsed composition range snaps split surrogate caret to insertion point`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)

        editText.setCompositionReplacementRange(2, 2)

        assertEquals(3 to 3, editText.compositionReplacementRange())
    }

    @Test
    fun `backspace deletes normalized backward selection range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        Selection.setSelection(editText.text, 11, 6)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        editText.handleBackspace()

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `backspace snaps split surrogate selection to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2, 3)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        editText.handleBackspace()

        assertEquals(1 to 2, deletedRange)
    }

    @Test
    fun `delete surrounding text deletes forward selected range`() {
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

        assertTrue(inputConnection!!.deleteSurroundingText(1, 0))

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `delete surrounding text in code points deletes backward selected range`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        Selection.setSelection(editText.text, 11, 6)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingTextInCodePoints(1, 0))

        assertEquals(6 to 11, deletedRange)
    }

    @Test
    fun `delete surrounding text snaps split surrogate ranges to scalar boundaries`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.deleteSurroundingText(0, 1))

        assertEquals(1 to 2, deletedRange)
    }

    @Test
    fun `plain paste replaces selected range`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "there"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `paste as plain text ignores html and routes plain text through rust`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        var insertedHtml: String? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        editText.onInsertContentHtmlInRustForTesting = { html ->
            insertedHtml = html
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(
            ClipData.newHtmlText("html", "there", "<strong>there</strong>")
        )

        assertTrue(editText.onTextContextMenuItem(android.R.id.pasteAsPlainText))

        assertNull(insertedHtml)
        assertEquals(Triple(6, 11, "there"), replacement)
    }

    @Test
    fun `plain paste coerces non text clipboard item through rust`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val intent = Intent(Intent.ACTION_VIEW, Uri.parse("https://example.test/share"))
        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newIntent("intent", intent))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(
            Triple(6, 11, intent.toUri(Intent.URI_INTENT_SCHEME)),
            replacement
        )
    }

    @Test
    fun `editable cut copies selection and deletes through rust`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        assertTrue(editText.onTextContextMenuItem(android.R.id.cut))

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        assertEquals("world", clipboard.primaryClip?.getItemAt(0)?.text?.toString())
        assertEquals(6 to 11, deletedRange)
        assertEquals("Hello world", editText.text?.toString())
    }

    @Test
    fun `read only cut and paste as plain text are consumed without mutating text`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.isEditable = false

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.cut))
        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))
        assertTrue(editText.onTextContextMenuItem(android.R.id.pasteAsPlainText))
        assertEquals("abc", editText.text?.toString())
    }

    @Test
    fun `editable accessibility set text replaces full document through rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        val args = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "there"
            )
        }

        assertTrue(
            editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                args
            )
        )

        assertEquals(Triple(0, 11, "there"), replacement)
        assertEquals("Hello world", editText.text?.toString())
    }

    @Test
    fun `editable accessibility set text replaces full document when selection is collapsed`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }
        val args = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "replacement"
            )
        }

        assertTrue(
            editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                args
            )
        )

        assertEquals(Triple(0, 11, "replacement"), replacement)
    }

    @Test
    fun `read only accessibility text mutations are rejected without mutating text`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(0, 3)
        editText.isEditable = false

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))
        val setTextArgs = Bundle().apply {
            putCharSequence(
                AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                "X"
            )
        }

        assertFalse(
            editText.performAccessibilityAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                setTextArgs
            )
        )
        assertFalse(editText.performAccessibilityAction(AccessibilityNodeInfo.ACTION_PASTE, null))
        assertFalse(editText.performAccessibilityAction(AccessibilityNodeInfo.ACTION_CUT, null))
        assertEquals("abc", editText.text?.toString())
    }

    @Test
    fun `read only input connection consumes printable and forward delete keys`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1
        editText.isEditable = false

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertTrue(inputConnection!!.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A)))
        assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_SPACE)))
        assertTrue(inputConnection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL)))
        assertEquals("abc", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `read only multiple character key events are consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(1)
        editText.editorId = 1
        editText.isEditable = false

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        val multipleCharactersEvent = KeyEvent(100L, "é", 0, 0)

        assertTrue(editText.dispatchKeyEvent(multipleCharactersEvent))
        assertTrue(inputConnection!!.sendKeyEvent(multipleCharactersEvent))
        assertEquals("abc", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `plain paste snaps split surrogate selection to scalar boundaries`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("A😀B"), notifyListener = false)
        editText.setSelection(2, 3)
        editText.editorId = 1

        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            replacement = Triple(scalarFrom, scalarTo, text)
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "X"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(Triple(1, 2, "X"), replacement)
    }

    @Test
    fun `multiline plain paste replaces its range atomically`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var insertedContent: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            insertedContent = Triple(scalarFrom, scalarTo, text)
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(ClipData.newPlainText("plain", "one\ntwo"))

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        val (scalarFrom, scalarTo, text) = insertedContent!!
        assertEquals(6, scalarFrom)
        assertEquals(11, scalarTo)
        assertEquals("one\ntwo", text)
    }

    @Test
    fun `html paste syncs current selection before inserting html`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.setSelection(6, 11)
        editText.editorId = 1

        var syncedSelection: Pair<Int, Int>? = null
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }
        var insertedHtml: String? = null
        editText.onInsertContentHtmlInRustForTesting = { html ->
            insertedHtml = html
        }

        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        clipboard.setPrimaryClip(
            ClipData.newHtmlText("html", "there", "<strong>there</strong>")
        )

        assertTrue(editText.onTextContextMenuItem(android.R.id.paste))

        assertEquals(6 to 11, syncedSelection)
        assertEquals("<strong>there</strong>", insertedHtml)
    }
}
