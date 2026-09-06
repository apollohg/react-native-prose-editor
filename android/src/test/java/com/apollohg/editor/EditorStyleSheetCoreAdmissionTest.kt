package com.apollohg.editor

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class EditorStyleSheetCoreAdmissionTest {
    @Test
    fun `real core language and mention style survive adapter admission`() {
        val config = """{"initialization":{"type":"localEmpty"},"schema":{"nodes":[{"name":"doc","content":"block+","role":"doc"},{"name":"paragraph","content":"inline*","group":"block","role":"textBlock"},{"name":"codeBlock","content":"text*","group":"block","role":"textBlock","attrs":{"language":{"default":null}}},{"name":"mention","group":"inline","isVoid":true,"content":"","role":"inline","attrs":{"id":{"default":""},"label":{"default":""},"mentionTheme":{"default":null}}},{"name":"text","group":"inline","role":"text"}]}}"""
        val created = UniffiEditorV2Backend.create(config, null)
        assertTrue(created.toString(), created is EditorV2CallResult.Ok)
        val id = JSONObject((created as EditorV2CallResult.Ok).value).getString("editorId")
        val adapter = requireNotNull(EditorV2Adapter.attach(UniffiEditorV2Backend, id, false))
        try {
            val update = adapter.setContentJson("""{"type":"doc","content":[{"type":"codeBlock","attrs":{"language":"rust"},"content":[{"type":"text","text":"let x = 1;"}]},{"type":"paragraph","content":[{"type":"mention","attrs":{"id":"ada","label":"Ada","mentionTheme":{"node":{"style":{"color":"#123456ff","borderLeftWidth":2,"fontWeight":"700"}}}}}]}]}""")
            assertNotNull(update)
            val blocks = JSONObject(requireNotNull(update)).getJSONArray("renderBlocks")
            assertEquals("rust", blocks.getJSONArray(0).getJSONObject(0).getString("language"))
            val mention = blocks.getJSONArray(1).getJSONObject(1)
            assertEquals("#123456ff", mention.getJSONObject("mentionTheme").getJSONObject("node").getJSONObject("style").getString("color"))
        } finally { adapter.destroy() }
    }
}
