package com.apollohg.editor

import org.json.JSONObject
import org.json.JSONArray


internal fun ulongField(object_: JSONObject, key: String): ULong? =
    canonicalV2U64(object_.opt(key) as? String)?.toULong()

internal fun scalarField(object_: JSONObject, key: String): Int? =
    exactV2ScalarInt(object_.opt(key) as? Number)

internal fun exactBool(value: Any?): Boolean? = value as? Boolean

private fun exactKeys(object_: JSONObject, keys: Set<String>): Boolean {
    val actual = mutableSetOf<String>()
    val iterator = object_.keys()
    while (iterator.hasNext()) actual += iterator.next()
    return actual == keys
}

private fun onlyKeys(object_: JSONObject, keys: Set<String>): Boolean {
    val iterator = object_.keys()
    while (iterator.hasNext()) if (iterator.next() !in keys) return false
    return true
}

private fun validJsonValue(value: Any?): Boolean = when (value) {
    null, JSONObject.NULL, is String, is Boolean -> true
    is Number -> value.toDouble().isFinite()
    is JSONArray -> (0 until value.length()).all { validJsonValue(value.opt(it)) }
    is JSONObject -> {
        val iterator = value.keys()
        var valid = true
        while (iterator.hasNext()) {
            if (!validJsonValue(value.opt(iterator.next()))) valid = false
        }
        valid
    }
    else -> false
}

private fun validRenderMark(value: Any?): Boolean = when (value) {
    is String -> true
    is JSONObject -> value.opt("type") is String && validJsonValue(value)
    else -> false
}

private fun validListContext(value: Any?): Boolean {
    val object_ = value as? JSONObject ?: return false
    if (!onlyKeys(object_, setOf("ordered", "index", "total", "start", "isFirst", "isLast", "kind", "checked"))) return false
    if (exactBool(object_.opt("ordered")) == null || scalarField(object_, "index") == null ||
        scalarField(object_, "total") == null || scalarField(object_, "start") == null ||
        exactBool(object_.opt("isFirst")) == null || exactBool(object_.opt("isLast")) == null
    ) return false
    val kind = object_.opt("kind")
    if (kind != null && kind !== JSONObject.NULL && kind !is String) return false
    val checked = object_.opt("checked")
    return checked == null || checked === JSONObject.NULL || exactBool(checked) != null
}

private fun validMentionThemeSection(
    value: Any?,
    stringKeys: Set<String>,
    extraKeys: Set<String>
): Boolean {
    val object_ = value as? JSONObject ?: return false
    val numberKeys = setOf("borderWidth", "borderRadius")
    if (!onlyKeys(object_, stringKeys + numberKeys + extraKeys)) return false
    if (stringKeys.any { object_.has(it) && object_.opt(it) !is String }) return false
    if (numberKeys.any { object_.has(it) && (object_.opt(it) !is Number || !(object_.opt(it) as Number).toDouble().isFinite()) }) return false
    val weight = object_.opt("fontWeight")
    return weight == null || weight in setOf("normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900")
}

private fun validMentionTheme(value: Any?): Boolean {
    val object_ = value as? JSONObject ?: return false
    if (!onlyKeys(object_, setOf("node", "suggestions"))) return false

    if (object_.has("node") && !validMentionThemeSection(
            object_.opt("node"),
            setOf("textColor", "backgroundColor", "borderColor"),
            setOf("fontWeight")
        )
    ) {
        return false
    }
    if (!object_.has("suggestions")) return true
    val suggestions = object_.opt("suggestions")
    if (!validMentionThemeSection(
            suggestions,
            setOf("backgroundColor", "borderColor", "shadowColor"),
            setOf("option")
        )
    ) {
        return false
    }
    val option = (suggestions as? JSONObject)?.opt("option") ?: return true
    return validMentionThemeSection(
        option,
        setOf("textColor", "secondaryTextColor", "backgroundColor", "borderColor", "highlightedBackgroundColor", "highlightedTextColor"),
        setOf("fontWeight")
    )
}

private fun validRenderElement(value: Any?): Boolean {
    val object_ = value as? JSONObject ?: return false
    return when (object_.opt("type") as? String) {
        "textRun" -> exactKeys(object_, setOf("type", "text", "marks")) && object_.opt("text") is String &&
            (object_.opt("marks") as? JSONArray)?.let { marks -> (0 until marks.length()).all { validRenderMark(marks.opt(it)) } } == true
        "blockStart" -> onlyKeys(object_, setOf("type", "nodeType", "depth", "listContext")) && object_.opt("nodeType") is String &&
            scalarField(object_, "depth") != null && (!object_.has("listContext") || validListContext(object_.opt("listContext")))
        "blockEnd" -> exactKeys(object_, setOf("type"))
        "voidInline" -> onlyKeys(object_, setOf("type", "nodeType", "docPos", "attrs")) && object_.opt("nodeType") is String &&
            scalarField(object_, "docPos") != null && (!object_.has("attrs") || object_.opt("attrs") is JSONObject)
        "voidBlock" -> onlyKeys(object_, setOf("type", "nodeType", "docPos", "attrs", "atomId")) && object_.opt("nodeType") is String &&
            scalarField(object_, "docPos") != null && (!object_.has("attrs") || object_.opt("attrs") is JSONObject) &&
            (!object_.has("atomId") || object_.opt("atomId") is String)
        "opaqueInlineAtom" -> onlyKeys(object_, setOf("type", "nodeType", "label", "docPos", "attrs", "mentionTheme")) &&
            object_.opt("nodeType") is String && object_.opt("label") is String && scalarField(object_, "docPos") != null &&
            (!object_.has("attrs") || object_.opt("attrs") is JSONObject) &&
            (!object_.has("mentionTheme") || validMentionTheme(object_.opt("mentionTheme")))
        "opaqueBlockAtom" -> onlyKeys(object_, setOf("type", "nodeType", "label", "docPos", "attrs")) &&
            object_.opt("nodeType") is String && object_.opt("label") is String && scalarField(object_, "docPos") != null &&
            (!object_.has("attrs") || object_.opt("attrs") is JSONObject)
        else -> false
    }
}

private fun validRenderBlocks(value: Any?): Boolean {
    val blocks = value as? JSONArray ?: return false
    return (0 until blocks.length()).all { blockIndex ->
        val block = blocks.opt(blockIndex) as? JSONArray ?: return@all false
        (0 until block.length()).all { validRenderElement(block.opt(it)) }
    }
}

private fun validRenderPatch(value: Any?): Boolean {
    if (value === JSONObject.NULL) return true
    val patch = value as? JSONObject ?: return false
    return exactKeys(
        patch,
        setOf("baseDocumentVersion", "startIndex", "deleteCount", "renderBlocks"),
    ) &&
        canonicalV2U64(patch.opt("baseDocumentVersion") as? String) != null &&
        scalarField(patch, "startIndex") != null && scalarField(patch, "deleteCount") != null &&
        validRenderBlocks(patch.opt("renderBlocks"))
}

private fun validBooleanRecord(value: Any?): Boolean {
    val object_ = value as? JSONObject ?: return false
    val iterator = object_.keys()
    while (iterator.hasNext()) if (exactBool(object_.opt(iterator.next())) == null) return false
    return true
}

private fun validStringArray(value: Any?): Boolean {
    val array = value as? JSONArray ?: return false
    return (0 until array.length()).all { array.opt(it) is String }
}

private fun validActiveState(value: Any?): Boolean {
    val object_ = value as? JSONObject ?: return false
    if (!exactKeys(object_, setOf("marks", "markAttrs", "nodes", "commands", "allowedMarks", "insertableNodes"))) return false
    val attrs = object_.opt("markAttrs") as? JSONObject ?: return false
    val attrsIterator = attrs.keys()
    while (attrsIterator.hasNext()) if (attrs.opt(attrsIterator.next()) !is JSONObject) return false
    return validBooleanRecord(object_.opt("marks")) && validBooleanRecord(object_.opt("nodes")) &&
        validBooleanRecord(object_.opt("commands")) && validStringArray(object_.opt("allowedMarks")) &&
        validStringArray(object_.opt("insertableNodes"))
}

internal fun scalarSelection(value: Any?): IntArray? {
    val selection = value as? JSONObject ?: return null
    if (selection.opt("type") != "text" || !exactKeys(selection, setOf("type", "anchor", "head", "anchorScalar", "headScalar"))) return null
    if (scalarField(selection, "anchor") == null || scalarField(selection, "head") == null) return null
    return intArrayOf(scalarField(selection, "anchorScalar") ?: return null, scalarField(selection, "headScalar") ?: return null)
}

private fun validSelection(value: Any?): Boolean {
    val selection = value as? JSONObject ?: return false
    return when (selection.opt("type") as? String) {
        "text" -> scalarSelection(selection) != null
        "node" -> exactKeys(selection, setOf("type", "pos", "posScalar")) && scalarField(selection, "pos") != null && scalarField(selection, "posScalar") != null
        "all" -> exactKeys(selection, setOf("type"))
        else -> false
    }
}

internal data class AtomicRenderSnapshot(
    /** Original validated wire payload for controlled-prop delivery. */
    val atomicRenderJson: String,
    val viewUpdateJson: String,
    val documentRevision: ULong,
    val stateRevision: ULong,
    val scalarLength: Int,
    val scalarSelection: IntArray?,
    val activeState: JSONObject,
    val historyState: JSONObject,
    val positionEpoch: String?,
)

internal data class PinnedAtomicRenderSnapshot(
    val snapshot: AtomicRenderSnapshot,
    val positionEpoch: String?,
)

internal fun parseAtomicRenderSnapshot(json: String): AtomicRenderSnapshot? {
    return try {
        val object_ = JSONObject(json)
        val requiredKeys = setOf("renderBlocks", "renderPatch", "selection", "activeState", "historyState", "documentVersion", "stateRevision", "scalarLength", "documentIsEmpty")
        val renderBlocks = object_.opt("renderBlocks")
        val renderPatch = object_.opt("renderPatch")
        val validRenderPayload =
            (validRenderBlocks(renderBlocks) && renderPatch === JSONObject.NULL) ||
                (renderBlocks === JSONObject.NULL && renderPatch is JSONObject && validRenderPatch(renderPatch))
        if (!onlyKeys(object_, requiredKeys + "positionEpoch") || requiredKeys.any { !object_.has(it) } ||
            !validRenderPayload ||
            !validSelection(object_.opt("selection")) || !validActiveState(object_.opt("activeState")) ||
            exactBool(object_.opt("documentIsEmpty")) == null
        ) return null
        val history = object_.opt("historyState") as? JSONObject ?: return null
        if (!exactKeys(history, setOf("canUndo", "canRedo")) || exactBool(history.opt("canUndo")) == null || exactBool(history.opt("canRedo")) == null) return null
        val revision = ulongField(object_, "documentVersion") ?: return null
        val state = ulongField(object_, "stateRevision") ?: return null
        val scalarLength = scalarField(object_, "scalarLength") ?: return null
        val scalarSelection = scalarSelection(object_.opt("selection"))
        val positionEpoch = if (object_.has("positionEpoch")) {
            canonicalV2U64(object_.opt("positionEpoch") as? String) ?: return null
        } else {
            null
        }
        object_.remove("positionEpoch")
        val atomicRenderJson = object_.toString()
        object_.remove("scalarLength")
        AtomicRenderSnapshot(
            atomicRenderJson,
            object_.toString(),
            revision,
            state,
            scalarLength,
            scalarSelection,
            JSONObject(object_.getJSONObject("activeState").toString()),
            JSONObject(history.toString()),
            positionEpoch,
        )
    } catch (_: Exception) {
        null
    }
}
