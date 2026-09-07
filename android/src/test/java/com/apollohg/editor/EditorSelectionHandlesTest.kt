package com.apollohg.editor

import android.app.Activity
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Typeface
import android.view.ContextThemeWrapper
import android.view.MotionEvent
import android.view.View
import android.widget.EditText
import org.junit.Assert.assertTrue
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Robolectric
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EditorSelectionHandlesTest {
    @Test
    fun `selection highlight uses the native theme color`() {
        val editor = editor("hello world")
        val nativeColor = EditText(editor.context).highlightColor
        assertTrue(Color.alpha(nativeColor) in 1..254)
        assertEquals(nativeColor, editor.highlightColor)
    }

    @Test
    @Config(sdk = [28, 34])
    fun `selected text paints a translucent highlight behind each paragraph`() {
        val editor = editor("hello world\nsecond paragraph")
        editor.background = null
        editor.setTextColor(Color.TRANSPARENT)
        editor.setSelection(4, 18)
        val actual = bitmap(editor)
        editor.draw(Canvas(actual))
        for (offset in listOf(6, 16)) {
            val layout = editor.layout
            val line = layout.getLineForOffset(offset)
            val x = ((layout.getPrimaryHorizontal(offset) + layout.getPrimaryHorizontal(offset + 1)) / 2).toInt()
            val y = (layout.editorTextLineTop(line) + layout.editorTextLineBottom(line)) / 2
            val pixel = actual.getPixel(x, y)
            assertEquals("Highlight opacity at offset $offset", Color.alpha(editor.highlightColor), Color.alpha(pixel))
            for (channel in listOf(Color::red, Color::green, Color::blue)) {
                assertEquals(channel(editor.highlightColor).toFloat(), channel(pixel).toFloat(), 2f)
            }
        }
    }

    @Test
    fun `text selection hides the insertion bar`() {
        val controller = Robolectric.buildActivity(Activity::class.java).setup().visible()
        try {
            val editor = editor("hello world")
            controller.get().setContentView(editor)
            controller.windowFocusChanged(true)
            editor.layout(0, 0, 600, 240)
            editor.requestFocus()
            assertTrue(editor.hasWindowFocus())
            editor.setTextColor(Color.TRANSPARENT)
            editor.highlightColor = Color.TRANSPARENT
            editor.setSelection(4)
            val caret = bitmap(editor)
            editor.draw(Canvas(caret))
            assertTrue("Collapsed selection must still draw the insertion bar", !caret.sameAs(bitmap(editor)))

            editor.setSelection(4, 10)
            val selected = bitmap(editor)
            editor.draw(Canvas(selected))
            editor.isCursorVisible = false
            val withoutCursor = bitmap(editor)
            editor.draw(Canvas(withoutCursor))
            assertTrue("A text selection must draw identically with the insertion bar disabled", selected.sameAs(withoutCursor))
        } finally {
            controller.pause().stop().destroy()
        }
    }

    @Test
    fun `tapping to place the caret does not leave a selection bubble`() {
        val editor = editor("hello world")
        val x = editor.layout.getPrimaryHorizontal(3)
        val y = editor.layout.getLineBaseline(0).toFloat()
        for ((time, action) in listOf(0L to MotionEvent.ACTION_DOWN, 50L to MotionEvent.ACTION_UP)) {
            val event = MotionEvent.obtain(0L, time, action, x, y, 0)
            editor.onTouchEvent(event)
            event.recycle()
        }
        assertTrue(editor.hasFocus())
        assertTrue(editor.selectionStart == editor.selectionEnd)
        val actual = bitmap(editor)
        editor.interaction.drawHandles(Canvas(actual))
        assertTrue("An insertion caret should have no selection handle", actual.sameAs(bitmap(editor)))
    }

    @Test
    fun `selection handles match themed Android drawables in both text directions`() {
        for (text in listOf("hello world", "אבגדה וזחטי")) {
            val editor = editor(text)
            val native = EditText(editor.context)
            for ((start, end) in listOf(4 to 10, 10 to 4)) {
                editor.setSelection(start, end)
                val expected = bitmap(editor)
                val canvas = Canvas(expected)
                for ((offset, isStart) in listOf(4 to true, 10 to false)) {
                    val layout = editor.layout
                    val rtl = layout.isRtlCharAt(if (isStart) offset else offset - 1)
                    val leftHandle = isStart != rtl
                    val drawable = requireNotNull(if (leftHandle) native.textSelectHandleLeft else native.textSelectHandleRight).mutate()
                    drawable.setTint(editor.caretColor)
                    val hotspot = if (leftHandle) drawable.intrinsicWidth * 3 / 4 else drawable.intrinsicWidth / 4
                    val x = (layout.getPrimaryHorizontal(offset) - 0.5f).toInt() - hotspot
                    val y = CaretGeometry.verticalBounds(layout, offset, editor.paint, editor.text).bottom.toInt()
                    drawable.setBounds(x, y, x + drawable.intrinsicWidth, y + drawable.intrinsicHeight)
                    drawable.draw(canvas)
                }
                val actual = bitmap(editor)
                editor.interaction.drawHandles(Canvas(actual))
                assertTrue("Handles must use native shapes for $text ($start..$end)", expected.sameAs(actual))
            }
        }
    }

    @Test
    fun `dragging at a mixed direction boundary does not jump across the RTL run`() {
        val editor = editor("hello אבג world")
        editor.setSelection(6, 9)
        val drawable = requireNotNull(EditText(editor.context).textSelectHandleRight)
        val x = editor.layout.getSecondaryHorizontal(6) + drawable.intrinsicWidth / 4f
        val y = CaretGeometry.verticalBounds(editor.layout, 6, editor.paint, editor.text).bottom + drawable.intrinsicHeight / 2f
        for ((time, action) in listOf(0L to MotionEvent.ACTION_DOWN, 50L to MotionEvent.ACTION_MOVE, 100L to MotionEvent.ACTION_UP)) {
            val event = MotionEvent.obtain(0L, time, action, x, y, 0)
            editor.onTouchEvent(event)
            event.recycle()
        }
        assertEquals(6, editor.selectionStart)
        assertEquals(9, editor.selectionEnd)
    }

    @Test
    fun `offscreen selection endpoints are not pinned to the viewport`() {
        val editor = editor("hello world\n".repeat(40))
        editor.setSelection(350, 400)
        val actual = bitmap(editor)
        editor.interaction.drawHandles(Canvas(actual))
        assertTrue(actual.sameAs(bitmap(editor)))
    }

    @Test
    fun `dragging a teardrop preserves the finger offset from its tip`() {
        val editor = editor("hello world again")
        editor.setSelection(4, 10)
        val drawable = requireNotNull(EditText(editor.context).textSelectHandleRight)
        val x = editor.layout.getPrimaryHorizontal(10) + drawable.intrinsicWidth / 4f
        val bottom = CaretGeometry.verticalBounds(editor.layout, 10, editor.paint, editor.text).bottom
        val y = bottom + drawable.intrinsicHeight / 2f
        val delta = editor.layout.getPrimaryHorizontal(12) - editor.layout.getPrimaryHorizontal(10)
        for ((time, action, touchX) in listOf(
            Triple(0L, MotionEvent.ACTION_DOWN, x),
            Triple(50L, MotionEvent.ACTION_MOVE, x + delta),
            Triple(100L, MotionEvent.ACTION_UP, x + delta),
        )) {
            val event = MotionEvent.obtain(0L, time, action, touchX, y, 0)
            editor.onTouchEvent(event)
            event.recycle()
        }
        assertEquals(4, editor.selectionStart)
        assertEquals(12, editor.selectionEnd)
        editor.setSelection(12)
        val actual = bitmap(editor)
        editor.interaction.drawHandles(Canvas(actual))
        assertTrue(actual.sameAs(bitmap(editor)))
    }

    private fun editor(text: String): EditorEditText {
        val context = ContextThemeWrapper(RuntimeEnvironment.getApplication(), android.R.style.Theme_Material_Light_NoActionBar)
        return EditorEditText(context).apply {
            typeface = Typeface.MONOSPACE
            setPadding(0, 0, 0, 0)
            setText(text)
            measure(View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY))
            layout(0, 0, measuredWidth, measuredHeight)
            requestFocus()
        }
    }

    private fun bitmap(editor: EditorEditText) = Bitmap.createBitmap(editor.width, editor.height, Bitmap.Config.ARGB_8888)
}
