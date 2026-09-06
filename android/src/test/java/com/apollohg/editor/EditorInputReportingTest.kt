package com.apollohg.editor

import android.content.Context
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.view.View
import android.view.inputmethod.CursorAnchorInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.shadow.api.Shadow
import org.robolectric.shadows.ShadowInputMethodManager

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], shadows = [EditorInputReportingTest.RecordingInputMethodManager::class])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
internal class EditorInputReportingTest : EditorInputConnectionTestFixture() {
    @Implements(InputMethodManager::class)
    class RecordingInputMethodManager : ShadowInputMethodManager() {
        val extracted = mutableListOf<Pair<Int, ExtractedText>>()
        val selections = mutableListOf<List<Int>>()
        @Implementation
        fun updateExtractedText(view: View, token: Int, text: ExtractedText) { extracted += token to text }
        @Implementation
        fun updateSelection(view: View, start: Int, end: Int, composingStart: Int, composingEnd: Int) {
            selections += listOf(start, end, composingStart, composingEnd)
        }
    }

    @Test
    fun `monitored extracted text and selection follow visible composition and retire together`() {
        val harness = structuredDeleteHarness("<ul><li><p></p></li><li><p>Alpha</p></li></ul>")
        try {
            val editor = harness.editText
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            val manager = editor.context.getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
            val recorder = Shadow.extract<RecordingInputMethodManager>(manager)
            input.getExtractedText(ExtractedTextRequest().apply { token = 71 }, InputConnection.GET_EXTRACTED_TEXT_MONITOR)
            val visible = requireNotNull(editor.imeTextCoordinateMapperForEditor()).visibleText.toString()
            val start = visible.indexOf("Alpha")
            input.setSelection(start, start + 5)
            input.setComposingText("日本", 1)
            assertTrue(recorder.extracted.isNotEmpty())
            val report = recorder.extracted.last()
            assertEquals(71, report.first)
            assertEquals(visible.replace("Alpha", "日本"), report.second.text.toString())
            assertEquals(listOf(start + 2, start + 2, start, start + 2), recorder.selections.last())
            input.finishComposingText()
            val next = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            recorder.extracted.clear()
            next.commitText("!", 1)
            assertTrue(recorder.extracted.isEmpty())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `cursor report follows owned line bounds and physical padding after scrolling`() {
        val editor = EditorEditText(RuntimeEnvironment.getApplication())
        val text = SpannableStringBuilder("first\nsecond")
        text.setSpan(EditorBlockBoxSpan(EditorBoxStyle(padding = EditorEdges(top = 23f, bottom = 41f, left = 19f)), EditorEdges(), 0), 0, 5, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        editor.setText(text)
        editor.setPadding(11, 13, 17, 7)
        editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(400, View.MeasureSpec.EXACTLY))
        editor.layout(0, 0, 320, 400)
        editor.setSelection(2)
        editor.scrollTo(3, 31)
        val layout = editor.layout as EditorDocumentLayout
        val report = editor.buildSurfaceCursorAnchorInfo()
        assertEquals(2, report.selectionEnd)
        assertTrue(report.insertionMarkerFlags and CursorAnchorInfo.FLAG_HAS_VISIBLE_REGION != 0)
        assertEquals(11f - 3f + layout.getPrimaryHorizontal(2), report.insertionMarkerHorizontal, 0.01f)
        assertEquals(13f - 31f + 23f, report.insertionMarkerTop, 0.01f)
        assertEquals(13f - 31f + layout.textLineBottom(0), report.insertionMarkerBottom, 0.01f)
        assertEquals(13f - 31f + layout.getLineBaseline(0), report.insertionMarkerBaseline, 0.01f)
        assertTrue(report.insertionMarkerBottom < 13f - 31f + layout.getLineTop(1))
    }
}
