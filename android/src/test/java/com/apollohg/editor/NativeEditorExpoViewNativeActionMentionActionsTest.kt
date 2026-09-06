package com.apollohg.editor
import android.app.Activity
import android.os.Looper
import android.view.inputmethod.EditorInfo
import android.widget.FrameLayout
import org.json.JSONArray
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

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class NativeEditorExpoViewNativeActionMentionActionsTest : NativeEditorExpoViewNativeActionTestFixture() {
    @Test
    fun `parked native mention selection survives controlled update document version acknowledgement`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779856L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val acknowledgedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "2")
            .toString()
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setLastDocumentVersionForTesting("1")
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(acknowledgedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)

        view.isApplyingJSUpdate = true
        view.onEditorUpdate(acknowledgedUpdateJson)
        view.isApplyingJSUpdate = false

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        addonPayload = null
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        val eventJson = JSONObject(addonPayload?.get("eventJson") as String)
        assertEquals("mentionsSelectRequest", eventJson.getString("type"))
        assertEquals("u1", eventJson.getString("suggestionKey"))
        assertEquals("2", eventJson.getString("documentVersion"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending controlled update parks native mention selection until cleared`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77886L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        val eventJson = JSONObject(addonPayload?.get("eventJson") as String)
        assertEquals("mentionsSelectRequest", eventJson.getString("type"))
        assertEquals("u1", eventJson.getString("suggestionKey"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending native mention action is parked after retry budget and wakes later`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779865L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        editText.blockExternalEditorCommandPreparationForTesting = true
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null

        view.insertMentionSuggestionForTesting(suggestion)
        repeat(4) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(16))
        }

        assertTrue(view.hasPendingNativeActionForTesting())
        assertTrue(view.pendingNativeActionRetryAttemptsForTesting() >= 3)

        editText.blockExternalEditorCommandPreparationForTesting = false
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        val eventJson = JSONObject(addonPayload?.get("eventJson") as String)
        assertEquals("mentionsSelectRequest", eventJson.getString("type"))
        assertEquals("u1", eventJson.getString("suggestionKey"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `destroyed editor clears parked native mention selection without emitting callback`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779862L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)
    }

    @Test
    fun `addons config change clears parked native mention selection`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778861L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var addonPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(7)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.onAddonEventForTesting = { payload ->
            addonPayload = payload
        }
        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put(
                            "suggestions",
                            JSONArray().put(
                                JSONObject()
                                    .put("key", "u1")
                                    .put("title", "Alice")
                                    .put("label", "@Alice")
                                    .put("attrs", JSONObject().put("id", "u1"))
                            )
                        )
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)

        view.insertMentionSuggestionForTesting(suggestion)

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setAddonsJson(
            JSONObject()
                .put(
                    "mentions",
                    JSONObject()
                        .put("resolveSelectionAttrs", true)
                        .put("suggestions", JSONArray())
                )
                .toString()
        )
        addonPayload = null
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(addonPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }
}
