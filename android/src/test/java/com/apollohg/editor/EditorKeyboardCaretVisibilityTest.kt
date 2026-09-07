package com.apollohg.editor

import android.app.Activity
import android.os.Looper
import android.view.View
import android.widget.FrameLayout
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorKeyboardCaretVisibilityTest {
    @Test
    fun `keyboard opening reveals caret at end of scrolling content`() {
        assertKeyboardRevealsCaret(100)
    }

    @Test
    fun `keyboard opening reveals caret near end of scrolling content`() {
        assertKeyboardRevealsCaret(96)
    }

    private fun assertKeyboardRevealsCaret(lineNumber: Int) {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val view = RichTextEditorView(activity)
        view.setHeightBehavior(EditorHeightBehavior.FIXED)
        host.addView(view, FrameLayout.LayoutParams(360, 600))
        val editor = view.editorEditText
        editor.setText((1..100).joinToString("\n") { "Line $it" })
        measureHost(host)
        assertTrue(editor.requestFocus())
        val caretOffset = editor.text.toString().indexOf("Line $lineNumber") + "Line $lineNumber".length
        editor.setSelection(caretOffset)
        shadowOf(Looper.getMainLooper()).idle()
        view.editorScrollView.scrollTo(0, editor.height)
        val keyboardTop = 300
        val location = IntArray(2)
        view.editorScrollView.getLocationOnScreen(location)

        view.setViewportBottomOcclusionTopOnScreenPx(location[1] + keyboardTop)
        view.setViewportBottomInsetPx(600 - keyboardTop)
        measureHost(host)
        shadowOf(Looper.getMainLooper()).idle()
        view.editorScrollView.computeScroll()
        shadowOf(Looper.getMainLooper()).idle()

        val caret = requireNotNull(view.caretRect())
        assertEquals(caretOffset, editor.selectionEnd)
        assertTrue("caret=$caret keyboardTop=$keyboardTop scroll=${view.editorScrollView.scrollY} padding=${view.editorScrollView.paddingBottom}", caret.bottom <= keyboardTop)
    }

    private fun measureHost(host: FrameLayout) {
        host.measure(
            View.MeasureSpec.makeMeasureSpec(360, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY),
        )
        host.layout(0, 0, 360, 600)
    }
}
