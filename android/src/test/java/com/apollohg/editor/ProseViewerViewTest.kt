package com.apollohg.editor

import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
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

    private fun paragraphRenderJson(text: String): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"$text","marks":[]},
          {"type":"blockEnd"}
        ]
        """.trimIndent()

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
}
