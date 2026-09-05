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
class NativeEditorExpoViewControlledUpdateTest : NativeEditorExpoViewTestSupport() {
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

}
