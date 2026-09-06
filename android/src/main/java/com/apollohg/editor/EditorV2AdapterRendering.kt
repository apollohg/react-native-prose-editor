package com.apollohg.editor

import org.json.JSONObject
import org.json.JSONArray


internal fun EditorV2Adapter.adopt(
    snapshot: AtomicRenderSnapshot,
    stripViewSelection: Boolean,
    engineOwnedSelection: Boolean,
    resolvedPositionEpoch: String? = snapshot.positionEpoch,
): String {
    val update = JSONObject(snapshot.viewUpdateJson)
    if (stripViewSelection) update.remove("selection")
    val updateJson = update.toString()
    baseDocumentRevision = snapshot.documentRevision
    stateRevision = snapshot.stateRevision
    cachedScalarLength = snapshot.scalarLength
    cachedAuthoritativeScalarSelection = snapshot.scalarSelection?.copyOf()
    lastSyncedScalarSelection =
        if (engineOwnedSelection) snapshot.scalarSelection?.copyOf() else null
    cachedActiveState = snapshot.activeState
    cachedHistoryState = snapshot.historyState
    cachedViewUpdateJson = updateJson
    cachedAtomicRenderJson = snapshot.atomicRenderJson
    cachedAtomicRenderDocumentRevision = snapshot.documentRevision
    if (resolvedPositionEpoch != null) positionEpoch = resolvedPositionEpoch
    return updateJson
}

internal fun EditorV2Adapter.fetchDocumentJson(): String? {
    return when (val result = backend.getDocumentJson(editorId)) {
        is EditorV2CallResult.Err -> {
            emit(result.error)
            null
        }
        is EditorV2CallResult.Ok -> result.value
    }
}


internal fun EditorV2Adapter.refreshInternal(
    mirrorSelection: IntArray?,
    stripViewSelection: Boolean = mirrorSelection == null,
    controlledPropSnapshot: Boolean = false,
): String? {
    if (destroyed) {
        emit(EditorV2Adapter.destroyedError())
        return null
    }
    val ownerId = nativeOwnerId
    val renderResult = if (ownerId == null) {
        backend.renderUpdate(editorId, mirrorSelection?.get(0), mirrorSelection?.get(1))
    } else {
        backend.renderNative(editorId, ownerId, mirrorSelection?.get(0), mirrorSelection?.get(1))
    }
    val derived = when (val result = renderResult) {
        is EditorV2CallResult.Err -> {
            // A render update that fails or violates the frozen shape is a
            // boundary failure like any other. Returning null without
            // reporting it leaves every caller — the paired view and the
            // stateless render probe alike — holding a bare null with no
            // cause to surface, so the engine's own error is what travels.
            emit(result.error)
            return null
        }
        is EditorV2CallResult.Ok -> result.value
    }
    renderUpdateCallCountForTesting += 1
    val snapshot = parseAtomicRenderSnapshot(derived)
    return if (snapshot == null) {
        emit(EditorV2Adapter.contractError("v2 render update violates the frozen shape"))
        null
    } else {
        // Preserve an IME-owned caret only after authoritative active and
        // history state has been adopted from the post-operation snapshot.
        val viewUpdateJson = adopt(
            snapshot,
            stripViewSelection = stripViewSelection,
            engineOwnedSelection = mirrorSelection == null,
        )
        if (controlledPropSnapshot) snapshot.atomicRenderJson else viewUpdateJson
    }
}

internal fun EditorV2Adapter.pinPositionEpochCandidate(documentRevision: ULong): String? {
    val ownerId = nativeOwnerId ?: return positionEpoch
    return when (val result = backend.pinPositionEpoch(editorId, ownerId, documentRevision.toString())) {
        is EditorV2CallResult.Err -> {
            emit(result.error)
            null
        }
        is EditorV2CallResult.Ok -> try {
            canonicalV2U64(JSONObject(result.value).opt("positionEpoch") as? String)
                ?: run {
                    emit(EditorV2Adapter.contractError("v2 position epoch result violates the frozen shape"))
                    null
                }
        } catch (_: Exception) {
            emit(EditorV2Adapter.contractError("v2 position epoch result violates the frozen shape"))
            null
        }
    }
}

internal fun EditorV2Adapter.pinCurrentPositionEpoch(documentRevision: ULong): Boolean {
    if (nativeOwnerId == null) return true
    val candidate = pinPositionEpochCandidate(documentRevision) ?: return false
    positionEpoch = candidate
    return true
}
