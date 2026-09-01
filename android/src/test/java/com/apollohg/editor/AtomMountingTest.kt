package com.apollohg.editor

import android.app.Activity
import android.content.Context
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
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class AtomMountingTest {
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
    fun `atom child stays under its React Native managed parent`() {
        val editor = nativeEditorView()
        val child = atomChild(editor.context, "counterCard:0")

        editor.addAtomChild(child, 0)

        assertSame(editor, child.parent)
        assertEquals(1, editor.atomChildCount)
        assertSame(child, editor.atomChildAt(0))
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

        assertEquals(280, child.measuredWidth)
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

        assertEquals(132, host.height)
        assertEquals(132, view.measuredAtomHeightForTesting("counterCard:0"))
        assertEquals(132, atomSpans(view).single().reservedHeightPx)
        assertEquals(1, view.atomHeightRenderApplyCountForTesting())
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
        activity.setContentView(editor)
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

        assertTrue(editor.richTextView.editorScrollView.scrollY > scrollAfterVerticalMove)
        assertEquals(0, presses)
    }

    @Test
    fun `horizontal atom gesture is not intercepted by editor scrolling`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editor = nativeEditorView()
        editor.onAtomLayoutForTesting = {}
        activity.setContentView(editor)
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
        activity.setContentView(editor)
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
        assertEquals(0, editor.atomScrollTouchDispatchCountForTesting)
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

        assertEquals(firstY - secondY, (first.translationY - second.translationY).toInt())
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
        assertEquals(host.childCount - 2, host.indexOfChild(first))
        assertEquals(host.childCount - 1, host.indexOfChild(second))
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

        assertSame(content, view.editorEditText.text)
        assertEquals(164, atomSpans(view).single().reservedHeightPx)
        assertEquals(1, view.atomHeightRenderApplyCountForTesting())
    }

    @Test
    fun `layout atom host views offsets from the React Native layout`() {
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

        assertEquals(13, child.left)
        assertEquals(19, child.top)
        assertEquals((expectedX - child.left).toFloat(), child.translationX)
        assertEquals((expectedY - child.top).toFloat(), child.translationY)
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
            listOf(mapOf("key" to "counterCard:0", "x" to expectedX, "y" to expectedY)),
            event["positions"],
        )
    }

    @Test
    fun `atom child removal clears height registry`() {
        val editor = nativeEditorView()
        installAtoms(editor.richTextView, listOf("counterCard:0"))
        val child = atomChild(editor.context, "counterCard:0")
        editor.addAtomChild(child, 0)
        child.layout(0, 0, 280, 164)
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
    fun `scrolling does not republish content anchored atom positions`() {
        val editor = nativeEditorView()
        val events = mutableListOf<List<AtomLayoutPosition>>()
        editor.richTextView.onAtomLayoutChange = { _, positions -> events.add(positions) }
        installAtoms(editor.richTextView, listOf("first", "second", "third", "fourth"))
        layout(editor, 320, 240)
        val initialPositions = events.last()
        val initialCount = events.size

        editor.richTextView.editorScrollView.scrollTo(0, 100)

        assertEquals(initialPositions, events.last())
        assertEquals(initialCount, events.size)
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
    }

}
