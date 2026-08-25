package com.apollohg.editor

import android.app.Activity
import android.content.Context
import android.graphics.Point
import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.view.Window
import android.view.inputmethod.EditorInfo
import android.widget.FrameLayout
import expo.modules.core.ModuleRegistry
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.ModulesProvider
import expo.modules.kotlin.modules.Module
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.lang.ref.WeakReference
import java.time.Duration
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorExpoViewTest {
    @Test
    fun `accessibility hint does not consume editor long press as a tooltip`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editor = view.richTextView.editorEditText

        view.setAccessibilityHint("Formatting controls are above the keyboard")

        assertNull(editor.tooltipText)
        val nodeInfo = editor.createAccessibilityNodeInfo()
        assertEquals("Formatting controls are above the keyboard", nodeInfo.tooltipText)
    }

    @Test
    fun `Android input options report and clear private IME options`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        view.setAndroidInputOptionsJson("""{"privateImeOptions":"nm"}""")
        val configuredEditorInfo = EditorInfo()
        assertNotNull(view.richTextView.editorEditText.onCreateInputConnection(configuredEditorInfo))
        assertEquals("nm", configuredEditorInfo.privateImeOptions)

        view.setAndroidInputOptionsJson(null)
        val clearedEditorInfo = EditorInfo()
        assertNotNull(view.richTextView.editorEditText.onCreateInputConnection(clearedEditorInfo))
        assertNull(clearedEditorInfo.privateImeOptions)
    }

    @Test
    fun `bound adapter failure emits one complete Expo error record`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(
            backend,
            "{\"initialization\":{\"type\":\"localEmpty\"},\"policy\":{\"readOnly\":true}}"
        )
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<Map<String, Any>>()
        try {
            view.onEditorErrorForTesting = { errors += it }
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)

            assertTrue(commitBoundText(view, "x"))
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(1, errors.size)
            val payload = errors.single()
            assertEquals(adapter.editorId, payload["editorId"])
            @Suppress("UNCHECKED_CAST")
            val error = payload["error"] as Map<String, Any?>
            assertEquals(
                setOf("domain", "code", "message", "requestId", "operationIndex", "limit", "actual", "detailsJson"),
                error.keys
            )
            assertEquals("boundary", error["domain"])
            assertEquals("MUTATION_REJECTED", error["code"])
            assertTrue((error["message"] as String).isNotEmpty())
            assertNotNull(error["requestId"])
            assertNull(error["operationIndex"])
            assertNull(error["limit"])
            assertNull(error["actual"])
            assertNull(error["detailsJson"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `equal bound adapter failures each route once`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<Map<String, Any>>()
        try {
            view.onEditorErrorForTesting = { errors += it }
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            adapter.destroy()

            assertTrue(commitBoundText(view, "x"))
            assertTrue(commitBoundText(view, "y"))
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(2, errors.size)
            assertEquals(errors[0]["error"], errors[1]["error"])
            @Suppress("UNCHECKED_CAST")
            val error = errors.first()["error"] as Map<String, Any?>
            assertEquals("ENGINE_DESTROYED", error["code"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `queued bound adapter error drops across A to B to A rebind`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapterA = attachAdapterForViewTest(
            backend,
            "{\"initialization\":{\"type\":\"localEmpty\"},\"policy\":{\"readOnly\":true}}"
        )
        val adapterB = attachAdapterForViewTest(backend)
        val tokenA = EditorV2Registry.register(adapterA)
        val tokenB = EditorV2Registry.register(adapterB)
        val errors = mutableListOf<Map<String, Any>>()
        try {
            view.onEditorErrorForTesting = { errors += it }
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(tokenA)

            assertTrue(commitBoundText(view, "x"))
            assertEquals(1, view.pendingEditorErrorEventCountForTesting())
            view.setEditorId(tokenB)
            view.setEditorId(tokenA)
            shadowOf(Looper.getMainLooper()).idle()

            assertTrue(errors.isEmpty())
            assertEquals(0, view.pendingEditorErrorEventCountForTesting())
        } finally {
            EditorV2Registry.remove(adapterA.editorId)
            EditorV2Registry.remove(adapterB.editorId)
            NativeEditorViewRegistry.unregister(tokenA, view)
            NativeEditorViewRegistry.unregister(tokenB, view)
        }
    }

    @Test
    fun `detached or destroyed view drops queued bound adapter errors`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(
            backend,
            "{\"initialization\":{\"type\":\"localEmpty\"},\"policy\":{\"readOnly\":true}}"
        )
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<Map<String, Any>>()
        try {
            view.onEditorErrorForTesting = { errors += it }
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)

            assertTrue(commitBoundText(view, "x"))
            assertEquals(1, view.pendingEditorErrorEventCountForTesting())
            view.handleDetachedFromWindowForTesting()
            shadowOf(Looper.getMainLooper()).idle()
            assertTrue(errors.isEmpty())

            view.setAttachedToNativeWindowForTesting(true)
            view.handleAttachedToWindowForTesting()
            assertTrue(commitBoundText(view, "y"))
            assertEquals(1, view.pendingEditorErrorEventCountForTesting())
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
            shadowOf(Looper.getMainLooper()).idle()
            assertTrue(errors.isEmpty())
            assertEquals(0, view.pendingEditorErrorEventCountForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `newest bound view exclusively owns adapter error delivery`() {
        val firstExpoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val secondExpoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val firstView = NativeEditorExpoView(firstExpoContext.context, firstExpoContext.appContext)
        val secondView = NativeEditorExpoView(secondExpoContext.context, secondExpoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val firstErrors = mutableListOf<Map<String, Any>>()
        val secondErrors = mutableListOf<Map<String, Any>>()
        try {
            firstView.onEditorErrorForTesting = { firstErrors += it }
            secondView.onEditorErrorForTesting = { secondErrors += it }
            listOf(firstView, secondView).forEach { view ->
                view.onEditorUpdateForTesting = {}
                view.onAddonEventForTesting = {}
                view.onEditorReadyForTesting = {}
                view.onSelectionChangeForTesting = {}
                view.setAttachedToNativeWindowForTesting(true)
                view.setEditorId(viewToken)
            }
            assertTrue(
                firstView.editorErrorCallbackTokenForTesting() !=
                    secondView.editorErrorCallbackTokenForTesting()
            )
            firstView.setEditorId(0L)
            adapter.destroy()

            assertTrue(commitBoundText(secondView, "x"))
            shadowOf(Looper.getMainLooper()).idle()

            assertTrue(firstErrors.isEmpty())
            assertEquals(1, secondErrors.size)
            assertEquals(adapter.editorId, secondErrors.single()["editorId"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, firstView)
            NativeEditorViewRegistry.unregister(viewToken, secondView)
        }
    }

    @Test
    fun `delayed editor updates drain with captured identity across rebind`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        fun registerAdapter(): Pair<EditorV2Adapter, Long> {
            val editorId = (backend.create("{\"initialization\":{\"type\":\"localEmpty\"}}", null) as EditorV2CallResult.Ok).value
            val adapter = EditorV2Adapter.attach(backend, JSONObject(editorId).getString("editorId"), roomBound = false)!!
            return adapter to EditorV2Registry.register(adapter)
        }
        val (adapterA, tokenA) = registerAdapter()
        val (adapterB, tokenB) = registerAdapter()
        val payloads = mutableListOf<Map<String, Any>>()
        try {
            assertNotNull(adapterA.adoptExternalRender(atomicRenderUpdateJson("A", "7")))
            assertNotNull(adapterB.adoptExternalRender(atomicRenderUpdateJson("B", "8")))
            view.onEditorUpdateForTesting = { payloads += it }
            view.onAddonEventForTesting = {}

            view.setEditorId(tokenA)
            view.onEditorUpdate(JSONObject(renderUpdateJson("A")).put("documentVersion", "7").toString())
            view.setEditorId(tokenB)
            view.onEditorUpdate(JSONObject(renderUpdateJson("B")).put("documentVersion", "8").toString())
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))

            assertEquals(2, payloads.size)
            assertEquals(adapterA.editorId, payloads[0]["editorId"])
            assertEquals("7", payloads[0]["documentRevision"])
            assertEquals(adapterB.editorId, payloads[1]["editorId"])
            assertEquals("8", payloads[1]["documentRevision"])
            assertEquals("B", JSONObject(payloads[1]["updateJson"] as String)
                .getJSONArray("renderBlocks").getJSONArray(0).getJSONObject(1).getString("text"))
        } finally {
            EditorV2Registry.remove(adapterA.editorId)
            EditorV2Registry.remove(adapterB.editorId)
            NativeEditorViewRegistry.unregister(tokenA, view)
            NativeEditorViewRegistry.unregister(tokenB, view)
        }
    }

    @Test
    fun `queued native update from prior A binding drains before A to B to A rebind`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        fun registerAdapter(): Pair<EditorV2Adapter, Long> {
            val editorId = (backend.create("{\"initialization\":{\"type\":\"localEmpty\"}}", null) as EditorV2CallResult.Ok).value
            val adapter = EditorV2Adapter.attach(backend, JSONObject(editorId).getString("editorId"), roomBound = false)!!
            return adapter to EditorV2Registry.register(adapter)
        }
        val (adapterA, tokenA) = registerAdapter()
        val (adapterB, tokenB) = registerAdapter()
        val payloads = mutableListOf<Map<String, Any>>()
        try {
            assertNotNull(adapterA.adoptExternalRender(atomicRenderUpdateJson("stale A", "7")))
            view.onEditorUpdateForTesting = { payloads += it }
            view.onAddonEventForTesting = {}

            view.setEditorId(tokenA)
            view.onEditorUpdate(JSONObject(renderUpdateJson("stale A")).put("documentVersion", "7").toString())
            assertEquals(1, view.pendingEditorUpdateEventCountForTesting())

            view.setEditorId(tokenB)
            view.setEditorId(tokenA)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))

            assertEquals(1, payloads.size)
            assertEquals(adapterA.editorId, payloads.single()["editorId"])
            assertEquals("7", payloads.single()["documentRevision"])
            assertEquals(0, view.pendingEditorUpdateEventCountForTesting())
        } finally {
            EditorV2Registry.remove(adapterA.editorId)
            EditorV2Registry.remove(adapterB.editorId)
            NativeEditorViewRegistry.unregister(tokenA, view)
            NativeEditorViewRegistry.unregister(tokenB, view)
        }
    }

    @Test
    fun `Rust editor destruction precedes registry invalidation even when destruction fails`() {
        val calls = mutableListOf<String>()

        runCatching {
            destroyEditorThenInvalidate(
                editorHandle = "42",
                viewToken = 42L,
                beginDestroy = {
                    calls += "begin:$it"
                    true
                },
                destroy = {
                    calls += "rust"
                    error("simulated destroy failure")
                },
                finalizeDestroy = { calls += "registry:$it" }
            )
        }

        assertEquals(listOf("begin:42", "rust", "registry:42"), calls)
    }

    @Test
    fun `destroy transition rejects commands and reentrant destroy before Rust returns`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 9_200_000L
        var rustDestroyCalls = 0
        NativeEditorViewRegistry.markEditorCreated(editorId)
        assertTrue(NativeEditorViewRegistry.register(editorId, view))

        destroyEditorThenInvalidate(
            editorHandle = "9200000",
            viewToken = editorId,
            destroy = {
                rustDestroyCalls += 1
                assertTrue(NativeEditorViewRegistry.isDestroyed(editorId))
                assertFalse(NativeEditorViewRegistry.register(editorId, view))
                val preparation = JSONObject(
                    NativeEditorViewRegistry.prepareForCommandJSON(editorId)
                )
                assertFalse(preparation.getBoolean("ready"))
                assertEquals("destroyed", preparation.getString("blockedReason"))
                destroyEditorThenInvalidate(
                    editorHandle = "9200000",
                    viewToken = editorId,
                    destroy = { rustDestroyCalls += 1 }
                )
            }
        )

        destroyEditorThenInvalidate(
            editorHandle = "9200000",
            viewToken = editorId,
            destroy = { rustDestroyCalls += 1 }
        )

        assertEquals(1, rustDestroyCalls)
        assertEquals(0, NativeEditorViewRegistry.retainedDestroyedIdCountForTests())
        assertEquals(0L, view.richTextView.editorId)
    }

    @Test
    fun `destroyed editor registry retains no tombstones`() {
        repeat(10_000) { index ->
            val editorId = 9_000_000L + index
            NativeEditorViewRegistry.markEditorCreated(editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        }

        assertEquals(0, NativeEditorViewRegistry.retainedDestroyedIdCountForTests())
    }

    @Test
    fun `cleared bound view references are pruned without retaining editor history`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 9_100_000L
        NativeEditorViewRegistry.markEditorCreated(editorId)
        assertTrue(NativeEditorViewRegistry.register(editorId, view))
        assertEquals(1, NativeEditorViewRegistry.boundViewReferenceCountForTests(editorId))

        NativeEditorViewRegistry.forceRegisteredViewsClearedForTesting(editorId)
        assertTrue(
            JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
                .getBoolean("ready")
        )
        assertEquals(0, NativeEditorViewRegistry.boundViewReferenceCountForTests(editorId))

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        assertEquals(0, NativeEditorViewRegistry.retainedDestroyedIdCountForTests())
    }

    @Test
    fun `editor image policy prop reaches editor text view`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        view.setImageLoadingPolicyJson("""{"readTimeoutMs":321}""")

        assertEquals(321, view.richTextView.editorEditText.imageLoadingPolicy.readTimeoutMs)
    }

    @Test
    fun `standalone toolbar hit testing uses normalized window coordinates only`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val density = expoContext.context.resources.displayMetrics.density
        val windowOriginOnScreen = Point(6, 24)

        view.setToolbarFrameJson("""{"x":20,"y":40,"width":100,"height":32}""")

        assertTrue(
            view.isPointInsideStandaloneToolbarForTesting(
                rawX = 30f * density + windowOriginOnScreen.x,
                rawY = 50f * density + windowOriginOnScreen.y,
                windowOriginOnScreen = windowOriginOnScreen
            )
        )
        assertFalse(
            view.isPointInsideStandaloneToolbarForTesting(
                rawX = 30f * density + windowOriginOnScreen.x,
                rawY = 90f * density + windowOriginOnScreen.y,
                windowOriginOnScreen = windowOriginOnScreen
            )
        )
    }

    @Test
    fun `standalone toolbar hit testing matches Fabric measureInWindow under edge to edge`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val density = expoContext.context.resources.displayMetrics.density

        view.setToolbarFrameJson("""{"x":74.2857,"y":483.4286,"width":262.8571,"height":56}""")

        assertTrue(
            view.isPointInsideStandaloneToolbarForTesting(
                rawX = 165f * density,
                rawY = 511f * density,
                windowOriginOnScreen = Point(0, 0)
            )
        )
    }

    @Test
    fun `toolbar focus preservation is inactive until a toolbar touch is recorded`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        assertFalse(view.shouldPreserveFocusAfterToolbarTouchForTesting())

        view.markRecentToolbarTouchForTesting()
        assertTrue(view.shouldPreserveFocusAfterToolbarTouchForTesting())

        view.blur()
        assertFalse(view.shouldPreserveFocusAfterToolbarTouchForTesting())
    }

    @Test
    fun `outside tap schedules native outside blur`() {
        val view = attachedNativeEditorView()
        val event = MotionEvent.obtain(0L, 0L, MotionEvent.ACTION_DOWN, 500f, 500f, 0)

        val decision = view.prepareOutsideTapDecisionForWindowEvent(event)
        view.handleOutsideTapDecisionFromWindowDispatcher(decision)
        event.recycle()

        assertEquals(NativeEditorOutsideTapDecision.OUTSIDE_EDITOR, decision)
        assertTrue(view.hasPendingOutsideTapBlurForTesting())
        view.cancelOutsideTapBlurFromWindowDispatcher()
    }

    @Test
    fun `toolbar frame tap preserves focus before dispatch result`() {
        val view = attachedNativeEditorView()
        val density = view.context.resources.displayMetrics.density
        view.setToolbarFrameJson("""{"x":20,"y":40,"width":100,"height":32}""")
        view.scheduleOutsideTapBlurFromWindowDispatcher()
        assertTrue(view.hasPendingOutsideTapBlurForTesting())
        val event = MotionEvent.obtain(
            0L,
            0L,
            MotionEvent.ACTION_DOWN,
            30f * density,
            50f * density,
            0
        )

        val decision = view.prepareOutsideTapDecisionForWindowEvent(event)
        view.handleOutsideTapDecisionFromWindowDispatcher(decision)
        event.recycle()

        assertEquals(NativeEditorOutsideTapDecision.PRESERVE_FOCUS, decision)
        assertTrue(view.shouldPreserveFocusAfterToolbarTouchForTesting())
        assertFalse(view.hasPendingOutsideTapBlurForTesting())
    }

    @Test
    fun `outside tap clears stale toolbar focus preservation`() {
        val view = attachedNativeEditorView()
        view.markRecentToolbarTouchForTesting()
        assertTrue(view.shouldPreserveFocusAfterToolbarTouchForTesting())

        val event = MotionEvent.obtain(0L, 0L, MotionEvent.ACTION_DOWN, 500f, 500f, 0)
        val decision = view.prepareOutsideTapDecisionForWindowEvent(event)
        view.handleOutsideTapDecisionFromWindowDispatcher(decision)
        event.recycle()

        assertEquals(NativeEditorOutsideTapDecision.OUTSIDE_EDITOR, decision)
        assertFalse(view.shouldPreserveFocusAfterToolbarTouchForTesting())
        assertTrue(view.hasPendingOutsideTapBlurForTesting())
        view.cancelOutsideTapBlurFromWindowDispatcher()
    }

    @Test
    fun `toolbar refocus does not cancel stale pending outside blur`() {
        val view = attachedNativeEditorView()

        view.scheduleOutsideTapBlurFromWindowDispatcher()
        assertTrue(view.hasPendingOutsideTapBlurForTesting())

        view.focusFromToolbarPreserveForTesting()

        assertTrue(view.hasPendingOutsideTapBlurForTesting())
        view.cancelOutsideTapBlurFromWindowDispatcher()
    }

    @Test
    fun `outside tap handler resolves the app context current activity`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(
            RuntimeEnvironment.getApplication(),
            currentActivity = activity
        )
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        view.onFocusChangeForTesting = {}
        view.onAddonEventForTesting = {}
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)

        try {
            view.installOutsideTapBlurHandlerForTesting()

            val event = MotionEvent.obtain(0L, 0L, MotionEvent.ACTION_DOWN, 500f, 500f, 0)
            assertEquals(
                NativeEditorOutsideTapDecision.OUTSIDE_EDITOR,
                view.prepareOutsideTapDecisionForWindowEvent(event)
            )
            event.recycle()
        } finally {
            view.cancelOutsideTapBlurFromWindowDispatcher()
            view.uninstallOutsideTapBlurHandlerForTesting()
        }
    }

    @Test
    fun `detached registered view blocks command preflight until it reattaches`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 12345L

        NativeEditorViewRegistry.markEditorCreated(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        assertTrue(
            JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
                .getBoolean("ready")
        )

        NativeEditorViewRegistry.unregister(editorId, view, blockCommandsUntilRegistered = true)

        assertFalse(
            JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
                .getBoolean("ready")
        )

        NativeEditorViewRegistry.register(editorId, view)

        assertTrue(
            JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
                .getBoolean("ready")
        )
        NativeEditorViewRegistry.unregister(editorId, view)
        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
    }

    @Test
    fun `non owner unregister does not clear detached command block`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val ownerView = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val otherView = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 22334L

        NativeEditorViewRegistry.register(editorId, ownerView)
        NativeEditorViewRegistry.unregister(
            editorId,
            ownerView,
            blockCommandsUntilRegistered = true
        )
        NativeEditorViewRegistry.unregister(editorId, otherView)

        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("detached", preparation.getString("blockedReason"))

        NativeEditorViewRegistry.register(editorId, ownerView)
        NativeEditorViewRegistry.unregister(editorId, ownerView)
    }

    @Test
    fun `editor id set while detached blocks command preflight without binding editor`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 23456L

        view.setEditorId(editorId)

        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("detached", preparation.getString("blockedReason"))
        assertEquals(editorId, view.richTextView.editorId)
        assertEquals(0L, view.richTextView.editorEditText.editorId)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `timed out off main command preflight does not flush composition later`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 24567L
        val editText = view.richTextView.editorEditText
        var insertedText: String? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
        }
        NativeEditorViewRegistry.register(editorId, view)

        val inputConnection = editText.onCreateInputConnection(android.view.inputmethod.EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        val result = AtomicReference<String?>(null)
        val thread = Thread {
            result.set(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        }
        thread.start()
        thread.join(1000)

        val preparation = JSONObject(result.get()!!)
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("unknown", preparation.getString("blockedReason"))

        shadowOf(Looper.getMainLooper()).idle()

        assertNull(insertedText)
        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `off main command preflight waits for started side effecting preparation`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 24568L

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        view.richTextView.editorEditText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        view.richTextView.editorEditText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        NativeEditorViewRegistry.register(editorId, view)
        val preparationStarted = AtomicBoolean(false)
        view.onBeforePrepareForEditorCommandForTesting = {
            preparationStarted.set(true)
            Thread.sleep(300)
        }

        val result = AtomicReference<String?>(null)
        val thread = Thread {
            result.set(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        }
        thread.start()
        while (!preparationStarted.get() && result.get() == null) {
            shadowOf(Looper.getMainLooper()).idle()
            Thread.sleep(10)
        }
        thread.join(1000)

        assertFalse(thread.isAlive)
        val preparation = JSONObject(result.get()!!)
        assertTrue(preparation.getBoolean("ready"))

        NativeEditorViewRegistry.unregister(editorId, view)
        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
    }

    @Test
    fun `detach preserves pending controlled editor update json`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = """{"renderElements":[],"selection":{"type":"text","anchor":0,"head":0}}"""

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateRevision(1)

        view.handleDetachedFromWindowForTesting()

        assertEquals(updateJson, view.pendingEditorUpdateJsonForTesting())
    }

    @Test
    fun `editor id change preserves pending controlled update until matching update editor id arrives`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = """{"renderElements":[],"selection":{"type":"text","anchor":0,"head":0}}"""

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateRevision(7)
        view.setEditorId(33445L)

        assertEquals(updateJson, view.pendingEditorUpdateJsonForTesting())
        assertEquals(7, view.pendingEditorUpdateRevisionForTesting())
        assertNull(view.pendingEditorUpdateEditorIdForTesting())

        view.setPendingEditorUpdateEditorId(33445L)

        assertEquals(33445L, view.pendingEditorUpdateEditorIdForTesting())
        assertEquals(updateJson, view.pendingEditorUpdateJsonForTesting())

        NativeEditorViewRegistry.unregister(33445L, view)
    }

    @Test
    fun `editor id change drops pending update scoped to a different editor`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = """{"renderElements":[],"selection":{"type":"text","anchor":0,"head":0}}"""

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(111L)
        view.setPendingEditorUpdateRevision(3)

        view.setEditorId(222L)

        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())

        NativeEditorViewRegistry.unregister(222L, view)
    }

    @Test
    fun `null controlled update clears queued update for matching editor`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = """{"renderElements":[],"selection":{"type":"text","anchor":0,"head":0}}"""

        view.richTextView.setEditorIdWhileDetached(55667L)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(55667L)
        view.setPendingEditorUpdateRevision(1)

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(55667L)
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()

        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
    }

    @Test
    fun `replayed applied controlled update revision clears stale pending state`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 55668L
        val replayedUpdateJson = renderUpdateJson("replayed")
        val editText = view.richTextView.editorEditText
        val readyPayloads = mutableListOf<Map<String, Any>>()

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("first"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onEditorReadyForTesting = { payload ->
            readyPayloads.add(payload)
        }
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onSelectionChangeForTesting = {}
        view.onAddonEventForTesting = {}
        view.setAppliedEditorUpdateRevisionForTesting(1)

        view.setPendingEditorUpdateJson(replayedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()

        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        assertEquals("first", editText.text?.toString())
        assertEquals(1, readyPayloads.size)
        assertEquals(1L, readyPayloads.single()["editorUpdateRevision"])
    }

    @Test
    fun `editor id change resets last document version`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        view.setLastDocumentVersionForTesting("20")

        assertEquals("20", view.lastDocumentVersionForTesting())

        view.setEditorId(66778L)

        assertNull(view.lastDocumentVersionForTesting())

        NativeEditorViewRegistry.unregister(66778L, view)
    }

    @Test
    fun `toolbar action preflight emits TS-compatible document revision matching atomic update`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val editText = view.richTextView.editorEditText
        val toolbarActionPayloads = mutableListOf<Map<String, Any>>()

        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            editText.setSelection(0)
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)
            assertTrue(inputConnection!!.setComposingText("native", 1))
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayloads += payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertEquals(1, toolbarActionPayloads.size)
            val payload = toolbarActionPayloads.single()
            val updateJson = payload["updateJson"] as String
            val snapshotRevision = JSONObject(updateJson).getString("documentVersion")
            assertEquals(snapshotRevision, payload["documentRevision"])
            assertFalse(payload.containsKey("documentVersion"))
            assertEquals(adapter.editorId, payload["editorId"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `toolbar action omits both preflight fields for malformed document version payload`() {
        assertInvalidToolbarPreflightOmitsAtomicFields("{malformed")
    }

    @Test
    fun `toolbar action omits both preflight fields for missing document version`() {
        assertInvalidToolbarPreflightOmitsAtomicFields(
            JSONObject(atomicRenderUpdateJson("native", "1")).apply {
                remove("documentVersion")
            }
                .toString()
        )
    }

    @Test
    fun `toolbar action omits both preflight fields for noncanonical document version`() {
        assertInvalidToolbarPreflightOmitsAtomicFields(
            JSONObject(atomicRenderUpdateJson("native", "1"))
                .put("documentVersion", "01")
                .toString()
        )
    }

    @Test
    fun `action-only toolbar event omits cached document revision`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val toolbarActionPayloads = mutableListOf<Map<String, Any>>()

        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            view.setLastDocumentVersionForTesting("42")
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayloads += payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertEquals(1, toolbarActionPayloads.size)
            val payload = toolbarActionPayloads.single()
            assertFalse(payload.containsKey("updateJson"))
            assertFalse(payload.containsKey("documentRevision"))
            assertEquals("custom", payload["key"])
            assertEquals(adapter.editorId, payload["editorId"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `autofocus requested before attach applies when editor becomes focusable`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val parent = FrameLayout(activity)
        activity.setContentView(parent)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 66779L
        val editText = view.richTextView.editorEditText

        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.setAutoFocus(true)
        parent.addView(view)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.editorId = editorId

        assertFalse(editText.hasFocus())

        view.applyAutoFocusForTesting()

        assertTrue(editText.hasFocus())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `destroyed editor id invalidates registry and matching view`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77881L

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)

        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("destroyed", preparation.getString("blockedReason"))
        assertEquals(0L, view.richTextView.editorId)
    }

    @Test
    fun `destroyed editor id cannot register a new view`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778811L

        NativeEditorViewRegistry.markEditorCreated(editorId)
        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)

        assertFalse(NativeEditorViewRegistry.register(editorId, view))
        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("destroyed", preparation.getString("blockedReason"))
    }

    @Test
    fun `destroyed editor invalidation from background waits for view cleanup`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77882L
        val completed = AtomicBoolean(false)

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)

        val thread = Thread {
            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
            completed.set(true)
        }
        thread.start()
        shadowOf(Looper.getMainLooper()).idle()
        thread.join(1000)
        shadowOf(Looper.getMainLooper()).idle()

        assertFalse(thread.isAlive)
        assertTrue(completed.get())
        assertEquals(0L, view.richTextView.editorId)
        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("destroyed", preparation.getString("blockedReason"))
    }

    @Test
    fun `destroyed editor invalidation from background times out until main cleanup runs`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77884L
        val completed = AtomicBoolean(false)

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        val editText = view.richTextView.editorEditText
        editText.applyUpdateJSON(renderUpdateJson("ready"), notifyListener = false)
        editText.setSelection(5)
        editText.editorId = editorId
        var insertedText: String? = null
        var syncedSelection: Pair<Int, Int>? = null
        editText.onInsertTextInRustForTesting = { text, _ -> insertedText = text }
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        NativeEditorViewRegistry.register(editorId, view)

        val thread = Thread {
            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
            completed.set(true)
        }
        thread.start()
        thread.join(1000)

        assertFalse(thread.isAlive)
        assertTrue(completed.get())
        assertEquals(editorId, view.richTextView.editorId)
        assertFalse(NativeEditorViewRegistry.register(editorId, view))
        assertTrue(inputConnection!!.commitText("x", 1))
        editText.setSelection(0)
        assertNull(insertedText)
        assertNull(syncedSelection)

        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(0L, view.richTextView.editorId)
        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("destroyed", preparation.getString("blockedReason"))
    }

    @Test
    fun `cleared detached weak owner does not block command preflight forever`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77883L

        NativeEditorViewRegistry.markEditorCreated(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        NativeEditorViewRegistry.unregister(
            editorId,
            view,
            blockCommandsUntilRegistered = true
        )
        NativeEditorViewRegistry.forceDetachedOwnerClearedForTesting(editorId)

        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertTrue(preparation.getBoolean("ready"))
    }

    @Test
    fun `pending controlled update blocks command preflight`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77884L
        val updateJson = renderUpdateJson("")

        view.richTextView.setEditorIdWhileDetached(editorId)
        view.richTextView.editorEditText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        val preparation = JSONObject(view.prepareForEditorCommandJSON())

        assertFalse(preparation.getBoolean("ready"))
        assertEquals("pendingUpdate", preparation.getString("blockedReason"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending controlled update keeps retrying after fast retry budget`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778842L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("recovered")
        val readyPayloads = mutableListOf<Map<String, Any>>()

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.blockEditorUpdatePreflightForTesting = true
        view.onEditorReadyForTesting = { payload ->
            readyPayloads.add(payload)
        }
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onSelectionChangeForTesting = {}
        view.onAddonEventForTesting = {}
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(9)

        view.applyPendingEditorUpdateIfNeeded()
        repeat(6) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))
        }

        assertEquals(updateJson, view.pendingEditorUpdateJsonForTesting())

        view.blockEditorUpdatePreflightForTesting = false
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(1000))

        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(9L, readyPayloads.last()["editorUpdateRevision"])

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `malformed pending editor update is classified once without retrying and is consumed`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<EditorV2Error>()
        val malformedUpdateJson = renderUpdateJson("malformed")
        try {
            adapter.onAutonomousError = { errors += it }
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)
            view.setPendingEditorUpdateJson(malformedUpdateJson)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(21)

            view.applyPendingEditorUpdateIfNeeded()
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(500))

            assertEquals(1, errors.size)
            assertEquals("FFI_RESULT_INVALID", errors.single().code)
            assertNull(view.pendingEditorUpdateJsonForTesting())
            assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `identical malformed pending editor prop redelivery is consumed once`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<EditorV2Error>()
        val malformedUpdateJson = renderUpdateJson("malformed")
        try {
            adapter.onAutonomousError = { errors += it }
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)

            view.setPendingEditorUpdateJson(malformedUpdateJson)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(41)
            view.applyPendingEditorUpdateIfNeeded()

            view.setPendingEditorUpdateJson(malformedUpdateJson)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(41)
            view.applyPendingEditorUpdateIfNeeded()

            assertEquals(1, errors.size)
            assertNull(view.pendingEditorUpdateJsonForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `new malformed pending editor revision is classified independently`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<EditorV2Error>()
        val malformedUpdateJson = renderUpdateJson("malformed")
        try {
            adapter.onAutonomousError = { errors += it }
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)

            view.setPendingEditorUpdateJson(malformedUpdateJson)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(42)
            view.applyPendingEditorUpdateIfNeeded()

            view.setPendingEditorUpdateJson(malformedUpdateJson)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(43)
            view.applyPendingEditorUpdateIfNeeded()

            assertEquals(2, errors.size)
            assertNull(view.pendingEditorUpdateJsonForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `renderer exception permanently consumes pending editor prop without retry`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)
            view.richTextView.editorEditText.throwOnNextApplyUpdateForTesting = IllegalStateException("renderer")
            view.setPendingEditorUpdateJson(atomicRenderUpdateJson("controlled", "0"))
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(44)

            view.applyPendingEditorUpdateIfNeeded()
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(500))

            assertNull(view.pendingEditorUpdateJsonForTesting())
            assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `renderer exception does not schedule a view command retry`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)
            view.richTextView.editorEditText.throwOnNextApplyUpdateForTesting = IllegalStateException("renderer")

            assertFalse(view.applyEditorUpdate(atomicRenderUpdateJson("controlled", "0")))
            assertNull(view.pendingViewCommandUpdateJsonForTesting())
            assertEquals(0, view.pendingViewCommandUpdateRetryAttemptsForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `successful JS editor update preserves queued native update events`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778843L
        val editText = view.richTextView.editorEditText
        val nativeUpdateJson = renderUpdateJson("native")
        val jsUpdateJson = renderUpdateJson("controlled")
        val payloads = mutableListOf<Map<String, Any>>()

        view.onAddonEventForTesting = {}
        view.onEditorUpdateForTesting = { payloads += it }
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("initial"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)
        editText.onSetSelectionScalarInRustForTesting = { _, _ -> }
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)

        view.onEditorUpdate(nativeUpdateJson)

        assertEquals(1, view.pendingEditorUpdateEventCountForTesting())

        val applied = AtomicBoolean(false)
        Handler(Looper.getMainLooper()).post {
            applied.set(view.applyEditorUpdate(jsUpdateJson))
        }
        shadowOf(Looper.getMainLooper()).idle()

        assertTrue(applied.get())
        assertEquals(0, view.pendingEditorUpdateEventCountForTesting())
        assertEquals(1, payloads.size)
        assertEquals(nativeUpdateJson, payloads.single()["updateJson"])
        assertEquals("controlled", editText.text?.toString())

        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))

        assertEquals(0, view.pendingEditorUpdateEventCountForTesting())
        assertTrue(
            editText.imeTraceSnapshotForTesting().any {
                it.contains("nativeViewEditorUpdateDrained")
            }
        )

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `JS editor reset update bypasses preflight and clears stale pending updates`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778844L
        val editText = view.richTextView.editorEditText
        val staleUpdateJson = renderUpdateJson("stale")
        val resetUpdateJson = renderUpdateJson("reset")

        view.onAddonEventForTesting = {}
        view.onEditorUpdateForTesting = {}
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onEditorReadyForTesting = {}
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("before"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(staleUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(8)
        view.scheduleViewCommandUpdateRetryForTesting(staleUpdateJson)
        view.onEditorUpdate(staleUpdateJson)
        view.blockEditorUpdatePreflightForTesting = true

        assertEquals(editorId, view.richTextView.editorId)
        assertEquals(editorId, editText.editorId)
        val editTextShadow = shadowOf(editText)
        editTextShadow.clearWasInvalidated()
        val applied = AtomicBoolean(false)
        Handler(Looper.getMainLooper()).post {
            applied.set(view.applyEditorResetUpdate(resetUpdateJson))
        }
        shadowOf(Looper.getMainLooper()).idle()

        assertTrue(applied.get())
        assertEquals("reset", editText.text?.toString())
        assertTrue(editTextShadow.wasInvalidated())
        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        assertNull(view.pendingViewCommandUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateEventCountForTesting())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending JS editor reset prop applies through reset path and clears stale pending updates`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778845L
        val editText = view.richTextView.editorEditText
        val staleUpdateJson = renderUpdateJson("stale")
        val resetUpdateJson = renderUpdateJson("")

        view.onAddonEventForTesting = {}
        view.onEditorUpdateForTesting = {}
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onEditorReadyForTesting = {}
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("before"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(staleUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(8)
        view.scheduleViewCommandUpdateRetryForTesting(staleUpdateJson)
        view.onEditorUpdate(staleUpdateJson)
        view.setPendingEditorResetUpdateJson(resetUpdateJson)
        view.setPendingEditorResetUpdateEditorId(editorId)
        view.setPendingEditorResetUpdateRevision(9)
        view.blockEditorUpdatePreflightForTesting = true
        val editTextShadow = shadowOf(editText)
        editTextShadow.clearWasInvalidated()

        Handler(Looper.getMainLooper()).post {
            view.applyPendingEditorResetUpdateIfNeeded()
        }
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("", editText.text?.toString())
        assertTrue(editTextShadow.wasInvalidated())
        assertNull(view.pendingEditorResetUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorResetUpdateRevisionForTesting())
        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        assertNull(view.pendingViewCommandUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateEventCountForTesting())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `malformed pending reset update is classified once and preserves valid ordinary pending update`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<EditorV2Error>()
        val ordinaryUpdateJson = atomicRenderUpdateJson("ordinary", "1")
        val malformedResetUpdateJson = renderUpdateJson("malformed reset")
        try {
            adapter.onAutonomousError = { errors += it }
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)
            view.setPendingEditorUpdateJson(ordinaryUpdateJson)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(31)
            view.setPendingEditorResetUpdateJson(malformedResetUpdateJson)
            view.setPendingEditorResetUpdateEditorId(viewToken)
            view.setPendingEditorResetUpdateRevision(32)

            view.applyPendingEditorResetUpdateIfNeeded()
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(500))

            assertEquals(1, errors.size)
            assertEquals("FFI_RESULT_INVALID", errors.single().code)
            assertNull(view.pendingEditorResetUpdateJsonForTesting())
            assertEquals(0, view.pendingEditorResetUpdateRevisionForTesting())
            assertEquals(ordinaryUpdateJson, view.pendingEditorUpdateJsonForTesting())
            assertEquals(31, view.pendingEditorUpdateRevisionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `identical malformed pending reset prop redelivery is consumed once`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val errors = mutableListOf<EditorV2Error>()
        val malformedUpdateJson = renderUpdateJson("malformed reset")
        try {
            adapter.onAutonomousError = { errors += it }
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.richTextView.setEditorIdWhileDetached(viewToken)
            view.richTextView.editorEditText.editorId = viewToken
            view.setAttachedToNativeWindowForTesting(true)

            view.setPendingEditorResetUpdateJson(malformedUpdateJson)
            view.setPendingEditorResetUpdateEditorId(viewToken)
            view.setPendingEditorResetUpdateRevision(51)
            view.applyPendingEditorResetUpdateIfNeeded()

            view.setPendingEditorResetUpdateJson(malformedUpdateJson)
            view.setPendingEditorResetUpdateEditorId(viewToken)
            view.setPendingEditorResetUpdateRevision(51)
            view.applyPendingEditorResetUpdateIfNeeded()

            assertEquals(1, errors.size)
            assertNull(view.pendingEditorResetUpdateJsonForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `pending JS editor reset prop retries when editor view is not ready`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778846L
        val editText = view.richTextView.editorEditText
        val resetUpdateJson = renderUpdateJson("")

        view.onAddonEventForTesting = {}
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onEditorReadyForTesting = {}
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("before"), notifyListener = false)
        editText.editorId = 0L
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorResetUpdateJson(resetUpdateJson)
        view.setPendingEditorResetUpdateEditorId(editorId)
        view.setPendingEditorResetUpdateRevision(10)

        Handler(Looper.getMainLooper()).post {
            view.applyPendingEditorResetUpdateIfNeeded()
        }
        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("before", editText.text?.toString())
        assertEquals(resetUpdateJson, view.pendingEditorResetUpdateJsonForTesting())

        editText.editorId = editorId
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))

        assertEquals("", editText.text?.toString())
        assertNull(view.pendingEditorResetUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorResetUpdateRevisionForTesting())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending JS editor update applies again when the unchanged editor id prop is not redelivered`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778849L
        val editText = view.richTextView.editorEditText

        view.onAddonEventForTesting = {}
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onEditorReadyForTesting = {}
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)

        view.setPendingEditorUpdateJson(renderUpdateJson("first"))
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()

        assertEquals("first", editText.text?.toString())
        assertNull(view.pendingEditorUpdateEditorIdForTesting())

        view.setPendingEditorUpdateJson(renderUpdateJson(""))
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()

        assertEquals("", editText.text?.toString())
        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending JS editor update applies again when only revision changes`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778850L
        val editText = view.richTextView.editorEditText

        view.onAddonEventForTesting = {}
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onEditorReadyForTesting = {}
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)

        view.setPendingEditorUpdateJson(renderUpdateJson(""))
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.applyPendingEditorUpdateIfNeeded()

        assertEquals("", editText.text?.toString())

        editText.applyUpdateJSON(renderUpdateJson("typed"), notifyListener = false)
        view.setPendingEditorUpdateRevision(2)
        view.applyPendingEditorUpdateIfNeeded()

        assertEquals("", editText.text?.toString())
        assertNull(view.pendingEditorUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorUpdateRevisionForTesting())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending JS editor reset prop applies again when only revision changes`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778847L
        val editText = view.richTextView.editorEditText
        val resetUpdateJson = renderUpdateJson("")

        view.onAddonEventForTesting = {}
        view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
        view.onEditorReadyForTesting = {}
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        editText.applyUpdateJSON(renderUpdateJson("first"), notifyListener = false)
        view.setPendingEditorResetUpdateJson(resetUpdateJson)
        view.setPendingEditorResetUpdateEditorId(editorId)
        view.setPendingEditorResetUpdateRevision(1)
        view.applyPendingEditorResetUpdateIfNeeded()

        assertEquals("", editText.text?.toString())
        assertNull(view.pendingEditorResetUpdateJsonForTesting())

        editText.applyUpdateJSON(renderUpdateJson("second"), notifyListener = false)
        view.setPendingEditorResetUpdateRevision(2)
        view.applyPendingEditorResetUpdateIfNeeded()

        assertEquals("", editText.text?.toString())
        assertNull(view.pendingEditorResetUpdateJsonForTesting())
        assertEquals(0, view.pendingEditorResetUpdateRevisionForTesting())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `editor ready payload includes acknowledged update revision`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778841L
        val readyPayloads = mutableListOf<Map<String, Any>>()

        view.richTextView.setEditorIdWhileDetached(editorId)
        view.richTextView.editorEditText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onEditorReadyForTesting = { payload ->
            readyPayloads.add(payload)
        }

        assertTrue(view.emitEditorReadyForTesting(editorUpdateRevision = 4))

        assertEquals(1, readyPayloads.size)
        assertEquals("0", readyPayloads.single()["editorId"])
        assertEquals(4L, readyPayloads.single()["editorUpdateRevision"])

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `editor ready is suppressed while reset update is pending`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778848L
        val readyPayloads = mutableListOf<Map<String, Any>>()

        view.richTextView.setEditorIdWhileDetached(editorId)
        view.richTextView.editorEditText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorResetUpdateJson(renderUpdateJson(""))
        view.setPendingEditorResetUpdateEditorId(editorId)
        view.setPendingEditorResetUpdateRevision(12)
        view.onEditorReadyForTesting = { payload ->
            readyPayloads.add(payload)
        }

        assertFalse(view.emitEditorReadyForTesting(editorUpdateRevision = 12))
        assertTrue(readyPayloads.isEmpty())

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending controlled update parks native toolbar action until cleared`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77885L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        val action = NativeToolbarItem(
            type = ToolbarItemKind.action,
            key = "custom",
            label = "Custom"
        )

        view.handleToolbarItemPressForTesting(action)

        assertTrue(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `parked native toolbar action survives controlled update document version acknowledgement`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779855L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        val acknowledgedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "2")
            .toString()
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setLastDocumentVersionForTesting("1")
        view.onAddonEventForTesting = {}
        view.setPendingEditorUpdateJson(acknowledgedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.isApplyingJSUpdate = true
        view.onEditorUpdate(acknowledgedUpdateJson)
        view.isApplyingJSUpdate = false

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))
        assertFalse(toolbarActionPayload!!.containsKey("updateJson"))
        assertFalse(toolbarActionPayload.containsKey("documentRevision"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `parked native toolbar action is dropped when unrelated document version changes`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779857L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        val acknowledgedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "2")
            .toString()
        val unrelatedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "3")
            .toString()
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setLastDocumentVersionForTesting("1")
        view.setPendingEditorUpdateJson(acknowledgedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onAddonEventForTesting = {}
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.isApplyingJSUpdate = true
        view.onEditorUpdate(unrelatedUpdateJson)
        view.isApplyingJSUpdate = false

        assertFalse(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `parked native mention selection survives controlled update document version acknowledgement`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779856L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val acknowledgedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "2")
            .toString()
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setLastDocumentVersionForTesting("1")
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(acknowledgedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)

        view.isApplyingJSUpdate = true
        view.onEditorUpdate(acknowledgedUpdateJson)
        view.isApplyingJSUpdate = false

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        addonPayload = null
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        val eventJson = JSONObject(addonPayload?.get("eventJson") as String)
        assertEquals("mentionsSelectRequest", eventJson.getString("type"))
        assertEquals("u1", eventJson.getString("suggestionKey"))
        assertEquals("2", eventJson.getString("documentVersion"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `destroyed editor clears parked native toolbar action without emitting callback`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779853L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)
    }

    @Test
    fun `toolbar visibility placement and editability changes clear parked native toolbar action`() {
        val cases = listOf<(NativeEditorExpoView) -> Unit>(
            { view -> view.setShowToolbar(false) },
            { view -> view.setToolbarPlacement("inline") },
            { view -> view.setEditable(false) }
        )

        cases.forEachIndexed { index, clearAction ->
            val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
            val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
            val editorId = 778852L + index
            val editText = view.richTextView.editorEditText
            val updateJson = renderUpdateJson("")
            var toolbarActionPayload: Map<String, Any>? = null

            view.richTextView.setEditorIdWhileDetached(editorId)
            editText.applyUpdateJSON(updateJson, notifyListener = false)
            editText.setSelection(0)
            editText.editorId = editorId
            view.setAttachedToNativeWindowForTesting(true)
            view.setPendingEditorUpdateJson(updateJson)
            view.setPendingEditorUpdateEditorId(editorId)
            view.setPendingEditorUpdateRevision(1)
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayload = payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertTrue(view.hasPendingNativeActionForTesting())

            clearAction(view)
            view.setPendingEditorUpdateJson(null)
            view.setPendingEditorUpdateEditorId(editorId)
            view.setPendingEditorUpdateRevision(2)
            view.wakePendingPreflightWorkForTesting()

            assertFalse(view.hasPendingNativeActionForTesting())
            assertNull(toolbarActionPayload)

            NativeEditorViewRegistry.unregister(editorId, view)
        }
    }

    @Test
    fun `real blur clears parked native toolbar action`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778856L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        host.addView(view)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setCurrentImeBottomForTesting(120)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }
        assertTrue(editText.requestFocus())
        shadowOf(Looper.getMainLooper()).idle()

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        editText.clearFocus()
        shadowOf(Looper.getMainLooper()).idle()
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `toolbar preserved blur keeps parked native toolbar action current while refocus is pending`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778857L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setCurrentImeBottomForTesting(120)
        view.onFocusChangeForTesting = {}
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }
        view.scheduleToolbarRefocusForTesting()
        assertTrue(view.hasPendingToolbarRefocusForTesting())

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `keyboard toolbar becoming invisible clears parked native toolbar action`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778858L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setCurrentImeBottomForTesting(0)
        view.updateAttachedKeyboardToolbarForInsetsForTesting()
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `read only native toolbar and mention callbacks are consumed without mutation`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778859L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var toolbarActionPayload: Map<String, Any>? = null
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }
        addonPayload = null

        view.setEditable(false)
        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )
        view.insertMentionSuggestionForTesting(suggestion)

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)
        assertNull(addonPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `toolbar config change clears parked native toolbar action`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778851L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setToolbarItemsJson(
            JSONArray()
                .put(
                    JSONObject()
                        .put("type", "action")
                        .put("key", "other")
                        .put("label", "Other")
                )
                .toString()
        )
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending controlled update parks native mention selection until cleared`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77886L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        val eventJson = JSONObject(addonPayload?.get("eventJson") as String)
        assertEquals("mentionsSelectRequest", eventJson.getString("type"))
        assertEquals("u1", eventJson.getString("suggestionKey"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending native mention action is parked after retry budget and wakes later`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779865L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        editText.blockExternalEditorCommandPreparationForTesting = true
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null

        view.insertMentionSuggestionForTesting(suggestion)
        repeat(4) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(16))
        }

        assertTrue(view.hasPendingNativeActionForTesting())
        assertTrue(view.pendingNativeActionRetryAttemptsForTesting() >= 3)

        editText.blockExternalEditorCommandPreparationForTesting = false
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        val eventJson = JSONObject(addonPayload?.get("eventJson") as String)
        assertEquals("mentionsSelectRequest", eventJson.getString("type"))
        assertEquals("u1", eventJson.getString("suggestionKey"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `destroyed editor clears parked native mention selection without emitting callback`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779862L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)
    }

    @Test
    fun `addons config change clears parked native mention selection`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778861L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put("suggestions", JSONArray())
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `view command update retry attempts advance instead of resetting for same payload`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = """{"renderElements":[],"selection":{"type":"text","anchor":0,"head":0}}"""

        view.richTextView.setEditorIdWhileDetached(44556L)
        view.scheduleViewCommandUpdateRetryForTesting(updateJson)

        assertEquals(updateJson, view.pendingViewCommandUpdateJsonForTesting())
        assertEquals(1, view.pendingViewCommandUpdateRetryAttemptsForTesting())

        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(20))

        assertEquals(updateJson, view.pendingViewCommandUpdateJsonForTesting())
        assertEquals(2, view.pendingViewCommandUpdateRetryAttemptsForTesting())

        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(32))

        assertEquals(updateJson, view.pendingViewCommandUpdateJsonForTesting())
        assertEquals(3, view.pendingViewCommandUpdateRetryAttemptsForTesting())
    }

    @Test
    fun `theme update applies when preflight is ready`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#ff0000"}"""

        view.setThemeJson(themeJson)

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(themeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `theme update queues latest value while preflight is blocked`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val firstThemeJson = """{"backgroundColor":"#00ff00"}"""
        val latestThemeJson = """{"backgroundColor":"#0000ff"}"""

        view.blockThemePreflightForTesting = true
        view.setThemeJson(firstThemeJson)
        view.setThemeJson(latestThemeJson)

        assertEquals(latestThemeJson, view.pendingThemeJsonForTesting())
        assertNull(view.lastThemeJsonForTesting())

        view.blockThemePreflightForTesting = false
        view.applyPendingThemeForTesting()

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(latestThemeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `scheduled theme retry applies pending theme after preflight unblocks`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#112233"}"""

        view.blockThemePreflightForTesting = true
        view.setThemeJson(themeJson)
        assertEquals(themeJson, view.pendingThemeJsonForTesting())

        view.blockThemePreflightForTesting = false
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(16))

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(themeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `theme retry is bounded while preflight remains blocked`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#112233"}"""

        view.blockThemePreflightForTesting = true
        view.setThemeJson(themeJson)

        repeat(10) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))
        }

        assertEquals(themeJson, view.pendingThemeJsonForTesting())
        assertTrue(view.pendingThemeRetryAttemptsForTesting() <= 5)
        assertNull(view.lastThemeJsonForTesting())
    }

    @Test
    fun `theme update wakes after retry budget is exhausted`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#445566"}"""

        view.blockThemePreflightForTesting = true
        view.setThemeJson(themeJson)

        repeat(10) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))
        }

        assertEquals(themeJson, view.pendingThemeJsonForTesting())
        assertTrue(view.pendingThemeRetryAttemptsForTesting() <= 5)
        assertNull(view.lastThemeJsonForTesting())

        view.blockThemePreflightForTesting = false
        view.wakePendingPreflightWorkForTesting()

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(themeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `theme update can clear an applied theme with null json`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#ff0000"}"""

        view.setThemeJson(themeJson)
        view.setThemeJson(null)

        assertNull(view.pendingThemeJsonForTesting())
        assertNull(view.lastThemeJsonForTesting())
    }

    @Test
    fun `blur retries preflight until it unblocks`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText

        editText.blockExternalEditorUpdatePreparationForTesting = true

        view.performBlurForTesting()

        assertEquals(1, view.pendingBlurRetryAttemptsForTesting())

        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(20))

        assertEquals(2, view.pendingBlurRetryAttemptsForTesting())

        editText.blockExternalEditorUpdatePreparationForTesting = false
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(32))

        assertEquals(0, view.pendingBlurRetryAttemptsForTesting())
    }

    @Test
    fun `blur retry clears when editor is destroyed before preflight unblocks`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779900L
        val editText = view.richTextView.editorEditText

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        editText.editorId = editorId
        editText.blockExternalEditorUpdatePreparationForTesting = true

        view.performBlurForTesting()

        assertEquals(1, view.pendingBlurRetryAttemptsForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(20))

        assertEquals(0, view.pendingBlurRetryAttemptsForTesting())
        assertEquals(0L, view.richTextView.editorId)
        assertEquals(0L, editText.editorId)
    }

    @Test
    fun `destroyed editor cancels pending outside tap keyboard dismiss and preflight wake`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779902L
        val editText = view.richTextView.editorEditText

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        NativeEditorViewRegistry.register(editorId, view)

        view.scheduleOutsideTapBlurFromWindowDispatcher()
        view.performBlurForTesting(deferKeyboardDismiss = true)
        view.schedulePendingPreflightWakeForTesting()

        assertTrue(view.hasPendingOutsideTapBlurForTesting())
        assertTrue(view.hasPendingKeyboardDismissForTesting())
        assertTrue(view.hasPendingPreflightWakeForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)

        assertFalse(view.hasPendingOutsideTapBlurForTesting())
        assertFalse(view.hasPendingKeyboardDismissForTesting())
        assertFalse(view.hasPendingPreflightWakeForTesting())
        assertFalse(view.isKeyboardToolbarAttachedForTesting())
        assertEquals(0L, view.richTextView.editorId)
        assertEquals(0L, editText.editorId)
    }

    @Test
    fun `destroyed editor cancels pending toolbar refocus`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779903L
        val editText = view.richTextView.editorEditText

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        NativeEditorViewRegistry.register(editorId, view)

        view.scheduleToolbarRefocusForTesting()

        assertTrue(view.hasPendingToolbarRefocusForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        shadowOf(Looper.getMainLooper()).idle()

        assertFalse(view.hasPendingToolbarRefocusForTesting())
        assertFalse(editText.hasFocus())
        assertEquals(0L, view.richTextView.editorId)
        assertEquals(0L, editText.editorId)
    }

    @Test
    fun `outside tap route is shared per window and restores the latest foreign callback`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val firstExpoContext = testExpoContext(activity)
        val secondExpoContext = testExpoContext(activity)
        val firstView = NativeEditorExpoView(firstExpoContext.context, firstExpoContext.appContext)
        val secondView = NativeEditorExpoView(secondExpoContext.context, secondExpoContext.appContext)
        val originalChildCount = host.childCount
        val originalCallback = activity.window.callback
        val firstForeignCallback = object : Window.Callback by originalCallback {}
        val latestForeignCallback = object : Window.Callback by originalCallback {}
        activity.window.callback = firstForeignCallback

        firstView.installOutsideTapBlurHandlerForTesting()
        val firstRoute = activity.window.callback
        assertEquals(originalChildCount, host.childCount)
        assertFalse(firstRoute === firstForeignCallback)

        secondView.installOutsideTapBlurHandlerForTesting()

        assertEquals(originalChildCount, host.childCount)
        assertSame(firstRoute, activity.window.callback)

        firstView.uninstallOutsideTapBlurHandlerForTesting()

        assertSame(firstRoute, activity.window.callback)

        activity.window.callback = latestForeignCallback
        secondView.installOutsideTapBlurHandlerForTesting()
        assertFalse(activity.window.callback === latestForeignCallback)

        secondView.uninstallOutsideTapBlurHandlerForTesting()

        assertSame(latestForeignCallback, activity.window.callback)
    }

    @Test
    fun `outside tap route forwards the exact foreign result and confirms a real tap on up`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val trace = mutableListOf<String>()
        val originalCallback = activity.window.callback
        var foreignDispatchCount = 0
        val foreignCallback = object : Window.Callback by originalCallback {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                foreignDispatchCount += 1
                return true
            }
        }
        activity.window.callback = foreignCallback
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        host.layout(0, 0, 1000, 1000)
        view.layout(0, 0, 200, 200)
        view.richTextView.layout(0, 0, 200, 200)
        view.richTextView.editorEditText.layout(0, 0, 200, 200)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onOutsideTapTraceForTesting = { event -> trace.add(event) }

        view.installOutsideTapBlurHandlerForTesting()
        val down = MotionEvent.obtain(100L, 100L, MotionEvent.ACTION_DOWN, 9999f, 9999f, 0)
        val up = MotionEvent.obtain(100L, 116L, MotionEvent.ACTION_UP, 9999f, 9999f, 0)
        val downHandled = view.dispatchOutsideTapWindowEventForTesting(down)
        val upHandled = view.dispatchOutsideTapWindowEventForTesting(up)
        down.recycle()
        up.recycle()

        assertTrue(downHandled)
        assertTrue(upHandled)
        assertEquals(2, foreignDispatchCount)
        assertTrue(trace.joinToString(separator = "\n"), view.hasPendingOutsideTapBlurForTesting())

        view.cancelOutsideTapBlurFromWindowDispatcher()
        view.uninstallOutsideTapBlurHandlerForTesting()
    }

    @Test
    fun `outside tap route observes once through a foreign wrapper around an old route`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val trace = mutableListOf<String>()
        val originalCallback = activity.window.callback
        var baseDispatchCount = 0
        val baseCallback = object : Window.Callback by originalCallback {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                baseDispatchCount += 1
                return false
            }
        }
        activity.window.callback = baseCallback
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        host.layout(0, 0, 1000, 1000)
        view.layout(0, 0, 200, 200)
        view.richTextView.layout(0, 0, 200, 200)
        view.richTextView.editorEditText.layout(0, 0, 200, 200)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onOutsideTapTraceForTesting = { event -> trace.add(event) }

        view.installOutsideTapBlurHandlerForTesting()
        val oldRoute = activity.window.callback
        var foreignDispatchCount = 0
        val foreignCallback = object : Window.Callback by oldRoute {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                foreignDispatchCount += 1
                return oldRoute.dispatchTouchEvent(event)
            }
        }
        activity.window.callback = foreignCallback
        view.installOutsideTapBlurHandlerForTesting()

        val down = MotionEvent.obtain(100L, 100L, MotionEvent.ACTION_DOWN, 9999f, 9999f, 0)
        val handled = view.dispatchOutsideTapWindowEventForTesting(down)
        down.recycle()

        assertFalse(handled)
        assertEquals(1, foreignDispatchCount)
        assertEquals(1, baseDispatchCount)
        assertEquals(
            trace.joinToString(separator = "\n"),
            1,
            trace.count { it.startsWith("dispatch callback action=") }
        )

        view.cancelOutsideTapBlurFromWindowDispatcher()
        view.uninstallOutsideTapBlurHandlerForTesting()

        assertSame(foreignCallback, activity.window.callback)
    }

    @Test
    fun `outside tap route breaks dynamic foreign callback re-entry once`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val trace = mutableListOf<String>()
        val originalCallback = activity.window.callback
        var baseDispatchCount = 0
        val baseCallback = object : Window.Callback by originalCallback {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                baseDispatchCount += 1
                return false
            }
        }
        activity.window.callback = baseCallback
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        host.layout(0, 0, 1000, 1000)
        view.layout(0, 0, 200, 200)
        view.richTextView.layout(0, 0, 200, 200)
        view.richTextView.editorEditText.layout(0, 0, 200, 200)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onOutsideTapTraceForTesting = { event -> trace.add(event) }

        view.installOutsideTapBlurHandlerForTesting()
        val oldRoute = activity.window.callback
        var foreignDispatchCount = 0
        var foreignInnerResult: Boolean? = null
        val foreignCallback = object : Window.Callback by oldRoute {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                foreignDispatchCount += 1
                return activity.window.callback.dispatchTouchEvent(event).also { result ->
                    foreignInnerResult = result
                }.not()
            }
        }
        activity.window.callback = foreignCallback
        view.installOutsideTapBlurHandlerForTesting()
        var cycleBreakDispatchCount = 0
        assertTrue(
            view.setOutsideTapCycleBreakDispatcherForTesting {
                cycleBreakDispatchCount += 1
                true
            }
        )

        val down = MotionEvent.obtain(100L, 100L, MotionEvent.ACTION_DOWN, 9999f, 9999f, 0)
        val handled = view.dispatchOutsideTapWindowEventForTesting(down)
        down.recycle()

        assertTrue(requireNotNull(foreignInnerResult))
        assertFalse(handled)
        assertEquals(1, foreignDispatchCount)
        assertEquals(1, cycleBreakDispatchCount)
        assertEquals(0, baseDispatchCount)
        assertEquals(
            trace.joinToString(separator = "\n"),
            1,
            trace.count { it.startsWith("dispatch callback action=") }
        )

        view.cancelOutsideTapBlurFromWindowDispatcher()
        view.uninstallOutsideTapBlurHandlerForTesting()

        assertSame(foreignCallback, activity.window.callback)
    }

    @Test
    fun `outside tap route clears a pruned final weak view and restores the latest foreign callback`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val originalCallback = activity.window.callback
        val firstForeignCallback = object : Window.Callback by originalCallback {}
        activity.window.callback = firstForeignCallback

        view.installOutsideTapBlurHandlerForTesting()
        val oldRoute = activity.window.callback
        val latestForeignCallback = object : Window.Callback by oldRoute {}
        activity.window.callback = latestForeignCallback
        view.installOutsideTapBlurHandlerForTesting()

        val cleanup = view.clearOutsideTapRouteViewReferenceAndReconcileForTesting()

        assertFalse(cleanup.isRegistered)
        assertFalse(cleanup.hasCallbackReconciler)
        assertSame(latestForeignCallback, activity.window.callback)

        view.uninstallOutsideTapBlurHandlerForTesting()
    }

    @Test
    fun `outside tap route cancels an outside blur candidate when a gesture moves like scroll`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        host.layout(0, 0, 1000, 1000)
        view.layout(0, 0, 200, 200)
        view.richTextView.layout(0, 0, 200, 200)
        view.richTextView.editorEditText.layout(0, 0, 200, 200)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}

        view.installOutsideTapBlurHandlerForTesting()
        val down = MotionEvent.obtain(100L, 100L, MotionEvent.ACTION_DOWN, 9999f, 9999f, 0)
        val move = MotionEvent.obtain(100L, 116L, MotionEvent.ACTION_MOVE, 9999f, 10099f, 0)
        view.dispatchOutsideTapWindowEventForTesting(down)
        view.dispatchOutsideTapWindowEventForTesting(move)
        down.recycle()
        move.recycle()

        assertFalse(view.hasPendingOutsideTapBlurForTesting())

        view.uninstallOutsideTapBlurHandlerForTesting()
    }

    @Test
    fun `outside tap handler reinstall does not duplicate route for the same view`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        host.addView(view)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}

        view.installOutsideTapBlurHandlerForTesting()
        val route = activity.window.callback

        view.installOutsideTapBlurHandlerForTesting()

        assertTrue(view.isOutsideTapBlurHandlerInstalledForTesting())
        assertSame(route, activity.window.callback)

        val event = MotionEvent.obtain(100L, 100L, MotionEvent.ACTION_DOWN, 9999f, 9999f, 0)
        assertEquals(
            NativeEditorOutsideTapDecision.OUTSIDE_EDITOR,
            view.prepareOutsideTapDecisionForWindowEvent(event)
        )
        event.recycle()

        view.cancelOutsideTapBlurFromWindowDispatcher()
        view.uninstallOutsideTapBlurHandlerForTesting()
    }

    @Test
    fun `detach clears keyboard toolbar viewport inset`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        view.richTextView.setViewportBottomInsetPx(42)
        view.setCurrentImeBottomForTesting(120)

        assertEquals(42, view.richTextView.viewportBottomInsetPxForTesting())
        assertEquals(120, view.currentImeBottomForTesting())

        view.handleDetachedFromWindowForTesting()

        assertEquals(0, view.richTextView.viewportBottomInsetPxForTesting())
        assertEquals(0, view.currentImeBottomForTesting())
    }

    @Test
    fun `toolbar theme refreshes fixed viewport inset`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778895L
        val editText = view.richTextView.editorEditText

        host.addView(view, FrameLayout.LayoutParams(360, 480))
        view.measure(
            android.view.View.MeasureSpec.makeMeasureSpec(360, android.view.View.MeasureSpec.EXACTLY),
            android.view.View.MeasureSpec.makeMeasureSpec(480, android.view.View.MeasureSpec.EXACTLY)
        )
        view.layout(0, 0, 360, 480)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setCurrentImeBottomForTesting(160)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        assertTrue(editText.requestFocus())
        shadowOf(Looper.getMainLooper()).idle()
        editText.editorId = 0L
        view.richTextView.setViewportBottomInsetPx(1)

        view.setThemeJson("""{"toolbar":{"appearance":"native"}}""")
        shadowOf(Looper.getMainLooper()).idle()

        assertTrue(view.richTextView.viewportBottomInsetPxForTesting() > 1)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `auto grow content height re-emits when editor id changes`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val events = mutableListOf<Map<String, Any>>()

        view.onContentHeightChangeForTesting = { event ->
            events.add(event)
        }
        view.setHeightBehavior("autoGrow")
        editText.applyUpdateJSON(
            renderUpdateJson("Line one\nLine two\nLine three"),
            notifyListener = false
        )

        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            360,
            android.view.View.MeasureSpec.EXACTLY
        )
        val heightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            0,
            android.view.View.MeasureSpec.UNSPECIFIED
        )
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        assertTrue(events.isNotEmpty())
        val initialEvent = events.last()
        val contentHeight = initialEvent["contentHeight"] as Int
        assertEquals("0", initialEvent["editorId"])

        events.clear()
        val editorId = 779902L

        view.setEditorId(editorId)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)

        assertEquals(1, events.size)
        assertEquals(contentHeight, events.single()["contentHeight"])
        assertEquals("0", events.single()["editorId"])

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `detach preflight flushes pending composition before unregistering`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText

        view.richTextView.setEditorIdWhileDetached(77889L)
        editText.setSelection(0)
        editText.editorId = 77889L

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
            editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
        }

        val inputConnection = editText.onCreateInputConnection(android.view.inputmethod.EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        view.handleDetachedFromWindowForTesting()

        assertEquals("abc", insertedText)

        NativeEditorViewRegistry.unregister(77889L, view)
    }

    @Test
    fun `detach retry clears when editor is destroyed before preflight unblocks`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779901L
        val editText = view.richTextView.editorEditText

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        editText.editorId = editorId
        editText.blockExternalEditorUpdatePreparationForTesting = true

        view.handleDetachedFromWindowForTesting()

        assertEquals(1, view.pendingDetachPreflightRetryAttemptsForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(20))

        assertEquals(0, view.pendingDetachPreflightRetryAttemptsForTesting())
        assertEquals(0L, view.richTextView.editorId)
        assertEquals(0L, editText.editorId)
    }

    @Test
    fun `child detach preflight flushes pending composition before editor unbind`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val parent = FrameLayout(activity)
        activity.setContentView(parent)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val editorId = 778891L

        parent.addView(view)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
            editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
        }

        val inputConnection = editText.onCreateInputConnection(android.view.inputmethod.EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        parent.removeView(view)

        assertEquals("abc", insertedText)
        assertEquals(0L, editText.editorId)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending native toolbar action is parked after retry budget and wakes later`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText

        view.richTextView.setEditorIdWhileDetached(88990L)
        editText.setSelection(0)
        editText.editorId = 88990L
        editText.blockExternalEditorCommandPreparationForTesting = true
        var toolbarActionPayload: Map<String, Any>? = null
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        val action = NativeToolbarItem(
            type = ToolbarItemKind.action,
            key = "custom",
            label = "Custom"
        )

        view.handleToolbarItemPressForTesting(action)
        repeat(4) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(16))
        }

        assertTrue(view.hasPendingNativeActionForTesting())
        assertTrue(view.pendingNativeActionRetryAttemptsForTesting() >= 3)

        editText.blockExternalEditorCommandPreparationForTesting = false
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))
        assertEquals("0", toolbarActionPayload?.get("editorId"))

        NativeEditorViewRegistry.unregister(88990L, view)
    }

    @Test
    fun `view command update wakes after retry budget is exhausted`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 88991L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("next")

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("before"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.blockEditorUpdatePreflightForTesting = true

        assertFalse(view.applyEditorUpdate(updateJson))

        repeat(10) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))
        }

        assertEquals(updateJson, view.pendingViewCommandUpdateJsonForTesting())
        assertTrue(view.pendingViewCommandUpdateRetryAttemptsForTesting() <= 5)
        assertEquals("before", editText.text?.toString())

        view.blockEditorUpdatePreflightForTesting = false
        view.wakePendingPreflightWorkForTesting()
        shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(16))

        assertNull(view.pendingViewCommandUpdateJsonForTesting())
        assertEquals("next", editText.text?.toString())
    }

    @Test
    fun `off main view command update is ignored after editor rebind`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val firstEditorId = 99101L
        val secondEditorId = 99102L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("stale")

        view.richTextView.setEditorIdWhileDetached(firstEditorId)
        editText.applyUpdateJSON(renderUpdateJson("first"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = firstEditorId
        view.setAttachedToNativeWindowForTesting(true)

        val posted = CountDownLatch(1)
        Thread {
            view.applyEditorUpdate(updateJson)
            posted.countDown()
        }.start()
        assertTrue(posted.await(2, java.util.concurrent.TimeUnit.SECONDS))

        view.richTextView.setEditorIdWhileDetached(secondEditorId)
        editText.applyUpdateJSON(renderUpdateJson("second"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = secondEditorId

        shadowOf(Looper.getMainLooper()).idle()

        assertEquals("second", editText.text?.toString())
        assertNull(view.pendingViewCommandUpdateJsonForTesting())
    }

    @Test
    fun `interrupted running off main preflight returns completed result`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 99103L
        val started = CountDownLatch(1)
        val release = CountDownLatch(1)
        val result = AtomicReference<String>()

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        view.richTextView.editorEditText.applyUpdateJSON(renderUpdateJson("ready"), notifyListener = false)
        view.richTextView.editorEditText.setSelection(0)
        view.richTextView.editorEditText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onBeforePrepareForEditorCommandForTesting = {
            started.countDown()
            assertTrue(release.await(2, java.util.concurrent.TimeUnit.SECONDS))
        }
        NativeEditorViewRegistry.register(editorId, view)

        val worker = Thread {
            result.set(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        }
        val interrupter = Thread {
            assertTrue(started.await(2, java.util.concurrent.TimeUnit.SECONDS))
            worker.interrupt()
            release.countDown()
        }

        worker.start()
        interrupter.start()
        val mainLooper = shadowOf(Looper.getMainLooper())
        val deadlineNanos = System.nanoTime() + java.util.concurrent.TimeUnit.SECONDS.toNanos(2)
        while (started.count > 0 && worker.isAlive && System.nanoTime() < deadlineNanos) {
            mainLooper.idle()
            if (started.count > 0) {
                Thread.sleep(10)
            }
        }
        worker.join(2000)
        interrupter.join(2000)

        val preparation = JSONObject(result.get())
        assertTrue(preparation.getBoolean("ready"))

        NativeEditorViewRegistry.unregister(editorId, view)
        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
    }

    @Test
    fun `mutating controlled update preflight preserves committed composition and next input`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val editText = view.richTextView.editorEditText
        try {
            view.onAddonEventForTesting = {}
            view.onEditorUpdateForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            editText.setSelection(0)
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)
            assertTrue(inputConnection!!.setComposingText("native", 1))
            val renderCallsBeforeControlledUpdate = adapter.renderUpdateCallCountForTesting

            assertTrue(view.applyEditorUpdate(atomicRenderUpdateJson("controlled", "0")))

            assertEquals(
                "backend calls=${backend.calls}",
                renderCallsBeforeControlledUpdate + 2,
                adapter.renderUpdateCallCountForTesting
            )
            assertEquals("native", editText.text?.toString())

            editText.setSelection(editText.text?.length ?: 0)
            val nextInputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(nextInputConnection)
            assertTrue(nextInputConnection!!.commitText("!", 1))
            assertEquals("native!", editText.text?.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external atomic view update keeps adopted selection through toolbar state refresh without state reads`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val session = backend.sessions.getValue(adapter.editorId)
        session.text = StringBuilder("ab")
        session.anchor = 2
        session.head = 2
        session.revision = 1u
        val externalSnapshot = JSONObject(atomicRenderUpdateJson("ab", "1"))
            .put(
                "selection",
                JSONObject()
                    .put("type", "text")
                    .put("anchor", 2)
                    .put("head", 2)
                    .put("anchorScalar", 2)
                    .put("headScalar", 2)
            )
            .toString()
        try {
            view.onAddonEventForTesting = {}
            view.onEditorUpdateForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)

            assertTrue(view.applyEditorUpdate(externalSnapshot))
            backend.calls.clear()

            val state = JSONObject(
                requireNotNull(view.refreshToolbarStateFromEditorSelectionForTesting())
            )
            assertEquals(2, state.getJSONObject("selection").getInt("anchorScalar"))
            assertEquals(0, backend.calls.count { it == "getState" })
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `prop update generated by adapter refresh renders remote replace without typing`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val editText = view.richTextView.editorEditText
        try {
            view.onAddonEventForTesting = {}
            view.onEditorUpdateForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)

            val session = backend.sessions.getValue(adapter.editorId)
            session.text = StringBuilder("Remote replace sync")
            session.revision += 1u
            val update = requireNotNull(adapter.refreshFromRustState(null))
            assertTrue(JSONObject(update).has("selection"))
            assertTrue(JSONObject(update).has("scalarLength"))

            view.setPendingEditorUpdateJson(update)
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(1)
            view.applyPendingEditorUpdateIfNeeded()

            assertEquals("Remote replace sync", editText.text?.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition event carries bound editor identity`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            view.onExternalTextCompositionEndForTesting = events::add
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            adapter.setContentHtml("<p>arrival</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.richTextView.editorEditText.setSelection(0, 7)

            view.beginExternalTextComposition("speech-1")
            view.updateExternalTextComposition("speech-1", "on arrival")
            view.commitExternalTextComposition("speech-1", "O/A")

            assertEquals(1, events.size)
            assertEquals(adapter.editorId, events.single()["editorId"])
            assertNotNull(events.single()["resultJson"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition merges a remote first change after deferred registry refresh`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            """{"initialization":{"type":"room","documentId":"doc","lineageId":"lineage"}}""",
            null
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = true
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        val updates = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            adapter.setContentHtml("<p>abc</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.onEditorUpdateForTesting = updates::add
            val editText = view.richTextView.editorEditText
            editText.setSelection(1, 2)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            updates.clear()
            view.beginExternalTextComposition("speech-remote-first")
            view.updateExternalTextComposition("speech-remote-first", "X")
            val session = backend.sessions.getValue(adapter.editorId)
            val outboxBeforeRemote = session.outbox.size
            session.text.insert(0, "Z")
            session.revision += 1u
            backend.calls.clear()

            NativeEditorViewRegistry.rebaseAfterRemoteCommit(adapter.editorId)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals("aXc", editText.text.toString())
            assertEquals(0, backend.calls.count { it == "renderNative" })

            val resultJson = view.commitExternalTextComposition("speech-remote-first", "Y")
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val result = JSONObject(resultJson)

            assertEquals("committed", result.getString("outcome"))
            assertEquals("consumer", result.getString("cause"))
            assertFalse(result.has("error"))
            assertEquals("ZaYc", session.text.toString())
            assertEquals("ZaYc", editText.text.toString())
            assertEquals(1, backend.calls.count { it == "applyNativeIntent" })
            assertEquals(outboxBeforeRemote + 1, session.outbox.size)
            assertEquals(1, updates.size)
            assertEquals(1, events.size)
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition remote first no-op adopts render without local update`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            """{"initialization":{"type":"room","documentId":"doc","lineageId":"lineage"}}""",
            null
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = true
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        val updates = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            adapter.setContentHtml("<p>abc</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.onEditorUpdateForTesting = updates::add
            val editText = view.richTextView.editorEditText
            editText.setSelection(1, 2)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            updates.clear()
            view.beginExternalTextComposition("speech-remote-noop")
            view.updateExternalTextComposition("speech-remote-noop", "X")
            val session = backend.sessions.getValue(adapter.editorId)
            session.text.insert(0, "Z")
            session.revision += 1u
            val outboxBeforeCommit = session.outbox.size
            backend.calls.clear()

            NativeEditorViewRegistry.rebaseAfterRemoteCommit(adapter.editorId)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            val resultJson = view.commitExternalTextComposition("speech-remote-noop", "b")
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val result = JSONObject(resultJson)

            assertEquals("committed", result.getString("outcome"))
            assertEquals("consumer", result.getString("cause"))
            assertFalse(result.has("error"))
            assertEquals("Zabc", session.text.toString())
            assertEquals("Zabc", editText.text.toString())
            assertEquals(1, backend.calls.count { it == "applyNativeIntent" })
            assertEquals(outboxBeforeCommit, session.outbox.size)
            assertTrue(updates.isEmpty())
            assertEquals(1, events.size)
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition remote first collapsed empty remaps caret without local update`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val created = backend.create(
            """{"initialization":{"type":"room","documentId":"doc","lineageId":"lineage"}}""",
            null
        ) as EditorV2CallResult.Ok
        val adapter = EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = true
        )!!
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        val updates = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            adapter.setContentHtml("<p>abc</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.onEditorUpdateForTesting = updates::add
            val editText = view.richTextView.editorEditText
            editText.setSelection(2)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            updates.clear()
            view.beginExternalTextComposition("speech-remote-empty")
            view.updateExternalTextComposition("speech-remote-empty", "X")
            val session = backend.sessions.getValue(adapter.editorId)
            session.text.insert(0, "Z")
            session.revision += 1u
            val outboxBeforeCommit = session.outbox.size
            backend.calls.clear()

            NativeEditorViewRegistry.rebaseAfterRemoteCommit(adapter.editorId)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals("abXc", editText.text.toString())
            val resultJson = view.commitExternalTextComposition("speech-remote-empty", "")
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val result = JSONObject(resultJson)

            assertEquals("committed", result.getString("outcome"))
            assertEquals("consumer", result.getString("cause"))
            assertFalse(result.has("error"))
            assertEquals("Zabc", session.text.toString())
            assertEquals("Zabc", editText.text.toString())
            assertEquals(3, editText.selectionStart)
            assertEquals(3, editText.selectionEnd)
            assertEquals(1, backend.calls.count { it == "applyNativeIntent" })
            assertEquals(outboxBeforeCommit, session.outbox.size)
            assertTrue(updates.isEmpty())
            assertEquals(1, events.size)
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `reset cancels external composition without document mutation`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            view.onExternalTextCompositionEndForTesting = events::add
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            adapter.setContentHtml("<p>arrival</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.richTextView.editorEditText.setSelection(0, 7)
            view.beginExternalTextComposition("speech-1")
            view.updateExternalTextComposition("speech-1", "O/A")

            view.applyEditorResetUpdate(
                atomicRenderUpdateJson("reset", (adapter.baseDocumentRevision + 1u).toString())
            )

            assertFalse(backend.sessions.getValue(adapter.editorId).text.contains("O/A"))
            assertEquals(1, events.size)
            val result = JSONObject(events.single()["resultJson"] as String)
            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("documentChange", result.getString("cause"))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition rebind cancels with old editor identity`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val firstAdapter = attachAdapterForViewTest(backend)
        val secondAdapter = attachAdapterForViewTest(backend)
        val firstViewToken = EditorV2Registry.register(firstAdapter)
        val secondViewToken = EditorV2Registry.register(secondAdapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, firstAdapter, firstViewToken, events)
            view.beginExternalTextComposition("speech-rebind")
            view.updateExternalTextComposition("speech-rebind", "O/A")

            view.setEditorId(secondViewToken)

            assertFalse(backend.sessions.getValue(firstAdapter.editorId).text.contains("O/A"))
            assertEquals(1, events.size)
            assertEquals(firstAdapter.editorId, events.single()["editorId"])
            val result = JSONObject(events.single()["resultJson"] as String)
            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("lifecycle", result.getString("cause"))
        } finally {
            EditorV2Registry.remove(firstAdapter.editorId)
            EditorV2Registry.remove(secondAdapter.editorId)
            NativeEditorViewRegistry.unregister(firstViewToken, view)
            NativeEditorViewRegistry.unregister(secondViewToken, view)
        }
    }

    @Test
    fun `external composition destroy cancels once with old editor identity`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            view.beginExternalTextComposition("speech-destroy")
            view.updateExternalTextComposition("speech-destroy", "O/A")

            EditorV2Registry.dropPair(adapter.editorId)
            NativeEditorViewRegistry.invalidateDestroyedEditor(viewToken)
            view.handleEditorDestroyed(viewToken)

            assertEquals(0L, view.richTextView.editorId)
            assertEquals(1, events.size)
            assertEquals(adapter.editorId, events.single()["editorId"])
            val result = JSONObject(events.single()["resultJson"] as String)
            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("lifecycle", result.getString("cause"))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition read only cancels once without mutation`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            val revisionBefore = adapter.baseDocumentRevision
            view.beginExternalTextComposition("speech-read-only")
            view.updateExternalTextComposition("speech-read-only", "O/A")

            view.setEditable(false)
            view.setEditable(false)

            assertFalse(view.richTextView.editorEditText.isEditable)
            assertEquals(revisionBefore, adapter.baseDocumentRevision)
            assertEquals("arrival", backend.sessions.getValue(adapter.editorId).text.toString())
            assertEquals("arrival", view.richTextView.editorEditText.text.toString())
            assertEquals(1, events.size)
            assertEquals(adapter.editorId, events.single()["editorId"])
            val result = JSONObject(events.single()["resultJson"] as String)
            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("lifecycle", result.getString("cause"))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition final unbind after detach cancels once`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            view.beginExternalTextComposition("speech-unbind")
            view.updateExternalTextComposition("speech-unbind", "O/A")

            view.handleDetachedFromWindowForTesting()
            assertTrue(events.isEmpty())
            view.richTextView.unbindEditorForDetachedViewIfNeeded()
            view.richTextView.unbindEditorForDetachedViewIfNeeded()

            assertEquals(1, events.size)
            assertEquals(adapter.editorId, events.single()["editorId"])
            val result = JSONObject(events.single()["resultJson"] as String)
            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("lifecycle", result.getString("cause"))
            assertEquals("arrival", backend.sessions.getValue(adapter.editorId).text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition temporary detach is non terminal`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            view.beginExternalTextComposition("speech-detach")
            view.updateExternalTextComposition("speech-detach", "O/A")

            view.handleDetachedFromWindowForTesting()

            assertTrue(events.isEmpty())
            assertEquals("arrival", backend.sessions.getValue(adapter.editorId).text.toString())
            assertEquals("O/A", view.richTextView.editorEditText.text.toString())

            view.handleAttachedToWindowForTesting()
            view.commitExternalTextComposition("speech-detach", "O/A")
            assertEquals(1, events.size)
            assertEquals("O/A", backend.sessions.getValue(adapter.editorId).text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition real temporary detach retains active session`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            view.onExternalTextCompositionEndForTesting = events::add
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            host.addView(view)
            view.setEditorId(viewToken)
            adapter.setContentHtml("<p>arrival</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.richTextView.editorEditText.setSelection(0, 7)
            view.beginExternalTextComposition("speech-real-detach")
            view.updateExternalTextComposition("speech-real-detach", "O/A")

            host.removeView(view)

            assertTrue(events.isEmpty())
            assertEquals(viewToken, view.richTextView.editorEditText.editorId)
            assertEquals("O/A", view.richTextView.editorEditText.text.toString())

            host.addView(view)
            view.commitExternalTextComposition("speech-real-detach", "O/A")
            assertEquals(1, events.size)
            assertEquals("O/A", backend.sessions.getValue(adapter.editorId).text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition real final detach cancels and unbinds once`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            view.onExternalTextCompositionEndForTesting = events::add
            view.onEditorUpdateForTesting = {}
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            host.addView(view)
            view.setEditorId(viewToken)
            adapter.setContentHtml("<p>arrival</p>")?.let {
                view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
            }
            view.richTextView.editorEditText.setSelection(0, 7)
            view.beginExternalTextComposition("speech-final-detach")
            view.updateExternalTextComposition("speech-final-detach", "O/A")

            host.removeView(view)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(500))

            assertEquals(1, events.size)
            assertEquals(adapter.editorId, events.single()["editorId"])
            val result = JSONObject(events.single()["resultJson"] as String)
            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("lifecycle", result.getString("cause"))
            assertEquals(0L, view.richTextView.editorEditText.editorId)
            assertEquals("arrival", backend.sessions.getValue(adapter.editorId).text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition stale session IDs leave active session untouched`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            view.beginExternalTextComposition("speech-current")
            view.updateExternalTextComposition("speech-current", "O/A")

            val staleUpdate = JSONObject(
                view.updateExternalTextComposition("speech-stale", "wrong")
            )
            val staleCommit = JSONObject(
                view.commitExternalTextComposition("speech-stale", "wrong")
            )
            val staleCancel = JSONObject(
                view.cancelExternalTextComposition("speech-stale", "consumer")
            )

            assertEquals("error", staleUpdate.getString("type"))
            assertEquals("error", staleCommit.getString("type"))
            assertEquals("error", staleCancel.getString("type"))
            assertTrue(events.isEmpty())
            assertEquals("O/A", view.richTextView.editorEditText.text.toString())

            view.commitExternalTextComposition("speech-current", "O/A")
            assertEquals(1, events.size)
            assertEquals("O/A", backend.sessions.getValue(adapter.editorId).text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `external composition terminal result dispatches exactly once`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val events = mutableListOf<Map<String, Any>>()
        try {
            prepareExternalCompositionView(view, adapter, viewToken, events)
            view.beginExternalTextComposition("speech-once")
            view.updateExternalTextComposition("speech-once", "O/A")

            val first = view.commitExternalTextComposition("speech-once", "O/A")
            val second = view.commitExternalTextComposition("speech-once", "ignored")
            val third = view.cancelExternalTextComposition("speech-once", "consumer")

            assertEquals(first, second)
            assertEquals(first, third)
            assertEquals(1, events.size)
            assertEquals("O/A", backend.sessions.getValue(adapter.editorId).text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    private fun renderUpdateJson(text: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray()
                        .put(
                            JSONObject()
                                .put("type", "blockStart")
                                .put("nodeType", "paragraph")
                                .put("depth", 0)
                        )
                        .put(
                            JSONObject()
                                .put("type", "textRun")
                                .put("text", text)
                                .put("marks", JSONArray())
                        )
                        .put(JSONObject().put("type", "blockEnd"))
                )
            )
            .put("documentVersion", "1")
            .toString()

    private fun atomicRenderUpdateJson(text: String, revision: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray()
                        .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph").put("depth", 0))
                        .put(JSONObject().put("type", "textRun").put("text", text).put("marks", JSONArray()))
                        .put(JSONObject().put("type", "blockEnd"))
                )
            )
            .put("renderPatch", JSONObject.NULL)
            .put("selection", JSONObject().put("type", "text").put("anchor", 1).put("head", 1).put("anchorScalar", 0).put("headScalar", 0))
            .put(
                "activeState",
                JSONObject()
                    .put("marks", JSONObject())
                    .put("markAttrs", JSONObject())
                    .put("nodes", JSONObject().put("paragraph", true))
                    .put("commands", JSONObject())
                    .put("allowedMarks", JSONArray().put("bold"))
                    .put("insertableNodes", JSONArray().put("hardBreak"))
            )
            .put("historyState", JSONObject().put("canUndo", true).put("canRedo", false))
            .put("documentVersion", revision)
            .put("stateRevision", revision)
            .put("scalarLength", text.length)
            .put("documentIsEmpty", text.isEmpty())
            .toString()

    private fun commitBoundText(view: NativeEditorExpoView, text: String): Boolean {
        val editText = view.richTextView.editorEditText
        editText.setSelection(editText.selectionStart.coerceAtLeast(0))
        val inputConnection = editText.onCreateInputConnection(EditorInfo()) ?: return false
        return inputConnection.commitText(text, 1)
    }

    private fun attachAdapterForViewTest(
        backend: FakeEditorV2Backend,
        configJson: String = "{\"initialization\":{\"type\":\"localEmpty\"}}"
    ): EditorV2Adapter {
        val created = backend.create(configJson, null)
            as EditorV2CallResult.Ok
        return EditorV2Adapter.attach(
            backend,
            JSONObject(created.value).getString("editorId"),
            roomBound = false
        )!!
    }

    private fun prepareExternalCompositionView(
        view: NativeEditorExpoView,
        adapter: EditorV2Adapter,
        viewToken: Long,
        events: MutableList<Map<String, Any>>
    ) {
        view.onExternalTextCompositionEndForTesting = events::add
        view.onEditorUpdateForTesting = {}
        view.onAddonEventForTesting = {}
        view.onEditorReadyForTesting = {}
        view.onSelectionChangeForTesting = {}
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorId(viewToken)
        adapter.setContentHtml("<p>arrival</p>")?.let {
            view.richTextView.editorEditText.applyUpdateJSON(it, notifyListener = false)
        }
        view.richTextView.editorEditText.setSelection(0, 7)
    }

    private fun assertInvalidToolbarPreflightOmitsAtomicFields(preflightUpdateJson: String) {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val editText = view.richTextView.editorEditText
        val toolbarActionPayloads = mutableListOf<Map<String, Any>>()

        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            editText.v2Driver = object : EditorV2Driver by adapter {
                override fun insertText(text: String, atScalarPos: Int): String = preflightUpdateJson
            }
            editText.setSelection(0)
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)
            assertTrue(inputConnection!!.setComposingText("native", 1))
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayloads += payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertEquals(1, toolbarActionPayloads.size)
            val toolbarActionPayload = toolbarActionPayloads.single()
            assertFalse(toolbarActionPayload.containsKey("updateJson"))
            assertFalse(toolbarActionPayload.containsKey("documentRevision"))
            assertFalse(view.hasPendingNativeActionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    private data class TestExpoContext(
        val context: Context,
        val appContext: AppContext
    )

    private fun attachedNativeEditorView(): NativeEditorExpoView {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779904L
        val editText = view.richTextView.editorEditText

        view.onFocusChangeForTesting = {}
        view.onAddonEventForTesting = {}
        host.addView(view, FrameLayout.LayoutParams(200, 200))
        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            200,
            android.view.View.MeasureSpec.EXACTLY
        )
        val heightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            200,
            android.view.View.MeasureSpec.EXACTLY
        )
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, 200, 200)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson("ready"), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorFocusedForOutsideTapDecisionForTesting(true)
        return view
    }

    private fun testExpoContext(
        context: Context,
        currentActivity: Activity? = null
    ): TestExpoContext {
        val resolvedCurrentActivity = currentActivity ?: context as? Activity
        val reactContext = Class
            .forName("com.facebook.react.bridge.BridgeReactContext")
            .getConstructor(Context::class.java)
            .newInstance(context) as Context

        if (resolvedCurrentActivity != null) {
            reactContext.javaClass
                .getMethod("onHostResume", Activity::class.java)
                .invoke(reactContext, resolvedCurrentActivity)
        }

        val modulesProvider = object : ModulesProvider {
            override fun getModulesMap(): Map<Class<out Module>, String?> = emptyMap()
        }
        val constructor = AppContext::class.java.constructors.first { constructor ->
            constructor.parameterTypes.size == 3
        }
        val appContext = constructor.newInstance(
            modulesProvider,
            ModuleRegistry(emptyList(), emptyList()),
            WeakReference(reactContext)
        ) as AppContext
        return TestExpoContext(reactContext, appContext)
    }

    @Test
    fun `a remote commit rebases the view so the next keystroke is not refused`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val payloads = mutableListOf<Map<String, Any>>()
        try {
            bindFocusedViewForTypingTest(activity, view, viewToken, payloads)
            val editText = view.richTextView.editorEditText
            assertTrue(commitBoundText(view, "a"))
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            val session = backend.sessions.getValue(adapter.editorId)
            session.text.append("R")
            session.revision += 1uL
            NativeEditorViewRegistry.rebaseAfterRemoteCommit(adapter.editorId)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals("the remote commit is visible without a JS round trip", "aR", editText.text.toString())

            backend.calls.clear()
            assertTrue(commitBoundText(view, "b"))
            assertEquals(
                "the rebase means the keystroke is admitted first time",
                1L,
                backend.calls.count { it == "applyNativeIntent" }.toLong()
            )
            assertTrue(editText.text.toString().contains("b"))
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `only the current native owner consumes a remote commit refresh`() {
        val firstExpoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val secondExpoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val firstView = NativeEditorExpoView(firstExpoContext.context, firstExpoContext.appContext)
        val secondView = NativeEditorExpoView(secondExpoContext.context, secondExpoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        try {
            listOf(firstView, secondView).forEach { view ->
                view.onEditorUpdateForTesting = {}
                view.onAddonEventForTesting = {}
                view.onEditorReadyForTesting = {}
                view.onSelectionChangeForTesting = {}
                view.setAttachedToNativeWindowForTesting(true)
                view.setEditorId(viewToken)
            }
            val session = backend.sessions.getValue(adapter.editorId)
            session.text.append("Remote")
            session.revision += 1uL
            backend.calls.clear()

            NativeEditorViewRegistry.rebaseAfterRemoteCommit(adapter.editorId)
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals("", firstView.richTextView.editorEditText.text.toString())
            assertEquals("Remote", secondView.richTextView.editorEditText.text.toString())
            assertEquals(1, backend.calls.count { it == "renderNative" })
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, firstView)
            NativeEditorViewRegistry.unregister(viewToken, secondView)
        }
    }

    @Test
    fun `controlled push at an already rendered revision keeps newer typed text`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val payloads = mutableListOf<Map<String, Any>>()
        try {
            bindFocusedViewForTypingTest(activity, view, viewToken, payloads)
            val editText = view.richTextView.editorEditText

            assertTrue(commitBoundText(view, "a"))
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val deliveredRevision = payloads.last()["documentRevision"] as String

            assertTrue(commitBoundText(view, "b"))
            assertEquals("ab", editText.text.toString())

            view.setPendingEditorUpdateJson(atomicRenderUpdateJson("a", deliveredRevision))
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(1)
            view.applyPendingEditorUpdateIfNeeded()
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals(
                "a superseded push must not rewind the typed character",
                "ab",
                editText.text.toString()
            )
            assertTrue(
                view.imeTraceSnapshotForTypingTest().any {
                    it.startsWith("pendingEditorUpdateSuperseded")
                }
            )
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `malformed older controlled push is rejected before supersession`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val payloads = mutableListOf<Map<String, Any>>()
        val errors = mutableListOf<Map<String, Any>>()
        try {
            view.onEditorErrorForTesting = { errors += it }
            bindFocusedViewForTypingTest(activity, view, viewToken, payloads)
            val editText = view.richTextView.editorEditText
            assertTrue(commitBoundText(view, "a"))
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val olderRevision = payloads.last()["documentRevision"] as String
            assertTrue(commitBoundText(view, "b"))

            val malformed = JSONObject(atomicRenderUpdateJson("a", olderRevision))
            malformed.remove("historyState")
            view.setPendingEditorUpdateJson(malformed.toString())
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(2)
            view.applyPendingEditorUpdateIfNeeded()
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals("ab", editText.text.toString())
            assertEquals(1, errors.size)
            @Suppress("UNCHECKED_CAST")
            val error = errors.single()["error"] as Map<String, Any?>
            assertEquals("FFI_RESULT_INVALID", error["code"])
            assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `controlled push at equal rendered revision is not superseded`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val payloads = mutableListOf<Map<String, Any>>()
        try {
            bindFocusedViewForTypingTest(activity, view, viewToken, payloads)
            assertTrue(commitBoundText(view, "a"))
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val revision = payloads.last()["documentRevision"] as String

            view.setPendingEditorUpdateJson(atomicRenderUpdateJson("a", revision))
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(3)
            view.applyPendingEditorUpdateIfNeeded()

            assertFalse(view.imeTraceSnapshotForTypingTest().any {
                it.startsWith("pendingEditorUpdateSuperseded")
            })
            assertEquals(0, view.pendingEditorUpdateRevisionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `controlled push at a newer revision still applies over typed text`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val expoContext = testExpoContext(activity, activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val payloads = mutableListOf<Map<String, Any>>()
        try {
            bindFocusedViewForTypingTest(activity, view, viewToken, payloads)
            val editText = view.richTextView.editorEditText

            assertTrue(commitBoundText(view, "a"))
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))
            val renderedRevision = (payloads.last()["documentRevision"] as String).toULong()
            val session = backend.sessions.getValue(adapter.editorId)
            session.text = StringBuilder("remote")
            session.revision = renderedRevision + 1u
            session.anchor = session.text.length
            session.head = session.text.length

            view.setPendingEditorUpdateJson(
                atomicRenderUpdateJson("remote", (renderedRevision + 1u).toString())
            )
            view.setPendingEditorUpdateEditorId(viewToken)
            view.setPendingEditorUpdateRevision(1)
            view.applyPendingEditorUpdateIfNeeded()
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(200))

            assertEquals("remote", editText.text.toString())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    private fun bindFocusedViewForTypingTest(
        activity: Activity,
        view: NativeEditorExpoView,
        viewToken: Long,
        payloads: MutableList<Map<String, Any>>
    ) {
        view.onEditorUpdateForTesting = { payloads += it }
        view.onAddonEventForTesting = {}
        view.onEditorReadyForTesting = {}
        view.onSelectionChangeForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onContentHeightChangeForTesting = {}
        activity.setContentView(view)
        view.setAttachedToNativeWindowForTesting(true)
        view.setEditorId(viewToken)
        assertTrue(view.richTextView.editorEditText.requestFocus())
    }

    private fun NativeEditorExpoView.imeTraceSnapshotForTypingTest(): List<String> =
        richTextView.editorEditText.imeTraceSnapshotForTesting()

    @Test
    fun `auto grow honours a host minimum height so the whole box is tappable`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        view.onContentHeightChangeForTesting = { }
        view.setHeightBehavior("autoGrow")
        editText.applyUpdateJSON(renderUpdateJson("One line"), notifyListener = false)

        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            360,
            android.view.View.MeasureSpec.EXACTLY
        )
        val minHeightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            AUTO_GROW_MIN_HEIGHT_PX,
            android.view.View.MeasureSpec.EXACTLY
        )
        view.measure(widthSpec, minHeightSpec)
        view.layout(0, 0, 360, AUTO_GROW_MIN_HEIGHT_PX)

        // Auto-grow still measures content-sized on purpose; it is the frame
        // RN assigns that must be covered.
        assertTrue(view.measuredHeight < AUTO_GROW_MIN_HEIGHT_PX)
        assertEquals(
            "a tap anywhere in the minimum-height box must reach the field",
            AUTO_GROW_MIN_HEIGHT_PX,
            editText.height
        )
    }

    @Test
    fun `auto grow reports content height unchanged by a host minimum height`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val events = mutableListOf<Map<String, Any>>()
        view.onContentHeightChangeForTesting = { events.add(it) }
        view.setHeightBehavior("autoGrow")
        editText.applyUpdateJSON(renderUpdateJson("One line"), notifyListener = false)

        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            360,
            android.view.View.MeasureSpec.EXACTLY
        )
        val wrapSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            0,
            android.view.View.MeasureSpec.UNSPECIFIED
        )
        view.measure(widthSpec, wrapSpec)
        view.layout(0, 0, 360, view.measuredHeight)
        val naturalHeight = events.last()["contentHeight"] as Int
        assertTrue(naturalHeight < AUTO_GROW_MIN_HEIGHT_PX)

        events.clear()
        val minHeightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            AUTO_GROW_MIN_HEIGHT_PX,
            android.view.View.MeasureSpec.EXACTLY
        )
        view.measure(widthSpec, minHeightSpec)
        view.layout(0, 0, 360, AUTO_GROW_MIN_HEIGHT_PX)

        // Re-emitting the floor as content height would pin the editor open
        // and it could never shrink back when the text is deleted.
        events.forEach { event ->
            assertEquals(
                "the host floor must not be reported as content height",
                naturalHeight,
                event["contentHeight"]
            )
        }
    }

    @Test
    fun `auto grow still grows past a host minimum height`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        view.onContentHeightChangeForTesting = { }
        view.setHeightBehavior("autoGrow")
        editText.applyUpdateJSON(
            renderUpdateJson((1..60).joinToString("\n") { "line $it" }),
            notifyListener = false
        )

        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            360,
            android.view.View.MeasureSpec.EXACTLY
        )
        val wrapSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            0,
            android.view.View.MeasureSpec.UNSPECIFIED
        )
        view.measure(widthSpec, wrapSpec)

        assertTrue(
            "tall content must still drive the measured height (was ${view.measuredHeight})",
            view.measuredHeight > AUTO_GROW_MIN_HEIGHT_PX
        )
    }

    private companion object {
        const val AUTO_GROW_MIN_HEIGHT_PX = 900
    }
}
