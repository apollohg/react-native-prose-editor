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
internal class RichTextEditorViewCaretGeometryTest : RichTextEditorViewTestFixture() {
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
}
