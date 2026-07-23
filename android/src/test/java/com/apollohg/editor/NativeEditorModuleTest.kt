package com.apollohg.editor

import java.math.BigDecimal
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import uniffi.editor_core.FfiJsonResult
import uniffi.editor_core.FfiUnitResult

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class NativeEditorModuleTest {
    @Test
    fun `generation parser rejects every non-canonical decimal spelling`() {
        val parser = Class.forName("com.apollohg.editor.NativeEditorModuleKt")
            .getDeclaredMethod("parseGeneration", String::class.java)
            .apply { isAccessible = true }

        assertEquals("18446744073709551615", parser.invoke(null, "18446744073709551615"))
        for (value in listOf("+1", "01", " 1", "1 ", "1e3")) {
            assertNull("generation $value must be rejected", parser.invoke(null, value))
        }
    }

    @Test
    fun `collaboration tick forwards canonical maximum and raw json result`() {
        var forwardedEditorId: String? = null
        var forwardedNowMillis: String? = null
        val rawValue = """{"nextDeadlineMillis":null,"renewedLocal":false,"expiredPeers":[],"outboundChanged":false,"peersChanged":false}"""

        val result = collaborationTickResult("editor-1", ULong.MAX_VALUE.toString()) { editorId, nowMillis ->
            forwardedEditorId = editorId
            forwardedNowMillis = nowMillis
            FfiJsonResult(rawValue, null)
        }

        assertEquals("editor-1", forwardedEditorId)
        assertEquals(ULong.MAX_VALUE.toString(), forwardedNowMillis)
        assertEquals(rawValue, result["value"])
        assertNull(result["error"])
    }

    @Test
    fun `collaboration tick rejects malformed nowMillis before backend`() {
        var called = false

        val result = collaborationTickResult("editor-1", "01") { _, _ ->
            called = true
            FfiJsonResult("{}", null)
        }

        assertFalse(called)
        val error = result["error"] as Map<*, *>
        assertEquals("CONFIG_INVALID", error["code"])
    }

    @Test
    fun `collaboration detach and reattach bridge raw unit results`() {
        val invoked = mutableListOf<String>()
        val detach = collaborationUnitResult("editor-1") { editorId ->
            invoked += "detach:$editorId"
            FfiUnitResult(true, null)
        }
        val reattach = collaborationUnitResult("editor-1") { editorId ->
            invoked += "reattach:$editorId"
            FfiUnitResult(true, null)
        }

        assertEquals(listOf("detach:editor-1", "reattach:editor-1"), invoked)
        assertEquals(true, detach["value"])
        assertNull(detach["error"])
        assertEquals(true, reattach["value"])
        assertNull(reattach["error"])
    }

    @Test
    fun `v2 u32 parser admits only exact finite integral values`() {
        assertEquals(UInt.MAX_VALUE, exactV2U32(4_294_967_295L))
        assertEquals(0u, exactV2U32(0))
        for (value in listOf<Number>(
            -1,
            1.5,
            Double.NaN,
            Double.POSITIVE_INFINITY,
            4_294_967_296L,
            BigDecimal("1.0000000000000000001"),
        )) {
            assertNull("u32 $value must be rejected", exactV2U32(value))
        }
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
