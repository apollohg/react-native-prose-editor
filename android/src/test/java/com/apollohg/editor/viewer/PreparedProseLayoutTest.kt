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
internal class PreparedProseLayoutTest : PreparedProseLayoutTestFixture() {
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
}
