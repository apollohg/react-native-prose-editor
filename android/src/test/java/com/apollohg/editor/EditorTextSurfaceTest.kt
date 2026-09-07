package com.apollohg.editor

import android.text.Editable
import android.text.Selection
import android.text.TextWatcher
import android.text.Spanned
import android.os.Bundle
import android.view.KeyEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.widget.EditText
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class EditorTextSurfaceTest {
    @Test
    fun `production editor owns text layout without a stock editable widget`() {
        val editor = EditorEditText(RuntimeEnvironment.getApplication())
        assertFalse(EditText::class.java.isInstance(editor))
        editor.setText("first\nsecond")
        editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.AT_MOST))
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
        assertEquals("first\nsecond", editor.layout.text.toString())
        assertTrue(editor.layout.getLineTop(1) > editor.layout.getLineTop(0))
    }

    @Test
    fun `direct editable changes retain watchers selection and composing span identity`() {
        val editor = EditorEditText(RuntimeEnvironment.getApplication())
        editor.setText("first")
        var changes = 0
        editor.addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) = Unit
            override fun afterTextChanged(s: Editable?) { changes++ }
        })
        val editable = editor.editableText
        Selection.setSelection(editable, 5)
        val composing = Any()
        editable.setSpan(composing, 0, 3, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE or Spanned.SPAN_COMPOSING)
        editable.append("!")
        assertSame(editable, editor.editableText)
        assertEquals(1, changes)
        assertEquals(0, editable.getSpanStart(composing))
        assertEquals(3, editable.getSpanEnd(composing))
        assertEquals("first!", editor.text.toString())
        Selection.setSelection(editable, 2, 4)
        assertEquals(2, editor.selectionStart)
        assertEquals(4, editor.selectionEnd)
    }

    @Test
    fun `hardware navigation uses laid out lines and preserves shift anchor`() {
        val editor = measuredEditor("first\nsecond")
        editor.typeface = android.graphics.Typeface.MONOSPACE
        editor.setSelection(2)
        editor.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_DOWN))
        assertEquals(8, editor.selectionEnd)
        editor.dispatchKeyEvent(KeyEvent(0, 0, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_RIGHT, 0, KeyEvent.META_SHIFT_ON))
        assertEquals(8, editor.selectionStart)
        assertEquals(9, editor.selectionEnd)
    }

    @Test
    fun `accessibility advertises and changes text selection without replacing editable`() {
        val editor = measuredEditor("first second")
        val buffer = editor.editableText
        val info = AccessibilityNodeInfo.obtain()
        editor.onInitializeAccessibilityNodeInfo(info)
        assertTrue(info.isEditable)
        assertTrue(info.actionList.any { it.id == AccessibilityNodeInfo.ACTION_CLICK })
        assertEquals("first second", info.text.toString())
        assertTrue(editor.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, 2)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, 5)
        }))
        assertSame(buffer, editor.editableText)
        assertEquals(2, editor.selectionStart)
        assertEquals(5, editor.selectionEnd)
        assertFalse(editor.performAccessibilityAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, -1)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, 50)
        }))
    }

    @Test
    fun `reflow keeps composing spans and replaces geometry after width shrink`() {
        val editor = measuredEditor("one two three four five six seven eight nine ten")
        val buffer = editor.editableText
        val composing = Any()
        buffer.setSpan(composing, 0, 3, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE or Spanned.SPAN_COMPOSING)
        val previous = editor.layout
        editor.measure(View.MeasureSpec.makeMeasureSpec(120, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.AT_MOST))
        assertTrue(editor.layout.lineCount > previous.lineCount)
        assertEquals(120 - editor.paddingLeft - editor.paddingRight, editor.layout.width)
        assertSame(buffer, editor.editableText)
        assertEquals(3, buffer.getSpanEnd(composing))
    }

    @Test
    fun `vertical navigation remembers column across a short intervening line`() {
        val editor = measuredEditor("abcdefghij\nx\nabcdefghij")
        editor.typeface = android.graphics.Typeface.MONOSPACE
        editor.setSelection(8)
        repeat(2) { editor.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_DOWN)) }
        assertEquals(21, editor.selectionEnd)
    }

    @Test
    fun `cursor monitoring coalesces syntax style spans into one document layout`() {
        val activity = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        val editor = EditorEditText(activity)
        activity.setContentView(editor)
        editor.setText("token ".repeat(100))
        editor.requestFocus()
        editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.AT_MOST))
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
        editor.requestSurfaceCursorUpdates(android.view.inputmethod.InputConnection.CURSOR_UPDATE_MONITOR)
        val before = editor.documentLayoutBuildCount
        repeat(100) { index ->
            editor.text.setSpan(android.text.style.ForegroundColorSpan(android.graphics.Color.BLUE), index * 6, index * 6 + 5, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
        }
        assertEquals(before, editor.documentLayoutBuildCount)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idleFor(java.time.Duration.ofMillis(32))
        assertEquals(100, (editor.layout.text as Spanned).getSpans(0, editor.text.length, android.text.style.ForegroundColorSpan::class.java).size)
        assertTrue(editor.documentLayoutBuildCount <= before + 1)
    }

    @Test
    fun `accessibility word navigation skips whitespace only ranges`() {
        val editor = measuredEditor("one two")
        editor.setSelection(3)
        assertTrue(editor.performAccessibilityAction(AccessibilityNodeInfo.ACTION_NEXT_AT_MOVEMENT_GRANULARITY, Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_MOVEMENT_GRANULARITY_INT, AccessibilityNodeInfo.MOVEMENT_GRANULARITY_WORD)
        }))
        assertEquals(7, editor.selectionEnd)
    }

    @Test
    fun `standalone fixed surface reveals caret while a scroll host owns nested scrolling`() {
        val editor = measuredEditor("line\n".repeat(50))
        editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(120, View.MeasureSpec.EXACTLY))
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
        assertTrue(editor.bringPointIntoView(editor.text.length))
        assertTrue(editor.scrollY > 0)
        editor.bringPointIntoView(0)
        assertEquals(0, editor.scrollY)
    }

    @Test
    fun `final line selection handles stay inside a surface without bottom padding`() {
        val editor = measuredEditor("abc")
        editor.setPadding(0, 0, 0, 0)
        editor.measure(View.MeasureSpec.makeMeasureSpec(120, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED))
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
        editor.requestFocus()
        editor.setSelection(1, 2)
        val bitmap = android.graphics.Bitmap.createBitmap(editor.width, editor.height, android.graphics.Bitmap.Config.ARGB_8888)
        editor.interaction.drawHandles(android.graphics.Canvas(bitmap))
        assertTrue((0 until editor.width).any { x ->
            android.graphics.Color.alpha(bitmap.getPixel(x, editor.height - 2)) > 0
        })
        bitmap.recycle()
    }

    @Test
    fun `unmeasured host does not shape a document at one pixel width`() {
        val host = RichTextEditorView(RuntimeEnvironment.getApplication())
        host.editorEditText.applyRenderJSON("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"A paragraph waiting for its real width.","marks":[]},{"type":"blockEnd"}]""")
        assertEquals(0, host.editorEditText.documentLayoutBuildCount)
    }

    @Test
    fun `horizontal arrows collapse RTL selections toward the physical edge`() {
        val editor = measuredEditor("אבג")
        editor.setSelection(0, 3)
        editor.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_LEFT))
        assertEquals(3, editor.selectionEnd)
        editor.setSelection(0, 3)
        editor.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_RIGHT))
        assertEquals(0, editor.selectionEnd)
    }

    private fun measuredEditor(value: String): EditorEditText {
        val editor = EditorEditText(RuntimeEnvironment.getApplication())
        editor.setText(value)
        editor.measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.AT_MOST))
        editor.layout(0, 0, editor.measuredWidth, editor.measuredHeight)
        return editor
    }

}
