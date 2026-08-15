package com.apollohg.editor.viewer

import com.facebook.react.bridge.JavaOnlyMap
import com.facebook.react.uimanager.ReactStylesDiffMap
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
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
