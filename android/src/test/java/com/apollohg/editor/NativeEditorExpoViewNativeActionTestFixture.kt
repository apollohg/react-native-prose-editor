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

internal abstract class NativeEditorExpoViewNativeActionTestFixture : NativeEditorExpoViewTestSupport(){
    protected fun assertInvalidToolbarPreflightOmitsAtomicFields(preflightUpdateJson: String) {
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
            editText.v2Driver = object : EditorV2Driver by adapter {
                override fun insertText(text: String, atScalarPos: Int): String = preflightUpdateJson
            }
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
            val toolbarActionPayload = toolbarActionPayloads.single()
            assertFalse(toolbarActionPayload.containsKey("updateJson"))
            assertFalse(toolbarActionPayload.containsKey("documentRevision"))
            assertFalse(view.hasPendingNativeActionForTesting())
        } finally {
            EditorV2Registry.remove(adapter.editorId)
            NativeEditorViewRegistry.unregister(viewToken, view)
        }
    }
}
