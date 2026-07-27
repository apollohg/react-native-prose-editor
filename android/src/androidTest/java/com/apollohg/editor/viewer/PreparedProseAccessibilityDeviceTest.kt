package com.apollohg.editor.viewer

import android.graphics.Rect
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.editor_core.FfiViewerMark

/**
 * Device text shaping owns the stronger contour contract. Robolectric may
 * expose one valid shaped contour for this range, so host tests only assert
 * that a nonempty contour is retained and leave this discontiguity assertion
 * to physical/instrumentation coverage.
 */
@RunWith(AndroidJUnit4::class)
class PreparedProseAccessibilityDeviceTest {
    @Test
    fun shaped_bidi_link_uses_discontiguous_rects_in_visual_order() {
        val document = ViewerDocument(
            semanticKey = "device-bidi-link-fixture",
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
        val layout = StaticLayoutAndroidProseLayoutEngine().prepare(
            document = document,
            key = ProseLayoutKey(
                semanticKey = document.semanticKey,
                widthPx = 300,
                themeDigest = "device-fixture",
                nativeFontRevision = 0,
                fontEnvironmentRevision = 0,
                densityBits = 1f.toRawBits().toLong(),
                attachmentRevision = 0,
                generationIdentity = "device-fixture",
            ),
            theme = PreparedProseTheme.resolve(null, 1f),
            widthPx = 300,
            density = 1f,
            imagesEnabled = false,
        )
        val link = layout.interactions.single { it.kind == PreparedProseInteraction.Kind.LINK }

        assertTrue("device shaping must retain separate visual Bidi contours", link.rects.size >= 2)
        assertEquals(link.rects.sortedWith(compareBy<Rect> { it.top }.thenBy { it.left }), link.rects)
    }

    @Test
    fun shaped_wrapped_link_keeps_discontiguous_rects_before_long_mention() {
        val document = ViewerDocument(
            semanticKey = "device-wrapped-link-fixture",
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
            document = document,
            key = ProseLayoutKey(
                semanticKey = document.semanticKey,
                widthPx = 90,
                themeDigest = "device-fixture",
                nativeFontRevision = 0,
                fontEnvironmentRevision = 0,
                densityBits = 1f.toRawBits().toLong(),
                attachmentRevision = 0,
                generationIdentity = "device-fixture",
            ),
            theme = PreparedProseTheme.resolve(null, 1f),
            widthPx = 90,
            density = 1f,
            imagesEnabled = false,
        )
        val link = layout.interactions.single { it.kind == PreparedProseInteraction.Kind.LINK }
        val mention = layout.interactions.single { it.kind == PreparedProseInteraction.Kind.MENTION }

        assertTrue("device shaping must split wrapped link contours", link.rects.size >= 2)
        assertEquals(link.rects.sortedWith(compareBy<Rect> { it.top }.thenBy { it.left }), link.rects)
        assertEquals(UInt.MAX_VALUE.toLong(), mention.docPos)
    }
}
