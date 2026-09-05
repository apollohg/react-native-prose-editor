import UIKit

// MARK: - EditorTextView + NSTextStorageDelegate (Reconciliation Fallback)

extension EditorTextView: NSTextStorageDelegate {

    /// Detect unauthorized text storage mutations after UIKit finishes
    /// processing an editing operation. If the text storage diverges from
    /// the last Rust-authorized content and the change was NOT initiated by
    /// our Rust apply path, re-render from Rust ("Rust wins").
    func textStorage(
        _ textStorage: NSTextStorage,
        didProcessEditing editedMask: NSTextStorage.EditActions,
        range editedRange: NSRange,
        changeInLength delta: Int
    ) {
        // Only care about actual character edits, not attribute-only changes.
        guard editedMask.contains(.editedCharacters) else { return }
        // Skip if this change came from our own Rust apply path, transient IME
        // composition, or an inline prediction. iOS inline predictions (iOS 17+)
        // mutate textStorage directly and set markedTextRange without calling
        // setMarkedText, so isComposing remains false — check markedTextRange too.
        guard !isApplyingRustState, !isComposing, markedTextRange == nil else { return }

        if case let .awaitingUIKitCleanup(_, _, cleanupRanges) = localTextDragState,
           delta < 0,
           cleanupRanges.contains(where: {
               $0.location == editedRange.location && $0.length == -delta
           })
        {
            pendingNativeTextMutation = nil
            nativeTextMutationCommitScheduled = false
            restoreAfterLocalTextDragCleanup()
            return
        }

        // Skip if no editor is bound yet (nothing to reconcile against).
        guard editorId != 0 else { return }

        PositionBridge.invalidateCache(for: self)

        // Compare current text storage content against last authorized snapshot.
        let currentText = textStorage.string
        guard currentText != lastAuthorizedText else { return }
        currentTopLevelChildMetadata = nil

        let allowAfterBlur = canAdoptNativeTextMutationAfterBlur()
        if let mutation = nativeTextMutationFromAuthorizedDiff(currentText: currentText),
           isInterceptingInput
                || shouldAdoptNativeTextStorageMutation(
                    mutation,
                    allowAfterBlur: allowAfterBlur
                )
        {
            scheduleNativeTextMutationCommit(mutation)
            return
        }

        let authorizedPreview = preview(lastAuthorizedText)
        let storagePreview = preview(currentText)

        reconciliationCount += 1

        Self.reconciliationLog.warning(
            """
            [NativeEditor:reconciliation] Text storage diverged from Rust state \
            (count: \(self.reconciliationCount), \
            delta: \(delta), \
            editedRange: \(editedRange.location)..<\(editedRange.location + editedRange.length), \
            authorizedLen: \(self.lastAuthorizedText.count), \
            storageLen: \(currentText.count), \
            selection: \(self.selectionSummary(), privacy: .public), \
            interceptedDepth: \(self.interceptedInputDepth), \
            composing: \(self.isComposing), \
            authorizedPreview: \(authorizedPreview, privacy: .public), \
            storagePreview: \(storagePreview, privacy: .public))
            """
        )

        scheduleReconciliationFromRust()
    }

    func scheduleReconciliationFromRust() {
        guard !reconciliationWorkScheduled else { return }
        reconciliationWorkScheduled = true

        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.reconciliationWorkScheduled = false

            guard !self.isApplyingRustState, !self.isInterceptingInput, !self.isComposing else { return }
            guard self.editorId != 0 else { return }
            guard self.textStorage.string != self.lastAuthorizedText else { return }

            // Reconcile by pulling the current editor state without rebuilding
            // the Rust backend or clearing history. This must run after the
            // current NSTextStorage edit transaction has finished.
            let stateJSON = EditorV2Shadow.getCurrentState(id: self.editorId)
            self.applyUpdateJSON(stateJSON)
        }
    }
}
