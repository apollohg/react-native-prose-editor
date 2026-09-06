import Foundation

extension EditorV2Adapter {
    private static func viewUpdate(
        from snapshot: AtomicRenderSnapshot,
        strippingViewSelection: Bool
    ) -> String? {
        guard strippingViewSelection,
              let data = snapshot.viewUpdateJSON.data(using: .utf8),
              var update = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return strippingViewSelection ? nil : snapshot.viewUpdateJSON
        }
        update.removeValue(forKey: "selection")
        guard let strippedData = try? JSONSerialization.data(withJSONObject: update) else { return nil }
        return String(data: strippedData, encoding: .utf8)
    }

    /// Fetch one complete locked render snapshot. This is deliberately the
    /// only read in the refresh path: never splice a getState revision onto
    /// the render payload.
    private func fetchAtomicRenderSnapshot(
        mirrorScalarSelection: (anchor: UInt32, head: UInt32)?
    ) -> AtomicRenderSnapshot? {
        renderUpdateCallCountForTesting += 1
        let result: FfiJsonResult
        if let nativeOwnerId {
            result = editorV2RenderNative(
                editorId: editorId,
                ownerId: String(nativeOwnerId),
                mirrorScalarAnchor: mirrorScalarSelection?.anchor,
                mirrorScalarHead: mirrorScalarSelection?.head
            )
        } else {
            result = editorV2RenderUpdate(
                editorId: editorId,
                mirrorScalarAnchor: mirrorScalarSelection?.anchor,
                mirrorScalarHead: mirrorScalarSelection?.head
            )
        }
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            // A render update that fails or violates the frozen shape is a
            // boundary failure like any other. Returning nil without
            // reporting it leaves every caller — the paired view and the
            // stateless render probe alike — holding a bare nil with no
            // cause to surface, so the engine's own error is what travels.
            emit(error)
            return nil
        case .success(let json):
            guard let snapshot = Self.parseAtomicRenderSnapshot(json) else {
                emit(Self.contractError("v2 render update violates the frozen shape"))
                return nil
            }
            return snapshot
        }
    }

    @discardableResult
    private func adopt(_ snapshot: AtomicRenderSnapshot, strippingViewSelection: Bool) -> EditorV2DerivedUpdate? {
        guard let updateJSON = Self.viewUpdate(
            from: snapshot,
            strippingViewSelection: strippingViewSelection
        ) else {
            return nil
        }
        baseDocumentRevision = snapshot.documentRevision
        stateRevision = snapshot.stateRevision
        cachedScalarLength = snapshot.scalarLength
        cachedActiveState = snapshot.activeState
        cachedHistoryState = snapshot.historyState
        cachedViewUpdateJSON = updateJSON
        cachedAtomicRenderJSON = snapshot.atomicRenderJSON
        cachedAtomicRenderDocumentRevision = snapshot.documentRevision
        if let epoch = snapshot.positionEpoch {
            positionEpoch = epoch
        }
        // This is the engine's selection from the locked snapshot. Keep it
        // distinct from a caller-provided mirror: treating it as a mirror on
        // the next refresh would change the frozen no-mirror render shape.
        cachedAuthoritativeScalarSelection = snapshot.selection
        return EditorV2DerivedUpdate(updateJSON: updateJSON, scalarLength: snapshot.scalarLength)
    }

    func atomicRenderJSON(matchingDocumentRevision documentRevision: UInt64) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard cachedAtomicRenderDocumentRevision == documentRevision else { return nil }
        return cachedAtomicRenderJSON
    }

    private func parseExternalReset(_ resetJSON: String) -> (payload: [String: Any], revision: UInt64)? {
        guard let data = resetJSON.data(using: .utf8),
              let reset = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              reset["history"] as? String == "resetAndClear",
              Set(reset.keys) == ["history", "documentRevision", reset["setJson"] != nil ? "setJson" : "setHtml"],
              reset["setJson"] is [String: Any] || reset["setHtml"] is String,
              let revision = Self.uint64Field(reset, "documentRevision")
        else {
            rejectExternalRenderEnvelope("external reset intent is malformed")
            return nil
        }
        return (reset, revision)
    }

    func validateExternalReset(_ resetJSON: String) -> Bool {
        parseExternalReset(resetJSON) != nil
    }

    func adoptExternalReset(_ renderJSON: String, resetJSON: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let intent = parseExternalReset(resetJSON) else { return nil }
        var reset = intent.payload
        let resetRevision = intent.revision
        guard let current = fetchAtomicRenderSnapshot(mirrorScalarSelection: nil) else { return nil }
        if current.documentRevision == resetRevision {
            return adoptExternalRender(renderJSON)
        }
        if latestJSDrivenDocumentRevision > resetRevision {
            return adopt(current, strippingViewSelection: false)?.updateJSON
        }
        switch Self.normalizeJsonResult(editorV2GetState(editorId: editorId)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let state = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let origin = state["documentOrigin"] as? String
            else {
                emit(contractError("v2 reset state violates the frozen shape"))
                return nil
            }
            if origin != "nativeView" {
                return refreshInternal(mirrorSelection: nil, strippingViewSelection: false)?.updateJSON
            }
        }
        reset.removeValue(forKey: "documentRevision")
        let result = callWithEnvelope(reset, includeBaseRevision: false) { requestJSON in
            editorV2ReplaceDocument(editorId: self.editorId, requestJson: requestJSON)
        }
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let commit = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let changed = commit["changed"] as? Bool,
                  Self.uint64Field(commit, "documentRevision") != nil
            else {
                emit(contractError("v2 reset result violates the frozen shape"))
                return nil
            }
            guard let update = refreshInternal(mirrorSelection: nil, strippingViewSelection: false) else {
                return nil
            }
            if changed {
                publishCachedCollaborationSelection()
                notifyCollaborationMutation()
            }
            return update.updateJSON
        }
    }

    func adoptExternalRender(_ renderJSON: String) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard !destroyed else {
            rejectExternalRenderEnvelope("external editor update adapter is destroyed")
            return nil
        }
        guard let snapshot = Self.parseAtomicRenderSnapshot(renderJSON),
              let adopted = adopt(snapshot, strippingViewSelection: false)
        else {
            rejectAtomicRenderSnapshot()
            return nil
        }
        if snapshot.positionEpoch == nil, !pinCurrentPositionEpoch(snapshot.documentRevision) {
            return nil
        }
        return adopted.updateJSON
    }

    /// Validate an externally supplied atomic render without adopting it.
    /// A preflight commit can make an otherwise valid render stale; callers
    /// still need to reject malformed envelopes exactly once before replacing
    /// that stale render with a current atomic refresh.
    func validateExternalRender(_ renderJSON: String) -> Bool {
        guard beginRuntimeOperation() else { return false }
        defer { endRuntimeOperation() }
        guard !destroyed else {
            rejectExternalRenderEnvelope("external editor update adapter is destroyed")
            return false
        }
        guard Self.parseAtomicRenderSnapshot(renderJSON) != nil else {
            rejectAtomicRenderSnapshot()
            return false
        }
        return true
    }

    func pinCurrentPositionEpoch(_ documentRevision: UInt64) -> Bool {
        guard let nativeOwnerId else { return true }
        switch Self.normalizeJsonResult(
            editorV2PinPositionEpoch(
                editorId: editorId,
                ownerId: String(nativeOwnerId),
                documentRevision: String(documentRevision)
            )
        ) {
        case .failure(let error):
            emit(error)
            return false
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  Set(object.keys) == ["positionEpoch"],
                  let value = object["positionEpoch"] as? String,
                  let epoch = UInt64(value), String(epoch) == value
            else {
                emit(Self.contractError("v2 position epoch result violates the frozen shape"))
                return false
            }
            positionEpoch = epoch
            return true
        }
    }

    /// Re-read the authoritative v2 state and update the render caches.
    @discardableResult
    func refreshInternal(
        mirrorSelection: (anchor: UInt32, head: UInt32)?,
        strippingViewSelection: Bool = false
    ) -> EditorV2DerivedUpdate? {
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
        guard let snapshot = fetchAtomicRenderSnapshot(mirrorScalarSelection: mirrorSelection),
              let derived = adopt(snapshot, strippingViewSelection: strippingViewSelection)
        else {
            debugNotes.append("deriveUpdateJSON failed")
            return nil
        }
        return derived
    }

    /// Public recovery entry (stale-revision recovery, external refresh).
    func refreshFromRustState(mirrorSelection: (anchor: UInt32, head: UInt32)?) -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return refreshInternal(mirrorSelection: mirrorSelection)?.updateJSON
    }

    func recoverNativeRender() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if let ownerId = nativeOwnerId {
            positionEpoch = nil
            _ = editorV2ReleaseNativeBinding(editorId: editorId, ownerId: String(ownerId))
        }
        return refreshFromRustState(mirrorSelection: nil)
    }

    /// Synthesized current-state update (selection/activeState included,
    /// mirroring the legacy `editorGetCurrentState` contract).
    func currentStateJSON() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return refreshInternal(mirrorSelection: lastSyncedScalarSelection)?.updateJSON
    }

    /// The initial bind render. The host passes this exact snapshot directly
    /// to the text view and toolbar, so it must not be replayed by a later
    /// independent current-state read.
    func initialUpdateJSON() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        return refreshInternal(mirrorSelection: nil)?.updateJSON
    }

    func documentHtml() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard !destroyed else { return nil }
        switch Self.normalizeJsonResult(editorV2GetDocumentHtml(editorId: editorId)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else {
                emit(contractError("v2 getDocumentHtml value violates the frozen shape"))
                return nil
            }
            return object["html"] as? String
        }
    }

    /// The authoritative v2 document JSON.
    func documentJson() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard !destroyed else { return nil }
        return fetchDocumentJson()
    }

    /// The v2 content snapshot `{html, json}` (same frozen shape as legacy).
    func contentSnapshotJSON() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard !destroyed else { return nil }
        switch Self.normalizeJsonResult(editorV2GetContentSnapshot(editorId: editorId)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            return json
        }
    }

    /// Engine-owned history flags (module `editorCanUndo/editorCanRedo`).
    func historyFlags() -> (canUndo: Bool, canRedo: Bool)? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        if let cachedHistoryState { return cachedHistoryState }
        guard refreshInternal(mirrorSelection: nil) != nil else { return nil }
        return cachedHistoryState
    }

    /// The resolved selection JSON (legacy `editorGetSelection` shape).
    func selectionJSON() -> String? {
        guard beginRuntimeOperation() else { return nil }
        defer { endRuntimeOperation() }
        guard let derived = refreshInternal(mirrorSelection: lastSyncedScalarSelection),
              let updateData = derived.updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: updateData) as? [String: Any],
              let selection = update["selection"],
              let data = try? JSONSerialization.data(withJSONObject: selection)
        else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

}
