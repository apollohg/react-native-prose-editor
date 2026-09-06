import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func insertMentionSuggestion(
        _ suggestion: NativeMentionSuggestion
    ) {
        insertMentionSuggestion(suggestionKey: suggestion.key)
    }

    func insertMentionSuggestion(
        retryScope: PendingMentionSuggestionRetry
    ) {
        insertMentionSuggestion(
            suggestionKey: retryScope.suggestionKey,
            retryScope: retryScope
        )
    }

    func insertMentionSuggestion(
        suggestionKey: String,
        retryScope: PendingMentionSuggestionRetry? = nil
    ) {
        guard let mentions = addons.mentions,
              mentionQueryState != nil
        else {
            return
        }
        if let retryScope,
           !isMentionSuggestionRetryScopeCurrent(retryScope)
        {
            return
        }

        let scopedQueryState = currentMentionQueryState(trigger: mentions.trigger) ?? mentionQueryState
        guard let scopedQueryState else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        let preparation = richTextView.textView.prepareForExternalEditorCommand()
        guard preparation.ready else {
            scheduleMentionSuggestionRetry(
                PendingMentionSuggestionRetry(
                    suggestionKey: suggestionKey,
                    editorId: richTextView.editorId,
                    trigger: mentions.trigger,
                    query: scopedQueryState.query,
                    anchor: scopedQueryState.anchor,
                    head: scopedQueryState.head,
                    documentVersion: currentDocumentVersion(),
                    textSnapshot: richTextView.textView.text ?? ""
                )
            )
            return
        }
        let queryState = currentMentionQueryState(trigger: mentions.trigger)
            ?? (richTextView.textView.isFirstResponder ? nil : mentionQueryState)
        guard let queryState else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        if let retryScope,
           !doesMentionQueryState(
                queryState,
                match: retryScope,
                acceptingPreflightDocumentVersion: documentVersion(fromUpdateJSON: preparation.updateJSON),
                currentText: richTextView.textView.text ?? ""
           )
        {
            return
        }
        guard let currentSuggestion = filteredMentionSuggestions(
            for: queryState,
            config: mentions
        ).first(where: { $0.key == suggestionKey }) else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        mentionQueryState = queryState

        let attrs = resolvedMentionAttrs(trigger: mentions.trigger, suggestion: currentSuggestion)
        if mentions.resolveSelectionAttrs || mentions.resolveTheme {
            emitMentionSelectRequest(
                trigger: mentions.trigger,
                suggestion: currentSuggestion,
                attrs: attrs,
                range: queryState,
                preflightUpdateJSON: preparation.updateJSON
            )
            lastMentionEventJSON = nil
            clearMentionQueryStateAndHidePopover()
            return
        }
        let payload: [String: Any] = [
            "type": "doc",
            "content": [[
                "type": "mention",
                "attrs": attrs,
            ]],
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }

        let updateJSON = EditorV2Shadow.insertContentJsonAtSelectionScalar(
            id: richTextView.editorId,
            scalarAnchor: queryState.anchor,
            scalarHead: queryState.head,
            json: json
        )
        imageLoadOwner.withCurrent {
            _ = richTextView.textView.applyUpdateJSON(updateJSON)
        }
        emitMentionSelect(trigger: mentions.trigger, suggestion: currentSuggestion, attrs: attrs)
        lastMentionEventJSON = nil
        clearMentionQueryStateAndHidePopover()
    }

    private func scheduleMentionSuggestionRetry(_ retry: PendingMentionSuggestionRetry) {
        pendingMentionSuggestionRetry = retry
        guard !pendingMentionSuggestionRetryScheduled else { return }
        pendingMentionSuggestionRetryScheduled = true
        pendingMentionSuggestionRetryGeneration &+= 1
        let retryGeneration = pendingMentionSuggestionRetryGeneration
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            guard retryGeneration == self.pendingMentionSuggestionRetryGeneration else { return }
            guard let retry = self.pendingMentionSuggestionRetry else {
                self.pendingMentionSuggestionRetryScheduled = false
                return
            }
            guard retry.editorId == self.richTextView.editorId else {
                self.clearPendingMentionSuggestionRetry()
                return
            }
            self.pendingMentionSuggestionRetry = nil
            self.pendingMentionSuggestionRetryScheduled = false
            self.insertMentionSuggestion(retryScope: retry)
        }
    }

    private func isMentionSuggestionRetryScopeCurrent(
        _ retry: PendingMentionSuggestionRetry
    ) -> Bool {
        guard retry.editorId == richTextView.editorId,
              addons.mentions?.trigger == retry.trigger
        else {
            return false
        }
        let queryState = currentMentionQueryState(trigger: retry.trigger) ?? mentionQueryState
        guard let queryState else { return false }
        guard doesMentionQueryStateMatchRetryIdentity(queryState, match: retry) else {
            return false
        }
        return isMentionSuggestionRetryDocumentVersionCurrent(retry)
    }

    private func doesMentionQueryState(
        _ queryState: MentionQueryState,
        match retry: PendingMentionSuggestionRetry,
        acceptingPreflightDocumentVersion preflightDocumentVersion: String? = nil,
        currentText: String? = nil
    ) -> Bool {
        guard doesMentionQueryStateMatchRetryIdentity(queryState, match: retry) else {
            return false
        }

        let currentVersion = currentDocumentVersion()
        var acceptedPreflightVersionChange = false
        if let retryVersion = retry.documentVersion,
           let currentVersion,
           currentVersion != retryVersion
        {
            guard let preflightDocumentVersion,
                  currentVersion == preflightDocumentVersion
            else {
                return false
            }
            acceptedPreflightVersionChange = true
        }

        if queryState.anchor == retry.anchor && queryState.head == retry.head {
            return true
        }

        guard acceptedPreflightVersionChange else {
            return false
        }

        guard let currentText,
              let diff = mentionRetryTextDiff(
                from: retry.textSnapshot,
                to: currentText
              ),
              let mappedRange = mappedMentionRetryRange(retry, through: diff)
        else {
            return false
        }

        return queryState.anchor == mappedRange.anchor && queryState.head == mappedRange.head
    }

    private func doesMentionQueryStateMatchRetryIdentity(
        _ queryState: MentionQueryState,
        match retry: PendingMentionSuggestionRetry
    ) -> Bool {
        queryState.trigger == retry.trigger && queryState.query == retry.query
    }

    private func isMentionSuggestionRetryDocumentVersionCurrent(
        _ retry: PendingMentionSuggestionRetry
    ) -> Bool {
        let currentVersion = currentDocumentVersion()
        if let retryVersion = retry.documentVersion,
           let currentVersion,
           currentVersion != retryVersion
        {
            return false
        }
        return true
    }

    private func mentionRetryTextDiff(
        from oldText: String,
        to newText: String
    ) -> MentionRetryTextDiff? {
        let oldScalars = Array(oldText.unicodeScalars)
        let newScalars = Array(newText.unicodeScalars)
        let sharedLength = min(oldScalars.count, newScalars.count)

        var prefix = 0
        while prefix < sharedLength,
              oldScalars[prefix] == newScalars[prefix]
        {
            prefix += 1
        }

        var oldEnd = oldScalars.count
        var newEnd = newScalars.count
        while oldEnd > prefix,
              newEnd > prefix,
              oldScalars[oldEnd - 1] == newScalars[newEnd - 1]
        {
            oldEnd -= 1
            newEnd -= 1
        }

        guard prefix != oldEnd || prefix != newEnd else {
            return nil
        }

        return MentionRetryTextDiff(
            start: prefix,
            oldEnd: oldEnd,
            newEnd: newEnd
        )
    }

    private func mappedMentionRetryRange(
        _ retry: PendingMentionSuggestionRetry,
        through diff: MentionRetryTextDiff
    ) -> (anchor: UInt32, head: UInt32)? {
        let anchor = Int(retry.anchor)
        let head = Int(retry.head)
        guard anchor <= head else { return nil }

        if head <= diff.start {
            return (retry.anchor, retry.head)
        }

        if anchor >= diff.oldEnd {
            let delta = diff.newEnd - diff.oldEnd
            let mappedAnchor = anchor + delta
            let mappedHead = head + delta
            guard mappedAnchor >= 0,
                  mappedHead >= mappedAnchor,
                  mappedHead <= Int(UInt32.max)
            else {
                return nil
            }
            return (UInt32(mappedAnchor), UInt32(mappedHead))
        }

        return nil
    }

    private func currentDocumentVersion() -> String? {
        guard richTextView.editorId != 0 else { return nil }
        return documentVersion(fromUpdateJSON: EditorV2Shadow.getCurrentState(id: richTextView.editorId))
    }

}
