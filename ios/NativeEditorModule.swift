@preconcurrency public import ExpoModulesCore
import UIKit

/// Test-facing parser for the v2 create value. Handles are never bridged
/// through signed native numbers: the frozen wire shape is a canonical
/// decimal string.
func createdEditorId(_ resultJson: String) -> String? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          result["error"] == nil,
          let editorId = result["editorId"] as? String,
          let value = v2UInt64Argument(editorId),
          value > 0,
          editorId == String(value)
    else {
        return nil
    }
    return editorId
}

// MARK: - v2 result-record bridging (frozen {value, error} contract)

/// One FfiError as the plain dictionary the TS boundary normalizes
/// (`normalizeNativeEditorV2Error`): required domain/code/message plus the
/// optional fields only when present.
private func v2FfiErrorDictionary(_ error: FfiError) -> [String: Any] {
    var dictionary: [String: Any] = [
        "domain": error.domain,
        "code": error.code,
        "message": error.message,
    ]
    if let requestId = error.requestId { dictionary["requestId"] = requestId }
    if let operationIndex = error.operationIndex { dictionary["operationIndex"] = operationIndex }
    if let limit = error.limit { dictionary["limit"] = limit }
    if let actual = error.actual { dictionary["actual"] = actual }
    if let detailsJson = error.detailsJson { dictionary["detailsJson"] = detailsJson }
    return dictionary
}

/// A boundary-fabricated error record for arguments that cannot reach Rust
/// (non-canonical generation strings and the like).
private func v2ContractErrorDictionary(_ message: String) -> [String: Any] {
    [
        "domain": "boundary",
        "code": "FFI_RESULT_INVALID",
        "message": message,
    ]
}

private func v2InvalidResultDictionary(_ message: String) -> [String: Any] {
    ["error": v2ContractErrorDictionary(message)]
}

private func v2ResultDictionary<Value>(
    value: Value?,
    error: FfiError?,
    mapValue: (Value) -> Any
) -> [String: Any] {
    switch (value, error) {
    case let (value?, nil):
        return ["value": mapValue(value)]
    case let (nil, error?):
        return ["error": v2FfiErrorDictionary(error)]
    default:
        return v2InvalidResultDictionary("v2 result must carry exactly one of value/error")
    }
}

private func v2JsonResultDictionary(_ result: FfiJsonResult) -> [String: Any] {
    v2ResultDictionary(value: result.value, error: result.error, mapValue: { $0 })
}

private func v2UnitResultDictionary(_ result: FfiUnitResult) -> [String: Any] {
    v2ResultDictionary(value: result.value, error: result.error, mapValue: { $0 })
}

func v2CollaborationUnitResultDictionary(
    editorId: String,
    operation: (String) -> FfiUnitResult
) -> [String: Any] {
    v2UnitResultDictionary(operation(editorId))
}

/// Boundary helper retained for canonical monotonic-u64 coverage. Production
/// transport timing is native-owned and calls the generated drive function
/// directly rather than exporting a JavaScript tick.
func v2CollaborationTickResultDictionary(
    editorId: String,
    nowMillis: String,
    tick: (String, String) -> FfiJsonResult
) -> [String: Any] {
    guard let canonicalNowMillis = v2CanonicalUInt64String(nowMillis) else {
        return v2InvalidResultDictionary("invalid nowMillis")
    }
    return v2JsonResultDictionary(tick(editorId, canonicalNowMillis))
}

private func v2MutationResultDictionary(
    editorId: String,
    result: FfiJsonResult
) -> [String: Any] {
    if result.error == nil,
       let value = result.value,
       let data = value.data(using: .utf8),
       let outcome = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
       outcome["changed"] as? Bool == true,
       let nativeEditorId = v2UInt64Argument(editorId)
    {
        NativeCollaborationTransportRegistry.notifyOutboundAvailable(
            editorId: nativeEditorId,
            reason: .moduleMutation
        )
    }
    return v2JsonResultDictionary(result)
}

private func v2SnapshotExportResultDictionary(_ result: FfiSnapshotExportResult) -> [String: Any] {
    v2ResultDictionary(value: result.value, error: result.error) { value in
        [
            "metadataJson": value.metadataJson,
            "encodedState": value.encodedState,
        ]
    }
}

/// Canonical decimal u64 values are the only v2 wire representation.
func v2CanonicalUInt64String(_ raw: String) -> String? {
    guard !raw.isEmpty,
          raw.allSatisfy({ $0 >= "0" && $0 <= "9" }),
          raw == "0" || raw.first != "0",
          UInt64(raw) != nil
    else {
        return nil
    }
    return raw
}

/// Native-only conversion after canonical syntax and range verification.
private func v2UInt64Argument(_ raw: String) -> UInt64? {
    guard let canonical = v2CanonicalUInt64String(raw) else { return nil }
    return UInt64(canonical)
}

func applyNativeEditorIdProp(_ raw: String, to view: NativeEditorExpoView) {
    guard let editorId = v2UInt64Argument(raw) else { return }
    view.setEditorId(editorId)
}

/// NSNumber is the Expo/Foundation numeric boundary. Do not use `uint32Value`:
/// it truncates fractions and wraps values outside the u32 domain.
func v2ExactUInt32(_ raw: NSNumber?) -> UInt32? {
    guard let raw,
          CFGetTypeID(raw) != CFBooleanGetTypeID()
    else {
        return nil
    }
    if let decimal = raw as? NSDecimalNumber {
        let maximum = NSDecimalNumber(value: UInt32.max)
        guard decimal != NSDecimalNumber.notANumber,
              decimal.compare(NSDecimalNumber.zero) != .orderedAscending,
              decimal.compare(maximum) != .orderedDescending
        else {
            return nil
        }
        let rounded = decimal.rounding(
            accordingToBehavior: NSDecimalNumberHandler(
                roundingMode: .down,
                scale: 0,
                raiseOnExactness: false,
                raiseOnOverflow: false,
                raiseOnUnderflow: false,
                raiseOnDivideByZero: false
            )
        )
        guard decimal.compare(rounded) == .orderedSame,
              let exact = UInt32(decimal.stringValue)
        else {
            return nil
        }
        return exact
    }
    let value = raw.doubleValue
    guard value.isFinite,
          value >= 0,
          value.rounded(.towardZero) == value,
          value <= Double(UInt32.max)
    else {
        return nil
    }
    return UInt32(exactly: value)
}

/// A created v2 handle preserves its exact engine-issued string for adapter
/// binding while retaining the current numeric native-view registry id.
private struct CreatedV2SessionHandle {
    let handle: String
    let nativeViewId: UInt64
}

/// Parse the created v2 session's canonical decimal handle from the frozen
/// create value (`{"editorId":"<decimal>"}`). The adapter must receive the
/// exact engine-issued string rather than a numeric reconstruction.
private func createdV2SessionHandle(_ resultJson: String) -> CreatedV2SessionHandle? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          result.count == 1,
          let editorIdString = result["editorId"] as? String,
          let editorId = v2UInt64Argument(editorIdString),
          editorId > 0,
          editorIdString == String(editorId)
    else {
        return nil
    }
    return CreatedV2SessionHandle(handle: editorIdString, nativeViewId: editorId)
}

/// A malformed create record may still expose a safely canonical handle. It
/// is not acceptable for pairing, but it is safe to destroy so Rust does not
/// retain an otherwise unreachable session.
private func cleanupCreatedV2SessionHandle(_ resultJson: String) -> CreatedV2SessionHandle? {
    guard let data = resultJson.data(using: .utf8),
          let result = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let editorIdString = result["editorId"] as? String,
          let editorId = v2UInt64Argument(editorIdString),
          editorId > 0,
          editorIdString == String(editorId)
    else {
        return nil
    }
    return CreatedV2SessionHandle(handle: editorIdString, nativeViewId: editorId)
}

private func cleanupCreatedV2Session(
    value: String?,
    destroy: (String) -> FfiUnitResult
) {
    guard let value,
          let handle = cleanupCreatedV2SessionHandle(value)
    else {
        return
    }
    _ = destroy(handle.handle)
}

/// Create a public v2 session and register its paired native view adapter
/// only after the FFI record is exact and the create value parses strictly.
/// The injected functions keep the exact-result boundary independently testable.
func createEditorV2SessionFromModule(
    configJson: String,
    snapshotState: Data?,
    create: (String, Data?) -> FfiJsonResult = { configJson, snapshotState in
        editorV2Create(configJson: configJson, snapshotState: snapshotState)
    },
    destroy: (String) -> FfiUnitResult = { editorId in
        editorV2Destroy(editorId: editorId)
    }
) -> [String: Any] {
    let result = create(configJson, snapshotState)
    switch (result.value, result.error) {
    case let (value?, nil):
        guard let createdHandle = createdV2SessionHandle(value) else {
            cleanupCreatedV2Session(value: value, destroy: destroy)
            return v2InvalidResultDictionary("v2 create value carries no canonical editor id")
        }
        guard let adapter = EditorV2Adapter.attach(
            editorId: createdHandle.handle,
            roomBound: v2ConfigIndicatesRoomBinding(configJson)
        ) else {
            _ = destroy(createdHandle.handle)
            return v2InvalidResultDictionary("v2 create could not bind its editor handle")
        }
        EditorV2Registry.register(adapter, forLegacyId: createdHandle.nativeViewId)
        NativeEditorViewRegistry.shared.markEditorCreated(editorId: createdHandle.nativeViewId)
        return v2JsonResultDictionary(result)
    case (nil, _?):
        return v2JsonResultDictionary(result)
    default:
        cleanupCreatedV2Session(value: result.value, destroy: destroy)
        return v2JsonResultDictionary(result)
    }
}

/// One canonical-handle destroy transaction. The handle reservation is the
/// contention authority because pairings are removed before native view
/// finalization. A paired adapter supplies its own FFI so its error owner is
/// committed only for exact terminal records; an unpaired session calls the
/// supplied public FFI directly.
func destroyEditorV2FromModule(
    editorId: String,
    destroy: (String) -> FfiUnitResult = { editorId in
        editorV2Destroy(editorId: editorId)
    }
) -> FfiUnitResult {
    guard let nativeViewId = v2UInt64Argument(editorId), nativeViewId > 0 else {
        return destroy(editorId)
    }
    guard EditorV2Registry.acquireHandleDestroyReservation(forLegacyId: nativeViewId) else {
        return FfiUnitResult(value: nil, error: v2DestroyAlreadyInProgressError())
    }
    defer {
        // The canonical-handle reservation remains visible until all paired
        // cleanup has completed, including after the pairing disappears.
        EditorV2Registry.releaseHandleDestroyReservation(forLegacyId: nativeViewId)
    }

    // Pair lookup follows reservation acquisition. Do not make routing or
    // contention decisions from a pairing that terminal teardown removes.
    let adapter = EditorV2Registry.adapter(forLegacyId: nativeViewId)
    let viewRegistry = NativeEditorViewRegistry.shared
    let reservation = viewRegistry.acquireDestroyReservation(editorId: nativeViewId)
    if reservation == .alreadyInProgress {
        return FfiUnitResult(value: nil, error: v2DestroyAlreadyInProgressError())
    }
    let reserved = reservation == .reserved
    if adapter != nil && !reserved {
        return FfiUnitResult(
            value: nil,
            error: FfiError(
                domain: "boundary",
                code: "FFI_RESULT_INVALID",
                message: "v2 destroy could not reserve its native view",
                requestId: nil,
                operationIndex: nil,
                limit: nil,
                actual: nil,
                detailsJson: nil
            )
        )
    }

    // Transport callbacks and retained sockets must lose ownership before
    // the Rust session can be destroyed or its numeric handle reused.
    let result: FfiUnitResult
    if let adapter {
        result = adapter.destroyForModuleTransaction {
            NativeCollaborationTransportRegistry.destroy(editorId: nativeViewId)
        }
    } else {
        NativeCollaborationTransportRegistry.destroy(editorId: nativeViewId)
        result = destroy(editorId)
    }
    invokeDestroyTestingHook(
        EditorV2Registry.onDestroyFfiResultReceivedForTesting,
        editorId: nativeViewId
    )
    switch (result.value, result.error) {
    case let (value?, nil) where value:
        break
    case let (nil, error?) where error.domain == "lifecycle"
        && (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING"):
        break
    case (nil, .some(_)):
        if reserved { viewRegistry.rollbackDestroy(editorId: nativeViewId) }
        return result
    default:
        if reserved { viewRegistry.rollbackDestroy(editorId: nativeViewId) }
        return FfiUnitResult(
            value: nil,
            error: FfiError(
                domain: "boundary",
                code: "FFI_RESULT_INVALID",
                message: "v2 destroy result violates the frozen unit-result shape",
                requestId: nil,
                operationIndex: nil,
                limit: nil,
                actual: nil,
                detailsJson: nil
            )
        )
    }

    // Terminal result only: the adapter clears its error owner before this
    // point, then its pairing is removed while the command/callback gate is
    // still held. The handle transaction remains held through finalization.
    if adapter != nil {
        _ = EditorV2Registry.removePairing(forLegacyId: nativeViewId)
        invokeDestroyTestingHook(
            EditorV2Registry.onPairRemovedBeforeDestroyFinalizationForTesting,
            editorId: nativeViewId
        )
    }
    if reserved { viewRegistry.finalizeDestroy(editorId: nativeViewId) }
    return result
}

/// Test-facing wrapper retained only to inject the raw FFI result for an
/// unpaired handle. Production routing uses `destroyEditorV2FromModule`.
func destroyUnpairedEditorV2FromModule(
    editorId: String,
    nativeViewId _: UInt64,
    destroy: (String) -> FfiUnitResult = { editorId in
        editorV2Destroy(editorId: editorId)
    }
) -> FfiUnitResult {
    destroyEditorV2FromModule(editorId: editorId, destroy: destroy)
}

/// Whether the JS-facing create config initializes a room-bound session
/// (the paired adapter mirrors the binding for its collaboration gating).
private func v2ConfigIndicatesRoomBinding(_ configJson: String) -> Bool {
    guard let data = configJson.data(using: .utf8),
          let config = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let initialization = config["initialization"] as? [String: Any]
    else {
        return false
    }
    return initialization["type"] as? String == "room"
}

final class NativeEditorModuleSessionOwner {
    private let lock = NSLock()
    private var editorIds: Set<String> = []

    var countForTesting: Int {
        lock.lock()
        defer { lock.unlock() }
        return editorIds.count
    }

    func insert(_ editorId: String) {
        guard let value = v2CanonicalUInt64String(editorId), value != "0" else { return }
        lock.lock()
        editorIds.insert(value)
        lock.unlock()
    }

    func remove(_ editorId: String) {
        lock.lock()
        editorIds.remove(editorId)
        lock.unlock()
    }

    func destroyAll(
        _ destroy: (String) -> FfiUnitResult = {
            destroyEditorV2FromModule(editorId: $0)
        }
    ) {
        lock.lock()
        let ownedEditorIds = editorIds.sorted()
        editorIds.removeAll()
        lock.unlock()
        for editorId in ownedEditorIds {
            _ = destroy(editorId)
        }
    }
}

private func isTerminalEditorV2DestroyResult(_ result: FfiUnitResult) -> Bool {
    if result.value == true, result.error == nil { return true }
    guard result.value == nil, let error = result.error else { return false }
    return error.domain == "lifecycle"
        && (error.code == "ENGINE_DESTROYED" || error.code == "ENGINE_DESTROYING")
}

// `Module` is `BaseModule & AnyModule`; the conformance is spelled out so
// `AnyModule` can carry `@preconcurrency`. That is what lets `definition()`
// below be `@MainActor` even though the protocol requirement is nonisolated.
//
// Why `definition()` has to be main-actor isolated: expo-modules-core declares
// `extension UIView: @MainActor AnyArgument`, so every `UIView` subclass gets a
// main-actor-isolated `AnyArgument` conformance. The `AsyncFunction` factories
// used for view functions require `A0: AnyArgument`, and an isolated conformance
// may only be used from a context isolated to the same actor — otherwise the
// compiler reports `#IsolatedConformances` (a warning in Swift 5 mode, an error
// in Swift 6).
//
// Caveat: `definition()` is actually invoked on the JS thread
// (`EXReactNativeFactory host:didInitializeRuntime:` → `registerNativeModules`),
// so the isolation claimed here is a compile-time device only. Under Swift 5
// mode the witness thunk is a plain passthrough — no executor check is emitted —
// but that also means nothing in this method body may touch main-actor state.
// Keep it pure definition building.
public class NativeEditorModule: BaseModule, @preconcurrency AnyModule {
    private var collaborationLifecycleObservers: [NSObjectProtocol] = []
    private let collaborationOwner = UUID()
    private let sessionOwner = NativeEditorModuleSessionOwner()

    @MainActor
    public func definition() -> ModuleDefinition {
        Name("NativeEditor")
        // Benchmark-only controls are deterministic and surface-independent;
        // they never participate in a viewer draw or measurement path.
        Function("preparedProseBenchmarkBegin") {
            PreparedProseInstrumentation.beginBenchmark()
        }
        Function("preparedProseBenchmarkBeginPhase") { (phase: String) in
            guard let traversal = PreparedProseInstrumentation.TraversalPhase(rawValue: phase) else { return }
            PreparedProseInstrumentation.beginPhase(traversal)
        }
        Function("preparedProseBenchmarkEndPhase") {
            PreparedProseInstrumentation.endPhase()
        }
        Function("preparedProseBenchmarkReset") {
            PreparedProseLayoutRegistry.shared.didReceiveMemoryWarning()
            PreparedProseInstrumentation.invalidated(.cacheReset)
        }
        Function("preparedProseBenchmarkExport") { () -> String in
            PreparedProseInstrumentation.exportJSON()
        }
        Events("onCollaborationTransportEvent")

        OnCreate {
            NativeCollaborationTransportRegistry.setEventEmitter(owner: self.collaborationOwner) { [weak self] payload in
                DispatchQueue.main.async {
                    self?.sendEvent("onCollaborationTransportEvent", payload)
                }
            }
            let center = NotificationCenter.default
            self.collaborationLifecycleObservers = [
                center.addObserver(
                    forName: UIApplication.didEnterBackgroundNotification,
                    object: nil,
                    queue: nil
                ) { _ in
                    NativeCollaborationTransportRegistry.enterBackground()
                },
                center.addObserver(
                    forName: UIApplication.willEnterForegroundNotification,
                    object: nil,
                    queue: nil
                ) { _ in
                    NativeCollaborationTransportRegistry.enterForeground()
                },
            ]
        }

        OnDestroy {
            let center = NotificationCenter.default
            self.collaborationLifecycleObservers.forEach(center.removeObserver)
            self.collaborationLifecycleObservers.removeAll()
            self.sessionOwner.destroyAll()
            NativeCollaborationTransportRegistry.destroyAll(owner: self.collaborationOwner)
        }

        // MARK: v2 UniFFI surface (production ABI)
        //
        // The JS document handle drives the engine directly through these
        // passthroughs; every call returns the frozen {value, error} result
        // record. Decimal-string handles/generations keep full u64 fidelity
        // across the JS boundary; binaries travel as Data.

        Function("editorV2Create") { (configJson: String, snapshotState: Data?) -> [String: Any] in
            let result = createEditorV2SessionFromModule(configJson: configJson, snapshotState: snapshotState)
            if let value = result["value"] as? String,
               let handle = createdV2SessionHandle(value)
            {
                self.sessionOwner.insert(handle.handle)
            }
            return result
        }
        Function("editorV2Destroy") { (editorId: String) -> [String: Any] in
            let result = destroyEditorV2FromModule(editorId: editorId)
            if isTerminalEditorV2DestroyResult(result) {
                self.sessionOwner.remove(editorId)
            }
            return v2UnitResultDictionary(result)
        }
        Function("editorV2GetState") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(performEditorV2JsonOperation(editorId: editorId) {
                editorV2GetState(editorId: editorId)
            })
        }
        Function("editorV2GetDocumentJson") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(performEditorV2JsonOperation(editorId: editorId) {
                editorV2GetDocumentJson(editorId: editorId)
            })
        }
        Function("editorV2GetDocumentHtml") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(performEditorV2JsonOperation(editorId: editorId) {
                editorV2GetDocumentHtml(editorId: editorId)
            })
        }
        Function("editorV2GetContentSnapshot") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(performEditorV2JsonOperation(editorId: editorId) {
                editorV2GetContentSnapshot(editorId: editorId)
            })
        }
        Function("editorV2ReplaceDocument") { (editorId: String, requestJson: String) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2ReplaceDocument(editorId: editorId, requestJson: requestJson)
                }
            )
        }
        Function("editorV2ApplyInput") { (editorId: String, requestJson: String) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2ApplyInput(editorId: editorId, requestJson: requestJson)
                }
            )
        }
        Function("editorV2ApplyCommand") { (editorId: String, requestJson: String) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2ApplyCommand(editorId: editorId, requestJson: requestJson)
                }
            )
        }
        Function("editorV2ApplyLocalApi") { (editorId: String, requestJson: String) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2ApplyLocalApi(editorId: editorId, requestJson: requestJson)
                }
            )
        }
        Function("editorV2SetSelection") { (editorId: String, requestJson: String) -> [String: Any] in
            v2JsonResultDictionary(performEditorV2JsonOperation(editorId: editorId) {
                editorV2SetSelection(editorId: editorId, requestJson: requestJson)
            })
        }
        Function("editorV2Undo") { (editorId: String, requestJson: String) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2Undo(editorId: editorId, requestJson: requestJson)
                }
            )
        }
        Function("editorV2Redo") { (editorId: String, requestJson: String) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2Redo(editorId: editorId, requestJson: requestJson)
                }
            )
        }
        Function("editorV2RenderUpdate") { (editorId: String, mirrorScalarAnchor: Double?, mirrorScalarHead: Double?) -> [String: Any] in
            // The render accessor for the interactive component: after a
            // JS-driven engine change the component fetches the current
            // render update here and pushes it to the bound view.
            let anchor = mirrorScalarAnchor.flatMap { v2ExactUInt32(NSNumber(value: $0)) }
            let head = mirrorScalarHead.flatMap { v2ExactUInt32(NSNumber(value: $0)) }
            if (mirrorScalarAnchor != nil && anchor == nil) || (mirrorScalarHead != nil && head == nil) {
                return v2InvalidResultDictionary("invalid render scalar position")
            }
            return v2JsonResultDictionary(
                performEditorV2JsonOperation(editorId: editorId) {
                    editorV2RenderUpdate(
                        editorId: editorId,
                        mirrorScalarAnchor: anchor,
                        mirrorScalarHead: head
                    )
                }
            )
        }
        Function("editorV2CollaborationConfigureTransport") {
            (editorId: String, configJsonOrNull: String?) -> [String: Any] in
            guard let nativeEditorId = v2UInt64Argument(editorId), nativeEditorId > 0 else {
                return v2InvalidResultDictionary("invalid editorId")
            }
            let error = NativeCollaborationTransportRegistry.configure(
                owner: self.collaborationOwner,
                editorId: nativeEditorId,
                configJSON: configJsonOrNull
            )
            return v2UnitResultDictionary(FfiUnitResult(
                value: error == nil ? true : nil,
                error: error
            ))
        }
        Function("editorV2CollaborationResolveProtocolAdapter") {
            (
                editorId: String,
                attemptId: String,
                eventId: String,
                responseJson: String
            ) -> [String: Any] in
            guard let nativeEditorId = v2UInt64Argument(editorId), nativeEditorId > 0 else {
                return v2InvalidResultDictionary("invalid editorId")
            }
            let error = NativeCollaborationTransportRegistry.resolveProtocolAdapter(
                editorId: nativeEditorId,
                attemptId: attemptId,
                eventId: eventId,
                responseJSON: responseJson
            )
            return v2UnitResultDictionary(FfiUnitResult(
                value: error == nil ? true : nil,
                error: error
            ))
        }
        Function("editorV2CollaborationSetAwareness") { (editorId: String, awarenessJson: String) -> [String: Any] in
            let result = performEditorV2UnitOperation(editorId: editorId) {
                editorV2CollaborationSetAwareness(
                    editorId: editorId,
                    awarenessJson: awarenessJson
                )
            }
            if result.value == true, result.error == nil,
               let nativeEditorId = v2UInt64Argument(editorId)
            {
                NativeCollaborationTransportRegistry.notifyOutboundAvailable(
                    editorId: nativeEditorId,
                    reason: .awareness
                )
            }
            return v2UnitResultDictionary(result)
        }
        Function("editorV2CollaborationPeers") { (editorId: String) -> [String: Any] in
            v2JsonResultDictionary(performEditorV2JsonOperation(editorId: editorId) {
                editorV2CollaborationPeers(editorId: editorId)
            })
        }
        Function("editorV2SnapshotExport") { (editorId: String) -> [String: Any] in
            v2SnapshotExportResultDictionary(performEditorV2SnapshotExportOperation(editorId: editorId) {
                editorV2SnapshotExport(editorId: editorId)
            })
        }
        Function("editorV2SnapshotRestore") { (editorId: String, metadataJson: String, encodedState: Data) -> [String: Any] in
            v2MutationResultDictionary(
                editorId: editorId,
                result: performEditorV2JsonOperation(editorId: editorId) {
                    editorV2SnapshotRestore(
                        editorId: editorId,
                        metadataJson: metadataJson,
                        encodedState: encodedState
                    )
                }
            )
        }

        View(NativeEditorExpoView.self) {
            Events(
                "onEditorUpdate",
                "onSelectionChange",
                "onFocusChange",
                "onContentHeightChange",
                "onToolbarAction",
                "onAddonEvent",
                "onEditorError",
                "onExternalTextCompositionEnd"
            )

            Prop("editorId") { (view: NativeEditorExpoView, id: String) in
                applyNativeEditorIdProp(id, to: view)
            }
            Prop("editable") { (view: NativeEditorExpoView, editable: Bool) in
                view.setEditable(editable)
            }
            Prop("accessibilityLabel") { (view: NativeEditorExpoView, label: String?) in
                view.setAccessibilityLabel(label)
            }
            Prop("accessibilityHint") { (view: NativeEditorExpoView, hint: String?) in
                view.setAccessibilityHint(hint)
            }
            Prop("placeholder") { (view: NativeEditorExpoView, placeholder: String) in
                view.richTextView.textView.placeholder = placeholder
            }
            Prop("autoFocus") { (view: NativeEditorExpoView, autoFocus: Bool) in
                view.setAutoFocus(autoFocus)
            }
            Prop("autoCapitalize") { (view: NativeEditorExpoView, autoCapitalize: String?) in
                view.setAutoCapitalize(autoCapitalize)
            }
            Prop("autoCorrect") { (view: NativeEditorExpoView, autoCorrect: Bool?) in
                view.setAutoCorrect(autoCorrect)
            }
            Prop("keyboardType") { (view: NativeEditorExpoView, keyboardType: String?) in
                view.setKeyboardType(keyboardType)
            }
            Prop("showToolbar") { (view: NativeEditorExpoView, showToolbar: Bool) in
                view.setShowToolbar(showToolbar)
            }
            Prop("toolbarPlacement") { (view: NativeEditorExpoView, toolbarPlacement: String?) in
                view.setToolbarPlacement(toolbarPlacement)
            }
            Prop("heightBehavior") { (view: NativeEditorExpoView, heightBehavior: String) in
                view.setHeightBehavior(heightBehavior)
            }
            Prop("allowImageResizing") { (view: NativeEditorExpoView, allowImageResizing: Bool) in
                view.setAllowImageResizing(allowImageResizing)
            }
            Prop("imageLoadingPolicyJson") { (view: NativeEditorExpoView, json: String?) in
                view.setImageLoadingPolicyJson(json)
            }
            Prop("themeJson") { (view: NativeEditorExpoView, themeJson: String?) in
                view.setThemeJson(themeJson)
            }
            Prop("addonsJson") { (view: NativeEditorExpoView, addonsJson: String?) in
                view.setAddonsJson(addonsJson)
            }
            Prop("remoteSelectionsJson") { (view: NativeEditorExpoView, remoteSelectionsJson: String?) in
                view.setRemoteSelectionsJson(remoteSelectionsJson)
            }
            Prop("toolbarItemsJson") { (view: NativeEditorExpoView, toolbarItemsJson: String?) in
                view.setToolbarButtonsJson(toolbarItemsJson)
            }
            Prop("toolbarFrameJson") { (view: NativeEditorExpoView, toolbarFrameJson: String?) in
                view.setToolbarFrameJson(toolbarFrameJson)
            }
            Prop("editorUpdateJson") { (view: NativeEditorExpoView, editorUpdateJson: String?) in
                view.setPendingEditorUpdateJson(editorUpdateJson)
            }
            Prop("editorUpdateEditorId") { (view: NativeEditorExpoView, editorUpdateEditorId: String?) in
                view.setPendingEditorUpdateEditorId(editorUpdateEditorId)
            }
            Prop("editorUpdateRevision") { (view: NativeEditorExpoView, editorUpdateRevision: Double) in
                guard let exactRevision = v2ExactUInt32(NSNumber(value: editorUpdateRevision)) else {
                    return
                }
                view.setPendingEditorUpdateRevision(Int(exactRevision))
            }
            OnViewDidUpdateProps { (view: NativeEditorExpoView) in
                view.applyPendingEditorUpdateIfNeeded()
            }

            AsyncFunction("applyEditorUpdate") { (view: NativeEditorExpoView, updateJson: String) -> Bool in
                view.applyEditorUpdate(updateJson)
            }
            AsyncFunction("beginExternalTextComposition") {
                (view: NativeEditorExpoView, sessionId: String) -> String in
                view.beginExternalTextComposition(sessionId: sessionId)
            }
            AsyncFunction("updateExternalTextComposition") {
                (view: NativeEditorExpoView, sessionId: String, text: String) -> String in
                view.updateExternalTextComposition(sessionId: sessionId, text: text)
            }
            AsyncFunction("commitExternalTextComposition") {
                (view: NativeEditorExpoView, sessionId: String, text: String) -> String in
                view.commitExternalTextComposition(sessionId: sessionId, finalText: text)
            }
            AsyncFunction("cancelExternalTextComposition") {
                (view: NativeEditorExpoView, sessionId: String, cause: String) -> String in
                view.cancelExternalTextComposition(sessionId: sessionId, cause: cause)
            }
            AsyncFunction("focus") { (view: NativeEditorExpoView) in
                view.focus()
            }
            AsyncFunction("blur") { (view: NativeEditorExpoView) in
                view.blur()
            }
            AsyncFunction("getCaretRect") { (view: NativeEditorExpoView) -> String? in
                view.getCaretRectJson()
            }
        }

    }
}
