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
internal class NativeEditorExpoViewCompositionTest : NativeEditorExpoViewTestFixture() {
    @Test
    fun `Android input options report and clear private IME options`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)

        view.setAndroidInputOptionsJson("""{"privateImeOptions":"nm"}""")
        val configuredEditorInfo = EditorInfo()
        assertNotNull(view.richTextView.editorEditText.onCreateInputConnection(configuredEditorInfo))
        assertEquals("nm", configuredEditorInfo.privateImeOptions)

        view.setAndroidInputOptionsJson(null)
        val clearedEditorInfo = EditorInfo()
        assertNotNull(view.richTextView.editorEditText.onCreateInputConnection(clearedEditorInfo))
        assertNull(clearedEditorInfo.privateImeOptions)
    }

    @Test
    fun `destroyed editor invalidation from background times out until main cleanup runs`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editorId = 77884L
        val completed = AtomicBoolean(false)

        NativeEditorViewRegistry.markEditorCreated(editorId)
        view.richTextView.setEditorIdWhileDetached(editorId)
        val editText = view.richTextView.editorEditText
        editText.applyUpdateJSON(renderUpdateJson("ready"), notifyListener = false)
        editText.setSelection(5)
        editText.editorId = editorId
        var insertedText: String? = null
        var syncedSelection: Pair<Int, Int>? = null
        editText.onInsertTextInRustForTesting = { text, _ -> insertedText = text }
        editText.onSetSelectionScalarInRustForTesting = { anchor, head ->
            syncedSelection = anchor to head
        }
        val inputConnection = editText.onCreateInputConnection(EditorInfo())
        assertNotNull(inputConnection)
        NativeEditorViewRegistry.register(editorId, view)

        val thread = Thread {
            NativeEditorViewRegistry.invalidateDestroyedEditor(editorId)
            completed.set(true)
        }
        thread.start()
        thread.join(1000)

        assertFalse(thread.isAlive)
        assertTrue(completed.get())
        assertEquals(editorId, view.richTextView.editorId)
        assertFalse(NativeEditorViewRegistry.register(editorId, view))
        assertTrue(inputConnection!!.commitText("x", 1))
        editText.setSelection(0)
        assertNull(insertedText)
        assertNull(syncedSelection)

        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(0L, view.richTextView.editorId)
        val preparation = JSONObject(NativeEditorViewRegistry.prepareForCommandJSON(editorId))
        assertFalse(preparation.getBoolean("ready"))
        assertEquals("destroyed", preparation.getString("blockedReason"))
    }

    @Test
    fun `detach preflight flushes pending composition before unregistering`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText

        view.richTextView.setEditorIdWhileDetached(77889L)
        editText.setSelection(0)
        editText.editorId = 77889L

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
            editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
        }

        val inputConnection = editText.onCreateInputConnection(android.view.inputmethod.EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        view.handleDetachedFromWindowForTesting()

        assertEquals("abc", insertedText)

        NativeEditorViewRegistry.unregister(77889L, view)
    }

    @Test
    fun `child detach preflight flushes pending composition before editor unbind`() {
        val activity = Robolectric.buildActivity(Activity::class.java).setup().get()
        val parent = FrameLayout(activity)
        activity.setContentView(parent)
        val expoContext = testExpoContext(activity)
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val editorId = 778891L

        parent.addView(view)
        view.richTextView.setEditorIdWhileDetached(editorId)
        editText.applyUpdateJSON(renderUpdateJson(""), notifyListener = false)
        editText.setSelection(0)
        editText.editorId = editorId

        var insertedText: String? = null
        editText.onInsertTextInRustForTesting = { text, _ ->
            insertedText = text
            editText.applyUpdateJSON(renderUpdateJson(text), notifyListener = false)
        }

        val inputConnection = editText.onCreateInputConnection(android.view.inputmethod.EditorInfo())
        assertNotNull(inputConnection)
        assertTrue(inputConnection!!.setComposingText("abc", 1))

        parent.removeView(view)

        assertEquals("abc", insertedText)
        assertEquals(0L, editText.editorId)

        NativeEditorViewRegistry.unregister(editorId, view)
    }
}
