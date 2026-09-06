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
import android.view.inputmethod.ExtractedText
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.SurroundingText
import android.view.inputmethod.TextAttribute
import android.view.inputmethod.TextSnapshot

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
    internal val editorView: EditorEditText,
    baseConnection: InputConnection,
    internal val boundEditorId: Long,
    internal val boundGeneration: Long,
    internal val boundMapperGeneration: Long,
) : InputConnectionWrapper(baseConnection, true) {
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

        internal const val DUPLICATE_CORRECTION_COMMIT_WINDOW_MS = 1_000L

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

    internal var closedForInput = false
    internal var extractedTextRequest: ExtractedTextRequest? = null
    internal var lastPublishedExtractedText: ExtractedText? = null
    internal var lastPublishedSelection: List<Int>? = null

    override fun commitText(text: CharSequence, newCursorPosition: Int, textAttribute: TextAttribute?): Boolean =
        commitText(text, newCursorPosition)

    override fun setComposingText(text: CharSequence, newCursorPosition: Int, textAttribute: TextAttribute?): Boolean =
        setComposingText(text, newCursorPosition)

    override fun setComposingRegion(start: Int, end: Int, textAttribute: TextAttribute?): Boolean =
        setComposingRegion(start, end)

    override fun replaceText(start: Int, end: Int, text: CharSequence, newCursorPosition: Int, textAttribute: TextAttribute?): Boolean {
        if (!isCurrentInputSessionFor("replaceText") || !editorView.isEditable) return true
        if (start < 0 || end < 0) return false
        val requestedText = currentMapper()?.visibleText?.toString() ?: return false
        if (!editorView.prepareForExternalEditorUpdate()) return true
        if (!isCurrentInputSession() || currentMapper()?.visibleText?.toString() != requestedText) return true
        val range = rawRangeForIme(start.coerceAtMost(requestedText.length), end.coerceAtMost(requestedText.length)) ?: return false
        val raw = editorView.editableText.toString()
        val normalized = if (range.first == range.second) {
            PositionBridge.snapToScalarBoundary(range.first, raw, biasForward = true).let { it to it }
        } else PositionBridge.snapRangeToScalarBoundaries(range.first, range.second, raw)
        pendingDuplicateCorrectionCommit = null
        pendingCompositionCorrectionCommit = null
        editorView.runWithTransientInputMutationGuard { super.setSelection(normalized.first, normalized.second) }
        commitTextToEditor(text.toString(), newCursorPosition)
        return true
    }

    override fun getExtractedText(request: ExtractedTextRequest?, flags: Int): ExtractedText? =
        extractedTextForIme(request, flags)

    override fun getSurroundingText(beforeLength: Int, afterLength: Int, flags: Int): SurroundingText? =
        surroundingTextForIme(beforeLength, afterLength, flags)

    override fun takeSnapshot(): TextSnapshot? = snapshotForIme()

    override fun beginBatchEdit(): Boolean = isCurrentInputSession() && super.beginBatchEdit()

    override fun performContextMenuAction(id: Int): Boolean =
        isCurrentInputSession() && super.performContextMenuAction(id)

    override fun performEditorAction(actionCode: Int): Boolean =
        isCurrentInputSession() && editorView.isEditable && super.performEditorAction(actionCode)

    override fun requestCursorUpdates(cursorUpdateMode: Int): Boolean =
        isCurrentInputSession() && super.requestCursorUpdates(cursorUpdateMode)

    override fun requestCursorUpdates(cursorUpdateMode: Int, cursorUpdateFilter: Int): Boolean =
        isCurrentInputSession() && super.requestCursorUpdates(cursorUpdateMode, cursorUpdateFilter)

    override fun closeConnection() {
        if (closedForInput) return
        if (isCurrentInputSession()) finishComposingText()
        closedForInput = true
        extractedTextRequest = null
        lastPublishedExtractedText = null
        super.closeConnection()
    }

    internal var pendingDuplicateCorrectionCommit: PendingDuplicateCorrectionCommit? = null
    internal var pendingCompositionCorrectionCommit: PendingCompositionCorrectionCommit? = null
    internal var pendingCompositionCorrectionGeneration: Long = 0L
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

    internal fun commitTextToEditor(committedText: String?, newCursorPosition: Int) {
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
