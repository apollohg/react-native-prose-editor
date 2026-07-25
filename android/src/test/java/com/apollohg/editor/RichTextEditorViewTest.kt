package com.apollohg.editor

import android.graphics.Color
import android.graphics.Bitmap
import android.graphics.Canvas
import android.text.SpannableStringBuilder
import android.text.StaticLayout
import android.text.TextPaint
import android.text.Spanned
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
    fun `tapping below rendered task marker does not toggle nearest task item`() {
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
    fun `painted caret rect is clipped to glyph height on a spacer line`() {
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
        // The exact rectangle drawCustomCaret paints (content coordinates, caret width).
        val caret = editText.customCaretDrawRect()

        assertNotNull("a caret rect should be produced for a collapsed selection", caret)
        assertTrue("painted caret should have width", caret!!.width() > 0f)
        assertTrue("painted caret should have height", caret.height() > 0f)
        assertTrue(
            "painted caret height ${caret.height()} must exclude the 60px gap (inflated=$inflatedLineHeight)",
            caret.height() < inflatedLineHeight - 20f
        )
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
}
