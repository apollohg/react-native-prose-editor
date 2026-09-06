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
internal class NativeEditorExpoViewControlledUpdateTest : NativeEditorExpoViewControlledUpdateTestFixture() {
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
}
