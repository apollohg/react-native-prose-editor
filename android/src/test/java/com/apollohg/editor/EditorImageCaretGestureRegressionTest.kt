package com.apollohg.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.SystemClock
import android.view.MotionEvent
import android.view.View
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
class EditorImageCaretGestureRegressionTest {
    private fun editor(): EditorEditText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
        showSoftInputOnFocus = false
        applyTheme(EditorTheme.fromJson("""{"version":1,"styles":{"content":{"padding":20},"text":{"fontSize":17},"paragraph":{"lineHeight":27,"marginBottom":12},"image":{"marginVertical":12}}}"""))
        applyRenderJSON("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"Hello","marks":[]},{"type":"blockEnd"},{"type":"voidBlock","nodeType":"image","docPos":7,"attrs":{"src":"https://example.com/image.png","width":140,"height":80}},{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"After","marks":[]},{"type":"blockEnd"}]""")
        measure(View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.AT_MOST))
        layout(0, 0, measuredWidth, measuredHeight)
    }

    @Test
    fun `tap at paragraph end above image places text caret`() {
        val editor = editor()
        val x = editor.totalPaddingLeft + editor.layout.getPrimaryHorizontal(5) + 4f
        val y = editor.totalPaddingTop + editor.layout.getLineBaseline(0) - 3f
        tap(editor, x, y)
        assertEquals(5, editor.selectionStart)
        assertEquals(5, editor.selectionEnd)
        assertNull(editor.selectedImageGeometry())
    }

    @Test
    fun `tap inside image still selects it`() {
        val editor = editor()
        val span = editor.text.getSpans(0, editor.text.length, BlockImageSpan::class.java).single()
        val start = editor.text.getSpanStart(span)
        val end = editor.text.getSpanEnd(span)
        val bounds = editor.resolvedImageRect(editor.layout, span, start, end)
        tap(editor, bounds.centerX(), bounds.centerY())
        assertEquals(start, editor.selectionStart)
        assertEquals(end, editor.selectionEnd)
    }

    @Test
    fun `scrolled image hit testing follows its visible bounds`() {
        val editor = editor()
        val span = editor.text.getSpans(0, editor.text.length, BlockImageSpan::class.java).single()
        val start = editor.text.getSpanStart(span)
        val end = editor.text.getSpanEnd(span)
        val bounds = editor.resolvedImageRect(editor.layout, span, start, end)
        editor.scrollTo(0, bounds.top.toInt())
        tap(editor, bounds.centerX(), 4f)
        assertEquals(start, editor.selectionStart)
        assertEquals(end, editor.selectionEnd)
    }

    @Test
    fun `handle stays joined to caret at the left edge`() {
        val editor = editor()
        editor.setPadding(0, 0, 0, 0)
        editor.requestFocus()
        editor.setSelection(0, 2)
        val caret = CaretGeometry.verticalBounds(editor.layout, 0, editor.paint, editor.text)
        val bitmap = Bitmap.createBitmap(editor.width, editor.height, Bitmap.Config.ARGB_8888)
        editor.interaction.drawHandles(Canvas(bitmap))
        val x = editor.layout.getPrimaryHorizontal(0).toInt()
        val y = kotlin.math.ceil(caret.bottom.toDouble()).toInt()
        assertTrue("Handle detached horizontally at ($x, $y)", Color.alpha(bitmap.getPixel(x, y)) > 0)
        bitmap.recycle()
    }

    @Test
    fun `insertion handle stem joins the drawn caret with paragraph line height`() {
        val editor = editor()
        val x = editor.totalPaddingLeft + editor.layout.getPrimaryHorizontal(2)
        val y = editor.totalPaddingTop + editor.layout.getLineBaseline(0) - 3f
        tap(editor, x, y)
        assertEquals(2, editor.selectionEnd)
        val caret = requireNotNull(editor.nativeCursorDrawRect())
        val bitmap = Bitmap.createBitmap(editor.width, editor.height, Bitmap.Config.ARGB_8888)
        editor.interaction.drawHandles(Canvas(bitmap))
        val stemX = editor.layout.getPrimaryHorizontal(2).toInt()
        val bottom = editor.layout.editorTextLineBottom(0)
        for (pixelY in kotlin.math.ceil(caret.bottom.toDouble()).toInt()..bottom) {
            assertTrue("Disconnected handle at y=$pixelY, caret=$caret, lineBottom=$bottom", Color.alpha(bitmap.getPixel(stemX, pixelY)) > 0)
        }
        bitmap.recycle()
    }

    private fun tap(editor: EditorEditText, x: Float, y: Float) {
        val time = SystemClock.uptimeMillis()
        for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
            MotionEvent.obtain(time, time + if (action == MotionEvent.ACTION_UP) 30 else 0, action, x, y, 0).also {
                editor.dispatchTouchEvent(it)
                it.recycle()
            }
        }
    }
}
