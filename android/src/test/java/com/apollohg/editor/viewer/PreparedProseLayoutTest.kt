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
import com.apollohg.editor.ProseViewerSource
import com.apollohg.editor.ProseViewerView
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
class PreparedProseLayoutTest {
    private val context
        get() = RuntimeEnvironment.getApplication()

    @Test
    fun windowedRecyclerWarmRevisitUsesOnePrimePreparation() {
        val corpus = JSONObject(context.assets.open("viewer-performance-corpus.json").bufferedReader().use { it.readText() })
        val entries = corpus.getJSONArray("documents").let { documents ->
            buildMap {
                for (index in 0 until documents.length()) {
                    val document = documents.getJSONObject(index)
                    put(
                        document.getString("id"),
                        PreparedProseRecyclerHarness.Entry(document.getString("id"), document.getJSONObject("contentJSON").toString()),
                    )
                }
            }
        }
        val literalWindows = corpus.getJSONArray("warmWindows").toWarmWindows()
        val shortWindow = literalWindows.single { it.id == "short-01" }
        assertEquals(60, shortWindow.primeIds.size)

        shadowOf(context.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val harness = PreparedProseRecyclerHarness(activity, PreparedProseBenchmarkConfiguration.load(context))
        activity.setContentView(FrameLayout(activity).apply {
            addView(harness, FrameLayout.LayoutParams(390, 844))
        })
        shadowOf(Looper.getMainLooper()).idle()

        PreparedProseInstrumentation.beginBenchmark()
        var shortResult: Result<List<PreparedProseRecyclerHarness.WindowTraversalResult>>? = null
        harness.traverseWindowsForTesting(
            windows = listOf(shortWindow),
            entriesById = entries,
            phase = PreparedProseInstrumentation.TraversalPhase.COLD,
            imagesEnabled = true,
        ) { shortResult = it }
        drainMainLooperUntil { shortResult != null }
        val traversal = requireNotNull(shortResult).getOrThrow().single()
        assertTrue("prime must wait for the attached, bound leading holder", traversal.initialLeadingHolderAttached)
        assertEquals(
            "prime compile=${traversal.prime.compileCount}, layout=${traversal.prime.layoutCount}, misses=${traversal.prime.cacheMisses}",
            60,
            traversal.prime.residentKeyCount,
        )
        assertEquals(0, traversal.warm.compileCount)
        assertEquals(0, traversal.warm.layoutCount)
        assertEquals(0, traversal.warm.cacheMisses)

        val oneItem = literalWindows.single { it.id == "very-long-01" }
        assertEquals(1, oneItem.primeIds.size)
        var oneItemResult: Result<List<PreparedProseRecyclerHarness.WindowTraversalResult>>? = null
        harness.traverseWindowsForTesting(
            windows = listOf(oneItem),
            entriesById = entries,
            phase = PreparedProseInstrumentation.TraversalPhase.COLD,
            imagesEnabled = true,
        ) { oneItemResult = it }
        drainMainLooperUntil { oneItemResult != null }
        assertEquals(oneItem.id, requireNotNull(oneItemResult).getOrThrow().single().windowId)

        val exactEvidence = JSONArray().apply {
            literalWindows.forEach { window ->
                put(windowEvidence(window, "cold", window.primeIds))
            }
            literalWindows.forEach { window ->
                put(windowEvidence(window, "warm", window.warmIds))
            }
            literalWindows.forEach { window ->
                put(windowEvidence(window, "imagesDisabled", window.primeIds))
                put(windowEvidence(window, "imagesDisabled", window.warmIds))
            }
        }
        PreparedProsePerformanceGates.assertExactWindowEvidence(exactEvidence, literalWindows)

        val harnessSource = sequenceOf(
            File("src/sharedTest/java/com/apollohg/editor/NativePerformanceSupport.kt"),
            File("../android/src/sharedTest/java/com/apollohg/editor/NativePerformanceSupport.kt"),
            File("../../android/src/sharedTest/java/com/apollohg/editor/NativePerformanceSupport.kt"),
        ).firstOrNull(File::isFile)?.readText()
        assertNotNull("PreparedProseRecyclerHarness source must be available to the contract test", harnessSource)
        val source = requireNotNull(harnessSource)
        assertTrue(source.contains("awaitInitialLeadingAttachment"))
        assertTrue(source.contains("completeDirectionIfIdleAndAttached"))
        assertTrue(source.contains("smoothScrollToPosition(lastIndex)"))
        assertTrue(source.contains("smoothScrollToPosition(0)"))
        assertTrue(source.contains("prepareForReuse()"))
        assertFalse(source.contains("scrollToPosition(position)"))
        assertFalse(source.contains("requestLayout()"))
        assertFalse(source.contains("settle(instrumentation)"))
    }

    private fun JSONArray.toWarmWindows() = List(length()) { index ->
        getJSONObject(index).let { window ->
            fun ids(key: String) = window.getJSONArray(key).let { values ->
                List(values.length()) { valueIndex -> values.getString(valueIndex) }
            }
            PreparedProseRecyclerHarness.WarmWindow(window.getString("id"), ids("primeIds"), ids("warmIds"))
        }
    }

    private fun windowEvidence(
        window: PreparedProseRecyclerHarness.WarmWindow,
        phase: String,
        ids: List<String>,
    ) = JSONObject()
        .put("windowId", window.id)
        .put("phase", phase)
        .put("entryIds", JSONArray(ids))

    private fun drainMainLooperUntil(predicate: () -> Boolean) {
        repeat(600) {
            if (predicate()) return
            shadowOf(Looper.getMainLooper()).idleFor(16, TimeUnit.MILLISECONDS)
        }
        assertTrue("expected attached RecyclerView lifecycle to complete", predicate())
    }

    @Test
    fun `one width prepares once while layout and draw only acquire the artifact`() {
        val engine = CountingLayoutEngine()
        val registry = testRegistry(engine)
        val viewer = ProseViewerView(context, registry)
        assertTrue(viewer.apply(jsonSource("first paragraph"), configuration()))

        val width = View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY)
        val height = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
        viewer.measure(width, height)
        viewer.measure(width, height)
        viewer.layout(0, 0, viewer.measuredWidth, viewer.measuredHeight)
        viewer.draw(Canvas())

        assertEquals(1, engine.preparationCount)
        assertEquals(320, viewer.measuredWidth)
        assertTrue(viewer.measuredHeight > 0)
    }

    @Test
    fun `a new effective width prepares one replacement artifact`() {
        val engine = CountingLayoutEngine()
        val viewer = ProseViewerView(context, testRegistry(engine))
        assertTrue(viewer.apply(jsonSource("width changes"), configuration()))

        viewer.measure(exactWidth(320), unspecifiedHeight())
        viewer.measure(exactWidth(480), unspecifiedHeight())
        viewer.measure(exactWidth(480), unspecifiedHeight())

        assertEquals(2, engine.preparationCount)
    }

    @Test
    fun `public direct prepared replacement announces once and clears focus`() {
        val viewer = ProseViewerView(context, testRegistry(LinkLayoutEngine()))
        val parent = CapturingAccessibilityParent(context).apply { addView(viewer) }

        assertTrue(viewer.apply(jsonSource("first generation"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())
        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )
        parent.clearEvents()

        assertTrue(viewer.apply(jsonSource("replacement generation"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())

        assertEquals(1, parent.subtreeChangeCount())
        assertEquals(
            listOf(
                AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED,
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED,
            ),
            parent.eventTypes,
        )
    }

    @Test
    fun `direct prepared detach reattach retains artifact and clears focus without republishing`() {
        val viewer = ProseViewerView(context, testRegistry(LinkLayoutEngine()))
        val parent = CapturingAccessibilityParent(context).apply { addView(viewer) }

        assertTrue(viewer.apply(jsonSource("detachment generation"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())
        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )
        parent.clearEvents()

        viewer.preparePreparedHostForWindowDetachment()

        assertEquals(
            listOf(AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED),
            parent.eventTypes,
        )
        assertEquals(0, parent.subtreeChangeCount())
        val retained = viewer.preparedLayoutForTesting
        assertNotNull(retained)
        ProseViewerView::class.java.getDeclaredMethod("onAttachedToWindow").apply { isAccessible = true }.invoke(viewer)
        assertTrue(viewer.preparedLayoutForTesting === retained)
        assertEquals(0, parent.subtreeChangeCount())
    }

    @Test
    fun `unbounded width publishes a zero height error once for the generation`() {
        val engine = CountingLayoutEngine()
        val viewer = ProseViewerView(context, testRegistry(engine))
        val errors = mutableListOf<ProseViewerError>()
        viewer.interactionListener = object : ProseViewerInteractionListenerAdapter() {
            override fun onViewerError(view: ProseViewerView, error: ProseViewerError) {
                errors += error
            }
        }
        assertTrue(viewer.apply(jsonSource("requires a width"), configuration()))

        viewer.measure(unspecifiedWidth(), unspecifiedHeight())
        viewer.measure(unspecifiedWidth(), unspecifiedHeight())

        assertEquals(0, viewer.measuredHeight)
        assertEquals(0, engine.preparationCount)
        assertEquals(1, errors.size)
        assertEquals(ProseViewerErrorCode.INVALID_WIDTH, errors.single().code)
    }

    @Test
    fun `compiler failure replaces stale content with a cached zero height error`() {
        val engine = CountingLayoutEngine()
        val compiler = CountingDocumentCompiler { request ->
            if (request.source.value == "malformed") {
                throw ProseViewerError.compiler("viewer", "MALFORMED", "Malformed prose content")
            }
            testDocument(request)
        }
        val viewer = ProseViewerView(context, PreparedProseLayoutRegistry(compiler, engine))
        val errors = mutableListOf<ProseViewerError>()
        viewer.interactionListener = object : ProseViewerInteractionListenerAdapter() {
            override fun onViewerError(view: ProseViewerView, error: ProseViewerError) {
                errors += error
            }
        }
        assertTrue(viewer.apply(jsonSource("visible first"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())

        assertFalse(viewer.apply(ProseViewerSource.Json("malformed"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())
        viewer.measure(exactWidth(320), unspecifiedHeight())

        assertEquals(0, viewer.measuredHeight)
        assertEquals(1, errors.size)
        assertEquals("MALFORMED", errors.single().code.value)
        assertEquals(1, compiler.failures)
        assertNotNull(viewer.preparedLayoutForTesting)
    }

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

        assertEquals(null, registry.acquireForFabricMount(generation, request, 321, 1f))
        registry.releaseFabricMountMiss(generation, 320, 1f)

        assertEquals(0, registry.fabricGenerationPinCountForTesting)
        assertEquals(0, registry.fabricLeaseCountForTesting)
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

    @Test
    fun `culling skips a large offscreen prefix and visits each visible block once`() {
        val blockLayout = StaticLayout.Builder
            .obtain("x", 0, 1, TextPaint().apply { textSize = 14f }, 10)
            .build()
        val artifact = PreparedProseLayout(
            key = ProseLayoutKey("culling", 10, "", 0, 0, 0, 0, "culling"),
            widthPx = 10,
            heightPx = 10_000,
            blocks = List(1_000) { index ->
                PreparedProseBlock(
                    fragments = listOf(
                        PreparedProseFragment(
                            PreparedProseFragmentKind.TEXT,
                            Rect(0, index * 10, 10, index * 10 + 10),
                            layout = blockLayout,
                        )
                    ),
                    bounds = Rect(0, index * 10, 10, index * 10 + 10),
                )
            },
            retainedBytes = 0,
        )
        val visited = mutableListOf<Int>()

        artifact.forEachBlockIntersecting(Rect(0, 9_000, 10, 9_030)) { block ->
            visited += block.topPx
        }

        assertEquals(listOf(9_000, 9_010, 9_020), visited)
        assertEquals(visited.size, visited.distinct().size)
    }

    private fun testRegistry(engine: AndroidProseLayoutEngine): PreparedProseLayoutRegistry =
        PreparedProseLayoutRegistry(CountingDocumentCompiler(::testDocument), engine)

    private fun testDocument(request: ProseViewerRequest): ViewerDocument = ViewerDocument(
        semanticKey = request.compiledCacheKey,
        blocks = listOf(
            ViewerBlock(
                nodeType = "paragraph",
                depth = 0,
                inBlockquote = false,
                listContext = null,
                listItemBoundary = null,
                inlines = listOf(ViewerInline.Text(request.source.value, emptyList())),
            )
        ),
        isEmpty = request.source.value.isEmpty(),
        retainedBytes = request.source.value.length.toLong(),
    )

    private fun jsonSource(value: String) = ProseViewerSource.Json(value)

    private fun configuration() = ProseViewerConfiguration(configJson = "{}")

    private fun request(value: String) = ProseViewerRequest(jsonSource(value), configuration())

    private fun testLayoutKey(generation: String) = ProseLayoutKey(
        semanticKey = generation,
        widthPx = 320,
        themeDigest = "theme",
        nativeFontRevision = 0,
        fontEnvironmentRevision = 0,
        densityBits = 1f.toRawBits().toLong(),
        attachmentRevision = 0,
        generationIdentity = generation,
    )

    private fun testArtifact(key: ProseLayoutKey, retainedBytes: Long) = PreparedProseLayout(
        key = key,
        widthPx = key.widthPx,
        heightPx = 1,
        blocks = emptyList(),
        retainedBytes = retainedBytes,
    )

    private fun exactWidth(width: Int) = View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY)

    private fun unspecifiedWidth() = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

    private fun unspecifiedHeight() = View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)

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

private class CountingDocumentCompiler(
    private val compile: (ProseViewerRequest) -> ViewerDocument,
) : (ProseViewerRequest) -> ViewerDocument {
    var failures = 0
        private set

    override fun invoke(request: ProseViewerRequest): ViewerDocument = try {
        compile(request)
    } catch (error: ProseViewerError) {
        failures += 1
        throw error
    }
}

private class CountingLayoutEngine : AndroidProseLayoutEngine {
    private val delegate = StaticLayoutAndroidProseLayoutEngine()
    var preparationCount = 0
        private set

    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout {
        preparationCount += 1
        return delegate.prepare(document, key, theme, widthPx, density, collapsesWhenEmpty)
    }
}

private class LinkLayoutEngine : AndroidProseLayoutEngine {
    private val delegate = StaticLayoutAndroidProseLayoutEngine()

    override fun prepare(
        document: ViewerDocument,
        key: ProseLayoutKey,
        theme: PreparedProseTheme,
        widthPx: Int,
        density: Float,
        collapsesWhenEmpty: Boolean,
    ): PreparedProseLayout = delegate.prepare(
        document,
        key,
        theme,
        widthPx,
        density,
        collapsesWhenEmpty,
    ).copy(
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
}
