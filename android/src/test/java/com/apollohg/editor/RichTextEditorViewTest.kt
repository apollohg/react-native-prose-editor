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

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class RichTextEditorViewTest {
    private class InterceptAwareFrameLayout(context: android.content.Context) : FrameLayout(context) {
        var disallowInterceptRequested = false

        override fun requestDisallowInterceptTouchEvent(disallowIntercept: Boolean) {
            disallowInterceptRequested = disallowIntercept
            super.requestDisallowInterceptTouchEvent(disallowIntercept)
        }
    }

    private class CaretVisibilityParent(context: android.content.Context) : FrameLayout(context) {
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

    private data class CaretVisibilityFixture(
        val parent: CaretVisibilityParent,
        val editText: EditorEditText
    )

    private data class ImageResizeGestureFixture(
        val parent: InterceptAwareFrameLayout,
        val view: RichTextEditorView,
        val resizeCommands: MutableList<Triple<Int, Int, Int>>,
    )

    private fun autoGrowCaretVisibilityFixture(
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

    private fun exampleTheme(markerScale: Float = 2f): EditorTheme? =
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

    private fun exampleRenderJson(): String = """
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

    private fun singleBulletListRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"listItem","depth":0,"listContext":{"ordered":false,"index":1,"total":1,"start":1,"isFirst":true,"isLast":true}},
          {"type":"blockStart","nodeType":"paragraph","depth":1},
          {"type":"textRun","text":"Bullet item","marks":[]},
          {"type":"blockEnd"},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    private fun singleTaskListRenderJson(): String = """
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
    private fun taskListWithOverflowRenderJson(fillerLineCount: Int = 30): String {
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

    private fun plainParagraphStartingWithCheckboxGlyphRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"☐ not a task","marks":[]},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    private fun emptyParagraphRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"\u200B","marks":[]},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    private fun imageResizeGestureFixture(renderJson: String): ImageResizeGestureFixture {
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

    private fun imageRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Hello","marks":[]},
          {"type":"blockEnd"},
          {"type":"voidBlock","nodeType":"image","docPos":7,"attrs":{"src":"https://example.com/cat.png","width":140,"height":80}},
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    private fun twoImageRenderJson(): String = """
        [
          {"type":"voidBlock","nodeType":"image","docPos":1,"attrs":{"src":"https://example.com/first.png","width":120,"height":60}},
          {"type":"voidBlock","nodeType":"image","docPos":2,"attrs":{"src":"https://example.com/second.png","width":120,"height":60}}
        ]
    """.trimIndent()

    private fun paragraphRenderBlock(text: String): JSONArray {
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

    private enum class ListRenderState { INITIAL, NESTED_EMPTY, PARENT_EMPTY }

    private fun nestedListRenderBlock(state: ListRenderState): JSONArray = JSONArray().apply {
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

    private fun blockquoteRenderBlock(text: String): JSONArray = JSONArray().apply {
        put(JSONObject().put("type", "blockStart").put("nodeType", "blockquote").put("depth", 0))
        put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph").put("depth", 1))
        put(JSONObject().put("type", "textRun").put("text", text).put("marks", JSONArray()))
        put(JSONObject().put("type", "blockEnd"))
        put(JSONObject().put("type", "blockEnd"))
    }

    private fun renderUpdateJson(
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

    @Test
    fun `placeholder shows for rendered empty paragraph`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.placeholderText = "Type here"
        editText.applyRenderJSON(emptyParagraphRenderJson())

        assertTrue(editText.shouldDisplayPlaceholderForTesting())
    }

    @Test
    fun `apply update json resolves patch-only payload for middle paragraph split`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
            put(paragraphRenderBlock("Gamma"))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        val patchedBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
            put(paragraphRenderBlock("\u200B"))
            put(paragraphRenderBlock("Gamma"))
        }
        val renderPatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 2)
            .put(
                "renderBlocks",
                JSONArray().apply {
                    put(paragraphRenderBlock("Beta"))
                    put(paragraphRenderBlock("\u200B"))
                    put(paragraphRenderBlock("Gamma"))
                }
            )

        editText.applyUpdateJSON(
            renderUpdateJson(
                patchedBlocks,
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertEquals("Alpha\nBeta\n\u200B\nGamma", editText.text?.toString())
        assertFalse(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `count changing patch falls back before later indexed patch`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        val insertPatch = JSONObject()
            .put("startIndex", 0)
            .put("deleteCount", 0)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Extra")))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = insertPatch
            ),
            notifyListener = false
        )

        assertEquals("Extra\nAlpha\nBeta", editText.text?.toString())
        assertFalse(editText.lastRenderAppliedPatch())

        val replacePatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Updated")))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = replacePatch
            ),
            notifyListener = false
        )

        assertEquals("Extra\nUpdated\nBeta", editText.text?.toString())
        assertTrue(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `registered atom types do not disable paragraph patching`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        assertTrue(editText.applyAtomRenderConfiguration(AtomRenderConfiguration.fromJson(
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        )))
        editText.applyUpdateJSON(
            renderUpdateJson(JSONArray().put(paragraphRenderBlock("Before"))),
            notifyListener = false
        )
        val renderPatch = JSONObject()
            .put("startIndex", 0)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("After")))

        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertEquals("After", editText.text.toString())
        assertTrue(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `atom patch without stable ids falls back to a full render`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        assertTrue(editText.applyAtomRenderConfiguration(AtomRenderConfiguration.fromJson(
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        )))
        fun atomBlock(docPos: Int): JSONArray = JSONArray().put(
            JSONObject()
                .put("type", "voidBlock")
                .put("nodeType", "counterCard")
                .put("docPos", docPos)
        )
        val initialBlocks = JSONArray()
            .put(atomBlock(1))
            .put(atomBlock(2))
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        val renderPatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(atomBlock(3)))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        val content = editText.text as Spanned
        val atomKeys = content.getSpans(0, content.length, AtomBlockSpan::class.java)
            .map { it.atomKey }
        assertEquals(listOf("counterCard:0", "counterCard:1"), atomKeys)
        assertFalse(editText.lastRenderAppliedPatch())
    }

    @Test
    fun `nested list return patches preserve list and blockquote layout`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(blockquoteRenderBlock("Quoted"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
            put(paragraphRenderBlock("After"))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun leftOf(text: String): Float {
            val content = editText.text as Spanned
            val offset = content.toString().indexOf(text)
            val layout = StaticLayout.Builder
                .obtain(content, 0, content.length, TextPaint().apply { textSize = 16f }, 800)
                .setIncludePad(false)
                .build()
            return layout.getPrimaryHorizontal(offset)
        }

        fun leadingMarginCount(text: String): Int {
            val content = editText.text as Spanned
            val offset = content.toString().indexOf(text)
            return content.getSpans(offset, offset + 1, LeadingMarginSpan::class.java).size
        }

        fun quoteSpanEnd(): Int {
            val content = editText.text as Spanned
            return content.getSpanEnd(content.getSpans(0, content.length, BlockquoteSpan::class.java).single())
        }

        fun applyListPatch(state: ListRenderState) {
            val renderPatch = JSONObject()
                .put("startIndex", 1)
                .put("deleteCount", 1)
                .put("renderBlocks", JSONArray().put(nestedListRenderBlock(state)))
            editText.applyUpdateJSON(
                renderUpdateJson(
                    JSONArray(),
                    includeFullRenderBlocks = false,
                    renderPatch = renderPatch
                ),
                notifyListener = false
            )
            assertTrue(editText.lastRenderAppliedPatch())
        }

        val expectedText = "Quoted\n• First\n• Second\n• Nested\n• \u200B\nAfter"
        val initialQuoteLeft = leftOf("Quoted")
        val initialRootLeft = leftOf("First")
        val initialSecondLeft = leftOf("Second")
        val initialNestedLeft = leftOf("Nested")
        val initialAfterLeft = leftOf("After")
        val initialRootMargins = leadingMarginCount("First")
        val initialNestedMargins = leadingMarginCount("Nested")

        applyListPatch(ListRenderState.NESTED_EMPTY)

        assertEquals(expectedText, editText.text.toString())
        assertEquals(initialQuoteLeft, leftOf("Quoted"), 0.01f)
        assertEquals(initialRootLeft, leftOf("First"), 0.01f)
        assertEquals(initialSecondLeft, leftOf("Second"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("Nested"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("\u200B"), 0.01f)
        assertEquals(initialAfterLeft, leftOf("After"), 0.01f)
        assertEquals(initialRootMargins, leadingMarginCount("First"))
        assertEquals(initialNestedMargins, leadingMarginCount("Nested"))
        assertEquals(editText.text.toString().indexOf("• First"), quoteSpanEnd())

        applyListPatch(ListRenderState.PARENT_EMPTY)

        assertEquals(expectedText, editText.text.toString())
        assertEquals(initialQuoteLeft, leftOf("Quoted"), 0.01f)
        assertEquals(initialRootLeft, leftOf("First"), 0.01f)
        assertEquals(initialSecondLeft, leftOf("Second"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("Nested"), 0.01f)
        assertEquals(initialRootLeft, leftOf("\u200B"), 0.01f)
        assertEquals(initialAfterLeft, leftOf("After"), 0.01f)
        assertEquals(initialRootMargins, leadingMarginCount("First"))
        assertEquals(initialNestedMargins, leadingMarginCount("Nested"))
        assertEquals(editText.text.toString().indexOf("• First"), quoteSpanEnd())
    }

    @Test
    fun `paragraph patch preserves following blockquote and list paragraph spans`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Before"))
            put(paragraphRenderBlock("Editable paragraph"))
            put(blockquoteRenderBlock("Quoted"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun paragraphSpanCounts(): Pair<Int, Int> {
            val content = editText.text as Spanned
            val quoteOffset = content.toString().indexOf("Quoted")
            val listOffset = content.toString().indexOf("First")
            return content.getSpans(
                quoteOffset,
                quoteOffset + 1,
                BlockquoteSpan::class.java
            ).size to content.getSpans(
                listOffset,
                listOffset + 1,
                LeadingMarginSpan::class.java
            ).size
        }

        val initialSpanCounts = paragraphSpanCounts()
        val renderPatch = JSONObject()
            .put("startIndex", 1)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(paragraphRenderBlock("Edited")))

        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertTrue(editText.lastRenderAppliedPatch())
        assertEquals("Before\nEdited\nQuoted\n• First\n• Second\n• Nested", editText.text.toString())
        assertEquals(initialSpanCounts, paragraphSpanCounts())
    }

    @Test
    fun `blockquote patch preserves following list paragraph spans`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(blockquoteRenderBlock("Quoted content"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun listParagraphSpanCounts(): Pair<Int, Int> {
            val content = editText.text as Spanned
            val rootOffset = content.toString().indexOf("First")
            val nestedOffset = content.toString().indexOf("Nested")
            return content.getSpans(
                rootOffset,
                rootOffset + 1,
                LeadingMarginSpan::class.java
            ).size to content.getSpans(
                nestedOffset,
                nestedOffset + 1,
                LeadingMarginSpan::class.java
            ).size
        }

        val initialSpanCounts = listParagraphSpanCounts()
        val renderPatch = JSONObject()
            .put("startIndex", 0)
            .put("deleteCount", 1)
            .put("renderBlocks", JSONArray().put(blockquoteRenderBlock("Quote")))

        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = renderPatch
            ),
            notifyListener = false
        )

        assertTrue(editText.lastRenderAppliedPatch())
        assertEquals("Quote\n• First\n• Second\n• Nested", editText.text.toString())
        assertEquals(initialSpanCounts, listParagraphSpanCounts())
    }

    @Test
    fun `terminal list patch preserves preceding blockquote span`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val initialBlocks = JSONArray().apply {
            put(blockquoteRenderBlock("Quoted"))
            put(nestedListRenderBlock(ListRenderState.INITIAL))
        }
        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)

        fun leftOf(text: String): Float {
            val content = editText.text as Spanned
            val offset = content.toString().indexOf(text)
            val layout = StaticLayout.Builder
                .obtain(content, 0, content.length, TextPaint().apply { textSize = 16f }, 800)
                .setIncludePad(false)
                .build()
            return layout.getPrimaryHorizontal(offset)
        }

        val initialRootLeft = leftOf("First")
        val initialNestedLeft = leftOf("Nested")

        fun applyListPatch(state: ListRenderState) {
            val renderPatch = JSONObject()
                .put("startIndex", 1)
                .put("deleteCount", 1)
                .put("renderBlocks", JSONArray().put(nestedListRenderBlock(state)))
            editText.applyUpdateJSON(
                renderUpdateJson(
                    JSONArray(),
                    includeFullRenderBlocks = false,
                    renderPatch = renderPatch
                ),
                notifyListener = false
            )
        }

        applyListPatch(ListRenderState.NESTED_EMPTY)
        applyListPatch(ListRenderState.PARENT_EMPTY)

        val content = editText.text as Spanned
        assertEquals(1, content.getSpans(0, content.length, BlockquoteSpan::class.java).size)
        assertEquals(initialRootLeft, leftOf("First"), 0.01f)
        assertEquals(initialNestedLeft, leftOf("Nested"), 0.01f)
    }

    @Test
    fun `apply update json skips render work when render blocks are unchanged`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            captureApplyUpdateTraceForTesting = true
        }
        val initialBlocks = JSONArray().apply {
            put(paragraphRenderBlock("Alpha"))
            put(paragraphRenderBlock("Beta"))
        }

        editText.applyUpdateJSON(renderUpdateJson(initialBlocks), notifyListener = false)
        editText.lastApplyUpdateTrace()

        val selectionOnlyUpdate = JSONObject()
            .put("renderBlocks", JSONArray(initialBlocks.toString()))
            .toString()

        editText.applyUpdateJSON(selectionOnlyUpdate, notifyListener = false)

        val trace = editText.lastApplyUpdateTrace()
        assertNotNull(trace)
        assertTrue(trace?.skippedRender == true)
        assertFalse(trace?.usedPatch == true)
        assertEquals(0L, trace?.buildRenderNanos)
        assertEquals(0L, trace?.applyRenderNanos)
        assertEquals("Alpha\nBeta", editText.text?.toString())
    }

    @Test
    fun `appearance change forces full render for an empty render patch`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val blocks = JSONArray().put(paragraphRenderBlock("Alpha"))
        editText.applyTheme(EditorTheme.fromJson("""{"text":{"color":"#112233"}}"""))
        editText.applyUpdateJSON(renderUpdateJson(blocks), notifyListener = false)

        editText.applyTheme(EditorTheme.fromJson("""{"text":{"color":"#DDEEFF"}}"""))
        editText.applyUpdateJSON(
            renderUpdateJson(
                JSONArray(),
                includeFullRenderBlocks = false,
                renderPatch = JSONObject()
                    .put("startIndex", 0)
                    .put("deleteCount", 0)
                    .put("renderBlocks", JSONArray())
            ),
            notifyListener = false
        )

        val color = editText.text
            ?.getSpans(0, 1, ForegroundColorSpan::class.java)
            ?.firstOrNull()
            ?.foregroundColor
        assertEquals(Color.parseColor("#DDEEFF"), color)
        assertFalse(editText.lastRenderAppliedPatch())
    }

    /**
     * An empty bullet is content the user can see, so the placeholder must go.
     *
     * The document renders no characters at all — the bullet marker comes from
     * block structure, never from stored text — so the view cannot work this
     * out by scanning its own content. It has to take the core's
     * `documentIsEmpty` from the update, which is what this drives.
     */
    @Test
    fun `placeholder hides when the core reports an empty list item as content`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.placeholderText = "Type here"
        editText.applyUpdateJSON(
            JSONObject().apply {
                put("renderBlocks", JSONArray())
                put("documentIsEmpty", false)
            }.toString()
        )

        assertFalse(editText.shouldDisplayPlaceholderForTesting())
    }

    /**
     * The companion: a document the core reports as empty keeps its
     * placeholder, so the fix cannot be "never show the placeholder".
     */
    @Test
    fun `placeholder shows when the core reports an empty document`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.placeholderText = "Type here"
        editText.applyUpdateJSON(
            JSONObject().apply {
                put("renderBlocks", JSONArray())
                put("documentIsEmpty", true)
            }.toString()
        )

        assertTrue(editText.shouldDisplayPlaceholderForTesting())
    }

    @Test
    fun `placeholder hides for rendered non-empty paragraph`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.placeholderText = "Type here"
        editText.setText("Hello")

        assertTrue(!editText.shouldDisplayPlaceholderForTesting())
    }

    @Test
    fun `multiline placeholder expands empty editor height`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        editText.placeholderText =
            "Type a much longer placeholder that should wrap onto multiple lines in the empty editor"
        editText.applyRenderJSON(emptyParagraphRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(220, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val availableWidth =
            editText.measuredWidth - editText.compoundPaddingLeft - editText.compoundPaddingRight
        val expectedPlaceholderHeight =
            StaticLayout.Builder
                .obtain(
                    editText.placeholderText,
                    0,
                    editText.placeholderText.length,
                    editText.paint,
                    availableWidth
                )
                .setAlignment(android.text.Layout.Alignment.ALIGN_NORMAL)
                .setIncludePad(editText.includeFontPadding)
                .build()
                .height +
                editText.compoundPaddingTop +
                editText.compoundPaddingBottom

        assertTrue(editText.measuredHeight >= expectedPlaceholderHeight)
        assertTrue(editText.resolveAutoGrowHeight() >= expectedPlaceholderHeight)
    }

    @Test
    fun `placeholder uses paragraph font size from theme`() {
        val context = RuntimeEnvironment.getApplication()
        val density = context.resources.displayMetrics.density
        val editText = EditorEditText(context)
        editText.setBaseStyle(24f * density, Color.BLACK, Color.WHITE)
        editText.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        editText.placeholderText = "Placeholder wraps"
        editText.applyTheme(
            EditorTheme.fromJson(
                """
                {
                  "text": { "fontSize": 12 },
                  "paragraph": { "fontSize": 10 }
                }
                """.trimIndent()
            )
        )
        editText.applyRenderJSON(emptyParagraphRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(220, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val availableWidth =
            editText.measuredWidth - editText.compoundPaddingLeft - editText.compoundPaddingRight
        val expectedPlaceholderHeight =
            StaticLayout.Builder
                .obtain(
                    editText.placeholderText,
                    0,
                    editText.placeholderText.length,
                    TextPaint(editText.paint).apply {
                        textSize = 10f * density
                    },
                    availableWidth
                )
                .setAlignment(android.text.Layout.Alignment.ALIGN_NORMAL)
                .setIncludePad(editText.includeFontPadding)
                .build()
                .height +
                editText.compoundPaddingTop +
                editText.compoundPaddingBottom

        assertEquals(expectedPlaceholderHeight, editText.resolveAutoGrowHeight())
    }

    @Test
    fun `editor disables clickable links`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        assertTrue(!editText.linksClickable)
    }

    @Test
    fun `editor auto grow height resolves from text layout`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.setText("Line one\nLine two\nLine three")

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val expectedHeight =
            (editText.layout?.height ?: 0) + editText.compoundPaddingTop + editText.compoundPaddingBottom

        assertTrue(expectedHeight > 0)
        assertEquals(expectedHeight, editText.resolveAutoGrowHeight())
    }

    @Test
    fun `rich text editor auto grow measures to content height within parent limit`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        richTextEditorView.editorEditText.setText("Short content")

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(1600, View.MeasureSpec.AT_MOST)
        richTextEditorView.measure(widthSpec, heightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )

        val contentHeight = richTextEditorView.editorEditText.resolveAutoGrowHeight()

        assertTrue(contentHeight > 0)
        assertEquals(contentHeight, richTextEditorView.measuredHeight)
    }

    @Test
    fun `rich text editor auto grow ignores oversized exact parent height`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        richTextEditorView.editorEditText.setText("Short content")

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        richTextEditorView.measure(widthSpec, wrapHeightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )
        val expectedContentHeight = richTextEditorView.editorEditText.resolveAutoGrowHeight()

        val oversizedExactHeightSpec = View.MeasureSpec.makeMeasureSpec(1600, View.MeasureSpec.EXACTLY)
        richTextEditorView.measure(widthSpec, oversizedExactHeightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )

        assertEquals(expectedContentHeight, richTextEditorView.measuredHeight)
    }

    @Test
    fun `editor auto grow height ignores stale exact measured height before layout`() {
        val context = RuntimeEnvironment.getApplication()
        val expectedView = EditorEditText(context)
        expectedView.setText("Short content")

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        expectedView.measure(widthSpec, wrapHeightSpec)
        expectedView.layout(0, 0, expectedView.measuredWidth, expectedView.measuredHeight)
        val expectedHeight = expectedView.resolveAutoGrowHeight()

        val subject = EditorEditText(context)
        subject.setText("Short content")
        val fixedHeightSpec = View.MeasureSpec.makeMeasureSpec(1200, View.MeasureSpec.EXACTLY)
        subject.measure(widthSpec, fixedHeightSpec)

        assertEquals(1200, subject.measuredHeight)
        val resolvedHeight = subject.resolveAutoGrowHeight()
        assertEquals(
                "expected=$expectedHeight resolved=$resolvedHeight " +
                "isLaidOut=${subject.isLaidOut} measuredWidth=${subject.measuredWidth} " +
                "layoutHeight=${subject.layout?.height} lineHeight=${subject.lineHeight} " +
                "compoundPaddingTop=${subject.compoundPaddingTop} compoundPaddingBottom=${subject.compoundPaddingBottom}",
            expectedHeight,
            resolvedHeight
        )
    }

    @Test
    fun `editor auto grow height ignores stale exact height after layout`() {
        val context = RuntimeEnvironment.getApplication()
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

        val expectedView = EditorEditText(context)
        expectedView.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        expectedView.setText("Short content")
        expectedView.measure(widthSpec, wrapHeightSpec)
        expectedView.layout(0, 0, expectedView.measuredWidth, expectedView.measuredHeight)
        val expectedHeight = expectedView.resolveAutoGrowHeight()

        val subject = EditorEditText(context)
        subject.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        subject.setText("Short content")
        val staleHeight = expectedHeight + 320
        val exactHeightSpec = View.MeasureSpec.makeMeasureSpec(staleHeight, View.MeasureSpec.EXACTLY)
        subject.measure(widthSpec, exactHeightSpec)
        subject.layout(0, 0, subject.measuredWidth, subject.measuredHeight)

        assertEquals(staleHeight, subject.height)
        val resolvedHeight = subject.resolveAutoGrowHeight()

        assertEquals(expectedHeight, resolvedHeight)
    }

    @Test
    fun `editor auto grow height expands after exact-height feedback loop`() {
        val context = RuntimeEnvironment.getApplication()
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        val shortText = "Short content"
        val tallText = "Line one\nLine two\nLine three\nLine four\nLine five"

        val expectedTallView = EditorEditText(context)
        expectedTallView.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        expectedTallView.setText(tallText)
        expectedTallView.measure(widthSpec, wrapHeightSpec)
        expectedTallView.layout(
            0,
            0,
            expectedTallView.measuredWidth,
            expectedTallView.measuredHeight
        )
        val expectedTallHeight = expectedTallView.resolveAutoGrowHeight()

        val subject = EditorEditText(context)
        subject.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        subject.setText(shortText)
        subject.measure(widthSpec, wrapHeightSpec)
        subject.layout(0, 0, subject.measuredWidth, subject.measuredHeight)
        val shortHeight = subject.resolveAutoGrowHeight()

        // Simulate React Native feeding the previous contentHeight back as an exact height.
        val exactShortHeightSpec = View.MeasureSpec.makeMeasureSpec(shortHeight, View.MeasureSpec.EXACTLY)
        subject.measure(widthSpec, exactShortHeightSpec)
        subject.layout(0, 0, subject.measuredWidth, subject.measuredHeight)

        subject.setText(tallText)
        val expandedHeight = subject.resolveAutoGrowHeight()

        assertTrue(expandedHeight > shortHeight)
        assertEquals(expectedTallHeight, expandedHeight)
    }

    @Test
    fun `editor auto grow height shrinks after exact-height feedback loop`() {
        val context = RuntimeEnvironment.getApplication()
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        val shortText = "Short content"
        val tallText = "Line one\nLine two\nLine three\nLine four\nLine five"

        val expectedShortView = EditorEditText(context)
        expectedShortView.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        expectedShortView.setText(shortText)
        expectedShortView.measure(widthSpec, wrapHeightSpec)
        expectedShortView.layout(
            0,
            0,
            expectedShortView.measuredWidth,
            expectedShortView.measuredHeight
        )
        val expectedShortHeight = expectedShortView.resolveAutoGrowHeight()

        val subject = EditorEditText(context)
        subject.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        subject.setText(tallText)
        subject.measure(widthSpec, wrapHeightSpec)
        subject.layout(0, 0, subject.measuredWidth, subject.measuredHeight)
        val tallHeight = subject.resolveAutoGrowHeight()

        // Simulate React Native feeding the previous contentHeight back as an exact height.
        val exactTallHeightSpec = View.MeasureSpec.makeMeasureSpec(tallHeight, View.MeasureSpec.EXACTLY)
        subject.measure(widthSpec, exactTallHeightSpec)
        subject.layout(0, 0, subject.measuredWidth, subject.measuredHeight)

        subject.setText(shortText)
        val shrunkHeight = subject.resolveAutoGrowHeight()

        assertTrue(shrunkHeight < tallHeight)
        assertEquals(expectedShortHeight, shrunkHeight)
    }

    @Test
    fun `rich text editor auto grow expands after content changes`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(1600, View.MeasureSpec.AT_MOST)

        richTextEditorView.editorEditText.setText("Short content")
        richTextEditorView.measure(widthSpec, heightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )
        val shortHeight = richTextEditorView.measuredHeight

        richTextEditorView.editorEditText.setText("Line one\nLine two\nLine three\nLine four")
        richTextEditorView.measure(widthSpec, heightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )
        val tallHeight = richTextEditorView.measuredHeight

        assertTrue("Auto-grow height should expand when content grows", tallHeight > shortHeight)
    }

    @Test
    fun `rich text editor auto grow keeps edit text height aligned with container`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        richTextEditorView.editorEditText.setText(
            "Line one\nLine two\nLine three\nLine four\nLine five"
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(1600, View.MeasureSpec.AT_MOST)
        richTextEditorView.measure(widthSpec, heightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )

        assertEquals(
            "EditText should fill the auto-grow container height",
            richTextEditorView.measuredHeight,
            richTextEditorView.editorEditText.measuredHeight
        )
    }

    @Test
    fun `rich text editor auto grow lays out edit text to container height`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        richTextEditorView.editorEditText.setText(
            "Line one\nLine two\nLine three\nLine four\nLine five"
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(1600, View.MeasureSpec.AT_MOST)
        richTextEditorView.measure(widthSpec, heightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )

        assertEquals(
            "EditText should be laid out to the container height in auto-grow mode",
            richTextEditorView.height,
            richTextEditorView.editorEditText.height
        )
    }

    @Test
    fun `fixed height editor disallows parent intercept while scrolling`() {
        val context = RuntimeEnvironment.getApplication()
        val parent = InterceptAwareFrameLayout(context)
        val richTextEditorView = RichTextEditorView(context)
        richTextEditorView.layoutParams = FrameLayout.LayoutParams(
            FrameLayout.LayoutParams.MATCH_PARENT,
            200
        )
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.FIXED)
        richTextEditorView.editorEditText.setText((1..40).joinToString("\n") { "Line $it" })
        parent.addView(richTextEditorView)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(200, View.MeasureSpec.EXACTLY)
        parent.measure(widthSpec, heightSpec)
        parent.layout(0, 0, parent.measuredWidth, parent.measuredHeight)

        assertTrue(
            "Expected fixed editor content to overflow vertically",
            richTextEditorView.editorScrollView.canScrollVertically(1)
        )

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 10f, 10f, 0)
        richTextEditorView.editorScrollView.onTouchEvent(down)
        down.recycle()

        assertTrue(
            "Fixed-height editor should disallow parent intercept while scrolling",
            parent.disallowInterceptRequested
        )

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, 10f, 40f, 0)
        richTextEditorView.editorScrollView.onTouchEvent(up)
        up.recycle()

        assertTrue(
            "Fixed-height editor should release parent intercept after the gesture ends",
            !parent.disallowInterceptRequested
        )
    }

    @Test
    fun `selected image shows resize overlay at rendered image bounds`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.editorEditText.applyRenderJSON(imageRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        val text = view.editorEditText.text as? Spanned
        assertNotNull("Expected rendered text with spans", text)
        text ?: return

        val imageSpan = text.getSpans(0, text.length, BlockImageSpan::class.java).firstOrNull()
        assertNotNull("Expected a rendered image span", imageSpan)
        imageSpan ?: return

        val spanStart = text.getSpanStart(imageSpan)
        val spanEnd = text.getSpanEnd(imageSpan)
        view.editorEditText.setSelection(spanStart, spanEnd)
        view.editorEditText.onSelectionOrContentMayChange?.invoke()

        val overlayRect = view.imageResizeOverlayRectForTesting()
        assertNotNull("Selecting an image should show the resize overlay", overlayRect)
        overlayRect ?: return
        assertEquals(140f, overlayRect.width(), 1f)
        assertEquals(80f, overlayRect.height(), 1f)
    }

    @Test
    fun `semantic overlay transitions cancel active image resize gestures`() {
        val transitions = listOf<Pair<String, (ImageResizeGestureFixture) -> Unit>>(
            "render refresh" to { fixture ->
                fixture.view.editorEditText.applyRenderJSON(imageRenderJson())
            },
            "image policy rebuild" to { fixture ->
                fixture.view.editorEditText.setImageLoadingPolicyJson("""{"readTimeoutMs":1234}""")
            },
            "editor rebind" to { fixture ->
                fixture.view.setEditorIdWhileDetached(2)
            },
            "overlay hide" to { fixture ->
                fixture.view.setImageResizingEnabled(false)
            },
            "image identity replacement" to { fixture ->
                val text = fixture.view.editorEditText.text as Spanned
                val spans = text.getSpans(0, text.length, BlockImageSpan::class.java)
                fixture.view.editorEditText.setSelection(
                    text.getSpanStart(spans[1]),
                    text.getSpanEnd(spans[1]),
                )
                fixture.view.editorEditText.onSelectionOrContentMayChange?.invoke()
            },
        )

        transitions.forEach { (name, transition) ->
            val fixture = imageResizeGestureFixture(
                if (name == "image identity replacement") twoImageRenderJson() else imageRenderJson(),
            )
            val rect = requireNotNull(fixture.view.imageResizeOverlayRectForTesting())
            val down = MotionEvent.obtain(
                0,
                0,
                MotionEvent.ACTION_DOWN,
                rect.right,
                rect.bottom,
                0,
            )
            assertTrue(name, fixture.view.dispatchImageResizeTouchForTesting(down))
            down.recycle()
            assertTrue(name, fixture.parent.disallowInterceptRequested)

            transition(fixture)

            assertFalse(name, fixture.parent.disallowInterceptRequested)
            val up = MotionEvent.obtain(
                0,
                16,
                MotionEvent.ACTION_UP,
                rect.right + 20f,
                rect.bottom + 20f,
                0,
            )
            fixture.view.dispatchImageResizeTouchForTesting(up)
            up.recycle()
            assertTrue(name, fixture.resizeCommands.isEmpty())
        }
    }

    @Test
    fun `tapping rendered image selects it for resize overlay`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.editorEditText.applyRenderJSON(imageRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        val text = view.editorEditText.text as? Spanned
        assertNotNull("Expected rendered text with spans", text)
        text ?: return

        val imageSpan = text.getSpans(0, text.length, BlockImageSpan::class.java).firstOrNull()
        assertNotNull("Expected a rendered image span", imageSpan)
        imageSpan ?: return

        val spanStart = text.getSpanStart(imageSpan)
        val spanEnd = text.getSpanEnd(imageSpan)
        val canvasBitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.editorEditText.draw(Canvas(canvasBitmap))

        val drawnRect = imageSpan.currentDrawRect()
        assertNotNull("Expected drawn image bounds", drawnRect)
        drawnRect ?: return
        val tapX = drawnRect.centerX()
        val tapY = drawnRect.centerY()

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        view.editorEditText.onTouchEvent(down)
        down.recycle()

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        view.editorEditText.onTouchEvent(up)
        up.recycle()

        assertEquals(spanStart, view.editorEditText.selectionStart)
        assertEquals(spanEnd, view.editorEditText.selectionEnd)

        val overlayRect = view.imageResizeOverlayRectForTesting()
        assertNotNull("Tapping an image should show the resize overlay", overlayRect)
        overlayRect ?: return
        assertEquals(140f, overlayRect.width(), 1f)
        assertEquals(80f, overlayRect.height(), 1f)
    }

    @Test
    fun `dragging between images does not select either image`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.editorEditText.editorId = 1
        view.editorEditText.applyRenderJSON(twoImageRenderJson())
        view.editorEditText.onSetSelectionScalarInRustForTesting = { _, _ -> }

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val spans = (view.editorEditText.text as Spanned)
            .getSpans(0, view.editorEditText.text!!.length, BlockImageSpan::class.java)
        assertEquals(2, spans.size)
        val canvasBitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.editorEditText.draw(Canvas(canvasBitmap))
        val first = requireNotNull(spans[0].currentDrawRect())
        val second = requireNotNull(spans[1].currentDrawRect())

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, first.centerX(), first.centerY(), 0)
        val move = MotionEvent.obtain(0, 8, MotionEvent.ACTION_MOVE, second.centerX(), second.centerY(), 0)
        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, second.centerX(), second.centerY(), 0)
        view.editorEditText.onTouchEvent(down)
        view.editorEditText.onTouchEvent(move)
        view.editorEditText.onTouchEvent(up)
        down.recycle()
        move.recycle()
        up.recycle()

        assertNull(view.imageResizeOverlayRectForTesting())
    }

    @Test
    fun `cancel or additional pointer aborts pending image selection`() {
        listOf(MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN).forEach { abortAction ->
            val context = RuntimeEnvironment.getApplication()
            val view = RichTextEditorView(context)
            view.editorEditText.applyRenderJSON(imageRenderJson())
            val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
            val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
            view.measure(widthSpec, heightSpec)
            view.layout(0, 0, view.measuredWidth, view.measuredHeight)
            val span = (view.editorEditText.text as Spanned)
                .getSpans(0, view.editorEditText.text!!.length, BlockImageSpan::class.java)
                .single()
            val canvasBitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            view.editorEditText.draw(Canvas(canvasBitmap))
            val rect = requireNotNull(span.currentDrawRect())
            val pointX = rect.centerX()
            val pointY = rect.centerY()

            listOf(MotionEvent.ACTION_DOWN, abortAction, MotionEvent.ACTION_UP).forEach { action ->
                val event = MotionEvent.obtain(0, 0, action, pointX, pointY, 0)
                view.editorEditText.onTouchEvent(event)
                event.recycle()
            }

            assertNull(view.imageResizeOverlayRectForTesting())
        }
    }

    @Test
    fun `disabling image resizing keeps image taps from showing resize overlay`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.editorEditText.applyRenderJSON(imageRenderJson())
        view.setImageResizingEnabled(false)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        val text = view.editorEditText.text as? Spanned
        assertNotNull("Expected rendered text with spans", text)
        text ?: return

        val imageSpan = text.getSpans(0, text.length, BlockImageSpan::class.java).firstOrNull()
        assertNotNull("Expected a rendered image span", imageSpan)
        imageSpan ?: return

        val canvasBitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.editorEditText.draw(Canvas(canvasBitmap))

        val drawnRect = imageSpan.currentDrawRect()
        assertNotNull("Expected drawn image bounds", drawnRect)
        drawnRect ?: return
        val tapX = drawnRect.centerX()
        val tapY = drawnRect.centerY()

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        view.editorEditText.onTouchEvent(down)
        down.recycle()

        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        view.editorEditText.onTouchEvent(up)
        up.recycle()

        assertNull("Tapping an image should not show the resize overlay when disabled", view.imageResizeOverlayRectForTesting())
    }

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

    @Test
    fun `editor theme contentInsets apply padding in density-scaled pixels`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        val density = context.resources.displayMetrics.density
        editText.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        val theme = EditorTheme.fromJson(
            """
            {
              "contentInsets": { "top": 8, "right": 10, "bottom": 12, "left": 14 }
            }
            """.trimIndent()
        )

        editText.applyTheme(theme)

        assertEquals((14f * density).toInt(), editText.paddingLeft)
        assertEquals((8f * density).toInt(), editText.paddingTop)
        assertEquals((10f * density).toInt(), editText.paddingRight)
        assertEquals((12f * density).toInt(), editText.paddingBottom)
    }

    @Test
    fun `editor theme borderRadius applies to scroll container in density-scaled pixels`() {
        val context = RuntimeEnvironment.getApplication()
        val richTextEditorView = RichTextEditorView(context)
        val density = context.resources.displayMetrics.density
        val theme = EditorTheme.fromJson(
            """
            {
              "backgroundColor": "#d7e4ff",
              "borderRadius": 18
            }
            """.trimIndent()
        )

        richTextEditorView.applyTheme(theme)

        assertEquals(18f * density, richTextEditorView.appliedCornerRadiusPx, 0.1f)
        assertTrue(richTextEditorView.editorViewport.clipToOutline)
    }

    @Test
    fun `editor theme transparent backgroundColor applies transparent viewport background`() {
        val context = RuntimeEnvironment.getApplication()
        val richTextEditorView = RichTextEditorView(context)
        richTextEditorView.configure(
            textSizePx = 16f * context.resources.displayMetrics.density,
            textColor = Color.BLACK,
            backgroundColor = Color.WHITE
        )

        val theme = EditorTheme.fromJson(
            """
            {
              "backgroundColor": "transparent"
            }
            """.trimIndent()
        )

        assertEquals(Color.TRANSPARENT, theme?.backgroundColor)

        richTextEditorView.applyTheme(theme)

        assertEquals(Color.TRANSPARENT, richTextEditorView.appliedBackgroundColorForTesting)
    }

    @Test
    fun `fixed height editor reserves viewport inset in effective bottom padding`() {
        val context = RuntimeEnvironment.getApplication()
        val richTextEditorView = RichTextEditorView(context)
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.FIXED)
        richTextEditorView.applyTheme(
            EditorTheme.fromJson(
                """
                {
                  "contentInsets": { "bottom": 12 }
                }
                """.trimIndent()
            )
        )

        richTextEditorView.setViewportBottomInsetPx(96)

        val density = context.resources.displayMetrics.density
        assertEquals((12f * density).toInt() + 96, richTextEditorView.editorScrollView.paddingBottom)
        assertEquals(0, richTextEditorView.editorEditText.paddingBottom)
    }

    @Test
    fun `fixed height editor scrolls vertical contentInsets away while preserving viewport inset`() {
        val context = RuntimeEnvironment.getApplication()
        val richTextEditorView = RichTextEditorView(context)
        val density = context.resources.displayMetrics.density
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.FIXED)
        richTextEditorView.applyTheme(
            EditorTheme.fromJson(
                """
                {
                  "contentInsets": { "top": 8, "bottom": 12 }
                }
                """.trimIndent()
            )
        )

        richTextEditorView.setViewportBottomInsetPx(96)

        assertTrue(!richTextEditorView.editorScrollView.clipToPadding)
        assertEquals((8f * density).toInt(), richTextEditorView.editorScrollView.paddingTop)
        assertEquals((12f * density).toInt() + 96, richTextEditorView.editorScrollView.paddingBottom)
        assertEquals(0, richTextEditorView.editorEditText.paddingTop)
        assertEquals(0, richTextEditorView.editorEditText.paddingBottom)
    }

    @Test
    fun `caret rect is reported in editor view coordinates`() {
        val context = RuntimeEnvironment.getApplication()
        val richTextEditorView = RichTextEditorView(context)
        richTextEditorView.editorEditText.setText("Hello world")

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        richTextEditorView.measure(widthSpec, heightSpec)
        richTextEditorView.layout(
            0,
            0,
            richTextEditorView.measuredWidth,
            richTextEditorView.measuredHeight
        )
        richTextEditorView.editorEditText.setSelection(5)

        val editTextRect = richTextEditorView.editorEditText.caretRect()
        val actual = richTextEditorView.caretRect()

        assertNotNull(editTextRect)
        assertNotNull(actual)
        assertEquals(
            richTextEditorView.editorViewport.left +
                richTextEditorView.editorScrollView.left +
                richTextEditorView.editorEditText.left +
                editTextRect!!.left,
            actual!!.left,
            0.1f
        )
        assertEquals(
            richTextEditorView.editorViewport.top +
                richTextEditorView.editorScrollView.top +
                richTextEditorView.editorEditText.top +
                editTextRect.top -
                richTextEditorView.editorScrollView.scrollY,
            actual.top,
            0.1f
        )
        assertTrue(actual.height() > 0f)
    }

    @Test
    fun `native cursor stays enabled for Android insertion controls`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())

        assertTrue(
            "Android disables its insertion handle and magnifier when cursor visibility is false",
            editText.isCursorVisible
        )
    }

    @Test
    fun `native cursor drawable is clipped to glyph height on a spacer line`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.layoutParams = ViewGroup.LayoutParams(600, 240)
        val spanned = SpannableStringBuilder("Hello\nWorld")
        spanned.setSpan(
            ParagraphSpacerSpan(spacingPx = 60, baseFontSize = 16, textColor = Color.BLACK),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        editText.setText(spanned)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        editText.setSelection(5) // collapsed caret on the spacer line

        val layout = editText.layout!!
        val inflatedLineHeight = (layout.getLineBottom(0) - layout.getLineTop(0)).toFloat()
        val caret = editText.nativeCursorDrawRect()

        assertNotNull("a caret rect should be produced for a collapsed selection", caret)
        assertTrue("painted caret should have width", caret!!.width() > 0f)
        assertTrue("painted caret should have height", caret.height() > 0f)
        assertTrue(
            "painted caret height ${caret.height()} must exclude the 60px gap (inflated=$inflatedLineHeight)",
            caret.height() < inflatedLineHeight - 20f
        )
    }

    @Test
    fun `native cursor drawable uses magnifier local bounds`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val content = (1..20).joinToString("\n") { "Line $it" }
        editText.layoutParams = ViewGroup.LayoutParams(600, 1200)
        editText.setText(content)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(1200, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        editText.setSelection(content.indexOf("Line 16"))

        val bitmap = Bitmap.createBitmap(100, 80, Bitmap.Config.ARGB_8888)
        val drawable = editText.textCursorDrawable!!
        drawable.setBounds(48, 0, 50, bitmap.height)
        drawable.draw(Canvas(bitmap))

        assertTrue(
            "Magnifier-local cursor should be drawn through the source height",
            Color.alpha(bitmap.getPixel(48, bitmap.height - 1)) > 0
        )
    }

    @Test
    fun `native cursor drawable excludes paragraph spacer in editor bounds`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.layoutParams = ViewGroup.LayoutParams(600, 240)
        val spanned = SpannableStringBuilder("Hello\nWorld")
        spanned.setSpan(
            ParagraphSpacerSpan(spacingPx = 60, baseFontSize = 16, textColor = Color.BLACK),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        editText.setText(spanned)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        editText.setSelection(5)

        val layout = editText.layout!!
        val line = layout.getLineForOffset(editText.selectionEnd)
        val caret = editText.nativeCursorDrawRect()!!
        val bitmap = Bitmap.createBitmap(100, layout.height, Bitmap.Config.ARGB_8888)
        val drawable = editText.textCursorDrawable!!
        drawable.setBounds(48, layout.getLineTop(line), 50, layout.getLineBottom(line, false))
        drawable.draw(Canvas(bitmap))

        assertTrue(Color.alpha(bitmap.getPixel(48, caret.centerY().toInt())) > 0)
        assertEquals(0, Color.alpha(bitmap.getPixel(48, caret.bottom.toInt() + 10)))
    }

    @Test
    fun `caret rect height excludes the paragraph spacer gap`() {
        val context = RuntimeEnvironment.getApplication()
        val editText = EditorEditText(context)
        editText.layoutParams = ViewGroup.LayoutParams(600, 240)
        val spanned = SpannableStringBuilder("Hello\nWorld")
        // Spacer on the inter-block newline inflates the descent of line 0.
        spanned.setSpan(
            ParagraphSpacerSpan(spacingPx = 60, baseFontSize = 16, textColor = Color.BLACK),
            5,
            6,
            Spanned.SPAN_EXCLUSIVE_EXCLUSIVE
        )
        editText.setText(spanned)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        editText.setSelection(5) // caret on the spacer line (line 0)

        val layout = editText.layout!!
        val line = 0
        val inflatedLineHeight = (layout.getLineBottom(line) - layout.getLineTop(line)).toFloat()
        val rect = editText.caretRect()!!

        assertTrue(
            "reproduction guard: spacer should inflate the line box",
            layout.getLineDescent(line) > editText.paint.fontMetrics.descent
        )
        assertTrue("caret height should be positive", rect.height() > 0f)
        assertTrue(
            "caret height ${rect.height()} must exclude the 60px paragraph gap (inflated line height=$inflatedLineHeight)",
            rect.height() < inflatedLineHeight - 20f
        )
    }

    @Test
    fun `remote selections expose focused caret geometry without a badge`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.setRemoteSelectionEditorIdForTesting(1L)
        view.editorEditText.setText("Hello world")
        view.setRemoteSelectionScalarResolverForTesting { _, docPos -> docPos }

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        view.setRemoteSelections(
            listOf(
                RemoteSelectionDecoration(
                    clientId = "7",
                    anchor = 6,
                    head = 6,
                    color = Color.parseColor("#ff6b35"),
                    name = "Alice",
                    isFocused = true,
                )
            )
        )

        val snapshot = view.remoteSelectionDebugSnapshotsForTesting().single()
        assertEquals("7", snapshot.clientId)
        assertNotNull(snapshot.caretRect)
        assertTrue(snapshot.caretRect!!.height() > 0f)
    }

    @Test
    fun `unfocused collapsed remote selection does not expose caret or badge geometry`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.setRemoteSelectionEditorIdForTesting(1L)
        view.editorEditText.setText("Hello world")
        view.setRemoteSelectionScalarResolverForTesting { _, docPos -> docPos }

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        view.setRemoteSelections(
            listOf(
                RemoteSelectionDecoration(
                    clientId = "8",
                    anchor = 6,
                    head = 6,
                    color = Color.parseColor("#007aff"),
                    name = "Alice",
                    isFocused = false,
                )
            )
        )

        val snapshot = view.remoteSelectionDebugSnapshotsForTesting().single()
        assertEquals("8", snapshot.clientId)
        assertTrue(snapshot.caretRect == null)
    }

    @Test
    fun `remote selection geometry is cached across redraws`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.setRemoteSelectionEditorIdForTesting(1L)
        view.editorEditText.setText("Hello world from remote selections")

        var resolverCalls = 0
        view.setRemoteSelectionScalarResolverForTesting { _, docPos ->
            resolverCalls += 1
            docPos
        }

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        view.setRemoteSelections(
            listOf(
                RemoteSelectionDecoration(
                    clientId = "11",
                    anchor = 6,
                    head = 12,
                    color = Color.parseColor("#ff9500"),
                    name = "Range",
                    isFocused = true,
                )
            )
        )

        val bitmap = Bitmap.createBitmap(600, 240, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap)
        resolverCalls = 0

        view.draw(canvas)
        view.draw(canvas)

        assertEquals(0, resolverCalls)
    }

    @Test
    fun `setting identical remote selections does not invalidate cached geometry`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.setRemoteSelectionEditorIdForTesting(1L)
        view.editorEditText.setText("Hello world from remote selections")

        var resolverCalls = 0
        view.setRemoteSelectionScalarResolverForTesting { _, docPos ->
            resolverCalls += 1
            docPos
        }

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        val initialSelections = listOf(
            RemoteSelectionDecoration(
                clientId = "12",
                anchor = 6,
                head = 12,
                color = Color.parseColor("#34c759"),
                name = "Range",
                isFocused = true,
            )
        )
        view.setRemoteSelections(initialSelections)
        view.remoteSelectionDebugSnapshotsForTesting()

        resolverCalls = 0
        val identicalSelections = listOf(
            RemoteSelectionDecoration(
                clientId = "12",
                anchor = 6,
                head = 12,
                color = Color.parseColor("#34c759"),
                name = "Range",
                isFocused = true,
            )
        )
        view.setRemoteSelections(identicalSelections)
        view.remoteSelectionDebugSnapshotsForTesting()

        assertEquals(0, resolverCalls)
    }

    @Test
    fun `remote selection json parsing tolerates invalid colors`() {
        val context = RuntimeEnvironment.getApplication()

        val selections = RemoteSelectionDecoration.fromJson(
            context,
            """
            [
              {
                "clientId": "19",
                "anchor": 2,
                "head": 2,
                "color": "not-a-color",
                "name": "Alice",
                "isFocused": true
              }
            ]
            """.trimIndent()
        )

        assertEquals(1, selections.size)
        assertEquals("19", selections.single().clientId)
    }

    @Test
    fun `unordered marker scale does not change list item height`() {
        val context = RuntimeEnvironment.getApplication()
        val renderJson = singleBulletListRenderJson()
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

        fun measureHeight(markerScale: Float): Int {
            val theme = EditorTheme.fromJson(
                """
                {
                  "text": { "fontSize": 17 },
                  "list": { "markerScale": $markerScale }
                }
                """.trimIndent()
            )
            val editText = EditorEditText(context)
            editText.setText(
                RenderBridge.buildSpannable(
                    renderJson,
                    17f,
                    Color.BLACK,
                    theme,
                    1f
                )
            )
            editText.measure(widthSpec, heightSpec)
            editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
            return editText.measuredHeight
        }

        val normalHeight = measureHeight(1f)
        val scaledHeight = measureHeight(2f)

        assertEquals(normalHeight, scaledHeight)
    }

    @Test
    fun `unordered marker scale does not change spacer heavy example height`() {
        val context = RuntimeEnvironment.getApplication()
        val renderJson = exampleRenderJson()
        val widthSpec = View.MeasureSpec.makeMeasureSpec(902, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

        fun measureHeight(markerScale: Float): Int {
            val theme = exampleTheme(markerScale)
            val editText = EditorEditText(context)
            editText.setBaseStyle(
                17f * 2.625f,
                Color.parseColor("#2a2118"),
                Color.parseColor("#f6f1e8")
            )
            editText.applyTheme(theme)
            editText.setText(
                RenderBridge.buildSpannable(
                    renderJson,
                    17f,
                    Color.parseColor("#2a2118"),
                    theme,
                    2.625f
                )
            )
            editText.measure(widthSpec, heightSpec)
            editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
            return editText.measuredHeight
        }

        val normalHeight = measureHeight(1f)
        val scaledHeight = measureHeight(2f)

        assertEquals(normalHeight, scaledHeight)
    }

    @Test
    fun `editor auto grow height recomputes from new text before relayout`() {
        val context = RuntimeEnvironment.getApplication()
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

        val subject = EditorEditText(context)
        subject.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        subject.setText("Short content")
        subject.measure(widthSpec, wrapHeightSpec)
        subject.layout(0, 0, subject.measuredWidth, subject.measuredHeight)
        val shortHeight = subject.resolveAutoGrowHeight()

        val tallText = "Line one\nLine two\nLine three\nLine four\nLine five"
        val expectedView = EditorEditText(context)
        expectedView.layoutParams = ViewGroup.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        expectedView.setText(tallText)
        expectedView.measure(widthSpec, wrapHeightSpec)
        expectedView.layout(0, 0, expectedView.measuredWidth, expectedView.measuredHeight)
        val expectedTallHeight = expectedView.resolveAutoGrowHeight()

        subject.setText(tallText)

        val resolvedBeforeRelayout = subject.resolveAutoGrowHeight()

        assertTrue(
            "Expected taller content height to exceed original height",
            expectedTallHeight > shortHeight
        )
        assertEquals(expectedTallHeight, resolvedBeforeRelayout)
    }

    @Test
    fun `rich text editor auto grow keeps measured spacer content height before layout`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        val spannable = RenderBridge.buildSpannable(
            exampleRenderJson(),
            17f,
            Color.BLACK,
            exampleTheme(),
            2.625f
        )
        richTextEditorView.editorEditText.setText(spannable)

        val widthSpec = View.MeasureSpec.makeMeasureSpec(902, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.EXACTLY)
        richTextEditorView.measure(widthSpec, heightSpec)

        assertTrue(
            "Spacer-heavy content should have a positive measured height",
            richTextEditorView.measuredHeight > 0
        )
        assertEquals(
            "Auto-grow container should track the measured child height before layout",
            richTextEditorView.editorEditText.measuredHeight,
            richTextEditorView.measuredHeight
        )
        assertTrue(
            "Pre-layout fallback height should not exceed the measured spacer layout height",
            richTextEditorView.editorEditText.resolveAutoGrowHeight() <= richTextEditorView.measuredHeight
        )
    }

    @Test
    fun `focused auto grow editor requests ancestor visibility when the caret moves`() {
        val (parent, editText) = autoGrowCaretVisibilityFixture()

        editText.setSelection(editText.text?.length ?: 0)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()

        assertEquals(1, parent.requestedRectangles.size)
        assertTrue(parent.requestedRectangles.single().height() > 0)
    }

    @Test
    fun `unfocused auto grow editor does not request ancestor visibility`() {
        val (parent, editText) = autoGrowCaretVisibilityFixture(editorFocused = false)

        editText.setSelection(editText.text?.length ?: 0)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()

        assertTrue(parent.requestedRectangles.isEmpty())
    }

    @Test
    fun `auto grow caret visibility requests coalesce to the latest selection`() {
        val (parent, editText) = autoGrowCaretVisibilityFixture()

        editText.setSelection(5)
        editText.setSelection(editText.text?.length ?: 0)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()

        assertEquals(1, parent.requestedRectangles.size)
    }

    @Test
    fun `auto grow caret visibility clears the keyboard toolbar`() {
        val toolbarClearance = 120
        val (parent, editText) = autoGrowCaretVisibilityFixture(
            bottomClearance = toolbarClearance
        )

        editText.setSelection(editText.text?.length ?: 0)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()

        val requestedRectangle = parent.requestedRectangles.single()
        assertTrue(requestedRectangle.height() >= editText.lineHeight + toolbarClearance)
    }

    @Test
    fun `caret visibility recalculates ancestor occlusion for each movement`() {
        val fallbackClearance = 120
        val occlusionTop = 500
        val (parent, editText) = autoGrowCaretVisibilityFixture(
            bottomClearance = fallbackClearance
        )
        parent.verticallyScrollable = true
        editText.setViewportBottomOcclusionTopOnScreenPx(occlusionTop)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        parent.requestedRectangles.clear()
        parent.layout(0, 0, parent.width, 800)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        parent.requestedRectangles.clear()

        editText.setSelection(editText.text?.length ?: 0)
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()

        val parentLocation = IntArray(2)
        parent.getLocationOnScreen(parentLocation)
        val ancestorOcclusion = parentLocation[1] + parent.height - occlusionTop
        val requestedRectangle = parent.requestedRectangles.single()
        val textLayout = checkNotNull(editText.layout)
        val line = textLayout.getLineForOffset(editText.selectionEnd)
        val caretLineHeight = textLayout.getLineBottom(line) - textLayout.getLineTop(line)
        assertEquals(caretLineHeight + ancestorOcclusion, requestedRectangle.height())
    }

    @Test
    fun `focused Rust applied caret movement requests auto grow ancestor visibility`() {
        val (parent, editText) = autoGrowCaretVisibilityFixture()

        editText.isApplyingRustState = true
        editText.setSelection(editText.text?.length ?: 0)
        editText.isApplyingRustState = false
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()

        assertEquals(1, parent.requestedRectangles.size)
    }

    @Test
    fun `example content layout does not end with multiple blank lines`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        val theme = exampleTheme()
        editText.setBaseStyle(17f * 2.625f, Color.parseColor("#2a2118"), Color.parseColor("#f6f1e8"))
        editText.applyTheme(theme)
        editText.setText(
            RenderBridge.buildSpannable(
                exampleRenderJson(),
                17f,
                Color.parseColor("#2a2118"),
                theme,
                2.625f
            )
        )

        val widthSpec = View.MeasureSpec.makeMeasureSpec(902, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        editText.measure(widthSpec, heightSpec)
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)

        val layout = editText.layout
        assertTrue("Expected layout for example content", layout != null)
        layout ?: return

        val text = editText.text?.toString().orEmpty()
        var trailingBlankLines = 0
        for (line in layout.lineCount - 1 downTo 0) {
            val start = layout.getLineStart(line)
            val end = layout.getLineEnd(line)
            val lineText = text.substring(start, end).replace("\n", "").trim()
            if (lineText.isEmpty()) {
                trailingBlankLines += 1
                continue
            }
            break
        }

        val spacerSpans = editText.text?.getSpans(0, text.length, ParagraphSpacerSpan::class.java) ?: emptyArray()
        assertTrue(
            "Trailing blank lines=$trailingBlankLines lineCount=${layout.lineCount} text='${text.replace("\n", "\\n")}' spacerCount=${spacerSpans.size} measuredHeight=${editText.measuredHeight}",
            trailingBlankLines <= 1
        )
    }

    @Test
    fun `short content in a tall box keeps the whole viewport tappable`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.editorEditText.setText("One line")

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val tallSpec = View.MeasureSpec.makeMeasureSpec(MIN_HEIGHT_PX, View.MeasureSpec.EXACTLY)
        richTextEditorView.measure(widthSpec, tallSpec)
        richTextEditorView.layout(0, 0, 600, MIN_HEIGHT_PX)

        val editText = richTextEditorView.editorEditText
        val scrollView = richTextEditorView.editorScrollView
        val usableHeight = scrollView.height - scrollView.paddingTop - scrollView.paddingBottom
        assertTrue(
            "the scroll viewport must be taller than one line for this fixture " +
                "(viewport=$usableHeight, field=${editText.height})",
            usableHeight > editText.lineHeight * 2,
        )
        assertEquals(
            "a tap below the last line must land on the field, not the scroll container",
            usableHeight,
            editText.height,
        )
    }

    @Test
    fun `content taller than the box still scrolls`() {
        val richTextEditorView = RichTextEditorView(RuntimeEnvironment.getApplication())
        richTextEditorView.editorEditText.setText((1..80).joinToString("\n") { "line $it" })

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val tallSpec = View.MeasureSpec.makeMeasureSpec(MIN_HEIGHT_PX, View.MeasureSpec.EXACTLY)
        richTextEditorView.measure(widthSpec, tallSpec)
        richTextEditorView.layout(0, 0, 600, MIN_HEIGHT_PX)

        val editText = richTextEditorView.editorEditText
        val scrollView = richTextEditorView.editorScrollView
        assertTrue(
            "long content must not be clamped to the viewport " +
                "(viewport=${scrollView.height}, field=${editText.height})",
            editText.height > scrollView.height,
        )
    }

    private companion object {
        const val MIN_HEIGHT_PX = 900
    }
}
