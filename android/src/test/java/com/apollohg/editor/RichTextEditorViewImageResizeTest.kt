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
internal class RichTextEditorViewImageResizeTest : RichTextEditorViewTestFixture() {
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
    fun `image selection suppresses native text highlight until selection leaves`() {
        val activity = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java)
            .setup()
            .get()
        val view = RichTextEditorView(activity)
        activity.setContentView(view)
        view.editorEditText.applyRenderJSON(imageRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        assertTrue(view.editorEditText.requestFocus())

        val originalHighlightColor = view.editorEditText.highlightColor
        val text = view.editorEditText.text as Spanned
        val imageSpan = text.getSpans(0, text.length, BlockImageSpan::class.java).single()
        view.editorEditText.setSelection(text.getSpanStart(imageSpan), text.getSpanEnd(imageSpan))

        assertEquals(Color.TRANSPARENT, view.editorEditText.highlightColor)

        view.editorEditText.setSelection(0, 1)

        assertEquals(originalHighlightColor, view.editorEditText.highlightColor)
    }

    @Test
    fun `selected image overlay includes content inset while scrolled`() {
        val context = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(context)
        view.setHeightBehavior(EditorHeightBehavior.FIXED)
        view.applyTheme(
            EditorTheme.fromJson(
                """
                {
                  "contentInsets": { "top": 24 }
                }
                """.trimIndent()
            )
        )
        view.editorEditText.applyRenderJSON(imageRenderJson())

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(100, View.MeasureSpec.EXACTLY)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        view.editorScrollView.scrollTo(0, 12)

        val text = view.editorEditText.text as Spanned
        val imageSpan = text.getSpans(0, text.length, BlockImageSpan::class.java).single()
        view.editorEditText.setSelection(text.getSpanStart(imageSpan), text.getSpanEnd(imageSpan))
        view.editorEditText.onSelectionOrContentMayChange?.invoke()

        val imageRect = requireNotNull(view.editorEditText.selectedImageGeometry()).rect
        val editorOrigin = Rect()
        view.editorViewport.offsetDescendantRectToMyCoords(view.editorEditText, editorOrigin)
        val overlayRect = requireNotNull(view.imageResizeOverlayRectForTesting())

        assertTrue(view.editorScrollView.scrollY > 0)
        assertEquals(editorOrigin.left + imageRect.left, overlayRect.left, 0.1f)
        assertEquals(editorOrigin.top + imageRect.top, overlayRect.top, 0.1f)
        assertEquals(editorOrigin.right + imageRect.right, overlayRect.right, 0.1f)
        assertEquals(editorOrigin.bottom + imageRect.bottom, overlayRect.bottom, 0.1f)
    }

    @Test
    fun `loaded image reflows its line without overlapping preceding text`() {
        RenderImageLoader.resetForTesting()
        val decodeStarted = CountDownLatch(1)
        val releaseDecode = CountDownLatch(1)
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            decodeStarted.countDown()
            releaseDecode.await(2, TimeUnit.SECONDS)
            Bitmap.createBitmap(1200, 800, Bitmap.Config.ARGB_8888)
        }
        val activity = org.robolectric.Robolectric.buildActivity(android.app.Activity::class.java)
            .setup()
            .get()
        val parent = FrameLayout(activity)
        val view = RichTextEditorView(activity)
        parent.addView(view, FrameLayout.LayoutParams(600, 900))
        activity.setContentView(parent)
        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(900, View.MeasureSpec.EXACTLY)
        parent.measure(widthSpec, heightSpec)
        parent.layout(0, 0, 600, 900)

        try {
            view.editorEditText.applyRenderJSON(
                """
                [
                  {"type":"blockStart","nodeType":"paragraph","depth":0},
                  {"type":"textRun","text":"Before","marks":[]},
                  {"type":"blockEnd"},
                  {"type":"voidBlock","nodeType":"image","docPos":8,"attrs":{"src":"data:image/png;base64,AQ=="}},
                  {"type":"blockStart","nodeType":"paragraph","depth":0},
                  {"type":"textRun","text":"After","marks":[]},
                  {"type":"blockEnd"}
                ]
                """.trimIndent()
            )
            parent.measure(widthSpec, heightSpec)
            parent.layout(0, 0, 600, 900)
            assertTrue(decodeStarted.await(2, TimeUnit.SECONDS))

            releaseDecode.countDown()
            var attempts = 0
            while (
                view.editorEditText.activeImageLoadHandleCountForTesting() > 0 &&
                attempts < 50
            ) {
                org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
                Thread.sleep(10)
                attempts += 1
            }
            assertEquals(0, view.editorEditText.activeImageLoadHandleCountForTesting())
            org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
            parent.measure(widthSpec, heightSpec)
            parent.layout(0, 0, 600, 900)
            view.editorEditText.draw(Canvas(Bitmap.createBitmap(600, 900, Bitmap.Config.ARGB_8888)))

            val liveText = view.editorEditText.text as Spanned
            val liveImageSpan = liveText.getSpans(
                0,
                liveText.length,
                BlockImageSpan::class.java,
            ).single()
            val imageOffset = liveText.getSpanStart(liveImageSpan)
            val layout = requireNotNull(view.editorEditText.layout)
            val imageLine = layout.getLineForOffset(imageOffset)
            val imageLineTop = view.editorEditText.totalPaddingTop + layout.getLineTop(imageLine)
            val imageRect = requireNotNull(liveImageSpan.currentDrawRect())
            val followingOffset = liveText.toString().indexOf("After")
            val followingLine = layout.getLineForOffset(followingOffset)
            val followingLineTop = view.editorEditText.totalPaddingTop +
                layout.getLineTop(followingLine)

            assertTrue(
                "Image top ${imageRect.top} must not precede its line top $imageLineTop",
                imageRect.top >= imageLineTop,
            )
            assertTrue(
                "Following line top $followingLineTop must not precede image bottom ${imageRect.bottom}",
                followingLineTop >= imageRect.bottom,
            )
        } finally {
            releaseDecode.countDown()
            RenderImageLoader.resetForTesting()
        }
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
    fun `image resize handle receives drag through view hierarchy`() {
        val fixture = imageResizeGestureFixture(imageRenderJson())
        val rect = requireNotNull(fixture.view.imageResizeOverlayRectForTesting())
        val events = listOf(
            MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, rect.right, rect.bottom, 0),
            MotionEvent.obtain(0, 8, MotionEvent.ACTION_MOVE, rect.right + 24f, rect.bottom + 24f, 0),
            MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, rect.right + 24f, rect.bottom + 24f, 0),
        )

        events.forEach { event ->
            fixture.view.dispatchTouchEvent(event)
            event.recycle()
        }

        assertEquals(1, fixture.resizeCommands.size)
    }

    @Test
    fun `image resize handle accepts a 48dp touch target`() {
        val fixture = imageResizeGestureFixture(imageRenderJson())
        val rect = requireNotNull(fixture.view.imageResizeOverlayRectForTesting())
        val density = fixture.view.resources.displayMetrics.density
        val downX = rect.left + (20f * density)
        val events = listOf(
            MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, downX, rect.bottom, 0),
            MotionEvent.obtain(0, 8, MotionEvent.ACTION_MOVE, rect.left - 24f, rect.bottom + 24f, 0),
            MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, rect.left - 24f, rect.bottom + 24f, 0),
        )

        events.forEach { event ->
            fixture.view.dispatchTouchEvent(event)
            event.recycle()
        }

        assertEquals(1, fixture.resizeCommands.size)
    }

    @Test
    fun `image resize handles exclude system edge gestures while visible`() {
        val fixture = imageResizeGestureFixture(imageRenderJson())
        fun descendants(group: ViewGroup): Sequence<View> = sequence {
            for (index in 0 until group.childCount) {
                val child = group.getChildAt(index)
                yield(child)
                if (child is ViewGroup) yieldAll(descendants(child))
            }
        }
        val overlay = descendants(fixture.view.editorViewport).filterIsInstance<ImageResizeOverlayView>().single()
        val rect = requireNotNull(fixture.view.imageResizeOverlayRectForTesting())

        val exclusions = overlay.systemGestureExclusionRects
        assertEquals(4, exclusions.size)
        assertTrue(exclusions.any { exclusion ->
            exclusion.contains(rect.left.toInt(), rect.top.toInt())
        })

        fixture.view.setImageResizingEnabled(false)

        assertTrue(overlay.systemGestureExclusionRects.isEmpty())
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
}
