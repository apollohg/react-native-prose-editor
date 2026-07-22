package com.apollohg.editor

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorModuleTest {
    @Test
    fun `native unsigned argument helper rejects negative values`() {
        assertNull(nativeULong(-1))
    }

    @Test
    fun `native unsigned argument helper keeps non-negative values`() {
        assertEquals(0UL, nativeULong(0))
        assertEquals(42UL, nativeULong(42))
    }

    @Test
    fun `render probe flattens v2 render blocks into a render elements array`() {
        val update = JSONObject()
            .put(
                "renderBlocks",
                JSONArray()
                    .put(
                        JSONArray()
                            .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph"))
                            .put(JSONObject().put("type", "textRun").put("text", "Hello"))
                            .put(JSONObject().put("type", "blockEnd"))
                    )
                    .put(
                        JSONArray()
                            .put(JSONObject().put("type", "blockStart").put("nodeType", "paragraph"))
                            .put(JSONObject().put("type", "blockEnd"))
                    )
            )
            .toString()

        val elements = JSONArray(renderElementsJsonFromUpdate(update))

        assertEquals(5, elements.length())
        assertEquals("blockStart", elements.getJSONObject(0).getString("type"))
        assertEquals("Hello", elements.getJSONObject(1).getString("text"))
        assertEquals("blockEnd", elements.getJSONObject(4).getString("type"))
    }

    @Test
    fun `render probe passes through an already flat render elements payload`() {
        val flat = JSONArray()
            .put(JSONObject().put("type", "textRun").put("text", "Hi"))
        val update = JSONObject().put("renderElements", flat).toString()

        assertEquals(flat.toString(), renderElementsJsonFromUpdate(update))
    }

    @Test
    fun `render probe reports a boundary error when the update carries no render payload`() {
        val parsed = JSONObject(renderElementsJsonFromUpdate("{\"historyState\":{}}"))

        val error = parsed.getJSONObject("error")
        assertEquals("boundary", error.getString("domain"))
        assertEquals("FFI_RESULT_INVALID", error.getString("code"))
    }

    @Test
    fun `render probe reports a boundary error for invalid update json`() {
        val parsed = JSONObject(renderElementsJsonFromUpdate("not json"))

        assertTrue(parsed.getJSONObject("error").getString("message").isNotEmpty())
    }
}
