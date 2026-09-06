import Foundation

extension EditorV2Adapter {
    func commitNativeTextMutation(
        from: UInt32,
        to: UInt32,
        with text: String,
        postSelection: (anchor: UInt32, head: UInt32)?
    ) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if nativeOwnerId != nil {
            var intent: [String: Any]
            if from == to {
                guard !text.isEmpty else {
                    guard let postSelection else { return currentStateJSON() }
                    intent = [
                        "type": "setSelection",
                        "anchor": Int(postSelection.anchor),
                        "head": Int(postSelection.head),
                    ]
                    return performNativeIntent(intent)?.updateJSON
                }
                intent = nativeIntent("insertText", anchor: from, head: from)
                intent["text"] = text
            } else if text.isEmpty {
                intent = nativeIntent("deleteRange", anchor: from, head: to)
            } else {
                intent = nativeIntent("replaceSelectionText", anchor: from, head: to)
                intent["text"] = text
            }
            guard let mutationOutcome = submitNativeIntent(
                intent,
                reportPositionEpochInvalid: true
            ) else {
                return nil
            }
            guard let postSelection else {
                return renderNativeIntentOutcome(mutationOutcome)?.updateJSON
            }
            guard let mutationRender = renderNativeIntentOutcome(
                mutationOutcome,
                publishMutation: false
            ) else {
                return nil
            }
            let selectionIntent: [String: Any] = [
                "type": "setSelection",
                "anchor": Int(postSelection.anchor),
                "head": Int(postSelection.head),
            ]
            guard let selectionOutcome = submitNativeIntent(
                selectionIntent,
                refreshPositionEpochInvalid: false
            ) else {
                if mutationOutcome.changed {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                return mutationRender.updateJSON
            }
            let combinedOutcome = NativeIntentOutcome(
                changed: mutationOutcome.changed || selectionOutcome.changed,
                documentChanged: mutationOutcome.documentChanged
            )
            guard let stateRender = renderNativeIntentOutcome(combinedOutcome) else {
                if combinedOutcome.changed {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                return nil
            }
            guard
                let combinedUpdateJSON = Self.replacingRender(
                    in: stateRender.updateJSON,
                    with: mutationRender.updateJSON
                )
            else {
                emit(contractError("v2 native mutation update could not combine render and selection"))
                return nil
            }
            return combinedUpdateJSON
        }

        let mutationUpdateJSON: String?
        if from == to {
            mutationUpdateJSON = text.isEmpty
                ? currentStateJSON()
                : insertText(text, atScalar: from)
        } else if text.isEmpty {
            mutationUpdateJSON = deleteScalarRange(from: from, to: to)
        } else {
            mutationUpdateJSON = replaceTextRange(from: from, to: to, with: text)
        }
        guard let mutationUpdateJSON,
              let postSelection,
              syncSelection(anchor: postSelection.anchor, head: postSelection.head) != nil
        else {
            return mutationUpdateJSON
        }

        return refreshInternal(mirrorSelection: nil)?.updateJSON ?? mutationUpdateJSON
    }

    func insertText(_ text: String, atScalar scalarPos: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard !text.isEmpty else { return currentStateJSON() }
        if nativeOwnerId != nil {
            var intent = nativeIntent("insertText", anchor: scalarPos, head: scalarPos)
            intent["text"] = text
            return performNativeIntent(intent)?.updateJSON
        }
        let postCaret = scalarPos &+ EditorV2PositionBridge.scalarLength(of: text)
        return performMutation(
            preSelection: (scalarPos, scalarPos),
            postSelectionMirror: (postCaret, postCaret),
        ) {
            self.callWithEnvelope(["text": text]) { requestJson in
                editorV2ApplyInput(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func replaceTextRange(from: UInt32, to: UInt32, with text: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if text.isEmpty {
            return deleteScalarRange(from: from, to: to)
        }
        if nativeOwnerId != nil {
            var intent = nativeIntent("replaceSelectionText", anchor: from, head: to)
            intent["text"] = text
            return performNativeIntent(intent)?.updateJSON
        }
        let postCaret = from &+ EditorV2PositionBridge.scalarLength(of: text)
        // A range-replacing commit (autocorrect, paste-over-selection, IME
        // commit over a marked range) is ONE typed ReplaceSelectionText
        // transaction: the planner's InsertText is collapsed-only, so the
        // command form carries the range replacement atomically.
        return performMutation(
            preSelection: (from, to),
            postSelectionMirror: (postCaret, postCaret),
        ) {
            self.callWithEnvelope([
                "command": ["type": "replaceSelectionText", "text": text]
            ]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func replaceTextRangeWithNativeOutcome(
        from: UInt32,
        to: UInt32,
        with text: String
    ) -> NativeMutationRender? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard nativeOwnerId != nil else { return nil }
        if text.isEmpty {
            guard from < to else {
                return refreshUnchangedNativeOutcome(
                    performNativeIntent(
                        nativeIntent("setSelection", anchor: from, head: from),
                        reportPositionEpochInvalid: true
                    )
                )
            }
            return refreshUnchangedNativeOutcome(
                performNativeIntent(
                    nativeIntent("deleteRange", anchor: from, head: to),
                    reportPositionEpochInvalid: true
                )
            )
        }
        var intent = nativeIntent("replaceSelectionText", anchor: from, head: to)
        intent["text"] = text
        return refreshUnchangedNativeOutcome(
            performNativeIntent(intent, reportPositionEpochInvalid: true)
        )
    }

    func insertTextWithNativeOutcome(
        _ text: String,
        atScalar scalarPos: UInt32
    ) -> NativeMutationRender? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard nativeOwnerId != nil else { return nil }
        guard !text.isEmpty else {
            return refreshUnchangedNativeOutcome(
                performNativeIntent(
                    nativeIntent("setSelection", anchor: scalarPos, head: scalarPos),
                    reportPositionEpochInvalid: true
                )
            )
        }
        var intent = nativeIntent("insertText", anchor: scalarPos, head: scalarPos)
        intent["text"] = text
        return refreshUnchangedNativeOutcome(
            performNativeIntent(intent, reportPositionEpochInvalid: true)
        )
    }

    private func refreshUnchangedNativeOutcome(
        _ outcome: NativeMutationRender?
    ) -> NativeMutationRender? {
        guard let outcome else { return nil }
        guard !outcome.documentChanged else { return outcome }
        guard let ownerId = nativeOwnerId else { return nil }
        nativeOwnerId = nil
        let updateJSON = refreshInternal(
            mirrorSelection: nil,
            strippingViewSelection: false
        )?.updateJSON
        nativeOwnerId = ownerId
        guard let updateJSON, pinCurrentPositionEpoch(baseDocumentRevision) else { return nil }
        return NativeMutationRender(
            updateJSON: updateJSON,
            changed: outcome.changed,
            documentChanged: outcome.documentChanged
        )
    }

    func deleteScalarRange(from: UInt32, to: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard from < to else { return currentStateJSON() }
        if nativeOwnerId != nil {
            return performNativeIntent(nativeIntent("deleteRange", anchor: from, head: to))?.updateJSON
        }
        return performMutation(postSelectionMirror: (from, from)) {
            self.callWithEnvelope([
                    "command": [
                        "type": "deleteRange",
                        "range": [
                            "from": EditorV2PositionBridge.positionEnvelope(scalar: from),
                            "to": EditorV2PositionBridge.positionEnvelope(scalar: to),
                        ],
                    ] as [String: Any],
            ]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func deleteRange(fromDoc: UInt32, toDoc: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let from = scalarPosition(forDoc: fromDoc), let to = scalarPosition(forDoc: toDoc) else {
            return nil
        }
        return deleteScalarRange(from: from, to: to)
    }

    func deleteBackward(anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if nativeOwnerId != nil {
            return performNativeIntent(
                nativeIntent("deleteBackward", anchor: anchor, head: head)
            )?.updateJSON
        }
        let postCaret = anchor == head ? (anchor > 0 ? anchor - 1 : 0) : min(anchor, head)
        return performMutation(
            preSelection: (anchor, head),
            postSelectionMirror: (postCaret, postCaret),
        ) {
            self.callWithEnvelope(["command": ["type": "deleteBackward"]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func splitBlock(atScalar scalarPos: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if nativeOwnerId != nil {
            return performNativeIntent(
                nativeIntent("splitBlock", anchor: scalarPos, head: scalarPos)
            )?.updateJSON
        }
        // The caret lands at the start of the new block: one scalar past the
        // split point (the block separator counts as one scalar).
        return performMutation(
            preSelection: (scalarPos, scalarPos),
            postSelectionMirror: (scalarPos &+ 1, scalarPos &+ 1),
        ) {
            self.callWithEnvelope(["command": ["type": "splitBlock"]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func deleteAndSplit(from: UInt32, to: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if nativeOwnerId != nil {
            return performNativeIntent(
                nativeIntent("deleteAndSplit", anchor: from, head: to)
            )?.updateJSON
        }
        return performMutation(
            preSelection: (from, to),
            postSelectionMirror: (from, from),
        ) {
            self.callWithEnvelope(["command": ["type": "deleteAndSplit"]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func insertNode(_ nodeType: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if nativeOwnerId != nil {
            return commandAtSelection(
                ["type": "insertNode", "nodeType": nodeType],
                anchor: anchor,
                head: head
            )
        }
        if EditorNodeTypes.isHardBreak(nodeType) {
            // Inline void: the caret lands immediately after the break.
            let caret = min(anchor, head) &+ 1
            return performMutation(
                preSelection: (anchor, head),
                postSelectionMirror: (caret, caret)
            ) {
                self.callWithEnvelope(["command": ["type": "insertNode", "nodeType": nodeType]]) { requestJson in
                    editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
                }
            }
        }
        // Block-level void (horizontalRule, image): the planner inserts the
        // block after the current block and moves the caret into the
        // trailing paragraph; the exact scalar is derived post-hoc.
        guard let update = performMutation(preSelection: (anchor, head), {
            self.callWithEnvelope(["command": ["type": "insertNode", "nodeType": nodeType]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }) else {
            return nil
        }
        return remirrorTrailingVoidCaretIfNeeded(update)
    }

    func insertContentHtml(_ html: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return commandAtSelection(
            ["type": "insertContentHtml", "html": html],
            anchor: anchor,
            head: head
        )
    }

    /// Paste-HTML path: the view pre-syncs the UIKit selection; the content
    /// insert applies at the engine selection.
    func insertContentHtmlAtEngineSelection(_ html: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if nativeOwnerId != nil, let selection = cachedAuthoritativeScalarSelection {
            return commandAtSelection(
                ["type": "insertContentHtml", "html": html],
                anchor: selection.anchor,
                head: selection.head
            )
        }
        return performMutation {
            self.callWithEnvelope(["command": ["type": "insertContentHtml", "html": html]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    /// Same as above for a JSON fragment (module `editorInsertContentJson`).
    func insertContentJsonAtEngineSelection(_ json: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let data = json.data(using: .utf8),
              let fragment = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("insertContentJson fragment is not valid JSON"))
            return nil
        }
        if nativeOwnerId != nil, let selection = cachedAuthoritativeScalarSelection {
            return commandAtSelection(
                ["type": "insertContentJson", "json": fragment],
                anchor: selection.anchor,
                head: selection.head
            )
        }
        return performMutation {
            self.callWithEnvelope(["command": ["type": "insertContentJson", "json": fragment]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func insertContentJson(_ json: String, anchor: UInt32, head: UInt32) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let data = json.data(using: .utf8),
              let fragment = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("insertContentJson fragment is not valid JSON"))
            return nil
        }
        if nativeOwnerId != nil {
            return commandAtSelection(
                ["type": "insertContentJson", "json": fragment],
                anchor: anchor,
                head: head
            )
        }
        guard let update = performMutation(preSelection: (anchor, head), {
            self.callWithEnvelope(["command": ["type": "insertContentJson", "json": fragment]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }) else {
            return nil
        }
        // A fragment of block voids (image/horizontalRule) leaves the caret
        // in the trailing paragraph the planner appends.
        return remirrorTrailingVoidCaretIfNeeded(update)
    }

}
