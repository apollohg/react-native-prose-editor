import Foundation

// MARK: - v2 editor adapter (the only construction path; Task 16 production cutover)
//
// `EditorV2Adapter` owns one v2 editor session (decimal-string handle) and
// translates the existing native view operations into the typed v2
// transactions/results (`editorV2*`). Every mutation is one typed
// transaction against the tracked base document revision. Transient
// IME/composing state never reaches the adapter — only final commits do.
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

enum EditorV2EnvelopeResult {
    case success(String)
    case failure(FfiError)
}

struct EditorV2SelectionSync {
    let docAnchor: UInt32
    let docHead: UInt32
    let refreshedUpdateJSON: String?
}

/// The v2 session adapter backing one bound editor view. See the file
/// header for the architecture and the render derivation notes.
final class EditorV2Adapter {
    private enum LifecycleState {
        case active
        case destroying
        case destroyed
    }

    /// Decimal-string v2 session handle.
    let editorId: String
    /// Whether the session is room-bound (owns a collaboration outbox).
    let roomBound: Bool

    /// Autonomous structured failures (input/accessibility/lifecycle), one
    /// event per failure, mirroring the v2 error-listener contract.
    let autonomousErrorLock = NSLock()
    var autonomousErrorCallback: ((FfiError) -> Void)?
    var autonomousErrorOwnerToken: UUID?

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
    var baseDocumentRevision: UInt64 = 0
    /// The state revision paired atomically with `baseDocumentRevision` by
    /// the render accessor. It is intentionally not reconstructed through a
    /// separate state read.
    var stateRevision: UInt64 = 0
    var nextRequestId: UInt64 = 0
    var nativeOwnerId: UInt64?
    var nativeOwnerToken: UUID?
    var positionEpoch: UInt64?
    var lastRequestIdForTesting: UInt64?
    var backendEnvelopeCallCountForTesting = 0
    var renderUpdateCallCountForTesting = 0
    var onRemoteRecoveryForTesting: (() -> Void)?
    var lastSyncedScalarSelection: (anchor: UInt32, head: UInt32)?
    var cachedAuthoritativeScalarSelection: (anchor: UInt32, head: UInt32)?
    var cachedScalarLength: UInt32?
    var cachedActiveState: [String: Any]?
    var cachedHistoryState: (canUndo: Bool, canRedo: Bool)?
    var cachedViewUpdateJSON: String?
    var cachedAtomicRenderJSON: String?
    var cachedAtomicRenderDocumentRevision: UInt64?
    /// Diagnostics: structured notes for adapter-path failures
    /// (mismatch refreshes, derivation failures) that never surface as
    /// autonomous error events.
    var debugNotes: [String] = []
    private let destroySession: (String) -> FfiUnitResult
    let setAwarenessSelection: (String, String) -> FfiJsonResult
    let collaborationWake: (UInt64, CollaborationWakeReason) -> Void
    let runtimeLock = NSRecursiveLock()
    private var lifecycleState = LifecycleState.active
    var destroyed = false

    static let nativeOwnerLock = NSLock()
    static var nextNativeOwnerId: UInt64 = 0

    var isDestroyed: Bool {
        runtimeLock.lock()
        defer { runtimeLock.unlock() }
        return lifecycleState == .destroyed
    }

    func beginRuntimeOperation() -> Bool {
        runtimeLock.lock()
        guard lifecycleState == .active else {
            let destroying = lifecycleState == .destroying
            emit(
                FfiError(
                    domain: "lifecycle",
                    code: destroying ? "ENGINE_DESTROYING" : "ENGINE_DESTROYED",
                    message: destroying
                        ? "editor session is being destroyed"
                        : "editor session is destroyed",
                    requestId: nil,
                    operationIndex: nil,
                    limit: nil,
                    actual: nil,
                    detailsJson: nil
                )
            )
            runtimeLock.unlock()
            return false
        }
        return true
    }

    func endRuntimeOperation() {
        runtimeLock.unlock()
    }

    func performRuntimeOperation<Result>(
        unavailable: @autoclosure () -> Result,
        _ operation: () -> Result
    ) -> Result {
        guard beginRuntimeOperation() else { return unavailable() }
        defer { endRuntimeOperation() }
        return operation()
    }

    func performRuntimeLifecycleOperation<Result>(
        unavailable: @autoclosure () -> Result,
        _ operation: () -> Result
    ) -> Result {
        runtimeLock.lock()
        guard lifecycleState != .destroyed else {
            runtimeLock.unlock()
            return unavailable()
        }
        defer { runtimeLock.unlock() }
        return operation()
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

    static func isCanonicalDecimalEditorId(_ editorId: String) -> Bool {
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
    func destroyForModuleTransaction(
        beforeDestroy: () -> Void = {}
    ) -> FfiUnitResult {
        runtimeLock.lock()
        switch lifecycleState {
        case .destroying:
            runtimeLock.unlock()
            return FfiUnitResult(value: nil, error: v2DestroyAlreadyInProgressError())
        case .destroyed:
            runtimeLock.unlock()
            return FfiUnitResult(
                value: nil,
                error: FfiError(
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
        case .active:
            lifecycleState = .destroying
        }
        runtimeLock.unlock()

        beforeDestroy()

        runtimeLock.lock()
        releaseNativeOwner()
        let result = destroySession(editorId)
        let normalized: FfiUnitResult
        switch (result.value, result.error) {
        case let (value?, nil) where value:
            clearAutonomousErrorOwner()
            destroyed = true
            lifecycleState = .destroyed
            normalized = result
        case let (nil, error?) where error.domain == "lifecycle"
            && (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING"):
            clearAutonomousErrorOwner()
            destroyed = true
            lifecycleState = .destroyed
            normalized = result
        case let (nil, error?):
            lifecycleState = .active
            normalized = FfiUnitResult(value: nil, error: error)
        default:
            lifecycleState = .active
            normalized = FfiUnitResult(
                value: nil,
                error: contractError("v2 destroy result violates the frozen unit-result shape")
            )
        }
        runtimeLock.unlock()
        return normalized
    }

    /// Internal cleanup convenience. Public destroy routing must use
    /// `destroyForModuleTransaction()` so lifecycle terminal records remain
    /// observable at the module boundary.
    @discardableResult
    func destroy() -> FfiError? {
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

    // MARK: - Exclusive autonomous-error owner

    // MARK: - Render derivation (v2 render accessor)

    static let atomicRenderSnapshotKeys: Set<String> = [
        "renderBlocks",
        "renderPatch",
        "selection",
        "activeState",
        "historyState",
        "documentVersion",
        "stateRevision",
        "scalarLength",
        "documentIsEmpty",
    ]

    static let activeStateKeys: Set<String> = [
        "marks",
        "markAttrs",
        "nodes",
        "commands",
        "allowedMarks",
        "insertableNodes",
    ]

    static let mentionNodeStringKeys: Set<String> = [
        "textColor",
        "backgroundColor",
        "borderColor",
    ]

    static let mentionOptionStringKeys: Set<String> = [
        "textColor",
        "secondaryTextColor",
        "backgroundColor",
        "borderColor",
        "highlightedBackgroundColor",
        "highlightedTextColor",
    ]

    static let mentionSuggestionsStringKeys: Set<String> = [
        "backgroundColor",
        "borderColor",
        "shadowColor",
    ]

    static let mentionThemeNumberKeys: Set<String> = [
        "borderWidth",
        "borderRadius",
    ]

    static let mentionThemeFontWeights: Set<String> = [
        "normal", "bold", "100", "200", "300", "400",
        "500", "600", "700", "800", "900",
    ]

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

    // MARK: - Selection sync and position mapping

    // MARK: - Mutation driver

    // MARK: - Native transport wake

    // MARK: - Typed verbs (one method per legacy choke point)

    // MARK: - Controlled content (local-API, passes read-only per Source::Api parity)

}
