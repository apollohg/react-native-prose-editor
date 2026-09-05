package com.apollohg.editor

import android.app.Activity
import android.graphics.Point
import android.os.Looper
import android.view.MotionEvent
import android.view.Window
import android.view.inputmethod.EditorInfo
import android.widget.FrameLayout
import android.widget.ScrollView
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
import java.time.Duration
import java.util.concurrent.atomic.AtomicBoolean

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorExpoViewTest : NativeEditorExpoViewTestSupport() {
    @Test
    fun `host detach retires the active input connection before teardown`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        editText.setText("abc")
        editText.setSelection(3)
        val inputConnection = requireNotNull(editText.onCreateInputConnection(EditorInfo()))

        view.handleDetachedFromWindowForTesting()

        assertEquals("", inputConnection.getTextBeforeCursor(3, 0).toString())
    }

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
    fun `theme update applies when preflight is ready`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#ff0000"}"""

        view.setThemeJson(themeJson)

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(themeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `atoms json reapplies the current render`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray().put(
                        JSONObject()
                            .put("type", "voidBlock")
                            .put("nodeType", "counterCard")
                            .put("docPos", 1)
                    )
                )
            )
            .put("documentVersion", "1")
            .toString()
        view.richTextView.editorEditText.applyUpdateJSON(updateJson, notifyListener = false)
        val textBeforeRegistration = requireNotNull(view.richTextView.editorEditText.text)
        assertTrue(textBeforeRegistration.getSpans(0, 1, AtomBlockSpan::class.java).isEmpty())

        view.setAtomsJson(
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        )

        val textAfterRegistration = requireNotNull(view.richTextView.editorEditText.text)
        assertEquals(
            120,
            textAfterRegistration.getSpans(0, 1, AtomBlockSpan::class.java)
                .single()
                .reservedHeightPx
        )
    }

    @Test
    fun `atoms json retries unchanged prop after transient preflight failure`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val atomsJson =
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        view.richTextView.editorEditText.editorId = 12345L
        view.richTextView.editorEditText.blockExternalEditorUpdatePreparationForTesting = true

        view.setAtomsJson(atomsJson)
        view.setAtomsJson(atomsJson)

        assertEquals(atomsJson, view.pendingAtomsJsonForTesting())
        assertNull(view.lastAtomsJsonForTesting())

        view.richTextView.editorEditText.blockExternalEditorUpdatePreparationForTesting = false
        view.richTextView.editorEditText.editorId = 0L
        view.wakePendingPreflightWorkForTesting()

        assertNull(view.pendingAtomsJsonForTesting())
        assertEquals(atomsJson, view.lastAtomsJsonForTesting())
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
    fun `keyboard toolbar provides caret clearance to auto grow editor`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val editorId = 778896L

        view.onContentHeightChangeForTesting = {}
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.setHeightBehavior("autoGrow")
        host.addView(view, FrameLayout.LayoutParams(360, FrameLayout.LayoutParams.WRAP_CONTENT))
        view.measure(
            android.view.View.MeasureSpec.makeMeasureSpec(360, android.view.View.MeasureSpec.EXACTLY),
            android.view.View.MeasureSpec.makeMeasureSpec(0, android.view.View.MeasureSpec.UNSPECIFIED)
        )
        view.layout(0, 0, 360, view.measuredHeight)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setThemeJson("""{"toolbar":{"height":60,"keyboardOffset":12}}""")
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(editText.requestFocus())
        view.setCurrentImeBottomForTesting(160)

        view.updateAttachedKeyboardToolbarForInsetsForTesting()

        val minimumClearance = (72f * view.resources.displayMetrics.density).toInt()
        val actualClearance = view.richTextView.viewportBottomInsetPxForTesting()
        assertTrue(
            "expected toolbar clearance >= $minimumClearance but was $actualClearance",
            actualClearance >= minimumClearance
        )

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `keyboard toolbar clearance includes occluded outer scroll viewport`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = activity.findViewById<FrameLayout>(android.R.id.content)
        val outerScrollView = ScrollView(activity)
        val outerContent = FrameLayout(activity)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val editorId = 778897L
        val width = 360
        val hostHeight = 900
        val scrollViewportHeight = 700
        val imeBottom = 400

        view.onContentHeightChangeForTesting = {}
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.setHeightBehavior("autoGrow")
        outerScrollView.addView(outerContent, FrameLayout.LayoutParams(width, 1400))
        outerContent.addView(view, FrameLayout.LayoutParams(width, 480))
        host.addView(outerScrollView, FrameLayout.LayoutParams(width, scrollViewportHeight))
        host.measure(
            android.view.View.MeasureSpec.makeMeasureSpec(width, android.view.View.MeasureSpec.EXACTLY),
            android.view.View.MeasureSpec.makeMeasureSpec(hostHeight, android.view.View.MeasureSpec.EXACTLY)
        )
        host.layout(0, 0, width, hostHeight)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setThemeJson("""{"toolbar":{"height":60,"keyboardOffset":0}}""")
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(editText.requestFocus())
        view.setCurrentImeBottomForTesting(imeBottom)

        view.updateAttachedKeyboardToolbarForInsetsForTesting()

        val toolbarHeight = (60f * view.resources.displayMetrics.density).toInt()
        val viewportBehindKeyboard = scrollViewportHeight - (hostHeight - imeBottom)
        val minimumClearance = toolbarHeight + viewportBehindKeyboard
        val actualClearance = view.richTextView.viewportBottomInsetPxForTesting()
        assertTrue(
            "expected occluded viewport clearance >= $minimumClearance but was $actualClearance",
            actualClearance >= minimumClearance
        )

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
    fun `auto grow publishes changed height during native render application`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val events = mutableListOf<Map<String, Any>>()
        view.onContentHeightChangeForTesting = { events += it }
        view.setHeightBehavior("autoGrow")

        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            360,
            android.view.View.MeasureSpec.EXACTLY
        )
        val heightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            0,
            android.view.View.MeasureSpec.UNSPECIFIED
        )
        editText.applyUpdateJSON(renderUpdateJson("One line"), notifyListener = false)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val initialHeight = events.last()["contentHeight"] as Int
        events.clear()

        editText.applyUpdateJSON(
            renderUpdateJson((1..8).joinToString("\n") { "Line $it" }),
            notifyListener = false
        )

        assertTrue("height must publish before the next looper turn", events.isNotEmpty())
        assertTrue((events.last()["contentHeight"] as Int) > initialHeight)
    }

    @Test
    fun `content size change hook ignores caret-only selection`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        var contentSizeChangeCount = 0
        editText.onContentSizeMayChange = { contentSizeChangeCount += 1 }

        editText.applyUpdateJSON(renderUpdateJson("Alpha"), notifyListener = false)
        assertTrue(contentSizeChangeCount > 0)
        contentSizeChangeCount = 0

        editText.setSelection(editText.length())

        assertEquals(0, contentSizeChangeCount)
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

}
