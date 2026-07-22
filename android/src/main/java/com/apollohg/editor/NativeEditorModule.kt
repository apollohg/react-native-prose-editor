package com.apollohg.editor

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONArray
import org.json.JSONObject
import uniffi.editor_core.*

internal fun nativeULong(value: Int): ULong? =
    if (value >= 0) value.toULong() else null

/**
 * Destroy one v2 session and invalidate every view bound to its public id.
 * The registry bookkeeping always runs, even when the engine destroy fails.
 */
internal fun destroyEditorThenInvalidate(
    editorId: ULong,
    destroy: (ULong) -> Unit = { id -> editorV2Destroy(id.toString()) },
    beginDestroy: (Long) -> Boolean = NativeEditorViewRegistry::beginDestroy,
    finalizeDestroy: (Long) -> Unit = NativeEditorViewRegistry::finalizeDestroy
) {
    val signedEditorId = editorId.toLong()
    if (!beginDestroy(signedEditorId)) return
    try {
        destroy(editorId)
    } finally {
        finalizeDestroy(signedEditorId)
    }
}

// ── Frozen v2 result-record bridging ─────────────────────────────────────
// Every editorV2* module entry returns the raw UniFFI result record as a
// plain map ({ value, error } with exactly one side set); the JS bridge
// normalizes it. ULong error fields cross as safe-integer numbers.

private fun FfiError.toJSMap(): Map<String, Any?> = mapOf(
    "domain" to domain,
    "code" to code,
    "message" to message,
    "requestId" to requestId,
    "operationIndex" to operationIndex?.toLong(),
    "limit" to limit?.toLong(),
    "actual" to actual?.toLong(),
    "detailsJson" to detailsJson,
)

private fun EditorV2Error.toJSMap(): Map<String, Any?> = mapOf(
    "domain" to domain,
    "code" to code,
    "message" to message,
    "requestId" to requestId,
    "operationIndex" to operationIndex?.toLong(),
    "limit" to limit?.toLong(),
    "actual" to actual?.toLong(),
    "detailsJson" to detailsJson,
)

private fun FfiJsonResult.toJSMap(): Map<String, Any?> =
    mapOf("value" to value, "error" to error?.toJSMap())

private fun FfiBytesResult.toJSMap(): Map<String, Any?> =
    mapOf("value" to value, "error" to error?.toJSMap())

private fun FfiUnitResult.toJSMap(): Map<String, Any?> =
    mapOf("value" to value, "error" to error?.toJSMap())

private fun FfiSnapshotExportResult.toJSMap(): Map<String, Any?> = mapOf(
    "value" to value?.let {
        mapOf("metadataJson" to it.metadataJson, "encodedState" to it.encodedState)
    },
    "error" to error?.toJSMap(),
)

private fun v2BoundaryErrorRecord(message: String): Map<String, Any?> = mapOf(
    "value" to null,
    "error" to mapOf(
        "domain" to "boundary",
        "code" to "CONFIG_INVALID",
        "message" to message,
        "requestId" to null,
        "operationIndex" to null,
        "limit" to null,
        "actual" to null,
        "detailsJson" to null,
    ),
)

private fun parseGeneration(generation: String): ULong? =
    generation.toULongOrNull()

class NativeEditorModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("NativeEditor")

        // ── v2 engine surface (the only construction path) ─────────────

        Function("editorV2Create") { configJson: String, snapshotState: ByteArray? ->
            val result = editorV2Create(configJson, snapshotState)
            var pairingError: Map<String, Any?>? = null
            result.value?.let { value ->
                val editorId = runCatching {
                    JSONObject(value).getString("editorId")
                }.getOrNull()
                val signedId = editorId?.toLongOrNull()
                if (editorId == null || signedId == null || signedId <= 0) {
                    editorId?.let(::editorV2Destroy)
                    pairingError = mapOf(
                        "value" to null,
                        "error" to EditorV2Error(
                            domain = "boundary",
                            code = "FFI_RESULT_INVALID",
                            message = "v2 create value carries no native-view editor id",
                        ).toJSMap(),
                    )
                } else {
                    // Mark the public id live so views may bind to it, and
                    // pair the view-facing adapter with the JS-created
                    // session: the Expo view receives the handle's editorId
                    // and routes every interaction through the shared
                    // session. Ids beyond Long range skip view-registry
                    // tracking.
                    NativeEditorViewRegistry.markEditorCreated(signedId)
                    val roomBound = runCatching {
                        JSONObject(configJson).optJSONObject("initialization")
                            ?.optString("type") == "room"
                    }.getOrDefault(false)
                    val adapter = EditorV2Adapter.attach(UniffiEditorV2Backend, editorId, roomBound)
                    if (adapter == null) {
                        editorV2Destroy(editorId)
                        NativeEditorViewRegistry.invalidateDestroyedEditor(signedId)
                        pairingError = mapOf(
                            "value" to null,
                            "error" to EditorV2Error(
                                domain = "boundary",
                                code = "FFI_RESULT_INVALID",
                                message = "v2 create could not bind its editor handle",
                            ).toJSMap(),
                        )
                    } else {
                        EditorV2Registry.register(adapter, signedId)
                    }
                }
            }
            pairingError ?: result.toJSMap()
        }
        Function("editorV2Destroy") { editorId: String ->
            val result = editorV2Destroy(editorId)
            editorId.toLongOrNull()?.let { signedId ->
                NativeEditorViewRegistry.invalidateDestroyedEditor(signedId)
                EditorV2Registry.destroyPair(signedId)
            }
            result.toJSMap()
        }
        Function("editorV2GetState") { editorId: String ->
            editorV2GetState(editorId).toJSMap()
        }
        Function("editorV2GetDocumentJson") { editorId: String ->
            editorV2GetDocumentJson(editorId).toJSMap()
        }
        Function("editorV2GetDocumentHtml") { editorId: String ->
            editorV2GetDocumentHtml(editorId).toJSMap()
        }
        Function("editorV2GetContentSnapshot") { editorId: String ->
            editorV2GetContentSnapshot(editorId).toJSMap()
        }
        Function("editorV2ReplaceDocument") { editorId: String, requestJson: String ->
            editorV2ReplaceDocument(editorId, requestJson).toJSMap()
        }
        Function("editorV2ApplyInput") { editorId: String, requestJson: String ->
            editorV2ApplyInput(editorId, requestJson).toJSMap()
        }
        Function("editorV2ApplyCommand") { editorId: String, requestJson: String ->
            editorV2ApplyCommand(editorId, requestJson).toJSMap()
        }
        Function("editorV2ApplyLocalApi") { editorId: String, requestJson: String ->
            editorV2ApplyLocalApi(editorId, requestJson).toJSMap()
        }
        Function("editorV2SetSelection") { editorId: String, requestJson: String ->
            editorV2SetSelection(editorId, requestJson).toJSMap()
        }
        Function("editorV2Undo") { editorId: String, requestJson: String ->
            editorV2Undo(editorId, requestJson).toJSMap()
        }
        Function("editorV2Redo") { editorId: String, requestJson: String ->
            editorV2Redo(editorId, requestJson).toJSMap()
        }
        Function("editorV2RenderUpdate") { editorId: String, mirrorScalarAnchor: Int?, mirrorScalarHead: Int? ->
            // The render accessor for the interactive component: after a
            // JS-driven engine change the component fetches the current
            // render update here and pushes it to the bound view.
            editorV2RenderUpdate(
                editorId,
                mirrorScalarAnchor?.let { if (it >= 0) it.toUInt() else null },
                mirrorScalarHead?.let { if (it >= 0) it.toUInt() else null },
            ).toJSMap()
        }

        // ── v2 collaboration runtime ─────────────────────────────────────

        Function("editorV2CollaborationBeginConnect") { editorId: String ->
            editorV2CollaborationBeginConnect(editorId).toJSMap()
        }
        Function("editorV2CollaborationSocketOpen") { editorId: String, generation: String ->
            val parsed = parseGeneration(generation)
                ?: return@Function v2BoundaryErrorRecord("invalid generation")
            editorV2CollaborationSocketOpen(editorId, parsed).toJSMap()
        }
        Function("editorV2CollaborationReceive") { editorId: String, generation: String, message: ByteArray ->
            val parsed = parseGeneration(generation)
                ?: return@Function v2BoundaryErrorRecord("invalid generation")
            editorV2CollaborationReceive(editorId, parsed, message).toJSMap()
        }
        Function("editorV2CollaborationSocketClose") {
            editorId: String,
            generation: String,
            code: Int?,
            reason: String? ->
            val parsed = parseGeneration(generation)
                ?: return@Function v2BoundaryErrorRecord("invalid generation")
            if (code != null && code < 0) {
                return@Function v2BoundaryErrorRecord("invalid close code")
            }
            editorV2CollaborationSocketClose(editorId, parsed, code?.toUInt(), reason).toJSMap()
        }
        Function("editorV2CollaborationTakeOutbound") { editorId: String, generation: String ->
            val parsed = parseGeneration(generation)
                ?: return@Function v2BoundaryErrorRecord("invalid generation")
            editorV2CollaborationTakeOutbound(editorId, parsed).toJSMap()
        }
        Function("editorV2CollaborationSetAwareness") { editorId: String, awarenessJson: String ->
            editorV2CollaborationSetAwareness(editorId, awarenessJson).toJSMap()
        }
        Function("editorV2CollaborationPeers") { editorId: String ->
            editorV2CollaborationPeers(editorId).toJSMap()
        }

        // ── v2 snapshots ───────────────────────────────────────────────

        Function("editorV2SnapshotExport") { editorId: String ->
            editorV2SnapshotExport(editorId).toJSMap()
        }
        Function("editorV2SnapshotRestore") { editorId: String, metadataJson: String, encodedState: ByteArray ->
            editorV2SnapshotRestore(editorId, metadataJson, encodedState).toJSMap()
        }

        // ── Stateless render probes (NativeProseViewer) ────────────────
        // A transient v2 session applies the content and reports the
        // flattened render-elements array; the session is always destroyed.

        Function("renderDocumentJson") { configJson: String, json: String ->
            renderDocumentProbe(configJson) { adapter -> adapter.setContentJson(json) }
        }
        Function("measureContentHeight") { renderJson: String, themeJson: String?, width: Double ->
            val density = appContext.reactContext?.resources?.displayMetrics?.density ?: 1f
            val height = RenderBridge.measureHeight(
                json = renderJson,
                themeJson = themeJson,
                width = width.toFloat(),
                density = density
            )
            height.toDouble()
        }
        Function("renderDocumentHtml") { configJson: String, html: String ->
            renderDocumentProbe(configJson) { adapter -> adapter.setContentHtml(html) }
        }

        View(NativeEditorExpoView::class) {
            Events(
                "onEditorUpdate",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onEditorReady",
                "onToolbarAction",
                "onAddonEvent"
            )

            Prop("editorId") { view: NativeEditorExpoView, id: Int ->
                view.setEditorId(nativeULong(id)?.toLong() ?: 0L)
            }
            Prop("editable") { view: NativeEditorExpoView, editable: Boolean ->
                view.setEditable(editable)
            }
            Prop("accessibilityLabel") { view: NativeEditorExpoView, label: String? ->
                view.setAccessibilityLabel(label)
            }
            Prop("accessibilityHint") { view: NativeEditorExpoView, hint: String? ->
                view.setAccessibilityHint(hint)
            }
            Prop("placeholder") { view: NativeEditorExpoView, placeholder: String ->
                view.richTextView.editorEditText.placeholderText = placeholder
            }
            Prop("autoFocus") { view: NativeEditorExpoView, autoFocus: Boolean ->
                view.setAutoFocus(autoFocus)
            }
            Prop("autoCapitalize") { view: NativeEditorExpoView, autoCapitalize: String? ->
                view.setAutoCapitalize(autoCapitalize)
            }
            Prop("autoCorrect") { view: NativeEditorExpoView, autoCorrect: Boolean? ->
                view.setAutoCorrect(autoCorrect)
            }
            Prop("keyboardType") { view: NativeEditorExpoView, keyboardType: String? ->
                view.setKeyboardType(keyboardType)
            }
            Prop("showToolbar") { view: NativeEditorExpoView, showToolbar: Boolean ->
                view.setShowToolbar(showToolbar)
            }
            Prop("toolbarPlacement") { view: NativeEditorExpoView, toolbarPlacement: String? ->
                view.setToolbarPlacement(toolbarPlacement)
            }
            Prop("heightBehavior") { view: NativeEditorExpoView, heightBehavior: String ->
                view.setHeightBehavior(heightBehavior)
            }
            Prop("allowImageResizing") { view: NativeEditorExpoView, allowImageResizing: Boolean ->
                view.setAllowImageResizing(allowImageResizing)
            }
            Prop("imageLoadingPolicyJson") { view: NativeEditorExpoView, policyJson: String? ->
                view.setImageLoadingPolicyJson(policyJson)
            }
            Prop("themeJson") { view: NativeEditorExpoView, themeJson: String? ->
                view.setThemeJson(themeJson)
            }
            Prop("addonsJson") { view: NativeEditorExpoView, addonsJson: String? ->
                view.setAddonsJson(addonsJson)
            }
            Prop("remoteSelectionsJson") { view: NativeEditorExpoView, remoteSelectionsJson: String? ->
                view.setRemoteSelectionsJson(remoteSelectionsJson)
            }
            Prop("toolbarItemsJson") { view: NativeEditorExpoView, toolbarItemsJson: String? ->
                view.setToolbarItemsJson(toolbarItemsJson)
            }
            Prop("toolbarFrameJson") { view: NativeEditorExpoView, toolbarFrameJson: String? ->
                view.setToolbarFrameJson(toolbarFrameJson)
            }
            Prop("editorUpdateJson") { view: NativeEditorExpoView, editorUpdateJson: String? ->
                view.setPendingEditorUpdateJson(editorUpdateJson)
            }
            Prop("editorUpdateEditorId") { view: NativeEditorExpoView, editorUpdateEditorId: Int? ->
                view.setPendingEditorUpdateEditorId(editorUpdateEditorId?.let { nativeULong(it)?.toLong() })
            }
            Prop("editorUpdateRevision") { view: NativeEditorExpoView, editorUpdateRevision: Int ->
                view.setPendingEditorUpdateRevision(editorUpdateRevision)
            }
            Prop("editorResetUpdateJson") { view: NativeEditorExpoView, editorResetUpdateJson: String? ->
                view.setPendingEditorResetUpdateJson(editorResetUpdateJson)
            }
            Prop("editorResetUpdateEditorId") { view: NativeEditorExpoView, editorResetUpdateEditorId: Int? ->
                view.setPendingEditorResetUpdateEditorId(editorResetUpdateEditorId?.let { nativeULong(it)?.toLong() })
            }
            Prop("editorResetUpdateRevision") { view: NativeEditorExpoView, editorResetUpdateRevision: Int ->
                view.setPendingEditorResetUpdateRevision(editorResetUpdateRevision)
            }
            OnViewDidUpdateProps { view: NativeEditorExpoView ->
                view.applyPendingEditorResetUpdateIfNeeded()
                view.applyPendingEditorUpdateIfNeeded()
            }

            AsyncFunction("focus") { view: NativeEditorExpoView ->
                view.focus()
            }
            AsyncFunction("blur") { view: NativeEditorExpoView ->
                view.blur()
            }

            AsyncFunction("getCaretRect") { view: NativeEditorExpoView ->
                view.getCaretRectJson()
            }

            AsyncFunction("applyEditorUpdate") { view: NativeEditorExpoView, updateJson: String ->
                view.applyEditorUpdate(updateJson)
            }

            AsyncFunction("applyEditorResetUpdate") { view: NativeEditorExpoView, updateJson: String ->
                view.applyEditorResetUpdate(updateJson)
            }

        }

        View(NativeProseViewerExpoView::class) {
            Name("NativeProseViewer")
            Events("onContentHeightChange", "onPressLink", "onPressMention")

            Prop("renderJson") { view: NativeProseViewerExpoView, renderJson: String? ->
                view.setRenderJson(renderJson)
            }
            Prop("themeJson") { view: NativeProseViewerExpoView, themeJson: String? ->
                view.setThemeJson(themeJson)
            }
            Prop("imageLoadingPolicyJson") { view: NativeProseViewerExpoView, policyJson: String? ->
                view.setImageLoadingPolicyJson(policyJson)
            }
            Prop("collapsesWhenEmpty") {
                view: NativeProseViewerExpoView,
                collapsesWhenEmpty: Boolean? ->
                view.setCollapsesWhenEmpty(collapsesWhenEmpty)
            }
            Prop("enableLinkTaps") { view: NativeProseViewerExpoView, enableLinkTaps: Boolean? ->
                view.setEnableLinkTaps(enableLinkTaps)
            }
            Prop("interceptLinkTaps") { view: NativeProseViewerExpoView, interceptLinkTaps: Boolean? ->
                view.setInterceptLinkTaps(interceptLinkTaps)
            }
        }
    }
}

// ── Render-probe plumbing ────────────────────────────────────────────────

private fun probeErrorJson(error: EditorV2Error): String =
    JSONObject()
        .put(
            "error",
            JSONObject()
                .put("domain", error.domain)
                .put("code", error.code)
                .put("message", error.message),
        )
        .toString()

private fun probeContractErrorJson(message: String): String =
    probeErrorJson(EditorV2Error(domain = "boundary", code = "FFI_RESULT_INVALID", message = message))

/**
 * The probe contract: a flat JSON array of render elements (what the legacy
 * set-content probes returned). The v2 render accessor emits block form, so
 * blocks are flattened in order; a pre-flattened payload passes through.
 */
internal fun renderElementsJsonFromUpdate(updateJson: String): String {
    val update = runCatching { JSONObject(updateJson) }.getOrNull()
        ?: return probeContractErrorJson("v2 render update is not valid JSON")
    update.optJSONArray("renderElements")?.let { return it.toString() }
    val blocks = update.optJSONArray("renderBlocks")
        ?: return probeContractErrorJson("v2 render update carries no render payload")
    val elements = JSONArray()
    for (blockIndex in 0 until blocks.length()) {
        val block = blocks.optJSONArray(blockIndex) ?: continue
        for (elementIndex in 0 until block.length()) {
            elements.put(block.opt(elementIndex))
        }
    }
    return elements.toString()
}

private fun renderDocumentProbe(configJson: String, apply: (EditorV2Adapter) -> String?): String {
    val adapter = when (val created = UniffiEditorV2Backend.create(configJson, snapshotState = null)) {
        is EditorV2CallResult.Err -> return probeErrorJson(created.error)
        is EditorV2CallResult.Ok -> {
            val editorId = runCatching { JSONObject(created.value).getString("editorId") }.getOrNull()
                ?: return probeContractErrorJson("v2 render probe create carries no editor id")
            val adapter = EditorV2Adapter.attach(UniffiEditorV2Backend, editorId, roomBound = false)
            if (adapter == null) {
                UniffiEditorV2Backend.destroy(editorId)
                return probeContractErrorJson("v2 render probe could not bind its created editor")
            }
            adapter
        }
    }
    try {
        val updateJson = apply(adapter)
            ?: return probeContractErrorJson("v2 render probe could not apply content")
        return renderElementsJsonFromUpdate(updateJson)
    } finally {
        adapter.destroy()
    }
}
