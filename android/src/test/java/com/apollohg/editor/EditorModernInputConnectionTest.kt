package com.apollohg.editor

import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorModernInputConnectionTest : EditorInputConnectionTestFixture() {
    @Test
    fun `rejected multiline composition restores authorized display`() {
        val harness = realExternalCompositionHarness(
            "safe",
            """{"initialization":{"type":"localEmpty"},"limits":{"editing":{"maxOperationsPerTransaction":3}}}"""
        )
        try {
            val editor = harness.editText
            val errors = mutableListOf<EditorV2Error>()
            harness.adapter.onAutonomousError = { errors.add(it) }
            val revision = harness.adapter.baseDocumentRevision
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            input.setSelection(0, 4)
            input.setComposingText("x\ny\nz", 1)
            input.commitText("x\ny\nz", 1)
            org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
            assertTrue(errors.any { it.code == "OPERATION_LIMIT_EXCEEDED" })
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
            assertEquals(revision, harness.adapter.baseDocumentRevision)
            assertEquals("safe", editor.text.toString())
            assertEquals(-1, BaseInputConnection.getComposingSpanStart(editor.editableText))
        } finally { harness.adapter.destroy() }
    }

    @Test
    @Config(sdk = [24])
    fun `minimum API preserves real core composition and extracted text`() {
        val harness = realExternalCompositionHarness("a😀b")
        try {
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            input.setSelection(1, 3)
            input.setComposingText("に", 1)
            assertEquals("<p>a😀b</p>", harness.adapter.documentHtml())
            input.commitText("日本", 1)
            assertEquals("<p>a日本b</p>", harness.adapter.documentHtml())
            assertEquals("a日本b", requireNotNull(input.getExtractedText(ExtractedTextRequest(), 0)).text.toString())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `modern commit remains Rust authorized`() {
        val harness = realExternalCompositionHarness("a😀b")
        try {
            harness.editText.setSelection(1, 3)
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            assertTrue(input.commitText("文", 1, null))
            assertEquals("<p>a文b</p>", harness.adapter.documentHtml())
            assertEquals("a文b", harness.editText.text.toString())
            assertEquals(2, harness.editText.selectionEnd)
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `modern composition preserves the original replacement until commit`() {
        val harness = realExternalCompositionHarness("before")
        try {
            harness.editText.setSelection(1, 4)
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            assertTrue(input.setComposingText("に", 1, null))
            assertEquals("<p>before</p>", harness.adapter.documentHtml())
            assertTrue(input.setComposingText("日本", 1, null))
            assertTrue(input.commitText("日本", 1, null))
            assertEquals("<p>b日本re</p>", harness.adapter.documentHtml())
            assertEquals("b日本re", harness.editText.text.toString())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `explicit modern replacement uses its range rather than selection`() {
        val harness = structuredDeleteHarness("<p>a😀b</p><p>tail</p>")
        try {
            harness.editText.setSelection(0)
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            assertTrue(input.replaceText(1, 6, "文\n字", 1, null))
            assertEquals(harness.editText.imeTraceSnapshotForTesting().joinToString("\n"), "<p>a文</p><p>字ail</p>", harness.adapter.documentHtml())
            assertEquals("a文\n字ail", harness.editText.text.toString())
            assertEquals(4, harness.editText.selectionEnd)
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `multiline replacement cursor uses normalized CRLF length`() {
        for (cursor in listOf(1, 2)) {
            val harness = realExternalCompositionHarness("abcdtail")
            try {
                val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
                input.replaceText(1, 3, "x\r\ny", cursor, null)
                assertEquals("<p>ax</p><p>ydtail</p>", harness.adapter.documentHtml())
                assertEquals(3 + cursor, harness.editText.selectionEnd)
            } finally { harness.adapter.destroy() }
        }
    }

    @Test
    fun `explicit replacement retains transient composing text outside its range`() {
        val harness = realExternalCompositionHarness("abc tail")
        try {
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            input.setSelection(0, 3)
            input.setComposingText("日本", 1)
            assertTrue(input.replaceText(3, 7, "end", 1, null))
            assertEquals("<p>日本 end</p>", harness.adapter.documentHtml())
            assertEquals("日本 end", harness.editText.text.toString())
            assertEquals(-1, BaseInputConnection.getComposingSpanStart(harness.editText.editableText))
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `generation retirement blocks context actions before base replacement`() {
        val harness = realExternalCompositionHarness("safe")
        try {
            val editor = harness.editText
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            val clipboard = editor.context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
            clipboard.setPrimaryClip(android.content.ClipData.newPlainText("", "bad"))
            editor.invalidateInputConnectionsForEditor()
            input.performContextMenuAction(android.R.id.paste)
            input.performEditorAction(EditorInfo.IME_ACTION_UNSPECIFIED)
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
            assertEquals("safe", editor.text.toString())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `modern mutation entrypoints reject retired sessions`() {
        val harness = realExternalCompositionHarness("safe")
        try {
            val old = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            harness.editText.onCreateInputConnection(EditorInfo())
            old.replaceText(0, 4, "bad", 1, null)
            old.commitText("bad", 1, null)
            old.setComposingText("bad", 1, null)
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
            assertEquals("safe", harness.editText.text.toString())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `closed connection cannot authorize later mutations`() {
        val harness = realExternalCompositionHarness("safe")
        try {
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            input.closeConnection()
            input.commitText("bad", 1)
            input.replaceText(0, 4, "bad", 1, null)
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
            assertEquals("safe", harness.editText.text.toString())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `snapshot maps selection and composition through generated list placeholders`() {
        val harness = structuredDeleteHarness("<ul><li><p></p></li><li><p>Alpha Beta</p></li></ul>")
        try {
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            val mapper = requireNotNull(harness.editText.imeTextCoordinateMapperForEditor())
            val wordStart = mapper.visibleText.toString().indexOf("Alpha")
            input.setComposingRegion(wordStart + 1, wordStart + 4, null)
            input.setSelection(wordStart + 2, wordStart + 2)
            assertTrue(harness.editText.text.toString().contains(LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER))
            val snapshot = requireNotNull(input.takeSnapshot())
            assertEquals(mapper.visibleText.toString(), snapshot.surroundingText.text.toString())
            assertEquals(wordStart + 2, snapshot.selectionStart)
            assertEquals(wordStart + 2, snapshot.selectionEnd)
            assertEquals(wordStart + 1, snapshot.compositionStart)
            assertEquals(wordStart + 4, snapshot.compositionEnd)
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `extracted and surrounding text use visible IME coordinates`() {
        val harness = structuredDeleteHarness("<ul><li><p></p></li><li><p>Alpha Beta</p></li></ul>")
        try {
            val input = requireNotNull(harness.editText.onCreateInputConnection(EditorInfo()))
            val mapper = requireNotNull(harness.editText.imeTextCoordinateMapperForEditor())
            assertTrue(harness.editText.text.toString().contains(LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER))
            input.setSelection(1, 4)
            val extracted = requireNotNull(input.getExtractedText(ExtractedTextRequest(), 0))
            assertEquals(mapper.visibleText.toString(), extracted.text.toString())
            assertEquals(1, extracted.selectionStart)
            assertEquals(4, extracted.selectionEnd)
            val surrounding = requireNotNull(input.getSurroundingText(100, 100, 0))
            assertEquals(mapper.visibleText.toString(), surrounding.text.toString())
            assertEquals(1, surrounding.selectionStart)
            assertEquals(4, surrounding.selectionEnd)
        } finally { harness.adapter.destroy() }
    }
}
