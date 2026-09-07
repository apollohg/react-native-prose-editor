package com.apollohg.editor

import android.view.inputmethod.EditorInfo
import android.view.View
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorCodeBlockReturnTest : EditorInputConnectionTestFixture() {
    @Test
    fun `return adds code line and extra return exits code block`() {
        assertCodeBlockReturn("<p>After</p>", false)
    }

    @Test
    fun `return permits typing another code line at document end before exiting`() {
        assertCodeBlockReturn("", true)
    }

    private fun assertCodeBlockReturn(suffix: String, typeCode: Boolean) {
        val harness = structuredDeleteHarness("<pre><code>let x = 1</code></pre>" + suffix)
        try {
            val editor = harness.editText
            editor.applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"codeBlock":{"fontSize":26,"lineHeight":40,"padding":12,"backgroundColor":"#eeeeee"}}}"""))
            harness.adapter.claimNativeBindingIfUnowned(1L)
            editor.setSelection(9)
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))

            assertTrue(input.commitText("\n", 1))

            var blocks = JSONObject(requireNotNull(harness.adapter.documentJson())).getJSONArray("content")
            assertEquals("codeBlock", blocks.getJSONObject(0).getString("type"))
            assertEquals("let x = 1\n", codeText(blocks.getJSONObject(0)))
            assertTrue("text=${editor.text} selection=${editor.selectionEnd}", editor.text.toString().startsWith("let x = 1\n"))
            assertEquals(10, editor.selectionEnd)
            editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED))
            editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
            val layout = editor.layout as EditorDocumentLayout
            val codeBox = editor.text.getSpans(0, editor.text.length, EditorBlockBoxSpan::class.java).first { it.nodeType == "codeBlock" }
            val bounds = requireNotNull(layout.boxBounds(codeBox))
            val caret = requireNotNull(editor.nativeCursorDrawRect())
            val line = layout.getLineForOffset(editor.selectionEnd)
            assertTrue("blank line height=${layout.editorTextLineBottom(line) - layout.editorTextLineTop(line)}", layout.editorTextLineBottom(line) - layout.editorTextLineTop(line) >= 40)
            assertTrue("caret=$caret code=$bounds", caret.bottom <= bounds.bottom && caret.left >= bounds.left + 11f)

            val expectedCode = if (typeCode) {
                assertTrue(input.commitText("next", 1))
                assertTrue(input.commitText("\n", 1))
                "let x = 1\nnext"
            } else "let x = 1"
            assertTrue(input.commitText("\n", 1))

            blocks = JSONObject(requireNotNull(harness.adapter.documentJson())).getJSONArray("content")
            assertEquals(if (suffix.isEmpty()) 2 else 3, blocks.length())
            assertEquals("codeBlock", blocks.getJSONObject(0).getString("type"))
            assertEquals(expectedCode, codeText(blocks.getJSONObject(0)))
            assertEquals("paragraph", blocks.getJSONObject(1).getString("type"))
            assertTrue(input.commitText("Outside", 1))
            blocks = JSONObject(requireNotNull(harness.adapter.documentJson())).getJSONArray("content")
            assertEquals("Outside", blocks.getJSONObject(1).getJSONArray("content").getJSONObject(0).getString("text"))
        } finally {
            harness.adapter.destroy()
        }
    }

    private fun codeText(block: JSONObject): String {
        val content = block.getJSONArray("content")
        return (0 until content.length()).joinToString("") { content.getJSONObject(it).getString("text") }
    }
}
