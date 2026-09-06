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
internal class PreparedProseLayoutFabricOwnershipTest : PreparedProseLayoutTestFixture() {
    @Test
    fun `final Fabric ticket rebuilds after cache pressure and rejects a stale generation`() {
        val registry = testRegistry(CountingLayoutEngine())
        val surface = FabricSurfaceToken(surfaceId = 52, componentTag = 520)
        val handle = 52L
        val firstRequest = request("first final ticket")
        val secondRequest = request("second final ticket")
        val first = FabricGenerationToken(surface, firstRequest.generationIdentity, handle)
        val second = FabricGenerationToken(surface, secondRequest.generationIdentity, handle)

        registry.registerFabricLease(surface, handle)
        registry.prepareFinalLayout(firstRequest, 320, 1f, 4, 5, surface, handle)
        registry.prepareFinalLayout(secondRequest, 321, 1f, 6, 7, surface, handle)
        registry.activateFabricGeneration(second)
        registry.didReceiveMemoryWarning()

        assertEquals(null, registry.acquirePreparedMountTicket(first))
        assertEquals(null, registry.acquirePreparedMountTicket(second))
        val completion = java.util.concurrent.CountDownLatch(1)
        var prepared = false
        assertTrue(registry.prepareForFabricMount(second) { succeeded ->
            prepared = succeeded
            completion.countDown()
        })
        assertTrue(completion.await(5, TimeUnit.SECONDS))
        assertTrue(prepared)
        val ticket = requireNotNull(registry.acquirePreparedMountTicket(second))
        assertEquals(321, ticket.contentWidthPx)
        assertEquals(6, ticket.contentOriginXPx)
        assertEquals(7, ticket.contentOriginYPx)
    }

    @Test
    fun `slower final Fabric preparation cannot overwrite newer same-generation geometry`() {
        val firstStarted = java.util.concurrent.CountDownLatch(1)
        val releaseFirst = java.util.concurrent.CountDownLatch(1)
        val delegate = CountingLayoutEngine()
        val engine = object : AndroidProseLayoutEngine {
            override fun prepare(
                document: ViewerDocument,
                key: ProseLayoutKey,
                theme: PreparedProseTheme,
                widthPx: Int,
                density: Float,
                collapsesWhenEmpty: Boolean,
            ): PreparedProseLayout {
                if (widthPx == 320) {
                    firstStarted.countDown()
                    assertTrue(releaseFirst.await(5, TimeUnit.SECONDS))
                }
                return delegate.prepare(document, key, theme, widthPx, density, collapsesWhenEmpty)
            }
        }
        val registry = testRegistry(engine)
        val request = request("same generation race")
        val surface = FabricSurfaceToken(53, 530)
        val generation = FabricGenerationToken(surface, request.generationIdentity, 53)
        registry.registerFabricLease(surface, generation.leaseHandle)

        val older = java.util.concurrent.CompletableFuture.supplyAsync {
            registry.prepareFinalLayout(request, 320, 1f, 3, 4, surface, generation.leaseHandle)
        }
        assertTrue(firstStarted.await(5, TimeUnit.SECONDS))
        registry.prepareFinalLayout(request, 321, 1f, 7, 8, surface, generation.leaseHandle)
        releaseFirst.countDown()
        older.get(5, TimeUnit.SECONDS)
        registry.activateFabricGeneration(generation)

        val ticket = requireNotNull(registry.acquirePreparedMountTicket(generation))
        assertEquals(321, ticket.contentWidthPx)
        assertEquals(7, ticket.contentOriginXPx)
        assertEquals(8, ticket.contentOriginYPx)
    }

    @Test
    fun `Fabric surface stop clears every measured generation pin and lease`() {
        val registry = testRegistry(CountingLayoutEngine())
        val first = FabricSurfaceToken(7, 71)
        val second = FabricSurfaceToken(7, 72)
        registry.registerFabricLease(first, 1)
        registry.registerFabricLease(second, 2)
        registry.measure(request("first"), 320, 1f, first, fabricLeaseHandle = 1)
        registry.measure(request("second"), 320, 1f, second, fabricLeaseHandle = 2)

        registry.deactivateFabricSurfaceId(7)

        assertEquals(0, registry.fabricGenerationPinCountForTesting)
        assertEquals(0, registry.fabricLeaseCountForTesting)
        // Surface stop leaves bounded inactive family records until their C++
        // guards terminate. A delayed H1 cannot recreate ownership.
        registry.measure(request("late"), 320, 1f, first, fabricLeaseHandle = 1)
        assertEquals(0, registry.fabricLeaseCountForTesting)
        registry.finalizeFabricLease(first, 1)
        registry.registerFabricLease(first, 3)
        registry.measure(request("fresh"), 320, 1f, first, fabricLeaseHandle = 3)
        assertTrue(registry.fabricLeaseCountForTesting > 0)
    }

    @Test
    fun `Fabric mount miss releases the exact generation pin and lease`() {
        val registry = testRegistry(CountingLayoutEngine())
        val request = request("mount miss")
        val surface = FabricSurfaceToken(8, 81)
        val generation = FabricGenerationToken(surface, request.generationIdentity, 1)
        registry.registerFabricLease(surface, generation.leaseHandle)
        registry.measure(request, 320, 1f, surface, fabricLeaseHandle = generation.leaseHandle)

        assertEquals(null, registry.acquireForFabricMount(generation, request, 330, 1f))
        registry.releaseFabricMountMiss(generation, 320, 1f)

        assertEquals(0, registry.fabricGenerationPinCountForTesting)
        assertEquals(0, registry.fabricLeaseCountForTesting)
    }

    @Test
    fun `Fabric leases retain mounted handoffs until their surface releases them`() {
        val registry = testRegistry(CountingLayoutEngine())
        repeat(33) { index ->
            val surface = FabricSurfaceToken(10, 100 + index)
            registry.registerFabricLease(surface, index + 1L)
            registry.measure(
                request("lease $index"),
                320,
                1f,
                surface,
                fabricLeaseHandle = index + 1L,
            )
        }

        assertEquals(33, registry.fabricLeaseCountForTesting)
        registry.deactivateFabricSurfaceId(10)
        assertEquals(0, registry.fabricLeaseCountForTesting)
    }

    @Test
    fun `Fabric mount requires its exact pending lease handle and stale H1 cannot disturb H2`() {
        val registry = testRegistry(CountingLayoutEngine())
        val request = request("exact lease")
        val surface = FabricSurfaceToken(12, 120)
        val h1 = FabricGenerationToken(surface, request.generationIdentity, 1)
        val h2 = FabricGenerationToken(surface, request.generationIdentity, 2)

        registry.registerFabricLease(surface, h1.leaseHandle)
        registry.registerFabricLease(surface, h2.leaseHandle)
        registry.measure(request, 320, 1f, surface, fabricLeaseHandle = h1.leaseHandle)
        registry.measure(request, 320, 1f, surface, fabricLeaseHandle = h2.leaseHandle)

        assertEquals(null, registry.acquireForFabricMount(FabricGenerationToken(surface, request.generationIdentity, 3), request, 320, 1f))
        registry.releaseFabricMountMiss(h1, 320, 1f)
        registry.measure(request, 0, 1f, surface, fabricLeaseHandle = h1.leaseHandle)

        assertNotNull(registry.acquireForFabricMount(h2, request, 320, 1f))
        assertEquals(1, registry.fabricLeaseCountForTesting)
    }

    @Test
    fun `Fabric invalid width retires only exact pending ownership`() {
        val registry = testRegistry(CountingLayoutEngine())
        val request = request("invalid H1")
        val surface = FabricSurfaceToken(13, 130)
        val h1 = FabricGenerationToken(surface, request.generationIdentity, 1)
        val h2 = FabricGenerationToken(surface, request.generationIdentity, 2)

        registry.registerFabricLease(surface, h1.leaseHandle)
        registry.registerFabricLease(surface, h2.leaseHandle)
        registry.measure(request, 320, 1f, surface, fabricLeaseHandle = h1.leaseHandle)
        registry.measure(request, 320, 1f, surface, fabricLeaseHandle = h2.leaseHandle)
        registry.measure(request, 0, 1f, surface, fabricLeaseHandle = h1.leaseHandle)

        assertNotNull(registry.acquireForFabricMount(h2, request, 320, 1f))
    }

    @Test
    fun `released never-mounted H1 cannot recreate Android sidecars pins or leases`() {
        val registry = testRegistry(CountingLayoutEngine())
        val request = request("terminal H1")
        val surface = FabricSurfaceToken(14, 140)
        val h1 = FabricGenerationToken(surface, request.generationIdentity, 1)
        val h2 = FabricGenerationToken(surface, request.generationIdentity, 2)

        registry.registerFabricLease(surface, h1.leaseHandle)
        registry.measure(request, 320, 1f, surface, h1.leaseHandle)
        assertNotNull(FabricAttachmentSidecars.state(h1))
        assertEquals(1, registry.fabricLeaseCountForTesting)
        assertEquals(1, registry.fabricGenerationPinCountForTesting)

        registry.deactivateFabricLease(surface, h1.leaseHandle)
        assertEquals(null, FabricAttachmentSidecars.state(h1))
        assertEquals(0, registry.fabricLeaseCountForTesting)
        assertEquals(0, registry.fabricGenerationPinCountForTesting)
        // Java recycle keeps this inactive guard until C++ destroys the
        // state-family. A delayed Yoga callback must not revive it.
        assertEquals(1, registry.activeFabricLeaseCountForTesting)

        registry.measure(request, 320, 1f, surface, h1.leaseHandle)
        assertEquals(0, registry.fabricLeaseCountForTesting)
        assertEquals(0, registry.fabricGenerationPinCountForTesting)
        assertEquals(null, FabricAttachmentSidecars.state(h1))

        registry.registerFabricLease(surface, h2.leaseHandle)
        registry.measure(request, 320, 1f, surface, h2.leaseHandle)
        assertNotNull(registry.acquireForFabricMount(h2, request, 320, 1f))

        registry.finalizeFabricLease(surface, h1.leaseHandle)
        assertEquals(1, registry.activeFabricLeaseCountForTesting)
    }

    @Test
    fun `Fabric commit permits only its canonical generation for one family handle`() {
        val registry = testRegistry(CountingLayoutEngine())
        val first = request("first committed revision")
        val second = request("second committed revision")
        val surface = FabricSurfaceToken(42, 420)
        val handle = 42L
        val g1 = FabricGenerationToken(surface, first.generationIdentity, handle)
        val g2 = FabricGenerationToken(surface, second.generationIdentity, handle)

        registry.registerFabricLease(surface, handle)
        // Yoga may finish both pre-commit measurements; the component commit
        // selects G2 without cancelling its own already-created handoff.
        registry.measure(first, 320, 1f, surface, handle)
        registry.measure(second, 320, 1f, surface, handle)
        registry.activateFabricGeneration(g2)

        assertEquals(g2.generationIdentity, registry.permittedFabricGenerationForTesting(FabricLeaseOwner(surface, handle)))
        assertEquals(null, registry.acquireForFabricMount(g1, first, 320, 1f))
        assertNotNull(registry.acquireForFabricMount(g2, second, 320, 1f))

        // A delayed G1 Yoga callback cannot recreate its sidecar or lease.
        registry.measure(first, 320, 1f, surface, handle)
        assertEquals(null, FabricAttachmentSidecars.state(g1))
    }

    @Test
    fun `committed Fabric generation does not collapse prospective measurement`() {
        val registry = testRegistry(CountingLayoutEngine())
        val first = request("first")
        val second = request("second")
        val surface = FabricSurfaceToken(44, 440)
        val handle = 44L
        val g1 = FabricGenerationToken(surface, first.generationIdentity, handle)
        val g2 = FabricGenerationToken(surface, second.generationIdentity, handle)

        registry.registerFabricLease(surface, handle)
        assertTrue(registry.measure(first, 320, 1f, surface, handle).heightPx > 0)
        registry.activateFabricGeneration(g1)
        assertNotNull(registry.acquireForFabricMount(g1, first, 320, 1f))

        assertTrue(registry.measure(second, 320, 1f, surface, handle).heightPx > 0)
        registry.activateFabricGeneration(g2)
        assertNotNull(registry.acquireForFabricMount(g2, second, 320, 1f))
    }

    @Test
    fun `terminal Fabric owner sweep removes retained G1 and pending G2 exactly once`() {
        val registry = testRegistry(CountingLayoutEngine())
        val first = request("mounted G1")
        val second = request("failed G2")
        val surface = FabricSurfaceToken(43, 430)
        val isolatedSurface = FabricSurfaceToken(43, 431)
        val handle = 43L
        val isolatedHandle = 44L
        val g1 = FabricGenerationToken(surface, first.generationIdentity, handle)
        val g2 = FabricGenerationToken(surface, second.generationIdentity, handle)
        val isolated = FabricGenerationToken(isolatedSurface, first.generationIdentity, isolatedHandle)
        registry.registerFabricLease(surface, handle)
        registry.registerFabricLease(isolatedSurface, isolatedHandle)

        registry.measure(first, 320, 1f, surface, handle)
        assertNotNull(registry.acquireForFabricMount(g1, first, 320, 1f))
        registry.activateFabricGeneration(g2)
        registry.measure(second, 280, 1f, surface, handle)
        registry.measure(first, 320, 1f, isolatedSurface, isolatedHandle)

        assertEquals(3, registry.fabricLeaseCountForTesting)
        registry.deactivateFabricLease(surface, handle)

        assertEquals(1, registry.fabricLeaseCountForTesting)
        assertEquals(1, registry.fabricGenerationPinCountForTesting)
        assertEquals(null, FabricAttachmentSidecars.state(g1))
        assertEquals(null, FabricAttachmentSidecars.state(g2))
        assertNotNull(FabricAttachmentSidecars.state(isolated))
        assertNotNull(registry.acquireForFabricMount(isolated, first, 320, 1f))

        // The family guard terminal callback is idempotent after view release.
        registry.finalizeFabricLease(surface, handle)
        assertEquals(1, registry.fabricLeaseCountForTesting)
        assertEquals(1, registry.fabricGenerationPinCountForTesting)
    }

    @Test
    fun `Java terminal cleanup keeps H1 inactive until the C++ lifecycle finalizes it`() {
        val registry = testRegistry(CountingLayoutEngine())
        val first = request("recycled H1")
        val second = request("unaffected H2")
        val h1Surface = FabricSurfaceToken(46, 460)
        val h2Surface = FabricSurfaceToken(47, 470)
        val h1 = FabricGenerationToken(h1Surface, first.generationIdentity, 46)
        val h2 = FabricGenerationToken(h2Surface, second.generationIdentity, 47)

        registry.registerFabricLease(h1Surface, h1.leaseHandle)
        registry.registerFabricLease(h2Surface, h2.leaseHandle)
        registry.measure(first, 320, 1f, h1Surface, h1.leaseHandle)
        registry.measure(second, 320, 1f, h2Surface, h2.leaseHandle)
        assertNotNull(registry.acquireForFabricMount(h2, second, 320, 1f))

        // View recycle and surface stop sweep H1 resources but retain the
        // inactive family record. A delayed C++ bind/measure cannot recreate it.
        registry.deactivateFabricLease(h1Surface, h1.leaseHandle)
        registry.deactivateFabricSurfaceId(h1Surface.surfaceId)
        registry.registerFabricLease(h1Surface, h1.leaseHandle)
        registry.measure(first, 320, 1f, h1Surface, h1.leaseHandle)

        assertEquals(null, FabricAttachmentSidecars.state(h1))
        assertEquals(null, registry.acquireForFabricMount(h1, first, 320, 1f))
        assertEquals(1, registry.fabricLeaseCountForTesting)
        assertEquals(1, registry.fabricGenerationPinCountForTesting)
        assertEquals(2, registry.activeFabricLeaseCountForTesting)
        assertNotNull(FabricAttachmentSidecars.state(h2))

        // This simulates PreparedProseViewerLeaseLifecycle's last-owner
        // destructor callback. It is idempotent and removes the bounded guard.
        registry.finalizeFabricLease(h1Surface, h1.leaseHandle)
        registry.finalizeFabricLease(h1Surface, h1.leaseHandle)

        assertEquals(1, registry.activeFabricLeaseCountForTesting)
        assertEquals(1, registry.fabricGenerationPinCountForTesting)
        assertEquals(1, registry.fabricLeaseCountForTesting)
        assertNotNull(FabricAttachmentSidecars.state(h2))
    }

    @Test
    fun `live exact artifact is shared across Fabric owners after cache eviction`() {
        val cache = PreparedProseLayoutCache(byteBudget = 100, pendingLeaseBudget = 2)
        val key = testLayoutKey("shared")
        val artifact = testArtifact(key, retainedBytes = 80)
        val first = FabricGenerationToken(FabricSurfaceToken(15, 151), key.generationIdentity, 1)
        val second = FabricGenerationToken(FabricSurfaceToken(15, 152), key.generationIdentity, 2)

        assertTrue(cache.value(key, first) { artifact } === artifact)
        assertTrue(cache.acquireForFabricMount(first, key.widthPx, key.densityBits) === artifact)
        cache.removeAllUnmounted()

        assertTrue(cache.value(key, second) { error("live owner must be reused") } === artifact)
        assertEquals(80, cache.retainedLeaseBytesForTesting)
        assertTrue(cache.acquireForFabricMount(second, key.widthPx, key.densityBits) === artifact)
        assertEquals(80, cache.retainedLeaseBytesForTesting)

        cache.releaseLease(first)
        cache.releaseLease(second)
        cache.registerDirectMount("direct", artifact)
        assertTrue(cache.value(key) { error("direct owner must be reused") } === artifact)
    }

    @Test
    fun `terminal cleanup cannot be followed by a stale Fabric mount publication`() {
        val cache = PreparedProseLayoutCache()
        val key = testLayoutKey("terminal acquisition race")
        val generation = FabricGenerationToken(
            FabricSurfaceToken(54, 540),
            key.generationIdentity,
            54,
        )
        val owner = FabricLeaseOwner(generation.surface, generation.leaseHandle)
        cache.value(key, generation) { testArtifact(key, retainedBytes = 1) }
        val predicateEntered = java.util.concurrent.CountDownLatch(1)
        val releasePredicate = java.util.concurrent.CountDownLatch(1)
        val cleanupStarted = java.util.concurrent.CountDownLatch(1)
        val active = java.util.concurrent.atomic.AtomicBoolean(true)

        val acquisition = java.util.concurrent.CompletableFuture.supplyAsync {
            cache.acquireForFabricMount(generation, key.widthPx, key.densityBits) {
                predicateEntered.countDown()
                assertTrue(releasePredicate.await(5, TimeUnit.SECONDS))
                active.get()
            }
        }
        assertTrue(predicateEntered.await(5, TimeUnit.SECONDS))
        val cleanup = java.util.concurrent.CompletableFuture.runAsync {
            active.set(false)
            cleanupStarted.countDown()
            cache.releaseOwner(owner)
        }
        assertTrue(cleanupStarted.await(5, TimeUnit.SECONDS))
        releasePredicate.countDown()

        assertEquals(null, acquisition.get(5, TimeUnit.SECONDS))
        cleanup.get(5, TimeUnit.SECONDS)
        assertFalse(cache.hasLease(generation))
    }

    @Test
    fun `pending Fabric leases are bounded without evicting the current handoff`() {
        val cache = PreparedProseLayoutCache(byteBudget = 100, pendingLeaseBudget = 2)
        val generations = (1L..3L).map { handle ->
            FabricGenerationToken(FabricSurfaceToken(16, 160 + handle.toInt()), "pending-$handle", handle)
        }
        val keys = generations.map { generation -> testLayoutKey(generation.generationIdentity) }

        keys.zip(generations).forEach { (key, generation) ->
            cache.value(key, generation) { testArtifact(key, retainedBytes = 50) }
        }

        assertEquals(2, cache.pendingLeaseCountForTesting)
        assertTrue(cache.acquireForFabricMount(generations.last(), keys.last().widthPx, keys.last().densityBits) != null)
    }

    @Test
    fun `pending entry cap evicts duplicate metadata without touching mounted or preferred owners`() {
        val cache = PreparedProseLayoutCache(byteBudget = 1, pendingLeaseBudget = 2)
        val key = testLayoutKey("shared duplicate")
        val artifact = testArtifact(key, retainedBytes = 80)
        val surface = FabricSurfaceToken(18, 180)
        val mounted = FabricGenerationToken(surface, key.generationIdentity, 1)
        val firstPending = FabricGenerationToken(surface, key.generationIdentity, 2)
        val secondPending = FabricGenerationToken(surface, key.generationIdentity, 3)
        val preferred = FabricGenerationToken(surface, key.generationIdentity, 4)

        assertTrue(cache.value(key, mounted) { artifact } === artifact)
        assertTrue(cache.acquireForFabricMount(mounted, key.widthPx, key.densityBits) === artifact)
        listOf(firstPending, secondPending, preferred).forEach { generation ->
            assertTrue(cache.value(key, generation) { error("live artifact must be reused") } === artifact)
        }

        assertEquals(2, cache.pendingLeaseCountForTesting)
        assertEquals(3, cache.leaseCountForTesting)
        assertEquals(null, cache.acquireForFabricMount(firstPending, key.widthPx, key.densityBits))
        assertTrue(cache.acquireForFabricMount(preferred, key.widthPx, key.densityBits) === artifact)
    }

    @Test
    fun `entry cap evicts old duplicate handoff while preserving current oversized owner`() {
        val cache = PreparedProseLayoutCache(byteBudget = 1, pendingLeaseBudget = 1)
        val sharedKey = testLayoutKey("mounted duplicate")
        val shared = testArtifact(sharedKey, retainedBytes = 80)
        val mountedOwner = FabricGenerationToken(FabricSurfaceToken(17, 171), sharedKey.generationIdentity, 1)
        val pendingOwner = FabricGenerationToken(FabricSurfaceToken(17, 172), sharedKey.generationIdentity, 2)
        val oversizedKey = testLayoutKey("oversized pending")
        val oversizedOwner = FabricGenerationToken(FabricSurfaceToken(17, 173), oversizedKey.generationIdentity, 3)

        assertTrue(cache.value(sharedKey, mountedOwner) { shared } === shared)
        assertTrue(cache.acquireForFabricMount(mountedOwner, sharedKey.widthPx, sharedKey.densityBits) === shared)
        assertTrue(cache.value(sharedKey, pendingOwner) { error("mounted artifact must be reused") } === shared)
        cache.value(oversizedKey, oversizedOwner) { testArtifact(oversizedKey, retainedBytes = 80) }

        // Removing pendingOwner frees no bytes, but metadata pressure still
        // bounds it. The mounted owner remains intact and the current pending
        // handoff is preferred.
        assertEquals(null, cache.acquireForFabricMount(pendingOwner, sharedKey.widthPx, sharedKey.densityBits))
        assertTrue(cache.acquireForFabricMount(oversizedOwner, oversizedKey.widthPx, oversizedKey.densityBits) != null)
    }

    @Test
    fun `Fabric error reporting is once per generation`() {
        val reporter = FabricErrorReporter()

        assertTrue(reporter.shouldReport("first"))
        assertFalse(reporter.shouldReport("first"))
        assertTrue(reporter.shouldReport("replacement"))
        assertFalse(reporter.shouldReport("replacement"))
    }
}
