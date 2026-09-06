import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func clearPendingEditorUpdateRetries() {
        pendingEditorUpdateJSON = nil
        pendingEditorUpdateEditorId = nil
        pendingEditorUpdateRevision = 0
        appliedEditorUpdateRevision = 0
        renderedRevision = nil
        pendingEditorUpdateRetryScheduled = false
        pendingEditorUpdateRetryEditorId = nil
        pendingEditorUpdateRetryGeneration &+= 1
    }

    func clearPendingViewCommandUpdateRetry() {
        pendingViewCommandUpdateJSON = nil
        pendingViewCommandUpdateEditorId = nil
        pendingViewCommandUpdateRetryScheduled = false
        pendingViewCommandUpdateRetryGeneration &+= 1
    }

    func clearPendingEditableRetry() {
        pendingEditableRetryValue = nil
        pendingEditableRetryEditorId = nil
        pendingEditableRetryScheduled = false
        pendingEditableRetryGeneration &+= 1
    }

    func clearPendingThemeRetry() {
        pendingThemeRetry.clear()
    }

    func clearPendingAtomsRetry() {
        pendingAtomsRetry.clear()
    }

    func clearPendingAccessoryRetry() {
        pendingAccessoryRetryActions = []
        invalidatedAccessoryRetryActions.removeAll()
        pendingAccessoryRetryEditorId = nil
        pendingAccessoryRetryScheduled = false
        pendingAccessoryRetryGeneration &+= 1
    }

    func clearPendingMentionSuggestionRetry() {
        pendingMentionSuggestionRetry = nil
        pendingMentionSuggestionRetryScheduled = false
        pendingMentionSuggestionRetryGeneration &+= 1
    }

    func scheduleThemeRetry(_ themeJson: String?) {
        guard let token = pendingThemeRetry.schedule(
            json: themeJson,
            editorId: richTextView.editorId,
            maxAttempts: nil
        ) else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard let retry = self.pendingThemeRetry.consume(token) else { return }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingThemeRetry()
                return
            }
            guard retry.json == self.desiredThemeJSON else {
                self.clearPendingThemeRetry()
                return
            }
            self.setThemeJson(retry.json)
        }
    }

    func scheduleAtomsRetry(_ atomsJson: String?) {
        guard let token = pendingAtomsRetry.schedule(
            json: atomsJson,
            editorId: richTextView.editorId,
            maxAttempts: Self.maxPendingUpdateRetryAttempts
        ) else { return }
        let delay = Self.nativeActionRetryDelay * Double(token.attempt)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self else { return }
            guard let retry = self.pendingAtomsRetry.consume(token) else { return }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingAtomsRetry()
                return
            }
            guard retry.json == self.desiredAtomsJSON else {
                self.clearPendingAtomsRetry()
                return
            }
            self.setAtomsJson(retry.json)
        }
    }

    func schedulePendingAtomsWakeIfNeeded() {
        guard desiredAtomsJSON != lastAtomsJSON,
              !pendingAtomsWakeScheduled
        else { return }
        clearPendingAtomsRetry()
        pendingAtomsWakeScheduled = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            self.pendingAtomsWakeScheduled = false
            guard self.desiredAtomsJSON != self.lastAtomsJSON else { return }
            self.setAtomsJson(self.desiredAtomsJSON)
        }
    }

    func prepareForInputAccessoryMutationOrRetry(_ action: PendingAccessoryRetryAction) -> Bool {
        guard richTextView.editorId != 0, richTextView.textView.isFirstResponder else {
            return true
        }
        guard richTextView.textView.prepareForExternalEditorUpdate() else {
            scheduleAccessoryRetry(action)
            return false
        }
        return true
    }

    func reloadInputViewsAfterPreparingOrRetry() {
        guard prepareForInputAccessoryMutationOrRetry(.reloadInputViews) else { return }
        richTextView.textView.reloadInputViews()
        markAccessoryMutationSucceeded(.reloadInputViews)
    }

    private func scheduleAccessoryRetry(_ action: PendingAccessoryRetryAction) {
        invalidatedAccessoryRetryActions.remove(action)
        pendingAccessoryRetryActions.removeAll { $0 == action }
        pendingAccessoryRetryActions.append(action)
        pendingAccessoryRetryEditorId = richTextView.editorId
        guard !pendingAccessoryRetryScheduled else { return }
        pendingAccessoryRetryScheduled = true
        pendingAccessoryRetryGeneration &+= 1
        let retryGeneration = pendingAccessoryRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingAccessoryRetryGeneration else { return }
            guard self.pendingAccessoryRetryEditorId == self.richTextView.editorId else {
                self.clearPendingAccessoryRetry()
                return
            }
            let actions = self.pendingAccessoryRetryActions
            self.pendingAccessoryRetryActions = []
            self.pendingAccessoryRetryEditorId = nil
            self.pendingAccessoryRetryScheduled = false
            for index in actions.indices {
                let action = actions[index]
                guard retryGeneration == self.pendingAccessoryRetryGeneration else { return }
                guard !self.invalidatedAccessoryRetryActions.contains(action) else {
                    self.invalidatedAccessoryRetryActions.remove(action)
                    continue
                }
                let generationBeforeAction = self.pendingAccessoryRetryGeneration
                self.performAccessoryRetryAction(action)
                guard self.pendingAccessoryRetryGeneration == generationBeforeAction else {
                    let remainingIndex = actions.index(after: index)
                    if remainingIndex < actions.endIndex {
                        self.requeueUnprocessedAccessoryRetryActions(actions[remainingIndex...])
                    }
                    return
                }
            }
            self.invalidatedAccessoryRetryActions.subtract(actions)
        }
    }

    private func requeueUnprocessedAccessoryRetryActions(
        _ actions: ArraySlice<PendingAccessoryRetryAction>
    ) {
        for action in actions {
            guard !invalidatedAccessoryRetryActions.contains(action) else {
                invalidatedAccessoryRetryActions.remove(action)
                continue
            }
            pendingAccessoryRetryActions.removeAll { $0 == action }
            pendingAccessoryRetryActions.append(action)
        }
        if !pendingAccessoryRetryActions.isEmpty {
            pendingAccessoryRetryEditorId = richTextView.editorId
        }
    }

    private func performAccessoryRetryAction(_ action: PendingAccessoryRetryAction) {
        switch action {
        case .reloadInputViews:
            reloadInputViewsAfterPreparingOrRetry()
        case .refreshMentionQuery:
            refreshMentionQuery()
        case .clearMentionQueryState:
            clearMentionQueryStateAndHidePopover()
        case .updateAccessoryToolbarVisibility:
            updateAccessoryToolbarVisibility()
        }
    }

    func markAccessoryMutationSucceeded(_ action: PendingAccessoryRetryAction) {
        var invalidated: Set<PendingAccessoryRetryAction> = [action]
        switch action {
        case .refreshMentionQuery:
            invalidated.insert(.clearMentionQueryState)
        case .clearMentionQueryState:
            if !hasActiveMentionQueryForCurrentAddons() {
                invalidated.insert(.refreshMentionQuery)
            }
        case .reloadInputViews, .updateAccessoryToolbarVisibility:
            break
        }
        invalidatePendingAccessoryRetries(invalidated)
    }

    private func invalidatePendingAccessoryRetries(_ actions: Set<PendingAccessoryRetryAction>) {
        guard !actions.isEmpty else { return }
        invalidatedAccessoryRetryActions.formUnion(actions)
        pendingAccessoryRetryActions.removeAll { actions.contains($0) }
    }

}
