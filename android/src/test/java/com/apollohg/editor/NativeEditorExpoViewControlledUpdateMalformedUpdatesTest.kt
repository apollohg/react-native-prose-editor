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
internal class NativeEditorExpoViewControlledUpdateMalformedUpdatesTest : NativeEditorExpoViewControlledUpdateTestFixture() {
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
}
