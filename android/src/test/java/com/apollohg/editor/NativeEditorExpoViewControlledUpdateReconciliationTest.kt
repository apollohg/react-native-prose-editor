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
internal class NativeEditorExpoViewControlledUpdateReconciliationTest : NativeEditorExpoViewControlledUpdateTestFixture() {
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
}
