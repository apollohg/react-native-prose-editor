package com.apollohg.editor

import android.view.KeyEvent
import org.junit.Assert.assertEquals
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
}
