import UIKit
import os

extension EditorTextView {
    @objc func handleIndentKeyCommand() {
        handleListDepthKeyCommand(outdent: false)
    }

    @objc func handleHardBreakKeyCommand() {
        guard !isApplyingRustState, editorId != 0, isEditable else { return }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        guard !isCollapsedAtomBoundary(selectedUtf16Range()) else { return }
        performInterceptedInput {
            insertNodeInRust(preferredHardBreakNodeType())
        }
    }

    @objc func handleOutdentKeyCommand() {
        handleListDepthKeyCommand(outdent: true)
    }

    func adjacentVoidBlockDeleteRangeForBackwardDelete(
        cursorUtf16Offset: Int,
        cursorScalar: UInt32
    ) -> (from: UInt32, to: UInt32)? {
        guard cursorUtf16Offset >= 0, cursorUtf16Offset < textStorage.length else {
            return nil
        }
        let attrs = textStorage.attributes(at: cursorUtf16Offset, effectiveRange: nil)
        guard attrs[.attachment] is NSTextAttachment,
              attrs[RenderBridgeAttributes.voidNodeType] as? String != nil,
              cursorScalar < UInt32.max
        else {
            return nil
        }
        return (from: cursorScalar, to: cursorScalar + 1)
    }

    func trailingVoidBlockDeleteRangeForBackwardDelete(
        cursorUtf16Offset: Int
    ) -> (from: UInt32, to: UInt32)? {
        let text = textStorage.string as NSString
        guard text.length > 0 else { return nil }

        let clampedCursor = min(max(cursorUtf16Offset, 0), text.length)
        let paragraphProbe = min(max(clampedCursor - 1, 0), text.length - 1)
        let paragraphRange = text.paragraphRange(for: NSRange(location: paragraphProbe, length: 0))

        let placeholderRange = NSRange(location: paragraphRange.location, length: 1)
        guard placeholderRange.location + placeholderRange.length <= text.length else {
            return nil
        }

        let paragraphText = text.substring(with: placeholderRange)
        guard paragraphText == "\u{200B}" else { return nil }
        guard paragraphRange.location >= 2 else { return nil }
        guard text.character(at: paragraphRange.location - 1) == 0x000A else { return nil }

        let attachmentIndex = paragraphRange.location - 2
        guard
            let deleteRange = scalarDeleteRangeForVoidAttachment(at: attachmentIndex)
        else {
            return nil
        }

        return deleteRange
    }

    private func scalarDeleteRangeForVoidAttachment(
        at utf16Offset: Int
    ) -> (from: UInt32, to: UInt32)? {
        guard utf16Offset >= 0, utf16Offset < textStorage.length else {
            return nil
        }
        let attrs = textStorage.attributes(at: utf16Offset, effectiveRange: nil)
        guard let attachment = attrs[.attachment] as? NSTextAttachment,
              !(attachment is AtomBlockAttachment),
              attrs[RenderBridgeAttributes.voidNodeType] as? String != nil
        else {
            return nil
        }

        let attachmentEndScalar = PositionBridge.utf16OffsetToScalar(
            utf16Offset + 1,
            in: self
        )
        guard attachmentEndScalar > 0 else { return nil }
        return (from: attachmentEndScalar - 1, to: attachmentEndScalar)
    }

    private func handleListDepthKeyCommand(outdent: Bool) {
        guard !isApplyingRustState else { return }
        guard editorId != 0 else { return }
        guard isEditable else { return }
        guard finishExternalTextCompositionBeforeInteractionIfNeeded() else { return }
        guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        guard isCaretInsideList() else { return }
        guard let selection = currentScalarSelection() else { return }

        performInterceptedInput {
            let updateJSON = outdent
                ? EditorV2Shadow.outdentListItemAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
                : EditorV2Shadow.indentListItemAtSelectionScalar(
                    id: editorId,
                    scalarAnchor: selection.anchor,
                    scalarHead: selection.head
                )
            applyUpdateJSON(updateJSON)
        }
    }

    private func isCaretInsideList() -> Bool {
        guard editorId != 0 else { return false }
        guard
            let data = EditorV2Shadow.getCurrentState(id: editorId).data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let activeState = object["activeState"] as? [String: Any],
            let nodes = activeState["nodes"] as? [String: Any]
        else {
            return false
        }

        return nodes.contains { nodeType, value in
            EditorNodeTypes.isListContainer(nodeType) && value as? Bool == true
        }
    }

    private func preferredHardBreakNodeType() -> String {
        guard
            let data = EditorV2Shadow.getCurrentState(id: editorId).data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let activeState = object["activeState"] as? [String: Any],
            let insertableNodes = activeState["insertableNodes"] as? [String]
        else {
            return "hardBreak"
        }

        return EditorNodeTypes.preferredHardBreak(in: Set(insertableNodes))
    }

    func ensureInternalTextViewDelegate() {
        // Some keyboard integrations replace UITextView's private delegate ivar
        // directly. The editor must own delegate callbacks so external observers
        // cannot inspect transient TextKit state during Rust-driven edits.
        // The delegate is a dedicated object rather than the text view itself;
        // see EditorTextViewInternalDelegate for why (APOLLO-REACT-56).
        guard (delegate as AnyObject?) !== internalTextViewDelegate else { return }
        delegate = internalTextViewDelegate
    }

    func performInterceptedInput(
        flushPendingNativeTextMutation: Bool = true,
        _ action: () -> Void
    ) {
        if flushPendingNativeTextMutation, interceptedInputDepth == 0 {
            guard flushPendingNativeTextMutationCommitIfNeeded() else { return }
        }
        interceptedInputDepth += 1
        Self.inputLog.debug(
            "[intercept.begin] depth=\(self.interceptedInputDepth) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
        )
        action()
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.interceptedInputDepth = max(0, self.interceptedInputDepth - 1)
            Self.inputLog.debug(
                "[intercept.end] depth=\(self.interceptedInputDepth) selection=\(self.selectionSummary(), privacy: .public) textState=\(self.textSnapshotSummary(), privacy: .public)"
            )
            if self.interceptedInputDepth == 0 {
                _ = self.drainPendingNativeTextMutation(
                    allowAfterBlur: false,
                    allowWhileIntercepting: false
                )
                self.drainDeferredInsertTextIfReady()
            }
        }
    }

    func enqueueDeferredInsertText(_ text: String) {
        deferredInsertTexts.append(text)
        scheduleDeferredInsertDrain()
    }

    private func scheduleDeferredInsertDrain() {
        guard !deferredInsertDrainScheduled else { return }
        deferredInsertDrainScheduled = true
        DispatchQueue.main.async { [weak self] in
            self?.drainDeferredInsertTextIfReady()
        }
    }

    private func drainDeferredInsertTextIfReady() {
        deferredInsertDrainScheduled = false
        guard !deferredInsertTexts.isEmpty else { return }
        guard editorId != 0 else {
            deferredInsertTexts.removeAll()
            return
        }
        guard !isApplyingRustState,
              interceptedInputDepth == 0,
              pendingNativeTextMutation == nil,
              !nativeTextMutationCommitScheduled
        else {
            scheduleDeferredInsertDrain()
            return
        }

        let text = deferredInsertTexts.removeFirst()
        isReplayingDeferredInsertText = true
        defer { isReplayingDeferredInsertText = false }
        insertText(text)
        if !deferredInsertTexts.isEmpty {
            scheduleDeferredInsertDrain()
        }
    }

    /// Handle return key press as a block split operation.
    private func handleReturnKey() {
        // If there's a range selection, atomically delete and split.
        if let selectedRange = selectedTextRange, !selectedRange.isEmpty {
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            let updateJSON = EditorV2Shadow.deleteAndSplitScalar(
                id: editorId,
                scalarFrom: range.from,
                scalarTo: range.to
            )
            applyUpdateJSON(updateJSON)
        } else {
            let scalarPos = PositionBridge.cursorScalarOffset(in: self)
            splitBlockInRust(at: scalarPos)
        }
    }

    func interceptReturnInput(
        _ text: String,
        replacing replacementRange: UITextRange? = nil
    ) -> Bool {
        guard text == "\n" || text == "\r" else { return false }
        let scalarRange = replacementRange.map {
            PositionBridge.textRangeToScalarRange($0, in: self)
        }
        guard commitActiveMarkedTextBeforeReturn() else { return true }
        performInterceptedInput {
            if let scalarRange {
                if scalarRange.from == scalarRange.to {
                    splitBlockInRust(at: scalarRange.from)
                } else {
                    let updateJSON = EditorV2Shadow.deleteAndSplitScalar(
                        id: editorId,
                        scalarFrom: scalarRange.from,
                        scalarTo: scalarRange.to
                    )
                    applyUpdateJSON(updateJSON)
                }
            } else {
                handleReturnKey()
            }
        }
        return true
    }

    /// Split a block at a scalar position via the Rust editor.
    private func splitBlockInRust(at scalarPos: UInt32) {
        Self.inputLog.debug(
            "[rust.splitBlockScalar] scalarPos=\(scalarPos) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.splitBlockScalar(id: editorId, scalarPos: scalarPos)
        applyUpdateJSON(updateJSON)
    }

    /// Paste HTML content through Rust.
    @discardableResult
    func pasteHTML(_ html: String, detectContentChange: Bool = false) -> Bool {
        let previousHTML = detectContentChange ? EditorV2Shadow.getHtml(id: editorId) : nil
        syncCurrentUIKitSelectionToRust()
        Self.inputLog.debug(
            "[rust.pasteHTML] html=\(self.preview(html), privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
        )
        let updateJSON = EditorV2Shadow.insertContentHtml(id: editorId, html: html)
        applyUpdateJSON(updateJSON)
        guard let previousHTML else { return true }
        return EditorV2Shadow.getHtml(id: editorId) != previousHTML
    }

    private func syncCurrentUIKitSelectionToRust() {
        guard editorId != 0, let range = selectedTextRange else { return }
        let anchor = PositionBridge.textViewToScalar(range.start, in: self)
        let head = PositionBridge.textViewToScalar(range.end, in: self)
        EditorV2Shadow.setSelectionScalar(id: editorId, scalarAnchor: anchor, scalarHead: head)
    }

    /// Paste plain text through Rust.
    func pastePlainText(_ text: String) {
        if let selectedRange = selectedTextRange, !selectedRange.isEmpty {
            // Atomically replace the selection with the pasted text.
            let range = PositionBridge.textRangeToScalarRange(selectedRange, in: self)
            Self.inputLog.debug(
                "[rust.pastePlainText.replace] text=\(self.preview(text), privacy: .public) scalar=\(range.from)-\(range.to) selection=\(self.selectionSummary(), privacy: .public)"
            )
            let updateJSON = EditorV2Shadow.replaceTextScalar(
                id: editorId,
                scalarFrom: range.from,
                scalarTo: range.to,
                text: text
            )
            applyUpdateJSON(updateJSON)
        } else {
            Self.inputLog.debug(
                "[rust.pastePlainText.insert] text=\(self.preview(text), privacy: .public) selection=\(self.selectionSummary(), privacy: .public)"
            )
            insertTextInRust(text, at: PositionBridge.cursorScalarOffset(in: self))
        }
    }

}
