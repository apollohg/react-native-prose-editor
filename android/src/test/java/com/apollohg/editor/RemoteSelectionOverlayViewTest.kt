package com.apollohg.editor

import android.graphics.Color
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.view.View
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [24, 34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class RemoteSelectionOverlayViewTest {
    @Test
    fun `remote caret excludes the following block gap`() {
        val text = SpannableStringBuilder("one\ntwo")
        text.setSpan(EditorBlockBoxSpan(EditorBoxStyle(padding = EditorEdges(bottom = 40f)), EditorEdges(), 0), 0, 3, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val (editor, overlay) = fixture(text)
        val layout = editor.editorEditText.layout
        assertTrue(layout.editorTextLineTop(1) - layout.editorTextLineBottom(0) >= 40)

        overlay.setRemoteSelections(listOf(RemoteSelectionDecoration("1", 0, 0, Color.RED, null, true)))
        val caret = overlay.debugSnapshotsForTesting().single().caretRect!!
        assertEquals(layout.editorTextLineBottom(0) - layout.editorTextLineTop(0), caret.height().toInt())
    }

    @Test
    @Config(sdk = [34])
    fun `remote caret within a padded line uses its shaped horizontal`() {
        val text = SpannableStringBuilder("one\ntwo")
        text.setSpan(EditorBlockBoxSpan(EditorBoxStyle(padding = EditorEdges(left = 23f, bottom = 40f)), EditorEdges(), 0), 0, 3, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        val (editor, overlay) = fixture(text)
        val input = editor.editorEditText
        val layout = input.layout
        overlay.setRemoteSelections(listOf(RemoteSelectionDecoration("1", 1, 1, Color.RED, null, true)))
        val caret = overlay.debugSnapshotsForTesting().single().caretRect!!
        val baseX = editor.editorViewport.left + editor.editorScrollView.left + input.left + input.compoundPaddingLeft
        assertTrue(layout.getPrimaryHorizontal(1) > 23f)
        assertEquals(baseX + layout.getPrimaryHorizontal(1), caret.left, 0.01f)
        assertEquals(layout.editorTextLineBottom(0) - layout.editorTextLineTop(0), caret.height().toInt())
    }

    @Test
    fun `remote caret at document end uses the trailing empty paragraph`() {
        val text = SpannableStringBuilder("one\n")
        text.setSpan(EditorBlockBoxSpan(EditorBoxStyle(padding = EditorEdges(top = 17f, left = 29f)), EditorEdges(), 0), text.length, text.length, Spanned.SPAN_INCLUSIVE_INCLUSIVE)
        val (editor, overlay) = fixture(text)
        val input = editor.editorEditText
        val layout = input.layout
        val line = layout.getLineForOffset(text.length)
        assertEquals(1, line)

        overlay.setRemoteSelections(listOf(RemoteSelectionDecoration("1", text.length, text.length, Color.RED, null, true)))
        val caret = overlay.debugSnapshotsForTesting().single().caretRect!!
        val baseY = editor.editorViewport.top + editor.editorScrollView.top + input.top + input.compoundPaddingTop - editor.editorScrollView.scrollY
        assertEquals(baseY + layout.editorTextLineTop(line), caret.top.toInt())
        assertEquals(baseY + layout.editorTextLineBottom(line), caret.bottom.toInt())
    }

    private fun fixture(text: CharSequence): Pair<RichTextEditorView, RemoteSelectionOverlayView> {
        val context = RuntimeEnvironment.getApplication()
        val editor = RichTextEditorView(context)
        editor.editorEditText.setText(text)
        editor.measure(View.MeasureSpec.makeMeasureSpec(400, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.EXACTLY))
        editor.layout(0, 0, 400, 300)
        val overlay = RemoteSelectionOverlayView(context).apply {
            editorIdOverrideForTesting = 1L
            docToScalarResolver = { _, position -> position }
            bind(editor)
            layout(0, 0, 400, 300)
        }
        return editor to overlay
    }
}
