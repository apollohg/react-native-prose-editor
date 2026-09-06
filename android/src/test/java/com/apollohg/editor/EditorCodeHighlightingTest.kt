package com.apollohg.editor

import android.app.Activity
import android.os.Looper
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@org.robolectric.annotation.GraphicsMode(org.robolectric.annotation.GraphicsMode.Mode.NATIVE)
class EditorCodeHighlightingTest {
    @Test
    fun `syntax traits independently map bold underline and italic bits`() {
        fun paint(trait: Int) = android.text.TextPaint().apply {
            typeface = android.graphics.Typeface.DEFAULT
            EditorCodeHighlightSpan(CodeHighlightRange(0, 1, 0xff0000ff, trait)).updateDrawState(this)
        }
        assertTrue(paint(1).typeface.isBold)
        assertFalse(paint(1).isUnderlineText)
        assertTrue(paint(2).isUnderlineText)
        assertFalse(paint(2).typeface.isItalic)
        assertTrue(paint(4).typeface.isItalic)
        assertFalse(paint(4).isUnderlineText)
    }
    @Test
    fun `removal rejects pending syntax without changing text or selection`() {
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val finished = CountDownLatch(1)
        CodeHighlightingRegistry.register(object : CodeHighlightingProvider {
            override val id = "editor-test"
            override val version = 1
            override fun highlight(text: String, language: String?, theme: String): List<CodeHighlightRange> {
                started.countDown()
                release.await(3, TimeUnit.SECONDS)
                finished.countDown()
                return listOf(CodeHighlightRange(0, text.length, 0xff0000ff, 1))
            }
        })
        val controller = Robolectric.buildActivity(Activity::class.java).setup()
        val editor = EditorEditText(controller.get())
        controller.get().setContentView(editor)
        editor.applyRenderJSON("""[{"type":"blockStart","nodeType":"codeBlock","depth":0,"language":"rust"},{"type":"textRun","text":"let x = 1;","marks":[]},{"type":"blockEnd"}]""")
        editor.setSelection(3)
        editor.setCodeHighlighting(NativeCodeHighlightingConfig("editor-test", "theme"))
        assertTrue(started.await(3, TimeUnit.SECONDS))
        editor.setCodeHighlighting(null)
        release.countDown()
        assertTrue(finished.await(3, TimeUnit.SECONDS))
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals("let x = 1;", editor.text.toString())
        assertEquals(3, editor.selectionStart)
        assertTrue(editor.text!!.getSpans(0, editor.length(), EditorCodeHighlightSpan::class.java).isEmpty())
        controller.pause().stop().destroy()
    }
}
