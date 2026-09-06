package com.apollohg.editor.prototype

import android.text.Selection
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CorrectionInfo
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PrototypeInputConnectionTest {
    private fun connection(session: PrototypeDocumentSession) = PrototypeInputConnection(View(RuntimeEnvironment.getApplication()), session)

    @Test
    fun `modern replacement reconciles explicit range and cursor to core`() {
        PrototypeDocumentSession(listOf("a😀b", "tail")).use { session ->
            val input = connection(session)
            input.setSelection(0, 0)
            assertTrue(input.replaceText(1, 6, "文\n字", 1, null))
            assertEquals("a文\n字ail", session.editable.toString())
            assertEquals("a文\n字ail", session.committedText)
            assertEquals(4, session.selectionEnd)
            assertEquals(2, JSONObject(session.committedDocumentJson()).getJSONArray("content").length())
            assertFalse(input.replaceText(0, 1, "\uD800", 1, null))
            input.closeConnection()
            assertFalse(input.replaceText(0, 1, "stale", 1, null))
        }
    }

    @Test
    @Config(sdk = [24])
    fun `minimum SDK composition commits and joins paragraphs`() {
        PrototypeDocumentSession(listOf("a", "b")).use { session ->
            val input = connection(session)
            input.setSelection(1, 1)
            assertTrue(input.setComposingText("文", 1))
            assertEquals("a\nb", session.committedText)
            assertTrue(input.commitText("文字", 1))
            assertEquals("a文字\nb", session.committedText)
            input.setSelection(4, 4)
            assertTrue(input.deleteSurroundingText(1, 0))
            assertEquals("a文字b", session.committedText)
        }
    }

    @Test
    fun `cursor bias and malformed UTF16 never split a scalar`() {
        PrototypeDocumentSession(listOf("ab")).use { session ->
            val input = connection(session)
            input.setSelection(1, 1)
            assertTrue(input.commitText("😀", 0))
            assertEquals(1, session.selectionEnd)
            assertEquals("a😀b", session.committedText)
            assertFalse(input.commitText("\uD800", 1))
            assertEquals("a😀b", session.committedText)
            session.setSelection(2, 2)
            assertEquals(1, session.selectionEnd)
        }
    }

    @Test
    fun `replacement connection retires an unclosed composing connection`() {
        PrototypeDocumentSession(listOf("a")).use { session ->
            val first = connection(session)
            first.setSelection(1, 1)
            first.setComposingText("draft", 1)
            val second = connection(session)
            assertEquals("a", session.editable.toString())
            assertFalse(first.finishComposingText())
            first.closeConnection()
            assertTrue(second.commitText("b", 1))
            assertEquals("ab", session.committedText)
        }
    }

    @Test
    fun `commits through Rust and keeps UTF16 selection`() {
        PrototypeDocumentSession(listOf("ab", "cd")).use { session ->
            val input = connection(session)
            session.setSelection(1, 1)
            assertTrue(input.commitText("😀", 1))
            assertEquals("a😀b\ncd", session.committedText)
            assertEquals(session.committedText, session.editable.toString())
            assertEquals(3, session.selectionStart)
            assertEquals(2, JSONObject(session.committedDocumentJson()).getJSONArray("content").length())
            assertTrue(input.deleteSurroundingTextInCodePoints(1, 0))
            assertEquals("ab\ncd", session.committedText)
            assertEquals(1, session.selectionStart)
        }
    }

    @Test
    fun `two stage CJK composition stays transient until finish`() {
        PrototypeDocumentSession(listOf("A", "B")).use { session ->
            val input = connection(session)
            session.setSelection(1, 1)
            assertTrue(input.setComposingText("に", 1))
            assertEquals("Aに\nB", session.editable.toString())
            assertEquals("A\nB", session.committedText)
            assertEquals(1, BaseInputConnection.getComposingSpanStart(session.editable))
            assertTrue(input.setComposingText("日本", 1))
            assertEquals("A日本\nB", session.editable.toString())
            assertEquals("A\nB", session.committedText)
            assertTrue(input.finishComposingText())
            assertEquals("A日本\nB", session.committedText)
            assertEquals(-1, BaseInputConnection.getComposingSpanStart(session.editable))
            assertEquals(3, session.selectionStart)
        }
    }

    @Test
    fun `composition commit replaces marked range after selection moves`() {
        PrototypeDocumentSession(listOf("word tail")).use { session ->
            val input = connection(session)
            assertTrue(input.setComposingRegion(0, 4))
            assertTrue(input.setSelection(2, 2))
            assertTrue(input.commitText("文", 1))
            assertEquals("文 tail", session.committedText)
            assertEquals(1, session.selectionEnd)
        }
    }

    @Test
    fun `cross block selected replacement merges paragraphs`() {
        PrototypeDocumentSession(listOf("first", "second")).use { session ->
            val input = connection(session)
            session.setSelection(3, 9)
            assertEquals("st\nsec", input.getSelectedText(0).toString())
            assertTrue(input.commitText("X", 1))
            assertEquals("firXond", session.committedText)
            assertEquals(1, JSONObject(session.committedDocumentJson()).getJSONArray("content").length())
            assertEquals(4, session.selectionStart)
        }
    }

    @Test
    fun `enter creates paragraphs and backspace joins them`() {
        PrototypeDocumentSession(listOf("abcd")).use { session ->
            val input = connection(session)
            session.setSelection(2, 2)
            assertTrue(input.commitText("\n", 1))
            assertEquals("ab\ncd", session.committedText)
            assertEquals(2, JSONObject(session.committedDocumentJson()).getJSONArray("content").length())
            assertTrue(input.deleteSurroundingText(1, 0))
            assertEquals("abcd", session.committedText)
            assertEquals(1, JSONObject(session.committedDocumentJson()).getJSONArray("content").length())
            assertTrue(input.commitText("\n\n", 1))
            assertEquals("ab\n\ncd", session.committedText)
            assertEquals(3, JSONObject(session.committedDocumentJson()).getJSONArray("content").length())
        }
    }

    @Test
    fun `batch edit publishes and commits one final buffer`() {
        PrototypeDocumentSession(listOf("a")).use { session ->
            val input = connection(session)
            var changes = 0
            session.onChange = { changes++ }
            assertTrue(input.beginBatchEdit())
            input.setSelection(1, 1)
            input.commitText("b", 1)
            input.commitText("c", 1)
            assertEquals("a", session.committedText)
            assertEquals(0, changes)
            assertTrue(input.endBatchEdit())
            assertEquals("abc", session.committedText)
            assertEquals(1, changes)
            assertFalse(input.endBatchEdit())
        }
    }

    @Test
    fun `correction and reverse selections retain scalar boundaries`() {
        PrototypeDocumentSession(listOf("😀 teh")).use { session ->
            val input = connection(session)
            input.setSelection(3, 6)
            assertTrue(input.commitText("the", 1))
            assertTrue(input.commitCorrection(CorrectionInfo(3, "teh", "the")))
            assertEquals("😀 the", session.committedText)
            session.setSelection(6, 3)
            assertEquals(6, Selection.getSelectionStart(session.editable))
            assertEquals(3, Selection.getSelectionEnd(session.editable))
            assertTrue(input.commitText("ok", 1))
            assertEquals("😀 ok", session.committedText)
            session.setSelection(2, 2)
            assertTrue(input.deleteSurroundingText(1, 0))
            assertEquals(" ok", session.committedText)
        }
    }

    @Test
    fun `correction notification preserves document selection and active composition`() {
        PrototypeDocumentSession(listOf("teh tail")).use { session ->
            val input = connection(session)
            input.setSelection(3, 3)
            input.setComposingText("中", 1)
            assertTrue(input.commitCorrection(CorrectionInfo(0, "teh", "the")))
            assertEquals("teh tail", session.committedText)
            assertEquals("teh中 tail", session.editable.toString())
            assertEquals(4, session.selectionEnd)
            assertEquals(3, BaseInputConnection.getComposingSpanStart(session.editable))
            assertEquals(4, BaseInputConnection.getComposingSpanEnd(session.editable))
            input.closeConnection()
            assertFalse(input.commitCorrection(CorrectionInfo(0, "teh", "the")))
        }
    }

    @Test
    fun `retired connections cannot change live or closed sessions`() {
        val session = PrototypeDocumentSession(listOf("base"))
        val first = connection(session)
        first.setSelection(4, 4)
        first.setComposingText("draft", 1)
        first.closeConnection()
        assertEquals("base", session.editable.toString())
        assertFalse(first.commitText("stale", 1))
        val second = connection(session)
        assertFalse(first.setSelection(0, 0))
        assertTrue(second.commitText("!", 1))
        val committed = session.committedText
        session.close()
        assertFalse(second.commitText("closed", 1))
        assertEquals(committed, session.committedText)
        session.close()
    }
}
