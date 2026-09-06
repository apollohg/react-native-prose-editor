package com.apollohg.editor
import android.app.Activity
import android.os.Handler
import android.os.Looper
import android.view.inputmethod.EditorInfo
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class NativeEditorExpoViewControlledUpdateResetTest : NativeEditorExpoViewControlledUpdateTestFixture() {
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
}
