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
internal class RichTextEditorViewTest : RichTextEditorViewTestFixture() {
    @Test
    fun `placeholder shows for rendered empty paragraph`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.placeholderText = "Type here"
        editText.applyRenderJSON(emptyParagraphRenderJson())

        assertTrue(editText.shouldDisplayPlaceholderForTesting())
    }

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
}
