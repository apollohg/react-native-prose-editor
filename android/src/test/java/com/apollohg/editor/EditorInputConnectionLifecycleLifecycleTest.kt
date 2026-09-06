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
internal class EditorInputConnectionLifecycleLifecycleTest : EditorInputConnectionTestSupport() {
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
    fun `focused native multiline insertion uses atomic plain text replacement`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello world"), notifyListener = false)
        assertTrue(editText.requestFocus())
        editText.editorId = 1

        var insertedContent: Triple<Int, Int, String>? = null
        editText.onReplaceTextInRustForTesting = { scalarFrom, scalarTo, text ->
            insertedContent = Triple(scalarFrom, scalarTo, text)
        }

        editText.text!!.insert(6, "one\ntwo")

        val (scalarFrom, scalarTo, text) = insertedContent!!
        assertEquals(6, scalarFrom)
        assertEquals(6, scalarTo)
        assertEquals("one\ntwo", text)
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
