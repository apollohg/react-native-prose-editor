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
internal class NativeEditorExpoViewRenderingTest : NativeEditorExpoViewTestFixture() {
    @Test
    fun `theme update applies when preflight is ready`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#ff0000"}"""

        view.setThemeJson(themeJson)

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(themeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `atoms json reapplies the current render`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val updateJson = JSONObject()
            .put(
                "renderBlocks",
                JSONArray().put(
                    JSONArray().put(
                        JSONObject()
                            .put("type", "voidBlock")
                            .put("nodeType", "counterCard")
                            .put("docPos", 1)
                    )
                )
            )
            .put("documentVersion", "1")
            .toString()
        view.richTextView.editorEditText.applyUpdateJSON(updateJson, notifyListener = false)
        val textBeforeRegistration = requireNotNull(view.richTextView.editorEditText.text)
        assertTrue(textBeforeRegistration.getSpans(0, 1, AtomBlockSpan::class.java).isEmpty())

        view.setAtomsJson(
            """{"nodeTypes":["counterCard"],"estimatedHeights":{"counterCard":120}}"""
        )

        val textAfterRegistration = requireNotNull(view.richTextView.editorEditText.text)
        assertEquals(
            120,
            textAfterRegistration.getSpans(0, 1, AtomBlockSpan::class.java)
                .single()
                .reservedHeightPx
        )
    }

    @Test
    fun `theme update queues latest value while preflight is blocked`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val firstThemeJson = """{"backgroundColor":"#00ff00"}"""
        val latestThemeJson = """{"backgroundColor":"#0000ff"}"""

        view.blockThemePreflightForTesting = true
        view.setThemeJson(firstThemeJson)
        view.setThemeJson(latestThemeJson)

        assertEquals(latestThemeJson, view.pendingThemeJsonForTesting())
        assertNull(view.lastThemeJsonForTesting())

        view.blockThemePreflightForTesting = false
        view.applyPendingThemeForTesting()

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(latestThemeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `theme update wakes after retry budget is exhausted`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#445566"}"""

        view.blockThemePreflightForTesting = true
        view.setThemeJson(themeJson)

        repeat(10) {
            shadowOf(Looper.getMainLooper()).idleFor(Duration.ofMillis(100))
        }

        assertEquals(themeJson, view.pendingThemeJsonForTesting())
        assertTrue(view.pendingThemeRetryAttemptsForTesting() <= 5)
        assertNull(view.lastThemeJsonForTesting())

        view.blockThemePreflightForTesting = false
        view.wakePendingPreflightWorkForTesting()

        assertNull(view.pendingThemeJsonForTesting())
        assertEquals(themeJson, view.lastThemeJsonForTesting())
    }

    @Test
    fun `theme update can clear an applied theme with null json`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val themeJson = """{"backgroundColor":"#ff0000"}"""

        view.setThemeJson(themeJson)
        view.setThemeJson(null)

        assertNull(view.pendingThemeJsonForTesting())
        assertNull(view.lastThemeJsonForTesting())
    }

    @Test
    fun `auto grow publishes changed height during native render application`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        val events = mutableListOf<Map<String, Any>>()
        view.onContentHeightChangeForTesting = { events += it }
        view.setHeightBehavior("autoGrow")

        val widthSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            360,
            android.view.View.MeasureSpec.EXACTLY
        )
        val heightSpec = android.view.View.MeasureSpec.makeMeasureSpec(
            0,
            android.view.View.MeasureSpec.UNSPECIFIED
        )
        editText.applyUpdateJSON(renderUpdateJson("One line"), notifyListener = false)
        view.measure(widthSpec, heightSpec)
        view.layout(0, 0, view.measuredWidth, view.measuredHeight)
        val initialHeight = events.last()["contentHeight"] as Int
        events.clear()

        editText.applyUpdateJSON(
            renderUpdateJson((1..8).joinToString("\n") { "Line $it" }),
            notifyListener = false
        )

        assertTrue("height must publish before the next looper turn", events.isNotEmpty())
        assertTrue((events.last()["contentHeight"] as Int) > initialHeight)
    }

    @Test
    fun `content size change hook ignores caret-only selection`() {
        val expoContext = testExpoContext(RuntimeEnvironment.getApplication())
        val view = NativeEditorExpoView(expoContext.context, expoContext.appContext)
        val editText = view.richTextView.editorEditText
        var contentSizeChangeCount = 0
        editText.onContentSizeMayChange = { contentSizeChangeCount += 1 }

        editText.applyUpdateJSON(renderUpdateJson("Alpha"), notifyListener = false)
        assertTrue(contentSizeChangeCount > 0)
        contentSizeChangeCount = 0

        editText.setSelection(editText.length())

        assertEquals(0, contentSizeChangeCount)
    }
}
