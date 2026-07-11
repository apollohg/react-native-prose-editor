package com.apollohg.editor

import android.content.Context
import android.os.SystemClock
import android.text.Annotation
import android.text.Spanned
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import java.lang.ref.WeakReference

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeProseViewerExpoViewTest {
    @Test
    fun `drag ending over a link does not open it`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }
        val link = pointForAnnotation(proseView, RenderBridge.NATIVE_LINK_HREF_ANNOTATION)
        val plain = TouchPoint(proseView.width - 1f, proseView.height / 2f)

        dispatchGesture(
            proseView,
            motion(MotionEvent.ACTION_DOWN, plain),
            motion(MotionEvent.ACTION_MOVE, link),
            motion(MotionEvent.ACTION_UP, link)
        )

        assertEquals(0, activations)
    }

    @Test
    fun `drag ending over a mention does not activate it`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.MENTION)
        var activations = 0
        viewer.onMentionTapForTesting = { activations += 1 }
        val mention = pointForAnnotation(proseView, "nativeVoidNodeType")
        val plain = TouchPoint(proseView.width - 1f, proseView.height / 2f)

        proseView.dispatchTouchEvent(motion(MotionEvent.ACTION_DOWN, plain))
        proseView.dispatchTouchEvent(motion(MotionEvent.ACTION_MOVE, mention))
        proseView.dispatchTouchEvent(motion(MotionEvent.ACTION_UP, mention))

        assertEquals(0, activations)
    }

    @Test
    fun `action cancel prevents a later up from opening a link`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }
        val link = pointForAnnotation(proseView, RenderBridge.NATIVE_LINK_HREF_ANNOTATION)

        dispatchGesture(
            proseView,
            motion(MotionEvent.ACTION_DOWN, link),
            motion(MotionEvent.ACTION_CANCEL, link),
            motion(MotionEvent.ACTION_UP, link)
        )

        assertEquals(0, activations)
    }

    @Test
    fun `moving beyond touch slop cancels a link tap`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }
        val link = pointForAnnotation(proseView, RenderBridge.NATIVE_LINK_HREF_ANNOTATION)
        val farAway = TouchPoint(link.x, link.y + 100f)

        dispatchGesture(
            proseView,
            motion(MotionEvent.ACTION_DOWN, link),
            motion(MotionEvent.ACTION_MOVE, farAway),
            motion(MotionEvent.ACTION_UP, link)
        )

        assertEquals(0, activations)
    }

    @Test
    fun `distant up without a move does not activate the same link`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }
        val (down, up) = distantLinkPoints(proseView)

        handleProseTouch(viewer, motion(MotionEvent.ACTION_DOWN, down))
        val consumed = handleProseTouch(viewer, motion(MotionEvent.ACTION_UP, up))

        assertFalse(consumed)
        assertEquals(0, activations)
    }

    @Test
    fun `targeted down and in slop move pass through until matched up`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }
        val link = pointForAnnotation(proseView, RenderBridge.NATIVE_LINK_HREF_ANNOTATION)
        val inSlop = link

        val downConsumed = handleProseTouch(viewer, motion(MotionEvent.ACTION_DOWN, link))
        val moveConsumed = handleProseTouch(viewer, motion(MotionEvent.ACTION_MOVE, inSlop))
        val upConsumed = handleProseTouch(viewer, motion(MotionEvent.ACTION_UP, inSlop))

        assertFalse(downConsumed)
        assertFalse(moveConsumed)
        assertTrue(upConsumed)
        assertEquals(1, activations)
    }

    @Test
    fun `additional pointer cancels a mention tap`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.MENTION)
        var activations = 0
        viewer.onMentionTapForTesting = { activations += 1 }
        val mention = pointForAnnotation(proseView, "nativeVoidNodeType")

        dispatchGesture(
            proseView,
            motion(MotionEvent.ACTION_DOWN, mention),
            motion(MotionEvent.ACTION_POINTER_DOWN, mention),
            motion(MotionEvent.ACTION_UP, mention)
        )

        assertEquals(0, activations)
    }

    @Test
    fun `paired down and up on a link opens it`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }
        val link = pointForAnnotation(proseView, RenderBridge.NATIVE_LINK_HREF_ANNOTATION)

        dispatchGesture(
            proseView,
            motion(MotionEvent.ACTION_DOWN, link),
            motion(MotionEvent.ACTION_UP, link)
        )

        assertEquals(1, activations)
    }

    @Test
    fun `paired down and up on a mention is consumed`() {
        val (viewer, proseView) = laidOutInteractiveViewer(TargetKind.MENTION)
        var activations = 0
        viewer.onMentionTapForTesting = { activations += 1 }
        val mention = pointForAnnotation(proseView, "nativeVoidNodeType")

        proseView.dispatchTouchEvent(motion(MotionEvent.ACTION_DOWN, mention))
        proseView.dispatchTouchEvent(motion(MotionEvent.ACTION_UP, mention))

        assertEquals(1, activations)
    }

    @Test
    fun `viewer measure ignores stale exact parent height`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeProseViewerExpoView(expoContext.context, expoContext.appContext)
        view.suppressContentHeightEventsForTesting = true
        view.setRenderJson(paragraphRenderJson(LONG_MESSAGE_TEXT))

        val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
        val wrapHeightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        view.measure(widthSpec, wrapHeightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val contentHeight = view.measuredHeight

        assertTrue(contentHeight > 0)

        val staleExactHeightSpec = View.MeasureSpec.makeMeasureSpec(
            contentHeight + 480,
            View.MeasureSpec.EXACTLY
        )
        view.measure(widthSpec, staleExactHeightSpec)

        assertEquals(contentHeight, view.measuredHeight)
    }

    private data class TestExpoContext(
        val context: Context,
        val appContext: AppContext
    )

    private fun testExpoContext(context: Context): TestExpoContext {
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context

        val modulesProvider = object : ModulesProvider {
            override fun getModulesList(): List<Class<out Module>> = emptyList()
        }
        val constructor = AppContext::class.java.constructors.first { constructor ->
            constructor.parameterTypes.size == 3
        }
        val appContext = constructor.newInstance(
            modulesProvider,
            ModuleRegistry(emptyList(), emptyList()),
            WeakReference(reactContext)
        ) as AppContext
        return TestExpoContext(reactContext, appContext)
    }

    private fun paragraphRenderJson(text: String): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"$text","marks":[]},
          {"type":"blockEnd"}
        ]
        """.trimIndent()

    private enum class TargetKind { LINK, MENTION }

    private fun laidOutInteractiveViewer(
        targetKind: TargetKind
    ): Pair<NativeProseViewerExpoView, EditorEditText> {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeProseViewerExpoView(expoContext.context, expoContext.appContext)
        view.suppressContentHeightEventsForTesting = true
        val targetJson = when (targetKind) {
            TargetKind.LINK ->
                """{"type":"textRun","text":"link target with plenty of width","marks":[{"type":"link","href":"https://example.com/viewer"}]}"""
            TargetKind.MENTION ->
                """{"type":"opaqueInlineAtom","nodeType":"mention","label":"@Alice","docPos":31}"""
        }
        view.setRenderJson(
            """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"textRun","text":"plain text ","marks":[]},
              $targetJson,
              {"type":"blockEnd"}
            ]
            """.trimIndent()
        )
        val widthSpec = View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val proseView = view.getChildAt(0) as EditorEditText
        return view to proseView
    }

    private data class TouchPoint(val x: Float, val y: Float)

    private fun pointForAnnotation(view: EditorEditText, key: String): TouchPoint {
        val text = view.text as Spanned
        val annotation = text.getSpans(0, text.length, Annotation::class.java)
            .first { it.key == key }
        return pointForOffset(view, text.getSpanStart(annotation) + 1)
    }

    private fun pointForOffset(view: EditorEditText, offset: Int): TouchPoint {
        val layout = requireNotNull(view.layout)
        val line = layout.getLineForOffset(offset)
        val startX = layout.getPrimaryHorizontal(offset)
        val endX = layout.getPrimaryHorizontal((offset + 1).coerceAtMost(view.text?.length ?: 0))
        return TouchPoint(
            x = view.totalPaddingLeft + ((startX + endX) / 2f),
            y = view.totalPaddingTop + ((layout.getLineTop(line) + layout.getLineBottom(line)) / 2f)
        )
    }

    private fun distantLinkPoints(view: EditorEditText): Pair<TouchPoint, TouchPoint> {
        val hits = mutableListOf<TouchPoint>()
        for (y in 0 until view.height step 2) {
            for (x in 0 until view.width step 2) {
                if (view.linkHitAt(x.toFloat(), y.toFloat()) != null) {
                    hits += TouchPoint(x.toFloat(), y.toFloat())
                }
            }
        }
        val down = hits.first()
        val up = hits.maxBy { point ->
            val deltaX = point.x - down.x
            val deltaY = point.y - down.y
            deltaX * deltaX + deltaY * deltaY
        }
        val deltaX = up.x - down.x
        val deltaY = up.y - down.y
        val touchSlop = ViewConfiguration.get(view.context).scaledTouchSlop.toFloat()
        assertTrue(deltaX * deltaX + deltaY * deltaY > touchSlop * touchSlop)
        return down to up
    }

    private fun motion(action: Int, point: TouchPoint): MotionEvent {
        val now = SystemClock.uptimeMillis()
        return MotionEvent.obtain(now, now, action, point.x, point.y, 0)
    }

    private fun dispatchGesture(view: View, vararg events: MotionEvent) {
        events.forEach { event ->
            view.dispatchTouchEvent(event)
            event.recycle()
        }
    }

    private fun handleProseTouch(
        viewer: NativeProseViewerExpoView,
        event: MotionEvent
    ): Boolean {
        val method = NativeProseViewerExpoView::class.java.getDeclaredMethod(
            "handleProseTouch",
            MotionEvent::class.java
        )
        method.isAccessible = true
        return try {
            method.invoke(viewer, event) as Boolean
        } finally {
            event.recycle()
        }
    }

    private companion object {
        private val LONG_MESSAGE_TEXT = List(80) {
            "Long Android viewer message"
        }.joinToString(" ")
    }
}
