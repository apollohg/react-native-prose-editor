package com.apollohg.editor.prototype

import android.app.Activity
import android.graphics.Rect
import android.view.View
import android.view.KeyEvent
import android.view.inputmethod.EditorInfo
import android.widget.Button
import android.widget.ScrollView
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PrototypeEditorViewTest {
    @Test
    fun `vertical keyboard navigation crosses paragraph padding and native child`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        PrototypeDocumentSession(listOf("First", "Second")).use { session ->
            val view = PrototypeEditorView(activity, session)
            view.mountAtom(Button(activity), 80)
            activity.setContentView(view)
            size(view)
            session.setSelection(2, 2)
            assertTrue(view.onKeyDown(KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_DOWN)))
            assertTrue("Selection must enter second paragraph", session.selectionEnd >= 6)
            assertTrue(view.onKeyDown(KeyEvent.KEYCODE_DPAD_UP, KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_UP)))
            assertTrue("Selection must return to first paragraph", session.selectionEnd <= 5)
        }
    }

    @Test
    fun `mounted view uses current composition layout for caret and commits through Rust`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        PrototypeDocumentSession(listOf("First paragraph", "Second paragraph")).use { session ->
            val view = PrototypeEditorView(activity, session)
            activity.setContentView(view)
            size(view)
            view.requestFocus()
            val connection = requireNotNull(view.onCreateInputConnection(EditorInfo()))
            session.setSelection(3, 20)
            assertTrue(connection.setComposingText("日本語", 1))
            size(view)
            val caret = view.documentLayout.caret(session.selectionEnd)
            assertEquals(session.selectionEnd, view.documentLayout.offsetAt(caret.left, caret.centerY()))
            assertEquals("First paragraph\nSecond paragraph", session.committedText)
            assertTrue(connection.finishComposingText())
            assertEquals(session.editable.toString(), session.committedText)
            assertFalse(view.documentLayout.selection(0, session.editable.length).isEmpty)
        }
    }

    @Test
    fun `native atom is a child in content coordinates and resizes the following paragraph`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        PrototypeDocumentSession(listOf("First paragraph", "Second paragraph")).use { session ->
            val editor = PrototypeEditorView(activity, session)
            val atom = Button(activity).apply { text = "Native atom"; minimumHeight = 0 }
            editor.mountAtom(atom, 80)
            val scroll = ScrollView(activity).apply { addView(editor) }
            activity.setContentView(scroll)
            size(editor)
            val firstBounds = Rect(atom.left, atom.top, atom.right, atom.bottom)
            val secondStart = session.editable.indexOf('\n') + 1
            val before = editor.documentLayout.caret(secondStart)
            assertSame(editor, atom.parent)
            assertTrue(before.top >= atom.bottom)
            editor.mountAtom(atom, 160)
            size(editor)
            assertEquals(80f, editor.documentLayout.caret(secondStart).top - before.top, 0.01f)
            assertEquals(firstBounds.top, atom.top)
            scroll.scrollTo(0, 40)
            assertEquals(firstBounds.top, atom.top)
        }
    }

    private fun size(view: View) {
        view.measure(View.MeasureSpec.makeMeasureSpec(380, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(1000, View.MeasureSpec.AT_MOST))
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
    }
}
