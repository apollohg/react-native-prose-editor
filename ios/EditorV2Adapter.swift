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

private enum EditorV2EnvelopeResult {
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
    private let autonomousErrorLock = NSLock()
    private var autonomousErrorCallback: ((FfiError) -> Void)?
    private var autonomousErrorOwnerToken: UUID?

    /// Compatibility callback for adapter-focused tests and non-view owners.
    /// View bindings use the tokened owner API below so an old view cannot
    /// clear a callback claimed by a newer view.
    var onAutonomousError: ((FfiError) -> Void)? {
        get {
            autonomousErrorLock.lock()
            defer { autonomousErrorLock.unlock() }
            return autonomousErrorCallback
        }
        set {
            autonomousErrorLock.lock()
            autonomousErrorOwnerToken = nil
            autonomousErrorCallback = newValue
            autonomousErrorLock.unlock()
        }
    }
    /// The document revision the next mutation will be based on.
    private(set) var baseDocumentRevision: UInt64 = 0
    /// The state revision paired atomically with `baseDocumentRevision` by
    /// the render accessor. It is intentionally not reconstructed through a
    /// separate state read.
    private(set) var stateRevision: UInt64 = 0
    private var nextRequestId: UInt64 = 0
    private(set) var lastRequestIdForTesting: UInt64?
    private(set) var backendEnvelopeCallCountForTesting = 0
    private(set) var renderUpdateCallCountForTesting = 0
    private var lastSyncedScalarSelection: (anchor: UInt32, head: UInt32)?
    private var cachedAuthoritativeScalarSelection: (anchor: UInt32, head: UInt32)?
    private var cachedScalarLength: UInt32?
    private var cachedActiveState: [String: Any]?
    private var cachedHistoryState: (canUndo: Bool, canRedo: Bool)?
    private var cachedViewUpdateJSON: String?
    /// Diagnostics: structured notes for adapter-path failures
    /// (mismatch refreshes, derivation failures) that never surface as
    /// autonomous error events.
    private(set) var debugNotes: [String] = []
    private let destroySession: (String) -> FfiUnitResult
    private let setAwarenessSelection: (String, String) -> FfiJsonResult
    private let collaborationWake: (UInt64, CollaborationWakeReason) -> Void
    private var destroyed = false

    var isDestroyed: Bool {
        destroyed
    }

    private init(
        editorId: String,
        roomBound: Bool,
        baseDocumentRevision: UInt64,
        destroySession: @escaping (String) -> FfiUnitResult,
        setAwarenessSelection: @escaping (String, String) -> FfiJsonResult,
        collaborationWake: @escaping (UInt64, CollaborationWakeReason) -> Void
    ) {
        self.editorId = editorId
        self.roomBound = roomBound
        self.baseDocumentRevision = baseDocumentRevision
        self.destroySession = destroySession
        self.setAwarenessSelection = setAwarenessSelection
        self.collaborationWake = collaborationWake
    }

    // MARK: - Construction

    /// Attach to an existing v2 session created through the module's
    /// JS-facing `editorV2Create` entry. The session is NOT re-created; the
    /// adapter routes the bound view's interactions through the shared
    /// session (the TS document handle and collaboration controller drive
    /// the same session over the module surface).
    static func attach(
        editorId: String,
        roomBound: Bool,
        destroySession: @escaping (String) -> FfiUnitResult = { editorV2Destroy(editorId: $0) },
        setAwarenessSelection: @escaping (String, String) -> FfiJsonResult = {
            editorV2CollaborationSetAwarenessSelection(editorId: $0, selectionJson: $1)
        },
        collaborationWake: @escaping (UInt64, CollaborationWakeReason) -> Void = {
            NativeCollaborationTransportRegistry.notifyOutboundAvailable(
                editorId: $0,
                reason: $1
            )
        }
    ) -> EditorV2Adapter? {
        guard isCanonicalDecimalEditorId(editorId) else {
            return nil
        }
        let adapter = EditorV2Adapter(
            editorId: editorId,
            roomBound: roomBound,
            baseDocumentRevision: 0,
            destroySession: destroySession,
            setAwarenessSelection: setAwarenessSelection,
            collaborationWake: collaborationWake
        )
        // Attachment only establishes that the handle is live. It must not
        // render (a render resolves an otherwise absent selection) or stamp
        // a state revision; the first actual refresh atomically adopts the
        // complete render snapshot before input is enabled.
        guard case .success = normalizeJsonResult(editorV2GetState(editorId: editorId)) else {
            return nil
        }
        return adapter
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

    /// Execute one destroy FFI call and preserve its exact terminal result.
    /// The public module transaction owns pairing removal and view teardown;
    /// this adapter owns only its local lifecycle and autonomous-error owner.
    @discardableResult
    func destroyForModuleTransaction() -> FfiUnitResult {
        let result = destroySession(editorId)
        switch (result.value, result.error) {
        case let (value?, nil) where value:
            clearAutonomousErrorOwner()
            destroyed = true
            return result
        case let (nil, error?) where error.domain == "lifecycle"
            && (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING"):
            clearAutonomousErrorOwner()
            destroyed = true
            return result
        case let (nil, error?):
            return FfiUnitResult(value: nil, error: error)
        default:
            return FfiUnitResult(
                value: nil,
                error: contractError("v2 destroy result violates the frozen unit-result shape")
            )
        }
    }

    /// Internal cleanup convenience. Public destroy routing must use
    /// `destroyForModuleTransaction()` so lifecycle terminal records remain
    /// observable at the module boundary.
    @discardableResult
    func destroy() -> FfiError? {
        if destroyed { return nil }
        let result = destroyForModuleTransaction()
        switch (result.value, result.error) {
        case let (value?, nil) where value:
            return nil
        case let (nil, error?) where error.domain == "lifecycle"
            && (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING"):
            return nil
        case let (nil, error?):
            return error
        default:
            return contractError("v2 destroy result violates the frozen unit-result shape")
        }
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
        dispatchAutonomousError(error)
    }

    /// A malformed externally supplied render is a permanent boundary
    /// rejection. Surface it once through the adapter's autonomous-error
    /// channel so the paired view can clear/report it, but do not also add a
    /// diagnostic note: that would expose a second observable emission for
    /// an otherwise atomic no-op.
    private func rejectAtomicRenderSnapshot() {
        dispatchAutonomousError(Self.contractError("v2 atomic render snapshot violates the frozen shape"))
    }

    /// View-side envelope failures (missing/mismatched source ids) have no
    /// engine call to classify. Route them through the same boundary error
    /// channel as a rejected external snapshot, once per discarded envelope.
    func rejectExternalRenderEnvelope(_ message: String) {
        dispatchAutonomousError(Self.contractError(message))
    }

    // MARK: - Exclusive autonomous-error owner

    /// A later claim replaces the earlier owner. Clearing is conditional on
    /// the same token, so stale views cannot erase a newer binding.
    func bindAutonomousErrorOwner(token: UUID, _ callback: @escaping (FfiError) -> Void) {
        autonomousErrorLock.lock()
        autonomousErrorOwnerToken = token
        autonomousErrorCallback = callback
        autonomousErrorLock.unlock()
    }

    func clearAutonomousErrorOwner(token: UUID) {
        autonomousErrorLock.lock()
        defer { autonomousErrorLock.unlock() }
        guard autonomousErrorOwnerToken == token else { return }
        autonomousErrorOwnerToken = nil
        autonomousErrorCallback = nil
    }

    func clearAutonomousErrorOwner() {
        autonomousErrorLock.lock()
        if autonomousErrorOwnerToken != nil {
            autonomousErrorOwnerToken = nil
            autonomousErrorCallback = nil
        }
        autonomousErrorLock.unlock()
    }

    func isAutonomousErrorOwner(token: UUID) -> Bool {
        autonomousErrorLock.lock()
        defer { autonomousErrorLock.unlock() }
        return autonomousErrorOwnerToken == token
    }

    private func dispatchAutonomousError(_ error: FfiError) {
        autonomousErrorLock.lock()
        let callback = autonomousErrorCallback
        autonomousErrorLock.unlock()
        callback?(error)
    }

    private func requestIdExhaustedError() -> FfiError {
        FfiError(
            domain: "boundary",
            code: "CONFIG_INVALID",
            message: "v2 request id counter exhausted",
            requestId: String(nextRequestId),
            operationIndex: nil,
            limit: String(UInt64.max),
            actual: nil,
            detailsJson: nil
        )
    }

    /// Serialize one request envelope with canonical decimal-string u64
    /// fields, so Foundation never bridges them through NSNumber.
    private func buildEnvelope(
        _ payload: [String: Any],
        includeBaseRevision: Bool = true
    ) -> EditorV2EnvelopeResult {
        guard nextRequestId < UInt64.max else {
            return .failure(requestIdExhaustedError())
        }
        nextRequestId += 1
        lastRequestIdForTesting = nextRequestId
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
        return .success("{\(parts.joined(separator: ","))}")
    }

    private func callWithEnvelope(
        _ payload: [String: Any],
        includeBaseRevision: Bool = true,
        _ call: (String) -> FfiJsonResult
    ) -> FfiJsonResult {
        switch buildEnvelope(payload, includeBaseRevision: includeBaseRevision) {
        case .success(let requestJson):
            backendEnvelopeCallCountForTesting += 1
            return call(requestJson)
        case .failure(let error):
            return FfiJsonResult(value: nil, error: error)
        }
    }

    func setNextRequestIdForTesting(_ requestId: UInt64) {
        nextRequestId = requestId
    }

    private func selectionEnvelope(anchor: UInt32, head: UInt32, affinity: String) -> [String: Any] {
        ["selection": EditorV2PositionBridge.textSelectionEnvelope(anchor: anchor, head: head, affinity: affinity)]
    }

    private static func uint64Field(_ object: [String: Any], _ key: String) -> UInt64? {
        guard let string = object[key] as? String,
              isCanonicalDecimalEditorId(string)
        else { return nil }
        return UInt64(string)
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

    /// One view-facing update plus the document's scalar extent (the lenient
    /// `UInt32.max` doc→scalar mapping, used to clamp transient-IME
    /// positions the way the legacy engine did).
    private struct EditorV2DerivedUpdate {
        let updateJSON: String
        let scalarLength: UInt32
    }

    private static func uint32Field(_ object: [String: Any], _ key: String) -> UInt32? {
        v2ExactUInt32(object[key] as? NSNumber)
    }

    private struct AtomicRenderSnapshot {
        let viewUpdateJSON: String
        let documentRevision: UInt64
        let stateRevision: UInt64
        let scalarLength: UInt32
        let selection: (anchor: UInt32, head: UInt32)?
        let activeState: [String: Any]
        let historyState: (canUndo: Bool, canRedo: Bool)
    }

    private static let atomicRenderSnapshotKeys: Set<String> = [
        "renderBlocks",
        "renderPatch",
        "selection",
        "activeState",
        "historyState",
        "documentVersion",
        "stateRevision",
        "scalarLength",
    ]

    private static let activeStateKeys: Set<String> = [
        "marks",
        "markAttrs",
        "nodes",
        "commands",
        "allowedMarks",
        "insertableNodes",
    ]

    private static let mentionThemeStringKeys: Set<String> = [
        "textColor",
        "backgroundColor",
        "borderColor",
        "popoverBackgroundColor",
        "popoverBorderColor",
        "popoverShadowColor",
        "optionTextColor",
        "optionSecondaryTextColor",
        "optionHighlightedBackgroundColor",
        "optionHighlightedTextColor",
    ]

    private static let mentionThemeNumberKeys: Set<String> = [
        "borderWidth",
        "borderRadius",
        "popoverBorderWidth",
        "popoverBorderRadius",
    ]

    private static let mentionThemeFontWeights: Set<String> = [
        "normal", "bold", "100", "200", "300", "400",
        "500", "600", "700", "800", "900",
    ]

    private static func exactBool(_ value: Any?) -> Bool? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) == CFBooleanGetTypeID()
        else {
            return nil
        }
        return number.boolValue
    }

    private static func finiteNumber(_ value: Any?) -> NSNumber? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID(),
              number.doubleValue.isFinite
        else {
            return nil
        }
        return number
    }

    private static func hasOnlyKeys(_ object: [String: Any], _ allowed: Set<String>) -> Bool {
        Set(object.keys).isSubset(of: allowed)
    }

    private static func isValidJSONValue(_ value: Any) -> Bool {
        if value is NSNull || value is String || exactBool(value) != nil || finiteNumber(value) != nil {
            return true
        }
        if let array = value as? [Any] {
            return array.allSatisfy(isValidJSONValue)
        }
        if let object = value as? [String: Any] {
            return object.values.allSatisfy(isValidJSONValue)
        }
        return false
    }

    private static func isValidRenderMark(_ value: Any) -> Bool {
        if value is String { return true }
        guard let object = value as? [String: Any],
              object["type"] is String
        else {
            return false
        }
        return object.values.allSatisfy(isValidJSONValue)
    }

    private static func isValidListContext(_ value: Any) -> Bool {
        guard let object = value as? [String: Any],
              hasOnlyKeys(
                object,
                ["ordered", "index", "total", "start", "isFirst", "isLast", "kind", "checked"]
              ),
              exactBool(object["ordered"]) != nil,
              uint32Field(object, "index") != nil,
              uint32Field(object, "total") != nil,
              uint32Field(object, "start") != nil,
              exactBool(object["isFirst"]) != nil,
              exactBool(object["isLast"]) != nil
        else {
            return false
        }
        if let kind = object["kind"], !(kind is NSNull), !(kind is String) { return false }
        if let checked = object["checked"], !(checked is NSNull), exactBool(checked) == nil { return false }
        return true
    }

    private static func isValidMentionTheme(_ value: Any) -> Bool {
        guard let object = value as? [String: Any] else { return false }
        let allowed = mentionThemeStringKeys
            .union(mentionThemeNumberKeys)
            .union(["fontWeight"])
        guard hasOnlyKeys(object, allowed) else { return false }
        for key in mentionThemeStringKeys where object[key] != nil {
            guard object[key] is String else { return false }
        }
        for key in mentionThemeNumberKeys where object[key] != nil {
            guard finiteNumber(object[key]) != nil else { return false }
        }
        if let fontWeight = object["fontWeight"] {
            guard let fontWeight = fontWeight as? String,
                  mentionThemeFontWeights.contains(fontWeight)
            else {
                return false
            }
        }
        return true
    }

    private static func isValidRenderElement(_ value: Any) -> Bool {
        guard let object = value as? [String: Any],
              let type = object["type"] as? String
        else {
            return false
        }
        switch type {
        case "textRun":
            guard Set(object.keys) == ["type", "text", "marks"],
                  object["text"] is String,
                  let marks = object["marks"] as? [Any]
            else {
                return false
            }
            return marks.allSatisfy(isValidRenderMark)
        case "blockStart":
            guard hasOnlyKeys(object, ["type", "nodeType", "depth", "listContext"]),
                  object["nodeType"] is String,
                  uint32Field(object, "depth") != nil
            else {
                return false
            }
            return object["listContext"].map(isValidListContext) ?? true
        case "blockEnd":
            return Set(object.keys) == ["type"]
        case "voidInline", "voidBlock":
            guard hasOnlyKeys(object, ["type", "nodeType", "docPos", "attrs"]),
                  object["nodeType"] is String,
                  uint32Field(object, "docPos") != nil
            else {
                return false
            }
            return object["attrs"].map { $0 is [String: Any] } ?? true
        case "opaqueInlineAtom":
            guard hasOnlyKeys(object, ["type", "nodeType", "label", "docPos", "mentionTheme"]),
                  object["nodeType"] is String,
                  object["label"] is String,
                  uint32Field(object, "docPos") != nil
            else {
                return false
            }
            return object["mentionTheme"].map(isValidMentionTheme) ?? true
        case "opaqueBlockAtom":
            return Set(object.keys) == ["type", "nodeType", "label", "docPos"]
                && object["nodeType"] is String
                && object["label"] is String
                && uint32Field(object, "docPos") != nil
        default:
            return false
        }
    }

    private static func isValidRenderBlocks(_ value: Any) -> Bool {
        guard let blocks = value as? [Any] else { return false }
        return blocks.allSatisfy { block in
            guard let elements = block as? [Any] else { return false }
            return elements.allSatisfy(isValidRenderElement)
        }
    }

    private static func isValidRenderPatch(_ value: Any) -> Bool {
        if value is NSNull { return true }
        guard let object = value as? [String: Any],
              Set(object.keys) == ["startIndex", "deleteCount", "renderBlocks"],
              uint32Field(object, "startIndex") != nil,
              uint32Field(object, "deleteCount") != nil,
              let renderBlocks = object["renderBlocks"],
              isValidRenderBlocks(renderBlocks)
        else {
            return false
        }
        return true
    }

    private static func isBooleanRecord(_ value: Any?) -> Bool {
        guard let object = value as? [String: Any] else { return false }
        return object.values.allSatisfy { exactBool($0) != nil }
    }

    private static func isStringArray(_ value: Any?) -> Bool {
        guard let array = value as? [Any] else { return false }
        return array.allSatisfy { $0 is String }
    }

    private static func isValidActiveState(_ value: Any) -> Bool {
        guard let object = value as? [String: Any],
              Set(object.keys) == activeStateKeys,
              isBooleanRecord(object["marks"]),
              let markAttrs = object["markAttrs"] as? [String: Any],
              markAttrs.values.allSatisfy({ $0 is [String: Any] }),
              isBooleanRecord(object["nodes"]),
              isBooleanRecord(object["commands"]),
              isStringArray(object["allowedMarks"]),
              isStringArray(object["insertableNodes"])
        else {
            return false
        }
        return true
    }

    private static func scalarSelection(from value: Any) -> (anchor: UInt32, head: UInt32)? {
        guard let selection = value as? [String: Any],
              let type = selection["type"] as? String
        else {
            return nil
        }
        switch type {
        case "text":
            guard Set(selection.keys) == ["type", "anchor", "head", "anchorScalar", "headScalar"],
                  uint32Field(selection, "anchor") != nil,
                  uint32Field(selection, "head") != nil,
                  let anchor = uint32Field(selection, "anchorScalar"),
                  let head = uint32Field(selection, "headScalar")
            else {
                return nil
            }
            return (anchor, head)
        case "node":
            guard Set(selection.keys) == ["type", "pos", "posScalar"],
                  uint32Field(selection, "pos") != nil,
                  uint32Field(selection, "posScalar") != nil
            else {
                return nil
            }
            return nil
        case "all":
            return Set(selection.keys) == ["type"] ? nil : nil
        default:
            return nil
        }
    }

    private static func isValidSelection(_ value: Any) -> Bool {
        guard let selection = value as? [String: Any],
              let type = selection["type"] as? String
        else {
            return false
        }
        switch type {
        case "text":
            return scalarSelection(from: selection) != nil
        case "node":
            return Set(selection.keys) == ["type", "pos", "posScalar"]
                && uint32Field(selection, "pos") != nil
                && uint32Field(selection, "posScalar") != nil
        case "all":
            return Set(selection.keys) == ["type"]
        default:
            return false
        }
    }

    private static func parseAtomicRenderSnapshot(_ json: String) -> AtomicRenderSnapshot? {
        guard let data = json.data(using: .utf8),
              var object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys) == atomicRenderSnapshotKeys,
              let renderBlocks = object["renderBlocks"],
              isValidRenderBlocks(renderBlocks),
              let renderPatch = object["renderPatch"],
              isValidRenderPatch(renderPatch),
              let selectionValue = object["selection"],
              isValidSelection(selectionValue),
              let activeState = object["activeState"] as? [String: Any],
              isValidActiveState(activeState),
              let history = object["historyState"] as? [String: Any],
              Set(history.keys) == ["canUndo", "canRedo"],
              let canUndo = exactBool(history["canUndo"]),
              let canRedo = exactBool(history["canRedo"]),
              let documentRevision = uint64Field(object, "documentVersion"),
              let stateRevision = uint64Field(object, "stateRevision"),
              let scalarLength = uint32Field(object, "scalarLength")
        else {
            return nil
        }

        let selection = scalarSelection(from: selectionValue)
        object.removeValue(forKey: "scalarLength")
        guard let viewData = try? JSONSerialization.data(withJSONObject: object),
              let viewUpdateJSON = String(data: viewData, encoding: .utf8)
        else {
            return nil
        }
        return AtomicRenderSnapshot(
            viewUpdateJSON: viewUpdateJSON,
            documentRevision: documentRevision,
            stateRevision: stateRevision,
            scalarLength: scalarLength,
            selection: selection,
            activeState: activeState,
            historyState: (canUndo, canRedo)
        )
    }

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
            guard let snapshot = Self.parseAtomicRenderSnapshot(json) else {
                debugNotes.append("renderUpdate shape invalid")
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
        // This is the engine's selection from the locked snapshot. Keep it
        // distinct from a caller-provided mirror: treating it as a mirror on
        // the next refresh would change the frozen no-mirror render shape.
        cachedAuthoritativeScalarSelection = snapshot.selection
        return EditorV2DerivedUpdate(updateJSON: updateJSON, scalarLength: snapshot.scalarLength)
    }

    /// Atomically parse and adopt a render supplied by JS before the paired
    /// native view accepts further input. The caller applies the returned
    /// payload synchronously in the same editor-scoped operation.
    func adoptExternalRender(_ renderJSON: String) -> String? {
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
        return adopted.updateJSON
    }

    /// Validate an externally supplied atomic render without adopting it.
    /// A preflight commit can make an otherwise valid render stale; callers
    /// still need to reject malformed envelopes exactly once before replacing
    /// that stale render with a current atomic refresh.
    func validateExternalRender(_ renderJSON: String) -> Bool {
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

    var cacheStateForTesting: String {
        let selection: Any
        if let cachedAuthoritativeScalarSelection {
            selection = [
                "anchor": cachedAuthoritativeScalarSelection.anchor,
                "head": cachedAuthoritativeScalarSelection.head,
            ]
        } else {
            selection = NSNull()
        }
        let history: Any
        if let cachedHistoryState {
            history = ["canUndo": cachedHistoryState.canUndo, "canRedo": cachedHistoryState.canRedo]
        } else {
            history = NSNull()
        }
        let object: [String: Any] = [
            "documentRevision": String(baseDocumentRevision),
            "stateRevision": String(stateRevision),
            "scalarLength": cachedScalarLength.map(NSNumber.init(value:)) ?? NSNull(),
            "selection": selection,
            "activeState": cachedActiveState ?? NSNull(),
            "historyState": history,
            "viewUpdateJSON": cachedViewUpdateJSON ?? NSNull(),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) else {
            return ""
        }
        return String(data: data, encoding: .utf8) ?? ""
    }

    /// Re-read the authoritative v2 state and derive one synthesized update
    /// JSON through the v2 render accessor. Updates revision tracking
    /// and the scalar-extent cache. This is the REVISION_MISMATCH recovery
    /// path: it never re-issues the failed operation.
    @discardableResult
    private func refreshInternal(
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
        refreshInternal(mirrorSelection: mirrorSelection)?.updateJSON
    }

    /// Synthesized current-state update (selection/activeState included,
    /// mirroring the legacy `editorGetCurrentState` contract).
    func currentStateJSON() -> String? {
        return refreshInternal(mirrorSelection: lastSyncedScalarSelection)?.updateJSON
    }

    /// The initial bind render. The host passes this exact snapshot directly
    /// to the text view and toolbar, so it must not be replayed by a later
    /// independent current-state read.
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
        if let cachedHistoryState { return cachedHistoryState }
        guard refreshInternal(mirrorSelection: nil) != nil else { return nil }
        return cachedHistoryState
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
        var result = callWithEnvelope(
                selectionEnvelope(
                    anchor: clampedAnchor,
                    head: clampedHead,
                    affinity: collapsed ? "after" : "before"
                )
        ) { requestJson in
            editorV2SetSelection(editorId: self.editorId, requestJson: requestJson)
        }
        if collapsed, let error = result.error, error.code == "POSITION_INVALID" {
            result = callWithEnvelope(
                    selectionEnvelope(anchor: clampedAnchor, head: clampedHead, affinity: "before")
            ) { requestJson in
                editorV2SetSelection(editorId: self.editorId, requestJson: requestJson)
            }
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
        let mapping: (docAnchor: UInt32, docHead: UInt32)?
        switch ensureSelection(anchor: anchor, head: head) {
        case .ok:
            mapping = resolveSelectionMapping(scalarAnchor: anchor, scalarHead: head)
        case .refreshed(let updateJSON):
            mapping = Self.textDocumentSelection(from: updateJSON)
        case .failed:
            return nil
        }
        guard let mapping else { return nil }
        publishCollaborationSelection(
            docAnchor: mapping.docAnchor,
            docHead: mapping.docHead
        )
        return mapping
    }

    private static func textDocumentSelection(
        from updateJSON: String
    ) -> (docAnchor: UInt32, docHead: UInt32)? {
        guard let data = updateJSON.data(using: .utf8),
              let update = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let selection = update["selection"] as? [String: Any],
              selection["type"] as? String == "text",
              let docAnchor = uint32Field(selection, "anchor"),
              let docHead = uint32Field(selection, "head")
        else {
            return nil
        }
        return (docAnchor, docHead)
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
        let mapping: (docAnchor: UInt32, docHead: UInt32)?
        switch ensureSelection(anchor: anchor, head: head) {
        case .ok:
            guard let selection = lastSyncedScalarSelection else { return }
            mapping = resolveSelectionMapping(
                scalarAnchor: selection.anchor,
                scalarHead: selection.head
            )
        case .refreshed(let updateJSON):
            mapping = Self.textDocumentSelection(from: updateJSON)
        case .failed:
            return
        }
        guard let mapping else { return }
        publishCollaborationSelection(
            docAnchor: mapping.docAnchor,
            docHead: mapping.docHead
        )
    }

    private func publishCachedCollaborationSelection() {
        guard let selection = cachedAuthoritativeScalarSelection,
              let mapping = resolveSelectionMapping(
                  scalarAnchor: selection.anchor,
                  scalarHead: selection.head
              )
        else {
            return
        }
        publishCollaborationSelection(
            docAnchor: mapping.docAnchor,
            docHead: mapping.docHead
        )
    }

    private func publishCollaborationSelection(docAnchor: UInt32, docHead: UInt32) {
        guard roomBound, let nativeEditorId = UInt64(editorId) else { return }
        let selection: [String: Any] = [
            "type": "text",
            "anchor": Int(docAnchor),
            "head": Int(docHead),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: selection),
              let selectionJSON = String(data: data, encoding: .utf8)
        else {
            emit(contractError("awareness selection serialization failed"))
            return
        }
        switch Self.normalizeJsonResult(
            setAwarenessSelection(editorId, selectionJSON)
        ) {
        case .failure(let error):
            emit(error)
        case .success(let value):
            guard let data = value.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  Set(object.keys) == ["outboundChanged"],
                  let outboundChanged = Self.exactBool(object["outboundChanged"])
            else {
                emit(contractError("awareness selection result violates the frozen shape"))
                return
            }
            if outboundChanged {
                collaborationWake(nativeEditorId, .awareness)
            }
        }
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
            let changed: Bool
            switch outcome.kind {
            case .transaction(let didChange, let revision):
                changed = didChange
                baseDocumentRevision = revision
                if let postSelectionMirror {
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
                mirrorSelection: postSelectionMirror ?? (includeSelectionInUpdate ? preSelection : nil),
                // Paste and composition-preserving paths still derive active
                // and history state from the authoritative post-operation
                // selection. Only the view-facing selection is omitted so
                // UIKit retains its IME-owned caret.
                strippingViewSelection: postSelectionMirror == nil && !includeSelectionInUpdate
            )?.updateJSON else {
                return nil
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

    // MARK: - Native transport wake

    /// A successful room mutation wakes the handle-owned native driver.
    /// The adapter never owns a generation, socket, frame, or retry timer.
    private func notifyCollaborationMutation() {
        guard roomBound, let nativeEditorId = UInt64(editorId) else { return }
        collaborationWake(nativeEditorId, .localMutation)
    }

    // MARK: - Typed verbs (one method per legacy choke point)

    func insertText(_ text: String, atScalar scalarPos: UInt32) -> String? {
        guard !text.isEmpty else { return currentStateJSON() }
        let postCaret = scalarPos &+ EditorV2PositionBridge.scalarLength(of: text)
        return performMutation(
            preSelection: (scalarPos, scalarPos),
            postSelectionMirror: (postCaret, postCaret)
        ) {
            self.callWithEnvelope(["text": text]) { requestJson in
                editorV2ApplyInput(editorId: self.editorId, requestJson: requestJson)
            }
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
            self.callWithEnvelope([
                "command": ["type": "replaceSelectionText", "text": text]
            ]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func deleteScalarRange(from: UInt32, to: UInt32) -> String? {
        guard from < to else { return currentStateJSON() }
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
            self.callWithEnvelope(["command": ["type": "deleteBackward"]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func splitBlock(atScalar scalarPos: UInt32) -> String? {
        // The caret lands at the start of the new block: one scalar past the
        // split point (the block separator counts as one scalar).
        performMutation(
            preSelection: (scalarPos, scalarPos),
            postSelectionMirror: (scalarPos &+ 1, scalarPos &+ 1)
        ) {
            self.callWithEnvelope(["command": ["type": "splitBlock"]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func deleteAndSplit(from: UInt32, to: UInt32) -> String? {
        performMutation(
            preSelection: (from, to),
            postSelectionMirror: (from, from)
        ) {
            self.callWithEnvelope(["command": ["type": "deleteAndSplit"]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
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
        performMutation(preSelection: (anchor, head)) {
            self.callWithEnvelope(["command": ["type": "insertContentHtml", "html": html]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    /// Paste-HTML path: the view pre-syncs the UIKit selection; the content
    /// insert applies at the engine selection.
    func insertContentHtmlAtEngineSelection(_ html: String) -> String? {
        performMutation {
            self.callWithEnvelope(["command": ["type": "insertContentHtml", "html": html]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
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
            self.callWithEnvelope(["command": ["type": "insertContentJson", "json": fragment]]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
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
            self.callWithEnvelope(["command": command]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    func resizeImage(atDocPos docPos: UInt32, width: UInt32, height: UInt32) -> String? {
        guard let scalar = scalarPosition(forDoc: docPos) else { return nil }
        return performMutation {
            self.callWithEnvelope([
                    "command": [
                        "type": "resizeImage",
                        "at": EditorV2PositionBridge.positionEnvelope(scalar: scalar),
                        "width": Int(width),
                        "height": Int(height),
                    ] as [String: Any],
            ]) { requestJson in
                editorV2ApplyCommand(editorId: self.editorId, requestJson: requestJson)
            }
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
            self.callWithEnvelope(["setHtml": html, "history": "resetAndClear"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
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
            self.callWithEnvelope(["setJson": document, "history": "resetAndClear"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }

    /// Undoable whole-document replace (legacy `editorReplaceHtml` parity:
    /// one undoable local-API boundary, selection preserved where possible).
    func replaceContentHtml(_ html: String) -> String? {
        performMutation(postSelectionMirror: (0, 0), includeSelectionInUpdate: true) {
            self.callWithEnvelope(["setHtml": html, "history": "undoableBoundary"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
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
            self.callWithEnvelope(["setJson": document, "history": "undoableBoundary"]) { requestJson in
                editorV2ApplyLocalApi(editorId: self.editorId, requestJson: requestJson)
            }
        }
    }
}

// MARK: - Session pairing registry

func v2DestroyAlreadyInProgressError() -> FfiError {
    FfiError(
        domain: "operation",
        code: "OPERATION_INVALID",
        message: "destroy already in progress",
        requestId: nil,
        operationIndex: nil,
        limit: nil,
        actual: nil,
        detailsJson: nil
    )
}

func invokeDestroyTestingHook(
    _ hook: ((UInt64) throws -> Void)?,
    editorId: UInt64
) {
    do {
        try hook?(editorId)
    } catch {
        // Destroy test hooks are observational and cannot alter the transaction.
    }
}

/// Maps the public (module-visible) editor id to the v2 adapter backing it.
/// The module's create path registers one pairing per editor; views bound
/// to a paired id route every interaction through the adapter.
enum EditorV2Registry {
    private static let lock = NSLock()
    private static var pairings: [UInt64: EditorV2Adapter] = [:]
    private static var destroyingLegacyIds: Set<UInt64> = []
    static var onHandleDestroyReservationAcquiredForTesting: ((UInt64) throws -> Void)?
    static var onDestroyFfiResultReceivedForTesting: ((UInt64) throws -> Void)?
    static var onPairRemovedBeforeDestroyFinalizationForTesting: ((UInt64) throws -> Void)?

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

    /// Acquire canonical public-handle ownership before consulting the
    /// pairing. Pair removal is part of terminal teardown, so the pairing
    /// cannot be the source of truth for transaction contention.
    static func acquireHandleDestroyReservation(forLegacyId legacyId: UInt64) -> Bool {
        lock.lock()
        guard !destroyingLegacyIds.contains(legacyId) else {
            lock.unlock()
            return false
        }
        destroyingLegacyIds.insert(legacyId)
        lock.unlock()
        invokeDestroyTestingHook(onHandleDestroyReservationAcquiredForTesting, editorId: legacyId)
        return true
    }

    static func releaseHandleDestroyReservation(forLegacyId legacyId: UInt64) {
        lock.lock()
        destroyingLegacyIds.remove(legacyId)
        lock.unlock()
    }

    static func isHandleDestroyReservedForTesting(_ legacyId: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return destroyingLegacyIds.contains(legacyId)
    }

    @discardableResult
    static func removePairing(forLegacyId legacyId: UInt64) -> EditorV2Adapter? {
        lock.lock()
        defer { lock.unlock() }
        return pairings.removeValue(forKey: legacyId)
    }

    /// Destroy the v2 session backing a pairing and drop the pairing only
    /// once native destruction has succeeded or the session is already gone.
    @discardableResult
    static func destroyPair(forLegacyId legacyId: UInt64) -> FfiError? {
        destroyEditorV2FromModule(editorId: String(legacyId)).error
    }

}
