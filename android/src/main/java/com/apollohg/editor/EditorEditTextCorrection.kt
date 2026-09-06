package com.apollohg.editor

internal fun EditorEditText.handleCompositionCommitImpl(
    text: String,
    replacementStartUtf16: Int,
    replacementEndUtf16: Int,
    newCursorPosition: Int = 1
) {
    val startedAt = System.nanoTime()
    if (!isEditable) {
        recordImeTraceForTesting("handleCompositionCommitNoop", "reason=notEditable textLength=${text.length}")
        return
    }
    if (isApplyingRustState) {
        recordImeTraceForTesting("handleCompositionCommitNoop", "reason=applyingRust textLength=${text.length}")
        return
    }
    if (!hasLiveEditor()) {
        recordImeTraceForTesting("handleCompositionCommitNoop", "reason=noLiveEditor textLength=${text.length}")
        return
    }

    val authorizedText = lastAuthorizedText
    val (startUtf16, endUtf16) = PositionBridge.snapRangeToScalarBoundaries(
        replacementStartUtf16,
        replacementEndUtf16,
        authorizedText
    )
    if (isCollapsedAtomBoundarySelection(startUtf16, endUtf16)) {
        recordImeTraceForTesting("handleCompositionCommitNoop", "reason=atomBoundary textLength=${text.length}")
        return
    }
    val scalarStart = PositionBridge.utf16ToScalar(startUtf16, authorizedText)
    val scalarEnd = PositionBridge.utf16ToScalar(endUtf16, authorizedText)

    if (
        startUtf16 <= endUtf16 &&
        endUtf16 <= authorizedText.length &&
        authorizedText.substring(startUtf16, endUtf16) == text
    ) {
        val requestedCursor = requestedCursorScalar(
            scalarStart,
            scalarEnd,
            authorizedText,
            text,
            newCursorPosition
        ) ?: scalarEnd
        recordImeTraceForTesting(
            "handleCompositionCommitNoop",
            "reason=alreadyAuthorized textLength=${text.length} requestedCursor=$requestedCursor range=$startUtf16..$endUtf16"
        )
        restoreAuthorizedTextIfNeeded()
        applyRequestedCursorScalar(requestedCursor)
        return
    }

    if (text == "\n") {
        recordImeTraceForTesting(
            "handleCompositionCommit",
            "route=return textLength=${text.length} utf16Range=$startUtf16..$endUtf16 scalarRange=$scalarStart..$scalarEnd"
        )
        if (scalarStart != scalarEnd) {
            deleteAndSplitInRust(scalarStart, scalarEnd)
        } else {
            splitBlockInRust(scalarStart)
        }
        recordImeTraceForTesting(
            "handleCompositionCommitDone",
            "route=return totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
        )
        return
    }

    val requestedCursor = requestedCursorScalar(
        scalarStart,
        scalarEnd,
        authorizedText,
        text,
        newCursorPosition
    )
    recordImeTraceForTesting(
        "handleCompositionCommit",
        "textLength=${text.length} cursor=$newCursorPosition utf16Range=$startUtf16..$endUtf16 scalarRange=$scalarStart..$scalarEnd requestedCursor=$requestedCursor"
    )
    insertPlainTextRangeInRust(
        scalarStart,
        scalarEnd,
        text,
        requestedCursorScalar = requestedCursor
    )
    recordImeTraceForTesting(
        "handleCompositionCommitDone",
        "textLength=${text.length} totalUs=${nanosToMicros(System.nanoTime() - startedAt)}"
    )
}

internal fun EditorEditText.handleCorrectionCommitImpl(
    startUtf16: Int,
    endUtf16: Int,
    renderedOldText: String,
    newText: String
): Boolean {
    if (!isEditable) return true
    if (isApplyingRustState) return true
    if (!hasLiveEditor()) return false

    val authorizedText = lastAuthorizedText
    if (startUtf16 < 0 || endUtf16 < startUtf16) {
        recordImeTraceForTesting(
            "correctionExplicitNoop",
            "reason=invalidRange range=$startUtf16..$endUtf16 oldLength=${renderedOldText.length} newLength=${newText.length}"
        )
        return false
    }
    if (endUtf16 > authorizedText.length) {
        recordImeTraceForTesting(
            "correctionExplicitNoop",
            "reason=outOfBounds range=$startUtf16..$endUtf16 authorizedLength=${authorizedText.length}"
        )
        return false
    }
    if (authorizedText.substring(startUtf16, endUtf16) != renderedOldText) {
        recordImeTraceForTesting(
            "correctionExplicitNoop",
            "reason=staleText range=$startUtf16..$endUtf16 oldLength=${renderedOldText.length} newLength=${newText.length}"
        )
        return false
    }

    val (snappedStartUtf16, snappedEndUtf16) = PositionBridge.snapRangeToScalarBoundaries(
        startUtf16,
        endUtf16,
        authorizedText
    )
    if (
        snappedStartUtf16 != startUtf16 ||
        snappedEndUtf16 != endUtf16 ||
        snappedStartUtf16 > snappedEndUtf16
    ) {
        recordImeTraceForTesting(
            "correctionExplicitNoop",
            "reason=unsnappedScalarBoundary range=$startUtf16..$endUtf16 snapped=$snappedStartUtf16..$snappedEndUtf16"
        )
        return false
    }

    val scalarStart = PositionBridge.utf16ToScalar(snappedStartUtf16, authorizedText)
    val scalarEnd = PositionBridge.utf16ToScalar(snappedEndUtf16, authorizedText)
    recordImeTraceForTesting(
        "correctionExplicitApply",
        "range=$scalarStart..$scalarEnd newLength=${newText.length}"
    )
    insertPlainTextRangeInRust(scalarStart, scalarEnd, newText)
    return true
}

internal fun EditorEditText.handleMissingOldTextCorrectionCommitImpl(
    startUtf16: Int,
    endUtf16: Int,
    renderedOldText: String,
    newText: String
): Boolean {
    if (!isEditable) return true
    if (isApplyingRustState) return true
    if (!hasLiveEditor()) return false

    val authorizedText = lastAuthorizedText
    if (
        startUtf16 < 0 ||
        endUtf16 <= startUtf16 ||
        endUtf16 > authorizedText.length ||
        authorizedText.substring(startUtf16, endUtf16) != renderedOldText
    ) {
        recordImeTraceForTesting(
            "correctionInferredNoop",
            "reason=staleRange range=$startUtf16..$endUtf16 newLength=${newText.length}"
        )
        return false
    }

    val (snappedStartUtf16, snappedEndUtf16) = PositionBridge.snapRangeToScalarBoundaries(
        startUtf16,
        endUtf16,
        authorizedText
    )
    if (snappedStartUtf16 >= snappedEndUtf16) {
        recordImeTraceForTesting(
            "correctionInferredNoop",
            "reason=emptySnappedRange token=$startUtf16..$endUtf16 snapped=$snappedStartUtf16..$snappedEndUtf16"
        )
        return false
    }

    val scalarStart = PositionBridge.utf16ToScalar(snappedStartUtf16, authorizedText)
    val scalarEnd = PositionBridge.utf16ToScalar(snappedEndUtf16, authorizedText)
    recordImeTraceForTesting(
        "correctionInferredApply",
        "range=$scalarStart..$scalarEnd utf16=$snappedStartUtf16..$snappedEndUtf16 newLength=${newText.length}"
    )
    insertPlainTextRangeInRust(scalarStart, scalarEnd, newText)
    return true
}

internal fun EditorEditText.missingOldTextCorrectionTokenRangeForEditorImpl(
    text: String,
    offsetUtf16: Int
): Pair<Int, Int>? {
    if (offsetUtf16 < 0 || offsetUtf16 >= text.length) return null

    val tokenOffset = PositionBridge.snapToScalarBoundary(
        offsetUtf16,
        text,
        biasForward = false
    )
    if (tokenOffset < 0 || tokenOffset >= text.length) return null
    if (!isMissingOldTextCorrectionTokenCodePointAt(text, tokenOffset)) return null

    var startUtf16 = tokenOffset
    while (startUtf16 > 0) {
        val previousUtf16 = Character.offsetByCodePoints(text, startUtf16, -1)
        if (!isMissingOldTextCorrectionTokenCodePointAt(text, previousUtf16)) break
        startUtf16 = previousUtf16
    }

    var endUtf16 = tokenOffset + Character.charCount(Character.codePointAt(text, tokenOffset))
    while (endUtf16 < text.length) {
        if (!isMissingOldTextCorrectionTokenCodePointAt(text, endUtf16)) break
        endUtf16 += Character.charCount(Character.codePointAt(text, endUtf16))
    }

    return if (startUtf16 < endUtf16) startUtf16 to endUtf16 else null
}

internal fun EditorEditText.isMissingOldTextCorrectionTokenCodePointAt(text: String, utf16Offset: Int): Boolean {
    if (utf16Offset < 0 || utf16Offset >= text.length) return false
    val codePoint = Character.codePointAt(text, utf16Offset)
    if (isMissingOldTextCorrectionCoreTokenCodePoint(codePoint)) return true
    if (!isMissingOldTextCorrectionJoinerCodePoint(codePoint)) return false

    val previousCodePoint = previousCodePointBefore(text, utf16Offset) ?: return false
    val nextUtf16Offset = utf16Offset + Character.charCount(codePoint)
    val nextCodePoint = nextCodePointAt(text, nextUtf16Offset) ?: return false
    return isMissingOldTextCorrectionCoreTokenCodePoint(previousCodePoint) &&
        isMissingOldTextCorrectionCoreTokenCodePoint(nextCodePoint)
}

internal fun EditorEditText.isMissingOldTextCorrectionCoreTokenCodePoint(codePoint: Int): Boolean {
    if (Character.isLetterOrDigit(codePoint)) return true
    return when (Character.getType(codePoint)) {
        Character.NON_SPACING_MARK.toInt(),
        Character.COMBINING_SPACING_MARK.toInt(),
        Character.ENCLOSING_MARK.toInt(),
        Character.CONNECTOR_PUNCTUATION.toInt(),
        Character.MATH_SYMBOL.toInt(),
        Character.CURRENCY_SYMBOL.toInt(),
        Character.MODIFIER_SYMBOL.toInt(),
        Character.OTHER_SYMBOL.toInt(),
        Character.SURROGATE.toInt() -> true
        else -> false
    }
}

internal fun EditorEditText.isMissingOldTextCorrectionJoinerCodePoint(codePoint: Int): Boolean =
    codePoint == '\''.code ||
        codePoint == 0x2018 ||
        codePoint == 0x2019 ||
        codePoint == 0x201B ||
        codePoint == 0xFF07 ||
        codePoint == '-'.code ||
        codePoint == 0x2010 ||
        codePoint == 0x2011 ||
        codePoint == 0x2012 ||
        codePoint == 0x2013 ||
        codePoint == 0x2014 ||
        codePoint == 0x2212 ||
        codePoint == 0x200D

internal fun EditorEditText.previousCodePointBefore(text: String, utf16Offset: Int): Int? {
    if (utf16Offset <= 0 || utf16Offset > text.length) return null
    val previousUtf16 = Character.offsetByCodePoints(text, utf16Offset, -1)
    return Character.codePointAt(text, previousUtf16)
}

internal fun EditorEditText.nextCodePointAt(text: String, utf16Offset: Int): Int? {
    if (utf16Offset < 0 || utf16Offset >= text.length) return null
    return Character.codePointAt(text, utf16Offset)
}
