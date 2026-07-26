package com.apollohg.editor

import android.view.View
import android.view.MotionEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityEvent
import android.view.ViewGroup
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ProseViewerViewTest {
    private val context
        get() = RuntimeEnvironment.getApplication()

    @Test
    fun `valid render applies and invalid render clears content`() {
        val viewer = ProseViewerView(context)

        assertTrue(viewer.apply(paragraphRenderJson("Hello"), "{}"))
        assertEquals("Hello", viewer.renderedTextForTesting)

        assertFalse(viewer.apply("""{"type":"textRun"}""", "{}"))
        assertEquals("", viewer.renderedTextForTesting)
        assertFalse(viewer.apply("[1]", "{}"))
    }

    @Test
    fun `live and headless measurements use pixels and reject invalid input`() {
        val viewer = ProseViewerView(context)
        viewer.apply(paragraphRenderJson("Measured content"), "{}")

        assertTrue(viewer.measuredHeightForWidth(600) > 0)
        assertEquals(0, viewer.measuredHeightForWidth(0))
        assertTrue(
            requireNotNull(
                ProseViewerView.measureHeight(
                    context,
                    paragraphRenderJson("Measured content"),
                    "{}",
                    600
                )
            ) > 0
        )
        assertNull(ProseViewerView.measureHeight(context, "invalid", "{}", 600))
    }

    @Test
    fun `prepare for reuse restores native defaults and retains listener`() {
        val viewer = ProseViewerView(context)
        val listener = RecordingListener()
        viewer.interactionListener = listener
        viewer.setCollapsesWhenEmpty(true)
        viewer.setImageLoadingPolicyJson("""{"maxSourceBytes":123}""")
        viewer.apply(paragraphRenderJson("Before reuse"), "{}")

        viewer.prepareForReuse()

        assertSame(listener, viewer.interactionListener)
        assertEquals("", viewer.renderedTextForTesting)
        assertFalse(viewer.isContentCollapsedForHost)
        assertEquals(
            ImageLoadingPolicy.DEFAULT.maxSourceBytes,
            viewer.proseViewForTesting.imageLoadingPolicy.maxSourceBytes
        )
    }

    @Test
    fun `public image policy setter reaches bounded image loader`() {
        val viewer = ProseViewerView(context)
        viewer.setImageLoadingPolicyJson("""{"maxSourceBytes":321}""")
        assertEquals(321, viewer.proseViewForTesting.imageLoadingPolicy.maxSourceBytes)
    }

    @Test
    fun `accessibility activation reaches public mention listener`() {
        val viewer = ProseViewerView(context)
        val listener = RecordingListener()
        viewer.interactionListener = listener
        viewer.apply(
            """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"opaqueInlineAtom","nodeType":"mention","label":"@Alice","docPos":31},
              {"type":"blockEnd"}
            ]
            """.trimIndent(),
            "{}"
        )
        val widthSpec = View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        viewer.measure(widthSpec, heightSpec)
        viewer.layout(0, 0, viewer.measuredWidth, viewer.measuredHeight)

        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null
            )
        )
        assertEquals(listOf(31L to "@Alice"), listener.mentions)
    }

    @Test
    fun `legacy mention accessibility preserves unsigned positions above signed int max`() {
        val viewer = ProseViewerView(context)
        val listener = RecordingListener()
        viewer.interactionListener = listener
        viewer.apply(
            """
            [
              {"type":"blockStart","nodeType":"paragraph","depth":0},
              {"type":"opaqueInlineAtom","nodeType":"mention","label":"@Ada","docPos":4294967295},
              {"type":"blockEnd"}
            ]
            """.trimIndent(),
            "{}"
        )
        measureAndLayout(viewer)

        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null,
            )
        )
        assertEquals(listOf(UInt.MAX_VALUE.toLong() to "@Ada"), listener.mentions)
    }

    @Test
    fun `disabled public links are absent from accessibility and cannot activate`() {
        val viewer = ProseViewerView(context)
        viewer.apply(linkRenderJson(), "{}")
        measureAndLayout(viewer)

        assertTrue(viewer.accessibilityNodeProvider.createAccessibilityNodeInfo(1) != null)
        viewer.linkTapsEnabled = false

        assertNull(viewer.accessibilityNodeProvider.createAccessibilityNodeInfo(1))
        assertFalse(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null,
            )
        )
    }

    @Test
    fun `touch slop cancels public link activation`() {
        val viewer = ProseViewerView(context)
        val listener = RecordingListener()
        viewer.interactionListener = listener
        viewer.apply(linkRenderJson(), "{}")
        measureAndLayout(viewer)
        val bounds = requireNotNull(viewer.proseViewForTesting.accessibleAnnotations().firstOrNull()).bounds
        val x = bounds.centerX().toFloat()
        val y = bounds.centerY().toFloat()

        viewer.proseViewForTesting.dispatchTouchEvent(MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, x, y, 0))
        viewer.proseViewForTesting.dispatchTouchEvent(MotionEvent.obtain(0, 1, MotionEvent.ACTION_MOVE, x + viewer.touchSlopForTesting + 1, y, 0))
        viewer.proseViewForTesting.dispatchTouchEvent(MotionEvent.obtain(0, 2, MotionEvent.ACTION_UP, x + viewer.touchSlopForTesting + 1, y, 0))

        assertTrue(listener.links.isEmpty())
    }

    @Test
    fun `virtual focus clears when public host recycles`() {
        val viewer = ProseViewerView(context)
        val parent = CapturingAccessibilityParent().apply { addView(viewer) }
        viewer.apply(linkRenderJson(), "{}")
        measureAndLayout(viewer)

        assertTrue(viewer.accessibilityNodeProvider.performAction(1, AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, null))
        viewer.prepareForReuse()

        assertFalse(viewer.accessibilityNodeProvider.performAction(1, AccessibilityNodeInfo.ACTION_CLEAR_ACCESSIBILITY_FOCUS, null))
        assertTrue(parent.events.any { it.type == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUSED })
        assertTrue(parent.events.any { it.type == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED })
        assertEquals(1, parent.subtreeChangeCount())
    }

    private fun paragraphRenderJson(text: String): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"$text","marks":[]},
          {"type":"blockEnd"}
        ]
        """.trimIndent()

    private fun linkRenderJson(): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Open","marks":[{"type":"link","href":"https://example.test"}]},
          {"type":"blockEnd"}
        ]
        """.trimIndent()

    private fun measureAndLayout(viewer: ProseViewerView) {
        val widthSpec = View.MeasureSpec.makeMeasureSpec(800, View.MeasureSpec.EXACTLY)
        val heightSpec = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        viewer.measure(widthSpec, heightSpec)
        viewer.layout(0, 0, viewer.measuredWidth, viewer.measuredHeight)
    }

    private class RecordingListener : ProseViewerInteractionListener {
        val links = mutableListOf<Pair<String, String>>()
        val mentions = mutableListOf<Pair<Long, String>>()

        override fun onLinkTap(view: ProseViewerView, href: String, text: String) {
            links += href to text
        }

        override fun onMentionTap(
            view: ProseViewerView,
            docPos: Long,
            label: String
        ) {
            mentions += docPos to label
        }
    }

    private inner class CapturingAccessibilityParent : ViewGroup(context) {
        data class Event(val type: Int, val changeTypes: Int)
        val events = mutableListOf<Event>()

        override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            events += Event(event.eventType, event.contentChangeTypes)
            return true
        }

        fun subtreeChangeCount(): Int = events.count { event ->
            event.type == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED &&
                event.changeTypes == AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
        }

        override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) = Unit
    }
}
