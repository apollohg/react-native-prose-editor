package com.apollohg.editor.viewer

import android.graphics.Rect
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import uniffi.editor_core.FfiViewerMark

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedProseAccessibilityTest {
    @Test
    fun `selection fragments merge edge-touching pieces on one visual line`() {
        assertEquals(
            listOf(Rect(2, 10, 18, 20)),
            mergeAdjacentSameLineSelectionFragments(
                listOf(Rect(2, 10, 10, 20), Rect(10, 10, 18, 20))
            )
        )
    }

    @Test
    fun `selection fragments preserve one pixel gaps and separate visual lines`() {
        assertEquals(
            listOf(Rect(2, 10, 10, 20), Rect(11, 10, 18, 20), Rect(0, 30, 8, 40)),
            mergeAdjacentSameLineSelectionFragments(
                listOf(Rect(11, 10, 18, 20), Rect(0, 30, 8, 40), Rect(2, 10, 10, 20))
            )
        )
    }

    @Test
    fun `Fabric-equivalent prepared replacement announces one subtree change and preserves focus clear`() {
        val context = RuntimeEnvironment.getApplication()
        val view = PreparedProseDrawingView(context)
        val parent = CapturingAccessibilityParent(context).apply { addView(view) }
        view.install(preparedArtifact("first"))
        assertTrue(
            view.accessibilityNodeProvider.performAction(
                1,
                android.view.accessibility.AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )

        parent.clearEvents()
        view.install(null, announceAccessibilitySubtree = false)
        view.install(preparedArtifact("replacement"))

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )

        parent.clearEvents()
        val installedArtifact = view.preparedLayout
        view.linkInteractionsEnabled = false
        assertEquals(1, parent.subtreeChangeCount())
        assertTrue(view.preparedLayout === installedArtifact)

        parent.clearEvents()
        view.install(null)
        assertEquals(1, parent.subtreeChangeCount())
    }

    @Test
    fun `bidi link uses discontiguous shaped selection rects in visual order`() {
        val document = ViewerDocument(
            semanticKey = "bidi-link-fixture",
            blocks = listOf(
                ViewerBlock(
                    nodeType = "paragraph",
                    depth = 0,
                    inBlockquote = false,
                    listContext = null,
                    listItemBoundary = null,
                    inlines = listOf(
                        ViewerInline.Text(
                            "Latin \u05e2\u05d1\u05e8\u05d9\u05ea Latin",
                            listOf(FfiViewerMark("link", "{\"href\":\"https://example.test/bidi\"}")),
                        ),
                    ),
                ),
            ),
            isEmpty = false,
            retainedBytes = 64,
        )

        val link = StaticLayoutAndroidProseLayoutEngine().prepare(
            document,
            key(document),
            PreparedProseTheme.resolve(null, 1f),
            300,
            1f,
            false,
        ).interactions.single { it.kind == PreparedProseInteraction.Kind.LINK }

        assertTrue(link.rects.size >= 2)
        assertEquals(link.rects.sortedWith(compareBy<android.graphics.Rect> { it.top }.thenBy { it.left }), link.rects)
    }

    @Test
    fun `prepared geometry freezes wrapped links and long-safe mentions in reading order`() {
        val document = ViewerDocument(
            semanticKey = "interaction-fixture",
            blocks = listOf(
                ViewerBlock(
                    nodeType = "paragraph",
                    depth = 0,
                    inBlockquote = false,
                    listContext = null,
                    listItemBoundary = null,
                    inlines = listOf(
                        ViewerInline.Text(
                            "linked ".repeat(12),
                            listOf(FfiViewerMark("link", "{\"href\":\"https://example.test/wrapped\"}")),
                        ),
                        ViewerInline.Atom("mention", UInt.MAX_VALUE.toLong(), "{}", "@Ada"),
                    ),
                ),
            ),
            isEmpty = false,
            retainedBytes = 64,
        )
        val layout = StaticLayoutAndroidProseLayoutEngine().prepare(
            document,
            key(document),
            PreparedProseTheme.resolve(null, 1f),
            90,
            1f,
            false,
        )

        assertEquals(listOf(PreparedProseInteraction.Kind.LINK, PreparedProseInteraction.Kind.MENTION), layout.interactions.map { it.kind })
        assertEquals("https://example.test/wrapped", layout.interactions.first().href)
        assertTrue(layout.interactions.first().rects.size >= 2)
        assertEquals(UInt.MAX_VALUE.toLong(), layout.interactions.last().docPos)
        assertEquals(listOf(PreparedProseAccessibilityNode.Role.LINK, PreparedProseAccessibilityNode.Role.MENTION), layout.accessibilityNodes.map { it.role })
        assertTrue(layout.retainedBytes > document.retainedBytes)
    }

    private fun key(document: ViewerDocument) = ProseLayoutKey(
        semanticKey = document.semanticKey,
        widthPx = 90,
        themeDigest = "fixture",
        fontRevision = 0,
        densityBits = 1f.toRawBits().toLong(),
        attachmentRevision = 0,
        generationIdentity = "fixture",
    )

    private fun preparedArtifact(generation: String): PreparedProseLayout = PreparedProseLayout(
        key = ProseLayoutKey(
            semanticKey = generation,
            widthPx = 100,
            themeDigest = "fixture",
            fontRevision = 0,
            densityBits = 1f.toRawBits().toLong(),
            attachmentRevision = 0,
            generationIdentity = generation,
        ),
        widthPx = 100,
        heightPx = 20,
        blocks = emptyList(),
        interactions = listOf(
            PreparedProseInteraction(
                kind = PreparedProseInteraction.Kind.LINK,
                rects = listOf(Rect(0, 0, 20, 20)),
                href = "https://example.test/$generation",
                visibleText = generation,
                label = generation,
            ),
        ),
        accessibilityNodes = listOf(
            PreparedProseAccessibilityNode(
                interactionIndex = 0,
                role = PreparedProseAccessibilityNode.Role.LINK,
                label = generation,
                bounds = Rect(0, 0, 20, 20),
            ),
        ),
        retainedBytes = 0,
    )

    private class CapturingAccessibilityParent(context: android.content.Context) : ViewGroup(context) {
        val eventTypes = mutableListOf<Int>()
        private val changeTypes = mutableListOf<Int>()

        override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            eventTypes += event.eventType
            changeTypes += event.contentChangeTypes
            return true
        }

        fun clearEvents() {
            eventTypes.clear()
            changeTypes.clear()
        }

        fun subtreeChangeCount(): Int = eventTypes.indices.count { index ->
            eventTypes[index] == AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED &&
                changeTypes[index] == AccessibilityEvent.CONTENT_CHANGE_TYPE_SUBTREE
        }

        override fun onLayout(changed: Boolean, left: Int, top: Int, right: Int, bottom: Int) = Unit
    }
}
