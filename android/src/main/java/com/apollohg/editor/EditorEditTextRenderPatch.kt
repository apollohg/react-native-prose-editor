package com.apollohg.editor

import android.text.Annotation
import android.text.Spanned
import android.text.SpannableStringBuilder
import org.json.JSONObject

internal fun EditorEditText.parseRenderPatch(raw: org.json.JSONObject?): ParsedRenderPatch? {
    if (raw == null) return null
    val renderBlocks = raw.optJSONArray("renderBlocks") ?: return null
    val baseDocumentVersion = if (raw.has("baseDocumentVersion")) {
        canonicalV2U64(raw.opt("baseDocumentVersion") as? String) ?: return null
    } else {
        null
    }
    return ParsedRenderPatch(
        baseDocumentVersion = baseDocumentVersion,
        startIndex = raw.optInt("startIndex", -1),
        deleteCount = raw.optInt("deleteCount", -1),
        renderBlocks = renderBlocks
    ).takeIf { it.startIndex >= 0 && it.deleteCount >= 0 }
}

internal fun EditorEditText.patchMatchesCurrentRenderBlocks(
    patch: ParsedRenderPatch,
    updateDocumentVersion: String?,
): Boolean = if (patch.baseDocumentVersion == null) {
    updateDocumentVersion == null && currentRenderBlocksDocumentVersion == null
} else {
    patch.baseDocumentVersion == currentRenderBlocksDocumentVersion
}

internal fun EditorEditText.retainCurrentRenderBlocks(
    blocks: org.json.JSONArray?,
    documentVersion: String?,
    needFullApply: Boolean,
) {
    currentRenderBlocksJson = blocks?.let(::cloneJsonArray)
    currentRenderBlocksDocumentVersion = documentVersion.takeIf { blocks != null }
    currentRenderBlocksNeedFullApply = blocks != null && needFullApply
}

internal fun EditorEditText.invalidateCurrentRenderBlocks() {
    currentRenderBlocksJson = null
    currentRenderBlocksDocumentVersion = null
    currentRenderBlocksNeedFullApply = false
}

internal fun EditorEditText.recoverRenderPatchBaseMismatch(
    notifyListener: Boolean,
    refreshInputConnectionForExternalUpdate: Boolean,
): Boolean {
    invalidateCurrentRenderBlocks()
    if (recoveringRenderPatchBaseMismatch) return false
    val adapter = v2Driver as? EditorV2Adapter ?: return false
    val recovery = adapter.recoverNativeRender() ?: return false
    recoveringRenderPatchBaseMismatch = true
    return try {
        applyUpdateJSON(
            recovery,
            notifyListener = notifyListener,
            refreshInputConnectionForExternalUpdate = refreshInputConnectionForExternalUpdate,
        )
    } finally {
        recoveringRenderPatchBaseMismatch = false
    }
}

internal fun EditorEditText.hasTopLevelChildMetadata(content: Spanned): Boolean =
    content.getSpans(0, content.length, Annotation::class.java).any {
        it.key == RenderBridge.NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION
    }

internal fun EditorEditText.firstCharacterOffsetForTopLevelChildIndex(content: Spanned, index: Int): Int? {
    val targetValue = index.toString()
    return content
        .getSpans(0, content.length, Annotation::class.java)
        .asSequence()
        .filter { it.key == RenderBridge.NATIVE_TOP_LEVEL_CHILD_INDEX_ANNOTATION && it.value == targetValue }
        .mapNotNull { span ->
            val spanStart = content.getSpanStart(span)
            val spanEnd = content.getSpanEnd(span)
            if (spanStart < 0 || spanEnd <= spanStart) {
                null
            } else {
                var candidate = spanStart
                while (candidate < spanEnd && candidate < content.length) {
                    when (content[candidate]) {
                        '\n', '\r' -> {
                            val isHardBreak = content.getSpans(candidate, candidate + 1, Annotation::class.java).any {
                                it.key == "nativeVoidNodeType" && EditorNodeTypes.isHardBreak(it.value) &&
                                    content.getSpanStart(it) <= candidate && content.getSpanEnd(it) > candidate
                            }
                            if (isHardBreak) return@mapNotNull candidate
                            candidate += 1
                        }
                        else -> return@mapNotNull candidate
                    }
                }
                null
            }
        }
        .minOrNull()
}

internal fun EditorEditText.replacementRangeForRenderPatch(
    content: Spanned,
    startIndex: Int,
    deleteCount: Int
): RenderReplaceRange? {
    val start = firstCharacterOffsetForTopLevelChildIndex(content, startIndex)
        ?: if (deleteCount == 0) content.length else return null
    val endExclusive = firstCharacterOffsetForTopLevelChildIndex(content, startIndex + deleteCount)
        ?: content.length
    if (start > endExclusive) return null
    return RenderReplaceRange(start = start, endExclusive = endExclusive)
}

internal fun EditorEditText.spannedRangeContainsImageSpan(content: Spanned, start: Int, endExclusive: Int): Boolean {
    if (start >= endExclusive) return false
    return content.getSpans(start, endExclusive, BlockImageSpan::class.java).isNotEmpty()
}

internal fun EditorEditText.spannedContainsImageSpan(content: Spanned): Boolean =
    spannedRangeContainsImageSpan(content, 0, content.length)

internal fun EditorEditText.spannedRangeContainsUnstableAtom(
    content: Spanned,
    start: Int,
    endExclusive: Int,
): Boolean {
    if (start >= endExclusive) return false
    return content.getSpans(start, endExclusive, AtomBlockSpan::class.java)
        .any { !it.hasStableAtomId }
}

internal fun EditorEditText.spannedContainsUnstableAtom(content: Spanned): Boolean =
    spannedRangeContainsUnstableAtom(content, 0, content.length)

internal fun EditorEditText.buildPatchedSpannable(
    patch: ParsedRenderPatch,
    includeTrailingInterBlockSeparator: Boolean
): android.text.SpannableStringBuilder =
    RenderBridge.buildSpannableFromBlocks(
        patch.renderBlocks,
        startIndex = patch.startIndex,
        includeTrailingInterBlockSeparator = includeTrailingInterBlockSeparator,
        baseFontSize = baseFontSize,
        textColor = baseTextColor,
        theme = theme,
        density = resources.displayMetrics.density,
        hostView = this,
        atomConfiguration = atomRenderConfiguration,
        reuseImages = false,
    )

internal fun EditorEditText.cloneJsonArray(array: org.json.JSONArray): org.json.JSONArray =
    org.json.JSONArray().also { clone ->
        for (index in 0 until array.length()) {
            clone.put(array.opt(index))
        }
    }

internal fun EditorEditText.normalizedJsonValue(value: Any?): Any? =
    if (value === org.json.JSONObject.NULL) null else value

internal fun EditorEditText.jsonValuesEqual(left: Any?, right: Any?): Boolean {
    val normalizedLeft = normalizedJsonValue(left)
    val normalizedRight = normalizedJsonValue(right)
    if (normalizedLeft === normalizedRight) return true
    if (normalizedLeft == null || normalizedRight == null) return false

    if (normalizedLeft is org.json.JSONArray && normalizedRight is org.json.JSONArray) {
        if (normalizedLeft.length() != normalizedRight.length()) return false
        for (index in 0 until normalizedLeft.length()) {
            if (!jsonValuesEqual(normalizedLeft.opt(index), normalizedRight.opt(index))) {
                return false
            }
        }
        return true
    }

    if (normalizedLeft is org.json.JSONObject && normalizedRight is org.json.JSONObject) {
        if (normalizedLeft.length() != normalizedRight.length()) return false
        val keys = normalizedLeft.keys()
        while (keys.hasNext()) {
            val key = keys.next()
            if (!normalizedRight.has(key)) return false
            if (!jsonValuesEqual(normalizedLeft.opt(key), normalizedRight.opt(key))) {
                return false
            }
        }
        return true
    }

    if (normalizedLeft is Number && normalizedRight is Number) {
        return normalizedLeft.toDouble() == normalizedRight.toDouble()
    }

    return normalizedLeft == normalizedRight
}

internal fun EditorEditText.renderBlocksEqual(
    current: org.json.JSONArray,
    updated: org.json.JSONArray
): Boolean {
    if (current.length() != updated.length()) return false
    for (index in 0 until current.length()) {
        if (!jsonValuesEqual(current.opt(index), updated.opt(index))) {
            return false
        }
    }
    return true
}

internal fun EditorEditText.mergeRenderBlocks(
    current: org.json.JSONArray,
    patch: ParsedRenderPatch
): org.json.JSONArray? {
    if (
        patch.startIndex < 0 ||
        patch.deleteCount < 0 ||
        patch.startIndex > current.length() ||
        patch.startIndex + patch.deleteCount > current.length()
    ) {
        return null
    }

    return org.json.JSONArray().also { merged ->
        for (index in 0 until patch.startIndex) {
            merged.put(current.opt(index))
        }
        for (index in 0 until patch.renderBlocks.length()) {
            merged.put(patch.renderBlocks.opt(index))
        }
        for (index in (patch.startIndex + patch.deleteCount) until current.length()) {
            merged.put(current.opt(index))
        }
    }
}

internal fun EditorEditText.applyRenderPatchIfPossible(
    patch: ParsedRenderPatch,
    preserveInputConnectionForExternalUpdate: Boolean
): PatchApplyTrace {
    val eligibilityStartedAt = System.nanoTime()
    if (patch.deleteCount == 0 && patch.renderBlocks.length() == 0) {
        return PatchApplyTrace(
            applied = true,
            eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
            buildRenderNanos = 0L,
            applyRenderNanos = 0L
        )
    }
    if (patch.deleteCount != patch.renderBlocks.length()) {
        return PatchApplyTrace(
            applied = false,
            eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
            buildRenderNanos = 0L,
            applyRenderNanos = 0L
        )
    }
    val content = text as? Spanned ?: return PatchApplyTrace(
        applied = false,
        eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
        buildRenderNanos = 0L,
        applyRenderNanos = 0L
    )
    if (!hasTopLevelChildMetadata(content)) {
        return PatchApplyTrace(
            applied = false,
            eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
            buildRenderNanos = 0L,
            applyRenderNanos = 0L
        )
    }

    val replaceRange = replacementRangeForRenderPatch(content, patch.startIndex, patch.deleteCount)
        ?: return PatchApplyTrace(
            applied = false,
            eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
            buildRenderNanos = 0L,
            applyRenderNanos = 0L
        )
    if (
        spannedRangeContainsImageSpan(content, replaceRange.start, replaceRange.endExclusive) ||
        spannedRangeContainsUnstableAtom(content, replaceRange.start, replaceRange.endExclusive)
    ) {
        return PatchApplyTrace(
            applied = false,
            eligibilityNanos = System.nanoTime() - eligibilityStartedAt,
            buildRenderNanos = 0L,
            applyRenderNanos = 0L
        )
    }
    val eligibilityNanos = System.nanoTime() - eligibilityStartedAt

    val buildStartedAt = System.nanoTime()
    val patchedSpannable = buildPatchedSpannable(
        patch,
        includeTrailingInterBlockSeparator = replaceRange.endExclusive < content.length
    )
    val buildRenderNanos = System.nanoTime() - buildStartedAt
    if (
        spannedContainsImageSpan(patchedSpannable) ||
        spannedContainsUnstableAtom(patchedSpannable)
    ) {
        return PatchApplyTrace(
            applied = false,
            eligibilityNanos = eligibilityNanos,
            buildRenderNanos = buildRenderNanos,
            applyRenderNanos = 0L
        )
    }

    val applyStartedAt = System.nanoTime()
    applyRenderedSpannable(
        spannable = patchedSpannable,
        replaceRange = replaceRange,
        replacedTopLevelStartIndex = patch.startIndex,
        replacedTopLevelDeleteCount = patch.deleteCount,
        usedPatch = true,
        preserveInputConnectionForExternalUpdate = preserveInputConnectionForExternalUpdate
    )
    return PatchApplyTrace(
        applied = true,
        eligibilityNanos = eligibilityNanos,
        buildRenderNanos = buildRenderNanos,
        applyRenderNanos = System.nanoTime() - applyStartedAt
    )
}
