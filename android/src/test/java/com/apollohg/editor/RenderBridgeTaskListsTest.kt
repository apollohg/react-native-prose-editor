package com.apollohg.editor
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.text.Annotation
import android.text.Layout
import android.text.Spanned
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.util.Base64
import android.view.View
import android.view.ViewGroup
import android.widget.TextView
import kotlin.math.abs
import android.text.style.AbsoluteSizeSpan
import android.text.style.BackgroundColorSpan
import android.text.style.ForegroundColorSpan
import android.text.style.LeadingMarginSpan
import android.text.style.StrikethroughSpan
import android.text.style.StyleSpan
import android.text.style.TypefaceSpan
import android.text.style.URLSpan
import android.text.style.UnderlineSpan
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class RenderBridgeTaskListsTest : RenderBridgeTestFixture() {
    @Test
    fun `task list marker carries the task marker annotation`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "taskItem", "depth": 1,
             "listContext": {"ordered": false, "index": 1, "total": 1, "start": 1,
                             "isFirst": true, "isLast": true, "kind": "task", "checked": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 2},
            {"type": "textRun", "text": "todo", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertTrue(
            "Task list item should start with the unchecked marker. Got: '${result}'",
            result.toString().startsWith(LayoutConstants.TASK_LIST_MARKER_UNCHECKED)
        )
        val annotations = result.getSpans(0, 2, android.text.Annotation::class.java)
            .filter { it.key == RenderBridge.NATIVE_TASK_LIST_MARKER_ANNOTATION }
        assertEquals("Marker chars must carry the task-marker annotation", 1, annotations.size)
        assertEquals(0, result.getSpanStart(annotations[0]))
        assertEquals(2, result.getSpanEnd(annotations[0]))
    }

    @Test
    fun `task marker kind takes precedence over ordered visual presentation`() {
        val json = """
        [
            {"type": "blockStart", "nodeType": "taskItem", "depth": 0,
             "listContext": {"ordered": true, "index": 1, "total": 1, "start": 1,
                             "isFirst": true, "isLast": true, "kind": "task", "checked": false}},
            {"type": "blockStart", "nodeType": "paragraph", "depth": 1},
            {"type": "textRun", "text": "todo", "marks": []},
            {"type": "blockEnd"},
            {"type": "blockEnd"}
        ]
        """.trimIndent()

        val result = RenderBridge.buildSpannable(json, baseFontSize, textColor)

        assertTrue(result.toString().startsWith(LayoutConstants.TASK_LIST_MARKER_UNCHECKED))
        val annotations = result.getSpans(0, 2, android.text.Annotation::class.java)
            .filter { it.key == RenderBridge.NATIVE_TASK_LIST_MARKER_ANNOTATION }
        assertEquals(1, annotations.size)
        assertTrue(
            result.getSpans(0, result.length, OrderedListMarkerSpan::class.java).isEmpty()
        )
    }
}
