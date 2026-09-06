package com.apollohg.editor

import android.os.Looper
import android.view.inputmethod.EditorInfo
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorAtomSelectionEventRegressionTest : NativeEditorExpoViewTestSupport() {
    @Test
    fun `backspace selecting card emits Node selection instead of a document commit`() {
        val created = UniffiEditorV2Backend.create("""
            {"schema":{"nodes":[
                {"name":"doc","content":"block+","role":"doc"},
                {"name":"paragraph","content":"text*","group":"block","role":"textBlock"},
                {"name":"text","content":"","role":"text"},
                {"name":"counterCard","content":"","group":"block","role":"block","isVoid":true}
            ],"marks":[]},"initialization":{"type":"localJson","json":{
                "type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Before"}]},{"type":"counterCard"},{"type":"paragraph"}]
            }}}
        """.trimIndent(), null) as EditorV2CallResult.Ok
        val adapter = requireNotNull(EditorV2Adapter.attach(
            UniffiEditorV2Backend, JSONObject(created.value).getString("editorId"), roomBound = false,
        ))
        val token = EditorV2Registry.register(adapter)
        val context = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(context.context, context.appContext)
        val selections = mutableListOf<Map<String, Any>>()
        val commits = mutableListOf<Map<String, Any>>()
        try {
            view.onSelectionChangeForTesting = { selections += it }
            view.onEditorUpdateForTesting = { commits += it }
            view.onAddonEventForTesting = {}
            view.onEditorReadyForTesting = {}
            view.setAttachedToNativeWindowForTesting(true)
            view.setEditorId(token)
            val editor = view.richTextView.editorEditText
            editor.applyAtomRenderConfiguration(
                AtomRenderConfiguration(setOf("counterCard"), mapOf("counterCard" to 72f), emptyMap())
            )
            editor.setSelection(editor.text!!.length)
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            shadowOf(Looper.getMainLooper()).idle()
            selections.clear()
            commits.clear()
            val documentVersion = adapter.baseDocumentRevision.toString()

            assertTrue(input.deleteSurroundingText(1, 0))
            shadowOf(Looper.getMainLooper()).idle()

            assertEquals(1, selections.size)
            assertTrue(commits.isEmpty())
            val event = selections.single()
            assertEquals(documentVersion, event["documentVersion"])
            val state = JSONObject(event.getValue("stateJson") as String)
            assertEquals("node", state.getJSONObject("selection").getString("type"))
            assertEquals(state.getJSONObject("selection").getInt("pos"), event["anchor"])
            assertEquals((event["anchor"] as Int) + 1, event["head"])
        } finally {
            NativeEditorViewRegistry.unregister(token, view)
            EditorV2Registry.remove(adapter.editorId)
            adapter.destroy()
        }
    }
}
