package com.apollohg.editor
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.os.Looper
import android.provider.Settings
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.InputType
import android.text.style.AbsoluteSizeSpan
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.RobolectricTestRunner
import org.robolectric.Robolectric
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputConnectionInputTest : EditorInputConnectionTestFixture() {
    @Test
    fun `plain input recovery falls back when an incremental render cannot apply`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyRenderJSON(
            JSONArray()
                .put(
                    JSONObject()
                        .put("type", "blockStart")
                        .put("nodeType", "paragraph")
                        .put("depth", 0),
                )
                .put(
                    JSONObject()
                        .put("type", "textRun")
                        .put("text", "Alpha")
                        .put("marks", JSONArray()),
                )
                .put(JSONObject().put("type", "blockEnd"))
                .toString(),
        )
        editText.setSelection(5)
        val authoritative = editText.captureAuthoritativeInputSnapshotForEditor()
        editText.runWithTransientInputMutationGuard {
            editText.text!!.delete(4, 5)
            true
        }
        val patchOnlyRecovery = JSONObject()
            .put(
                "renderPatch",
                JSONObject()
                    .put("startIndex", 0)
                    .put("deleteCount", 1)
                    .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Alpha"))),
            )
            .toString()

        editText.restoreAuthoritativeInputForEditor(authoritative, patchOnlyRecovery)

        assertEquals("Alpha", editText.text.toString())
        assertEquals("Alpha", editText.authorizedTextForTesting())
        assertEquals(5, editText.selectionStart)
        assertEquals(5, editText.selectionEnd)
    }

    @Test
    fun `keyboard input commits external phrase before typing`() {
        val backend = FakeEditorV2Backend()
        val created = backend.create("""{"initialization":{"type":"localEmpty"}}""", null)
            as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(backend, editorId, roomBound = false)!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>arrival</p>")
            ?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        editText.setSelection(0, 7)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!
        editText.beginExternalTextComposition("speech-1")
        editText.updateExternalTextComposition("speech-1", "O/A")

        assertTrue(inputConnection.commitText("!", 1))

        assertEquals("O/A!", editText.text.toString())
    }

    @Test
    fun `editor input traits use rich text defaults`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        assertEquals(InputType.TYPE_CLASS_TEXT, editText.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_MULTI_LINE)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_AUTO_CORRECT)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
    }

    @Test
    fun `editor input traits apply React keyboard props`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        editText.setKeyboardType("email-address")
        editText.setAutoCapitalize("none")
        editText.setAutoCorrect(false)

        assertEquals(InputType.TYPE_CLASS_TEXT, editText.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_MULTI_LINE)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS)
        assertFalse(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_AUTO_CORRECT)
        assertFalse(editText.inputType hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
    }

    @Test
    fun `private IME option changes restart focused input only when value changes`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        assertTrue(editText.requestFocus())
        fun restartCount() = editText.imeTraceSnapshotForTesting().count {
            it.contains("restartInput:source=privateImeOptions")
        }

        editText.setPrivateImeOptionsForEditor("nm")
        assertEquals(1, restartCount())

        editText.setPrivateImeOptionsForEditor("nm")
        assertEquals(1, restartCount())

        editText.setPrivateImeOptionsForEditor(null)
        assertEquals(2, restartCount())
    }

    @Test
    fun `all surrounding text queries use placeholder free coordinates and retain styles`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val raw = SpannableStringBuilder("\u200Ba\uD83D\uDE00\u200Bb").apply {
            setSpan(AbsoluteSizeSpan(30), 1, length, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        editText.setText(raw)
        editText.setSelection(5)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        val before = requireNotNull(
            inputConnection.getTextBeforeCursor(20, InputConnection.GET_TEXT_WITH_STYLES),
        )
        assertEquals("a\uD83D\uDE00", before.toString())
        assertTrue(before is Spanned)
        assertEquals(1, (before as Spanned).getSpans(0, before.length, AbsoluteSizeSpan::class.java).size)
        assertEquals("b", inputConnection.getTextAfterCursor(20, 0).toString())

        editText.setSelection(1, 6)
        assertEquals("a\uD83D\uDE00b", inputConnection.getSelectedText(0).toString())

        editText.setSelection(1)
        assertNull(inputConnection.getSelectedText(0))

        editText.setSelection(0, 1)
        assertNull(inputConnection.getSelectedText(0))
    }

    @Test
    fun `correction offsets map past synthetic placeholders`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("\u200Bteh"), notifyListener = false)
        editText.editorId = 1
        editText.setSelection(editText.text!!.length)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacement = Triple(from, to, text)
        }
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertEquals(Triple(1, 4, "the"), replacement)
    }

    @Test
    fun `explicit correction maps both ends around an interior synthetic placeholder`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("te\u200Bh"), notifyListener = false)
        editText.editorId = 1
        editText.setSelection(editText.text!!.length)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacement = Triple(from, to, text)
        }
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "teh", "the")))

        assertEquals(Triple(0, 4, "the"), replacement)
    }

    @Test
    fun `inferred correction maps its visible token around an interior synthetic placeholder`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("te\u200Bh "), notifyListener = false)
        editText.editorId = 1
        editText.setSelection(editText.text!!.length)
        var replacement: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacement = Triple(from, to, text)
        }
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        assertTrue(inputConnection.commitCorrection(CorrectionInfo(2, null, "the")))

        assertEquals(Triple(0, 4, "the"), replacement)
    }

    @Test
    fun `initial surrounding text removes synthetic placeholder for IME sentence caps`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        val editorInfo = EditorInfo()
        val inputConnection = editText.onCreateInputConnection(editorInfo)
        assertNotNull(inputConnection)

        assertEquals("Hello\n", editorInfo.getInitialTextBeforeCursor(20, 0).toString())
        assertEquals(editText.selectionStart - 1, editorInfo.initialSelStart)
        assertEquals(editText.selectionEnd - 1, editorInfo.initialSelEnd)
        assertFalse(editorInfo.getInitialTextBeforeCursor(20, 0).toString().contains("\u200B"))
        assertTrue(
            editorInfo.initialCapsMode hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )
    }

    @Test
    fun `editor numeric keyboard type maps to numeric input class`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        editText.setKeyboardType("numeric")

        assertEquals(InputType.TYPE_CLASS_NUMBER, editText.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_NUMBER_FLAG_DECIMAL)
        assertTrue(editText.inputType hasInputFlag InputType.TYPE_NUMBER_FLAG_SIGNED)
    }

    @Test
    fun `input trait changes stale old connection and fresh connection accepts input`() {
        val traitChanges: List<(EditorEditText) -> Unit> = listOf(
            { it.setAutoCorrect(false) },
            { it.setKeyboardType("email-address") }
        )

        for (changeTrait in traitChanges) {
            val editText = EditorEditText(RuntimeEnvironment.getApplication())
            editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
            editText.setSelection(3)
            editText.editorId = 1
            editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

            var insertedText: String? = null
            var insertedScalar: Int? = null
            editText.onInsertTextInRustForTesting = { text, scalar ->
                insertedText = text
                insertedScalar = scalar
            }

            val oldConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(oldConnection)

            changeTrait(editText)

            assertTrue(oldConnection!!.commitText("old", 1))
            assertNull(insertedText)

            val freshConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(freshConnection)
            assertTrue(freshConnection!!.commitText("fresh", 1))

            assertEquals("fresh", insertedText)
        }
    }

    @Test
    fun `external clear keeps same editor input connection accepting input`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("sent"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(4)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.applyUpdateJSON(
            renderUpdateJson("\u200B"),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertTrue(
            editText.imeTraceSnapshotForTesting().any {
                it.contains("restartInput:source=externalUpdate")
            }
        )

        assertTrue(inputConnection!!.commitText("fresh", 1))

        assertEquals("fresh", insertedText)
    }

    @Test
    fun `external clear invalidates rendered editor content`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("sent"), notifyListener = false)
        shadowOf(editText).clearWasInvalidated()

        editText.applyUpdateJSON(
            renderUpdateJson(""),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )

        assertEquals("", editText.text?.toString())
        assertTrue(shadowOf(editText).wasInvalidated())
    }

    @Test
    fun `external clear after deferred Rust update clears stale visible text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(0)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.runWithDeferredRustUpdateApplication {
            editText.runWithTransientInputMutationGuard {
                editText.text!!.insert(0, "second")
                editText.setSelection(6)
                true
            }
            editText.applyRustUpdateJSONForTesting(renderUpdateJson("second"))
        }

        assertTrue(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("second", editText.text?.toString())

        editText.applyUpdateJSON(
            renderUpdateJson(""),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
        assertEquals("", editText.text?.toString())

        assertTrue(inputConnection!!.commitText("next", 1))
        assertEquals("next", insertedText)
    }

    @Test
    fun `external clear after preflight native mutation keeps same editor input connection accepting input`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(0)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var renderedText = ""
        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            renderedText = renderedText.substring(0, scalar.coerceIn(0, renderedText.length)) +
                text +
                renderedText.substring(scalar.coerceIn(0, renderedText.length))
            editText.applyUpdateJSON(renderUpdateJson(renderedText), notifyListener = false)
            editText.setSelection(renderedText.length)
        }

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(0, "second")
            editText.setSelection(6)
            true
        }

        assertTrue(editText.prepareForExternalEditorUpdate())
        assertEquals("second", insertedText)
        assertEquals("second", editText.text?.toString())

        renderedText = ""
        insertedText = null
        editText.applyUpdateJSON(
            renderUpdateJson("\u200B"),
            notifyListener = false,
            refreshInputConnectionForExternalUpdate = true
        )
        editText.setSelection(editText.text?.length ?: 0)

        assertEquals("\u200B", editText.text?.toString())
        assertTrue(inputConnection!!.commitText("next", 1))
        assertEquals("next", insertedText)
    }

    @Test
    fun `destroyed editor input session consumes IME changes without Rust mutation`() {
        val editorId = 880001L
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("abc"), notifyListener = false)
        editText.setSelection(3)
        editText.editorId = editorId

        var insertedText: String? = null
        var replacedText: Triple<Int, Int, String>? = null
        var deletedRange: Pair<Int, Int>? = null
        var deletedBackward: Pair<Int, Int>? = null
        var syncedSelection: Pair<Int, Int>? = null
        editText.onInsertTextInRustForTesting = { text, _ -> insertedText = text }
        editText.onReplaceTextInRustForTesting = { from, to, text ->
            replacedText = Triple(from, to, text)
        }
        editText.onDeleteRangeInRustForTesting = { from, to -> deletedRange = from to to }
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            deletedBackward = anchor to head
        }
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }

        NativeEditorViewRegistry.markEditorCreated(editorId)
        try {
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)

            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)

            assertTrue(inputConnection!!.commitText("x", 1))
            assertTrue(inputConnection.commitCompletion(CompletionInfo(0, 0, "done")))
            assertTrue(inputConnection.commitCorrection(CorrectionInfo(0, "abc", "xyz")))
            assertTrue(inputConnection.setComposingText("z", 1))
            assertTrue(inputConnection.deleteSurroundingText(1, 0))
            editText.setSelection(0)

            assertNull(insertedText)
            assertNull(replacedText)
            assertNull(deletedRange)
            assertNull(deletedBackward)
            assertNull(syncedSelection)
        } finally {
            NativeEditorViewRegistry.markEditorCreated(editorId)
        }
    }

    @Test
    fun `old input connection remains usable after framework recreation from render`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        val inserted = mutableListOf<Pair<String, Int>>()
        val rendered = StringBuilder()
        editText.onInsertTextInRustForTesting = { text, scalar ->
            inserted.add(text to scalar)
            rendered.insert(scalar.coerceIn(0, rendered.length), text)
            editText.applyUpdateJSON(renderUpdateJson(rendered.toString()), notifyListener = false)
            editText.setSelection(rendered.length)
        }

        val originalConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(originalConnection)
        assertTrue(originalConnection!!.commitText("a", 1))

        val recreatedConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(recreatedConnection)

        assertTrue(originalConnection.commitText("b", 1))

        assertEquals(listOf("a" to 0, "b" to 1), inserted)
        assertEquals("ab", editText.text?.toString())
    }

    @Test
    fun `input trait change suppresses stale direct native mutation adoption`() {
        val traitChanges: List<(EditorEditText) -> Unit> = listOf(
            { it.setAutoCorrect(false) },
            { it.setKeyboardType("email-address") }
        )

        for (changeTrait in traitChanges) {
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

            changeTrait(editText)
            assertEquals("Hello world", editText.text?.toString())

            editText.runWithTransientInputMutationGuard {
                editText.text!!.insert(6, "stale ")
                true
            }

            assertFalse(editText.prepareForExternalEditorUpdate())
            assertNull(insertedText)
            assertNull(replacement)
        }
    }

    @Test
    fun `native mutation adoption suppression clears after authorized render update`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(6)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val oldConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(oldConnection)
        assertTrue(oldConnection!!.setComposingText("brave ", 1))

        editText.setAutoCorrect(false)
        editText.runWithTransientInputMutationGuard {
            editText.text!!.insert(6, "stale ")
            true
        }

        assertFalse(editText.prepareForExternalEditorUpdate())
        assertNull(insertedText)
        assertNull(insertedScalar)

        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        editText.text!!.insert(6, "fresh ")

        assertEquals("fresh ", insertedText)
        assertEquals(6, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }

    @Test
    fun `native mutation adoption suppression clears after skipped authorized render update`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            captureApplyUpdateTraceForTesting = true
        }
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.setSelection(6)
        editText.editorId = 1
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        var insertedText: String? = null
        var insertedScalar: Int? = null
        editText.onInsertTextInRustForTesting = { text, scalar ->
            insertedText = text
            insertedScalar = scalar
        }

        val oldConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(oldConnection)
        assertTrue(oldConnection!!.setComposingText("brave ", 1))

        editText.setAutoCorrect(false)
        assertEquals("Hello world", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.lastApplyUpdateTrace()?.skippedRender == true)

        editText.text!!.insert(6, "fresh ")

        assertEquals("fresh ", insertedText)
        assertEquals(6, insertedScalar)
        assertEquals(0, editText.reconciliationCount)
    }
}
