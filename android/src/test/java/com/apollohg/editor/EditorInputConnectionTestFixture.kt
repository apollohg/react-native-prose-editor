package com.apollohg.editor
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.os.Bundle
import android.os.Looper
import android.provider.Settings
import android.text.SpannableStringBuilder
import android.text.Spanned
import android.text.InputType
import android.text.style.AbsoluteSizeSpan
import android.view.MotionEvent
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.RobolectricTestRunner
import org.robolectric.Robolectric
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

internal abstract class EditorInputConnectionTestFixture : EditorInputConnectionTestSupport(){
    protected fun terminalAtomEditText(
        activity: Activity,
        renderJson: String =
            """[{"type":"voidBlock","nodeType":"counterCard","docPos":1,"atomId":"counter-1"}]""",
    ): EditorEditText =
        EditorEditText(activity).apply {
            applyAtomRenderConfiguration(
                AtomRenderConfiguration(
                    registeredNodeTypes = setOf("counterCard"),
                    estimatedHeightsDp = mapOf("counterCard" to 72f),
                    measuredHeightsPx = emptyMap(),
                )
            )
            applyRenderJSON(renderJson)
            editorId = 9_001L
        }

    protected fun paragraphThenAtomRenderJson(): String =
        """
        [
          {"type":"blockStart","nodeType":"paragraph","depth":0},
          {"type":"textRun","text":"Before","marks":[]},
          {"type":"blockEnd"},
          {"type":"voidBlock","nodeType":"counterCard","docPos":8,"atomId":"counter-1"}
        ]
        """.trimIndent()

    protected class StructuredDeleteHarness(
        val adapter: EditorV2Adapter,
        val editText: EditorEditText,
        initialBlocks: JSONArray
    ) {
        private var blocks = JSONArray(initialBlocks.toString())

        fun adopt(updateJSON: String) {
            val update = JSONObject(updateJSON)
            update.optJSONArray("renderBlocks")?.let { replacement ->
                blocks = JSONArray(replacement.toString())
                return
            }
            val patch = update.getJSONObject("renderPatch")
            val start = patch.getInt("startIndex")
            val deleteCount = patch.getInt("deleteCount")
            val replacement = patch.getJSONArray("renderBlocks")
            blocks = JSONArray().apply {
                for (index in 0 until start) put(blocks.get(index))
                for (index in 0 until replacement.length()) put(replacement.get(index))
                for (index in start + deleteCount until blocks.length()) {
                    put(blocks.get(index))
                }
            }
        }

        fun expectedText(): String = RenderBridge.buildSpannableFromBlocks(
            blocks,
            baseFontSize = 16f,
            textColor = Color.BLACK
        ).toString()
    }

    protected fun structuredDeleteHarness(initialHtml: String): StructuredDeleteHarness {
        val created = UniffiEditorV2Backend.create(
            """{"initialization":{"type":"localEmpty"}}""",
            null
        ) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(
            UniffiEditorV2Backend,
            editorId,
            roomBound = false
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        val initialUpdate = JSONObject(adapter.setContentHtml(initialHtml)!!)
        val harness = StructuredDeleteHarness(
            adapter,
            editText,
            initialUpdate.getJSONArray("renderBlocks")
        )
        editText.applyUpdateJSON(initialUpdate.toString(), notifyListener = false)
        editText.editorListener = object : EditorEditText.EditorListener {
            override fun onSelectionChanged(anchor: Int, head: Int) = Unit
            override fun onEditorUpdate(updateJSON: String) = harness.adopt(updateJSON)
        }
        return harness
    }

    protected fun assertGeneratedBackspaceDoesNotMutateNative(
        editText: EditorEditText,
        selection: Int
    ) {
        editText.editorId = 1
        var routedDeleteCount = 0
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { _, _ ->
            routedDeleteCount += 1
        }
        editText.onDeleteRangeInRustForTesting = { _, _ ->
            routedDeleteCount += 1
        }
        editText.setSelection(selection)
        val before = editText.text.toString()
        val inputConnection = editText.onCreateInputConnection(EditorInfo())!!

        assertTrue(inputConnection.deleteSurroundingText(1, 0))

        assertEquals(before, editText.text.toString())
        assertEquals(1, routedDeleteCount)
        assertFalse(editText.hasDeferredRustUpdateApplicationForTesting())
    }

    protected data class RealExternalCompositionHarness(
        val editorId: String,
        val adapter: EditorV2Adapter,
        val editText: EditorEditText
    )

    protected fun realExternalCompositionHarness(
        initialText: String,
        configJson: String = """{"initialization":{"type":"localEmpty"}}""",
        roomBound: Boolean = false,
        collaborationWake: (String, CollaborationWakeReason) -> Unit = { _, _ -> }
    ): RealExternalCompositionHarness {
        val created = UniffiEditorV2Backend.create(configJson, null) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(
            UniffiEditorV2Backend,
            editorId,
            roomBound = roomBound,
            collaborationWake = collaborationWake
        )!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>$initialText</p>")
            ?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        return RealExternalCompositionHarness(editorId, adapter, editText)
    }

    protected fun assertRealExternalCompositionPolicyFailure(
        configJson: String,
        initialText: String,
        finalText: String
    ) {
        val collaborationWakes = mutableListOf<CollaborationWakeReason>()
        val harness = realExternalCompositionHarness(
            initialText,
            configJson,
            roomBound = true,
            collaborationWake = { _, reason -> collaborationWakes.add(reason) }
        )
        try {
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            harness.editText.setSelection(0, initialText.length)
            val revisionBefore = harness.adapter.baseDocumentRevision
            val canUndoBefore = harness.adapter.historyCanUndo()
            val canRedoBefore = harness.adapter.historyCanRedo()
            collaborationWakes.clear()

            harness.editText.beginExternalTextComposition("speech-policy")
            harness.editText.updateExternalTextComposition("speech-policy", finalText)
            assertTrue(collaborationWakes.isEmpty())
            val resultJson = harness.editText.commitExternalTextComposition(
                "speech-policy",
                finalText
            )
            val duplicate = harness.editText.commitExternalTextComposition(
                "speech-policy",
                "ignored"
            )
            val result = JSONObject(resultJson)

            assertEquals("cancelled", result.getString("outcome"))
            assertEquals("EXTERNAL_COMPOSITION_COMMIT_FAILED", result.errorCode())
            assertExternalCompositionErrorShape(result)
            assertEquals("<p>$initialText</p>", harness.adapter.documentHtml())
            assertEquals(initialText, harness.editText.text.toString())
            assertEquals(revisionBefore, harness.adapter.baseDocumentRevision)
            assertEquals(canUndoBefore, harness.adapter.historyCanUndo())
            assertEquals(canRedoBefore, harness.adapter.historyCanRedo())
            assertTrue(collaborationWakes.isEmpty())
            assertEquals(resultJson, duplicate)
            assertEquals(listOf(resultJson), listener.externalCompositionEnds)
            assertTrue(listener.receivedUpdates.isEmpty())
        } finally {
            harness.adapter.destroy()
        }
    }

    protected fun assertExternalCompositionErrorShape(result: JSONObject) {
        val error = result.getJSONObject("error")
        assertEquals(
            setOf(
                "domain",
                "code",
                "message",
                "requestId",
                "operationIndex",
                "limit",
                "actual",
                "details"
            ),
            error.keys().asSequence().toSet()
        )
        assertEquals("lifecycle", error.getString("domain"))
        assertTrue(error.getString("message").isNotEmpty())
        listOf("requestId", "operationIndex", "limit", "actual", "details").forEach {
            assertTrue(error.isNull(it))
        }
    }

    protected class RecordingEditorListener : EditorEditText.EditorListener {
        val externalCompositionEnds = mutableListOf<String>()
        val receivedUpdates = mutableListOf<String>()
        val events = mutableListOf<String>()

        override fun onSelectionChanged(anchor: Int, head: Int) = Unit

        override fun onEditorUpdate(updateJSON: String) {
            receivedUpdates.add(updateJSON)
            events.add("update")
        }

        override fun onExternalTextCompositionEnded(resultJson: String) {
            externalCompositionEnds.add(resultJson)
            events.add("external")
        }
    }

    protected fun JSONObject.errorCode(): String =
        getJSONObject("error").getString("code")

    protected fun withDefaultInputMethod(context: Context, inputMethodId: String, block: () -> Unit) {
        val previous = Settings.Secure.getString(
            context.contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD
        )
        Settings.Secure.putString(
            context.contentResolver,
            Settings.Secure.DEFAULT_INPUT_METHOD,
            inputMethodId
        )
        try {
            block()
        } finally {
            Settings.Secure.putString(
                context.contentResolver,
                Settings.Secure.DEFAULT_INPUT_METHOD,
                previous
            )
        }
    }
}
