import ExpoModulesCore
import UIKit

extension NativeEditorExpoView {
    func refreshMentionQuery() {
        guard richTextView.editorId != 0,
              richTextView.textView.isFirstResponder,
              let mentions = addons.mentions
        else {
            clearMentionQueryStateAndHidePopover()
            return
        }
        guard prepareForInputAccessoryMutationOrRetry(.refreshMentionQuery) else { return }

        guard let queryState = currentMentionQueryState(trigger: mentions.trigger) else {
            emitMentionQueryChange(query: "", trigger: mentions.trigger, anchor: 0, head: 0, isActive: false)
            clearMentionQueryStateAndHidePopover()
            return
        }

        let suggestions = filteredMentionSuggestions(for: queryState, config: mentions)
        mentionQueryState = queryState
        accessoryToolbar.apply(mentionTheme: richTextView.textView.theme?.mentions ?? mentions.theme)
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions(
            suggestions,
            trigger: mentions.trigger
        )
        refreshSystemAssistantToolbarIfNeeded()
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        markAccessoryMutationSucceeded(.refreshMentionQuery)
        emitMentionQueryChange(
            query: queryState.query,
            trigger: queryState.trigger,
            anchor: queryState.anchor,
            head: queryState.head,
            isActive: true
        )
    }

    func clearMentionQueryStateAndHidePopover() {
        guard prepareForInputAccessoryMutationOrRetry(.clearMentionQueryState) else { return }
        mentionQueryState = nil
        let didChangeToolbarHeight = accessoryToolbar.setMentionSuggestions([])
        refreshSystemAssistantToolbarIfNeeded()
        if didChangeToolbarHeight,
           richTextView.textView.isFirstResponder,
           richTextView.textView.inputAccessoryView === accessoryToolbar
        {
            richTextView.textView.reloadInputViews()
        }
        markAccessoryMutationSucceeded(.clearMentionQueryState)
    }

    private func emitMentionQueryChange(
        query: String,
        trigger: String,
        anchor: UInt32,
        head: UInt32,
        isActive: Bool
    ) {
        let payload: [String: Any] = [
            "type": "mentionsQueryChange",
            "query": query,
            "trigger": trigger,
            "range": [
                "anchor": Int(anchor),
                "head": Int(head),
            ],
            "isActive": isActive,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        guard json != lastMentionEventJSON else { return }
        lastMentionEventJSON = json
        dispatchAddonEvent(json)
    }

    func resolvedMentionAttrs(
        trigger: String,
        suggestion: NativeMentionSuggestion
    ) -> [String: Any] {
        var attrs = suggestion.attrs
        if attrs["label"] == nil {
            attrs["label"] = suggestion.label
        }
        if attrs["mentionSuggestionChar"] == nil {
            attrs["mentionSuggestionChar"] = trigger
        }
        return attrs
    }

    func emitMentionSelect(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: [String: Any]
    ) {
        let payload: [String: Any] = [
            "type": "mentionsSelect",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": attrs,
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        dispatchAddonEvent(json)
    }

    func emitMentionSelectRequest(
        trigger: String,
        suggestion: NativeMentionSuggestion,
        attrs: [String: Any],
        range: MentionQueryState,
        preflightUpdateJSON: String? = nil
    ) {
        var payload: [String: Any] = [
            "type": "mentionsSelectRequest",
            "trigger": trigger,
            "suggestionKey": suggestion.key,
            "attrs": attrs,
            "range": [
                "anchor": Int(range.anchor),
                "head": Int(range.head),
            ],
        ]
        if let preflightUpdateJSON {
            payload["updateJson"] = preflightUpdateJSON
        }
        if let documentVersion = documentVersion(fromUpdateJSON: preflightUpdateJSON) {
            payload["documentVersion"] = documentVersion
        }
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else {
            return
        }
        dispatchAddonEvent(json)
    }

    private func dispatchAddonEvent(_ json: String) {
        let originatingEditorId = richTextView.editorId
        lastAddonEventJSONForTestingValue = json
        guard let event = Self.editorScopedEventPayload(
            ["eventJson": json],
            originatingEditorId: originatingEditorId
        ) else { return }
        onAddonEvent(event)
    }

    func isSupersededEditorUpdate(_ updateJSON: String) -> Bool {
        guard let rendered = renderedRevision,
              let incoming = renderRevision(fromUpdateJSON: updateJSON)
        else {
            return false
        }
        if incoming.document != rendered.document {
            return incoming.document < rendered.document
        }
        return incoming.state < rendered.state
    }

    func renderRevision(
        fromUpdateJSON updateJSON: String
    ) -> (document: UInt64, state: UInt64)? {
        guard let data = updateJSON.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let document = (raw["documentVersion"] as? String)
                .flatMap(v2CanonicalUInt64String)
                .flatMap(UInt64.init),
              let state = (raw["stateRevision"] as? String)
                .flatMap(v2CanonicalUInt64String)
                .flatMap(UInt64.init)
        else {
            return nil
        }
        return (document: document, state: state)
    }

    func documentVersion(fromUpdateJSON updateJSON: String?) -> String? {
        guard let updateJSON,
              let data = updateJSON.data(using: .utf8),
              let raw = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return (raw["documentVersion"] as? String).flatMap(v2CanonicalUInt64String)
    }

    func filteredMentionSuggestions(
        for queryState: MentionQueryState,
        config: NativeMentionsAddonConfig
    ) -> [NativeMentionSuggestion] {
        let query = queryState.query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else {
            return config.suggestions
        }

        return config.suggestions.filter { suggestion in
            suggestion.title.lowercased().contains(query)
                || suggestion.label.lowercased().contains(query)
                || (suggestion.subtitle?.lowercased().contains(query) ?? false)
        }
    }

    func currentMentionQueryState(trigger: String) -> MentionQueryState? {
        guard let selectedTextRange = richTextView.textView.selectedTextRange,
              selectedTextRange.isEmpty
        else {
            return nil
        }

        let currentText = richTextView.textView.text ?? ""
        let cursorUtf16Offset = richTextView.textView.offset(
            from: richTextView.textView.beginningOfDocument,
            to: selectedTextRange.start
        )
        let visibleCursorScalar = PositionBridge.utf16OffsetToScalar(
            cursorUtf16Offset,
            in: currentText
        )

        guard let visibleQueryState = resolveMentionQueryState(
            in: currentText,
            cursorScalar: visibleCursorScalar,
            trigger: trigger,
            isCaretInsideMention: isCaretInsideMention(
                cursorScalar: PositionBridge.textViewToScalar(
                    selectedTextRange.start,
                    in: richTextView.textView
                )
            )
        ) else {
            return nil
        }

        let anchorUtf16Offset = PositionBridge.scalarToUtf16Offset(
            visibleQueryState.anchor,
            in: currentText
        )
        let headUtf16Offset = PositionBridge.scalarToUtf16Offset(
            visibleQueryState.head,
            in: currentText
        )

        return MentionQueryState(
            query: visibleQueryState.query,
            trigger: visibleQueryState.trigger,
            anchor: PositionBridge.utf16OffsetToScalar(
                anchorUtf16Offset,
                in: richTextView.textView
            ),
            head: PositionBridge.utf16OffsetToScalar(
                headUtf16Offset,
                in: richTextView.textView
            )
        )
    }

    private func isCaretInsideMention(cursorScalar: UInt32) -> Bool {
        let utf16Offset = PositionBridge.scalarToUtf16Offset(
            cursorScalar,
            in: richTextView.textView.text ?? ""
        )
        let textStorage = richTextView.textView.textStorage
        guard textStorage.length > 0 else { return false }
        let candidateOffsets = [
            min(max(utf16Offset, 0), max(textStorage.length - 1, 0)),
            min(max(utf16Offset - 1, 0), max(textStorage.length - 1, 0)),
        ]

        for offset in candidateOffsets where offset >= 0 && offset < textStorage.length {
            if let nodeType = textStorage.attribute(RenderBridgeAttributes.voidNodeType, at: offset, effectiveRange: nil) as? String,
               nodeType == "mention" {
                return true
            }
        }
        return false
    }

}
