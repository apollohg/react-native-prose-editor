package com.apollohg.editor

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.text.Selection
import android.text.Spanned
import android.view.KeyEvent
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.CompletionInfo
import android.view.inputmethod.CorrectionInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputConnectionWrapper

/**
 * Custom [InputConnectionWrapper] that intercepts all text input from the soft keyboard
 * and routes it through the Rust editor-core engine via the hosting [EditorEditText].
 *
 * Instead of letting Android's EditText text storage handle insertions and deletions
 * directly, this class captures the user's intent (typing, deleting, IME composition)
 * and delegates to the Rust editor. The Rust editor returns render elements, which are
 * converted to [android.text.SpannableStringBuilder] via [RenderBridge] and applied
 * back to the EditText.
 *
 * ## Composition (IME) Handling
 *
 * For CJK input methods, swipe keyboards, and some autocorrect flows, [setComposingText],
 * [commitText], and [finishComposingText] are used together. During composition, we let
 * the base [InputConnection] render transient composing text, but keep the original
 * Rust-authorized replacement range so the final committed text lands at the correct
 * document position.
 *
 * ## Key Events
 *
 * Hardware keyboard events (backspace, enter) arrive via [sendKeyEvent]. We intercept
 * DEL and ENTER to route through the Rust editor.
 */
class EditorInputConnection(
    private val editorView: EditorEditText,
    baseConnection: InputConnection,
    private val boundEditorId: Long,
    private val boundGeneration: Long,
    private val boundMapperGeneration: Long,
) : InputConnectionWrapper(baseConnection, true) {
    private data class SurroundingDeleteRange(
        val utf16Start: Int,
        val utf16End: Int,
        val scalarStart: Int,
        val scalarEnd: Int
    )


    companion object {
        private fun textTraceSummary(text: CharSequence?): String {
            if (text == null) return "text=null"
            val value = text.toString()
            val codePoints = mutableListOf<String>()
            var index = 0
            while (index < value.length && codePoints.size < 4) {
                val codePoint = Character.codePointAt(value, index)
                codePoints.add(codePoint.toString(16))
                index += Character.charCount(codePoint)
            }
            return "textLength=${value.length} codePoints=${codePoints.joinToString(",")}"
        }

        private const val DUPLICATE_CORRECTION_COMMIT_WINDOW_MS = 1_000L

        internal fun codePointsToUtf16Length(
            text: String,
            fromUtf16Offset: Int,
            codePointCount: Int,
            forward: Boolean
        ): Int {
            if (codePointCount <= 0 || text.isEmpty()) return 0

            var remaining = codePointCount
            var utf16Length = 0

            if (forward) {
                var index = fromUtf16Offset.coerceIn(0, text.length)
                while (index < text.length && remaining > 0) {
                    val codePoint = Character.codePointAt(text, index)
                    val charCount = Character.charCount(codePoint)
                    utf16Length += charCount
                    index += charCount
                    remaining--
                }
            } else {
                var index = fromUtf16Offset.coerceIn(0, text.length)
                while (index > 0 && remaining > 0) {
                    val codePoint = Character.codePointBefore(text, index)
                    val charCount = Character.charCount(codePoint)
                    utf16Length += charCount
                    index -= charCount
                    remaining--
                }
            }

            return utf16Length
        }
    }

    private data class PendingDuplicateCorrectionCommit(
        val text: String,
        val deadlineMs: Long
    )

    private data class PendingCompositionCorrectionCommit(
        val text: String,
        val deadlineMs: Long,
        val generation: Long
    )

    private data class GeneratedCompositionAdjustment(
        val leadingText: String,
        val trailingText: String
    ) {
        fun sanitize(text: String): String {
            var sanitized = text
            if (leadingText.isNotEmpty() && sanitized.startsWith(leadingText)) {
                sanitized = sanitized.substring(leadingText.length)
            }
            if (trailingText.isNotEmpty() && sanitized.endsWith(trailingText)) {
                sanitized = sanitized.substring(0, sanitized.length - trailingText.length)
            }
            return sanitized
        }
    }

    private var pendingDuplicateCorrectionCommit: PendingDuplicateCorrectionCommit? = null
    private var pendingCompositionCorrectionCommit: PendingCompositionCorrectionCommit? = null
    private var pendingCompositionCorrectionGeneration: Long = 0L
    private var generatedCompositionAdjustment: GeneratedCompositionAdjustment? = null

    /**
     * Called when the IME commits finalized text (single character, word,
     * autocomplete selection, etc.).
     *
     * Routes the text through Rust instead of directly inserting into the EditText.
     */
    override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!isCurrentInputSessionFor("commitText")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        if (editorView.isApplyingRustState) {
            editorView.recordImeTraceForTesting(
                "commitTextPassthrough",
                "reason=applyingRust ${textTraceSummary(text)} cursor=$newCursorPosition"
            )
            return super.commitText(text, newCursorPosition)
        }
        if (editorView.editorId == 0L) {
            editorView.recordImeTraceForTesting(
                "commitTextPassthrough",
                "reason=noEditor ${textTraceSummary(text)} cursor=$newCursorPosition"
            )
            return super.commitText(text, newCursorPosition)
        }

        editorView.recordImeTraceForTesting(
            "commitText",
            "${textTraceSummary(text)} cursor=$newCursorPosition"
        )
        val committedText = text?.toString()
        if (consumePendingCompositionCorrectionCommitIfNeeded(committedText, newCursorPosition)) {
            return true
        }
        applyPendingCompositionCorrectionCommitIfNeeded("commitTextBeforePlain")
        if (consumePendingDuplicateCorrectionCommitIfNeeded(committedText)) {
            editorView.recordImeTraceForTesting(
                "commitTextDuplicateCorrectionIgnored",
                "textLength=${committedText?.length ?: 0}"
            )
            return true
        }
        commitTextToEditor(committedText, newCursorPosition)
        return true
    }

    override fun commitCompletion(text: CompletionInfo?): Boolean {
        if (!isCurrentInputSessionFor("commitCompletion")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        if (editorView.isApplyingRustState) {
            return super.commitCompletion(text)
        }
        if (editorView.editorId == 0L) {
            return super.commitCompletion(text)
        }
        editorView.recordImeTraceForTesting(
            "commitCompletion",
            textTraceSummary(text?.text)
        )
        commitTextToEditor(text?.text?.toString(), 1)
        return true
    }

    override fun getCursorCapsMode(reqModes: Int): Int {
        val baseCapsMode = super.getCursorCapsMode(reqModes)
        if (!isCurrentInputSession()) return baseCapsMode
        val capsMode = editorView.cursorCapsModeForEditor(reqModes, baseCapsMode)
        if (capsMode != baseCapsMode) {
            editorView.recordImeTraceForTesting(
                "getCursorCapsModeAdjusted",
                "req=$reqModes base=$baseCapsMode caps=$capsMode"
            )
        }
        return capsMode
    }

    override fun getTextBeforeCursor(n: Int, flags: Int): CharSequence? {
        val mapper = currentMapper() ?: return ""
        val cursor = minOf(editorView.selectionStart, editorView.selectionEnd)
        if (cursor < 0) return ""
        val end = mapper.rawToIme(cursor)
        return imeTextSlice(mapper, maxOf(0, end - n.coerceAtLeast(0)), end, flags)
    }

    override fun getTextAfterCursor(n: Int, flags: Int): CharSequence? {
        val mapper = currentMapper() ?: return ""
        val cursor = maxOf(editorView.selectionStart, editorView.selectionEnd)
        if (cursor < 0) return ""
        val start = mapper.rawToIme(cursor)
        return imeTextSlice(
            mapper,
            start,
            minOf(mapper.visibleText.length, start + n.coerceAtLeast(0)),
            flags,
        )
    }

    override fun getSelectedText(flags: Int): CharSequence? {
        val mapper = currentMapper() ?: return ""
        val rawStart = editorView.selectionStart
        val rawEnd = editorView.selectionEnd
        if (rawStart < 0 || rawEnd < 0) return ""
        if (rawStart == rawEnd) return null
        val imeStart = mapper.rawToIme(minOf(rawStart, rawEnd))
        val imeEnd = mapper.rawToIme(maxOf(rawStart, rawEnd))
        if (imeStart == imeEnd) return null
        return imeTextSlice(
            mapper,
            imeStart,
            imeEnd,
            flags,
        )
    }

    override fun commitCorrection(correctionInfo: CorrectionInfo?): Boolean {
        if (!isCurrentInputSessionFor("commitCorrection")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        if (editorView.isApplyingRustState) {
            return super.commitCorrection(correctionInfo)
        }
        if (editorView.editorId == 0L) {
            return super.commitCorrection(correctionInfo)
        }
        val newText = correctionInfo?.newText?.toString()
        if (newText == null) return true
        editorView.recordImeTraceForTesting(
            "commitCorrection",
            "offset=${correctionInfo.offset} oldMissing=${correctionInfo.oldText == null} newLength=${newText.length}"
        )
        if (trackedCompositionReplacementRange() != null) {
            editorView.recordImeTraceForTesting(
                "commitCorrectionComposition",
                "newLength=${newText.length}"
            )
            rememberPendingCompositionCorrectionCommit(newText)
            return true
        }
        if (consumeInvalidatedCompositionReplacementRangeAndRestore()) {
            editorView.recordImeTraceForTesting("commitCorrectionRestoredInvalidComposition")
            return true
        }
        val oldText = correctionInfo.oldText?.toString()
        val imeOffset = correctionInfo.offset
        val mapper = currentMapper()
        val applied = if (oldText != null && mapper != null) {
            val validOffset = imeOffset in 0..mapper.visibleText.length
            val imeEnd = if (
                validOffset &&
                oldText.length <= mapper.visibleText.length - imeOffset
            ) {
                imeOffset + oldText.length
            } else {
                -1
            }
            if (
                imeEnd >= 0 &&
                mapper.visibleText.subSequence(imeOffset, imeEnd).toString() == oldText
            ) {
                val rawStart = mapper.imeToRaw(
                    imeOffset,
                    ImeTextCoordinateMapper.Affinity.AFTER,
                )
                val rawEnd = mapper.imeToRaw(
                    imeEnd,
                    ImeTextCoordinateMapper.Affinity.BEFORE,
                )
                val renderedOldText = editorView.text
                    ?.subSequence(rawStart, rawEnd)
                    ?.toString()
                    ?: ""
                editorView.handleCorrectionCommit(
                    rawStart,
                    rawEnd,
                    renderedOldText,
                    newText,
                )
            } else {
                editorView.recordImeTraceForTesting(
                    "correctionExplicitNoop",
                    "reason=staleVisibleText offset=$imeOffset oldLength=${oldText.length}",
                )
                false
            }
        } else if (oldText == null) {
            val visibleText = mapper?.visibleText?.toString()
            val tokenRange = visibleText?.let {
                editorView.missingOldTextCorrectionTokenRangeForEditor(it, imeOffset)
            }
            if (mapper != null && tokenRange != null) {
                val rawStart = mapper.imeToRaw(
                    tokenRange.first,
                    ImeTextCoordinateMapper.Affinity.AFTER,
                )
                val rawEnd = mapper.imeToRaw(
                    tokenRange.second,
                    ImeTextCoordinateMapper.Affinity.BEFORE,
                )
                val renderedOldText = editorView.text
                    ?.subSequence(rawStart, rawEnd)
                    ?.toString()
                    ?: ""
                editorView.handleMissingOldTextCorrectionCommit(
                    rawStart,
                    rawEnd,
                    renderedOldText,
                    newText,
                )
            } else {
                editorView.recordImeTraceForTesting(
                    "correctionInferredNoop",
                    "reason=noVisibleToken offset=$imeOffset newLength=${newText.length}",
                )
                false
            }
        } else {
            false
        }
        editorView.recordImeTraceForTesting(
            "commitCorrectionResult",
            "applied=$applied"
        )
        if (applied) {
            rememberPendingDuplicateCorrectionCommit(newText)
        }
        return true
    }

    private fun rememberPendingDuplicateCorrectionCommit(text: String) {
        pendingDuplicateCorrectionCommit = PendingDuplicateCorrectionCommit(
            text = text,
            deadlineMs = SystemClock.uptimeMillis() + DUPLICATE_CORRECTION_COMMIT_WINDOW_MS
        )
    }

    private fun consumePendingDuplicateCorrectionCommitIfNeeded(text: String?): Boolean {
        val pending = pendingDuplicateCorrectionCommit ?: return false
        pendingDuplicateCorrectionCommit = null
        if (text == null) return false
        if (SystemClock.uptimeMillis() > pending.deadlineMs) return false
        return text == pending.text
    }

    private fun rememberPendingCompositionCorrectionCommit(text: String) {
        val generation = ++pendingCompositionCorrectionGeneration
        pendingCompositionCorrectionCommit = PendingCompositionCorrectionCommit(
            text = text,
            deadlineMs = SystemClock.uptimeMillis() + DUPLICATE_CORRECTION_COMMIT_WINDOW_MS,
            generation = generation
        )
        Handler(Looper.getMainLooper()).post {
            val pending = pendingCompositionCorrectionCommit ?: return@post
            if (pending.generation != generation) return@post
            applyPendingCompositionCorrectionCommitIfNeeded("commitCorrectionDeferred")
        }
    }

    private fun consumePendingCompositionCorrectionCommitIfNeeded(
        text: String?,
        newCursorPosition: Int
    ): Boolean {
        val pending = pendingCompositionCorrectionCommit ?: return false
        if (SystemClock.uptimeMillis() > pending.deadlineMs) {
            pendingCompositionCorrectionCommit = null
            return false
        }
        if (text != pending.text) return false
        pendingCompositionCorrectionCommit = null
        pendingCompositionCorrectionGeneration += 1L
        editorView.recordImeTraceForTesting(
            "commitTextConsumesPendingCorrection",
            "textLength=${text.length}"
        )
        commitTextToEditor(text, newCursorPosition)
        return true
    }

    private fun applyPendingCompositionCorrectionCommitIfNeeded(source: String): Boolean {
        val pending = pendingCompositionCorrectionCommit ?: return false
        pendingCompositionCorrectionCommit = null
        pendingCompositionCorrectionGeneration += 1L
        if (!isCurrentInputSessionFor("applyPendingCompositionCorrection")) return false
        if (!editorView.isEditable || editorView.editorId == 0L) return false
        editorView.recordImeTraceForTesting(
            "applyPendingCompositionCorrection",
            "source=$source textLength=${pending.text.length}"
        )
        commitTextToEditor(pending.text, 1)
        return true
    }

    private fun commitTextToEditor(committedText: String?, newCursorPosition: Int) {
        val startedAt = System.nanoTime()
        val trackedReplacementRange = trackedCompositionReplacementRange()
        val rawComposingSpanRange = currentComposingSpanRawRange()
        val currentAuthorizedComposingSpanRange = currentComposingSpanRange()
        val visibleReplacementRange = rawComposingSpanRange ?: trackedReplacementRange
        val replacementRange = trackedReplacementRange?.let { range ->
            if (range.first == range.second) {
                currentAuthorizedComposingSpanRange ?: range
            } else {
                range
            }
        }
        if (replacementRange != null) {
            editorView.recordImeTraceForTesting(
                "commitTextRoute",
                "route=composition replacement=${replacementRange.first}..${replacementRange.second} visible=${visibleReplacementRange?.first}..${visibleReplacementRange?.second} textLength=${committedText?.length ?: 0}"
            )
            clearCompositionTracking()
            editorView.runWithTransientInputMutationGuard {
                super.finishComposingText()
            }
            if (committedText != null) {
                var didCommitAlreadyVisibleMutation = false
                if (
                    trackedReplacementRange?.first == trackedReplacementRange?.second &&
                    rawComposingSpanRange == null
                ) {
                    editorView.runWithDeferredRustUpdateApplication {
                        didCommitAlreadyVisibleMutation =
                            editorView.commitAlreadyVisibleCompositionMutationForPendingImeOperationForEditor(
                                committedText,
                                newCursorPosition
                            )
                    }
                }
                if (!didCommitAlreadyVisibleMutation) {
                    visibleReplacementRange?.let { visibleRange ->
                        editorView.applyVisibleCompositionCommitForPendingImeOperationForEditor(
                            committedText,
                            visibleRange.first,
                            visibleRange.second,
                            newCursorPosition
                        )
                    }
                    editorView.runWithDeferredRustUpdateApplication {
                        editorView.handleCompositionCommit(
                            committedText,
                            replacementRange.first,
                            replacementRange.second,
                            newCursorPosition
                        )
                    }
                }
            } else {
                editorView.restoreAuthorizedTextIfNeeded()
            }
        } else {
            if (consumeInvalidatedCompositionReplacementRangeAndRestore()) {
                editorView.recordImeTraceForTesting(
                    "commitTextRoute",
                    "route=restoreInvalidComposition textLength=${committedText?.length ?: 0}"
                )
                return
            }
            clearCompositionTracking()
            editorView.recordImeTraceForTesting(
                "commitTextRoute",
                "route=plain textLength=${committedText?.length ?: 0}"
            )
            committedText?.let { editorView.handleTextCommit(it, newCursorPosition) }
        }
        editorView.recordImeTraceForTesting(
            "commitTextRouteDone",
            "textLength=${committedText?.length ?: 0} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
    }

    /**
     * Called when the IME requests deletion of text surrounding the cursor.
     *
     * Routes the deletion through Rust instead of directly modifying the EditText.
     *
     * @param beforeLength Number of UTF-16 code units to delete before the cursor.
     * @param afterLength Number of UTF-16 code units to delete after the cursor.
     */
    override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
        if (!isCurrentInputSessionFor("deleteSurroundingText")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        if (editorView.isApplyingRustState) {
            return super.deleteSurroundingText(beforeLength, afterLength)
        }
        editorView.recordImeTraceForTesting(
            "deleteSurroundingText",
            "before=$beforeLength after=$afterLength"
        )
        if (
            editorView.hasInvalidatedCompositionReplacementRangeForEditor() &&
            isNoOpSurroundingDelete(beforeLength, afterLength)
        ) {
            return finishStaleComposingUpdateAfterInvalidation()
        }
        if (consumeInvalidatedCompositionReplacementRangeAndRestore()) {
            return true
        }
        if (trackedCompositionReplacementRange() != null) {
            return performMappedCompositionSurroundingDelete(
                beforeLength,
                afterLength,
                deleteInCodePoints = false,
            )
        }
        if (shouldDeferPlainSurroundingDelete(beforeLength, afterLength)) {
            return performDeferredPlainSurroundingDelete(
                beforeLength = beforeLength,
                afterLength = afterLength,
                deleteInCodePoints = false
            )
        }
        editorView.handleDelete(beforeLength, afterLength)
        return true
    }

    override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean {
        if (!isCurrentInputSessionFor("deleteSurroundingTextInCodePoints")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        if (editorView.isApplyingRustState) {
            return super.deleteSurroundingTextInCodePoints(beforeLength, afterLength)
        }
        editorView.recordImeTraceForTesting(
            "deleteSurroundingTextInCodePoints",
            "before=$beforeLength after=$afterLength"
        )
        if (
            editorView.hasInvalidatedCompositionReplacementRangeForEditor() &&
            isNoOpSurroundingDelete(beforeLength, afterLength)
        ) {
            return finishStaleComposingUpdateAfterInvalidation()
        }
        if (consumeInvalidatedCompositionReplacementRangeAndRestore()) {
            return true
        }
        if (trackedCompositionReplacementRange() != null) {
            return performMappedCompositionSurroundingDelete(
                beforeLength,
                afterLength,
                deleteInCodePoints = true,
            )
        }
        if (shouldDeferPlainSurroundingDelete(beforeLength, afterLength)) {
            return performDeferredPlainSurroundingDelete(
                beforeLength = beforeLength,
                afterLength = afterLength,
                deleteInCodePoints = true
            )
        }

        val currentText = editorView.text?.toString().orEmpty()
        val cursor = editorView.selectionStart.coerceAtLeast(0)
        val beforeUtf16Length = codePointsToUtf16Length(
            text = currentText,
            fromUtf16Offset = cursor,
            codePointCount = beforeLength,
            forward = false
        )
        val afterUtf16Length = codePointsToUtf16Length(
            text = currentText,
            fromUtf16Offset = editorView.selectionEnd.coerceAtLeast(cursor),
            codePointCount = afterLength,
            forward = true
        )
        editorView.handleDelete(beforeUtf16Length, afterUtf16Length)
        return true
    }

    private fun shouldDeferPlainSurroundingDelete(beforeLength: Int, afterLength: Int): Boolean =
        beforeLength.coerceAtLeast(0) + afterLength.coerceAtLeast(0) > 0

    private fun performMappedCompositionSurroundingDelete(
        beforeLength: Int,
        afterLength: Int,
        deleteInCodePoints: Boolean,
    ): Boolean {
        val mapper = currentMapper() ?: return true
        val rawStart = editorView.selectionStart
        val rawEnd = editorView.selectionEnd
        if (rawStart < 0 || rawEnd < 0) return true
        val imeStart = mapper.rawToIme(minOf(rawStart, rawEnd))
        val imeEnd = mapper.rawToIme(maxOf(rawStart, rawEnd))
        val visibleText = mapper.visibleText.toString()
        val beforeUtf16Length = if (deleteInCodePoints) {
            codePointsToUtf16Length(visibleText, imeStart, beforeLength, forward = false)
        } else {
            beforeLength
        }
        val afterUtf16Length = if (deleteInCodePoints) {
            codePointsToUtf16Length(visibleText, imeEnd, afterLength, forward = true)
        } else {
            afterLength
        }
        val imeDeleteStart = if (imeStart != imeEnd) {
            imeStart
        } else {
            maxOf(0, imeStart - beforeUtf16Length.coerceAtLeast(0))
        }
        val imeDeleteEnd = if (imeStart != imeEnd) {
            imeEnd
        } else {
            minOf(visibleText.length, imeEnd + afterUtf16Length.coerceAtLeast(0))
        }
        val rawDeleteStart = mapper.imeToRaw(
            imeDeleteStart,
            ImeTextCoordinateMapper.Affinity.AFTER,
        )
        val rawDeleteEnd = mapper.imeToRaw(
            imeDeleteEnd,
            ImeTextCoordinateMapper.Affinity.BEFORE,
        )
        editorView.runWithTransientInputMutationGuard {
            deleteVisibleTextInRawRange(rawDeleteStart, rawDeleteEnd, imeDeleteStart)
        }
        refreshComposingTextFromEditable()
        return true
    }

    private fun performDeferredPlainSurroundingDelete(
        beforeLength: Int,
        afterLength: Int,
        deleteInCodePoints: Boolean
    ): Boolean {
        val beforeText = editorView.text?.toString() ?: return true
        val mapper = currentMapper() ?: return true
        val rawSelectionStart = editorView.selectionStart
        val rawSelectionEnd = editorView.selectionEnd
        if (rawSelectionStart < 0 || rawSelectionEnd < 0) return true
        val normalizedRawStart = minOf(rawSelectionStart, rawSelectionEnd)
            .coerceIn(0, beforeText.length)
        val normalizedRawEnd = maxOf(rawSelectionStart, rawSelectionEnd)
            .coerceIn(0, beforeText.length)
        val imeSelectionStart = mapper.rawToIme(normalizedRawStart)
        val imeSelectionEnd = mapper.rawToIme(normalizedRawEnd)
        val beforeUtf16Length: Int
        val afterUtf16Length: Int
        if (deleteInCodePoints) {
            val visibleText = mapper.visibleText.toString()
            beforeUtf16Length = codePointsToUtf16Length(
                text = visibleText,
                fromUtf16Offset = imeSelectionStart,
                codePointCount = beforeLength,
                forward = false
            )
            afterUtf16Length = codePointsToUtf16Length(
                text = visibleText,
                fromUtf16Offset = imeSelectionEnd,
                codePointCount = afterLength,
                forward = true
            )
        } else {
            beforeUtf16Length = beforeLength
            afterUtf16Length = afterLength
        }
        val imeDeleteStart: Int
        val imeDeleteEnd: Int
        if (imeSelectionStart != imeSelectionEnd) {
            imeDeleteStart = imeSelectionStart
            imeDeleteEnd = imeSelectionEnd
        } else {
            imeDeleteStart = maxOf(0, imeSelectionStart - beforeUtf16Length.coerceAtLeast(0))
            imeDeleteEnd = minOf(
                mapper.visibleText.length,
                imeSelectionEnd + afterUtf16Length.coerceAtLeast(0),
            )
        }
        val rawDeleteStart = mapper.imeToRaw(
            imeDeleteStart,
            ImeTextCoordinateMapper.Affinity.AFTER,
        )
        val rawDeleteEnd = mapper.imeToRaw(
            imeDeleteEnd,
            ImeTextCoordinateMapper.Affinity.BEFORE,
        )
        val deleteRange = surroundingDeleteRange(
            text = beforeText,
            rawDeleteStart = rawDeleteStart,
            rawDeleteEnd = rawDeleteEnd,
            selectionStart = normalizedRawStart,
            selectionEnd = normalizedRawEnd,
        )
        val isCollapsedBackwardDelete =
            beforeLength == 1 &&
                afterLength == 0 &&
                editorView.selectionStart == editorView.selectionEnd

        if (isCollapsedBackwardDelete) {
            val hiddenGapStart = mapper.imeToRaw(
                imeSelectionStart,
                ImeTextCoordinateMapper.Affinity.BEFORE,
            )
            if (
                hiddenGapStart < normalizedRawStart &&
                editorView.renderedRangeContainsGeneratedStructure(
                    hiddenGapStart,
                    normalizedRawStart,
                )
            ) {
                editorView.recordImeTraceForTesting(
                    "structuralSurroundingDelete",
                    "before=$beforeLength after=$afterLength codePoints=$deleteInCodePoints hiddenGap=true",
                )
                editorView.handleStructuralBackspace()
                return true
            }
        }

        if (
            deleteRange != null &&
            editorView.renderedRangeContainsGeneratedStructure(
                deleteRange.utf16Start,
                deleteRange.utf16End
            )
        ) {
            editorView.recordImeTraceForTesting(
                "structuralSurroundingDelete",
                "before=$beforeLength after=$afterLength codePoints=$deleteInCodePoints"
            )
            if (isCollapsedBackwardDelete) {
                editorView.handleStructuralBackspace()
            } else {
                editorView.handleStructuralDelete(
                    deleteRange.utf16Start,
                    deleteRange.utf16End,
                    deleteRange.scalarStart,
                    deleteRange.scalarEnd
                )
            }
            return true
        }

        editorView.recordImeTraceForTesting(
            "deferredSurroundingDeleteBegin",
            "before=$beforeLength after=$afterLength codePoints=$deleteInCodePoints utf16=$beforeUtf16Length,$afterUtf16Length scalar=${deleteRange?.scalarStart}..${deleteRange?.scalarEnd}"
        )

        val authoritative = editorView.captureAuthoritativeInputSnapshotForEditor()
        val didDeleteVisibleText = editorView.runWithTransientInputMutationGuard {
            deleteVisibleTextInRawRange(rawDeleteStart, rawDeleteEnd, imeDeleteStart)
        }
        if (didDeleteVisibleText && deleteRange != null) {
            when (
                val outcome = editorView.deleteScalarRangeForPendingImeOperationForEditor(
                    deleteRange.scalarStart,
                    deleteRange.scalarEnd,
                )
            ) {
                is EditorV2NativeIntentResult.Applied -> {
                    editorView.runWithDeferredRustUpdateApplication {
                        editorView.promoteOptimisticInputForEditor(
                            outcome.render,
                            deleteRange.scalarStart,
                        )
                    }
                }
                is EditorV2NativeIntentResult.Recovered -> {
                    editorView.restoreAuthoritativeInputForEditor(
                        authoritative,
                        outcome.updateJson,
                    )
                }
                EditorV2NativeIntentResult.Rejected -> {
                    editorView.restoreAuthoritativeInputForEditor(authoritative)
                }
                null -> {
                    editorView.authorizeCurrentVisibleTextForPendingImeOperationForEditor(
                        logicalCursorAfter = deleteRange.scalarStart,
                    )
                }
            }
        }
        editorView.recordImeTraceForTesting(
            "deferredSurroundingDeleteEnd",
            "visibleDeleted=$didDeleteVisibleText visibleLength=${editorView.text?.length ?: -1}"
        )
        return true
    }

    private fun surroundingDeleteRange(
        text: String,
        rawDeleteStart: Int,
        rawDeleteEnd: Int,
        selectionStart: Int,
        selectionEnd: Int,
    ): SurroundingDeleteRange? {
        val (deleteStart, deleteEnd) = PositionBridge.snapRangeToScalarBoundaries(
            rawDeleteStart,
            rawDeleteEnd,
            text
        )
        val logicalSelection = editorView.currentLogicalScalarSelectionForInput()
        if (logicalSelection != null) {
            val logicalStart = minOf(logicalSelection.first, logicalSelection.second)
            val logicalEnd = maxOf(logicalSelection.first, logicalSelection.second)
            if (logicalStart != logicalEnd) {
                return SurroundingDeleteRange(deleteStart, deleteEnd, logicalStart, logicalEnd)
            }
            val deletedBefore = visibleCodePointCount(text, deleteStart, selectionStart)
            val deletedAfter = visibleCodePointCount(text, selectionEnd, deleteEnd)
            val scalarStart = (logicalStart - deletedBefore).coerceAtLeast(0)
            val scalarEnd = logicalEnd + deletedAfter
            if (scalarStart < scalarEnd) {
                return SurroundingDeleteRange(deleteStart, deleteEnd, scalarStart, scalarEnd)
            }
        }
        val scalarStart = PositionBridge.utf16ToScalar(deleteStart, text)
        val scalarEnd = PositionBridge.utf16ToScalar(deleteEnd, text)
        if (scalarStart >= scalarEnd) return null
        return SurroundingDeleteRange(deleteStart, deleteEnd, scalarStart, scalarEnd)
    }

    private fun visibleCodePointCount(text: String, start: Int, end: Int): Int {
        val visible = text
            .substring(minOf(start, end), maxOf(start, end))
            .replace(LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER, "")
        return visible.codePointCount(0, visible.length)
    }

    private fun deleteVisibleTextInRawRange(
        rawStart: Int,
        rawEnd: Int,
        imeCursorAfter: Int,
    ): Boolean {
        val editable = editorView.text ?: return false
        val start = rawStart.coerceIn(0, editable.length)
        val end = rawEnd.coerceIn(start, editable.length)
        var chunkEnd = end
        var index = end - 1
        var didDelete = false
        while (index >= start) {
            if (editable[index] == LayoutConstants.SYNTHETIC_PLACEHOLDER_CHARACTER[0]) {
                if (index + 1 < chunkEnd) {
                    editable.delete(index + 1, chunkEnd)
                    didDelete = true
                }
                chunkEnd = index
            }
            index -= 1
        }
        if (start < chunkEnd) {
            editable.delete(start, chunkEnd)
            didDelete = true
        }
        if (didDelete) {
            val updatedMapper = currentMapper()
            val rawCursor = updatedMapper?.imeToRaw(
                imeCursorAfter,
                ImeTextCoordinateMapper.Affinity.AFTER,
            ) ?: start.coerceIn(0, editable.length)
            Selection.setSelection(editable, rawCursor.coerceIn(0, editable.length))
        }
        return didDelete
    }

    /**
     * Called when the IME sets composing (in-progress) text for CJK/swipe input.
     *
     * We let the base InputConnection handle this normally so the user sees
     * the composing text with its underline decoration. The text is NOT sent
     * to Rust during composition — only when the IME commits or finishes it.
     */
    override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean {
        if (!isCurrentInputSessionFor("setComposingText")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        if (editorView.editorId == 0L) return super.setComposingText(text, newCursorPosition)
        if (editorView.hasInvalidatedCompositionReplacementRangeForEditor()) {
            return finishStaleComposingUpdateAfterInvalidation()
        }
        captureCompositionReplacementRangeIfNeeded()
        val composingText = text?.toString()?.let { value ->
            generatedCompositionAdjustment?.sanitize(value) ?: value
        }
        val adjustedComposingText =
            editorView.samsungSentenceCapsComposingTextForEditor(composingText)
        val textForBaseConnection = adjustedComposingText ?: text
        editorView.recordImeTraceForTesting(
            "setComposingText",
            "${textTraceSummary(text)} cursor=$newCursorPosition adjusted=${textForBaseConnection.toString() != text?.toString()}"
        )
        editorView.setComposingTextForEditor(adjustedComposingText)
        val trackedRange = trackedCompositionReplacementRange()
        val currentText = editorView.text?.toString()
        if (
            trackedRange != null &&
            currentText != null &&
            editorView.isCurrentTextAuthorizedForEditor() &&
            currentText.substring(trackedRange.first, trackedRange.second) == adjustedComposingText
        ) {
            return editorView.runWithTransientInputMutationGuard {
                val regionSet = super.setComposingRegion(trackedRange.first, trackedRange.second)
                val requestedCursor = if (newCursorPosition > 0) {
                    trackedRange.second + newCursorPosition - 1
                } else {
                    trackedRange.first + newCursorPosition
                }.coerceIn(0, currentText.length)
                val selectionSet = super.setSelection(requestedCursor, requestedCursor)
                if (regionSet) {
                    editorView.applyTransientComposingTextStyleForEditor()
                }
                regionSet && selectionSet
            }
        }
        return editorView.runWithTransientInputMutationGuard {
            val result = super.setComposingText(textForBaseConnection, newCursorPosition)
            if (result) {
                editorView.applyTransientComposingTextStyleForEditor()
            }
            result
        }
    }

    override fun setComposingRegion(start: Int, end: Int): Boolean {
        if (!isCurrentInputSessionFor("setComposingRegion")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (!editorView.isEditable) return true
        val rawRange = rawRangeForIme(start, end) ?: return true
        if (editorView.editorId == 0L) {
            return super.setComposingRegion(rawRange.first, rawRange.second)
        }
        if (editorView.hasInvalidatedCompositionReplacementRangeForEditor()) {
            return finishStaleComposingUpdateAfterInvalidation()
        }
        val currentText = editorView.text?.toString().orEmpty()
        val requestedStart = minOf(rawRange.first, rawRange.second).coerceIn(0, currentText.length)
        val requestedEnd = maxOf(rawRange.first, rawRange.second).coerceIn(0, currentText.length)
        val contentRange = editorView.compositionContentRangeForEditor(requestedStart, requestedEnd)
        if (contentRange == null) {
            generatedCompositionAdjustment = null
            editorView.recordImeTraceForTesting(
                "setComposingRegionRejected",
                "range=$start..$end reason=generatedInterior"
            )
            return true
        }
        if (editorView.isCurrentTextAuthorizedForEditor()) {
            editorView.setCompositionReplacementRange(contentRange.first, contentRange.second)
        }
        generatedCompositionAdjustment = if (
            contentRange.first != requestedStart || contentRange.second != requestedEnd
        ) {
            GeneratedCompositionAdjustment(
                leadingText = currentText.substring(requestedStart, contentRange.first),
                trailingText = currentText.substring(contentRange.second, requestedEnd)
            )
        } else {
            null
        }
        editorView.recordImeTraceForTesting(
            "setComposingRegion",
            "range=$start..$end content=${contentRange.first}..${contentRange.second}"
        )
        return editorView.runWithTransientInputMutationGuard {
            val result = super.setComposingRegion(contentRange.first, contentRange.second)
            if (result) {
                editorView.applyTransientComposingTextStyleForEditor()
            }
            result
        }
    }

    override fun setSelection(start: Int, end: Int): Boolean {
        if (!isCurrentInputSessionFor("setSelection")) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        val rawRange = rawRangeForIme(start, end) ?: return true
        if (!editorView.isEditable) {
            consumeInvalidatedCompositionReplacementRangeAndRestore()
            return true
        }
        if (editorView.isApplyingRustState) {
            return super.setSelection(rawRange.first, rawRange.second)
        }
        if (editorView.editorId == 0L) {
            return super.setSelection(rawRange.first, rawRange.second)
        }
        if (editorView.hasInvalidatedCompositionReplacementRangeForEditor()) {
            return finishStaleComposingUpdateAfterInvalidation()
        }
        return super.setSelection(rawRange.first, rawRange.second)
    }

    /**
     * Called when IME composition is finalized (user selects a candidate or
     * presses space/enter to commit the composing text).
     *
     * At this point, the composed text is final. We notify the [EditorEditText]
     * so it can capture the result and send it to Rust.
     */
    override fun finishComposingText(): Boolean {
        if (!isCurrentInputSessionFor("finishComposingText")) return true
        if (editorView.hasActiveExternalTextCompositionForEditor()) return true
        if (applyPendingCompositionCorrectionCommitIfNeeded("finishComposingText")) return true
        return finishComposingTextInternal(blockWhenCompositionWasCancelled = false)
    }

    internal fun flushPendingCompositionForExternalMutation(): Boolean {
        if (!isCurrentInputSessionFor("flushPendingComposition")) return true
        if (!hasPendingComposition()) return true
        return finishComposingTextInternal(blockWhenCompositionWasCancelled = true)
    }

    internal fun hasPendingComposition(): Boolean {
        if (!isCurrentInputSessionFor("hasPendingComposition")) return false
        if (trackedCompositionReplacementRange() != null) return true
        val editable = editorView.text ?: return false
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        return start >= 0 && end >= 0 && start != end
    }

    internal fun refreshComposingTextFromEditableForEditor() {
        if (!isCurrentInputSessionFor("refreshComposingText")) return
        refreshComposingTextFromEditable()
    }

    internal fun clearCompositionTrackingForEditor() {
        if (!isCurrentInputSessionFor("clearCompositionTracking")) return
        clearCompositionTracking()
    }

    internal fun deleteTransientTextForHardwareKeyEvent(event: KeyEvent): Boolean =
        if (!isCurrentInputSession()) {
            false
        } else {
            when (event.keyCode) {
                KeyEvent.KEYCODE_DEL -> deleteTransientTextAroundSelectionInCodePoints(1, 0)
                KeyEvent.KEYCODE_FORWARD_DEL -> deleteTransientTextAroundSelectionInCodePoints(0, 1)
                else -> false
            }
        }

    private fun finishComposingTextInternal(blockWhenCompositionWasCancelled: Boolean): Boolean {
        if (!isCurrentInputSessionFor("finishComposingText")) return true
        if (!editorView.isEditable) {
            clearCompositionTracking()
            editorView.restoreAuthorizedTextIfNeeded()
            return true
        }
        if (editorView.editorId == 0L) return super.finishComposingText()
        refreshComposingTextFromEditable()
        val composed = editorView.composingTextForEditor() ?: currentComposingSpanText()
        val trackedReplacementRange = trackedCompositionReplacementRange()
        val didInvalidateReplacementRange = consumeInvalidatedCompositionReplacementRange()
        val replacementRange = if (didInvalidateReplacementRange) {
            null
        } else {
            trackedReplacementRange ?: currentComposingSpanRange()
        }
        editorView.recordImeTraceForTesting(
            "finishComposingText",
            "replacement=${replacementRange?.first}..${replacementRange?.second} composedLength=${composed?.length ?: 0} invalidated=$didInvalidateReplacementRange"
        )
        clearCompositionTracking()

        // Prevent selection sync while the base connection commits the composed
        // text, since the Rust document doesn't have it yet.
        val result = editorView.runWithTransientInputMutationGuard {
            super.finishComposingText()
        }

        // Now route the composed text through Rust.
        if (
            replacementRange != null &&
            (!composed.isNullOrEmpty() || replacementRange.first != replacementRange.second)
        ) {
            editorView.runWithDeferredRustUpdateApplication {
                editorView.handleCompositionCommit(
                    composed.orEmpty(),
                    replacementRange.first,
                    replacementRange.second
                )
            }
            return true
        } else if (replacementRange != null) {
            editorView.restoreAuthorizedTextIfNeeded()
            return !blockWhenCompositionWasCancelled
        } else if (didInvalidateReplacementRange) {
            editorView.restoreAuthorizedTextIfNeeded()
            return !blockWhenCompositionWasCancelled
        }
        return result
    }

    private fun captureCompositionReplacementRangeIfNeeded() {
        editorView.captureCompositionReplacementRangeIfNeeded()
    }

    private fun trackedCompositionReplacementRange(): Pair<Int, Int>? {
        return editorView.compositionReplacementRange()
    }

    private fun clearCompositionTracking() {
        generatedCompositionAdjustment = null
        editorView.clearCompositionTrackingForEditor()
    }

    private fun consumeInvalidatedCompositionReplacementRange(): Boolean =
        editorView.consumeInvalidatedCompositionReplacementRangeForEditor()

    private fun consumeInvalidatedCompositionReplacementRangeAndRestore(): Boolean {
        if (!consumeInvalidatedCompositionReplacementRange()) return false
        clearCompositionTracking()
        editorView.runWithTransientInputMutationGuard {
            super.finishComposingText()
        }
        editorView.restoreAuthorizedTextIfNeeded()
        return true
    }

    private fun finishStaleComposingUpdateAfterInvalidation(): Boolean {
        clearCompositionTracking()
        val result = editorView.runWithTransientInputMutationGuard {
            super.finishComposingText()
        }
        editorView.restoreAuthorizedTextIfNeeded()
        return result
    }

    private fun isCurrentInputSession(): Boolean =
        editorView.isInputConnectionCurrentForEditor(boundEditorId, boundGeneration)

    private fun currentMapper(): ImeTextCoordinateMapper? =
        editorView.imeTextCoordinateMapperForEditor(boundMapperGeneration)

    private fun imeTextSlice(
        mapper: ImeTextCoordinateMapper,
        start: Int,
        end: Int,
        flags: Int,
    ): CharSequence {
        val slice = mapper.visibleText.subSequence(start, end)
        return if ((flags and InputConnection.GET_TEXT_WITH_STYLES) != 0 && slice is Spanned) {
            slice
        } else {
            slice.toString()
        }
    }

    private fun rawRangeForIme(start: Int, end: Int): Pair<Int, Int>? {
        val mapper = currentMapper() ?: return null
        if (start == end) {
            val raw = mapper.imeToRaw(start, ImeTextCoordinateMapper.Affinity.AFTER)
            return raw to raw
        }
        return if (start < end) {
            mapper.imeToRaw(start, ImeTextCoordinateMapper.Affinity.AFTER) to
                mapper.imeToRaw(end, ImeTextCoordinateMapper.Affinity.BEFORE)
        } else {
            mapper.imeToRaw(start, ImeTextCoordinateMapper.Affinity.BEFORE) to
                mapper.imeToRaw(end, ImeTextCoordinateMapper.Affinity.AFTER)
        }
    }

    private fun nanosToMicros(nanos: Long): Long = nanos / 1_000L

    private fun isCurrentInputSessionFor(event: String): Boolean {
        val isCurrent = isCurrentInputSession()
        if (!isCurrent) {
            editorView.recordImeTraceForTesting(
                "${event}Ignored",
                "reason=stale boundEditor=$boundEditorId boundGen=$boundGeneration"
            )
        }
        return isCurrent
    }

    private fun refreshComposingTextFromEditable() {
        val editable = editorView.text ?: return
        val visibleReplacementText = editorView.composingTextFromVisibleReplacementForEditor()
        if (visibleReplacementText != null) {
            editorView.setComposingTextForEditor(
                ImeTextCoordinateMapper.build(visibleReplacementText, boundMapperGeneration)
                    .visibleText
                    .toString()
            )
            return
        }
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        if (start < 0 || end < 0 || start > end || end > editable.length) {
            editorView.setComposingTextForEditor(null)
            return
        }
        val mapper = currentMapper()
        val composingText = if (mapper != null) {
            mapper.visibleText.subSequence(
                mapper.rawToIme(start),
                mapper.rawToIme(end),
            ).toString()
        } else {
            editable.subSequence(start, end).toString()
        }
        editorView.setComposingTextForEditor(composingText)
    }

    private fun deleteTransientTextAroundSelection(beforeLength: Int, afterLength: Int): Boolean {
        val editable = editorView.text ?: return false
        val rawStart = editorView.selectionStart
        val rawEnd = editorView.selectionEnd
        if (rawStart < 0 || rawEnd < 0) return false
        val selectionStart = rawStart.coerceIn(0, editable.length)
        val selectionEnd = rawEnd.coerceIn(0, editable.length)
        val normalizedStart = minOf(selectionStart, selectionEnd)
        val normalizedEnd = maxOf(selectionStart, selectionEnd)
        val deleteStart: Int
        val deleteEnd: Int
        if (normalizedStart != normalizedEnd) {
            deleteStart = normalizedStart
            deleteEnd = normalizedEnd
        } else {
            deleteStart = maxOf(0, normalizedStart - beforeLength.coerceAtLeast(0))
            deleteEnd = minOf(editable.length, normalizedEnd + afterLength.coerceAtLeast(0))
        }
        if (deleteStart >= deleteEnd) return false
        val (snappedStart, snappedEnd) = PositionBridge.snapRangeToScalarBoundaries(
            deleteStart,
            deleteEnd,
            editable.toString()
        )
        if (snappedStart >= snappedEnd) return false
        editable.delete(snappedStart, snappedEnd)
        Selection.setSelection(editable, snappedStart.coerceIn(0, editable.length))
        return true
    }

    private fun deleteTransientTextAroundSelectionInCodePoints(
        beforeLength: Int,
        afterLength: Int
    ): Boolean {
        val currentText = editorView.text?.toString() ?: return false
        val rawStart = editorView.selectionStart
        val rawEnd = editorView.selectionEnd
        if (rawStart < 0 || rawEnd < 0) return false
        val selectionStart = rawStart.coerceIn(0, currentText.length)
        val selectionEnd = rawEnd.coerceIn(0, currentText.length)
        val normalizedStart = minOf(selectionStart, selectionEnd)
        val normalizedEnd = maxOf(selectionStart, selectionEnd)
        if (normalizedStart != normalizedEnd) {
            return deleteTransientTextAroundSelection(0, 0)
        }
        val beforeUtf16Length = codePointsToUtf16Length(
            text = currentText,
            fromUtf16Offset = normalizedStart,
            codePointCount = beforeLength,
            forward = false
        )
        val afterUtf16Length = codePointsToUtf16Length(
            text = currentText,
            fromUtf16Offset = normalizedEnd,
            codePointCount = afterLength,
            forward = true
        )
        return deleteTransientTextAroundSelection(beforeUtf16Length, afterUtf16Length)
    }

    private fun currentComposingSpanText(): String? {
        val editable = editorView.text ?: return null
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        if (start < 0 || end < 0 || start > end || end > editable.length) {
            return null
        }
        return editable.subSequence(start, end).toString()
    }

    private fun currentComposingSpanRange(): Pair<Int, Int>? {
        if (!editorView.isCurrentTextAuthorizedForEditor()) return null
        val editable = editorView.text ?: return null
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        if (start < 0 || end < 0 || start > end || end > editable.length) {
            return null
        }
        return editorView.authorizedUtf16Range(start, end)
    }

    private fun currentComposingSpanRawRange(): Pair<Int, Int>? {
        val editable = editorView.text ?: return null
        val start = BaseInputConnection.getComposingSpanStart(editable)
        val end = BaseInputConnection.getComposingSpanEnd(editable)
        if (start < 0 || end < 0 || start > end || end > editable.length) {
            return null
        }
        return start to end
    }

    /**
     * Called for hardware keyboard key events.
     *
     * Intercepts DEL (backspace) and ENTER to route through Rust. Other key
     * events are passed through to the base connection.
     */
    override fun sendKeyEvent(event: KeyEvent?): Boolean {
        if (!isCurrentInputSession()) return true
        if (!editorView.commitExternalTextCompositionBeforeInteractionIfNeeded()) return true
        if (
            event?.action == KeyEvent.ACTION_UP &&
            editorView.hasInvalidatedCompositionReplacementRangeForEditor()
        ) {
            return finishStaleComposingUpdateAfterInvalidation()
        }
        if (
            shouldConsumeInvalidatedCompositionForKeyEvent(event) &&
            consumeInvalidatedCompositionReplacementRangeAndRestore()
        ) {
            return true
        }
        if (!editorView.isEditable && event?.let { editorView.isReadOnlyTextMutationKeyEvent(it) } == true) {
            return true
        }
        if (event != null && editorView.handleCompositionKeyEvent(event) {
                super.sendKeyEvent(event)
            }) {
            return true
        }
        if (event != null && editorView.handleHardwareKeyEvent(event)) {
            return true
        }
        if (event != null && editorView.handlePrintableHardwareKeyEvent(event) {
                super.sendKeyEvent(event)
            }) {
            return true
        }
        return super.sendKeyEvent(event)
    }

    private fun shouldConsumeInvalidatedCompositionForKeyEvent(event: KeyEvent?): Boolean {
        if (event == null || event.action == KeyEvent.ACTION_UP) return false
        return editorView.isReadOnlyTextMutationKeyEvent(event)
    }

    private fun isNoOpSurroundingDelete(beforeLength: Int, afterLength: Int): Boolean =
        beforeLength <= 0 && afterLength <= 0
}
