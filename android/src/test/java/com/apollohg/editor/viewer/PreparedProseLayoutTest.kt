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
class PreparedProseLayoutTest {
    private val context
        get() = RuntimeEnvironment.getApplication()

    @Test
    fun `mention activation preserves attributes and rejects a non-object root`() {
        val viewer = ProseViewerView(context, testRegistry(CountingLayoutEngine()))
        val mentions = mutableListOf<ProseViewerMention>()
        val errors = mutableListOf<ProseViewerError>()
        viewer.interactionListener = object : ProseViewerInteractionListenerAdapter() {
            override fun onMentionTap(view: ProseViewerView, mention: ProseViewerMention) {
                mentions += mention
            }

            override fun onViewerError(view: ProseViewerView, error: ProseViewerError) {
                errors += error
            }
        }
        val interaction = PreparedProseInteraction(
            kind = PreparedProseInteraction.Kind.MENTION,
            rects = listOf(Rect(0, 0, 20, 20)),
            visibleText = "@alice",
            docPos = 0xFFFF_FFFFL,
            label = "@alice",
            attrsJson = """{"id":"user-9","profile":{"kind":"clinician"}}""",
        )

        assertTrue(viewer.activatePreparedInteractionForTesting(interaction))
        assertEquals(1, mentions.size)
        assertEquals(0xFFFF_FFFFL, mentions.single().docPos)
        assertEquals("@alice", mentions.single().label)
        assertEquals("user-9", mentions.single().attrs["id"])
        assertEquals("clinician", (mentions.single().attrs["profile"] as Map<*, *>)["kind"])

        assertFalse(
            viewer.activatePreparedInteractionForTesting(
                interaction.copy(docPos = 9, label = "@invalid", attrsJson = "[]"),
            ),
        )
        assertEquals(1, mentions.size)
        assertEquals("INVALID_MENTION_ATTRIBUTES", errors.single().code.value)
    }

    @Test
    fun `direct interactions are inert without a listener`() {
        val viewer = ProseViewerView(context, testRegistry(CountingLayoutEngine()))
        val link = PreparedProseInteraction(
            kind = PreparedProseInteraction.Kind.LINK,
            rects = listOf(Rect(0, 0, 20, 20)),
            href = "https://example.test",
            visibleText = "link",
            label = "link",
        )
        val mention = PreparedProseInteraction(
            kind = PreparedProseInteraction.Kind.MENTION,
            rects = listOf(Rect(0, 0, 20, 20)),
            visibleText = "@Ada",
            docPos = 1,
            label = "@Ada",
            attrsJson = "{}",
        )

        assertFalse(viewer.activatePreparedInteractionForTesting(link))
        assertFalse(viewer.activatePreparedInteractionForTesting(mention))
    }

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
        assertTrue(source.contains("submissionGeneration"))
        assertTrue(source.contains("boundSubmissionGeneration"))
        assertTrue(source.contains("boundEntryId"))
        assertTrue(source.contains("holder?.boundSubmissionGeneration == active.submissionGeneration"))
        assertTrue(source.contains("holder.boundEntryId == expectedEntryId"))
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
        val viewer = ProseViewerView(context, testRegistry(LinkLayoutEngine())).apply {
            onLinkTapForTesting = {}
        }
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, viewer)

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
        var clearedNodeLabel: CharSequence? = null
        parent.onEvent = { event ->
            if (event.eventType == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED) {
                clearedNodeLabel = viewer.accessibilityNodeProvider
                    .createAccessibilityNodeInfo(1)
                    ?.contentDescription
            }
        }

        assertTrue(viewer.apply(jsonSource("replacement generation"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())

        assertEquals("link-320", clearedNodeLabel)
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
        val viewer = ProseViewerView(context, testRegistry(LinkLayoutEngine())).apply {
            onLinkTapForTesting = {}
        }
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, viewer)

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
    fun `hidden direct annotations reject accessibility focus and activation`() {
        var activations = 0
        val viewer = ProseViewerView(context, testRegistry(LinkLayoutEngine())).apply {
            onLinkTapForTesting = { activations += 1 }
        }
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, viewer)
        assertTrue(viewer.apply(jsonSource("hidden direct annotation"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())
        viewer.layout(0, 0, 320, viewer.measuredHeight)
        viewer.accessibilityVisibilityForTesting = { false }
        viewer.visibility = View.INVISIBLE

        assertFalse(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_CLICK,
                null,
            )
        )
        assertFalse(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )
        assertEquals(0, activations)
    }

    @Test
    fun `direct width replacement clears focus while the old artifact is installed`() {
        val viewer = ProseViewerView(context, testRegistry(LinkLayoutEngine())).apply {
            onLinkTapForTesting = {}
        }
        val parent = CapturingAccessibilityParent(context)
        mountVisible(parent, viewer, width = 480)
        assertTrue(viewer.apply(jsonSource("width focus replacement"), configuration()))
        viewer.measure(exactWidth(320), unspecifiedHeight())
        viewer.layout(0, 0, 320, viewer.measuredHeight)
        assertTrue(
            viewer.accessibilityNodeProvider.performAction(
                1,
                AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS,
                null,
            )
        )
        var clearedNodeLabel: CharSequence? = null
        parent.onEvent = { event ->
            if (event.eventType == AccessibilityEvent.TYPE_VIEW_ACCESSIBILITY_FOCUS_CLEARED) {
                clearedNodeLabel = viewer.accessibilityNodeProvider
                    .createAccessibilityNodeInfo(1)
                    ?.contentDescription
            }
        }

        viewer.measure(exactWidth(480), unspecifiedHeight())

        assertEquals("link-320", clearedNodeLabel)
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

    @Test
    fun `ordered markers use default schemes by semantic ancestor depth`() {
        val orderedContext = ViewerListContext(
            ordered = true,
            index = 1,
            kind = null,
            checked = false,
            isLast = true,
        )
        val bulletContext = orderedContext.copy(ordered = false)
        val checkedTaskContext = bulletContext.copy(kind = "task", checked = true)
        fun ancestor(
            identity: Int,
            context: ViewerListContext,
            storedDepth: Int,
            isMarkerOwner: Boolean,
        ) = ViewerListItemAncestor(
            identity = identity,
            context = context,
            nestingDepth = storedDepth,
            isFirstRenderableLeaf = isMarkerOwner,
            isFinalRenderableLeaf = isMarkerOwner,
        )
        val ancestorChains = listOf(
            listOf(ancestor(0, orderedContext, storedDepth = 2, isMarkerOwner = true)),
            listOf(
                ancestor(100, bulletContext, storedDepth = 8, isMarkerOwner = false),
                ancestor(1, orderedContext, storedDepth = 0, isMarkerOwner = true),
            ),
            listOf(
                ancestor(101, orderedContext, storedDepth = 12, isMarkerOwner = false),
                ancestor(102, bulletContext, storedDepth = 3, isMarkerOwner = false),
                ancestor(2, orderedContext, storedDepth = 0, isMarkerOwner = true),
            ),
            listOf(
                ancestor(103, bulletContext, storedDepth = 20, isMarkerOwner = true),
                ancestor(104, checkedTaskContext, storedDepth = 2, isMarkerOwner = true),
                ancestor(105, orderedContext, storedDepth = 0, isMarkerOwner = true),
                ancestor(3, orderedContext, storedDepth = 1, isMarkerOwner = true),
            ),
        )
        val blocks = ancestorChains.mapIndexed { index, ancestors ->
            val markerOwner = ancestors.last()
            ViewerBlock(
                nodeType = "paragraph",
                depth = 40 + index,
                inBlockquote = index == 1,
                listContext = markerOwner.context,
                listItemBoundary = ViewerListItemBoundary(
                    identity = markerOwner.identity,
                    nestingDepth = markerOwner.nestingDepth,
                    isFirstRenderableLeaf = true,
                    isFinalRenderableLeaf = true,
                ),
                inlines = listOf(ViewerInline.Text("item", emptyList())),
                listItemAncestors = ancestors,
            )
        }
        val document = ViewerDocument(
            semanticKey = "ordered-marker-theme",
            blocks = blocks,
            isEmpty = false,
            retainedBytes = 128,
        )
        val theme = PreparedProseTheme.resolve(null, density = 1f)

        val layout = StaticLayoutAndroidProseLayoutEngine().prepare(
            document = document,
            key = testLayoutKey("ordered-marker-theme"),
            theme = theme,
            widthPx = 320,
            density = 1f,
            collapsesWhenEmpty = false,
        )
        val markerFragments = layout.blocks
            .flatMap { it.fragments }
            .filter { it.kind == PreparedProseFragmentKind.MARKER }
        val markerLabels = markerFragments.mapNotNull { it.label }
        val nestedLeafMarkers = layout.blocks.last().fragments
            .filter { it.kind == PreparedProseFragmentKind.MARKER }

        assertEquals(listOf("1.", "a.", "i.", "•", "", "i.", "1."), markerLabels)
        assertEquals(listOf("•", "", "i.", "1."), nestedLeafMarkers.mapNotNull { it.label })
        assertEquals("•", nestedLeafMarkers[0].label)
        assertFalse(nestedLeafMarkers[0].checked)
        assertEquals("", nestedLeafMarkers[1].label)
        assertTrue(nestedLeafMarkers[1].checked)
        assertEquals(listOf("i.", "1."), nestedLeafMarkers.drop(2).mapNotNull { it.label })
    }

    @Test
    fun `ordered marker editor and viewer rendering conform for shared tuples`() {
        data class Fixture(val index: Long, val semanticDepth: Int, val expected: String)

        val fixtures = listOf(
            Fixture(index = 27, semanticDepth = 0, expected = "AA)"),
            Fixture(index = 3_999, semanticDepth = 1, expected = "MMMCMXCIX)"),
            Fixture(index = 42, semanticDepth = 2, expected = "42)"),
        )
        val themeJson =
            """{"list":{"orderedMarker":{"schemes":["upperAlpha","upperRoman","decimal"],"suffix":")"}}}"""
        val editorTheme = com.apollohg.editor.EditorTheme.fromJson(themeJson)
        val viewerTheme = PreparedProseTheme.resolve(themeJson, density = 1f)

        fixtures.forEach { fixture ->
            val renderElements = JSONArray()
            repeat(fixture.semanticDepth + 1) { depth ->
                val deepest = depth == fixture.semanticDepth
                renderElements.put(
                    JSONObject()
                        .put("type", "blockStart")
                        .put("nodeType", "listItem")
                        .put("depth", depth)
                        .put(
                            "listContext",
                            JSONObject()
                                .put("ordered", deepest)
                                .put("index", if (deepest) fixture.index else 1)
                                .put("isFirst", true)
                                .put("isLast", true),
                        ),
                )
            }
            renderElements.put(
                JSONObject()
                    .put("type", "blockStart")
                    .put("nodeType", "paragraph")
                    .put("depth", fixture.semanticDepth + 1),
            )
            renderElements.put(
                JSONObject()
                    .put("type", "textRun")
                    .put("text", "item")
                    .put("marks", JSONArray()),
            )
            renderElements.put(JSONObject().put("type", "blockEnd"))
            repeat(fixture.semanticDepth + 1) {
                renderElements.put(JSONObject().put("type", "blockEnd"))
            }

            val editor = RenderBridge.buildSpannable(
                renderElements.toString(),
                16f,
                0xFF000000.toInt(),
                editorTheme,
            )
            val editorLabel = editor.getSpans(
                0,
                editor.length,
                OrderedListMarkerSpan::class.java,
            ).single().label

            val orderedContext = ViewerListContext(
                ordered = true,
                index = fixture.index,
                kind = null,
                checked = false,
                isLast = true,
            )
            val ancestors = (0..fixture.semanticDepth).map { depth ->
                val deepest = depth == fixture.semanticDepth
                ViewerListItemAncestor(
                    identity = depth,
                    context = if (deepest) orderedContext else orderedContext.copy(ordered = false),
                    nestingDepth = 50 - depth,
                    isFirstRenderableLeaf = deepest,
                    isFinalRenderableLeaf = deepest,
                )
            }
            val block = ViewerBlock(
                nodeType = "paragraph",
                depth = 80 + fixture.semanticDepth,
                inBlockquote = false,
                listContext = orderedContext,
                listItemBoundary = ViewerListItemBoundary(
                    identity = ancestors.last().identity,
                    nestingDepth = 40 - fixture.semanticDepth,
                    isFirstRenderableLeaf = true,
                    isFinalRenderableLeaf = true,
                ),
                inlines = listOf(ViewerInline.Text("item", emptyList())),
                listItemAncestors = ancestors,
            )
            val viewer = StaticLayoutAndroidProseLayoutEngine().prepare(
                document = ViewerDocument(
                    semanticKey = "conformance-${fixture.semanticDepth}",
                    blocks = listOf(block),
                    isEmpty = false,
                    retainedBytes = 64,
                ),
                key = testLayoutKey("conformance-${fixture.semanticDepth}"),
                theme = viewerTheme,
                widthPx = 320,
                density = 1f,
                collapsesWhenEmpty = false,
            )
            val viewerLabel = viewer.blocks
                .flatMap { it.fragments }
                .single { it.kind == PreparedProseFragmentKind.MARKER }
                .label

            assertEquals(fixture.expected, editorLabel)
            assertEquals(fixture.expected, viewerLabel)
            assertEquals(editorLabel, viewerLabel)
        }
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

    private fun mountVisible(
        parent: CapturingAccessibilityParent,
        child: View,
        width: Int = 320,
        height: Int = 200,
    ) {
        parent.addView(child)
        (child as? ProseViewerView)?.accessibilityVisibilityForTesting = { true }
        parent.measure(
            View.MeasureSpec.makeMeasureSpec(width, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(height, View.MeasureSpec.EXACTLY),
        )
        parent.layout(0, 0, width, height)
        child.layout(0, 0, width, height)
    }

    private class CapturingAccessibilityParent(context: android.content.Context) : ViewGroup(context) {
        init {
            shadowOf(context.getSystemService(AccessibilityManager::class.java)).setEnabled(true)
        }

        val eventTypes = mutableListOf<Int>()
        private val changeTypes = mutableListOf<Int>()
        var onEvent: ((AccessibilityEvent) -> Unit)? = null

        override fun requestSendAccessibilityEvent(child: View, event: AccessibilityEvent): Boolean {
            onEvent?.invoke(event)
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
                visibleText = "link-$widthPx",
                label = "link-$widthPx",
            ),
        ),
        accessibilityNodes = listOf(
            PreparedProseAccessibilityNode(
                interactionIndex = 0,
                role = PreparedProseAccessibilityNode.Role.LINK,
                label = "link-$widthPx",
                bounds = Rect(0, 0, 20, 20),
            ),
        ),
    )
}
