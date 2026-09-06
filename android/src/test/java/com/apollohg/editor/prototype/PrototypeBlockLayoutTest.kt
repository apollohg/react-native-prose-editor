package com.apollohg.editor.prototype

import android.graphics.RectF
import android.text.TextPaint
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class PrototypeBlockLayoutTest {
    private val paint = TextPaint().apply { textSize = 20f }

    @Test
    fun `right padding changes wrapping only in its own paragraph`() {
        val paragraph = "One paragraph with enough words to wrap across several lines."
        val text = "$paragraph\n$paragraph"
        val ordinary = PrototypeBlockLayout(text, 240, paint) { PrototypeInsets(12, 12) }
        val inset = PrototypeBlockLayout(text, 240, paint) { PrototypeInsets(12, if (it == 0) 112 else 12) }
        val narrowLast = inset.caret(paragraph.length)
        val ordinaryLast = ordinary.caret(paragraph.length)
        assertTrue(narrowLast.top > ordinaryLast.top)
        val secondStart = paragraph.length + 1
        assertEquals(
            ordinary.caret(text.length).top - ordinary.caret(secondStart).top,
            inset.caret(text.length).top - inset.caret(secondStart).top,
            0.01f,
        )
        for (offset in 0..paragraph.length) assertTrue(inset.caret(offset).left <= 128f)
    }

    @Test
    fun `drawing geometry maps back to global offsets in both blocks`() {
        val text = "abc def ghi\nSecond paragraph"
        val layout = PrototypeBlockLayout(text, 400, paint) { PrototypeInsets(if (it == 0) 16 else 70, 30) }
        for (offset in text.indices.filter { text[it] != '\n' }) {
            val caret = layout.caret(offset)
            assertEquals("offset $offset", offset, layout.offsetAt(caret.left, caret.centerY()))
        }
        assertEquals(70f, layout.caret(text.indexOf('\n') + 1).left, 0.01f)
    }

    @Test
    fun `selection spans paragraph boundaries and reverses without changing shape`() {
        val text = "First paragraph\nSecond paragraph"
        val layout = PrototypeBlockLayout(text, 400, paint) { PrototypeInsets(20, 40) }
        val forward = RectF().also { layout.selection(3, 23).computeBounds(it, true) }
        val reverse = RectF().also { layout.selection(23, 3).computeBounds(it, true) }
        assertEquals(forward, reverse)
        assertEquals(layout.caret(3).top, forward.top, 0.01f)
        assertEquals(layout.caret(23).bottom, forward.bottom, 0.01f)
        assertFalse(layout.selection(0, text.length).isEmpty)
        assertTrue(layout.selection(3, 3).isEmpty)
    }

    @Test
    fun `empty final paragraph has a usable caret and hit target`() {
        val layout = PrototypeBlockLayout("abc\n", 200, paint) { PrototypeInsets(20, 30) }
        val caret = layout.caret(4)
        assertTrue(caret.top > layout.caret(0).bottom)
        assertEquals(4, layout.offsetAt(caret.left, caret.centerY()))
        assertTrue(PrototypeBlockLayout("", 200, paint) { PrototypeInsets(0, 0) }.height > 0)
    }

    @Test
    fun `RTL paragraphs keep physical insets and emoji cannot split a surrogate`() {
        val text = "אבג דהו\nA😀B"
        val layout = PrototypeBlockLayout(text, 300, paint) { PrototypeInsets(25, 75) }
        val rtl = layout.caret(0)
        assertEquals(225f, rtl.left, 0.01f)
        assertEquals(0, layout.offsetAt(rtl.left, rtl.centerY()))
        val emojiStart = text.indexOf("😀")
        assertEquals(layout.caret(emojiStart + 2), layout.caret(emojiStart + 1))
    }
}
