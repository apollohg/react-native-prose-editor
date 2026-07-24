package com.apollohg.editor

import android.content.Context
import android.os.SystemClock
import android.text.Annotation
import android.text.Spanned
import android.view.MotionEvent
import android.view.View
import android.view.ViewConfiguration
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityEvent
import android.widget.FrameLayout
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
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
    private class AccessibilityEventParent(context: Context) : FrameLayout(context) {
        val eventTypes = mutableListOf<Int>()
        val contentChangeTypes = mutableListOf<Int>()

        override fun requestSendAccessibilityEvent(
            child: View,
            event: AccessibilityEvent
        ): Boolean {
            eventTypes += event.eventType
            contentChangeTypes += event.contentChangeTypes
            return true
        }
    }

    @Test
    fun `accessibility exposes and activates viewer link through touch emitter`() {
        val (viewer, _) = laidOutInteractiveViewer(TargetKind.LINK)
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }

        val host = AccessibilityNodeInfo.obtain(viewer)
        viewer.onInitializeAccessibilityNodeInfo(host)
        assertEquals("plain text link target with plenty of width", host.text.toString())
        assertEquals(1, host.childCount)

        val child = requireNotNull(viewer.accessibilityNodeProvider.createAccessibilityNodeInfo(1))
        val bounds = android.graphics.Rect()
        child.getBoundsInParent(bounds)
        assertEquals("link target with plenty of width", child.text.toString())
        assertEquals("link", AccessibilityNodeInfoCompat.wrap(child).roleDescription.toString())
        assertTrue(bounds.width() > 0)
        assertTrue(bounds.height() > 0)
        assertTrue(child.actionList.any { it.id == AccessibilityNodeInfo.ACTION_CLICK })
        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null
            )
        )
        assertEquals(1, activations)
    }

    @Test
    fun `accessibility exposes and activates viewer mention through touch emitter`() {
        val (viewer, _) = laidOutInteractiveViewer(TargetKind.MENTION)
        var activations = 0
        viewer.onMentionTapForTesting = { activations += 1 }

        val child = requireNotNull(viewer.accessibilityNodeProvider.createAccessibilityNodeInfo(1))
        assertEquals("@Alice", child.text.toString())
        assertEquals("mention", AccessibilityNodeInfoCompat.wrap(child).roleDescription.toString())
        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null
            )
        )
        assertEquals(1, activations)
    }

    @Test
    fun `virtual annotation accessibility focus can move clear and reject invalid ids`() {
        val (viewer, _) = laidOutInteractiveViewer(TargetKind.LINK)
        val parent = AccessibilityEventParent(viewer.context)
        parent.addView(viewer)
        val provider = viewer.accessibilityNodeProvider

        var first = requireNotNull(provider.createAccessibilityNodeInfo(1))
        assertFalse(first.isAccessibilityFocused)
        assertTrue(first.actionList.any {
            it.id == AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS
        })
        assertFalse(
            provider.performAction(
                99,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null
            )
        )

        assertTrue(
            provider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null
            )
        )
        first = requireNotNull(provider.createAccessibilityNodeInfo(1))
        assertTrue(first.isAccessibilityFocused)
        assertTrue(first.actionList.any {
            it.id == AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS
        })
        assertTrue(parent.eventTypes.contains(AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED))

        assertTrue(
            provider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS,
                null
            )
        )
        assertFalse(requireNotNull(provider.createAccessibilityNodeInfo(1)).isAccessibilityFocused)
        assertTrue(
            parent.eventTypes.contains(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED
            )
        )
        assertFalse(
            provider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS,
                null
            )
        )
    }

    @Test
    fun `replacing render clears virtual accessibility focus`() {
        val (viewer, _) = laidOutInteractiveViewer(TargetKind.LINK)
        val parent = AccessibilityEventParent(viewer.context)
        parent.addView(viewer)
        val provider = viewer.accessibilityNodeProvider
        assertTrue(
            provider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null
            )
        )

        viewer.apply(paragraphRenderJson("replacement"), "{}")

        assertTrue(
            parent.eventTypes.contains(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED
            )
        )
        assertTrue(provider.createAccessibilityNodeInfo(1) == null)
    }

    @Test
    fun `replacing render notifies accessibility subtree and updates virtual children`() {
        val (viewer, _) = laidOutInteractiveViewer(TargetKind.LINK)
        val parent = AccessibilityEventParent(viewer.context)
        parent.addView(viewer)
        val provider = viewer.accessibilityNodeProvider
        assertTrue(provider.createAccessibilityNodeInfo(1) != null)

        viewer.apply(paragraphRenderJson("replacement"), "{}")

        assertTrue(provider.createAccessibilityNodeInfo(1) == null)
        val eventIndex = parent.eventTypes.indexOfLast {
            it == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED
        }
        assertTrue(eventIndex >= 0)
        assertEquals(
            AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE,
            parent.contentChangeTypes[eventIndex]
        )
    }

    @Test
    fun `link tap setting notifies accessibility subtree and updates virtual children`() {
        val (viewer, _) = laidOutInteractiveViewer(TargetKind.LINK)
        val parent = AccessibilityEventParent(viewer.context)
        parent.addView(viewer)
        val provider = viewer.accessibilityNodeProvider
        assertTrue(provider.createAccessibilityNodeInfo(1) != null)

        viewer.linkTapsEnabled = false

        assertTrue(provider.createAccessibilityNodeInfo(1) == null)
        val eventIndex = parent.eventTypes.indexOfLast {
            it == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED
        }
        assertTrue(eventIndex >= 0)
        assertEquals(
            AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE,
            parent.contentChangeTypes[eventIndex]
        )

        viewer.linkTapsEnabled = true
        assertTrue(provider.createAccessibilityNodeInfo(1) != null)
        assertTrue(
            parent.eventTypes.count { it == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED } >= 2
        )
    }

    @Test
    fun `crossing adjacent equal links does not activate the up range`() {
        val viewer = ProseViewerView(RuntimeEnvironment.getApplication())
        viewer.apply(
            """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"textRun","text":"i","marks":[{"type":"link","href":"https://example.com/same"}]},
              {"type":"textRun","text":"i","marks":[{"type":"link","href":"https://example.com/same"}]},
              {"type":"blockEnd"}
            ]
            """.trimIndent(),
            "{}"
        )
        val widthSpec = View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        viewer.measure(widthSpec, heightSpec)
        viewer.layout(0, 0, viewer.measuredWidth, viewer.measuredHeight)
        val proseView = viewer.proseViewForTesting
        val text = proseView.text as Spanned
        val links = text.getSpans(0, text.length, Annotation::class.java)
            .filter { it.key == RenderBridge.NATIVE_LINK_HREF_ANNOTATION }
            .sortedBy { text.getSpanStart(it) }
        assertEquals(2, links.size)
        val down = pointForOffset(proseView, text.getSpanStart(links[0]))
        val up = pointForOffset(proseView, text.getSpanStart(links[1]))
        viewer.touchSlopForTesting = 1_000f
        var activations = 0
        viewer.onLinkTapForTesting = { activations += 1 }

        handleProseTouch(viewer, motion(MotionEvent.ACTION_DOWN, down))
        val consumed = handleProseTouch(viewer, motion(MotionEvent.ACTION_UP, up))

        assertFalse(consumed)
        assertEquals(0, activations)
    }

    @Test
    fun `viewer image policy prop reaches prose view`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeProseViewerExpoView(expoContext.context, expoContext.appContext)

        view.setImageLoadingPolicyJson("""{"maxSourceBytes":123}""")

        val proseView = view.viewerForTesting.proseViewForTesting
        assertEquals(123, proseView.imageLoadingPolicy.maxSourceBytes)
    }

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
    ): Pair<ProseViewerView, EditorEditText> {
        val view = ProseViewerView(RuntimeEnvironment.getApplication())
        val targetJson = when (targetKind) {
            TargetKind.LINK ->
                """{"type":"textRun","text":"link target with plenty of width","marks":[{"type":"link","href":"https://example.com/viewer"}]}"""
            TargetKind.MENTION ->
                """{"type":"opaqueInlineAtom","nodeType":"mention","label":"@Alice","docPos":31}"""
        }
        view.apply(
            """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"textRun","text":"plain text ","marks":[]},
              $targetJson,
              {"type":"blockEnd"}
            ]
            """.trimIndent(),
            "{}"
        )
        val widthSpec = View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val proseView = view.proseViewForTesting
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
        viewer: ProseViewerView,
        event: MotionEvent
    ): Boolean {
        val method = ProseViewerView::class.java.getDeclaredMethod(
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
