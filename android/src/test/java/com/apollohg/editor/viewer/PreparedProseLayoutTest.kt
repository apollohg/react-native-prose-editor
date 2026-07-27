package com.apollohg.editor.viewer

import android.graphics.Canvas
import android.graphics.Bitmap
import android.graphics.Rect
import android.text.StaticLayout
import android.text.TextPaint
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
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
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class PreparedProseLayoutTest {
    private val context
        get() = RuntimeEnvironment.getApplication()

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

        registry.measure(request, widthPx = 320, density = 1f, fabricSurface = surface)
        val artifact = registry.acquireForFabricMount(surface, request, widthPx = 320, density = 1f)
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
        registry.measure(request("first"), 320, 1f, FabricSurfaceToken(7, 71))
        registry.measure(request("second"), 320, 1f, FabricSurfaceToken(7, 72))

        registry.releaseFabricSurfaceId(7)

        assertEquals(0, registry.fabricGenerationPinCountForTesting)
        assertEquals(0, registry.fabricLeaseCountForTesting)
    }

    @Test
    fun `Fabric mount miss releases the exact generation pin and lease`() {
        val registry = testRegistry(CountingLayoutEngine())
        val request = request("mount miss")
        val surface = FabricSurfaceToken(8, 81)
        registry.measure(request, 320, 1f, surface)

        assertEquals(null, registry.acquireForFabricMount(surface, request, 321, 1f))
        registry.releaseFabricMountMiss(FabricGenerationToken(surface, request.generationIdentity))

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
        registry.measure(request, 320, 1f, FabricSurfaceToken(9, 91))

        assertEquals(0, registry.layoutRetainedBytesForTesting)
        assertEquals(1, registry.fabricLeaseCountForTesting)
    }

    @Test
    fun `Fabric leases retain mounted handoffs until their surface releases them`() {
        val registry = testRegistry(CountingLayoutEngine())
        repeat(33) { index ->
            registry.measure(
                request("lease $index"),
                320,
                1f,
                FabricSurfaceToken(10, 100 + index),
            )
        }

        assertEquals(33, registry.fabricLeaseCountForTesting)
        registry.releaseFabricSurfaceId(10)
        assertEquals(0, registry.fabricLeaseCountForTesting)
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
