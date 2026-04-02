package com.apollohg.editor

import android.graphics.Color
import android.widget.LinearLayout
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
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

    private fun emptyParagraphRenderJson(): String = """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"\u200B","marks":[]},
          {"type":"blockEnd"}
        ]
    """.trimIndent()

    @Test
    fun `placeholder shows for rendered empty paragraph`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.placeholderText = "Type here"
        editText.applyRenderJSON(emptyParagraphRenderJson())

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
                    clientId = 7,
                    anchor = 6,
                    head = 6,
                    color = Color.parseColor("#ff6b35"),
                    name = "Alice",
                    isFocused = true,
                )
            )
        )

        val snapshot = view.remoteSelectionDebugSnapshotsForTesting().single()
        assertEquals(7, snapshot.clientId)
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
                    clientId = 8,
                    anchor = 6,
                    head = 6,
                    color = Color.parseColor("#007aff"),
                    name = "Alice",
                    isFocused = false,
                )
            )
        )

        val snapshot = view.remoteSelectionDebugSnapshotsForTesting().single()
        assertEquals(8, snapshot.clientId)
        assertTrue(snapshot.caretRect == null)
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
