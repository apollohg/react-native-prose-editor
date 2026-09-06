package com.apollohg.editor

import android.text.InputType
import android.text.TextUtils
import android.view.inputmethod.EditorInfo
import android.view.MotionEvent
import android.view.View
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputBoundaryRegressionTest : EditorInputConnectionTestFixture() {
    @Test
    fun `new unordered list item requests sentence capitalization`() {
        val harness = structuredDeleteHarness("<ul><li><p>First item</p></li></ul>")
        try {
            val editor = harness.editText
            editor.setAutoCapitalize("sentences")
            editor.setSelection(editor.text!!.length)
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            assertTrue(input.commitText("\n", 1))

            assertEquals(
                "text=${editor.text} selection=${editor.selectionStart}",
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES,
                input.getCursorCapsMode(InputType.TYPE_TEXT_FLAG_CAP_SENTENCES),
            )
            val context = requireNotNull(input.getTextBeforeCursor(100, 0))
            assertTrue(
                "IME context=$context",
                TextUtils.getCapsMode(context, context.length, InputType.TYPE_TEXT_FLAG_CAP_SENTENCES) != 0,
            )
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `backspace selects then deletes atom after trailing empty paragraph`() {
        val harness = atomHarness()
        try {
            val editor = harness.editText
            editor.setSelection(editor.text!!.length)
            val scalar = requireNotNull(editor.currentScalarSelection())
            editor.applyUpdateJSON(
                requireNotNull(harness.adapter.insertContentJsonAtSelection(
                    """{"type":"doc","content":[{"type":"counterCard"}]}""",
                    scalar.first,
                    scalar.second,
                )),
                notifyListener = false,
            )
            assertTrue(requireNotNull(harness.adapter.documentJson()).contains("counterCard"))
            val listener = RecordingEditorListener()
            val selectionStates = mutableListOf<String>()
            editor.editorListener = object : EditorEditText.EditorListener by listener {
                override fun onSelectionChanged(anchor: Int, head: Int) {
                    selectionStates += requireNotNull(harness.adapter.currentStateJson())
                }
            }
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            val beforeDeletion = harness.adapter.documentJson()

            assertTrue(input.deleteSurroundingText(1, 0))
            assertEquals(beforeDeletion, harness.adapter.documentJson())
            val atomOffset = editor.text.toString().indexOf('\uFFFC')
            assertEquals(atomOffset, editor.selectionStart)
            assertEquals(atomOffset + 1, editor.selectionEnd)
            val coreSelection = JSONObject(requireNotNull(harness.adapter.selectionJson()))
            assertEquals("node", coreSelection.getString("type"))
            assertTrue(listener.receivedUpdates.isEmpty())
            val emittedSelection = JSONObject(selectionStates.last()).getJSONObject("selection")
            assertEquals("node", emittedSelection.getString("type"))
            assertEquals(coreSelection.getInt("pos"), emittedSelection.getInt("pos"))
            assertTrue(input.deleteSurroundingText(1, 0))

            assertFalse(
                "selection=${editor.selectionStart}..${editor.selectionEnd} text=${editor.text}\n" +
                    editor.imeTraceSnapshotForTesting().joinToString("\n"),
                requireNotNull(harness.adapter.documentJson()).contains("counterCard"),
            )
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `hardware backspace selects terminal atom in Rust before deleting it`() {
        val harness = atomHarness()
        try {
            val editor = harness.editText
            editor.applyUpdateJSON(requireNotNull(harness.adapter.setContentJson(
                """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Before"}]},{"type":"counterCard"},{"type":"paragraph"}]}"""
            )), notifyListener = false)
            editor.setSelection(editor.text!!.length)

            val beforeDeletion = harness.adapter.documentJson()
            editor.handleBackspace()
            assertEquals(beforeDeletion, harness.adapter.documentJson())

            val selection = JSONObject(requireNotNull(harness.adapter.selectionJson()))
            assertEquals("node", selection.getString("type"))
            val atomOffset = editor.text.toString().indexOf('\uFFFC')
            assertEquals(atomOffset, editor.selectionStart)
            assertEquals(atomOffset + 1, editor.selectionEnd)

            editor.handleBackspace()

            assertFalse(requireNotNull(harness.adapter.documentJson()).contains("counterCard"))
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `deleting inserted card preserves editable end after horizontal rule`() {
        val harness = atomHarness()
        try {
            val editor = harness.editText
            editor.applyUpdateJSON(requireNotNull(harness.adapter.setContentJson(
                """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Before"}]},{"type":"horizontalRule"},{"type":"paragraph"}]}"""
            )), notifyListener = false)
            editor.setSelection(editor.text!!.length)
            val scalar = requireNotNull(editor.currentScalarSelection())
            editor.applyUpdateJSON(requireNotNull(harness.adapter.insertContentJsonAtSelection(
                """{"type":"doc","content":[{"type":"counterCard"}]}""", scalar.first, scalar.second,
            )), notifyListener = false)
            val deletionInput = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            assertTrue(deletionInput.deleteSurroundingText(1, 0))
            assertTrue(deletionInput.deleteSurroundingText(1, 0))
            editor.measure(
                View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
                View.MeasureSpec.makeMeasureSpec(400, View.MeasureSpec.EXACTLY),
            )
            editor.layout(0, 0, 320, 400)
            editor.setSelection(3)
            val y = (editor.totalPaddingTop + editor.layout.height + 30).toFloat()
            for ((action, time) in listOf(MotionEvent.ACTION_DOWN to 0L, MotionEvent.ACTION_UP to 16L)) {
                val event = MotionEvent.obtain(0, time, action, 30f, y, 0)
                editor.dispatchTouchEvent(event)
                event.recycle()
            }
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))

            assertTrue(input.commitText("Q", 1))

            val document = JSONObject(requireNotNull(harness.adapter.documentJson()))
            val blocks = document.getJSONArray("content")
            val lastBlock = blocks.getJSONObject(blocks.length() - 1)
            assertEquals(document.toString() + "\n" + editor.imeTraceSnapshotForTesting().joinToString("\n"), "paragraph", lastBlock.getString("type"))
            assertEquals("Q", lastBlock.getJSONArray("content").getJSONObject(0).getString("text"))
            assertEquals("horizontalRule", blocks.getJSONObject(blocks.length() - 2).getString("type"))
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `selecting card publishes pending text deletion before selection event`() {
        val harness = atomHarness()
        try {
            val editor = harness.editText
            editor.applyUpdateJSON(requireNotNull(harness.adapter.setContentJson(
                """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Before"}]},{"type":"counterCard"},{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}"""
            )), notifyListener = false)
            editor.setSelection(editor.text!!.length)
            val events = mutableListOf<String>()
            editor.editorListener = object : EditorEditText.EditorListener {
                override fun onSelectionChanged(anchor: Int, head: Int) { events += "selection" }
                override fun onEditorUpdate(updateJSON: String) { events += "commit" }
            }
            val input = requireNotNull(editor.onCreateInputConnection(EditorInfo()))
            assertTrue(input.deleteSurroundingText(1, 0))
            assertTrue(editor.hasDeferredRustUpdateApplicationForTesting())
            val afterTextDeletion = harness.adapter.documentJson()

            assertTrue(input.deleteSurroundingText(1, 0))

            assertEquals(afterTextDeletion, harness.adapter.documentJson())
            assertEquals(listOf("commit", "selection"), events)
            assertEquals("node", JSONObject(requireNotNull(harness.adapter.selectionJson())).getString("type"))
        } finally {
            harness.adapter.destroy()
        }
    }

    private fun atomHarness(): RealExternalCompositionHarness {
        return realExternalCompositionHarness("Before", """
            {
                "schema":{"nodes":[
                    {"name":"doc","content":"block+","role":"doc"},
                    {"name":"paragraph","content":"text*","group":"block","role":"textBlock"},
                    {"name":"text","content":"","role":"text"},
                    {"name":"counterCard","content":"","group":"block","role":"block","isVoid":true},
                    {"name":"horizontalRule","content":"","group":"block","role":"block","isVoid":true}
                ],"marks":[]},
                "initialization":{"type":"localEmpty"}
            }
        """.trimIndent()).also {
            it.editText.applyUpdateJSON(requireNotNull(it.adapter.setContentJson(
                """{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Before"}]}]}"""
            )), notifyListener = false)
            it.editText.applyAtomRenderConfiguration(
                AtomRenderConfiguration(setOf("counterCard"), mapOf("counterCard" to 72f), emptyMap())
            )
            it.adapter.claimNativeBindingIfUnowned(1L)
        }
    }
}
