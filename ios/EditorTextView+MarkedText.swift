import UIKit
import os

extension EditorTextView {
    func captureMarkedTextReplacementRangeIfNeeded() {
        guard markedTextReplacementScalarRange == nil else { return }

        guard let selectedRange = selectedTextRange else {
            let scalarPos = PositionBridge.cursorScalarOffset(in: self)
            let utf16Pos = PositionBridge.scalarToUtf16Offset(scalarPos, in: lastAuthorizedText)
            markedTextReplacementScalarRange = (from: scalarPos, to: scalarPos)
            markedTextReplacementUtf16Range = NSRange(
                location: utf16Pos,
                length: 0
            )
            return
        }

        let scalarRange = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
        let startUtf16 = offset(from: beginningOfDocument, to: selectedRange.start)
        let endUtf16 = offset(from: beginningOfDocument, to: selectedRange.end)

        markedTextReplacementScalarRange = (from: scalarRange.from, to: scalarRange.to)
        markedTextReplacementUtf16Range = NSRange(
            location: min(startUtf16, endUtf16),
            length: abs(endUtf16 - startUtf16)
        )
    }

    func trackedMarkedTextReplacementRange() -> (from: UInt32, to: UInt32)? {
        if let markedTextReplacementScalarRange {
            return markedTextReplacementScalarRange
        }
        guard let selectedRange = selectedTextRange else { return nil }
        let scalarRange = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
        return (from: scalarRange.from, to: scalarRange.to)
    }

    func clearMarkedTextTracking() {
        markedTextReplacementScalarRange = nil
        markedTextReplacementUtf16Range = nil
        markedTextCompositionText = nil
        markedTextCompositionIsExplicitlyEmpty = false
        isComposing = false
    }

    func finishTransientMarkedTextMutation() {
        performTransientTextMutation {
            super.unmarkText()
        }
        clearMarkedTextTracking()
        onExternalUpdateReadinessMayChange?()
    }

    func performTransientTextMutation(_ action: () -> Void) {
        let wasApplyingRustState = isApplyingRustState
        isApplyingRustState = true
        action()
        isApplyingRustState = wasApplyingRustState
    }

    func currentMarkedTextForCommit() -> String? {
        if markedTextCompositionIsExplicitlyEmpty { return "" }
        return transientMarkedTextFromAuthorizedDiff()
            ?? markedTextRange.flatMap { text(in: $0) }
            ?? markedTextCompositionText
    }

    func validatedTrackedMarkedTextForCommit() -> String? {
        guard markedTextReplacementScalarRange != nil || markedTextReplacementUtf16Range != nil else {
            return nil
        }
        if markedTextCompositionIsExplicitlyEmpty { return "" }
        return transientMarkedTextFromAuthorizedDiff()
            ?? markedTextRange.flatMap { text(in: $0) }
    }

    func refreshMarkedTextCompositionText(fallback: String? = nil) {
        if fallback?.isEmpty == true {
            markedTextCompositionText = ""
            markedTextCompositionIsExplicitlyEmpty = true
            return
        }
        markedTextCompositionIsExplicitlyEmpty = false
        markedTextCompositionText = transientMarkedTextFromAuthorizedDiff()
            ?? markedTextRange.flatMap { text(in: $0) }
            ?? fallback
    }

    private func transientMarkedTextFromAuthorizedDiff() -> String? {
        guard let replacementRange = markedTextReplacementUtf16Range else { return nil }

        let currentText = textStorage.string as NSString
        let authorizedText = lastAuthorizedText as NSString
        let replacementEnd = replacementRange.location + replacementRange.length
        guard replacementRange.location >= 0,
              replacementEnd <= authorizedText.length
        else {
            return nil
        }

        let insertedLength = currentText.length - (authorizedText.length - replacementRange.length)
        guard insertedLength >= 0,
              replacementRange.location + insertedLength <= currentText.length
        else {
            return nil
        }

        if replacementRange.location > 0 {
            let prefixRange = NSRange(location: 0, length: replacementRange.location)
            guard currentText.substring(with: prefixRange) == authorizedText.substring(with: prefixRange) else {
                return nil
            }
        }

        let insertedEnd = replacementRange.location + insertedLength
        let authorizedSuffixLength = authorizedText.length - replacementEnd
        let currentSuffixLength = currentText.length - insertedEnd
        guard authorizedSuffixLength == currentSuffixLength else { return nil }
        if authorizedSuffixLength > 0 {
            let authorizedSuffixRange = NSRange(location: replacementEnd, length: authorizedSuffixLength)
            let currentSuffixRange = NSRange(location: insertedEnd, length: currentSuffixLength)
            guard currentText.substring(with: currentSuffixRange)
                == authorizedText.substring(with: authorizedSuffixRange)
            else {
                return nil
            }
        }

        return currentText.substring(
            with: NSRange(location: replacementRange.location, length: insertedLength)
        )
    }

    func commitMarkedText(
        _ text: String,
        replacementRange: (from: UInt32, to: UInt32)?
    ) -> String? {
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else {
            return nil
        }
        var adoptedUpdateJSON: String?
        performInterceptedInput(flushPendingNativeTextMutation: false) {
            if let replacementRange {
                if replacementRange.from == replacementRange.to {
                    adoptedUpdateJSON = adapter.insertText(text, atScalar: replacementRange.from)
                } else {
                    adoptedUpdateJSON = adapter.replaceTextRange(
                        from: replacementRange.from,
                        to: replacementRange.to,
                        with: text
                    )
                }
            } else {
                adoptedUpdateJSON = adapter.insertText(
                    text,
                    atScalar: PositionBridge.cursorScalarOffset(in: self)
                )
            }
            guard let adoptedUpdateJSON else { return }
            applyUpdateJSON(adoptedUpdateJSON)
        }
        return adoptedUpdateJSON
    }

    func commitMarkedTextWithNativeOutcome(
        _ text: String,
        replacementRange: (from: UInt32, to: UInt32)?
    ) -> EditorV2Adapter.NativeMutationRender? {
        guard editorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: editorId)
        else {
            return nil
        }
        var commit: EditorV2Adapter.NativeMutationRender?
        performInterceptedInput(flushPendingNativeTextMutation: false) {
            if let replacementRange {
                if replacementRange.from == replacementRange.to {
                    commit = adapter.insertTextWithNativeOutcome(
                        text,
                        atScalar: replacementRange.from
                    )
                } else {
                    commit = adapter.replaceTextRangeWithNativeOutcome(
                        from: replacementRange.from,
                        to: replacementRange.to,
                        with: text
                    )
                }
            } else {
                let position = PositionBridge.cursorScalarOffset(in: self)
                commit = adapter.insertTextWithNativeOutcome(text, atScalar: position)
            }
            guard let commit else { return }
            applyUpdateJSON(commit.updateJSON, notifyDelegate: commit.documentChanged)
        }
        return commit
    }

    func commitActiveMarkedTextBeforeReturn() -> Bool {
        guard markedTextReplacementScalarRange != nil || markedTextRange != nil else {
            return true
        }

        if markedTextReplacementScalarRange != nil || markedTextReplacementUtf16Range != nil {
            let composedText = currentMarkedTextForCommit()
            let replacementRange = trackedMarkedTextReplacementRange()
            finishTransientMarkedTextMutation()

            guard shouldCommitMarkedText(composedText, replacementRange: replacementRange) else {
                restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
                return true
            }
            return commitMarkedText(composedText ?? "", replacementRange: replacementRange) != nil
        }

        let mutation = nativeTextMutationFromAuthorizedDiff(currentText: textStorage.string)
        finishTransientMarkedTextMutation()
        guard let mutation else {
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            return true
        }

        switch commitNativeTextMutationIfPossible(
            mutation,
            allowAfterBlur: false,
            allowWhileIntercepting: true
        ) {
        case .committed:
            return true
        case .deferred, .rejected:
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            return false
        }
    }

    func shouldCommitMarkedText(
        _ text: String?,
        replacementRange: (from: UInt32, to: UInt32)?
    ) -> Bool {
        guard let text else { return false }
        if !text.isEmpty { return true }
        guard let replacementRange else { return false }
        return replacementRange.from != replacementRange.to
    }

    func restoreAuthorizedTextAfterCancelledCompositionIfNeeded() {
        guard editorId != 0 else { return }
        guard textStorage.string != lastAuthorizedText else { return }

        let stateJSON = EditorV2Shadow.getCurrentState(id: editorId)
        applyUpdateJSON(stateJSON)
    }

    func previewMarkedTextReplacementRange(
        _ range: (from: UInt32, to: UInt32)?
    ) -> String {
        guard let range else { return "none" }
        let utf16 = markedTextReplacementUtf16Range
            .map { "\($0.location)..<\($0.location + $0.length)" }
            ?? "none"
        return "scalar=\(range.from)..<\(range.to) utf16=\(utf16)"
    }

}
