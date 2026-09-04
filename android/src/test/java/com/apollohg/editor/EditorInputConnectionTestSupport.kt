package com.apollohg.editor

import org.json.JSONArray
import org.json.JSONObject
import org.robolectric.RuntimeEnvironment

abstract class EditorInputConnectionTestSupport {
    protected fun renderUpdateJson(text: String): String =
        renderBlocksUpdateJson(text)
    protected fun renderBlocksUpdateJson(vararg texts: String): String =
        JSONObject()
            .put(
                "renderBlocks",
                JSONArray().apply {
                    texts.forEach { put(paragraphRenderBlock(it)) }
                }
            )
            .toString()

    protected fun renderPatchUpdateJson(startIndex: Int, replacementText: String): String =
        JSONObject()
            .put(
                "renderPatch",
                JSONObject()
                    .put("startIndex", startIndex)
                    .put("deleteCount", 1)
                    .put("renderBlocks", JSONArray().put(paragraphRenderBlock(replacementText)))
            )
            .toString()

    protected fun paragraphRenderBlock(text: String): JSONArray =
        JSONArray()
            .put(
                JSONObject()
                    .put("type", "blockStart")
                    .put("nodeType", "paragraph")
                    .put("depth", 0)
            )
            .put(
                JSONObject()
                    .put("type", "textRun")
                    .put("text", text)
                    .put("marks", JSONArray())
            )
            .put(JSONObject().put("type", "blockEnd"))

    @ConsistentCopyVisibility
    protected data class ExternalCompositionHarness internal constructor(
        internal val backend: FakeEditorV2Backend,
        val editorId: String,
        internal val adapter: EditorV2Adapter,
        val editText: EditorEditText
    )

    protected fun externalCompositionHarness(
        initialText: String,
        configJson: String = """{"initialization":{"type":"localEmpty"}}""",
        roomBound: Boolean = false
    ): ExternalCompositionHarness {
        val backend = FakeEditorV2Backend()
        val created = backend.create(configJson, null) as EditorV2CallResult.Ok
        val editorId = JSONObject(created.value).getString("editorId")
        val adapter = EditorV2Adapter.attach(backend, editorId, roomBound = roomBound)!!
        val editText = EditorEditText(RuntimeEnvironment.getApplication()).apply {
            this.editorId = 1
            v2Driver = adapter
        }
        adapter.setContentHtml("<p>$initialText</p>")
            ?.let { editText.applyUpdateJSON(it, notifyListener = false) }
        return ExternalCompositionHarness(backend, editorId, adapter, editText)
    }
    protected infix fun Int.hasInputFlag(flag: Int): Boolean = (this and flag) == flag
}
