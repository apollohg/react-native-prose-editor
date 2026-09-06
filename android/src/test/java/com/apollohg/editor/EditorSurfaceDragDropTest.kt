package com.apollohg.editor

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.view.DragEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.util.ReflectionHelpers

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
internal class EditorSurfaceDragDropTest : EditorInputConnectionTestFixture() {
    @Test
    fun `external text drops at geometry caret through core without changing clipboard`() {
        val harness = realExternalCompositionHarness("a😀b")
        try {
            val editor = harness.editText
            measure(editor)
            editor.setSelection(0)
            val clipboard = editor.context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("clipboard", "keep"))
            val clip = ClipData.newPlainText("external", "X\nY")
            assertTrue(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            assertTrue(send(editor, DragEvent.ACTION_DRAG_ENTERED, clip))
            assertTrue(send(editor, DragEvent.ACTION_DROP, clip, 3))
            assertEquals("<p>a😀X</p><p>Yb</p>", harness.adapter.documentHtml())
            assertEquals("a😀X\nYb", editor.text.toString())
            assertEquals("keep", clipboard.primaryClip!!.getItemAt(0).text.toString())
            assertTrue(send(editor, DragEvent.ACTION_DRAG_ENDED, clip))
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `readonly editor rejects drag start and a previously accepted drop`() {
        val harness = realExternalCompositionHarness("safe")
        try {
            val editor = harness.editText
            measure(editor)
            val clip = ClipData.newPlainText("external", "bad")
            editor.isEditable = false
            assertFalse(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            editor.isEditable = true
            assertTrue(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            editor.isEditable = false
            assertFalse(send(editor, DragEvent.ACTION_DROP, clip, 2))
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `external drop commits transient composition before resolving its insertion caret`() {
        val harness = realExternalCompositionHarness("abc tail")
        try {
            val editor = harness.editText
            measure(editor)
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            input.setSelection(0, 3)
            input.setComposingText("日本", 1)
            assertEquals("<p>abc tail</p>", harness.adapter.documentHtml())
            val clip = ClipData.newPlainText("external", "!")
            assertTrue(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            assertTrue(send(editor, DragEvent.ACTION_DROP, clip, editor.text.length))
            assertEquals("<p>日本 tail!</p>", harness.adapter.documentHtml())
            assertEquals(-1, BaseInputConnection.getComposingSpanStart(editor.editableText))
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `drop cannot cross an editor owner change`() {
        val first = realExternalCompositionHarness("first")
        val second = realExternalCompositionHarness("second")
        try {
            val editor = first.editText
            measure(editor)
            val clip = ClipData.newPlainText("external", "bad")
            assertTrue(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            editor.editorId = 2L
            editor.v2Driver = second.adapter
            assertFalse(send(editor, DragEvent.ACTION_DROP, clip, 1))
            assertEquals("<p>first</p>", first.adapter.documentHtml())
            assertEquals("<p>second</p>", second.adapter.documentHtml())
        } finally {
            first.adapter.destroy()
            second.adapter.destroy()
        }
    }

    @Test
    fun `blocked composition preflight cannot insert dropped text`() {
        val harness = realExternalCompositionHarness("safe")
        try {
            val editor = harness.editText
            measure(editor)
            val clip = ClipData.newPlainText("external", "bad")
            assertTrue(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            editor.blockExternalEditorUpdatePreparationForTesting = true
            assertFalse(send(editor, DragEvent.ACTION_DROP, clip, 1))
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
        } finally { harness.adapter.destroy() }
    }

    @Test
    fun `detaching the surface retires its accepted drag`() {
        val harness = realExternalCompositionHarness("safe")
        try {
            val activity = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
            val editor = harness.editText
            activity.setContentView(editor)
            measure(editor)
            val clip = ClipData.newPlainText("external", "bad")
            assertTrue(send(editor, DragEvent.ACTION_DRAG_STARTED, clip))
            activity.setContentView(View(activity))
            assertFalse(send(editor, DragEvent.ACTION_DROP, clip, 1))
            assertEquals("<p>safe</p>", harness.adapter.documentHtml())
        } finally { harness.adapter.destroy() }
    }

    private fun measure(editor: EditorEditText) {
        editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.AT_MOST))
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
    }

    private fun send(editor: EditorEditText, action: Int, clip: ClipData, offset: Int = 0): Boolean {
        val line = editor.layout.getLineForOffset(offset)
        val event = ReflectionHelpers.callStaticMethod<DragEvent>(DragEvent::class.java, "obtain")
        ReflectionHelpers.setField(event, "mAction", action)
        ReflectionHelpers.setField(event, "mX", editor.layout.getPrimaryHorizontal(offset) + editor.totalPaddingLeft)
        ReflectionHelpers.setField(event, "mY", editor.layout.editorTextLineTop(line).toFloat() + editor.totalPaddingTop + 1f)
        ReflectionHelpers.setField(event, "mClipData", clip)
        ReflectionHelpers.setField(event, "mClipDescription", clip.description)
        ReflectionHelpers.setField(event, "mLocalState", Any())
        if (action == DragEvent.ACTION_DROP) {
            assertEquals("drop coordinate must target requested UTF16 boundary", offset, editor.getOffsetForPosition(event.x, event.y))
        }
        return try { editor.onDragEvent(event) } finally { ReflectionHelpers.callInstanceMethod<Unit>(event, "recycle") }
    }
}
