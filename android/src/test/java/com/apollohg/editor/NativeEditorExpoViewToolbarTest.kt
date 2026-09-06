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
internal class NativeEditorExpoViewToolbarTest : NativeEditorExpoViewTestFixture() {
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
}
