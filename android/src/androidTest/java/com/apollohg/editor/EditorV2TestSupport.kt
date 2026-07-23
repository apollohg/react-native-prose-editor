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
    val adapter = EditorV2Adapter.attach(UniffiEditorV2Backend, editorId, roomBound = false)
        ?: error("v2 editor create returned an unattached handle")
    val viewToken = EditorV2Registry.register(adapter)
    return adapter to viewToken
}

/** Release a test pairing by its opaque view token after destroying its engine session. */
internal fun releasePairedV2TestEditor(viewToken: Long) {
    val handle = EditorV2Registry.handleForViewToken(viewToken) ?: return
    try {
        EditorV2Registry.adapterForViewToken(viewToken)?.destroy()
    } finally {
        EditorV2Registry.dropPair(handle)
    }
}
