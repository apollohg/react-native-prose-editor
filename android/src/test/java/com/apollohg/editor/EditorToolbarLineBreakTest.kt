package com.apollohg.editor

import android.view.View
import android.view.inputmethod.EditorInfo
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
internal class EditorToolbarLineBreakTest : EditorInputConnectionTestFixture() {
    @Test
    fun `toolbar line break moves caret to next line before typing`() {
        assertToolbarLineBreak("Before")
    }

    @Test
    fun `toolbar line break in empty paragraph moves caret before typing`() {
        assertToolbarLineBreak("")
    }

    @Test
    fun `toolbar line break after existing break moves caret before typing`() {
        assertToolbarLineBreak("Before<br>")
    }

    @Test
    fun `toolbar line break after blank line keeps caret with inserted text`() {
        assertToolbarLineBreak("<br>")
    }

    private fun assertToolbarLineBreak(initialHtml: String) {
        val harness = realExternalCompositionHarness(initialHtml)
        try {
            val editor = harness.editText
            harness.adapter.claimNativeBindingIfUnowned(1L)
            editor.setSelection(editor.text!!.length)
            measureEditor(editor)
            val before = requireNotNull(editor.nativeCursorDrawRect())
            val prefix = editor.text.toString().replace(LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER, "")

            val nodeType = editor.preferredHardBreakNodeType()
            editor.performToolbarInsertNode(nodeType)
            measureEditor(editor)

            assertEquals(prefix + "\n" + LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER, editor.text.toString())
            val after = requireNotNull(editor.nativeCursorDrawRect())
            assertTrue("before=$before after=$after text=${editor.text} selection=${editor.selectionStart}", after.top >= before.bottom)
            assertEquals(0f, after.left, 1f)
            val breakText = editor.text.toString()
            val breakSelection = editor.selectionStart
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            assertTrue(input.commitText("Q", 1))
            measureEditor(editor)
            assertEquals(prefix + "\nQ", editor.text.toString())
            val typed = requireNotNull(editor.nativeCursorDrawRect())
            assertEquals("after=$after typed=$typed breakText=$breakText breakSelection=$breakSelection text=${editor.text} selection=${editor.selectionStart} document=${harness.adapter.documentJson()}", after.top, typed.top, 1f)
            assertTrue("after=$after typed=$typed text=${editor.text} selection=${editor.selectionStart}", typed.left > after.left)
            val content = JSONObject(requireNotNull(harness.adapter.documentJson()))
                .getJSONArray("content").getJSONObject(0).getJSONArray("content")
            assertEquals(nodeType, content.getJSONObject(content.length() - 2).getString("type"))
            assertEquals("Q", content.getJSONObject(content.length() - 1).getString("text"))
        } finally {
            harness.adapter.destroy()
        }
    }

    private fun measureEditor(editor: EditorEditText) {
        editor.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
        )
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
    }
}
