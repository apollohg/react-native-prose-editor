package com.apollohg.editor.viewer
import android.graphics.Canvas
import android.graphics.Bitmap
import android.graphics.Rect
import android.app.Activity
import android.os.Looper
import android.text.StaticLayout
import android.text.TextPaint
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import com.apollohg.editor.PreparedProseRecyclerHarness
import com.apollohg.editor.PreparedProseBenchmarkConfiguration
import com.apollohg.editor.PreparedProsePerformanceGates
import com.apollohg.editor.ProseViewerConfiguration
import com.apollohg.editor.ProseViewerError
import com.apollohg.editor.ProseViewerErrorCode
import com.apollohg.editor.ProseViewerInteractionListenerAdapter
import com.apollohg.editor.ProseViewerMention
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ProseViewerView
import com.apollohg.editor.OrderedListMarkerSpan
import com.apollohg.editor.RenderBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File
import java.util.concurrent.TimeUnit
import org.json.JSONArray
import org.json.JSONObject

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class PreparedProseLayoutFabricMeasurementTest : PreparedProseLayoutTestFixture() {
    @Test
    fun `Fabric mount only acquires the measured artifact and layout draw do not prepare`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val request = request("Fabric acquisition")
        val surface = FabricSurfaceToken(surfaceId = 41, componentTag = 420)

        val generation = FabricGenerationToken(surface, request.generationIdentity, 1)
        registry.registerFabricLease(surface, generation.leaseHandle)
        registry.measure(request, widthPx = 320, density = 1f, fabricSurface = surface, fabricLeaseHandle = generation.leaseHandle)
        val artifact = registry.acquireForFabricMount(generation, request, widthPx = 320, density = 1f)
        val drawingView = PreparedProseDrawingView(context)
        drawingView.install(artifact)
        drawingView.layout(0, 0, 320, artifact!!.heightPx)
        drawingView.draw(Canvas(Bitmap.createBitmap(320, artifact.heightPx.coerceAtLeast(1), Bitmap.Config.ARGB_8888)))

        assertEquals(1, engine.preparationCount)
    }

    @Test
    fun `Fabric mount accepts a one pixel grid rounding difference and prepares nothing new`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val request = request("pixel grid rounding")
        val measuredWidthPx = 896
        val laidOutWidthPx = 897
        val surface = FabricSurfaceToken(surfaceId = 1, componentTag = 1902)
        val generation = FabricGenerationToken(surface, request.generationIdentity, 2)

        registry.registerFabricLease(surface, generation.leaseHandle)
        val measured = registry.measure(
            request,
            widthPx = measuredWidthPx,
            density = 2.625f,
            fabricSurface = surface,
            fabricLeaseHandle = generation.leaseHandle,
        )
        val mounted = registry.acquireForFabricMount(generation, request, laidOutWidthPx, 2.625f)

        assertTrue(mounted === measured)
        assertEquals(measuredWidthPx, mounted!!.widthPx)
        assertEquals(1, engine.preparationCount)
    }

    @Test
    fun `final Fabric ticket replaces earlier widths with content box geometry`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val request = request("final content box")
        val surface = FabricSurfaceToken(surfaceId = 51, componentTag = 510)
        val generation = FabricGenerationToken(surface, request.generationIdentity, 51)

        registry.registerFabricLease(surface, generation.leaseHandle)
        registry.measure(request, 895, 2.625f, surface, generation.leaseHandle)
        registry.measure(request, 897, 2.625f, surface, generation.leaseHandle)
        val prepared = registry.prepareFinalLayout(
            request = request,
            widthPx = 896,
            density = 2.625f,
            contentOriginXPx = 17,
            contentOriginYPx = 23,
            fabricSurface = surface,
            fabricLeaseHandle = generation.leaseHandle,
        )
        registry.activateFabricGeneration(generation)

        assertEquals(
            null,
            registry.acquirePreparedMountTicket(
                generation,
                expectedNativeFontRevision = request.nativeFontRevision + 1,
            ),
        )
        val ticket = requireNotNull(registry.acquirePreparedMountTicket(generation))
        assertEquals(request.nativeFontRevision, ticket.nativeFontRevision)
        assertEquals(896, ticket.contentWidthPx)
        assertEquals(17, ticket.contentOriginXPx)
        assertEquals(23, ticket.contentOriginYPx)
        assertEquals(2.625f.toRawBits(), ticket.densityBits)
        assertTrue(ticket.artifact === prepared)
        assertEquals(3, engine.preparationCount)
    }

    @Test
    fun `Fabric mount rejects a width beyond the pixel grid rounding slack`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val request = request("beyond pixel grid slack")
        val surface = FabricSurfaceToken(surfaceId = 1, componentTag = 1903)
        val generation = FabricGenerationToken(surface, request.generationIdentity, 2)

        registry.registerFabricLease(surface, generation.leaseHandle)
        registry.measure(request, widthPx = 896, density = 2.625f, fabricSurface = surface, fabricLeaseHandle = generation.leaseHandle)

        assertEquals(null, registry.acquireForFabricMount(generation, request, 894, 2.625f))
        assertEquals(null, registry.acquireForFabricMount(generation, request, 898, 2.625f))
        assertNotNull(registry.acquireForFabricMount(generation, request, 895, 2.625f))
    }

    @Test
    fun `Fabric mount prefers the exactly measured width over a rounding neighbour`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val request = request("exact width preference")
        val surface = FabricSurfaceToken(surfaceId = 1, componentTag = 1904)
        val generation = FabricGenerationToken(surface, request.generationIdentity, 2)

        registry.registerFabricLease(surface, generation.leaseHandle)
        registry.measure(request, widthPx = 895, density = 2.625f, fabricSurface = surface, fabricLeaseHandle = generation.leaseHandle)
        val exact = registry.measure(request, widthPx = 896, density = 2.625f, fabricSurface = surface, fabricLeaseHandle = generation.leaseHandle)

        assertTrue(registry.acquireForFabricMount(generation, request, 896, 2.625f) === exact)
    }

    @Test
    fun `Fabric revision fields produce distinct measurement identities`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val base = request("revisions")
        val requests = listOf(
            base,
            base.copy(attachmentRevision = 1),
            base.copy(nativeFontRevision = 1),
            base.copy(fontEnvironmentRevision = 1),
        )

        requests.forEach { registry.measure(it, widthPx = 320, density = 1f) }

        assertEquals(4, requests.map { it.generationIdentity }.toSet().size)
        assertEquals(4, engine.preparationCount)
    }

    @Test
    fun `oversized Fabric artifacts bypass only the unmounted retained byte budget`() {
        val registry = PreparedProseLayoutRegistry(
            compiler = CountingDocumentCompiler(::testDocument),
            layoutEngine = CountingLayoutEngine(),
            byteBudget = 1,
        )
        val request = request("too large to retain")
        val surface = FabricSurfaceToken(9, 91)
        registry.registerFabricLease(surface, 1)
        registry.measure(request, 320, 1f, surface, fabricLeaseHandle = 1)

        assertEquals(0, registry.layoutRetainedBytesForTesting)
        assertEquals(1, registry.fabricLeaseCountForTesting)
    }
}
