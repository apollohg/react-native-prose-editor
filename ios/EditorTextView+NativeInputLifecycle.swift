import UIKit
import os

extension EditorTextView {
    enum NativeTextMutationCommitResult {
        case committed(adoptedUpdateJSON: String)
        case deferred
        case rejected
    }

    struct NativeTextMutationDrainResult {
        let ready: Bool
        let adoptedUpdateJSON: String?
    }

    func shouldAdoptNativeTextStorageMutation(
        _ mutation: NativeTextMutation,
        allowAfterBlur: Bool = false
    ) -> Bool {
        if isFirstResponder && isEditable {
            return true
        }
        return allowAfterBlur
            && mutation.capturedAfterBlur
            && mutation.inputGeneration == nativeTextMutationGeneration
            && canAdoptNativeTextMutationAfterBlur()
            && mutation.capturedWhileFirstResponder
            && mutation.capturedWhileEditable
    }

    func canAdoptNativeTextMutationAfterBlur() -> Bool {
        guard let deadline = nativeTextMutationAfterBlurDeadline else {
            return false
        }
        guard nativeTextMutationAfterBlurGeneration == nativeTextMutationGeneration else {
            clearNativeTextMutationAfterBlurWindow()
            return false
        }
        guard ProcessInfo.processInfo.systemUptime <= deadline else {
            clearNativeTextMutationAfterBlurWindow()
            return false
        }
        return true
    }

    func clearNativeTextMutationAfterBlurWindow() {
        nativeTextMutationAfterBlurDeadline = nil
        nativeTextMutationAfterBlurGeneration = nil
    }

    private func advanceNativeTextMutationGeneration() {
        nativeTextMutationGeneration &+= 1
        clearNativeTextMutationAfterBlurWindow()
    }

    func resetPendingNativeTextMutationState() {
        pendingNativeTextMutation = nil
        nativeTextMutationCommitScheduled = false
        advanceNativeTextMutationGeneration()
    }

    @discardableResult
    func cancelExternalTextCompositionForLifecycleIfNeeded() -> String? {
        finishExternalTextComposition(
            cause: "lifecycle",
            finalText: nil,
            cancel: true
        )?.resultJSON
    }

    func discardTransientNativeInputForEditorReset() {
        _ = finishExternalTextComposition(cause: "documentChange", finalText: nil, cancel: true)
        deferredInsertTexts.removeAll()
        deferredInsertDrainScheduled = false
        isReplayingDeferredInsertText = false
        resetPendingNativeTextMutationState()
        clearPendingInputTraitRetry()
        performTransientTextMutation {
            super.unmarkText()
        }
        clearMarkedTextTracking()
    }

    @discardableResult
    func discardTransientNativeInputForEditorRebind() -> String? {
        localTextDragState = .idle
        let externalCompositionResultJSON =
            cancelExternalTextCompositionForLifecycleIfNeeded()
        deferredInsertTexts.removeAll()
        deferredInsertDrainScheduled = false
        isReplayingDeferredInsertText = false
        resetPendingNativeTextMutationState()
        lastAuthorizedSelectedUtf16Range = nil
        lastAuthorizedSelectionIsBackward = false
        logicalSelectionScalarRange = nil
        logicalSelectionUtf16Range = nil
        clearPendingInputTraitRetry()
        markedTextReplacementScalarRange = nil
        markedTextReplacementUtf16Range = nil
        markedTextCompositionText = nil
        markedTextCompositionIsExplicitlyEmpty = false
        isComposing = false
        return externalCompositionResultJSON
    }

    @discardableResult
    func flushPendingNativeTextMutationCommitIfNeeded() -> Bool {
        drainPendingNativeTextMutation(
            allowAfterBlur: false,
            allowWhileIntercepting: true
        ).ready
    }

    struct ExternalEditorUpdatePreparation {
        let ready: Bool
        /// When preflight committed local UIKit state, the adapter already
        /// rendered and adopted this current atomic update. Reuse it instead
        /// of reading Rust again for the same external update operation.
        let adoptedUpdateJSON: String?
    }

    private struct ActiveCompositionPreparation {
        let ready: Bool
        let adoptedUpdateJSON: String?
    }

    @discardableResult
    func prepareForExternalEditorUpdate() -> Bool {
        prepareForExternalEditorUpdateResult().ready
    }

    func prepareForExternalEditorUpdateResult() -> ExternalEditorUpdatePreparation {
        let composition = prepareActiveCompositionForExternalMutation()
        guard composition.ready else {
            return ExternalEditorUpdatePreparation(ready: false, adoptedUpdateJSON: nil)
        }
        let nativeMutation = drainPendingNativeTextMutation(
            allowAfterBlur: true,
            allowWhileIntercepting: true
        )
        return ExternalEditorUpdatePreparation(
            ready: nativeMutation.ready,
            adoptedUpdateJSON: composition.adoptedUpdateJSON ?? nativeMutation.adoptedUpdateJSON
        )
    }

    @discardableResult
    func prepareForExternalEditorCommand() -> (ready: Bool, updateJSON: String?, blockedReason: String?) {
        let previousEditorId = editorId
        let previousAuthorizedText = lastAuthorizedText
        let previousStateJSON = previousEditorId != 0 ? EditorV2Shadow.getCurrentState(id: previousEditorId) : nil
        let preparation = prepareForExternalEditorUpdateResult()
        guard preparation.ready else {
            return (false, nil, "composition")
        }
        guard editorId != 0 else {
            return (true, nil, nil)
        }
        if let adoptedUpdateJSON = preparation.adoptedUpdateJSON {
            return (true, adoptedUpdateJSON, nil)
        }
        let currentStateJSON = EditorV2Shadow.getCurrentState(id: editorId)
        guard lastAuthorizedText != previousAuthorizedText
                || previousEditorId != editorId
                || previousStateJSON != currentStateJSON
        else {
            return (true, nil, nil)
        }
        return (true, currentStateJSON, nil)
    }

    private func prepareActiveCompositionForExternalMutation() -> ActiveCompositionPreparation {
        if externalTextComposition != nil {
            guard let finished = finishExternalTextComposition(
                cause: "documentChange",
                finalText: nil,
                cancel: false
            ) else {
                return ActiveCompositionPreparation(ready: false, adoptedUpdateJSON: nil)
            }
            return ActiveCompositionPreparation(
                ready: finished.succeeded,
                adoptedUpdateJSON: finished.adoptedUpdateJSON
            )
        }
        guard isComposing else {
            return ActiveCompositionPreparation(ready: true, adoptedUpdateJSON: nil)
        }

        let composedText = validatedTrackedMarkedTextForCommit()
        let replacementRange = trackedMarkedTextReplacementRange()
        finishTransientMarkedTextMutation()

        guard shouldCommitMarkedText(composedText, replacementRange: replacementRange) else {
            restoreAuthorizedTextAfterCancelledCompositionIfNeeded()
            return ActiveCompositionPreparation(ready: false, adoptedUpdateJSON: nil)
        }

        return ActiveCompositionPreparation(
            ready: true,
            adoptedUpdateJSON: commitMarkedText(composedText ?? "", replacementRange: replacementRange)
        )
    }

    @discardableResult
    func drainPendingNativeTextMutation(
        allowAfterBlur: Bool,
        allowWhileIntercepting: Bool
    ) -> NativeTextMutationDrainResult {
        if reconciliationWorkScheduled, textStorage.string != lastAuthorizedText {
            let adoptedUpdateJSON = restoreRejectedNativeTextMutation()
            return NativeTextMutationDrainResult(
                ready: textStorage.string == lastAuthorizedText,
                adoptedUpdateJSON: adoptedUpdateJSON
            )
        }
        guard nativeTextMutationCommitScheduled
                || pendingNativeTextMutation != nil
                || (!isComposing && markedTextRange == nil && textStorage.string != lastAuthorizedText)
        else {
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: nil)
        }

        nativeTextMutationCommitScheduled = false
        let currentText = textStorage.string
        let mutation: NativeTextMutation?
        if let pendingNativeTextMutation,
           pendingNativeTextMutation.resultingText == currentText,
           pendingNativeTextMutation.authorizedText == lastAuthorizedText
        {
            mutation = nativeTextMutationWithCurrentSelection(pendingNativeTextMutation)
        } else {
            mutation = nativeTextMutationFromAuthorizedDiff(currentText: currentText)
        }

        guard let mutation else {
            pendingNativeTextMutation = nil
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: nil)
        }

        switch commitNativeTextMutationIfPossible(
            mutation,
            allowAfterBlur: allowAfterBlur,
            allowWhileIntercepting: allowWhileIntercepting
        ) {
        case .committed(let adoptedUpdateJSON):
            pendingNativeTextMutation = nil
            return NativeTextMutationDrainResult(ready: true, adoptedUpdateJSON: adoptedUpdateJSON)
        case .rejected:
            pendingNativeTextMutation = nil
            let adoptedUpdateJSON = restoreRejectedNativeTextMutation()
            return NativeTextMutationDrainResult(
                ready: textStorage.string == lastAuthorizedText,
                adoptedUpdateJSON: adoptedUpdateJSON
            )
        case .deferred:
            pendingNativeTextMutation = mutation
            return NativeTextMutationDrainResult(ready: false, adoptedUpdateJSON: nil)
        }
    }

    private func restoreRejectedNativeTextMutation() -> String? {
        guard editorId != 0, textStorage.string != lastAuthorizedText else { return nil }
        reconciliationWorkScheduled = false
        let updateJSON = EditorV2Shadow.getCurrentState(id: editorId)
        return applyUpdateJSON(updateJSON) ? updateJSON : nil
    }

    func scheduleNativeTextMutationCommit(_ mutation: NativeTextMutation) {
        pendingNativeTextMutation = mutation
        guard !nativeTextMutationCommitScheduled else { return }

        nativeTextMutationCommitScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            _ = self.drainPendingNativeTextMutation(
                allowAfterBlur: true,
                allowWhileIntercepting: true
            )
        }
    }

    @discardableResult
    func commitNativeTextMutationIfPossible(
        _ mutation: NativeTextMutation,
        allowAfterBlur: Bool,
        allowWhileIntercepting: Bool
    ) -> NativeTextMutationCommitResult {
        guard editorId != 0 else {
            return .rejected
        }

        guard !isApplyingRustState,
              (!isInterceptingInput || allowWhileIntercepting),
              !isComposing
        else {
            return .deferred
        }

        guard shouldAdoptNativeTextStorageMutation(mutation, allowAfterBlur: allowAfterBlur) else {
            if textStorage.string != lastAuthorizedText {
                scheduleReconciliationFromRust()
            }
            return .rejected
        }

        guard textStorage.string == mutation.resultingText else {
            if let refreshedMutation = nativeTextMutationFromAuthorizedDiff(currentText: textStorage.string) {
                return commitNativeTextMutationIfPossible(
                    refreshedMutation,
                    allowAfterBlur: allowAfterBlur,
                    allowWhileIntercepting: allowWhileIntercepting
                )
            }
            return .rejected
        }

        guard mutation.authorizedText == lastAuthorizedText else {
            if let refreshedMutation = nativeTextMutationFromAuthorizedDiff(currentText: textStorage.string) {
                return commitNativeTextMutationIfPossible(
                    refreshedMutation,
                    allowAfterBlur: allowAfterBlur,
                    allowWhileIntercepting: allowWhileIntercepting
                )
            }
            return .rejected
        }

        var adoptedUpdateJSON: String?
        performInterceptedInput(flushPendingNativeTextMutation: false) {
            guard let adapter = EditorV2Registry.adapter(forLegacyId: editorId) else { return }
            let postSelection: (anchor: UInt32, head: UInt32)?
            if let anchor = mutation.selectionAnchor,
               let head = mutation.selectionHead {
                postSelection = (anchor: anchor, head: head)
            } else {
                postSelection = nil
            }
            adoptedUpdateJSON = adapter.commitNativeTextMutation(
                from: mutation.from,
                to: mutation.to,
                with: mutation.replacementText,
                postSelection: postSelection
            )
            guard let adoptedUpdateJSON else { return }
            applyUpdateJSON(adoptedUpdateJSON)
            notifyDelegateOfAuthoritativeTextSelection()
        }
        guard let adoptedUpdateJSON else { return .rejected }
        if mutation.capturedAfterBlur {
            clearNativeTextMutationAfterBlurWindow()
        }
        return .committed(adoptedUpdateJSON: adoptedUpdateJSON)
    }

    private func notifyDelegateOfAuthoritativeTextSelection() {
        guard editorId != 0,
              let selection = logicalSelectionScalarRange
        else {
            return
        }
        let docAnchor = EditorV2Shadow.scalarToDoc(id: editorId, scalar: selection.anchor)
        let docHead = EditorV2Shadow.scalarToDoc(id: editorId, scalar: selection.head)
        editorDelegate?.editorTextView(self, selectionDidChange: docAnchor, head: docHead)
    }

}
