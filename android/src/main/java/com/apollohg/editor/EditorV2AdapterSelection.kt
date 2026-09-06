package com.apollohg.editor

import org.json.JSONObject
import org.json.JSONArray


internal sealed interface SelectionSyncOutcome {
    object Ok : SelectionSyncOutcome
    data class Refreshed(val updateJson: String) : SelectionSyncOutcome
    object Failed : SelectionSyncOutcome
}

internal fun EditorV2Adapter.clampScalar(scalar: Int): Int {
    val extent = cachedScalarLength ?: return scalar
    return scalar.coerceIn(0, extent)
}

internal fun EditorV2Adapter.invalidateCachedAtomicState(selection: IntArray?) {
    cachedAuthoritativeScalarSelection = selection?.copyOf()
    cachedScalarLength = null
    cachedActiveState = null
    cachedHistoryState = null
    cachedViewUpdateJson = null
    cachedAtomicRenderJson = null
    cachedAtomicRenderDocumentRevision = null
}

internal fun EditorV2Adapter.ensureSelection(anchor: Int, head: Int): SelectionSyncOutcome {
    val clampedAnchor = clampScalar(anchor)
    val clampedHead = clampScalar(head)
    val last = lastSyncedScalarSelection
    if (last != null && last[0] == clampedAnchor && last[1] == clampedHead) {
        return SelectionSyncOutcome.Ok
    }
    // Affinity policy mirrors the engine's own cursor resolution: a
    // collapsed caret prefers After with a deterministic Before fallback
    // at text-boundary positions; a range uses Before. The fallback
    // changes only the stickiness of the SAME position.
    val collapsed = clampedAnchor == clampedHead
    var result = callWithEnvelope(
        selectionEnvelope(clampedAnchor, clampedHead, if (collapsed) "after" else "before"),
    ) { requestJson ->
        backend.setSelection(editorId, requestJson)
    }
    if (collapsed &&
        result is EditorV2CallResult.Err &&
        result.error.code == "POSITION_INVALID"
    ) {
        result = callWithEnvelope(selectionEnvelope(clampedAnchor, clampedHead, "before")) { requestJson ->
            backend.setSelection(editorId, requestJson)
        }
    }
    return when (result) {
        is EditorV2CallResult.Ok -> {
            parseMutationOutcome(result.value)?.let { outcome ->
                if (outcome is MutationOutcome.Transaction) {
                    baseDocumentRevision = outcome.revision
                }
            }
            val synchronizedSelection = intArrayOf(clampedAnchor, clampedHead)
            lastSyncedScalarSelection = synchronizedSelection
            cachedAuthoritativeScalarSelection = synchronizedSelection.copyOf()
            // Selection changes can affect active state and state revision,
            // but retain the last atomic history result until a document
            // mutation supplies its next locked snapshot.
            cachedActiveState = null
            cachedViewUpdateJson = null
            cachedAtomicRenderJson = null
            cachedAtomicRenderDocumentRevision = null
            SelectionSyncOutcome.Ok
        }
        is EditorV2CallResult.Err -> {
            if (result.error.code == "REVISION_MISMATCH") {
                val update = refreshInternal(null, stripViewSelection = false)
                if (update != null) {
                    SelectionSyncOutcome.Refreshed(update)
                } else {
                    SelectionSyncOutcome.Failed
                }
            } else {
                emit(result.error)
                SelectionSyncOutcome.Failed
            }
        }
    }
}

internal fun EditorV2Adapter.textDocumentSelection(updateJson: String): IntArray? {
    return try {
        val selection = JSONObject(updateJson).getJSONObject("selection")
        if (selection.optString("type") != "text") return null
        intArrayOf(
            scalarField(selection, "anchor") ?: return null,
            scalarField(selection, "head") ?: return null,
        )
    } catch (error: Exception) {
        null
    }
}

internal fun EditorV2Adapter.resolveSelectionMapping(anchor: Int, head: Int): IntArray? {
    // Engine-authoritative scalar→doc selection mapping for the delegate
    // callback's doc positions (v2 accessor).
    val resolved = when (val result = backend.resolveScalarSelection(editorId, anchor, head)) {
        is EditorV2CallResult.Err -> {
            debugNotes.add("resolveScalarSelection ${result.error.domain}/${result.error.code}")
            return null
        }
        is EditorV2CallResult.Ok -> result.value
    }
    return try {
        val selection = JSONObject(resolved)
        intArrayOf(
            scalarField(selection, "anchor") ?: return null,
            scalarField(selection, "head") ?: return null,
        )
    } catch (error: Exception) {
        null
    }
}

internal fun EditorV2Adapter.publishCachedCollaborationSelection() {
    if (!roomBound) return
    val selection = cachedAuthoritativeScalarSelection ?: return
    val mapping = resolveSelectionMapping(selection[0], selection[1]) ?: return
    publishCollaborationSelection(mapping[0], mapping[1])
}

internal fun EditorV2Adapter.publishCollaborationSelection(docAnchor: Int, docHead: Int) {
    if (!roomBound) return
    val selectionJson = JSONObject()
        .put("type", "text")
        .put("anchor", docAnchor)
        .put("head", docHead)
        .toString()
    when (
        val result = backend.collaborationSetAwarenessSelection(
            editorId,
            selectionJson,
        )
    ) {
        is EditorV2CallResult.Err -> emit(result.error)
        is EditorV2CallResult.Ok -> {
            val outboundChanged = try {
                val value = JSONObject(result.value)
                if (value.length() != 1 ||
                    !value.has("outboundChanged") ||
                    value.opt("outboundChanged") !is Boolean
                ) {
                    null
                } else {
                    value.getBoolean("outboundChanged")
                }
            } catch (error: Exception) {
                null
            }
            if (outboundChanged == null) {
                emit(EditorV2Adapter.contractError("awareness selection result violates the frozen shape"))
            } else if (outboundChanged) {
                collaborationWake(editorId, CollaborationWakeReason.AWARENESS)
            }
        }
    }
}

internal fun EditorV2Adapter.mapPosition(result: EditorV2CallResult<String>, key: String): Int? =
    when (result) {
        is EditorV2CallResult.Err -> {
            emit(result.error)
            null
        }
        is EditorV2CallResult.Ok -> try {
            scalarField(JSONObject(result.value), key)
        } catch (error: Exception) { null }
    }
