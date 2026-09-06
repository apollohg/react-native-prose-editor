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

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
internal class EditorInputConnectionTest : EditorInputConnectionTestFixture() {
    @Test
    fun `atom boundary does not expose a cursor`() {
        val activity = Robolectric.buildActivity(Activity::class.java)
            .setup()
            .visible()
            .windowFocusChanged(true)
            .get()
        val editText = terminalAtomEditText(activity)
        activity.setContentView(editText)
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.AT_MOST),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        assertTrue(editText.requestFocus())

        for (offset in listOf(0, editText.text!!.length)) {
            editText.setSelection(offset)
            assertNull(editText.nativeCursorDrawRect())
            assertFalse(editText.isCursorVisible)
        }
    }

    @Test
    fun `atom boundary selection restores last paragraph caret`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity, paragraphThenAtomRenderJson())
        activity.setContentView(editText)
        editText.setSelection(3)
        val atomOffset = editText.text!!.length - 1

        for (offset in listOf(atomOffset, atomOffset + 1)) {
            editText.setSelection(offset)
            assertEquals(3, editText.selectionStart)
            assertEquals(3, editText.selectionEnd)
        }
    }

    @Test
    fun `tap on atom line preserves paragraph caret`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity, paragraphThenAtomRenderJson())
        activity.setContentView(editText)
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(300, View.MeasureSpec.AT_MOST),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        val textLayout = requireNotNull(editText.layout)
        val atomOffset = editText.text!!.length - 1
        val atomLine = textLayout.getLineForOffset(atomOffset)
        val tapX = editText.totalPaddingLeft + textLayout.width - 4f
        val tapY = editText.totalPaddingTop +
            (textLayout.getLineTop(atomLine) + textLayout.getLineBottom(atomLine)) / 2f
        editText.setSelection(3)

        val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
        editText.onTouchEvent(down)
        down.recycle()
        val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
        editText.onTouchEvent(up)
        up.recycle()

        assertEquals(3, editText.selectionStart)
        assertEquals(3, editText.selectionEnd)
    }

    @Test
    fun `terminal atom does not add auto grow height`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        activity.setContentView(editText)
        editText.measure(
            View.MeasureSpec.makeMeasureSpec(320, View.MeasureSpec.EXACTLY),
            View.MeasureSpec.makeMeasureSpec(500, View.MeasureSpec.AT_MOST),
        )
        editText.layout(0, 0, editText.measuredWidth, editText.measuredHeight)
        val textLayout = requireNotNull(editText.layout)
        val expectedHeight = textLayout.height +
            editText.compoundPaddingTop + editText.compoundPaddingBottom

        assertEquals(expectedHeight, editText.resolveAutoGrowHeight())
    }

    @Test
    fun `terminal atom boundary ignores committed text`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        editText.setSelection(editText.text!!.length)
        var insertion: Pair<String, Int>? = null
        editText.onInsertTextInRustForTesting = { text, scalar -> insertion = text to scalar }

        editText.handleTextCommit("x")

        assertNull(insertion)
    }

    @Test
    fun `terminal atom boundary ignores return`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        editText.setSelection(editText.text!!.length)
        var splitPosition: Int? = null
        editText.onSplitBlockInRustForTesting = { scalar -> splitPosition = scalar }

        editText.handleTextCommit("\n")

        assertNull(splitPosition)
    }

    @Test
    fun `terminal atom boundary ignores backspace`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val editText = terminalAtomEditText(activity)
        editText.setSelection(editText.text!!.length)
        var deletion: Pair<Int, Int>? = null
        editText.onDeleteBackwardAtSelectionScalarInRustForTesting = { anchor, head ->
            deletion = anchor to head
        }

        editText.handleBackspace()

        assertNull(deletion)
    }

    @Test
    fun `external composition task marker tap recomputes filtered scalar`() {
        val harness = realExternalCompositionHarness(
            initialText = "12",
            configJson = """
                {
                  "schema": {
                    "nodes": [
                      {"name":"doc","content":"block+","role":"doc"},
                      {"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},
                      {"name":"taskList","content":"taskItem+","group":"block","role":"list"},
                      {"name":"taskItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},
                      {"name":"text","group":"inline","role":"text"}
                    ],
                    "marks": []
                  },
                  "initialization": {"type":"localEmpty"},
                  "policy": {"inputFilter":"[0-9]"}
                }
            """.trimIndent()
        )
        try {
            val document = """
                {
                  "type": "doc",
                  "content": [
                    {"type":"paragraph","content":[{"type":"text","text":"12"}]},
                    {"type":"taskList","content":[
                      {"type":"taskItem","attrs":{"checked":false},"content":[
                        {"type":"paragraph","content":[{"type":"text","text":"Task item"}]}
                      ]}
                    ]}
                  ]
                }
            """.trimIndent()
            harness.adapter.setContentJson(document)
                ?.let { harness.editText.applyUpdateJSON(it, notifyListener = false) }
            harness.editText.setSelection(0, 2)
            val listener = RecordingEditorListener()
            harness.editText.editorListener = listener
            val toggles = mutableListOf<Pair<Int, Int>>()
            harness.editText.onToggleTaskItemCheckedAtSelectionScalarInRustForTesting = { anchor, head ->
                listener.events.add("toggle")
                toggles.add(anchor to head)
            }
            harness.editText.beginExternalTextComposition("speech-filtered-task")
            harness.editText.updateExternalTextComposition("speech-filtered-task", "letters")
            harness.editText.layoutParams = android.view.ViewGroup.LayoutParams(600, 240)
            val widthSpec = View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY)
            val heightSpec = View.MeasureSpec.makeMeasureSpec(240, View.MeasureSpec.EXACTLY)
            harness.editText.measure(widthSpec, heightSpec)
            harness.editText.layout(
                0,
                0,
                harness.editText.measuredWidth,
                harness.editText.measuredHeight
            )
            val textLayout = requireNotNull(harness.editText.layout)
            val markerIndex = harness.editText.text.toString()
                .indexOf(LayoutConstants.TASK_LIST_MARKER_UNCHECKED)
            assertTrue(markerIndex >= 0)
            val provisionalScalar = PositionBridge.utf16ToScalar(
                markerIndex,
                harness.editText.text.toString()
            )
            val markerLine = textLayout.getLineForOffset(markerIndex)
            val tapX = harness.editText.totalPaddingLeft + 1f
            val tapY = harness.editText.totalPaddingTop +
                ((textLayout.getLineTop(markerLine) + textLayout.getLineBottom(markerLine)) / 2f)

            val down = MotionEvent.obtain(0, 0, MotionEvent.ACTION_DOWN, tapX, tapY, 0)
            harness.editText.onTouchEvent(down)
            down.recycle()
            val up = MotionEvent.obtain(0, 16, MotionEvent.ACTION_UP, tapX, tapY, 0)
            harness.editText.onTouchEvent(up)
            up.recycle()

            val authoritativeText = harness.editText.text.toString()
            val authoritativeMarker = authoritativeText
                .indexOf(LayoutConstants.TASK_LIST_MARKER_UNCHECKED)
            val authoritativeScalar = PositionBridge.utf16ToScalar(
                authoritativeMarker,
                authoritativeText
            )
            assertTrue(provisionalScalar != authoritativeScalar)
            assertEquals(listOf(authoritativeScalar to authoritativeScalar), toggles)
            assertEquals(
                "interaction",
                JSONObject(listener.externalCompositionEnds.single()).getString("cause")
            )
            assertEquals(listOf("external", "toggle"), listener.events)
        } finally {
            harness.adapter.destroy()
        }
    }

    @Test
    fun `external composition maximum length failure is atomic`() {
        assertRealExternalCompositionPolicyFailure(
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"maxLength":3}}""",
            initialText = "ab",
            finalText = "long"
        )
    }

    @Test
    fun `external composition input filter failure is atomic`() {
        assertRealExternalCompositionPolicyFailure(
            configJson = """{"initialization":{"type":"localEmpty"},"policy":{"inputFilter":"[unclosed"}}""",
            initialText = "12",
            finalText = "letters"
        )
    }

    @Test
    fun `cursor caps mode treats rendered empty block start as sentence start`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        assertEquals("Hello\n\u200B", editText.text.toString())
        assertTrue(
            editText.cursorCapsModeForEditor(
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES,
                baseCapsMode = 0
            ) hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )

        val editorInfo = EditorInfo()
        val inputConnection = editText.onCreateInputConnection(editorInfo)
        assertNotNull(inputConnection)
        assertTrue(editorInfo.initialCapsMode hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
        assertTrue(
            inputConnection!!.getCursorCapsMode(InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
                hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )
    }

    @Test
    fun `text before cursor hides synthetic empty block placeholder from IME context`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderBlocksUpdateJson("Hello", "\u200B"), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)

        assertEquals("\n", inputConnection!!.getTextBeforeCursor(1, 0).toString())
        assertEquals("Hello\n", inputConnection.getTextBeforeCursor(20, 0).toString())
    }

    @Test
    fun `cursor caps mode does not force sentence caps mid line`() {
        val editText = EditorEditText(RuntimeEnvironment.getApplication())
        editText.applyUpdateJSON(renderUpdateJson("Hello "), notifyListener = false)
        editText.setSelection(editText.text?.length ?: 0)

        assertFalse(
            editText.cursorCapsModeForEditor(
                InputType.TYPE_TEXT_FLAG_CAP_SENTENCES,
                baseCapsMode = 0
            ) hasInputFlag InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        )
    }
}
