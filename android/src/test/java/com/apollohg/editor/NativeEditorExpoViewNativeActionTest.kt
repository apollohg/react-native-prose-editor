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
internal class NativeEditorExpoViewNativeActionTest : NativeEditorExpoViewNativeActionTestFixture() {
    @Test
    fun `toolbar action preflight emits TS-compatible document revision matching atomic update`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val editText = view.richTextView.editorEditText
        val toolbarActionPayloads = mutableListOf<Map<String, Any>>()

        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            editText.setSelection(0)
            val inputConnection = editText.onCreateInputConnection(EditorInfo())
            assertNotNull(inputConnection)
            assertTrue(inputConnection!!.setComposingText("native", 1))
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayloads += payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertEquals(1, toolbarActionPayloads.size)
            val payload = toolbarActionPayloads.single()
            val updateJson = payload["updateJson"] as String
            val snapshotRevision = JSONObject(updateJson).getString("documentVersion")
            assertEquals(snapshotRevision, payload["documentRevision"])
            assertFalse(payload.containsKey("documentVersion"))
            assertEquals(adapter.editorId, payload["editorId"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }

    @Test
    fun `toolbar action omits both preflight fields for malformed document version payload`() {
        assertInvalidToolbarPreflightOmitsAtomicFields("{malformed")
    }

    @Test
    fun `toolbar action omits both preflight fields for missing document version`() {
        assertInvalidToolbarPreflightOmitsAtomicFields(
            JSONObject(atomicRenderUpdateJson("native", "1")).apply {
                remove("documentVersion")
            }
                .toString()
        )
    }

    @Test
    fun `toolbar action omits both preflight fields for noncanonical document version`() {
        assertInvalidToolbarPreflightOmitsAtomicFields(
            JSONObject(atomicRenderUpdateJson("native", "1"))
                .put("documentVersion", "01")
                .toString()
        )
    }

    @Test
    fun `action-only toolbar event omits cached document revision`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val backend = FakeEditorV2Backend()
        val adapter = attachAdapterForViewTest(backend)
        val viewToken = EditorV2Registry.register(adapter)
        val toolbarActionPayloads = mutableListOf<Map<String, Any>>()

        try {
            view.onAddonEventForTesting = {}
            view.onRefreshToolbarStateFromEditorSelectionForTesting = { null }
            view.onEditorReadyForTesting = {}
            view.onSelectionChangeForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(viewToken)
            view.setLastDocumentVersionForTesting("42")
            view.onToolbarActionForTesting = { payload ->
                toolbarActionPayloads += payload
            }

            view.handleToolbarItemPressForTesting(
                NativeToolbarItem(
                    type = ToolbarItemKind.action,
                    key = "custom",
                    label = "Custom"
                )
            )

            assertEquals(1, toolbarActionPayloads.size)
            val payload = toolbarActionPayloads.single()
            assertFalse(payload.containsKey("updateJson"))
            assertFalse(payload.containsKey("documentRevision"))
            assertEquals("custom", payload["key"])
            assertEquals(adapter.editorId, payload["editorId"])
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }
}
