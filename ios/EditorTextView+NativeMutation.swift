import UIKit
import os

extension EditorTextView {
    struct NativeTextMutation {
        let from: UInt32
        let to: UInt32
        /// The replaced span as UTF-16 in the authorized storage. Kept so a
        /// caret can be converted against that storage patched with this
        /// replacement, which is the only ruler that still carries the
        /// document's structural attributes.
        let authorizedReplacementUtf16Range: NSRange
        let replacementText: String
        let resultingText: String
        let authorizedText: String
        let selectionAnchor: UInt32?
        let selectionHead: UInt32?
        let authorizedSelectionUtf16Range: NSRange?
        let rawSelectionUtf16Range: NSRange?
        let acceptedSpaceCaretUtf16Offset: Int?
        let selectionRevision: UInt64
        let capturedWhileFirstResponder: Bool
        let capturedWhileEditable: Bool
        let capturedAfterBlur: Bool
        let inputGeneration: UInt64
    }

    /// The last Rust-authorized storage with one native replacement applied.
    ///
    /// A caret inside a natively mutated document cannot be converted against
    /// the live text view. A keyboard correction replaces the word with an
    /// unattributed string, which strips the structural attributes the
    /// utf16→scalar conversion reads to count a list's block openings, so the
    /// live view maps the caret as though the line were never wrapped. The
    /// authorized storage still carries those attributes; patching its text
    /// with the same replacement gives a ruler that matches the new text
    /// while keeping the structure. Falls back to the unpatched storage when
    /// the range no longer addresses it.
    private func authorizedStorageApplying(
        _ replacementText: String,
        inUtf16Range range: NSRange
    ) -> NSAttributedString {
        guard range.location >= 0,
              range.length >= 0,
              NSMaxRange(range) <= lastAuthorizedAttributedTextStorage.length
        else {
            return lastAuthorizedAttributedTextStorage
        }
        let patched = NSMutableAttributedString(
            attributedString: lastAuthorizedAttributedTextStorage
        )
        // The string overload inherits the replaced run's attributes, which is
        // exactly what keeps the structure the keyboard's replacement dropped.
        patched.replaceCharacters(in: range, with: replacementText)
        return patched
    }

    func nativeTextMutationFromAuthorizedDiff(
        currentText: String
    ) -> NativeTextMutation? {
        let authorizedText = lastAuthorizedText
        guard currentText != authorizedText else { return nil }

        let authorized = authorizedText as NSString
        let current = currentText as NSString
        let sharedLength = min(authorized.length, current.length)
        var prefix = 0
        while prefix < sharedLength,
              authorized.character(at: prefix) == current.character(at: prefix) {
            prefix += 1
        }
        prefix = sharedUtf16ScalarBoundary(atOrBefore: prefix, in: authorizedText, and: currentText)

        var authorizedEnd = authorized.length
        var currentEnd = current.length
        while authorizedEnd > prefix,
              currentEnd > prefix,
              authorized.character(at: authorizedEnd - 1) == current.character(at: currentEnd - 1) {
            authorizedEnd -= 1
            currentEnd -= 1
        }
        authorizedEnd = utf16ScalarBoundary(atOrAfter: authorizedEnd, in: authorizedText)
        currentEnd = utf16ScalarBoundary(atOrAfter: currentEnd, in: currentText)

        let replacementLength = currentEnd - prefix
        guard replacementLength >= 0 else { return nil }
        let rawReplacementText = current.substring(
            with: NSRange(location: prefix, length: replacementLength)
        )

        let rawSelectionUtf16Range = selectedUtf16Range()
        let authorizedSelectionUtf16Range = lastAuthorizedSelectedUtf16Range
        let preservesAcceptedSpace = shouldPreserveAcceptedAutocorrectSpace(
            authorizedText: authorized,
            replacementStartUtf16: prefix,
            authorizedEndUtf16: authorizedEnd,
            replacementText: rawReplacementText,
            rawSelectionUtf16Range: rawSelectionUtf16Range,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range,
            acceptedCaretUtf16Offset: currentEnd
        )
        let replacementText = preservesAcceptedSpace
            ? rawReplacementText + " "
            : rawReplacementText
        let selectionRangeForMapping = preservesAcceptedSpace
            ? NSRange(location: currentEnd + 1, length: 0)
            : rawSelectionUtf16Range
        let targetSelectionUtf16Range = targetSelectionUtf16RangeForNativeTextMutation(
            rawSelectionUtf16Range: selectionRangeForMapping,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range,
            replacementStartUtf16: prefix,
            authorizedEndUtf16: authorizedEnd,
            currentEndUtf16: currentEnd + (preservesAcceptedSpace ? 1 : 0),
            currentTextUtf16Length: current.length + (preservesAcceptedSpace ? 1 : 0)
        )
        let mappedAuthorizedSelectionUtf16Range = targetSelectionUtf16RangeForNativeTextMutation(
            rawSelectionUtf16Range: authorizedSelectionUtf16Range,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range,
            replacementStartUtf16: prefix,
            authorizedEndUtf16: authorizedEnd,
            currentEndUtf16: currentEnd + (preservesAcceptedSpace ? 1 : 0),
            currentTextUtf16Length: current.length + (preservesAcceptedSpace ? 1 : 0)
        )
        let authorizedReplacementUtf16Range = NSRange(
            location: prefix,
            length: authorizedEnd - prefix
        )
        let selectedScalarRange = targetSelectionUtf16Range.map { range in
            scalarRange(
                forUtf16Range: range,
                in: authorizedStorageApplying(
                    replacementText,
                    inUtf16Range: authorizedReplacementUtf16Range
                )
            )
        }
        let preservesAuthorizedSelectionDirection = lastAuthorizedSelectionIsBackward
            && targetSelectionUtf16Range.map { targetSelection in
                mappedAuthorizedSelectionUtf16Range.map {
                    NSEqualRanges(targetSelection, $0)
                } ?? false
            } == true
        let capturedAfterBlur = canAdoptNativeTextMutationAfterBlur()

        return NativeTextMutation(
            from: PositionBridge.utf16OffsetToScalar(prefix, in: lastAuthorizedAttributedTextStorage),
            to: PositionBridge.utf16OffsetToScalar(authorizedEnd, in: lastAuthorizedAttributedTextStorage),
            authorizedReplacementUtf16Range: authorizedReplacementUtf16Range,
            replacementText: replacementText,
            resultingText: currentText,
            authorizedText: authorizedText,
            selectionAnchor: preservesAuthorizedSelectionDirection
                ? selectedScalarRange?.to
                : selectedScalarRange?.from,
            selectionHead: preservesAuthorizedSelectionDirection
                ? selectedScalarRange?.from
                : selectedScalarRange?.to,
            authorizedSelectionUtf16Range: authorizedSelectionUtf16Range,
            rawSelectionUtf16Range: rawSelectionUtf16Range,
            acceptedSpaceCaretUtf16Offset: preservesAcceptedSpace ? currentEnd : nil,
            selectionRevision: selectionRevision,
            capturedWhileFirstResponder: isFirstResponder || capturedAfterBlur,
            capturedWhileEditable: isEditable,
            capturedAfterBlur: capturedAfterBlur,
            inputGeneration: nativeTextMutationGeneration
        )
    }

    func shouldPreserveAcceptedAutocorrectSpace(
        authorizedText: NSString,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        replacementText: String,
        rawSelectionUtf16Range: NSRange?,
        authorizedSelectionUtf16Range: NSRange?,
        acceptedCaretUtf16Offset: Int
    ) -> Bool {
        guard replacementStartUtf16 >= 0,
              authorizedEndUtf16 > replacementStartUtf16,
              authorizedEndUtf16 <= authorizedText.length,
              authorizedText.character(at: authorizedEndUtf16 - 1) == 0x20,
              !replacementText.isEmpty,
              let replacementLastCharacter = replacementText.last,
              replacementLastCharacter.isLetter || replacementLastCharacter.isNumber,
              let rawSelectionUtf16Range,
              rawSelectionUtf16Range.length == 0,
              rawSelectionUtf16Range.location == acceptedCaretUtf16Offset,
              let authorizedSelectionUtf16Range,
              authorizedSelectionUtf16Range.length == 0,
              authorizedSelectionUtf16Range.location == authorizedEndUtf16
        else {
            return false
        }

        let correctedText = authorizedText.substring(
            with: NSRange(
                location: replacementStartUtf16,
                length: authorizedEndUtf16 - replacementStartUtf16 - 1
            )
        )
        guard !correctedText.isEmpty,
              correctedText != replacementText,
              let correctedLastCharacter = correctedText.last
        else {
            return false
        }
        return correctedLastCharacter.isLetter || correctedLastCharacter.isNumber
    }

    func nativeTextMutationWithCurrentSelection(
        _ mutation: NativeTextMutation
    ) -> NativeTextMutation {
        let currentSelectionUtf16Range = selectedUtf16Range()
        let didSelectionChangeAfterCapture = selectionRevision != mutation.selectionRevision
        let didCurrentRangeMoveAfterCapture: Bool
        if let currentSelectionUtf16Range,
           let rawSelectionUtf16Range = mutation.rawSelectionUtf16Range {
            didCurrentRangeMoveAfterCapture = !NSEqualRanges(
                currentSelectionUtf16Range,
                rawSelectionUtf16Range
            )
        } else {
            didCurrentRangeMoveAfterCapture = false
        }
        let currentSelectionDiffersFromAuthorized: Bool
        if let currentSelectionUtf16Range,
           let authorizedSelectionUtf16Range = mutation.authorizedSelectionUtf16Range {
            currentSelectionDiffersFromAuthorized = !NSEqualRanges(
                currentSelectionUtf16Range,
                authorizedSelectionUtf16Range
            )
        } else {
            currentSelectionDiffersFromAuthorized = currentSelectionUtf16Range != nil
        }
        let repeatsCapturedTransientBeginningSelection: Bool
        if let currentSelectionUtf16Range,
           let rawSelectionUtf16Range = mutation.rawSelectionUtf16Range,
           let authorizedSelectionUtf16Range = mutation.authorizedSelectionUtf16Range,
           NSEqualRanges(currentSelectionUtf16Range, rawSelectionUtf16Range) {
            repeatsCapturedTransientBeginningSelection =
                isTransientBeginningSelectionDuringNativeReplacement(
                    currentSelectionUtf16Range,
                    authorizedSelection: authorizedSelectionUtf16Range,
                    replacementStartUtf16: mutation.authorizedReplacementUtf16Range.location,
                    authorizedEndUtf16: NSMaxRange(mutation.authorizedReplacementUtf16Range),
                    currentEndUtf16: mutation.authorizedReplacementUtf16Range.location
                        + mutation.replacementText.utf16.count
                )
        } else {
            repeatsCapturedTransientBeginningSelection = false
        }
        let shouldUseCurrentSelection = currentSelectionUtf16Range != nil
            && !repeatsCapturedTransientBeginningSelection
            && (
                (didSelectionChangeAfterCapture && currentSelectionDiffersFromAuthorized)
                    || didCurrentRangeMoveAfterCapture
                    || mutation.rawSelectionUtf16Range == nil
            )
        // Same ruler as the capture path: the live view has lost the
        // structural attributes wherever the keyboard replaced a run, so a
        // caret converted against it maps as though the block were never
        // wrapped.
        let selectionConversionStorage = authorizedStorageApplying(
            mutation.replacementText,
            inUtf16Range: mutation.authorizedReplacementUtf16Range
        )
        let selectionRangeForConversion: NSRange?
        if let currentSelectionUtf16Range,
           currentSelectionUtf16Range.length == 0,
           currentSelectionUtf16Range.location == mutation.acceptedSpaceCaretUtf16Offset {
            selectionRangeForConversion = NSRange(
                location: currentSelectionUtf16Range.location + 1,
                length: 0
            )
        } else {
            selectionRangeForConversion = currentSelectionUtf16Range
        }
        let selectedScalarRange = shouldUseCurrentSelection
            ? selectionRangeForConversion.map {
                scalarRange(forUtf16Range: $0, in: selectionConversionStorage)
            }
            : nil
        let preservesCapturedSelectionDirection = selectedScalarRange.map { selectedRange in
            guard let anchor = mutation.selectionAnchor,
                  let head = mutation.selectionHead,
                  anchor > head
            else {
                return false
            }
            return selectedRange.from == head && selectedRange.to == anchor
        } ?? false
        return NativeTextMutation(
            from: mutation.from,
            to: mutation.to,
            authorizedReplacementUtf16Range: mutation.authorizedReplacementUtf16Range,
            replacementText: mutation.replacementText,
            resultingText: mutation.resultingText,
            authorizedText: mutation.authorizedText,
            selectionAnchor: preservesCapturedSelectionDirection
                ? selectedScalarRange?.to
                : selectedScalarRange?.from ?? mutation.selectionAnchor,
            selectionHead: preservesCapturedSelectionDirection
                ? selectedScalarRange?.from
                : selectedScalarRange?.to ?? mutation.selectionHead,
            authorizedSelectionUtf16Range: mutation.authorizedSelectionUtf16Range,
            rawSelectionUtf16Range: shouldUseCurrentSelection
                ? currentSelectionUtf16Range
                : mutation.rawSelectionUtf16Range,
            acceptedSpaceCaretUtf16Offset: mutation.acceptedSpaceCaretUtf16Offset,
            selectionRevision: shouldUseCurrentSelection
                ? selectionRevision
                : mutation.selectionRevision,
            capturedWhileFirstResponder: mutation.capturedWhileFirstResponder,
            capturedWhileEditable: mutation.capturedWhileEditable,
            capturedAfterBlur: mutation.capturedAfterBlur,
            inputGeneration: mutation.inputGeneration
        )
    }

    private func targetSelectionUtf16RangeForNativeTextMutation(
        rawSelectionUtf16Range: NSRange?,
        authorizedSelectionUtf16Range: NSRange?,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int,
        currentTextUtf16Length: Int
    ) -> NSRange? {
        guard let authorizedSelection = authorizedSelectionUtf16Range else {
            return clampedUtf16Range(rawSelectionUtf16Range, length: currentTextUtf16Length)
        }
        guard authorizedSelection.location != NSNotFound else {
            return clampedUtf16Range(rawSelectionUtf16Range, length: currentTextUtf16Length)
        }

        if let rawSelection = rawSelectionUtf16Range,
           rawSelection.location != NSNotFound,
           !isTransientBeginningSelectionDuringNativeReplacement(
               rawSelection,
               authorizedSelection: authorizedSelection,
               replacementStartUtf16: replacementStartUtf16,
               authorizedEndUtf16: authorizedEndUtf16,
               currentEndUtf16: currentEndUtf16
           ),
           !NSEqualRanges(rawSelection, authorizedSelection) {
            return clampedUtf16Range(rawSelection, length: currentTextUtf16Length)
        }

        if authorizedSelection.length == 0 {
            let mappedOffset = mapCollapsedAuthorizedSelectionOffsetThroughNativeTextMutation(
                authorizedSelection.location,
                replacementStartUtf16: replacementStartUtf16,
                authorizedEndUtf16: authorizedEndUtf16,
                currentEndUtf16: currentEndUtf16
            )
            let clampedOffset = min(max(mappedOffset, 0), currentTextUtf16Length)
            return NSRange(location: clampedOffset, length: 0)
        }

        let mappedStart = mapAuthorizedSelectionOffsetThroughNativeTextMutation(
            authorizedSelection.location,
            replacementStartUtf16: replacementStartUtf16,
            authorizedEndUtf16: authorizedEndUtf16,
            currentEndUtf16: currentEndUtf16,
            isRangeStart: true
        )
        let mappedEnd = mapAuthorizedSelectionOffsetThroughNativeTextMutation(
            NSMaxRange(authorizedSelection),
            replacementStartUtf16: replacementStartUtf16,
            authorizedEndUtf16: authorizedEndUtf16,
            currentEndUtf16: currentEndUtf16,
            isRangeStart: false
        )
        let start = min(mappedStart, mappedEnd)
        let end = max(mappedStart, mappedEnd)
        let clampedStart = min(max(start, 0), currentTextUtf16Length)
        let clampedEnd = min(max(end, 0), currentTextUtf16Length)
        return NSRange(location: clampedStart, length: max(0, clampedEnd - clampedStart))
    }

    private func isTransientBeginningSelectionDuringNativeReplacement(
        _ rawSelection: NSRange,
        authorizedSelection: NSRange,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int
    ) -> Bool {
        rawSelection.location == 0
            && rawSelection.length == 0
            && authorizedSelection.location > 0
            && authorizedSelection.length == 0
            && authorizedEndUtf16 > replacementStartUtf16
            && currentEndUtf16 > replacementStartUtf16
    }

    private func clampedUtf16Range(_ range: NSRange?, length: Int) -> NSRange? {
        guard let range, range.location != NSNotFound else { return nil }
        let start = min(max(range.location, 0), length)
        let end = min(max(NSMaxRange(range), 0), length)
        return NSRange(location: min(start, end), length: abs(end - start))
    }

    private func mapCollapsedAuthorizedSelectionOffsetThroughNativeTextMutation(
        _ offset: Int,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int
    ) -> Int {
        // UIKit can leave a stale caret at the insertion point during autocomplete.
        // A collapsed authorized caret should stay collapsed after the inserted text.
        if replacementStartUtf16 == authorizedEndUtf16,
           offset == replacementStartUtf16,
           currentEndUtf16 > replacementStartUtf16 {
            return currentEndUtf16
        }
        if offset <= replacementStartUtf16 {
            return offset
        }
        if offset < authorizedEndUtf16 {
            return currentEndUtf16
        }
        return offset + currentEndUtf16 - authorizedEndUtf16
    }

    private func mapAuthorizedSelectionOffsetThroughNativeTextMutation(
        _ offset: Int,
        replacementStartUtf16: Int,
        authorizedEndUtf16: Int,
        currentEndUtf16: Int,
        isRangeStart: Bool
    ) -> Int {
        if offset <= replacementStartUtf16 {
            return offset
        }
        if offset >= authorizedEndUtf16 {
            return offset + currentEndUtf16 - authorizedEndUtf16
        }
        return isRangeStart ? replacementStartUtf16 : currentEndUtf16
    }

    func isUtf16ScalarBoundary(_ offset: Int, in text: String) -> Bool {
        guard offset >= 0, offset <= text.utf16.count else { return false }
        let utf16Index = text.utf16.index(text.utf16.startIndex, offsetBy: offset)
        return String.Index(utf16Index, within: text) != nil
    }

    private func utf16ScalarBoundary(atOrBefore offset: Int, in text: String) -> Int {
        var candidate = min(max(offset, 0), text.utf16.count)
        while candidate > 0, !isUtf16ScalarBoundary(candidate, in: text) {
            candidate -= 1
        }
        return candidate
    }

    private func utf16ScalarBoundary(atOrAfter offset: Int, in text: String) -> Int {
        var candidate = min(max(offset, 0), text.utf16.count)
        while candidate < text.utf16.count, !isUtf16ScalarBoundary(candidate, in: text) {
            candidate += 1
        }
        return candidate
    }

    func sharedUtf16ScalarBoundary(atOrBefore offset: Int, in lhs: String, and rhs: String) -> Int {
        var candidate = min(max(offset, 0), lhs.utf16.count, rhs.utf16.count)
        while candidate > 0,
              (!isUtf16ScalarBoundary(candidate, in: lhs) || !isUtf16ScalarBoundary(candidate, in: rhs)) {
            candidate -= 1
        }
        return candidate
    }

}
