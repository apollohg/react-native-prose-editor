import Foundation

// MARK: - v2 editor adapter (the only construction path; Task 16 production cutover)
//
// `EditorV2Adapter` owns one v2 editor session (decimal-string handle) and
// translates the existing native view operations into the typed v2
// transactions/results (`editorV2*`). Every mutation is one typed
// transaction against the tracked base document revision; a
// `REVISION_MISMATCH` refreshes from Rust state and is NEVER retried against
// guessed positions. Transient IME/composing state never reaches the
// adapter — only final commits do.
//
// Render derivation (Task 16B): the v2 render accessor
// (`editorV2RenderUpdate` / `editorV2ResolveScalarSelection` /
// `editorV2DocToScalar` / `editorV2ScalarToDoc`) returns everything the
// retired legacy stateless render probe provided — full render blocks,
// toolbar active state, the mirrored scalar selection resolved to doc
// positions, and the lenient doc↔scalar position mapping (including the
// document's scalar extent) — derived directly from the live v2 session.
// No legacy editor is ever created for derivation, and no legacy call
// touches the v2 session.

/// Normalized v2 string-result (exactly one of value/error).
enum EditorV2ValueResult {
    case success(String)
    case failure(FfiError)
}

/// The v2 session adapter backing one bound editor view. See the file
/// header for the architecture and the render derivation notes.
final class EditorV2Adapter {
    /// Decimal-string v2 session handle.
    let editorId: String
    /// Whether the session is room-bound (owns a collaboration outbox).
    let roomBound: Bool

    /// Autonomous structured failures (input/accessibility/lifecycle), one
    /// event per failure, mirroring the v2 error-listener contract.
    var onAutonomousError: ((FfiError) -> Void)?
    /// Sink for drained outbound collaboration frames; the socket owner
    /// (JS/TS in production, spies in tests) receives one frame per call.
    var outboundFrameSink: ((Data) -> Void)?
    /// The live collaboration generation, set by the transport owner while a
    /// socket is current. `nil` disables the local-commit drain ping (the TS
    /// controller's "no current socket" case).
    var collaborationGeneration: String?

    /// The document revision the next mutation will be based on.
    private(set) var baseDocumentRevision: UInt64 = 0
    private var nextRequestId: UInt64 = 0
    private var lastSyncedScalarSelection: (anchor: UInt32, head: UInt32)?
    private var cachedScalarLength: UInt32?
    /// Diagnostics: structured notes for adapter-path failures
    /// (mismatch refreshes, derivation failures) that never surface as
    /// autonomous error events.
    private(set) var debugNotes: [String] = []
    private var destroyed = false

    private init(editorId: String, roomBound: Bool, baseDocumentRevision: UInt64) {
        self.editorId = editorId
        self.roomBound = roomBound
        self.baseDocumentRevision = baseDocumentRevision
    }

    // MARK: - Construction

    /// Attach to an existing v2 session created through the module's
    /// JS-facing `editorV2Create` entry. The session is NOT re-created; the
    /// adapter routes the bound view's interactions through the shared
    /// session (the TS document handle and collaboration controller drive
    /// the same session over the module surface).
    static func attach(editorId: String, roomBound: Bool) -> EditorV2Adapter? {
        guard isCanonicalDecimalEditorId(editorId),
              case .success(let stateJson) = normalizeJsonResult(editorV2GetState(editorId: editorId)),
              let data = stateJson.data(using: .utf8),
              let state = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let documentRevision = uint64Field(state, "documentRevision")
        else {
            return nil
        }
        return EditorV2Adapter(
            editorId: editorId,
            roomBound: roomBound,
            baseDocumentRevision: documentRevision
        )
    }

    private static func isCanonicalDecimalEditorId(_ editorId: String) -> Bool {
        guard !editorId.isEmpty,
              editorId.allSatisfy({ $0 >= "0" && $0 <= "9" }),
              editorId == "0" || editorId.first != "0"
        else {
            return false
        }
        return UInt64(editorId) != nil
    }

    /// Destroy the v2 session. Repeated destroy is safe; an already-destroyed
    /// native session satisfies the caller's goal (mirrors the TS bridge).
    @discardableResult
    func destroy() -> FfiError? {
        if destroyed { return nil }
        destroyed = true
        let result = editorV2Destroy(editorId: editorId)
        if let error = result.error {
            if error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING" {
                return nil
            }
            return error
        }
        return nil
    }

    // MARK: - Envelopes and result normalization

    private func contractError(_ message: String) -> FfiError {
        Self.contractError(message)
    }

    private static func contractError(_ message: String) -> FfiError {
        FfiError(
            domain: "boundary",
            code: "FFI_RESULT_INVALID",
            message: message,
            requestId: nil,
            operationIndex: nil,
            limit: nil,
            actual: nil,
            detailsJson: nil
        )
    }

    private static func normalizeJsonResult(_ result: FfiJsonResult) -> EditorV2ValueResult {
        switch (result.value, result.error) {
        case let (value?, nil):
            return .success(value)
        case let (nil, error?):
            return .failure(error)
        default:
            return .failure(contractError("v2 result must carry exactly one of value/error"))
        }
    }

    private func emit(_ error: FfiError) {
        debugNotes.append("emit \(error.domain)/\(error.code): \(error.message)")
        onAutonomousError?(error)
    }

    /// Serialize one request envelope with canonical decimal-string u64
    /// fields, so Foundation never bridges them through NSNumber.
    private func buildEnvelope(_ payload: [String: Any], includeBaseRevision: Bool = true) -> String {
        nextRequestId &+= 1
        var parts = ["\"version\":1", "\"requestId\":\"\(nextRequestId)\""]
        if includeBaseRevision {
            parts.append("\"baseDocumentRevision\":\"\(baseDocumentRevision)\"")
        }
        if let data = try? JSONSerialization.data(withJSONObject: payload),
           let payloadJson = String(data: data, encoding: .utf8),
           payloadJson.count > 2
        {
            parts.append(String(payloadJson.dropFirst().dropLast()))
        }
        return "{\(parts.joined(separator: ","))}"
    }

    private func selectionEnvelope(anchor: UInt32, head: UInt32, affinity: String) -> [String: Any] {
        ["selection": EditorV2PositionBridge.textSelectionEnvelope(anchor: anchor, head: head, affinity: affinity)]
    }

    // MARK: - Structured v2 state reads

    private struct V2State {
        let documentState: String
        let transportState: String
        let renderState: String
        let documentRevision: UInt64
        let stateRevision: UInt64
        let canUndo: Bool
        let canRedo: Bool
    }

    private static func uint64Field(_ object: [String: Any], _ key: String) -> UInt64? {
        guard let string = object[key] as? String,
              isCanonicalDecimalEditorId(string)
        else { return nil }
        return UInt64(string)
    }

    private func fetchState() -> V2State? {
        switch Self.normalizeJsonResult(editorV2GetState(editorId: editorId)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let documentState = object["documentState"] as? String,
                  let transportState = object["transportState"] as? String,
                  let renderState = object["renderState"] as? String,
                  let documentRevision = Self.uint64Field(object, "documentRevision"),
                  let stateRevision = Self.uint64Field(object, "stateRevision"),
                  let canUndo = object["canUndo"] as? Bool,
                  let canRedo = object["canRedo"] as? Bool
            else {
                emit(contractError("v2 getState value violates the frozen shape"))
                return nil
            }
            return V2State(
                documentState: documentState,
                transportState: transportState,
                renderState: renderState,
                documentRevision: documentRevision,
                stateRevision: stateRevision,
                canUndo: canUndo,
                canRedo: canRedo
            )
        }
    }

    private func fetchDocumentJson() -> String? {
        switch Self.normalizeJsonResult(editorV2GetDocumentJson(editorId: editorId)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            return json
        }
    }

    // MARK: - Render derivation (v2 render accessor)

    /// One derived render update plus the document's scalar extent (the
    /// lenient `UInt32.max` doc→scalar mapping, used to clamp transient-IME
    /// positions the way the legacy engine did).
    private struct EditorV2DerivedUpdate {
        let updateJSON: String
        let scalarLength: UInt32?
    }

    private static func uint32Field(_ object: [String: Any], _ key: String) -> UInt32? {
        v2ExactUInt32(object[key] as? NSNumber)
    }

    /// The v2 render accessor: one synthesized update JSON carrying
    /// full render blocks, the toolbar active state, and (when mirrored) the
    /// resolved selection — all derived from the live v2 session. History
    /// state and document version are re-stamped from the same getState read
    /// that drives revision tracking so one state snapshot is authoritative
    /// per refresh.
    private func fetchRenderUpdate(
        mirrorScalarSelection: (anchor: UInt32, head: UInt32)?,
        canUndo: Bool,
        canRedo: Bool,
        documentVersion: UInt64
    ) -> EditorV2DerivedUpdate? {
        let result = editorV2RenderUpdate(
            editorId: editorId,
            mirrorScalarAnchor: mirrorScalarSelection?.anchor,
            mirrorScalarHead: mirrorScalarSelection?.head
        )
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            debugNotes.append("renderUpdate \(error.domain)/\(error.code)")
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  var update = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let scalarLength = Self.uint32Field(update, "scalarLength")
            else {
                debugNotes.append("renderUpdate shape invalid")
                return nil
            }
            update["historyState"] = ["canUndo": canUndo, "canRedo": canRedo]
            update["documentVersion"] = documentVersion.description
            // The scalar extent feeds the adapter's IME clamp only; the
            // view-facing update keeps the exact legacy update JSON shape.
            update.removeValue(forKey: "scalarLength")
            guard let serialized = try? JSONSerialization.data(withJSONObject: update),
                  let updateJSON = String(data: serialized, encoding: .utf8)
            else {
                return nil
            }
            return EditorV2DerivedUpdate(updateJSON: updateJSON, scalarLength: scalarLength)
        }
    }

    /// Re-read the authoritative v2 state and derive one synthesized update
    /// JSON through the v2 render accessor. Updates revision tracking
    /// and the scalar-extent cache. This is the REVISION_MISMATCH recovery
    /// path: it never re-issues the failed operation.
    @discardableResult
    private func refreshInternal(mirrorSelection: (anchor: UInt32, head: UInt32)?) -> EditorV2DerivedUpdate? {
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
        guard let state = fetchState() else { return nil }
        baseDocumentRevision = state.documentRevision
        let derived = fetchRenderUpdate(
            mirrorScalarSelection: mirrorSelection,
            canUndo: state.canUndo,
            canRedo: state.canRedo,
            documentVersion: state.documentRevision
        )
        cachedScalarLength = derived?.scalarLength
        if derived == nil {
            debugNotes.append("deriveUpdateJSON failed")
        }
        return derived
    }

    /// Public recovery entry (stale-revision recovery, external refresh).
    func refreshFromRustState(mirrorSelection: (anchor: UInt32, head: UInt32)?) -> String? {
        refreshInternal(mirrorSelection: mirrorSelection)?.updateJSON
    }

    /// Synthesized current-state update (selection/activeState included,
    /// mirroring the legacy `editorGetCurrentState` contract).
    func currentStateJSON() -> String? {
        refreshInternal(mirrorSelection: lastSyncedScalarSelection)?.updateJSON
    }

    /// The initial bind render.
    func initialUpdateJSON() -> String? {
        refreshInternal(mirrorSelection: nil)?.updateJSON
    }

    func documentHtml() -> String? {
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
        guard !destroyed else { return nil }
        return fetchDocumentJson()
    }

    /// The v2 content snapshot `{html, json}` (same frozen shape as legacy).
    func contentSnapshotJSON() -> String? {
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
        guard let state = fetchState() else { return nil }
        return (state.canUndo, state.canRedo)
    }

    /// The resolved selection JSON (legacy `editorGetSelection` shape).
    func selectionJSON() -> String? {
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

    // MARK: - Selection sync and position mapping

    private enum SelectionSyncOutcome {
        case ok
        case refreshed(String)
        case failed
    }

    /// Clamp one scalar position into the cached document extent (the legacy
    /// engine clamped leniently; the v2 engine rejects out-of-range scalars
    /// with POSITION_INVALID — transient-IME cursors can overshoot).
    private func clampScalar(_ scalar: UInt32) -> UInt32 {
        guard let extent = cachedScalarLength else { return scalar }
        return min(scalar, extent)
    }

    /// Cheap selection sync (no mapping harvest): one skip transaction.
    @discardableResult
    private func ensureSelection(anchor: UInt32, head: UInt32) -> SelectionSyncOutcome {
        let clampedAnchor = clampScalar(anchor)
        let clampedHead = clampScalar(head)
        if let last = lastSyncedScalarSelection, last == (clampedAnchor, clampedHead) {
            return .ok
        }
        // Affinity policy mirrors the engine's own cursor resolution
        // (cursor_sticky_index_from_doc_pos): a collapsed caret prefers
        // After with a deterministic Before fallback at text-boundary
        // positions; a range uses Before. The fallback changes only the
        // stickiness of the SAME position — it is not a guessed-position
        // retry.
        let collapsed = clampedAnchor == clampedHead
        var result = editorV2SetSelection(
            editorId: editorId,
            requestJson: buildEnvelope(
                selectionEnvelope(
                    anchor: clampedAnchor,
                    head: clampedHead,
                    affinity: collapsed ? "after" : "before"
                )
            )
        )
        if collapsed, let error = result.error, error.code == "POSITION_INVALID" {
            result = editorV2SetSelection(
                editorId: editorId,
                requestJson: buildEnvelope(
                    selectionEnvelope(anchor: clampedAnchor, head: clampedHead, affinity: "before")
                )
            )
        }
        switch Self.normalizeJsonResult(result) {
        case .success(let value):
            if let outcome = parseMutationOutcome(value), case .transaction(_, let revision) = outcome.kind {
                baseDocumentRevision = revision
            }
            lastSyncedScalarSelection = (clampedAnchor, clampedHead)
            return .ok
        case .failure(let error):
            if error.code == "REVISION_MISMATCH" {
                // Refresh from Rust state; never retry against guessed
                // positions. The next genuine selection event re-syncs.
                if let update = refreshInternal(mirrorSelection: (clampedAnchor, clampedHead))?.updateJSON {
                    return .refreshed(update)
                }
                return .failed
            }
            emit(error)
            return .failed
        }
    }

    /// Selection sync with the engine-authoritative doc-position mapping the
    /// delegate callback requires.
    @discardableResult
    func syncSelection(anchor: UInt32, head: UInt32) -> (docAnchor: UInt32, docHead: UInt32)? {
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
        switch ensureSelection(anchor: anchor, head: head) {
        case .ok:
            break
        case .refreshed(let updateJSON):
            guard let data = updateJSON.data(using: .utf8),
                  let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let selection = update["selection"] as? [String: Any],
                  let docAnchor = Self.uint32Field(selection, "anchor"),
                  let docHead = Self.uint32Field(selection, "head")
            else {
                return nil
            }
            return (docAnchor, docHead)
        case .failed:
            return nil
        }
        return resolveSelectionMapping(scalarAnchor: anchor, scalarHead: head)
    }

    /// Engine-authoritative scalar→doc selection mapping for one live v2
    /// session (the delegate callback's doc positions), resolved by the
    /// v2 accessor with the legacy lenient-mapping semantics.
    private func resolveSelectionMapping(
        scalarAnchor: UInt32,
        scalarHead: UInt32
    ) -> (docAnchor: UInt32, docHead: UInt32)? {
        let result = editorV2ResolveScalarSelection(
            editorId: editorId,
            scalarAnchor: scalarAnchor,
            scalarHead: scalarHead
        )
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            debugNotes.append("resolveScalarSelection \(error.domain)/\(error.code)")
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let selection = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let docAnchor = Self.uint32Field(selection, "anchor"),
                  let docHead = Self.uint32Field(selection, "head")
            else {
                debugNotes.append("resolveScalarSelection shape invalid")
                return nil
            }
            return (docAnchor, docHead)
        }
    }

    /// Selection sync where no doc mapping is consumed (shadow call sites
    /// that only need the engine to track the caret).
    func syncSelectionQuiet(anchor: UInt32, head: UInt32) {
        guard !destroyed else { return }
        _ = ensureSelection(anchor: anchor, head: head)
    }

    /// Lenient scalar→doc mapping through the v2 accessor (clamps at
    /// the document extent, exactly the legacy `editorScalarToDoc` semantics).
    func documentPosition(forScalar scalar: UInt32) -> UInt32? {
        switch Self.normalizeJsonResult(editorV2ScalarToDoc(editorId: editorId, scalar: scalar)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else {
                return nil
            }
            return Self.uint32Field(object, "doc")
        }
    }

    /// Lenient doc→scalar mapping through the v2 accessor (`docPos`
    /// of `UInt32.max` yields the document's scalar extent).
    func scalarPosition(forDoc docPos: UInt32) -> UInt32? {
        switch Self.normalizeJsonResult(editorV2DocToScalar(editorId: editorId, docPos: docPos)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            guard let data = json.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else {
                return nil
            }
            return Self.uint32Field(object, "scalar")
        }
    }

    // MARK: - Mutation driver

    private struct MutationOutcome {
        enum Kind {
            case transaction(changed: Bool, revision: UInt64)
            case notApplicable
            case replacement(changed: Bool, revision: UInt64)
        }
        let kind: Kind
    }

    private func parseMutationOutcome(_ json: String) -> MutationOutcome? {
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

    private func handleMutationError(_ error: FfiError, mirror: (UInt32, UInt32)?) -> String? {
        if error.code == "REVISION_MISMATCH" {
            // Refresh from Rust state; NEVER retry against guessed positions.
            let update = refreshInternal(mirrorSelection: mirror)?.updateJSON
            debugNotes.append("mismatch-refresh \(update == nil ? "nil" : "ok")")
            return update
        }
        emit(error)
        return nil
    }

    /// One typed v2 mutation: optional selection pre-sync, one transaction,
    /// revision tracking, render synthesis, and the collaboration drain ping.
    ///
    /// The synthesized update mirrors the post-mutation selection whenever
    /// the caller tracks one (typing/commands) so the view can restore the
    /// caret — the legacy engine's updates always carried the selection.
    /// Callers that must NOT move the view caret (paste paths) pass no
    /// postSelectionMirror and get a selection-less update.
    private func performMutation(
        preSelection: (UInt32, UInt32)? = nil,
        postSelectionMirror: (UInt32, UInt32)? = nil,
        includeSelectionInUpdate: Bool = false,
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
        if let preSelection {
            switch ensureSelection(anchor: preSelection.0, head: preSelection.1) {
            case .ok:
                break
            case .refreshed(let updateJSON):
                // The pre-sync discovered staleness: the operation is NOT
                // retried; the refresh update is the resolution.
                return updateJSON
            case .failed:
                return nil
            }
        }
        let result = call()
        switch Self.normalizeJsonResult(result) {
        case .failure(let error):
            return handleMutationError(error, mirror: postSelectionMirror ?? preSelection)
        case .success(let value):
            guard let outcome = parseMutationOutcome(value) else {
                emit(contractError("v2 mutation outcome violates the frozen shape"))
                return nil
            }
            switch outcome.kind {
            case .transaction(_, let revision):
                baseDocumentRevision = revision
                if let postSelectionMirror {
                    lastSyncedScalarSelection = postSelectionMirror
                }
            case .notApplicable:
                // Nothing applicable: no commit happened; surface the current
                // state (legacy no-op command parity) and skip the drain.
                return refreshInternal(mirrorSelection: postSelectionMirror ?? preSelection)?.updateJSON
            case .replacement(_, let revision):
                baseDocumentRevision = revision
                // Whole-root replacement resets the engine-side selection;
                // the cached sync point is no longer valid.
                lastSyncedScalarSelection = nil
            }
            guard let update = refreshInternal(
                mirrorSelection: postSelectionMirror ?? (includeSelectionInUpdate ? preSelection : nil)
            )?.updateJSON else {
                return nil
            }
            drainOutboundIfNeeded()
            return update
        }
    }

    /// Post-mutation caret for block-void insertions: when the document ends
    /// with a void block followed by an empty placeholder paragraph, the
    /// planner moved the caret into that trailing paragraph (scalar extent
    /// - 1, the placeholder position). The update is re-derived with the
    /// mirror so it carries the resolved selection the view applies.
    private func remirrorTrailingVoidCaretIfNeeded(_ updateJSON: String) -> String? {
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
    private func performHistoryMutation(_ call: (String) -> FfiJsonResult) -> String? {
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
        let result = call(buildEnvelope([:], includeBaseRevision: false))
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
                drainOutboundIfNeeded()
            }
            return update
        }
    }

    // MARK: - Drain ping (mirrors the TS controller's onLocalDocumentCommit)

    /// The manual drain ping: on a room-bound session with a live
    /// generation and a frame sink, drain the outbox one frame per call
    /// until empty (protocol replies before document updates — Rust owns the
    /// ordering). Called internally after every accepted local mutation;
    /// also the public entry for the transport owner (the TS controller's
    /// `onLocalDocumentCommit` semantics).
    func driveCollaborationDrainPing() {
        drainOutboundIfNeeded()
    }

    private func drainOutboundIfNeeded() {
        guard roomBound, let generation = collaborationGeneration, let sink = outboundFrameSink else {
            return
        }
        while true {
            let result = editorV2CollaborationTakeOutbound(editorId: editorId, generation: generation)
            if let error = result.error {
                emit(error)
                return
            }
            guard let frame = result.value else {
                emit(contractError("v2 takeOutbound result must carry exactly one of value/error"))
                return
            }
            if frame.isEmpty { return }
            sink(frame)
        }
    }

    // MARK: - Typed verbs (one method per legacy choke point)

    func insertText(_ text: String, atScalar scalarPos: UInt32) -> String? {
        guard !text.isEmpty else { return currentStateJSON() }
        let postCaret = scalarPos &+ EditorV2PositionBridge.scalarLength(of: text)
        return performMutation(
            preSelection: (scalarPos, scalarPos),
            postSelectionMirror: (postCaret, postCaret)
        ) {
            editorV2ApplyInput(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["text": text])
            )
        }
    }

    func replaceTextRange(from: UInt32, to: UInt32, with text: String) -> String? {
        if text.isEmpty {
            return deleteScalarRange(from: from, to: to)
        }
        let postCaret = from &+ EditorV2PositionBridge.scalarLength(of: text)
        // A range-replacing commit (autocorrect, paste-over-selection, IME
        // commit over a marked range) is ONE typed ReplaceSelectionText
        // transaction: the planner's InsertText is collapsed-only, so the
        // command form carries the range replacement atomically.
        return performMutation(
            preSelection: (from, to),
            postSelectionMirror: (postCaret, postCaret)
        ) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope([
                    "command": ["type": "replaceSelectionText", "text": text]
                ])
            )
        }
    }

    func deleteScalarRange(from: UInt32, to: UInt32) -> String? {
        guard from < to else { return currentStateJSON() }
        return performMutation(postSelectionMirror: (from, from)) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope([
                    "command": [
                        "type": "deleteRange",
                        "range": [
                            "from": EditorV2PositionBridge.positionEnvelope(scalar: from),
                            "to": EditorV2PositionBridge.positionEnvelope(scalar: to),
                        ],
                    ] as [String: Any],
                ])
            )
        }
    }

    func deleteRange(fromDoc: UInt32, toDoc: UInt32) -> String? {
        guard let from = scalarPosition(forDoc: fromDoc), let to = scalarPosition(forDoc: toDoc) else {
            return nil
        }
        return deleteScalarRange(from: from, to: to)
    }

    func deleteBackward(anchor: UInt32, head: UInt32) -> String? {
        let postCaret = anchor == head ? (anchor > 0 ? anchor - 1 : 0) : min(anchor, head)
        return performMutation(
            preSelection: (anchor, head),
            postSelectionMirror: (postCaret, postCaret)
        ) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "deleteBackward"]])
            )
        }
    }

    func splitBlock(atScalar scalarPos: UInt32) -> String? {
        // The caret lands at the start of the new block: one scalar past the
        // split point (the block separator counts as one scalar).
        performMutation(
            preSelection: (scalarPos, scalarPos),
            postSelectionMirror: (scalarPos &+ 1, scalarPos &+ 1)
        ) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "splitBlock"]])
            )
        }
    }

    func deleteAndSplit(from: UInt32, to: UInt32) -> String? {
        performMutation(
            preSelection: (from, to),
            postSelectionMirror: (from, from)
        ) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "deleteAndSplit"]])
            )
        }
    }

    func insertNode(_ nodeType: String, anchor: UInt32, head: UInt32) -> String? {
        if nodeType == "hardBreak" {
            // Inline void: the caret lands immediately after the break.
            let caret = min(anchor, head) &+ 1
            return performMutation(
                preSelection: (anchor, head),
                postSelectionMirror: (caret, caret)
            ) {
                editorV2ApplyCommand(
                    editorId: self.editorId,
                    requestJson: self.buildEnvelope(["command": ["type": "insertNode", "nodeType": nodeType]])
                )
            }
        }
        // Block-level void (horizontalRule, image): the planner inserts the
        // block after the current block and moves the caret into the
        // trailing paragraph; the exact scalar is derived post-hoc.
        guard let update = performMutation(preSelection: (anchor, head), {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "insertNode", "nodeType": nodeType]])
            )
        }) else {
            return nil
        }
        return remirrorTrailingVoidCaretIfNeeded(update)
    }

    func insertContentHtml(_ html: String, anchor: UInt32, head: UInt32) -> String? {
        performMutation(preSelection: (anchor, head)) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "insertContentHtml", "html": html]])
            )
        }
    }

    /// Paste-HTML path: the view pre-syncs the UIKit selection; the content
    /// insert applies at the engine selection.
    func insertContentHtmlAtEngineSelection(_ html: String) -> String? {
        performMutation {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "insertContentHtml", "html": html]])
            )
        }
    }

    /// Same as above for a JSON fragment (module `editorInsertContentJson`).
    func insertContentJsonAtEngineSelection(_ json: String) -> String? {
        guard let data = json.data(using: .utf8),
              let fragment = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("insertContentJson fragment is not valid JSON"))
            return nil
        }
        return performMutation {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "insertContentJson", "json": fragment]])
            )
        }
    }

    func insertContentJson(_ json: String, anchor: UInt32, head: UInt32) -> String? {
        guard let data = json.data(using: .utf8),
              let fragment = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("insertContentJson fragment is not valid JSON"))
            return nil
        }
        guard let update = performMutation(preSelection: (anchor, head), {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": ["type": "insertContentJson", "json": fragment]])
            )
        }) else {
            return nil
        }
        // A fragment of block voids (image/horizontalRule) leaves the caret
        // in the trailing paragraph the planner appends.
        return remirrorTrailingVoidCaretIfNeeded(update)
    }

    func toggleMark(_ markType: String, anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "toggleMark", "markType": markType], anchor: anchor, head: head)
    }

    func setMark(_ markType: String, attrsJson: String, anchor: UInt32, head: UInt32) -> String? {
        guard let data = attrsJson.data(using: .utf8),
              let attrs = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            emit(contractError("setMark attrs are not valid JSON"))
            return nil
        }
        return commandAtSelection(
            ["type": "setMark", "markType": markType, "attrs": attrs],
            anchor: anchor,
            head: head
        )
    }

    func unsetMark(_ markType: String, anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "unsetMark", "markType": markType], anchor: anchor, head: head)
    }

    func toggleHeading(level: UInt8, anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "toggleHeading", "level": Int(level)], anchor: anchor, head: head)
    }

    func toggleCodeBlock(anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "toggleCodeBlock"], anchor: anchor, head: head)
    }

    func toggleBlockquote(anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "toggleBlockquote"], anchor: anchor, head: head)
    }

    func wrapInList(listType: String, itemType: String, anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(
            ["type": "wrapInList", "listType": listType, "itemType": itemType],
            anchor: anchor,
            head: head
        )
    }

    func unwrapFromList(anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "unwrapFromList"], anchor: anchor, head: head)
    }

    func indentListItem(anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "indentListItem"], anchor: anchor, head: head)
    }

    func outdentListItem(anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "outdentListItem"], anchor: anchor, head: head)
    }

    func toggleTaskItemChecked(anchor: UInt32, head: UInt32) -> String? {
        commandAtSelection(["type": "toggleTaskItemChecked"], anchor: anchor, head: head)
    }

    private func commandAtSelection(_ command: [String: Any], anchor: UInt32, head: UInt32) -> String? {
        performMutation(preSelection: (anchor, head), postSelectionMirror: (anchor, head)) {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["command": command])
            )
        }
    }

    func resizeImage(atDocPos docPos: UInt32, width: UInt32, height: UInt32) -> String? {
        guard let scalar = scalarPosition(forDoc: docPos) else { return nil }
        return performMutation {
            editorV2ApplyCommand(
                editorId: self.editorId,
                requestJson: self.buildEnvelope([
                    "command": [
                        "type": "resizeImage",
                        "at": EditorV2PositionBridge.positionEnvelope(scalar: scalar),
                        "width": Int(width),
                        "height": Int(height),
                    ] as [String: Any],
                ])
            )
        }
    }

    func undo() -> String? {
        performHistoryMutation { requestJson in
            editorV2Undo(editorId: self.editorId, requestJson: requestJson)
        }
    }

    func redo() -> String? {
        performHistoryMutation { requestJson in
            editorV2Redo(editorId: self.editorId, requestJson: requestJson)
        }
    }

    // MARK: - Controlled content (local-API, passes read-only per Source::Api parity)

    func setContentHtml(_ html: String) -> String? {
        performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            editorV2ApplyLocalApi(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["setHtml": html, "history": "resetAndClear"])
            )
        }
    }

    func setContentJson(_ json: String) -> String? {
        guard let data = json.data(using: .utf8),
              let document = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("setContentJson document is not valid JSON"))
            return nil
        }
        return performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            editorV2ApplyLocalApi(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["setJson": document, "history": "resetAndClear"])
            )
        }
    }

    /// Undoable whole-document replace (legacy `editorReplaceHtml` parity:
    /// one undoable local-API boundary, selection preserved where possible).
    func replaceContentHtml(_ html: String) -> String? {
        performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            editorV2ApplyLocalApi(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["setHtml": html, "history": "undoableBoundary"])
            )
        }
    }

    /// Undoable whole-document replace from JSON (legacy `editorReplaceJson`
    /// parity).
    func replaceContentJson(_ json: String) -> String? {
        guard let data = json.data(using: .utf8),
              let document = try? JSONSerialization.jsonObject(with: data)
        else {
            emit(contractError("replaceContentJson document is not valid JSON"))
            return nil
        }
        return performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            editorV2ApplyLocalApi(
                editorId: self.editorId,
                requestJson: self.buildEnvelope(["setJson": document, "history": "undoableBoundary"])
            )
        }
    }
}

// MARK: - Session pairing registry

/// Maps the public (module-visible) editor id to the v2 adapter backing it.
/// The module's create path registers one pairing per editor; views bound
/// to a paired id route every interaction through the adapter.
enum EditorV2Registry {
    private static let lock = NSLock()
    private static var pairings: [UInt64: EditorV2Adapter] = [:]

    static func register(_ adapter: EditorV2Adapter, forLegacyId legacyId: UInt64) {
        lock.lock()
        pairings[legacyId] = adapter
        lock.unlock()
    }

    static func adapter(forLegacyId legacyId: UInt64) -> EditorV2Adapter? {
        lock.lock()
        defer { lock.unlock() }
        return pairings[legacyId]
    }

    @discardableResult
    static func removePairing(forLegacyId legacyId: UInt64) -> EditorV2Adapter? {
        lock.lock()
        defer { lock.unlock() }
        return pairings.removeValue(forKey: legacyId)
    }

    /// Destroy the v2 session backing a pairing and drop the pairing.
    static func destroyPair(forLegacyId legacyId: UInt64) {
        removePairing(forLegacyId: legacyId)?.destroy()
    }

}
