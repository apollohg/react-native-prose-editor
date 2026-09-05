package com.apollohg.editor

import android.app.Activity
import android.os.Looper
import android.widget.FrameLayout
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.time.Duration

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorExpoViewExternalCompositionTest : NativeEditorExpoViewTestSupport() {
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

}
