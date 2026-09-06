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
internal class NativeEditorExpoViewTest : NativeEditorExpoViewTestFixture() {
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
}
