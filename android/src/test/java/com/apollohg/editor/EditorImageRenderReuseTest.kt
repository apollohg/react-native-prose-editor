package com.apollohg.editor

import android.graphics.Bitmap
import android.os.Looper
import android.view.View
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.util.concurrent.atomic.AtomicInteger

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
@GraphicsMode(GraphicsMode.Mode.NATIVE)
internal class EditorImageRenderReuseTest : EditorInputConnectionTestFixture() {
    @After
    fun resetLoader() = RenderImageLoader.resetForTesting()

    @Test
    fun `ordinary full block refresh retains loaded image geometry and ownership`() = verifyLoadedRefresh("renderBlocks")

    @Test
    fun `ordinary element refresh retains loaded image geometry and ownership`() = verifyLoadedRefresh("renderElements")

    private fun verifyLoadedRefresh(payload: String) {
        val decodes = AtomicInteger()
        RenderImageLoader.decodeSourceOverride = { _, _ ->
            decodes.incrementAndGet()
            Bitmap.createBitmap(600, 400, Bitmap.Config.ARGB_8888)
        }
        val editor = editor()
        editor.applyUpdateJSON(update("before", payload = payload))
        awaitImages(editor)
        val original = images(editor).single()
        assertEquals(original.currentSizePx().first * 2 / 3, original.currentSizePx().second)
        val height = editor.layout.height
        val generation = editor.currentImageLoadGeneration()

        editor.applyUpdateJSON(update("after", payload = payload))

        assertSame(original, images(editor).single())
        assertEquals(generation, editor.currentImageLoadGeneration())
        assertEquals(height, editor.layout.height)
        assertEquals(1, decodes.get())
        original.close()
    }

    @Test
    fun `full refresh closes unmatched images and does not reuse cancelled generations`() {
        val editor = editor()
        editor.applyUpdateJSON(update("before", source = "invalid-source"))
        val original = images(editor).single()
        editor.cancelPendingImageLoads()
        editor.applyUpdateJSON(update("after", source = "invalid-source"))
        val replacement = images(editor).single()
        assertNotSame(original, replacement)
        assertFalse(original.matches("invalid-source", null, null))

        editor.applyUpdateJSON(update("changed", source = "different-invalid-source"))
        assertFalse(replacement.matches("invalid-source", null, null))
        images(editor).single().close()
    }

    @Test
    fun `duplicate source images retain distinct spans during full refresh`() {
        val editor = editor()
        editor.applyUpdateJSON(update("before", source = "invalid-source"))
        val original = images(editor).single()
        editor.applyUpdateJSON(update("after", source = "invalid-source", imageCount = 2))
        val updated = images(editor)
        assertEquals(2, updated.size)
        assertSame(original, updated.first())
        assertNotSame(updated.first(), updated.last())
        updated.forEach(BlockImageSpan::close)
    }

    @Test
    fun `full refresh does not carry image ownership across editor driver rebind`() {
        val harness = realExternalCompositionHarness("before")
        try {
            val editor = harness.editText
            editor.applyUpdateJSON(update("before", source = "invalid-source"))
            val original = images(editor).single()
            editor.v2Driver = object : EditorV2Driver by harness.adapter {}
            editor.applyUpdateJSON(update("after", source = "invalid-source"))
            assertNotSame(original, images(editor).single())
            assertFalse(original.matches("invalid-source", null, null))
            images(editor).single().close()
        } finally { harness.adapter.destroy() }
    }

    private fun editor(): EditorEditText {
        val activity = Robolectric.buildActivity(android.app.Activity::class.java).setup().get()
        val editor = EditorEditText(activity)
        activity.setContentView(editor)
        editor.measure(View.MeasureSpec.makeMeasureSpec(600, View.MeasureSpec.EXACTLY), View.MeasureSpec.makeMeasureSpec(900, View.MeasureSpec.EXACTLY))
        editor.layout(0, 0, 600, 900)
        return editor
    }

    private fun awaitImages(editor: EditorEditText) {
        repeat(100) {
            shadowOf(Looper.getMainLooper()).idle()
            if (editor.activeImageLoadHandleCountForTesting() == 0) return
            Thread.sleep(10)
        }
        fail("Image load did not finish")
    }

    private fun images(editor: EditorEditText) = editor.text.getSpans(0, editor.length(), BlockImageSpan::class.java).toList()

    private fun update(label: String, source: String = "data:image/png;base64,AQ==", payload: String = "renderBlocks", imageCount: Int = 1): String {
        val blocks = JSONArray()
        repeat(imageCount) {
            blocks.put(JSONArray().put(JSONObject().put("type", "voidBlock").put("nodeType", "image").put("docPos", it + 1).put("attrs", JSONObject().put("src", source))))
        }
        blocks.put(JSONArray("""[{"type":"blockStart","nodeType":"paragraph","depth":0},{"type":"textRun","text":"$label","marks":[]},{"type":"blockEnd"}]"""))
        val elements = JSONArray()
        for (index in 0 until blocks.length()) {
            val block = blocks.getJSONArray(index)
            for (element in 0 until block.length()) elements.put(block.get(element))
        }
        return JSONObject().put(payload, if (payload == "renderBlocks") blocks else elements).toString()
    }
}
