package com.apollohg.editor
import android.graphics.Color
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Rect
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.text.Spanned
import android.text.style.ForegroundColorSpan
import android.text.style.LeadingMarginSpan
import android.widget.LinearLayout
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RichTextEditorViewTaskMarkersTest : RichTextEditorViewTestFixture() {
    @Test
    fun `tapping rendered task marker toggles task item`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.applyRenderJSON(singleTaskListRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val textLayout = requireNotNull(editText.layout)
        val tapX = editText.totalPaddingLeft + 1f
        val tapY = editText.totalPaddingTop +
            ((textLayout.getLineTop(0) + textLayout.getLineBottom(0)) / 2f)
        val toggles = mutableListOf<Pair<Int, Int>>()
        editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { anchor, head ->
            toggles += anchor to head
        }

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(listOf(0 to 0), toggles)
    }

    @Test
    fun `tapping nested list leading margin snaps caret to item text`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.applyRenderJSON(nestedListRenderBlock(ListRenderState.INITIAL).toString())
        val parent = FrameLayout(context)
        parent.addView(
            editText,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT
            )
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.EXACTLY)
        parent.measure(widthSpec, heightSpec)
        parent.layout(0, 0, parent.measuredWidth, parent.measuredHeight)

        val content = editText.text as Spanned
        val bodyStart = content.toString().indexOf("Nested")
        val marker = content
            .getSpans(0, bodyStart, android.text.Annotation::class.java)
            .single {
                it.key == RenderBridge.NATIVE_LIST_MARKER_ANNOTATION &&
                    content.getSpanEnd(it) == bodyStart
            }
        val markerStart = content.getSpanStart(marker)
        val markerEnd = content.getSpanEnd(marker)
        assertEquals(bodyStart, markerEnd)

        editText.setSelection(bodyStart + 2)
        val syncedSelections = mutableListOf<Pair<Int, Int>>()
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelections += anchor to head
        }

        val textLayout = requireNotNull(editText.layout)
        val line = textLayout.getLineForOffset(markerStart)
        val tapX = editText.totalPaddingLeft + textLayout.getPrimaryHorizontal(markerStart) + 1f
        val tapY = editText.totalPaddingTop +
            ((textLayout.getLineTop(line) + textLayout.getLineBottom(line)) / 2f)

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()
        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(markerEnd, editText.selectionStart)
        assertEquals(markerEnd, editText.selectionEnd)
        assertEquals(markerEnd to markerEnd, syncedSelections.last())
    }

    @Test
    fun `tapping below rendered task marker does not toggle nearest task item`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.applyRenderJSON(singleTaskListRenderJson())
        editText.layoutParams = FrameLayout.LayoutParams(600, 240)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val textLayout = requireNotNull(editText.layout)
        val tapX = editText.totalPaddingLeft + 1f
        val tapY = editText.totalPaddingTop + textLayout.getLineBottom(0) + 24f
        var toggleCount = 0
        editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { _, _ ->
            toggleCount += 1
        }

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(0, toggleCount)
    }

    @Test
    fun `tapping plain paragraph starting with checkbox glyph does not toggle task item`() {
        // Regression: marker hit-testing must key off the nativeTaskListMarker
        // annotation, not the leading glyph. A plain paragraph whose text
        // happens to start with "☐ " (no listContext, no annotation) must not
        // be treated as a task marker.
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.applyRenderJSON(plainParagraphStartingWithCheckboxGlyphRenderJson())

        assertTrue(
            "Rendered text should start with the checkbox glyph. Got: '${editText.text}'",
            editText.text.toString().startsWith(LayoutConstants.TASK_LIST_MARKER_UNCHECKED)
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val textLayout = requireNotNull(editText.layout)
        val tapX = editText.totalPaddingLeft + 1f
        val tapY = editText.totalPaddingTop +
            ((textLayout.getLineTop(0) + textLayout.getLineBottom(0)) / 2f)
        var toggleCount = 0
        editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { _, _ ->
            toggleCount += 1
        }

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(
            "Tapping a plain paragraph's checkbox-like glyph must not toggle any task item",
            0,
            toggleCount
        )
    }

    @Test
    fun `down on marker then up elsewhere does not toggle`() {
        // A DOWN that lands on the marker followed by an UP far away (e.g. a
        // selection drag or a scroll gesture that started on the checkbox)
        // must not toggle the task item. Critically, the DOWN itself must
        // NOT be consumed by the marker handler: the pre-fix handler
        // hit-tested and unconditionally returned true for a DOWN on a
        // marker, short-circuiting onTouchEvent before the FIXED-height
        // scroll-intercept handling below it ever ran -- which is exactly
        // the "scrolls that start on a checkbox get blocked" bug. We prove
        // the DOWN reached that code by observing its side effect: it asks
        // the parent to disallow intercept while a FIXED-height, overflowing
        // editor is being touched.
        val context = RuntimeEnvironment.getApplication()
        val parent = InterceptAwareFrameLayout(context)
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.setHeightBehavior(EditorHeightBehavior.FIXED)
        editText.applyRenderJSON(taskListWithOverflowRenderJson())
        parent.addView(
            editText,
            FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, 120)
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(120, View.MeasureSpec.EXACTLY)
        parent.measure(widthSpec, heightSpec)
        parent.layout(0, 0, parent.measuredWidth, parent.measuredHeight)

        assertTrue(
            "Test setup requires the FIXED-height editor content to overflow vertically",
            editText.canScrollVertically(1)
        )

        val textLayout = requireNotNull(editText.layout)
        val tapX = editText.totalPaddingLeft + 1f
        val tapY = editText.totalPaddingTop +
            ((textLayout.getLineTop(0) + textLayout.getLineBottom(0)) / 2f)
        val toggles = mutableListOf<Pair<Int, Int>>()
        editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { anchor, head ->
            toggles += anchor to head
        }
        // Reaching super.onTouchEvent for real now drives the normal
        // EditText tap-to-place-cursor path, which syncs the new selection
        // to Rust. Route that through the testing hook (the suite's
        // established pattern, see EditorInputConnectionTest.kt) instead of
        // a real FFI call, since this test isn't exercising selection sync.
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()

        assertTrue(
            "ACTION_DOWN on a marker must reach the FIXED-height scroll handling so drags/scrolls starting on a checkbox keep working",
            parent.disallowInterceptRequested
        )

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY + 200f, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(
            "Lifting far away from the DOWN's marker must not toggle any task item",
            emptyList<Pair<Int, Int>>(),
            toggles
        )
    }

    @Test
    fun `clean tap on marker toggles exactly once`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.applyRenderJSON(singleTaskListRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val textLayout = requireNotNull(editText.layout)
        val tapX = editText.totalPaddingLeft + 1f
        val tapY = editText.totalPaddingTop +
            ((textLayout.getLineTop(0) + textLayout.getLineBottom(0)) / 2f)
        val toggles = mutableListOf<Pair<Int, Int>>()
        editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { anchor, head ->
            toggles += anchor to head
        }

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(
            "A clean DOWN+UP pair on the same marker must toggle exactly once",
            listOf(0 to 0),
            toggles
        )
    }

    @Test
    fun `up over marker without a paired down does not toggle`() {
        // Simulates the UP a selection drag delivers when it happens to end
        // over a marker, without that gesture's DOWN having started there.
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.editorId = 1
        editText.applyRenderJSON(singleTaskListRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val textLayout = requireNotNull(editText.layout)
        val tapX = editText.totalPaddingLeft + 1f
        val tapY = editText.totalPaddingTop +
            ((textLayout.getLineTop(0) + textLayout.getLineBottom(0)) / 2f)
        var toggleCount = 0
        editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { _, _ ->
            toggleCount += 1
        }

        val up = MotionEvent.obtain(0, 0, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(
            "An UP over a marker with no preceding paired DOWN must not toggle",
            0,
            toggleCount
        )
    }
}
