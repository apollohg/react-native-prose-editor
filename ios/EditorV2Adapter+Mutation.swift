import Foundation

extension EditorV2Adapter {
    struct MutationOutcome {
        enum Kind {
            case transaction(changed: Bool, revision: UInt64)
            case notApplicable
            case replacement(changed: Bool, revision: UInt64)
        }
        let kind: Kind
    }

    func parseMutationOutcome(_ json: String) -> MutationOutcome? {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let type = object["type"] as? String
        else { return nil }
        switch type {
        case "transaction":
            guard let changed = object["changed"] as? Bool,
                  let revision = Self.uint64Field(object, "documentRevision")
            else { return nil }
            return MutationOutcome(kind: .transaction(changed: changed, revision: revision))
        case "notApplicable":
            return MutationOutcome(kind: .notApplicable)
        case "replacement":
            guard let changed = object["changed"] as? Bool,
                  let revision = Self.uint64Field(object, "documentRevision")
            else { return nil }
            return MutationOutcome(kind: .replacement(changed: changed, revision: revision))
        default:
            return nil
        }
    }

    private func handleMutationError(_ error: FfiError) -> String? {
        if error.code == "REVISION_MISMATCH" {
            let update = refreshInternal(
                mirrorSelection: nil,
                strippingViewSelection: false
            )?.updateJSON
            debugNotes.append("mismatch-refresh \(update == nil ? "nil" : "ok")")
            return update
        }
        emit(error)
        return nil
    }

    struct NativeMutationRender {
        let updateJSON: String
        let changed: Bool
        let documentChanged: Bool
    }

    struct NativeIntentOutcome {
        let changed: Bool
        let documentChanged: Bool
    }

    func nativeIntent(_ type: String, anchor: UInt32, head: UInt32) -> [String: Any] {
        [
            "type": type,
            "anchor": Int(clampScalar(anchor)),
            "head": Int(clampScalar(head)),
        ]
    }

    func submitNativeIntent(
        _ intent: [String: Any],
        reportPositionEpochInvalid: Bool = false,
        refreshPositionEpochInvalid: Bool = true
    ) -> NativeIntentOutcome? {
        guard !destroyed else { return nil }
        guard let nativeOwnerId else { return nil }
        if positionEpoch == nil {
            guard refreshInternal(mirrorSelection: nil, strippingViewSelection: false) != nil else {
                return nil
            }
        }
        guard let positionEpoch else { return nil }
        let result = callWithEnvelope(
            [
                "ownerId": String(nativeOwnerId),
                "positionEpoch": String(positionEpoch),
                "intent": intent,
            ],
            includeBaseRevision: false
        ) { requestJson in
            editorV2ApplyNativeIntent(editorId: self.editorId, requestJson: requestJson)
        }
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            if error.code == "POSITION_EPOCH_INVALID" {
                debugNotes.append(
                    refreshPositionEpochInvalid
                        ? "position-epoch-refresh"
                        : "position-epoch-invalid"
                )
                if refreshPositionEpochInvalid {
                    _ = refreshInternal(mirrorSelection: nil, strippingViewSelection: false)
                }
                if reportPositionEpochInvalid {
                    emit(error)
                }
            } else {
                emit(error)
            }
            return nil
        case .success(let value):
            guard let outcome = parseMutationOutcome(value) else {
                emit(contractError("v2 native intent outcome violates the frozen shape"))
                return nil
            }
            let changed: Bool
            switch outcome.kind {
            case .transaction(let didChange, let revision):
                changed = didChange
                baseDocumentRevision = revision
            case .notApplicable:
                changed = false
            case .replacement(let didChange, let revision):
                changed = didChange
                baseDocumentRevision = revision
            }
            let documentChanged: Bool
            switch outcome.kind {
            case .notApplicable:
                documentChanged = false
            case .transaction(_, _), .replacement(_, _):
                guard let data = value.data(using: .utf8),
                      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let didChangeDocument = object["documentChanged"] as? Bool
                else {
                    emit(contractError("v2 native intent outcome violates the frozen shape"))
                    return nil
                }
                documentChanged = didChangeDocument
            }
            return NativeIntentOutcome(
                changed: changed,
                documentChanged: documentChanged
            )
        }
    }

    func renderNativeIntentOutcome(
        _ outcome: NativeIntentOutcome,
        publishMutation: Bool = true
    ) -> NativeMutationRender? {
        guard let update = refreshInternal(
            mirrorSelection: nil,
            strippingViewSelection: false
        )?.updateJSON else {
            return nil
        }
        lastSyncedScalarSelection = cachedAuthoritativeScalarSelection
        if publishMutation, outcome.changed {
            publishCachedCollaborationSelection()
            notifyCollaborationMutation()
        }
        return NativeMutationRender(
            updateJSON: update,
            changed: outcome.changed,
            documentChanged: outcome.documentChanged
        )
    }

    static func replacingRender(
        in stateUpdateJSON: String,
        with renderUpdateJSON: String
    ) -> String? {
        guard let stateData = stateUpdateJSON.data(using: .utf8),
              var stateUpdate = try? JSONSerialization.jsonObject(with: stateData) as? [String: Any],
              let renderData = renderUpdateJSON.data(using: .utf8),
              let renderUpdate = try? JSONSerialization.jsonObject(with: renderData) as? [String: Any]
        else {
            return nil
        }
        for key in ["renderBlocks", "renderPatch", "renderElements"] {
            if renderUpdate.keys.contains(key) {
                stateUpdate[key] = renderUpdate[key]
            } else {
                stateUpdate.removeValue(forKey: key)
            }
        }
        guard let combinedData = try? JSONSerialization.data(withJSONObject: stateUpdate) else {
            return nil
        }
        return String(data: combinedData, encoding: .utf8)
    }

    func performNativeIntent(
        _ intent: [String: Any],
        reportPositionEpochInvalid: Bool = false
    ) -> NativeMutationRender? {
        guard let outcome = submitNativeIntent(
            intent,
            reportPositionEpochInvalid: reportPositionEpochInvalid
        ) else {
            return nil
        }
        return renderNativeIntentOutcome(outcome)
    }

    /// One typed v2 mutation: optional selection pre-sync, one transaction,
    /// revision tracking, render synthesis, and the collaboration drain ping.
    ///
    /// The synthesized update mirrors the post-mutation selection whenever
    /// the caller tracks one (typing/commands) so the view can restore the
    /// caret — the legacy engine's updates always carried the selection.
    /// Callers that must NOT move the view caret (paste paths) pass no
    /// postSelectionMirror and get a selection-less update.
    /// - Parameter adoptEngineSelection: report the engine's own post-command
    ///   selection instead of forcing the caret back to caller-supplied
    ///   offsets. Required for any command that can restructure the document:
    ///   wrapping a line in a list inserts the list, item, and paragraph
    ///   openings ahead of the text, so every offset inside it shifts and the
    ///   pre-command numbers no longer address the character the caret was on.
    func performMutation(
        preSelection: (UInt32, UInt32)? = nil,
        postSelectionMirror: (UInt32, UInt32)? = nil,
        includeSelectionInUpdate: Bool = false,
        adoptEngineSelection: Bool = false,
        _ call: () -> FfiJsonResult
    ) -> String? {
        guard !destroyed else {
            emit(
                FfiError(
                    domain: "lifecycle",
                    code: "ENGINE_DESTROYED",
                    message: "editor session is destroyed",
                    requestId: nil,
                    operationIndex: nil,
                    limit: nil,
                    actual: nil,
                    detailsJson: nil
                )
            )
            return nil
        }
        let mirror = postSelectionMirror
        let pre = preSelection
        if let pre {
            switch ensureSelection(anchor: pre.0, head: pre.1) {
            case .ok:
                break
            case .refreshed(let updateJSON):
                return updateJSON
            case .failed:
                return nil
            }
        }
        switch Self.normalizeJsonResult(call()) {
        case .failure(let error):
            return handleMutationError(error)
        case .success(let value):
                guard let outcome = parseMutationOutcome(value) else {
                    emit(contractError("v2 mutation outcome violates the frozen shape"))
                    return nil
                }
                let postSelectionMirror = mirror
                let preSelection = pre
                let changed: Bool
                switch outcome.kind {
                case .transaction(let didChange, let revision):
                    changed = didChange
                    baseDocumentRevision = revision
                    if let postSelectionMirror, !adoptEngineSelection {
                        lastSyncedScalarSelection = postSelectionMirror
                    }
                case .notApplicable:
                    // Nothing applicable: no commit happened; surface the current
                    // state (legacy no-op command parity) and skip the drain.
                    return refreshInternal(mirrorSelection: postSelectionMirror ?? preSelection)?.updateJSON
                case .replacement(let didChange, let revision):
                    changed = didChange
                    baseDocumentRevision = revision
                    // Whole-root replacement resets the engine-side selection;
                    // the cached sync point is no longer valid.
                    lastSyncedScalarSelection = nil
                }
                guard let update = refreshInternal(
                    mirrorSelection: adoptEngineSelection
                        ? nil
                        : postSelectionMirror ?? (includeSelectionInUpdate ? preSelection : nil),
                    // Paste and composition-preserving paths still derive active
                    // and history state from the authoritative post-operation
                    // selection. Only the view-facing selection is omitted so
                    // UIKit retains its IME-owned caret.
                    strippingViewSelection: !adoptEngineSelection
                        && postSelectionMirror == nil
                        && !includeSelectionInUpdate
                )?.updateJSON else {
                    return nil
                }
                if adoptEngineSelection {
                    // The refresh adopted the engine's own post-command selection;
                    // that is now the view's caret, so it becomes the sync point.
                    // Leaving the pre-command offsets here would make the next
                    // ensureSelection push a stale caret back into the engine.
                    lastSyncedScalarSelection = cachedAuthoritativeScalarSelection
                }
                if changed {
                    publishCachedCollaborationSelection()
                    notifyCollaborationMutation()
                }
                return update
        }
    }

    /// Post-mutation caret for block-void insertions: when the document ends
    /// with a void block followed by an empty placeholder paragraph, the
    /// planner moved the caret into that trailing paragraph (scalar extent
    /// - 1, the placeholder position). The update is re-derived with the
    /// mirror so it carries the resolved selection the view applies.
    func remirrorTrailingVoidCaretIfNeeded(_ updateJSON: String) -> String? {
        guard let extent = cachedScalarLength,
              extent > 0,
              let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let blocks = update["renderBlocks"] as? [[[String: Any]]],
              blocks.count >= 2,
              Self.isEmptyPlaceholderParagraph(blocks[blocks.count - 1]),
              blocks[blocks.count - 2].contains(where: { ($0["type"] as? String) == "voidBlock" })
        else {
            return updateJSON
        }
        let caret = extent - 1
        lastSyncedScalarSelection = (caret, caret)
        return refreshInternal(mirrorSelection: (caret, caret))?.updateJSON
    }

    /// The trailing caret paragraph after a block-void insert renders as a
    /// single zero-width-space text run (the synthetic empty-paragraph
    /// placeholder).
    private static func isEmptyPlaceholderParagraph(_ block: [[String: Any]]) -> Bool {
        let types = block.compactMap { $0["type"] as? String }
        guard types.contains("blockStart"), types.contains("blockEnd") else { return false }
        let texts = block.filter { ($0["type"] as? String) == "textRun" }
        return texts.count == 1 && (texts[0]["text"] as? String) == "\u{200B}"
    }

    /// One history (undo/redo) mutation: no base-revision envelope, changed
    /// flag drives the synthesize path.
    func performHistoryMutation(_ call: (String) -> FfiJsonResult) -> String? {
        guard !destroyed else {
            emit(
                FfiError(
                    domain: "lifecycle",
                    code: "ENGINE_DESTROYED",
                    message: "editor session is destroyed",
                    requestId: nil,
                    operationIndex: nil,
                    limit: nil,
                    actual: nil,
                    detailsJson: nil
                )
            )
            return nil
        }
        let result = callWithEnvelope([:], includeBaseRevision: false, call)
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let value):
            guard let data = value.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let changed = object["changed"] as? Bool
            else {
                emit(contractError("v2 history outcome violates the frozen shape"))
                return nil
            }
            guard let update = refreshInternal(mirrorSelection: nil)?.updateJSON else { return nil }
            if changed {
                publishCachedCollaborationSelection()
                notifyCollaborationMutation()
            }
            return update
        }
    }

    /// A successful room mutation wakes the handle-owned native driver.
    /// The adapter never owns a generation, socket, frame, or retry timer.
    func notifyCollaborationMutation() {
        guard roomBound, let nativeEditorId = UInt64(editorId) else { return }
        collaborationWake(nativeEditorId, .localMutation)
    }

}
