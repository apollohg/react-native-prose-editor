package com.apollohg.editor

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import org.json.JSONArray
import org.json.JSONObject
import uniffi.editor_core.*

private fun destroyAlreadyInProgressResult(): FfiUnitResult = FfiUnitResult(
    null,
    FfiError(
        "operation",
        "OPERATION_INVALID",
        "destroy already in progress",
        null,
        null,
        null,
        null,
        null,
    ),
)

/**
 * Destroy one v2 session through its canonical public handle and invalidate
 * the associated opaque widget token. The token is supplied separately so a
 * public u64 handle is never narrowed through a signed Android id.
 */
internal fun destroyEditorThenInvalidate(
    editorHandle: String,
    viewToken: Long,
    destroy: (String) -> Unit = ::editorV2Destroy,
    beginDestroy: (Long) -> Boolean = NativeEditorViewRegistry::beginDestroy,
    finalizeDestroy: (Long) -> Unit = NativeEditorViewRegistry::finalizeDestroy
) {
    val canonicalHandle = canonicalV2U64(editorHandle) ?: return
    if (!beginDestroy(viewToken)) return
    try {
        destroy(canonicalHandle)
    } finally {
        finalizeDestroy(viewToken)
    }
}

/**
 * Module-facing destruction boundary for a paired v2 editor. Autonomous
 * callbacks, view teardown, and pairing release commit only after native
 * destruction succeeds (or the session is already gone), so a recoverable
 * failure leaves the paired session available for retry.
 */
internal fun destroyEditorV2FromModule(
    editorId: String,
    destroy: (String) -> FfiUnitResult = ::editorV2Destroy,
): FfiUnitResult {
    val canonicalHandle = canonicalV2U64(editorId) ?: return destroy(editorId)
    val viewToken = EditorV2Registry.viewTokenForHandle(canonicalHandle)
    val reservation = viewToken?.let(NativeEditorViewRegistry::acquireDestroyReservation)
        ?: NativeEditorDestroyReservationResult.UNAVAILABLE
    val reserved = reservation == NativeEditorDestroyReservationResult.RESERVED
    if (viewToken != null && !reserved) {
        // A competing destroy owns the reservation. Do not issue a second FFI
        // destroy; return a retryable operation error while teardown is pending.
        if (reservation == NativeEditorDestroyReservationResult.ALREADY_IN_PROGRESS) {
            return destroyAlreadyInProgressResult()
        }
        return FfiUnitResult(
            null,
            FfiError(
                "boundary",
                "FFI_RESULT_INVALID",
                "v2 destroy could not reserve its native view",
                null,
                null,
                null,
                null,
                null,
            ),
        )
    }
    val result = destroy(canonicalHandle)
    val error = result.error
    when {
        result.value == true && error == null -> Unit
        result.value == null &&
            error?.domain == "lifecycle" &&
            error.code in setOf("ENGINE_DESTROYED", "ENGINE_DESTROYING") -> Unit
        result.value == null && error != null -> {
            if (reserved) NativeEditorViewRegistry.rollbackDestroy(viewToken!!)
            return result
        }
        else -> {
            if (reserved) NativeEditorViewRegistry.rollbackDestroy(viewToken!!)
            return FfiUnitResult(
                null,
                FfiError(
                    "boundary",
                    "FFI_RESULT_INVALID",
                    "v2 destroy result violates the frozen unit-result shape",
                    null,
                    null,
                    null,
                    null,
                    null,
                ),
            )
        }
    }

    // Remove the pairing and its autonomous-error owner while the destroy
    // reservation still gates view commands and callback eligibility.
    EditorV2Registry.dropPair(canonicalHandle)
    if (reserved) {
        NativeEditorViewRegistry.finalizeDestroy(viewToken!!)
    }
    return result
}

// ── Frozen v2 result-record bridging ─────────────────────────────────────
// Every editorV2* module entry returns the raw UniFFI result record as a
// plain map ({ value, error } with exactly one side set); the JS bridge
// normalizes it. Every u64-shaped diagnostic is already a canonical string.

private fun FfiError.toJSMap(): Map<String, Any?> = mapOf(
    "domain" to domain,
    "code" to code,
    "message" to message,
    "requestId" to requestId,
    "operationIndex" to operationIndex,
    "limit" to limit,
    "actual" to actual,
    "detailsJson" to detailsJson,
)

internal fun EditorV2Error.toJSMap(): Map<String, Any?> = mapOf(
    "domain" to domain,
    "code" to code,
    "message" to message,
    "requestId" to requestId,
    "operationIndex" to operationIndex,
    "limit" to limit,
    "actual" to actual,
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

private fun v2InvalidFfiResultRecord(message: String): Map<String, Any?> = mapOf(
    "value" to null,
    "error" to EditorV2Error(
        domain = "boundary",
        code = "FFI_RESULT_INVALID",
        message = message,
    ).toJSMap(),
)

private fun createdV2SessionHandle(value: String, requireExactShape: Boolean): String? {
    val jsonObject = runCatching { JSONObject(value) }.getOrNull() ?: return null
    if (requireExactShape && jsonObject.length() != 1) return null
    val editorId = jsonObject.opt("editorId") as? String ?: return null
    return canonicalV2U64(editorId)?.takeIf { it != "0" }
}

private fun cleanupCreatedV2Session(value: String?, destroy: (String) -> FfiUnitResult) {
    val editorId = value?.let { createdV2SessionHandle(it, requireExactShape = false) } ?: return
    destroy(editorId)
}

/**
 * Create and pair a v2 session only from an exact FFI success with a strict
 * create value. A malformed-but-extractable result is explicitly destroyed
 * before returning the boundary failure so it cannot leak in Rust.
 */
internal fun createEditorV2FromModule(
    configJson: String,
    snapshotState: ByteArray?,
    create: (String, ByteArray?) -> FfiJsonResult = ::editorV2Create,
    destroy: (String) -> FfiUnitResult = ::editorV2Destroy,
): Map<String, Any?> {
    val result = create(configJson, snapshotState)
    val value = result.value
    val error = result.error
    if (value == null && error != null) return result.toJSMap()
    if (value == null || error != null) {
        cleanupCreatedV2Session(value, destroy)
        return v2InvalidFfiResultRecord("v2 result must carry exactly one of value/error")
    }

    val editorId = createdV2SessionHandle(value, requireExactShape = true)
    if (editorId == null) {
        cleanupCreatedV2Session(value, destroy)
        return v2InvalidFfiResultRecord("v2 create value carries no native-view editor id")
    }
    val roomBound = runCatching {
        JSONObject(configJson).optJSONObject("initialization")
            ?.optString("type") == "room"
    }.getOrDefault(false)
    val adapter = EditorV2Adapter.attach(UniffiEditorV2Backend, editorId, roomBound)
    if (adapter == null) {
        destroy(editorId)
        return v2InvalidFfiResultRecord("v2 create could not bind its editor handle")
    }
    NativeEditorViewRegistry.markEditorCreated(EditorV2Registry.register(adapter))
    return result.toJSMap()
}

private fun parseGeneration(generation: String): String? = canonicalV2U64(generation)

internal fun collaborationTickResult(
    editorId: String,
    nowMillis: String,
    tick: (String, String) -> FfiJsonResult,
): Map<String, Any?> {
    val canonicalNowMillis = canonicalV2U64(nowMillis)
        ?: return v2BoundaryErrorRecord("invalid nowMillis")
    return tick(editorId, canonicalNowMillis).toJSMap()
}

internal fun collaborationUnitResult(
    editorId: String,
    operation: (String) -> FfiUnitResult,
): Map<String, Any?> = operation(editorId).toJSMap()

class NativeEditorModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("NativeEditor")

        // ── v2 engine surface (the only construction path) ─────────────

        Function("editorV2Create") { configJson: String, snapshotState: ByteArray? ->
            createEditorV2FromModule(configJson, snapshotState)
        }
        Function("editorV2Destroy") { editorId: String ->
            destroyEditorV2FromModule(editorId).toJSMap()
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
        Function("editorV2RenderUpdate") { editorId: String, mirrorScalarAnchor: Number?, mirrorScalarHead: Number? ->
            // The render accessor for the interactive component: after a
            // JS-driven engine change the component fetches the current
            // render update here and pushes it to the bound view.
            val anchor = mirrorScalarAnchor?.let(::exactV2U32)
            val head = mirrorScalarHead?.let(::exactV2U32)
            if ((mirrorScalarAnchor != null && anchor == null) || (mirrorScalarHead != null && head == null)) {
                return@Function v2BoundaryErrorRecord("invalid render mirror")
            }
            editorV2RenderUpdate(editorId, anchor, head).toJSMap()
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
            code: Number?,
            reason: String? ->
            val parsed = parseGeneration(generation)
                ?: return@Function v2BoundaryErrorRecord("invalid generation")
            val exactCode = code?.let(::exactV2U32)
            if (code != null && exactCode == null) {
                return@Function v2BoundaryErrorRecord("invalid close code")
            }
            editorV2CollaborationSocketClose(editorId, parsed, exactCode, reason).toJSMap()
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
        Function("editorV2CollaborationTick") { editorId: String, nowMillis: String ->
            collaborationTickResult(editorId, nowMillis) { id, canonicalNowMillis ->
                editorV2CollaborationTick(id, canonicalNowMillis)
            }
        }
        Function("editorV2CollaborationDetach") { editorId: String ->
            collaborationUnitResult(editorId) { id -> editorV2CollaborationDetach(id) }
        }
        Function("editorV2CollaborationReattach") { editorId: String ->
            collaborationUnitResult(editorId) { id -> editorV2CollaborationReattach(id) }
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
                "onEditorError",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onEditorReady",
                "onToolbarAction",
                "onAddonEvent"
            )

            Prop("editorId") { view: NativeEditorExpoView, id: String? ->
                view.setEditorHandle(canonicalV2U64(id))
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
            Prop("editorUpdateEditorId") { view: NativeEditorExpoView, editorUpdateEditorId: String? ->
                view.setPendingEditorUpdateEditorHandle(canonicalV2U64(editorUpdateEditorId))
            }
            Prop("editorUpdateRevision") { view: NativeEditorExpoView, editorUpdateRevision: Number? ->
                view.setPendingEditorUpdateRevision(
                    exactV2U32(editorUpdateRevision)?.toLong()
                        ?: throw IllegalArgumentException("editorUpdateRevision must be an exact u32"),
                )
            }
            Prop("editorResetUpdateJson") { view: NativeEditorExpoView, editorResetUpdateJson: String? ->
                view.setPendingEditorResetUpdateJson(editorResetUpdateJson)
            }
            Prop("editorResetUpdateEditorId") { view: NativeEditorExpoView, editorResetUpdateEditorId: String? ->
                view.setPendingEditorResetUpdateEditorHandle(canonicalV2U64(editorResetUpdateEditorId))
            }
            Prop("editorResetUpdateRevision") { view: NativeEditorExpoView, editorResetUpdateRevision: Number? ->
                view.setPendingEditorResetUpdateRevision(
                    exactV2U32(editorResetUpdateRevision)?.toLong()
                        ?: throw IllegalArgumentException("editorResetUpdateRevision must be an exact u32"),
                )
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
