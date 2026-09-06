import UIKit
import os

extension EditorTextView {
    func selectedUtf16Range() -> NSRange? {
        guard let range = selectedTextRange else { return nil }
        let location = offset(from: beginningOfDocument, to: range.start)
        let length = offset(from: range.start, to: range.end)
        guard location >= 0, length >= 0 else { return nil }
        return NSRange(location: location, length: length)
    }

    func isAtomBoundaryCaretOffset(_ offset: Int) -> Bool {
        guard offset >= 0, offset <= textStorage.length else { return false }
        if offset < textStorage.length,
           textStorage.attribute(.attachment, at: offset, effectiveRange: nil) is AtomBlockAttachment
        {
            return true
        }
        return offset > 0
            && textStorage.attribute(.attachment, at: offset - 1, effectiveRange: nil)
                is AtomBlockAttachment
    }

    func isCollapsedAtomBoundary(_ range: NSRange?) -> Bool {
        guard let range, range.length == 0 else { return false }
        return isAtomBoundaryCaretOffset(range.location)
    }

    @discardableResult
    private func restoreSelectionFromAtomBoundaryIfNeeded() -> Bool {
        guard isCollapsedAtomBoundary(selectedUtf16Range()) else { return false }
        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        if let authorized = lastAuthorizedSelectedUtf16Range,
           NSMaxRange(authorized) <= textStorage.length,
           !isCollapsedAtomBoundary(authorized)
        {
            performTransientTextMutation {
                selectedRange = authorized
                noteSelectionDidChange()
            }
        }
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
        return true
    }

    func noteSelectionDidChange() {
        selectionRevision &+= 1
    }

    func recordAuthorizedSelectionIfPossible() {
        guard editorId != 0 else {
            lastAuthorizedSelectedUtf16Range = nil
            lastAuthorizedSelectionIsBackward = false
            return
        }
        let currentText = textStorage.string
        guard currentText.utf16.count == lastAuthorizedTextStorage.length,
              currentText == lastAuthorizedText
        else {
            return
        }
        lastAuthorizedSelectedUtf16Range = selectedUtf16Range()
        lastAuthorizedSelectionIsBackward = currentLogicalScalarSelection().map {
            $0.anchor > $0.head
        } ?? false
    }

    func scalarRange(forUtf16Range range: NSRange) -> (from: UInt32, to: UInt32) {
        let start = PositionBridge.utf16OffsetToScalar(range.location, in: self)
        let end = PositionBridge.utf16OffsetToScalar(NSMaxRange(range), in: self)
        return (from: min(start, end), to: max(start, end))
    }

    func scalarRange(
        forUtf16Range range: NSRange,
        in storage: NSAttributedString
    ) -> (from: UInt32, to: UInt32) {
        let start = PositionBridge.utf16OffsetToScalar(range.location, in: storage)
        let end = PositionBridge.utf16OffsetToScalar(NSMaxRange(range), in: storage)
        return (from: min(start, end), to: max(start, end))
    }

    /// UITextViewDelegate hook for user-driven selection updates.
    ///
    /// Using the delegate callback is more reliable than observing
    /// `selectedTextRange` directly because UIKit can adjust selection
    /// internally during tap handling and word-boundary resolution.
    func textViewDidChangeSelection(_ textView: UITextView) {
        guard textView === self else { return }
        ensureInternalTextViewDelegate()
        noteSelectionDidChange()
        if externalTextComposition != nil {
            guard !isApplyingRustState else { return }
            let interactionSelection = selectedRange
            guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
            if interactionSelection.location != NSNotFound,
               interactionSelection.location >= 0,
               interactionSelection.length >= 0,
               interactionSelection.location + interactionSelection.length <= textStorage.length
            {
                logicalSelectionScalarRange = nil
                logicalSelectionUtf16Range = nil
                performTransientTextMutation {
                    selectedRange = interactionSelection
                    noteSelectionDidChange()
                }
            }
        }
        guard !isApplyingRustState,
              !isComposing,
              !nativeTextMutationCommitScheduled,
              pendingNativeTextMutation == nil
        else {
            return
        }
        if restoreSelectionFromAtomBoundaryIfNeeded() {
            return
        }
        if normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded() {
            return
        }
        recordAuthorizedSelectionIfPossible()
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
        scheduleSelectionSync()
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith URL: URL,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        return false
    }

    func textView(
        _ textView: UITextView,
        shouldInteractWith textAttachment: NSTextAttachment,
        in characterRange: NSRange,
        interaction: UITextItemInteraction
    ) -> Bool {
        guard textView === self,
              characterRange.location >= 0,
              characterRange.location < textStorage.length
        else {
            return false
        }

        let attrs = textStorage.attributes(at: characterRange.location, effectiveRange: nil)
        guard (attrs[RenderBridgeAttributes.voidNodeType] as? String) == "image",
              let start = position(from: beginningOfDocument, offset: characterRange.location),
              let end = position(from: start, offset: characterRange.length)
        else {
            return false
        }

        selectedTextRange = textRange(from: start, to: end)
        noteSelectionDidChange()
        refreshNativeSelectionChromeVisibility()
        onSelectionOrContentMayChange?()
        scheduleSelectionSync()
        return false
    }

    func refreshTypingAttributesForSelection() {
        guard let range = selectedTextRange else {
            typingAttributes = defaultTypingAttributes()
            return
        }

        if textStorage.length == 0 {
            typingAttributes = defaultTypingAttributes()
            return
        }

        let startOffset = offset(from: beginningOfDocument, to: range.start)
        let attributeIndex: Int
        if startOffset < textStorage.length {
            attributeIndex = max(0, startOffset)
        } else {
            attributeIndex = textStorage.length - 1
        }

        var attrs = textStorage.attributes(at: attributeIndex, effectiveRange: nil)
        attrs[.font] = attrs[.font] ?? resolvedDefaultFont()
        attrs[.foregroundColor] = attrs[.foregroundColor] ?? resolvedDefaultTextColor()
        typingAttributes = attrs
    }

    private func setNativeSelectionChromeHidden(_ hidden: Bool) {
        guard hidesNativeSelectionChrome != hidden else { return }
        hidesNativeSelectionChrome = hidden
        super.tintColor = hidden ? .clear : visibleSelectionTintColor
    }

    func refreshNativeSelectionChromeVisibility() {
        let hidden = selectedImageSelectionState() != nil
        if !hidden, tintColor.cgColor.alpha > 0 {
            visibleSelectionTintColor = tintColor
        }
        setNativeSelectionChromeHidden(hidden)
    }

    private func showNativeSelectionChromeIfNeeded() {
        if tintColor.cgColor.alpha > 0 {
            visibleSelectionTintColor = tintColor
        }
        setNativeSelectionChromeHidden(false)
    }

    func refreshSelectionVisualState() {
        _ = normalizeSelectionForEmptyBlockAutocapitalizationIfNeeded()
        refreshNativeSelectionChromeVisibility()
        refreshTypingAttributesForSelection()
        onSelectionOrContentMayChange?()
    }

    func scheduleSelectionSync() {
        pendingSelectionSyncGeneration &+= 1
        let generation = pendingSelectionSyncGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard self.pendingSelectionSyncGeneration == generation else { return }
            self.syncSelectionToRustAndNotifyDelegate()
        }
    }

    private func syncSelectionToRustAndNotifyDelegate() {
        guard !isApplyingRustState,
              !isComposing,
              !nativeTextMutationCommitScheduled,
              pendingNativeTextMutation == nil,
              editorId != 0
        else {
            return
        }
        guard let selection = currentLogicalScalarSelection() else { return }

        let anchor = selection.anchor
        let head = selection.head
        let sync: EditorV2SelectionSync?
        if let image = selectedImageSelectionState() {
            sync = EditorV2Shadow.setNodeSelection(id: editorId, docPos: image.docPos)
        } else {
            sync = EditorV2Shadow.setSelectionScalar(
                id: editorId,
                scalarAnchor: anchor,
                scalarHead: head
            )
        }
        guard let sync else { return }
        Self.selectionLog.debug(
            "[textViewDidChangeSelection] scalar=\(anchor)-\(head) doc=\(sync.docAnchor)-\(sync.docHead) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )
        if let refreshed = sync.refreshedUpdateJSON {
            applyUpdateJSON(refreshed, notifyDelegate: false)
        }
        recordAuthorizedSelectionIfPossible()
        refreshTypingAttributesForSelection()
        editorDelegate?.editorTextView(
            self,
            selectionDidChange: sync.docAnchor,
            head: sync.docHead
        )
    }

    func currentLogicalScalarSelection() -> (anchor: UInt32, head: UInt32)? {
        guard let range = selectedTextRange else { return nil }
        let scalarRange = PositionBridge.textRangeToScalarRange(range, in: self)
        if let logicalSelectionScalarRange,
           min(logicalSelectionScalarRange.anchor, logicalSelectionScalarRange.head) == scalarRange.from,
           max(logicalSelectionScalarRange.anchor, logicalSelectionScalarRange.head) == scalarRange.to
        {
            return logicalSelectionScalarRange
        }
        // A lone empty block parks the caret ahead of the placeholder so UIKit
        // offers autocapitalization, which makes the UIKit caret disagree with
        // the engine scalar by exactly the placeholder. Matching the range that
        // nudge produced keeps the engine authoritative.
        //
        // Confined to that block: a matching UTF-16 range is not evidence a
        // scalar is current. Wrapping a line in a list shifts every scalar
        // while leaving UTF-16 offsets untouched, so a stale pre-wrap caret
        // would match here and place the next character inside the word.
        if let logicalSelectionScalarRange,
           let logicalSelectionUtf16Range,
           logicalSelectionUtf16Range == selectedRange,
           isLoneEmptyPlaceholderBlock
        {
            return logicalSelectionScalarRange
        }
        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        return (anchor: scalarRange.from, head: scalarRange.to)
    }

    func currentScalarSelection() -> (anchor: UInt32, head: UInt32)? {
        currentLogicalScalarSelection()
    }

    /// Apply a selection from a parsed JSON selection object.
    ///
    /// The selection JSON matches the format from `serialize_editor_update`:
    /// ```json
    /// {"type": "text", "anchor": 5, "head": 5}
    /// {"type": "node", "pos": 10}
    /// {"type": "all"}
    /// ```
    func applySelectionFromJSON(_ selection: [String: Any]) -> SelectionApplyTrace {
        guard let type = selection["type"] as? String else {
            return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
        }

        let totalStartedAt = DispatchTime.now().uptimeNanoseconds
        isApplyingRustState = true
        delegate = nil
        defer {
            ensureInternalTextViewDelegate()
            isApplyingRustState = false
        }

        switch type {
        case "text":
            let resolveStartedAt = DispatchTime.now().uptimeNanoseconds
            guard let anchor = v2ExactUInt32(selection["anchor"] as? NSNumber),
                  let head = v2ExactUInt32(selection["head"] as? NSNumber)
            else {
                return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
            }
            // anchor/head from Rust are document positions; convert to scalar offsets first.
            let anchorScalar: UInt32
            if let rawAnchorScalar = selection["anchorScalar"] {
                guard let exactAnchorScalar = v2ExactUInt32(rawAnchorScalar as? NSNumber) else {
                    return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
                }
                anchorScalar = exactAnchorScalar
            } else {
                anchorScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: anchor)
            }
            let headScalar: UInt32
            if let rawHeadScalar = selection["headScalar"] {
                guard let exactHeadScalar = v2ExactUInt32(rawHeadScalar as? NSNumber) else {
                    return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
                }
                headScalar = exactHeadScalar
            } else {
                headScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: head)
            }
            let startUtf16 = PositionBridge.scalarToUtf16Offset(
                min(anchorScalar, headScalar),
                in: self
            )
            let endUtf16 = PositionBridge.scalarToUtf16Offset(
                max(anchorScalar, headScalar),
                in: self
            )
            let resolveNanos = DispatchTime.now().uptimeNanoseconds - resolveStartedAt

            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            logicalSelectionScalarRange = (anchor: anchorScalar, head: headScalar)
            if anchorScalar == headScalar {
                let endPos = position(from: beginningOfDocument, offset: endUtf16) ?? endOfDocument
                if let adjustedPosition = autocapitalizationFriendlyEmptyBlockPosition(for: endPos) {
                    let adjustedOffset = offset(from: beginningOfDocument, to: adjustedPosition)
                    let adjustedRange = NSRange(location: adjustedOffset, length: 0)
                    if selectedRange != adjustedRange {
                        selectedRange = adjustedRange
                        noteSelectionDidChange()
                    }
                } else {
                    let targetRange = NSRange(location: endUtf16, length: 0)
                    if selectedRange != targetRange {
                        selectedRange = targetRange
                        noteSelectionDidChange()
                    }
                }
            } else {
                let targetRange = NSRange(location: startUtf16, length: endUtf16 - startUtf16)
                if selectedRange != targetRange {
                    selectedRange = targetRange
                    noteSelectionDidChange()
                }
            }
            logicalSelectionUtf16Range = selectedRange
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            showNativeSelectionChromeIfNeeded()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.text] doc=\(anchor)-\(head) scalar=\(anchorScalar)-\(headScalar) final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: resolveNanos,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        case "node":
            // Node selection: select the object replacement character at that position.
            let resolveStartedAt = DispatchTime.now().uptimeNanoseconds
            guard let pos = v2ExactUInt32(selection["pos"] as? NSNumber) else {
                return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
            }
            // pos from Rust is a document position; convert to scalar offset.
            let posScalar: UInt32
            if let rawPosScalar = selection["posScalar"] {
                guard let exactPosScalar = v2ExactUInt32(rawPosScalar as? NSNumber) else {
                    return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
                }
                posScalar = exactPosScalar
            } else {
                posScalar = EditorV2Shadow.docToScalar(id: editorId, docPos: pos)
            }
            let startUtf16 = PositionBridge.scalarToUtf16Offset(posScalar, in: self)
            let targetRange = NSRange(location: startUtf16, length: 1)
            let resolveNanos = DispatchTime.now().uptimeNanoseconds - resolveStartedAt
            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            logicalSelectionScalarRange = nil
            logicalSelectionUtf16Range = nil
            if selectedRange != targetRange {
                selectedRange = targetRange
                noteSelectionDidChange()
            }
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            refreshNativeSelectionChromeVisibility()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.node] doc=\(pos) scalar=\(posScalar) final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: resolveNanos,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        case "all":
            let assignmentStartedAt = DispatchTime.now().uptimeNanoseconds
            logicalSelectionScalarRange = nil
            logicalSelectionUtf16Range = nil
            selectedTextRange = textRange(from: beginningOfDocument, to: endOfDocument)
            noteSelectionDidChange()
            let assignmentNanos = DispatchTime.now().uptimeNanoseconds - assignmentStartedAt
            let chromeStartedAt = DispatchTime.now().uptimeNanoseconds
            showNativeSelectionChromeIfNeeded()
            let chromeNanos = DispatchTime.now().uptimeNanoseconds - chromeStartedAt
            Self.selectionLog.debug(
                "[applySelectionFromJSON.all] final=\(self.selectionSummary(), privacy: .public)"
            )
            return SelectionApplyTrace(
                totalNanos: DispatchTime.now().uptimeNanoseconds - totalStartedAt,
                resolveNanos: 0,
                assignmentNanos: assignmentNanos,
                chromeNanos: chromeNanos
            )

        default:
            return SelectionApplyTrace(totalNanos: 0, resolveNanos: 0, assignmentNanos: 0, chromeNanos: 0)
        }
    }

    private func autocapitalizationFriendlyEmptyBlockPosition(
        for position: UITextPosition
    ) -> UITextPosition? {
        guard isLoneEmptyPlaceholderBlock else { return nil }

        let utf16Offset = offset(from: beginningOfDocument, to: position)
        guard utf16Offset == textStorage.length else { return nil }
        return beginningOfDocument
    }

}
