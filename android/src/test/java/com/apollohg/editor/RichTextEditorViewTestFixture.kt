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

internal abstract class RichTextEditorViewTestFixture {
    protected class InterceptAwareFrameLayout(context: android.content.Context) : FrameLayout(context) {
        var disallowInterceptRequested = false

        override fun requestDisallowInterceptTouchEvent(disallowIntercept: Boolean) {
            disallowInterceptRequested = disallowIntercept
            super.requestDisallowInterceptTouchEvent(disallowIntercept)
        }
    }

    protected class CaretVisibilityParent(context: android.content.Context) : FrameLayout(context) {
        val requestedRectangles = mutableListOf<Rect>()
        var verticallyScrollable = false

        override fun canScrollVertically(direction: Int): Boolean = verticallyScrollable

        override fun requestChildRectangleOnScreen(
            child: View,
            rectangle: Rect,
            immediate: Boolean
        ): Boolean {
            requestedRectangles += Rect(rectangle)
            return true
        }
    }

    protected data class CaretVisibilityFixture(
        val parent: CaretVisibilityParent,
        val editText: EditorEditText
    )

    protected data class ImageResizeGestureFixture(
        val parent: InterceptAwareFrameLayout,
        val view: RichTextEditorView,
        val resizeCommands: MutableList<Triple<Int, Int, Int>>,
    )

    protected fun autoGrowCaretVisibilityFixture(
        editorFocused: Boolean = true,
        bottomClearance: Int = 0
    ): CaretVisibilityFixture {
        val activity = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java)
            .setup()
            .get()
        val parent = CaretVisibilityParent(activity).apply {
            isFocusableInTouchMode = !editorFocused
        }
        activity.setContentView(parent)
        val editText = EditorEditText(activity).apply {
            setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
            setViewportBottomInsetPx(bottomClearance)
            setText("First line\nSecond line\nThird line")
        }
        parent.addView(
            editText,
            FrameLayout.LayoutParams(600, ViewGroup.LayoutParams.WRAP_CONTENT)
        )
        parent.measure(
            View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(900, View.MeasureSpec.EXACTLY)
        )
        parent.layout(0, 0, 600, 900)
        assertTrue(if (editorFocused) editText.requestFocus() else parent.requestFocus())
        editText.setSelection(0)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        parent.requestedRectangles.clear()
        return CaretVisibilityFixture(parent, editText)
    }

    protected fun exampleTheme(markerScale: Float = 2f): EditorTheme? =
        EditorTheme.fromJson(
            """
            {
              "backgroundColor": "#f6f1e8",
              "text": { "color": "#2a2118", "fontSize": 17 },
              "paragraph": { "spacingAfter": 16 },
              "list": { "indent": 14, "itemSpacing": 6, "markerColor": "#9a4f2d", "markerScale": $markerScale }
            }
            """.trimIndent()
        )

    protected fun exampleRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Native Editor example app.","marks":["bold"]},
          {"type":"blockEnd"},
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Use this screen to test focus, theme updates, lists, line breaks, toolbar behavior, and optional addons.","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Enable mentions above, then type @ after a space, on a blank line, or after punctuation to show native mention suggestions in the toolbar.","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockStart","nodeType":"listItem","depth":1,"listContext":{"ordered":false,"index":1,"total":2,"start":1,"isFirst":true,"isLast":false}},
          {"type":"blockStart","nodeType":"paragraph","depth":2},
          {"type":"textRun","text":"Try typing","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockEnd"},
          {"type":"blockStart","nodeType":"listItem","depth":1,"listContext":{"ordered":false,"index":2,"total":2,"start":1,"isFirst":false,"isLast":true}},
          {"type":"blockStart","nodeType":"paragraph","depth":2},
          {"type":"textRun","text":"Try list indenting","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockEnd"},
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    protected fun singleBulletListRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}},
          {"type":"blockStart","nodeType":"paragraph","depth":1},
          {"type":"textRun","text":"Bullet item","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    protected fun singleTaskListRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"taskItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true,"kind":"task","checked":false}},
          {"type":"blockStart","nodeType":"paragraph","depth":1},
          {"type":"textRun","text":"Task item","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    /**
     * A single task item followed by enough filler paragraphs to overflow a
     * small FIXED-height viewport, so `canScrollVertically` is true and
     * ACTION_DOWN/MOVE on the marker can be observed reaching the
     * FIXED-height parent-intercept handling in EditorEditText.onTouchEvent.
     */
    protected fun taskListWithOverflowRenderJson(fillerLineCount: Int = 30): String {
        val blocks = StringBuilder()
        blocks.append(
            """
            {"type":"blockStart","nodeType":"taskItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true,"kind":"task","checked":false}},
            {"type":"blockStart","nodeType":"paragraph","depth":1},
            {"type":"textRun","text":"Task item","marks":[]},
            {"type":"blockEnd"},
            {"type":"blockEnd"}
            """.trimIndent()
        )
        for (index in 1..fillerLineCount) {
            blocks.append(
                """,{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"Filler line $index","marks":[]},{"type":"blockEnd"}"""
            )
        }
        return "[$blocks]"
    }

    protected fun plainParagraphStartingWithCheckboxGlyphRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"☐ not a task","marks":[]},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    protected fun emptyParagraphRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"\u200B","marks":[]},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    protected fun imageResizeGestureFixture(renderJson: String): ImageResizeGestureFixture {
        val context = RuntimeEnvironment.getApplication()
        val parent = InterceptAwareFrameLayout(context)
        val view = RichTextEditorView(context)
        view.setEditorIdWhileDetached(1)
        view.editorEditText.editorId = 1
        view.editorEditText.applyRenderJSON(renderJson)
        val resizeCommands = mutableListOf<Triple<Int, Int, Int>>()
        view.editorEditText.onResizeImageAtDocPosForTesting = { docPos, width, height ->
            resizeCommands += Triple(docPos, width, height)
        }
        parent.addView(
            view,
            FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.MATCH_PARENT,
            ),
        )
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.EXACTLY)
        parent.measure(widthSpec, heightSpec)
        parent.layout(0, 0, parent.measuredWidth, parent.measuredHeight)
        val text = view.editorEditText.text as Spanned
        val first = text.getSpans(0, text.length, BlockImageSpan::class.java).first()
        view.editorEditText.setSelection(text.getSpanStart(first), text.getSpanEnd(first))
        view.editorEditText.onSelectionOrContentMayChange?.invoke()
        return ImageResizeGestureFixture(parent, view, resizeCommands)
    }

    protected fun imageRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Hello","marks":[]},
          {"type":"blockEnd"},
          {"type":"voidBlock","nodeType":"image","docPos":7,"attrs":{"src":"https://example.com/cat.png","width":140,"height":80}},
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    protected fun twoImageRenderJson(): String = """
        [
          {"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"https://example.com/first.png","width":120,"height":60}},
          {"type":"voidBlock","nodeType":"image","docPos":2,"attrs":{"src":"https://example.com/second.png","width":120,"height":60}}
        ]
    """.trimIndent()

    protected fun paragraphRenderBlock(text: String): JSONArray {
        return JSONArray().apply {
            put(
                JSONObject()
                    .put("type", "blockStart")
                    .put("nodeType", "paragraph")
                    .put("depth", 0)
            )
            put(
                JSONObject()
                    .put("type", "textRun")
                    .put("text", text)
                    .put("marks", JSONArray())
            )
            put(JSONObject().put("type", "blockEnd"))
        }
    }

    protected enum class ListRenderState { INITIAL, NESTED_EMPTY, PARENT_EMPTY }

    protected fun nestedListRenderBlock(state: ListRenderState): JSONArray = JSONArray().apply {
        fun blockStart(nodeType: String, depth: Int): JSONObject = JSONObject()
            .put("type", "blockStart")
            .put("nodeType", nodeType)
            .put("depth", depth)

        fun textRun(text: String): JSONObject = JSONObject()
            .put("type", "textRun")
            .put("text", text)
            .put("marks", JSONArray())

        fun listItemStart(depth: Int, index: Int, total: Int): JSONObject =
            blockStart("listItem", depth).put(
                "listContext",
                JSONObject()
                    .put("ordered", false)
                    .put("index", index)
                    .put("total", total)
                    .put("start", 1)
                    .put("isFirst", index == 1)
                    .put("isLast", index == total)
            )

        fun paragraph(depth: Int, text: String) {
            put(blockStart("paragraph", depth))
            put(textRun(text))
            put(JSONObject().put("type", "blockEnd"))
        }

        val rootTotal = if (state == ListRenderState.PARENT_EMPTY) 3 else 2
        put(listItemStart(depth = 0, index = 1, total = rootTotal))
        paragraph(depth = 1, text = "First")
        put(JSONObject().put("type", "blockEnd"))

        put(listItemStart(depth = 0, index = 2, total = rootTotal))
        paragraph(depth = 1, text = "Second")
        val nestedTotal = if (state == ListRenderState.NESTED_EMPTY) 2 else 1
        put(listItemStart(depth = 1, index = 1, total = nestedTotal))
        paragraph(depth = 2, text = "Nested")
        put(JSONObject().put("type", "blockEnd"))
        if (state == ListRenderState.NESTED_EMPTY) {
            put(listItemStart(depth = 1, index = 2, total = nestedTotal))
            paragraph(depth = 2, text = "\u200B")
            put(JSONObject().put("type", "blockEnd"))
        }
        put(JSONObject().put("type", "blockEnd"))

        if (state == ListRenderState.PARENT_EMPTY) {
            put(listItemStart(depth = 0, index = 3, total = rootTotal))
            paragraph(depth = 1, text = "\u200B")
            put(JSONObject().put("type", "blockEnd"))
        }
    }

    protected fun blockquoteRenderBlock(text: String): JSONArray = JSONArray().apply {
        put(JSONObject().put("type", "blockStart").put("nodeType", "blockquote").put("depth", 0))
        put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph").put("depth", 1))
        put(JSONObject().put("type", "textRun").put("text", text).put("marks", JSONArray()))
        put(JSONObject().put("type", "blockEnd"))
        put(JSONObject().put("type", "blockEnd"))
    }

    protected fun renderUpdateJson(
        blocks: JSONArray,
        includeFullRenderBlocks: Boolean = true,
        renderPatch: JSONObject? = null
    ): String {
        return JSONObject().apply {
            if (includeFullRenderBlocks) {
                put("renderBlocks", blocks)
            }
            if (renderPatch != null) {
                put("renderPatch", renderPatch)
            }
        }.toString()
    }

    /**
     * An empty bullet is content the user can see, so the placeholder must go.
     *
     * The document renders no characters at all — the bullet marker comes from
     * block structure, never from stored text — so the view cannot work this
     * out by scanning its own content. It has to take the core's
     * `documentIsEmpty` from the update, which is what this drives.
     */

    /**
     * The companion: a document the core reports as empty keeps its
     * placeholder, so the fix cannot be "never show the placeholder".
     */

    protected companion object {
        const val MIN_HEIGHT_PX = 900
    }
}
