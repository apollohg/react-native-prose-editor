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
internal class RichTextEditorViewAutoGrowTest : RichTextEditorViewTestFixture() {
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
}
