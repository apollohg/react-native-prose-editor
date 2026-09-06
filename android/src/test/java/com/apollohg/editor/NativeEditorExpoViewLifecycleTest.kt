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
internal class NativeEditorExpoViewLifecycleTest : NativeEditorExpoViewTestFixture() {
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
}
