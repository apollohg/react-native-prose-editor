package com.apollohg.editor.viewer

import android.graphics.Rect
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.editor_core.FfiViewerMark

class PreparedProseAccessibilityTest {
    @Test
    fun `selection fragments merge only touching pieces on one visual line`() {
        assertEquals(
            listOf(Rect(2, 10, 18, 20)),
            mergeAdjacentSameLineSelectionFragments(
                listOf(Rect(2, 10, 10, 20), Rect(11, 10, 18, 20))
            )
        )
    }

    @Test
    fun `selection fragments preserve real gaps and separate visual lines`() {
        assertEquals(
            listOf(Rect(2, 10, 10, 20), Rect(14, 10, 22, 20), Rect(0, 30, 8, 40)),
            mergeAdjacentSameLineSelectionFragments(
                listOf(Rect(14, 10, 22, 20), Rect(0, 30, 8, 40), Rect(2, 10, 10, 20))
            )
        )
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
}
