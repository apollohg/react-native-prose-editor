package com.apollohg.editor

import org.json.JSONObject
import org.json.JSONArray


internal sealed interface MutationOutcome {
    data class Transaction(val changed: Boolean, val revision: ULong) : MutationOutcome
    object NotApplicable : MutationOutcome
    data class Replacement(val changed: Boolean, val revision: ULong) : MutationOutcome
}

internal fun EditorV2Adapter.parseMutationOutcome(json: String): MutationOutcome? {
    return try {
        val outcome = JSONObject(json)
        when (outcome.getString("type")) {
            "transaction" -> MutationOutcome.Transaction(
                changed = outcome.getBoolean("changed"),
                revision = ulongField(outcome, "documentRevision") ?: return null,
            )
            "notApplicable" -> MutationOutcome.NotApplicable
            "replacement" -> MutationOutcome.Replacement(
                changed = outcome.getBoolean("changed"),
                revision = ulongField(outcome, "documentRevision") ?: return null,
            )
            else -> null
        }
    } catch (error: Exception) {
        null
    }
}

internal fun EditorV2Adapter.handleMutationError(error: EditorV2Error): String? {
    if (error.code == "REVISION_MISMATCH") {
        val update = refreshInternal(null, stripViewSelection = false)
        debugNotes.add("mismatch-refresh ${if (update == null) "nil" else "ok"}")
        return update
    }
    emit(error)
    return null
}

internal fun EditorV2Adapter.performNativeIntent(
    intent: JSONObject,
    reportPositionEpochInvalid: Boolean = false,
): EditorV2NativeIntentResult {
    if (destroyed) {
        emit(EditorV2Adapter.destroyedError())
        return EditorV2NativeIntentResult.Rejected
    }
    val ownerId = nativeOwnerId ?: return EditorV2NativeIntentResult.Rejected
    if (positionEpoch == null && refreshInternal(null, stripViewSelection = false) == null) {
        return EditorV2NativeIntentResult.Rejected
    }
    val epoch = positionEpoch ?: return EditorV2NativeIntentResult.Rejected
    val result = callWithEnvelope(
        JSONObject()
            .put("ownerId", ownerId)
            .put("positionEpoch", epoch)
            .put("intent", intent),
        includeBaseRevision = false,
    ) { requestJson -> backend.applyNativeIntent(editorId, requestJson) }
    return when (result) {
        is EditorV2CallResult.Err -> {
            if (result.error.code == "POSITION_EPOCH_INVALID") {
                debugNotes.add("position-epoch-refresh")
                val recovery = refreshInternal(null, stripViewSelection = false)
                if (reportPositionEpochInvalid) emit(result.error)
                if (recovery != null) {
                    EditorV2NativeIntentResult.Recovered(recovery)
                } else {
                    EditorV2NativeIntentResult.Rejected
                }
            } else {
                emit(result.error)
                EditorV2NativeIntentResult.Rejected
            }
        }
        is EditorV2CallResult.Ok -> {
            val outcome = parseMutationOutcome(result.value)
            if (outcome == null) {
                emit(EditorV2Adapter.contractError("v2 native intent outcome violates the frozen shape"))
                return EditorV2NativeIntentResult.Rejected
            }
            val resultObject = try {
                JSONObject(result.value)
            } catch (_: Exception) {
                emit(EditorV2Adapter.contractError("v2 native intent outcome violates the frozen shape"))
                return EditorV2NativeIntentResult.Rejected
            }
            val documentChanged = when (outcome) {
                MutationOutcome.NotApplicable -> false
                is MutationOutcome.Transaction,
                is MutationOutcome.Replacement ->
                    exactBool(resultObject.opt("documentChanged")) ?: run {
                        emit(EditorV2Adapter.contractError("v2 native intent outcome violates the frozen shape"))
                        return EditorV2NativeIntentResult.Rejected
                    }
            }
            val changed = when (outcome) {
                is MutationOutcome.Transaction -> outcome.changed
                is MutationOutcome.NotApplicable -> false
                is MutationOutcome.Replacement -> outcome.changed
            }
            invalidateCachedAtomicState(null)
            val update = refreshInternal(null, stripViewSelection = false)
            if (update == null) {
                val recovery = recoverNativeRender()
                if (recovery != null) {
                    if (changed) {
                        publishCachedCollaborationSelection()
                        notifyCollaborationMutation()
                    }
                    return EditorV2NativeIntentResult.Applied(
                        EditorV2NativeMutationRender(recovery, changed, documentChanged),
                    )
                }
                return EditorV2NativeIntentResult.Rejected
            }
            if (changed) {
                publishCachedCollaborationSelection()
                notifyCollaborationMutation()
            }
            EditorV2NativeIntentResult.Applied(
                EditorV2NativeMutationRender(update, changed, documentChanged),
            )
        }
    }
}

internal fun EditorV2NativeIntentResult.updateJsonOrNull(): String? = when (this) {
    is EditorV2NativeIntentResult.Applied -> render.updateJson
    is EditorV2NativeIntentResult.Recovered -> updateJson
    EditorV2NativeIntentResult.Rejected -> null
}

internal fun EditorV2Adapter.nativeIntent(type: String, anchor: Int, head: Int): JSONObject =
    JSONObject()
        .put("type", type)
        .put("anchor", clampScalar(anchor))
        .put("head", clampScalar(head))

internal fun EditorV2Adapter.performMutation(
    preSelection: IntArray? = null,
    postSelectionMirror: IntArray? = null,
    includeSelectionInUpdate: Boolean = false,
    adoptEngineSelection: Boolean = false,
    call: () -> EditorV2CallResult<String>,
): String? {
    if (destroyed) {
        emit(EditorV2Adapter.destroyedError())
        return null
    }
    val pre = preSelection
    val post = postSelectionMirror
    if (pre != null) {
        when (val sync = ensureSelection(pre[0], pre[1])) {
            is SelectionSyncOutcome.Ok -> Unit
            is SelectionSyncOutcome.Refreshed -> return sync.updateJson
            is SelectionSyncOutcome.Failed -> return null
        }
    }
    return when (val result = call()) {
        is EditorV2CallResult.Err -> handleMutationError(result.error)
        is EditorV2CallResult.Ok -> {
                val outcome = parseMutationOutcome(result.value)
                if (outcome == null) {
                    emit(EditorV2Adapter.contractError("v2 mutation outcome violates the frozen shape"))
                    return null
                }
                val changed = when (outcome) {
                    is MutationOutcome.Transaction -> {
                        baseDocumentRevision = outcome.revision
                        invalidateCachedAtomicState(post ?: pre)
                        outcome.changed
                    }
                    is MutationOutcome.NotApplicable -> {
                        val fallbackMirror = if (adoptEngineSelection) null else post ?: pre
                        return refreshInternal(
                            fallbackMirror,
                            stripViewSelection = !adoptEngineSelection && fallbackMirror == null,
                        )
                    }
                    is MutationOutcome.Replacement -> {
                        baseDocumentRevision = outcome.revision
                        // Whole-root replacement resets the engine-side selection.
                        lastSyncedScalarSelection = null
                        invalidateCachedAtomicState(null)
                        outcome.changed
                    }
                }
                val mirror = if (adoptEngineSelection) {
                    null
                } else if (includeSelectionInUpdate) {
                    post ?: pre
                } else {
                    null
                }
                val update = refreshInternal(
                    mirror,
                    stripViewSelection = !adoptEngineSelection && mirror == null,
                ) ?: return null
                if (outcome is MutationOutcome.Transaction && mirror != null && post != null) {
                    lastSyncedScalarSelection = post
                }
                if (changed) {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                update
        }
    }
}

/**
 * Split commands need to distinguish a locally committed transaction from
 * a refresh recovered after a stale revision or a no-op outcome. Both
 * paths return a render, but only the former warrants IME-boundary work.
 */
internal fun EditorV2Adapter.performSplitMutation(
    preSelection: IntArray,
    postSelectionMirror: IntArray,
    call: () -> EditorV2CallResult<String>,
): EditorV2SplitRender? {
    if (destroyed) {
        emit(EditorV2Adapter.destroyedError())
        return null
    }
    val mirror = postSelectionMirror
    when (val sync = ensureSelection(preSelection[0], preSelection[1])) {
        is SelectionSyncOutcome.Ok -> Unit
        is SelectionSyncOutcome.Refreshed ->
            return EditorV2SplitRender(sync.updateJson, committed = false)
        is SelectionSyncOutcome.Failed -> return null
    }
    return when (val result = call()) {
        is EditorV2CallResult.Err -> handleMutationError(result.error)
            ?.let { EditorV2SplitRender(it, committed = false) }
        is EditorV2CallResult.Ok -> {
                val outcome = parseMutationOutcome(result.value)
                if (outcome == null) {
                    emit(EditorV2Adapter.contractError("v2 mutation outcome violates the frozen shape"))
                    return null
                }
                return when (outcome) {
                    is MutationOutcome.NotApplicable ->
                        refreshInternal(mirror)
                            ?.let { EditorV2SplitRender(it, committed = false) }
                    is MutationOutcome.Transaction -> {
                        baseDocumentRevision = outcome.revision
                        invalidateCachedAtomicState(mirror)
                        val update = refreshInternal(
                            mirror,
                            stripViewSelection = false,
                        ) ?: return null
                        lastSyncedScalarSelection = mirror
                        if (outcome.changed) {
                            publishCachedCollaborationSelection()
                            notifyCollaborationMutation()
                        }
                        EditorV2SplitRender(update, committed = outcome.changed)
                    }
                    is MutationOutcome.Replacement -> {
                        baseDocumentRevision = outcome.revision
                        lastSyncedScalarSelection = null
                        invalidateCachedAtomicState(null)
                        val update = refreshInternal(mirror) ?: return null
                        if (outcome.changed) {
                            publishCachedCollaborationSelection()
                            notifyCollaborationMutation()
                        }
                        EditorV2SplitRender(update, committed = outcome.changed)
                    }
                }
        }
    }
}

internal fun EditorV2Adapter.performHistoryMutation(call: (String) -> EditorV2CallResult<String>): String? {
    if (destroyed) {
        emit(EditorV2Adapter.destroyedError())
        return null
    }
    return when (val result = callWithEnvelope(JSONObject(), includeBaseRevision = false, call = call)) {
        is EditorV2CallResult.Err -> {
            emit(result.error)
            null
        }
        is EditorV2CallResult.Ok -> {
            val changed = try {
                JSONObject(result.value).getBoolean("changed")
            } catch (error: Exception) {
                emit(EditorV2Adapter.contractError("v2 history outcome violates the frozen shape"))
                return null
            }
            if (changed) {
                invalidateCachedAtomicState(null)
                lastSyncedScalarSelection = null
            }
            val update = refreshInternal(null) ?: return null
            if (changed) {
                publishCachedCollaborationSelection()
                notifyCollaborationMutation()
            }
            update
        }
    }
}


internal fun EditorV2Adapter.notifyCollaborationMutation() {
    if (!roomBound) return
    collaborationWake(
        editorId,
        CollaborationWakeReason.LOCAL_MUTATION,
    )
}


internal fun EditorV2Adapter.refreshUnchangedNativeOutcome(
    outcome: EditorV2NativeIntentResult,
): EditorV2NativeIntentResult {
    val applied = outcome as? EditorV2NativeIntentResult.Applied ?: return outcome
    if (applied.render.documentChanged) return outcome
    val ownerId = nativeOwnerId ?: return EditorV2NativeIntentResult.Rejected
    val updateJson = try {
        nativeOwnerId = null
        refreshInternal(null, stripViewSelection = false)
    } finally {
        nativeOwnerId = ownerId
    }
    if (updateJson == null || !pinCurrentPositionEpoch(baseDocumentRevision)) {
        return recoverNativeRender()
            ?.let { recovery ->
                EditorV2NativeIntentResult.Applied(
                    applied.render.copy(updateJson = recovery),
                )
            }
            ?: EditorV2NativeIntentResult.Rejected
    }
    return EditorV2NativeIntentResult.Applied(
        applied.render.copy(updateJson = updateJson),
    )
}

internal fun EditorV2Adapter.commandAtSelection(command: JSONObject, anchor: Int, head: Int): String? =
    if (nativeOwnerId != null) {
        performNativeIntent(
            nativeIntent("command", anchor, head).put("command", command)
        ).updateJsonOrNull()
    } else performMutation(
        preSelection = intArrayOf(anchor, head),
        postSelectionMirror = intArrayOf(anchor, head),
    ) {
        callWithEnvelope(JSONObject().put("command", command)) { requestJson ->
            backend.applyCommand(editorId, requestJson)
        }
    }
