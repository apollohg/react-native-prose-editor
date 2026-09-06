package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.os.Looper
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import com.facebook.react.R
import com.facebook.react.uimanager.TouchTargetHelper
import com.facebook.react.views.view.ReactViewGroup
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import java.lang.ref.WeakReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class AtomMountingTest {
    @Test
    fun `atom spacing before following paragraph uses atom theme spacing`() {
        val editor = EditorEditText(RuntimeEnvironment.getApplication())
        editor.applyTheme(
            EditorTheme.fromJson(
                """{"text":{"spacingAfter":11},"paragraph":{"spacingAfter":29}}"""
            )
        )
        editor.applyAtomRenderConfiguration(
            AtomRenderConfiguration(
                registeredNodeTypes = setOf("counterCard"),
                estimatedHeightsDp = mapOf("counterCard" to 72f),
                measuredHeightsPx = emptyMap(),
            )
        )
        editor.applyRenderJSON(
            """
            [
              {"type":"voidBlock","nodeType":"counterCard","docPos":1,"atomId":"counter-1"},
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"textRun","text":"Below","marks":[]},
              {"type":"blockEnd"}
            ]
            """.trimIndent()
        )
        val content = editor.text as android.text.Spanned
        val newline = content.toString().indexOf('\n')
        val spacer = content.getSpans(newline, newline + 1, ParagraphSpacerSpan::class.java).single()

        assertEquals(11, spacer.spacingPx)
    }

    @Test
    fun `content frame wraps edit text as scroll child`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())

        assertEquals(1, view.editorScrollView.childCount)
        assertSame(view.editorContentFrame, view.editorScrollView.getChildAt(0))
        assertSame(view.editorEditText, view.editorContentFrame.getChildAt(0))
    }

    @Test
    fun `content frame tap below content still focuses editor`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val view = RichTextEditorView(activity)
        activity.setContentView(view)
        view.editorEditText.setText("Short")
        layout(view, 320, 500)
        view.editorEditText.clearFocus()

        val y = view.editorContentFrame.height - 4f
        val handledDown = view.editorContentFrame.dispatchTouchEvent(
            MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 10f, y, 0)
        )
        val handledUp = view.editorContentFrame.dispatchTouchEvent(
            MotionEvent.obtain(0, 1, MotionEvent.ACTION_UP, 10f, y, 0)
        )

        assertTrue(
            "down=$handledDown up=$handledUp frame=${view.editorContentFrame.height} edit=${view.editorEditText.height}",
            view.editorEditText.hasFocus()
        )
    }

    @Test
    fun `atom child stays in scroll content with React logical ownership`() {
        val editor = nativeEditorView()
        val child = atomChild(editor.context, "counterCard:0")

        editor.addAtomChild(child, 0)

        assertSame(editor.richTextView.editorContentFrame, child.parent)
        assertEquals(1, editor.atomChildCount)
        assertSame(child, editor.atomChildAt(0))
    }

    @Test
    fun `atom child renders once from inside the scroll view`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editor = nativeEditorView()
        editor.onAtomLayoutForTesting = {}
        activity.setContentView(editor, ViewGroup.LayoutParams(320, 240))
        installAtoms(editor.richTextView, listOf("first", "second", "third", "fourth"))
        var drawCount = 0
        val host = object : FrameLayout(activity) {
            init {
                setWillNotDraw(false)
                setTag(R.id.view_tag_native_id, "prose-atom:fourth")
                layoutParams = FrameLayout.LayoutParams(280, 100)
            }

            override fun onDraw(canvas: Canvas) {
                super.onDraw(canvas)
                drawCount += 1
            }
        }
        editor.addAtomChild(host, 0)
        layout(editor, 320, 240)
        host.layout(0, 0, 280, 100)
        shadowOf(Looper.getMainLooper()).idle()
        editor.richTextView.layoutAtomHostViews()
        val initialAtomY = host.y.toInt()
        val initialWindowPosition = IntArray(2).also(host::getLocationOnScreen)
        val requestedScrollY = (host.y.toInt() - 160).coerceAtLeast(1)
        editor.richTextView.editorScrollView.scrollTo(0, requestedScrollY)
        editor.richTextView.layoutAtomHostViews()
        val atomY = IntArray(2).also(host::getLocationOnScreen)[1]
        drawCount = 0
        val bitmap = Bitmap.createBitmap(320, 240, Bitmap.Config.ARGB_8888)

        editor.draw(Canvas(bitmap))

        assertSame(editor.richTextView.editorContentFrame, host.parent)
        assertTrue(
            "editor=${editor.width}x${editor.height}, viewport=${editor.richTextView.editorScrollView.height}",
            editor.richTextView.editorScrollView.scrollY > 0,
        )
        assertTrue(
            "atomY=$atomY, hostHeight=${host.height}, spanHeight=${atomSpans(editor.richTextView).last().reservedHeightPx}",
            atomY in 0 until bitmap.height,
        )
        assertEquals(
            initialWindowPosition[1] - editor.richTextView.editorScrollView.scrollY,
            atomY,
        )
        assertEquals(1, drawCount)
        assertEquals(initialAtomY, host.top)
        assertEquals(0f, host.translationY)
        assertEquals(
            View.OVER_SCROLL_IF_CONTENT_SCROLLS,
            editor.richTextView.editorScrollView.overScrollMode,
        )
    }

    @Test
    fun `auto grow measures a mounted React atom host with explicit specs`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        view.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        installAtoms(view, listOf("counterCard:0"))
        val child = ReactViewGroup(view.context).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:counterCard:0")
            layoutParams = FrameLayout.LayoutParams(280, ViewGroup.LayoutParams.WRAP_CONTENT)
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(132, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 132)
        }
        view.mountAtomChild(child, "counterCard:0")

        view.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(500, View.MeasureSpec.AT_MOST),
        )

        assertEquals(view.editorEditText.measuredWidth - view.editorEditText.compoundPaddingLeft - view.editorEditText.compoundPaddingRight, child.measuredWidth)
        assertEquals(132, child.measuredHeight)
    }

    @Test
    fun `atom height follows rendered content instead of the editor sized React host`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        view.setHeightBehavior(EditorHeightBehavior.AUTO_GROW)
        installAtoms(view, listOf("counterCard:0"))
        val renderedCard = FrameLayout(view.context).apply {
            layoutParams = FrameLayout.LayoutParams(280, 132)
        }
        val host = ReactViewGroup(view.context).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:counterCard:0")
            addView(renderedCard)
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(500, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 500)
        }
        renderedCard.layout(0, 0, 280, 132)

        view.mountAtomChild(host, "counterCard:0")
        host.layout(0, 0, 280, 1_000)
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(132, host.height)
        assertEquals(132, view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(132, atomSpans(view).single().reservedHeightPx)
        assertEquals(1, view.atomHeightRenderApplyCountForTesting())
    }

    @Test
    fun `atom height follows Fabric parent first layout batches`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0"))
        val card = View(view.context)
        val host = ReactViewGroup(view.context).apply { addView(card) }
        host.layout(0, 0, 280, 96)
        card.layout(0, 0, 280, 96)
        view.mountAtomChild(host, "counterCard:0")

        for (height in listOf(72, 160, 0, 96)) {
            host.layout(0, 0, 280, height)
            card.layout(0, 0, 280, height)
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(height, host.height)
            assertEquals(height, view.measuredAtomHeightForTesting("counterCard:0"))
            assertEquals(height, atomSpans(view).single().reservedHeightPx)
        }
    }

    @Test
    fun `unmount cancels a pending atom measurement`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0"))
        val child = atomChild(view.context, "counterCard:0")
        child.layout(0, 0, 280, 96)
        view.mountAtomChild(child, "counterCard:0")
        child.layout(0, 0, 280, 160)
        view.unmountAtomChild(child)
        val fallback = atomSpans(view).single().reservedHeightPx
        shadowOf(Looper.getMainLooper()).idle()

        assertNull(view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(fallback, atomSpans(view).single().reservedHeightPx)
    }

    @Test
    fun `atom child can collapse to zero and expand again`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0"))
        val child = atomChild(view.context, "counterCard:0")
        child.layout(0, 0, 280, 96)
        view.mountAtomChild(child, "counterCard:0")

        for (height in listOf(0, 72)) {
            child.layout(0, 0, 280, height)
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(height, view.measuredAtomHeightForTesting("counterCard:0"))
            assertEquals(height, atomSpans(view).single().reservedHeightPx)
        }
    }

    @Test
    fun `unmeasured atom keeps estimate until its first zero height layout`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0"))
        val child = atomChild(view.context, "counterCard:0")
        val estimate = atomSpans(view).single().reservedHeightPx
        view.mountAtomChild(child, "counterCard:0")

        assertNull(view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(estimate, atomSpans(view).single().reservedHeightPx)

        child.layout(0, 0, 280, 0)
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(0, view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(0, atomSpans(view).single().reservedHeightPx)
    }

    @Test
    fun `collapsed rendered content clears an oversized React atom host`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0"))
        val card = View(view.context)
        val host = ReactViewGroup(view.context).apply { addView(card) }
        host.layout(0, 0, 280, 500)
        card.layout(0, 0, 280, 96)
        view.mountAtomChild(host, "counterCard:0")

        card.layout(0, 0, 280, 0)
        host.layout(0, 0, 280, 500)
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(0, host.height)
        assertEquals(0, view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(0, atomSpans(view).single().reservedHeightPx)
    }

    @Test
    fun `atom child mounted before its span supplies height to the later render`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        view.applyAtomRenderConfiguration(
            AtomRenderConfiguration(setOf("counterCard"), mapOf("counterCard" to 40f), emptyMap())
        )
        val child = atomChild(view.context, "counterCard:0").apply {
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(132, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 132)
        }

        view.mountAtomChild(child, "counterCard:0")
        view.editorEditText.applyRenderJSON(
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1}]"""
        )

        assertEquals(132, view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(132, atomSpans(view).single().reservedHeightPx)
    }

    @Test
    fun `decorative overlays do not mask a React atom touch target`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val root = FrameLayout(activity).apply { id = 100 }
        val editor = nativeEditorView().apply { id = 101 }
        editor.onAtomLayoutForTesting = {}
        root.addView(editor, FrameLayout.LayoutParams(320, 500))
        activity.setContentView(root)
        installAtoms(editor.richTextView, listOf("counterCard:0"))
        val child = ReactViewGroup(editor.context).apply {
            id = 202
            setTag(R.id.view_tag_native_id, "prose-atom:counterCard:0")
            layoutParams = FrameLayout.LayoutParams(280, 132)
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(132, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 132)
        }
        editor.addAtomChild(child, 0)
        layout(root, 320, 500)
        editor.richTextView.layoutAtomHostViews()
        val childLocation = IntArray(2)
        val rootLocation = IntArray(2)
        child.getLocationOnScreen(childLocation)
        root.getLocationOnScreen(rootLocation)
        val coords = floatArrayOf(
            childLocation[0] - rootLocation[0] + 50f,
            childLocation[1] - rootLocation[1] + 50f,
        )

        val target = TouchTargetHelper.findTargetTagAndCoordinatesForTouch(
            coords[0],
            coords[1],
            root,
            coords,
            null,
        )

        assertEquals(child.id, target)
    }

    @Test
    fun `dragging an atom scrolls a fixed height editor without pressing it`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editor = nativeEditorView()
        editor.onAtomLayoutForTesting = {}
        activity.setContentView(editor, ViewGroup.LayoutParams(320, 240))
        installAtoms(editor.richTextView, listOf("first", "second", "third", "fourth"))
        var presses = 0
        val action = object : View(activity) {
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if (event.actionMasked == MotionEvent.ACTION_UP) presses += 1
                return true
            }
        }
        val host = ReactViewGroup(activity).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:second")
            addView(action, FrameLayout.LayoutParams(280, 100))
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(100, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 100)
        }
        action.layout(0, 0, 280, 100)
        editor.addAtomChild(host, 0)
        layout(editor, 320, 240)
        editor.richTextView.layoutAtomHostViews()
        val startX = host.x + 50f
        val startY = host.y + 70f

        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 1, MotionEvent.ACTION_DOWN, startX, startY, 0)
        )
        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 2, MotionEvent.ACTION_MOVE, startX, startY - 80f, 0)
        )
        val scrollAfterVerticalMove = editor.richTextView.editorScrollView.scrollY
        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 3, MotionEvent.ACTION_MOVE, startX + 200f, startY - 120f, 0)
        )
        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 4, MotionEvent.ACTION_UP, startX + 200f, startY - 120f, 0)
        )

        assertTrue(
            "editor=${editor.width}x${editor.height}, before=$scrollAfterVerticalMove, after=${editor.richTextView.editorScrollView.scrollY}",
            editor.richTextView.editorScrollView.scrollY > scrollAfterVerticalMove,
        )
        assertEquals(0, presses)
    }

    @Test
    fun `horizontal atom gesture is not intercepted by editor scrolling`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editor = nativeEditorView()
        editor.onAtomLayoutForTesting = {}
        activity.setContentView(editor, ViewGroup.LayoutParams(320, 240))
        installAtoms(editor.richTextView, listOf("first", "second", "third", "fourth"))
        var releases = 0
        val action = object : View(activity) {
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if (event.actionMasked == MotionEvent.ACTION_UP) releases += 1
                return true
            }
        }
        val host = ReactViewGroup(activity).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:second")
            addView(action, FrameLayout.LayoutParams(280, 100))
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(100, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 100)
        }
        action.layout(0, 0, 280, 100)
        editor.addAtomChild(host, 0)
        layout(editor, 320, 240)
        editor.richTextView.layoutAtomHostViews()
        val startX = host.x + 50f
        val startY = host.y + 50f

        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 1, MotionEvent.ACTION_DOWN, startX, startY, 0)
        )
        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 2, MotionEvent.ACTION_MOVE, startX + 80f, startY - 20f, 0)
        )
        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 3, MotionEvent.ACTION_MOVE, startX + 20f, startY - 100f, 0)
        )
        editor.dispatchTouchEvent(
            MotionEvent.obtain(1, 4, MotionEvent.ACTION_UP, startX + 20f, startY - 100f, 0)
        )

        assertEquals(0, editor.richTextView.editorScrollView.scrollY)
        assertEquals(1, releases)
    }

    @Test
    fun `atom tap is delivered only to the React child`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editor = nativeEditorView()
        editor.onAtomLayoutForTesting = {}
        activity.setContentView(editor, ViewGroup.LayoutParams(320, 240))
        installAtoms(editor.richTextView, listOf("counterCard:0"))
        var releases = 0
        val action = object : View(activity) {
            override fun onTouchEvent(event: MotionEvent): Boolean {
                if (event.actionMasked == MotionEvent.ACTION_UP) releases += 1
                return true
            }
        }
        val host = ReactViewGroup(activity).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:counterCard:0")
            addView(action, FrameLayout.LayoutParams(280, 100))
            measure(
                View.MeasureSpec.makeMeasureSpec(280, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(100, View.MeasureSpec.EXACTLY),
            )
            layout(0, 0, 280, 100)
        }
        action.layout(0, 0, 280, 100)
        editor.addAtomChild(host, 0)
        layout(editor, 320, 240)
        editor.richTextView.layoutAtomHostViews()
        val x = host.x + 50f
        val y = host.y + 50f

        editor.dispatchTouchEvent(MotionEvent.obtain(1, 1, MotionEvent.ACTION_DOWN, x, y, 0))
        editor.dispatchTouchEvent(MotionEvent.obtain(1, 2, MotionEvent.ACTION_UP, x, y, 0))

        assertEquals(1, releases)
        assertEquals(0, editor.richTextView.editorScrollView.scrollY)
    }

    @Test
    fun `atom child binds to span by key not order`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("first", "second"))
        val second = atomChild(view.context, "second")
        val first = atomChild(view.context, "first")
        view.mountAtomChild(second, "second")
        view.mountAtomChild(first, "first")
        layout(view, 320, 500)
        view.layoutAtomHostViews()

        val spans = atomSpans(view)
        val firstSpan = spans.single { it.atomKey == "first" }
        val secondSpan = spans.single { it.atomKey == "second" }
        val text = requireNotNull(view.editorEditText.text)
        val textLayout = requireNotNull(view.editorEditText.layout)
        val firstY = textLayout.getLineTop(textLayout.getLineForOffset(text.getSpanStart(firstSpan)))
        val secondY = textLayout.getLineTop(textLayout.getLineForOffset(text.getSpanStart(secondSpan)))

        assertEquals(firstY - secondY, first.top - second.top)
    }

    @Test
    fun `laying out multiple atoms keeps the established host z order`() {
        val context = RuntimeEnvironment.getApplication() as Context
        val host = FrameLayout(context)
        val editor = RichTextEditorView(context)
        installAtoms(editor, listOf("first", "second"))
        val first = atomChild(context, "first")
        val second = atomChild(context, "second")
        host.addView(editor)
        host.addView(first)
        host.addView(second)
        editor.mountAtomChild(first, "first")
        editor.mountAtomChild(second, "second")
        layout(host, 320, 500)
        editor.layoutAtomHostViews()
        layout(host, 320, 500)

        editor.layoutAtomHostViews()

        assertFalse(host.isLayoutRequested)
        assertSame(editor.editorContentFrame, first.parent)
        assertSame(editor.editorContentFrame, second.parent)
        assertEquals(editor.editorContentFrame.childCount - 2, editor.editorContentFrame.indexOfChild(first))
        assertEquals(editor.editorContentFrame.childCount - 1, editor.editorContentFrame.indexOfChild(second))
    }

    @Test
    fun `atom child height change updates span exactly once`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0"))
        val child = atomChild(view.context, "counterCard:0")
        view.mountAtomChild(child, "counterCard:0")
        val content = view.editorEditText.text

        child.layout(0, 0, 280, 164)
        child.layout(0, 0, 280, 164)
        shadowOf(Looper.getMainLooper()).idle()

        assertSame(content, view.editorEditText.text)
        assertEquals(164, atomSpans(view).single().reservedHeightPx)
        assertEquals(1, view.atomHeightRenderApplyCountForTesting())
    }

    @Test
    fun `native content bounds replace stale React Native layout coordinates`() {
        val editor = nativeEditorView()
        editor.onAtomLayoutForTesting = {}
        val view = editor.richTextView
        view.editorEditText.setPadding(17, 23, 11, 7)
        installAtoms(view, listOf("counterCard:0"))
        val child = atomChild(view.context, "counterCard:0")
        editor.addAtomChild(child, 0)
        layout(editor, 320, 500)
        child.layout(13, 19, 293, 151)
        view.layoutAtomHostViews()

        val span = atomSpans(view).single()
        val text = requireNotNull(view.editorEditText.text)
        val textLayout = requireNotNull(view.editorEditText.layout)
        val spanStart = text.getSpanStart(span)
        val expectedX = view.editorEditText.left + view.editorEditText.compoundPaddingLeft
        val expectedY = view.editorEditText.top + view.editorEditText.totalPaddingTop +
            textLayout.getLineTop(textLayout.getLineForOffset(spanStart)) -
            view.editorEditText.scrollY

        assertEquals(expectedX, child.left)
        assertEquals(expectedY, child.top)
        assertEquals(0f, child.translationX)
        assertEquals(0f, child.translationY)
    }

    @Test
    @Config(sdk = [34], qualifiers = "xhdpi")
    fun `atom layout event reports content width and positions in dp`() {
        val editor = nativeEditorView()
        val events = mutableListOf<Map<String, Any>>()
        editor.onAtomLayoutForTesting = { events.add(it) }
        installAtoms(editor.richTextView, listOf("counterCard:0"))

        layout(editor, 400, 500)

        val editText = editor.richTextView.editorEditText
        val span = atomSpans(editor.richTextView).single()
        val text = requireNotNull(editText.text)
        val textLayout = requireNotNull(editText.layout)
        val spanStart = text.getSpanStart(span)
        val density = editor.resources.displayMetrics.density
        val expectedWidth = (
            editText.width - editText.compoundPaddingLeft - editText.compoundPaddingRight
        ).toFloat() / density
        val expectedX = (editText.left + editText.compoundPaddingLeft) / density
        val expectedY = (
            editText.top +
                editText.totalPaddingTop +
                textLayout.getLineTop(textLayout.getLineForOffset(spanStart)) -
                editText.scrollY
        ) / density
        val event = events.last()
        assertEquals(expectedWidth, event["width"] as Float)
        assertEquals(
            listOf(mapOf("key" to "counterCard:0", "x" to expectedX, "y" to expectedY, "hostX" to expectedX, "hostY" to expectedY, "height" to span.reservedHeightPx / density, "width" to (editText.width - editText.compoundPaddingLeft - editText.compoundPaddingRight) / density)),
            event["positions"],
        )
        assertEquals(mapOf("y" to 0f, "height" to editor.richTextView.editorScrollView.height / density), event["viewport"])
        val previousCount = events.size
        editor.richTextView.editorScrollView.scrollTo(0, 10)
        editor.richTextView.emitAtomLayoutIfAvailable(force = true)
        assertTrue(events.size > previousCount)
    }

    @Test
    fun `atom child removal clears height registry`() {
        val editor = nativeEditorView()
        installAtoms(editor.richTextView, listOf("counterCard:0"))
        val child = atomChild(editor.context, "counterCard:0")
        editor.addAtomChild(child, 0)
        child.layout(0, 0, 280, 164)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(164, editor.richTextView.measuredAtomHeightForTesting("counterCard:0"))

        editor.removeAtomChild(child)

        assertNull(editor.richTextView.measuredAtomHeightForTesting("counterCard:0"))
        assertNull(child.parent)
        assertEquals(0, editor.atomChildCount)
    }

    @Test
    fun `reassigning an atom child clears the old fallback key height`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("counterCard:0", "counterCard:1"))
        val child = atomChild(view.context, "counterCard:0")
        child.layout(0, 0, 280, 164)
        view.mountAtomChild(child, "counterCard:0")
        assertEquals(164, view.measuredAtomHeightForTesting("counterCard:0"))

        view.mountAtomChild(child, "counterCard:1")

        assertNull(view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(164, view.measuredAtomHeightForTesting("counterCard:1"))
    }

    @Test
    fun `scrolling preserves content positions while publishing viewport changes`() {
        val editor = nativeEditorView()
        val events = mutableListOf<List<AtomLayoutPosition>>()
        editor.richTextView.onAtomLayoutChange = { _, positions -> events.add(positions) }
        installAtoms(editor.richTextView, listOf("first", "second", "third", "fourth"))
        layout(editor, 320, 240)
        val initialPositions = events.last()
        val initialCount = events.size

        editor.richTextView.editorScrollView.scrollTo(0, 100)

        val scrollY = editor.richTextView.editorScrollView.scrollY
        assertTrue(scrollY > 0)
        assertEquals(
            initialPositions,
            events.last(),
        )
        assertTrue(events.size > initialCount)
    }

    @Test
    fun `React logical child reorder and removal preserve content parenting`() {
        val editor = nativeEditorView()
        installAtoms(editor.richTextView, listOf("first", "second"))
        val first = atomChild(editor.context, "first")
        val second = atomChild(editor.context, "second")
        editor.addAtomChild(first, 0)
        editor.addAtomChild(second, 1)
        editor.removeAtomChildAt(0)
        editor.addAtomChild(first, 1)
        assertSame(second, editor.atomChildAt(0))
        assertSame(first, editor.atomChildAt(1))
        val content = editor.richTextView.editorContentFrame
        assertSame(content, first.parent)
        assertSame(content, second.parent)
        assertTrue(content.indexOfChild(second) < content.indexOfChild(first))
        editor.removeAtomChildAt(0)
        assertNull(second.parent)
        assertSame(first, editor.atomChildAt(0))
    }

    @Test
    fun `stale width measurement cannot resize an atom after reflow`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("first", "second", "third"))
        layout(view, 320, 240)
        val card = View(view.context)
        val host = ReactViewGroup(view.context).apply { addView(card) }
        val width = view.editorEditText.width - view.editorEditText.compoundPaddingLeft - view.editorEditText.compoundPaddingRight
        host.layout(0, 0, width, 80)
        card.layout(0, 0, width, 80)
        view.mountAtomChild(host, "first")
        layout(view, 320, 240)
        host.layout(0, 0, width, 150)
        card.layout(0, 0, width, 150)
        layout(view, 400, 240)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(80, view.measuredAtomHeightForTesting("first"))
        val newWidth = host.width
        host.layout(0, 0, newWidth, 150)
        card.layout(0, 0, newWidth, 150)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(150, view.measuredAtomHeightForTesting("first"))
    }

    @Test
    fun `flattened Fabric siblings do not prevent root height changes`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("first", "second", "third"))
        layout(view, 320, 240)
        val root = ReactViewGroup(view.context).apply {
            setTag(R.id.view_tag_native_id, "prose-atom-content:first")
        }
        val button = View(view.context)
        val host = ReactViewGroup(view.context).apply { addView(root); addView(button) }
        val width = view.editorEditText.width
        host.layout(0, 0, width, 100)
        root.layout(0, 0, width, 100)
        button.layout(20, 20, 60, 60)
        view.mountAtomChild(host, "first")
        layout(view, 320, 240)

        host.layout(0, 0, width, 180)
        root.layout(0, 0, width, 180)
        button.layout(20, 80, 60, 120)
        shadowOf(Looper.getMainLooper()).idle()
        layout(view, 320, 240)
        assertEquals(180, view.measuredAtomHeightForTesting("first"))
        assertEquals(180, host.height)

        host.layout(0, 0, width, 80)
        root.layout(0, 0, width, 80)
        button.layout(20, 20, 60, 60)
        shadowOf(Looper.getMainLooper()).idle()
        layout(view, 320, 240)
        assertEquals(80, view.measuredAtomHeightForTesting("first"))
        assertEquals(80, host.height)
    }

    @Test
    fun `flattened measurement root rejects the previous Yoga width`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("first", "second", "third"))
        layout(view, 320, 240)
        val root = ReactViewGroup(view.context).apply {
            setTag(R.id.view_tag_native_id, "prose-atom-content:first")
            layout(0, 0, 320, 80)
        }
        val button = View(view.context).apply { layout(20, 20, 60, 60) }
        val host = ReactViewGroup(view.context).apply {
            addView(root); addView(button)
            layout(0, 0, 320, 80)
        }
        view.mountAtomChild(host, "first")
        layout(view, 400, 240)
        host.layout(0, 0, 400, 150)
        root.layout(0, 0, 320, 150)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(80, view.measuredAtomHeightForTesting("first"))

        host.layout(0, 0, 400, 150)
        root.layout(0, 0, 400, 150)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(150, view.measuredAtomHeightForTesting("first"))
    }

    @Test
    fun `Fabric parent cannot swallow native content remeasurement after atom growth`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val parent = object : FrameLayout(activity) {
            override fun requestLayout() = Unit
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                super.onMeasure(MeasureSpec.makeMeasureSpec(320, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(240, MeasureSpec.EXACTLY))
            }
        }
        val editor = nativeEditorView().apply { onAtomLayoutForTesting = {} }
        parent.addView(editor, FrameLayout.LayoutParams(320, 240))
        activity.setContentView(parent)
        val view = editor.richTextView
        installAtoms(view, listOf("first", "second", "third", "fourth", "fifth"))
        layout(editor, 320, 240)
        val root = ReactViewGroup(editor.context).apply {
            setTag(R.id.view_tag_native_id, "prose-atom-content:first")
            layout(0, 0, 320, 100)
        }
        val host = ReactViewGroup(editor.context).apply {
            setTag(R.id.view_tag_native_id, "prose-atom:first")
            addView(root)
            layout(0, 0, 320, 100)
        }
        editor.addAtomChild(host, 0)
        layout(editor, 320, 240)
        val before = view.editorEditText.height

        host.layout(0, 0, 320, 180)
        root.layout(0, 0, 320, 180)
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals(180, view.measuredAtomHeightForTesting("first"))
        assertEquals(before + 80, view.editorEditText.height)
        assertTrue(view.editorContentFrame.height >= view.editorEditText.layout.height)

        host.layout(0, 0, 320, 80)
        root.layout(0, 0, 320, 80)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(before - 20, view.editorEditText.height)
    }

    @Test
    fun `atom growth above viewport preserves the visible text anchor`() {
        val activity = RuntimeEnvironment.getApplication()
        val view = RichTextEditorView(activity)
        installAtoms(view, listOf("first", "second", "third", "fourth", "fifth"))
        layout(view, 320, 240)
        val width = view.editorEditText.width - view.editorEditText.compoundPaddingLeft - view.editorEditText.compoundPaddingRight
        val host = atomChild(activity, "first").apply { layout(0, 0, width, 100) }
        view.mountAtomChild(host, "first")
        layout(view, 320, 240)
        view.editorScrollView.scrollTo(0, 150)
        val text = requireNotNull(view.editorEditText.text)
        val anchor = text.getSpanStart(atomSpans(view).first { it.atomKey == "second" })
        fun anchorY(): Int {
            val layout = requireNotNull(view.editorEditText.layout)
            return layout.getLineTop(layout.getLineForOffset(anchor)) - view.editorScrollView.scrollY
        }
        val before = anchorY()
        host.layout(0, host.top, width, host.top + 180)
        shadowOf(Looper.getMainLooper()).idle()
        layout(view, 320, 240)
        assertEquals(before, anchorY())
        assertEquals(230, view.editorScrollView.scrollY)
        assertEquals(0f, host.translationY)
        assertSame(text, view.editorEditText.text)
    }

    @Test
    fun `owner change rejects queued measurements from the previous document`() {
        val view = RichTextEditorView(RuntimeEnvironment.getApplication())
        installAtoms(view, listOf("first"))
        val child = atomChild(view.context, "first").apply { layout(0, 0, 280, 80) }
        view.mountAtomChild(child, "first")
        child.layout(0, 0, 280, 160)
        view.setEditorIdWhileDetached(91L)
        shadowOf(Looper.getMainLooper()).idle()
        assertNull(view.measuredAtomHeightForTesting("first"))
    }

    private fun installAtoms(view: RichTextEditorView, keys: List<String>) {
        val nodeTypes = keys.map { key -> if (key.contains(':')) key.substringBefore(':') else key }
        view.applyAtomRenderConfiguration(
            AtomRenderConfiguration(nodeTypes.toSet(), nodeTypes.associateWith { 100f }, emptyMap())
        )
        val renderJson = keys.mapIndexed { index, key ->
            val nodeType = nodeTypes[index]
            val atomId = if (key == "$nodeType:$index") "" else ",\"atomId\":\"$key\""
            "{\"type\":\"voidBlock\",\"nodeType\":\"$nodeType\",\"docPos\":${index * 2 + 1}$atomId}"
        }.joinToString(prefix = "[", postfix = "]")
        view.editorEditText.applyRenderJSON(renderJson)
    }

    private fun atomSpans(view: RichTextEditorView): List<AtomBlockSpan> {
        val text = requireNotNull(view.editorEditText.text)
        return text.getSpans(0, text.length, AtomBlockSpan::class.java).toList()
    }

    private fun atomChild(context: Context, key: String): FrameLayout = FrameLayout(context).apply {
        setTag(R.id.view_tag_native_id, "prose-atom:$key")
        layoutParams = FrameLayout.LayoutParams(280, ViewGroup.LayoutParams.WRAP_CONTENT)
    }

    private fun nativeEditorView(): NativeEditorExpoView {
        val context = RuntimeEnvironment.getApplication() as Context
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context
        val modulesProvider = object : ModulesProvider {
            override fun getModulesMap(): Map<Class<out Module>, String?> = emptyMap()
        }
        val constructor = AppContext::class.java.constructors.first { it.parameterTypes.size == 3 }
        val appContext = constructor.newInstance(
            modulesProvider,
            ModuleRegistry(emptyList(), emptyList()),
            WeakReference(reactContext)
        ) as AppContext
        return NativeEditorExpoView(reactContext, appContext)
    }

    private fun layout(view: View, width: Int, height: Int) {
        view.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY)
        )
        view.layout(0, 0, width, height)
        shadowOf(Looper.getMainLooper()).idle()
    }

}
