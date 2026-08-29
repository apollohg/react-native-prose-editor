package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorEditTextHardwareKeyTest {

    @Test
    @Config(sdk = [29])
    fun `custom cursor drawing is safe before API 34`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.layoutParams = ViewGroup.LayoutParams(320, 80)
        editText.setText("line")
        editText.setSelection(editText.length())
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(80, View.MeasureSpec.EXACTLY),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val textLayout = requireNotNull(editText.layout)
        val line = textLayout.getLineForOffset(editText.selectionEnd)
        val drawable = requireNotNull(editText.textCursorDrawable)
        drawable.setBounds(24, textLayout.getLineTop(line), 26, textLayout.getLineBottom(line))

        drawable.draw(Canvas(Bitmap.createBitmap(320, 80, Bitmap.Config.ARGB_8888)))
    }

    @Test
    fun `hardware backspace deletes on first key press in dev mode`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)

        val downEvent = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL)
        val upEvent = KeyEvent(downEvent.downTime, downEvent.eventTime, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL, 0)

        val handledDown = editText.dispatchKeyEvent(downEvent)
        val handledUp = editText.dispatchKeyEvent(upEvent)

        assertTrue(handledDown)
        assertTrue(handledUp)
        assertEquals("ab", editText.text?.toString())
        assertEquals(2, editText.selectionStart)
        assertEquals(2, editText.selectionEnd)
    }

    @Test
    fun `duplicate hardware backspace from dispatch and input connection deletes once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val downEvent = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(inputConnection!!.sendKeyEvent(downEvent))

        assertEquals("ab", editText.text?.toString())
        assertEquals(2, editText.selectionStart)
        assertEquals(2, editText.selectionEnd)
    }

    @Test
    fun `duplicate hardware backspace with rewrapped event time deletes once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val dispatchedEvent = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        val rewrappedEvent = KeyEvent(100L, 101L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)

        assertTrue(editText.dispatchKeyEvent(dispatchedEvent))
        assertTrue(inputConnection!!.sendKeyEvent(rewrappedEvent))

        assertEquals("ab", editText.text?.toString())
        assertEquals(2, editText.selectionStart)
        assertEquals(2, editText.selectionEnd)
    }

    @Test
    fun `late duplicate hardware backspace after key up deletes once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val downEvent = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        val upEvent = KeyEvent(100L, 102L, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL, 0)
        val lateDuplicateDown = KeyEvent(100L, 103L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(editText.dispatchKeyEvent(upEvent))
        assertTrue(inputConnection!!.sendKeyEvent(lateDuplicateDown))

        assertEquals("ab", editText.text?.toString())
        assertEquals(2, editText.selectionStart)
        assertEquals(2, editText.selectionEnd)
    }

    @Test
    fun `duplicate printable hardware key from dispatch and input connection inserts once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("")
        editText.setSelection(0)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val downEvent = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A, 0)

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(inputConnection!!.sendKeyEvent(downEvent))

        assertEquals("a", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `duplicate hardware forward delete from dispatch and input connection deletes once`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(1)
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val downEvent = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL, 0)

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(inputConnection!!.sendKeyEvent(downEvent))

        assertEquals("ac", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `bound duplicate hardware forward delete with rewrapped event time deletes once in rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.isApplyingRustState = true
        editText.setText("abc")
        editText.isApplyingRustState = false
        editText.setSelection(1)
        editText.editorId = 1
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        val dispatchedEvent = KeyEvent(
            100L,
            100L,
            KeyEvent.ACTION_DOWN,
            KeyEvent.KEYCODE_FORWARD_DEL,
            0
        )
        val rewrappedEvent = KeyEvent(
            100L,
            101L,
            KeyEvent.ACTION_DOWN,
            KeyEvent.KEYCODE_FORWARD_DEL,
            0
        )
        var deleteCount = 0
        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deleteCount += 1
            deletedRange = scalarFrom to scalarTo
        }

        assertTrue(editText.dispatchKeyEvent(dispatchedEvent))
        assertTrue(inputConnection!!.sendKeyEvent(rewrappedEvent))

        assertEquals(1, deleteCount)
        assertEquals(1 to 2, deletedRange)
    }

    @Test
    fun `hardware forward delete routes scalar range through rust`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.isApplyingRustState = true
        editText.setText("A😀B")
        editText.isApplyingRustState = false
        editText.setSelection(1)
        editText.editorId = 1

        var deletedRange: Pair<Int, Int>? = null
        editText.onDeleteRangeInRustForTesting = { scalarFrom, scalarTo ->
            deletedRange = scalarFrom to scalarTo
        }

        val downEvent = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL)

        assertTrue(editText.dispatchKeyEvent(downEvent))

        assertEquals(1 to 2, deletedRange)
    }

    @Test
    fun `hardware backspace repeat events continue deleting`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)

        val firstDown = KeyEvent(100L, 100L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 0)
        val repeatDown = KeyEvent(100L, 120L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL, 1)

        assertTrue(editText.dispatchKeyEvent(firstDown))
        assertTrue(editText.dispatchKeyEvent(repeatDown))

        assertEquals("a", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `read only hardware backspace is consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)
        editText.isEditable = false

        val downEvent = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL)
        val upEvent = KeyEvent(downEvent.downTime, downEvent.eventTime, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL, 0)

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(editText.dispatchKeyEvent(upEvent))
        assertEquals("abc", editText.text?.toString())
        assertEquals(3, editText.selectionStart)
        assertEquals(3, editText.selectionEnd)
    }

    @Test
    fun `read only printable hardware key is consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(3)
        editText.isEditable = false

        val downEvent = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_A)
        val upEvent = KeyEvent(downEvent.downTime, downEvent.eventTime, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_A, 0)

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(editText.dispatchKeyEvent(upEvent))
        assertEquals("abc", editText.text?.toString())
        assertEquals(3, editText.selectionStart)
        assertEquals(3, editText.selectionEnd)
    }

    @Test
    fun `read only forward delete hardware key is consumed without mutating text`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("abc")
        editText.setSelection(1)
        editText.isEditable = false

        val downEvent = KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_FORWARD_DEL)
        val upEvent = KeyEvent(
            downEvent.downTime,
            downEvent.eventTime,
            KeyEvent.ACTION_UP,
            KeyEvent.KEYCODE_FORWARD_DEL,
            0
        )

        assertTrue(editText.dispatchKeyEvent(downEvent))
        assertTrue(editText.dispatchKeyEvent(upEvent))
        assertEquals("abc", editText.text?.toString())
        assertEquals(1, editText.selectionStart)
        assertEquals(1, editText.selectionEnd)
    }

    @Test
    fun `soft backspace over empty block placeholder routes caret scalar through fallback`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.editorId = 1
        editText.isApplyingRustState = true
        editText.setText("\u200B")
        editText.isApplyingRustState = false
        editText.setSelection(1)

        var fallbackSelection: Pair<Int, Int>? = null
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            fallbackSelection = anchor to head
        }

        editText.handleDelete(1, 0)

        assertEquals(1 to 1, fallbackSelection)
    }

    @Test
    fun `soft backspace over escaped list placeholder routes caret scalar through fallback`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.editorId = 1
        editText.isApplyingRustState = true
        editText.setText("\u2022 A\n\u2022 B\n\u200B")
        editText.isApplyingRustState = false
        editText.setSelection(editText.text?.length ?: 0)

        var fallbackSelection: Pair<Int, Int>? = null
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            fallbackSelection = anchor to head
        }

        editText.handleDelete(1, 0)

        assertEquals(9 to 9, fallbackSelection)
    }

    @Test
    fun `hardware backspace over escaped list placeholder routes caret scalar through fallback`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.editorId = 1
        editText.isApplyingRustState = true
        editText.setText("\u2022 A\n\u2022 B\n\u200B")
        editText.isApplyingRustState = false
        editText.setSelection(editText.text?.length ?: 0)

        var fallbackSelection: Pair<Int, Int>? = null
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            fallbackSelection = anchor to head
        }

        editText.handleBackspace()

        assertEquals(9 to 9, fallbackSelection)
    }
}
