package com.apollohg.editor.viewer

import android.graphics.Rect
import android.view.MotionEvent
import com.facebook.react.bridge.JavaOnlyMap
import com.facebook.react.uimanager.ReactStylesDiffMap
import org.junit.Assert.assertEquals
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
class PreparedProseViewerManagerUnitsTest {
    @Test
    fun `fabric constraints remain physical pixels until yoga output`() {
        val density = 2.625f
        val widthPixels = 891f

        assertEquals(891, fabricConstraintPixels(widthPixels))
        assertEquals(339.42856f, fabricPixelsToDp(widthPixels, density)!!, 0.0001f)
        assertEquals(200f, fabricPixelsToDp(525f, density)!!, 0.0001f)
        assertNull(fabricConstraintPixels(Float.POSITIVE_INFINITY))
        assertNull(fabricPixelsToDp(100f, 0f))
    }

    @Test
    fun `individual Fabric prop callbacks do not reconcile before the transaction boundary`() {
        val manager = PreparedProseViewerManager()
        val view = PreparedProseDrawingView(RuntimeEnvironment.getApplication())
        val mounted = preparedArtifact("mounted")
        view.install(mounted)

        manager.setSource(view, "staged")

        assertSame(mounted, view.preparedLayout)

        manager.updateProperties(
            view,
            ReactStylesDiffMap(JavaOnlyMap.of("source", "committed")),
        )
        assertNull(view.preparedLayout)
    }

    @Test
    fun `drawing view translates interactions and accessibility into the content box`() {
        val view = PreparedProseDrawingView(RuntimeEnvironment.getApplication())
        val artifact = preparedArtifact("content-origin").copy(
            interactions = listOf(
                PreparedProseInteraction(
                    kind = PreparedProseInteraction.Kind.LINK,
                    rects = listOf(Rect(0, 0, 20, 20)),
                    href = "https://example.test",
                    visibleText = "link",
                    label = "link",
                ),
            ),
            accessibilityNodes = listOf(
                PreparedProseAccessibilityNode(
                    interactionIndex = 0,
                    role = PreparedProseAccessibilityNode.Role.LINK,
                    label = "link",
                    bounds = Rect(0, 0, 20, 20),
                ),
            ),
        )
        var activated = false
        view.onInteractionActivated = { activated = true; true }
        view.install(artifact, contentOriginXPx = 11, contentOriginYPx = 13)

        val outside = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 10.5f, 17f, 0)
        assertEquals(false, view.onTouchEvent(outside))
        outside.recycle()
        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, 15f, 17f, 0)
        val up = MotionEvent.obtain(0, 1, MotionEvent.ACTION_UP, 15f, 17f, 0)
        assertTrue(view.onTouchEvent(down))
        assertTrue(view.onTouchEvent(up))
        down.recycle()
        up.recycle()
        assertTrue(activated)

        val bounds = Rect()
        requireNotNull(view.accessibilityNodeProvider.createAccessibilityNodeInfo(1))
            .getBoundsInParent(bounds)
        assertEquals(Rect(11, 13, 31, 33), bounds)
    }

    private fun preparedArtifact(generation: String): PreparedProseLayout = PreparedProseLayout(
        key = ProseLayoutKey(
            semanticKey = generation,
            widthPx = 100,
            themeDigest = "fixture",
            nativeFontRevision = 0,
            fontEnvironmentRevision = 0,
            densityBits = 1f.toRawBits().toLong(),
            attachmentRevision = 0,
            generationIdentity = generation,
        ),
        widthPx = 100,
        heightPx = 20,
        blocks = emptyList(),
        retainedBytes = 0,
    )
}
