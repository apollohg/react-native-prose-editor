package com.apollohg.editor

import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import com.apollohg.editor.viewer.PreparedProseInstrumentation
import com.apollohg.editor.viewer.PreparedProseLayoutRegistry
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
 * Module-facing destruction transaction for a canonical v2 handle. A paired
 * editor additionally owns a view reservation; an unpaired editor still owns
 * the handle transaction. Recoverable results leave both routes retryable.
 */
internal fun destroyEditorV2FromModule(
    editorId: String,
    destroy: (String) -> FfiUnitResult = ::editorV2Destroy,
): FfiUnitResult {
    val canonicalHandle = canonicalV2U64(editorId) ?: return destroy(editorId)
    if (
        EditorV2Registry.acquireHandleDestroyReservation(canonicalHandle) !=
            EditorV2DestroyReservationResult.RESERVED
    ) {
        return destroyAlreadyInProgressResult()
    }
    var releaseHandleAfterViewFinalization = false
    try {
        // Pair lookup is intentionally after canonical-handle ownership has
        // been acquired. The pairing can disappear during terminal teardown;
        // the handle reservation cannot.
        val viewToken = EditorV2Registry.viewTokenForHandle(canonicalHandle)
        val reservation = viewToken?.let(NativeEditorViewRegistry::acquireDestroyReservation)
            ?: NativeEditorDestroyReservationResult.UNAVAILABLE
        val reserved = reservation == NativeEditorDestroyReservationResult.RESERVED
        if (viewToken != null && !reserved) {
            if (reservation == NativeEditorDestroyReservationResult.ALREADY_IN_PROGRESS) {
                return destroyAlreadyInProgressResult()
            }
            return invalidDestroyResult("v2 destroy could not reserve its native view")
        }

        NativeCollaborationTransportRegistry.destroy(canonicalHandle)
        val result = destroy(canonicalHandle)
        invokeDestroyTestingHook(
            EditorV2Registry.onDestroyFfiResultReceivedForTesting,
            canonicalHandle,
        )
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
                return invalidDestroyResult("v2 destroy result violates the frozen unit-result shape")
            }
        }

        // The owner clears callback ownership and removes the pairing while
        // both reservations still gate callbacks and view commands.
        EditorV2Registry.dropPair(canonicalHandle)
        invokeDestroyTestingHook(
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting,
            canonicalHandle,
        )
        if (reserved) {
            // Off-main finalization can finish after its bounded wait. Keep
            // canonical-handle ownership until the view reservation releases,
            // not merely until this call returns.
            releaseHandleAfterViewFinalization = true
            NativeEditorViewRegistry.finalizeDestroy(viewToken!!) {
                EditorV2Registry.releaseHandleDestroyReservation(canonicalHandle)
            }
        }
        return result
    } finally {
        // This is deliberately last: contenders must remain blocked through
        // pairing removal and native-view invalidation. A paired off-main
        // finalization releases it from that finalization's completion.
        if (!releaseHandleAfterViewFinalization) {
            EditorV2Registry.releaseHandleDestroyReservation(canonicalHandle)
        }
    }
}

private fun invalidDestroyResult(message: String): FfiUnitResult = FfiUnitResult(
    null,
    FfiError(
        "boundary",
        "FFI_RESULT_INVALID",
        message,
        null,
        null,
        null,
        null,
        null,
    ),
)

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

private fun mutationResult(
    editorId: String,
    result: FfiJsonResult,
): Map<String, Any?> {
    if (result.error == null && result.value != null) {
        val changed = runCatching {
            JSONObject(result.value!!).optBoolean("changed", false)
        }.getOrDefault(false)
        if (changed) {
            NativeCollaborationTransportRegistry.notifyOutboundAvailable(
                editorId,
                CollaborationWakeReason.MODULE_MUTATION,
            )
        }
    }
    return result.toJSMap()
}

class NativeEditorModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("NativeEditor")
        // Benchmark controls remain outside measurement/drawing paths.
        Function("preparedProseBenchmarkBegin") {
            PreparedProseInstrumentation.beginBenchmark()
        }
        Function("preparedProseBenchmarkBeginPhase") { phase: String ->
            val normalizedPhase = phase.replace("imagesDisabled", "images_disabled")
            PreparedProseInstrumentation.TraversalPhase.values()
                .firstOrNull { it.name.equals(normalizedPhase, ignoreCase = true) }
                ?.let(PreparedProseInstrumentation::beginPhase)
        }
        Function("preparedProseBenchmarkEndPhase") {
            PreparedProseInstrumentation.endPhase()
        }
        Function("preparedProseBenchmarkReset") {
            PreparedProseLayoutRegistry.shared.didReceiveMemoryWarning()
            PreparedProseInstrumentation.invalidated(PreparedProseInstrumentation.InvalidationReason.CACHE_RESET)
        }
        Function("preparedProseBenchmarkExport") {
            PreparedProseInstrumentation.exportJson()
        }
        Events("onCollaborationTransportEvent")

        OnCreate {
            NativeCollaborationTransportRegistry.setEventEmitter { payload ->
                sendEvent("onCollaborationTransportEvent", payload)
            }
        }
        OnActivityEntersBackground {
            NativeCollaborationTransportRegistry.enterBackground()
        }
        OnActivityEntersForeground {
            NativeCollaborationTransportRegistry.enterForeground()
        }
        OnActivityDestroys {
            NativeCollaborationTransportRegistry.destroyAll()
        }
        OnDestroy {
            NativeCollaborationTransportRegistry.destroyAll()
        }


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
            mutationResult(editorId, editorV2ReplaceDocument(editorId, requestJson))
        }
        Function("editorV2ApplyInput") { editorId: String, requestJson: String ->
            mutationResult(editorId, editorV2ApplyInput(editorId, requestJson))
        }
        Function("editorV2ApplyCommand") { editorId: String, requestJson: String ->
            mutationResult(editorId, editorV2ApplyCommand(editorId, requestJson))
        }
        Function("editorV2ApplyLocalApi") { editorId: String, requestJson: String ->
            mutationResult(editorId, editorV2ApplyLocalApi(editorId, requestJson))
        }
        Function("editorV2SetSelection") { editorId: String, requestJson: String ->
            editorV2SetSelection(editorId, requestJson).toJSMap()
        }
        Function("editorV2Undo") { editorId: String, requestJson: String ->
            mutationResult(editorId, editorV2Undo(editorId, requestJson))
        }
        Function("editorV2Redo") { editorId: String, requestJson: String ->
            mutationResult(editorId, editorV2Redo(editorId, requestJson))
        }
        Function("editorV2RenderUpdate") { editorId: String, mirrorScalarAnchor: Double?, mirrorScalarHead: Double? ->
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


        Function("editorV2CollaborationConfigureTransport") {
            editorId: String,
            configJsonOrNull: String? ->
            val error = NativeCollaborationTransportRegistry.configure(
                editorId,
                configJsonOrNull,
            )
            mapOf(
                "value" to if (error == null) true else null,
                "error" to error?.toJSMap(),
            )
        }
        Function("editorV2CollaborationResolveProtocolAdapter") {
            editorId: String,
            attemptId: String,
            eventId: String,
            responseJson: String ->
            val error = NativeCollaborationTransportRegistry.resolveProtocolAdapter(
                editorId,
                attemptId,
                eventId,
                responseJson,
            )
            mapOf(
                "value" to if (error == null) true else null,
                "error" to error?.toJSMap(),
            )
        }
        Function("editorV2CollaborationSetAwareness") { editorId: String, awarenessJson: String ->
            val result = editorV2CollaborationSetAwareness(editorId, awarenessJson)
            if (result.value == true && result.error == null) {
                NativeCollaborationTransportRegistry.notifyOutboundAvailable(
                    editorId,
                    CollaborationWakeReason.AWARENESS,
                )
            }
            result.toJSMap()
        }
        Function("editorV2CollaborationPeers") { editorId: String ->
            editorV2CollaborationPeers(editorId).toJSMap()
        }

        Function("editorV2SnapshotExport") { editorId: String ->
            editorV2SnapshotExport(editorId).toJSMap()
        }
        Function("editorV2SnapshotRestore") { editorId: String, metadataJson: String, encodedState: ByteArray ->
            mutationResult(
                editorId,
                editorV2SnapshotRestore(editorId, metadataJson, encodedState),
            )
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
                "onAddonEvent",
                "onExternalTextCompositionEnd"
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
            Prop("androidInputOptionsJson") { view: NativeEditorExpoView, optionsJson: String? ->
                view.setAndroidInputOptionsJson(optionsJson)
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
            Prop("editorUpdateRevision") { view: NativeEditorExpoView, editorUpdateRevision: Double? ->
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
            Prop("editorResetUpdateRevision") { view: NativeEditorExpoView, editorResetUpdateRevision: Double? ->
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

            AsyncFunction("beginExternalTextComposition") { view: NativeEditorExpoView, sessionId: String ->
                view.beginExternalTextComposition(sessionId)
            }
            AsyncFunction("updateExternalTextComposition") { view: NativeEditorExpoView, sessionId: String, text: String ->
                view.updateExternalTextComposition(sessionId, text)
            }
            AsyncFunction("commitExternalTextComposition") { view: NativeEditorExpoView, sessionId: String, text: String ->
                view.commitExternalTextComposition(sessionId, text)
            }
            AsyncFunction("cancelExternalTextComposition") { view: NativeEditorExpoView, sessionId: String, cause: String ->
                view.cancelExternalTextComposition(sessionId, cause)
            }

        }

    }
}
