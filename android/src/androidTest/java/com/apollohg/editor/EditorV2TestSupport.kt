package com.apollohg.editor

import org.json.JSONObject

/** Test-only direct v2 creation: complete envelope first, then attach. */
internal fun createPairedV2TestEditor(): Pair<EditorV2Adapter, Long> {
    val created = when (
        val result = UniffiEditorV2Backend.create(
            """{"initialization":{"type":"localEmpty"}}""",
            snapshotState = null,
        )
    ) {
        is EditorV2CallResult.Ok -> result.value
        is EditorV2CallResult.Err ->
            error("v2 editor create failed: ${result.error.code}: ${result.error.message}")
    }
    val editorId = JSONObject(created).getString("editorId")
    val publicId = editorId.toLongOrNull()
        ?: error("v2 editor handle is outside the current native view id range")
    val adapter = EditorV2Adapter.attach(UniffiEditorV2Backend, editorId, roomBound = false)
        ?: error("v2 editor create returned an unattached handle")
    EditorV2Registry.register(adapter, publicId)
    return adapter to publicId
}
