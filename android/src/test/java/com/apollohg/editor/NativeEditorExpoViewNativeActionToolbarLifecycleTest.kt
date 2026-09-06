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
internal class NativeEditorExpoViewNativeActionToolbarLifecycleTest : NativeEditorExpoViewNativeActionTestFixture() {
    @Test
    fun `pending controlled update parks native toolbar action until cleared`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77885L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        val action = NativeToolbarItem(
            type = ToolbarItemKind.action,
            key = "custom",
            label = "Custom"
        )

        view.handleToolbarItemPressForTesting(action)

        assertTrue(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `parked native toolbar action survives controlled update document version acknowledgement`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779855L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        val acknowledgedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "2")
            .toString()
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setLastDocumentVersionForTesting("1")
        view.onAddonEventForTesting = {}
        view.setPendingEditorUpdateJson(acknowledgedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.isApplyingJSUpdate = true
        view.onEditorUpdate(acknowledgedUpdateJson)
        view.isApplyingJSUpdate = false

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))
        assertFalse(toolbarActionPayload!!.containsKey("updateJson"))
        assertFalse(toolbarActionPayload.containsKey("documentRevision"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `parked native toolbar action is dropped when unrelated document version changes`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779857L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        val acknowledgedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "2")
            .toString()
        val unrelatedUpdateJson = JSONObject(updateJson)
            .put("documentVersion", "3")
            .toString()
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setLastDocumentVersionForTesting("1")
        view.setPendingEditorUpdateJson(acknowledgedUpdateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onAddonEventForTesting = {}
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.isApplyingJSUpdate = true
        view.onEditorUpdate(unrelatedUpdateJson)
        view.isApplyingJSUpdate = false

        assertFalse(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `destroyed editor clears parked native toolbar action without emitting callback`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 779853L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        NativeEditorViewRegistry.register(editorId, view)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)
    }

    @Test
    fun `toolbar visibility placement and editability changes clear parked native toolbar action`() {
        val cases = listOf<(NativeEditorExpoView) -> Unit>(
            { view -> view.setShowToolbar(false) },
            { view -> view.setToolbarPlacement("inline") },
            { view -> view.setEditable(false) }
        )

        cases.forEachIndexed { index, clearAction ->
            val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
            val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
            val editorId = 778852L + index
            val editText = view.richTextView.editorEditText
            val updateJson = renderUpdateJson("")
            var toolbarActionPayload: Map<String, Any>? = null

            view.richTextView.setEditorIdWhileDetached(editorId)
            editText.applyUpdateJSON(updateJson, notifyListener = false)
            editText.setSelection(0)
            editText.editorId = editorId
            view.setAttachedToNativeWindowForTesting(true)
            view.setPendingEditorUpdateJson(updateJson)
            view.setPendingEditorUpdateEditorId(editorId)
            view.setPendingEditorUpdateRevision(1)
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayload = payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertTrue(view.hasPendingNativeActionForTesting())

            clearAction(view)
            view.setPendingEditorUpdateJson(null)
            view.setPendingEditorUpdateEditorId(editorId)
            view.setPendingEditorUpdateRevision(2)
            view.wakePendingPreflightWorkForTesting()

            assertFalse(view.hasPendingNativeActionForTesting())
            assertNull(toolbarActionPayload)

            NativeEditorViewRegistry.unregister(editorId, view)
        }
    }

    @Test
    fun `real blur clears parked native toolbar action`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val host = FrameLayout(activity)
        activity.setContentView(host)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778856L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        host.addView(view)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setCurrentImeBottomForTesting(120)
        view.onAddonEventForTesting = {}
        view.onFocusChangeForTesting = {}
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }
        assertTrue(editText.requestFocus())
        shadowOf(Looper.getMainLooper()).idle()

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        editText.clearFocus()
        shadowOf(Looper.getMainLooper()).idle()
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `toolbar preserved blur keeps parked native toolbar action current while refocus is pending`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778857L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setCurrentImeBottomForTesting(120)
        view.onFocusChangeForTesting = {}
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }
        view.scheduleToolbarRefocusForTesting()
        assertTrue(view.hasPendingToolbarRefocusForTesting())

        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `keyboard toolbar becoming invisible clears parked native toolbar action`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778858L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setCurrentImeBottomForTesting(0)
        view.updateAttachedKeyboardToolbarForInsetsForTesting()
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `read only native toolbar and mention callbacks are consumed without mutation`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778859L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("Hi @ali")
        val suggestion = NativeMentionSuggestion(
            key = "u1",
            title = "Alice",
            subtitle = null,
            label = "@Alice",
            attrs = JSONObject().put("id", "u1")
        )
        var toolbarActionPayload: Map<String, Any>? = null
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
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }
        addonPayload = null

        view.setEditable(false)
        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )
        view.insertMentionSuggestionForTesting(suggestion)

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)
        assertNull(addonPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `toolbar config change clears parked native toolbar action`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 778851L
        val editText = view.richTextView.editorEditText
        val updateJson = renderUpdateJson("")
        var toolbarActionPayload: Map<String, Any>? = null

        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(updateJson, notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId
        view.setAttachedToNativeWindowForTesting(true)
        view.setPendingEditorUpdateJson(updateJson)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(1)
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        view.handleToolbarItemPressForTesting(
            NativeToolbarItem(
                type = ToolbarItemKind.action,
                key = "custom",
                label = "Custom"
            )
        )

        assertTrue(view.hasPendingNativeActionForTesting())

        view.setToolbarItemsJson(
            JSONArray()
                .put(
                    JSONObject()
                        .put("type", "action")
                        .put("key", "other")
                        .put("label", "Other")
                )
                .toString()
        )
        view.setPendingEditorUpdateJson(null)
        view.setPendingEditorUpdateEditorId(editorId)
        view.setPendingEditorUpdateRevision(2)
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertNull(toolbarActionPayload)

        NativeEditorViewRegistry.unregister(editorId, view)
    }

    @Test
    fun `pending native toolbar action is parked after retry budget and wakes later`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText

        view.richTextView.setEditorIdWhileDetached(88990L)
        editText.setSelection(0)
        editText.editorId = 88990L
        editText.blockExternalEditorCommandPreparationForTesting = true
        var toolbarActionPayload: Map<String, Any>? = null
        view.onToolbarActionForTesting = { payload ->
            toolbarActionPayload = payload
        }

        val action = NativeToolbarItem(
            type = ToolbarItemKind.action,
            key = "custom",
            label = "Custom"
        )

        view.handleToolbarItemPressForTesting(action)
        repeat(4) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(16))
        }

        assertTrue(view.hasPendingNativeActionForTesting())
        assertTrue(view.pendingNativeActionRetryAttemptsForTesting() >= 3)

        editText.blockExternalEditorCommandPreparationForTesting = false
        view.wakePendingPreflightWorkForTesting()

        assertFalse(view.hasPendingNativeActionForTesting())
        assertEquals("custom", toolbarActionPayload?.get("key"))
        assertEquals("0", toolbarActionPayload?.get("editorId"))

        NativeEditorViewRegistry.unregister(88990L, view)
    }
}
