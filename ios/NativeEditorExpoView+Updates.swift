import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func setPendingEditorUpdateJson(_ editorUpdateJson: String?) {
        lastEditorUpdateJSONProp = editorUpdateJson
        pendingEditorUpdateJSON = editorUpdateJson
        if editorUpdateJson == nil {
            pendingEditorUpdateEditorId = nil
        }
    }

    func setPendingEditorUpdateEditorId(_ editorUpdateEditorId: String?) {
        guard let editorUpdateEditorId,
              let canonicalEditorId = v2CanonicalUInt64String(editorUpdateEditorId),
              canonicalEditorId != "0"
        else {
            pendingEditorUpdateEditorId = nil
            return
        }
        pendingEditorUpdateEditorId = canonicalEditorId
    }

    func setPendingEditorUpdateRevision(_ editorUpdateRevision: Int) {
        if editorUpdateRevision != 0, pendingEditorUpdateJSON == nil {
            pendingEditorUpdateJSON = lastEditorUpdateJSONProp
        }
        pendingEditorUpdateRevision = editorUpdateRevision
    }

    func applyPendingEditorUpdateIfNeeded() {
        guard pendingEditorUpdateRevision != 0 else { return }
        guard pendingEditorUpdateRevision != appliedEditorUpdateRevision else { return }
        let pendingRevision = pendingEditorUpdateRevision
        guard let updateJSON = pendingEditorUpdateJSON else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update JSON is missing",
                fallbackClassification: "missingUpdateJSON"
            )
            consumePendingEditorUpdate(revision: pendingRevision)
            return
        }
        switch applyEditorUpdateOutcome(updateJSON, sourceEditorId: pendingEditorUpdateEditorId, resetJSON: pendingEditorUpdateResetJSON) {
        case .applied:
            consumePendingEditorUpdate(revision: pendingRevision)
        case .retryableDeferred:
            schedulePendingEditorUpdateRetry()
        case .rejected:
            // Mark this prop revision consumed before discarding its envelope:
            // OnViewDidUpdateProps can run again with the same values, but a
            // permanent rejection must not report or retry a second time.
            if let sourceEditorId = pendingEditorUpdateEditorId,
               richTextView.editorId != 0,
               sourceEditorId != String(richTextView.editorId)
            {
                pendingEditorUpdateEditorId = nil
            }
            consumePendingEditorUpdate(revision: pendingRevision)
        }
    }

    private func consumePendingEditorUpdate(revision: Int) {
        appliedEditorUpdateRevision = revision
        pendingEditorUpdateJSON = nil
        pendingEditorUpdateRevision = 0
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = nil
        pendingEditorUpdateRetryGeneration &+= 1
    }

    private func schedulePendingEditorUpdateRetry() {
        guard !pendingEditorUpdateRetryScheduled else { return }
        pendingEditorUpdateRetryEditorId = richTextView.editorId
        pendingEditorUpdateRetryScheduled = true
        pendingEditorUpdateRetryGeneration &+= 1
        let retryGeneration = pendingEditorUpdateRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingEditorUpdateRetryGeneration else {
                return
            }
            guard self.pendingEditorUpdateRetryEditorId == self.richTextView.editorId else {
                self.pendingEditorUpdateRetryScheduled = false
                self.clearPendingEditorUpdateRetries()
                return
            }
            self.pendingEditorUpdateRetryScheduled = false
            self.pendingEditorUpdateRetryEditorId = nil
            self.applyPendingEditorUpdateIfNeeded()
        }
    }

    func beginExternalTextComposition(sessionId: String) -> String {
        richTextView.textView.beginExternalTextComposition(sessionId: sessionId)
    }

    func updateExternalTextComposition(sessionId: String, text: String) -> String {
        richTextView.textView.updateExternalTextComposition(sessionId: sessionId, text: text)
    }

    func commitExternalTextComposition(sessionId: String, finalText: String) -> String {
        richTextView.textView.commitExternalTextComposition(
            sessionId: sessionId,
            finalText: finalText
        )
    }

    func cancelExternalTextComposition(sessionId: String, cause: String) -> String {
        richTextView.textView.cancelExternalTextComposition(sessionId: sessionId, cause: cause)
    }

    private func reportRejectedEditorUpdateEnvelope(
        _ message: String,
        fallbackClassification: String
    ) {
        if let adapter = EditorV2Registry.adapter(forLegacyId: richTextView.editorId) {
            adapter.rejectExternalRenderEnvelope(message)
        } else {
            editorUpdateInternalRejections.append(
                "boundary/FFI_RESULT_INVALID/\(fallbackClassification)"
            )
        }
    }

    func applyRemoteCommitRefresh() {
        // Preparing an external update commits a live composition. The commit
        // re-bases the adapter itself, so leave the half-typed word alone.
        guard !richTextView.textView.hasPendingCompositionForExternalRefresh else { return }
        let boundEditorId = richTextView.editorId
        guard boundEditorId != 0,
              let adapter = EditorV2Registry.adapter(forLegacyId: boundEditorId),
              !adapter.isDestroyed
        else {
            return
        }
        let autonomousOwner = autonomousErrorBindingAdapter === adapter
            && autonomousErrorBindingToken.map { adapter.isNativeBindingOwner(token: $0) } == true
        guard autonomousOwner || richTextView.textView.ownsNativeBinding(adapter) else { return }
        let preflight = richTextView.textView.prepareForExternalEditorUpdateResult()
        guard preflight.ready else { return }
        guard let update = preflight.adoptedUpdateJSON
            ?? adapter.refreshFromRustState(mirrorSelection: nil)
        else {
            return
        }
        if !richTextView.textView.applyUpdateJSON(update),
           let recovery = adapter.recoverNativeRender() {
            richTextView.textView.applyUpdateJSON(recovery)
        }
    }

    private func applyEditorUpdateOutcome(
        _ updateJson: String,
        sourceEditorId: String?,
        resetJSON: String? = nil
    ) -> EditorUpdateApplyOutcome {
        let boundEditorId = richTextView.editorId
        guard boundEditorId != 0 else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update has no bound adapter",
                fallbackClassification: "missingAdapter"
            )
            return .rejected
        }
        guard let sourceEditorId else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update source id is missing or malformed",
                fallbackClassification: "malformedSourceEditorId"
            )
            return .rejected
        }
        guard sourceEditorId == String(boundEditorId) else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update source does not match the bound canonical editor id",
                fallbackClassification: "sourceEditorMismatch"
            )
            return .rejected
        }
        guard let adapter = EditorV2Registry.adapter(forLegacyId: boundEditorId) else {
            reportRejectedEditorUpdateEnvelope(
                "external editor update adapter is missing",
                fallbackClassification: "missingAdapter"
            )
            return .rejected
        }
        guard !adapter.isDestroyed else {
            adapter.rejectExternalRenderEnvelope("external editor update adapter is destroyed")
            return .rejected
        }
        // A malformed external envelope is permanent, even if UIKit is in a
        // transient composition that would otherwise make application
        // retryable. Classify it before entering composition preflight.
        guard adapter.validateExternalRender(updateJson) else {
            return .rejected
        }
        if let resetJSON {
            guard adapter.validateExternalReset(resetJSON) else { return .rejected }
            isApplyingJSUpdate = true
            defer { isApplyingJSUpdate = false }
            richTextView.textView.discardTransientNativeInputForEditorReset()
            guard let update = adapter.adoptExternalReset(updateJson, resetJSON: resetJSON) else {
                return .rejected
            }
            clearPendingViewCommandUpdateRetry()
            let applied = imageLoadOwner.withCurrent {
                richTextView.textView.applyUpdateJSON(update)
            }
            if applied, renderRevision(fromUpdateJSON: update)?.document != renderRevision(fromUpdateJSON: updateJson)?.document {
                isApplyingJSUpdate = false
                editorTextView(richTextView.textView, didReceiveUpdate: update)
            }
            return applied ? .applied : .rejected
        }
        if isSupersededEditorUpdate(updateJson) {
            return .applied
        }
        let preflight = richTextView.textView.prepareForExternalEditorUpdateResult()
        guard preflight.ready else {
            return .retryableDeferred
        }
        if preflight.adoptedUpdateJSON != nil {
            return .applied
        }
        let adoptedUpdateJSON = adapter.adoptExternalRender(updateJson)
        guard let adoptedUpdateJSON else {
            // The adapter owns strict-parser and destroyed-race reporting.
            // Do not add a second view-side record for the same rejection.
            return .rejected
        }
        isApplyingJSUpdate = true
        defer { isApplyingJSUpdate = false }
        imageLoadOwner.withCurrent {
            // The adapter cache and the payload are paired by the same
            // editor-scoped call above; do not let the view display a render
            // whose revision has not already been adopted for native input.
            _ = richTextView.textView.applyUpdateJSON(adoptedUpdateJSON)
        }
        return .applied
    }

    /// Apply an editor update from JS. Sets the echo-suppression flag so the
    /// resulting delegate callback is NOT re-dispatched back to JS.
    @discardableResult
    func applyEditorUpdate(_ updateJson: String) -> Bool {
        let sourceEditorId = richTextView.editorId == 0 ? nil : String(richTextView.editorId)
        switch applyEditorUpdateOutcome(updateJson, sourceEditorId: sourceEditorId) {
        case .applied:
            return true
        case .retryableDeferred:
            scheduleViewCommandUpdateRetry(updateJson, sourceEditorId: sourceEditorId)
            return false
        case .rejected:
            return false
        }
    }

    private func scheduleViewCommandUpdateRetry(_ updateJson: String, sourceEditorId: String?) {
        pendingViewCommandUpdateJSON = updateJson
        pendingViewCommandUpdateEditorId = richTextView.editorId
        guard !pendingViewCommandUpdateRetryScheduled else { return }
        pendingViewCommandUpdateRetryScheduled = true
        pendingViewCommandUpdateRetryGeneration &+= 1
        let retryGeneration = pendingViewCommandUpdateRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingViewCommandUpdateRetryGeneration else {
                return
            }
            guard self.pendingViewCommandUpdateJSON != nil else {
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            guard self.pendingViewCommandUpdateEditorId == self.richTextView.editorId else {
                self.pendingViewCommandUpdateJSON = nil
                self.pendingViewCommandUpdateEditorId = nil
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            guard self.richTextView.editorId != 0 else {
                self.pendingViewCommandUpdateJSON = nil
                self.pendingViewCommandUpdateEditorId = nil
                self.pendingViewCommandUpdateRetryScheduled = false
                return
            }
            let updateJSON = self.pendingViewCommandUpdateJSON
            self.pendingViewCommandUpdateJSON = nil
            self.pendingViewCommandUpdateEditorId = nil
            self.pendingViewCommandUpdateRetryScheduled = false
            guard let updateJSON else { return }
            switch self.applyEditorUpdateOutcome(
                updateJSON,
                sourceEditorId: sourceEditorId
            ) {
            case .applied, .rejected:
                return
            case .retryableDeferred:
                self.scheduleViewCommandUpdateRetry(updateJSON, sourceEditorId: sourceEditorId)
            }
        }
    }

    func prepareForEditorCommandJSON() -> String {
        isApplyingJSUpdate = true
        defer { isApplyingJSUpdate = false }
        let preparation = richTextView.textView.prepareForExternalEditorCommand()
        return NativeEditorViewRegistry.commandPreparationJSON(
            ready: preparation.ready,
            updateJSON: preparation.updateJSON,
            blockedReason: preparation.blockedReason
        )
    }

}
