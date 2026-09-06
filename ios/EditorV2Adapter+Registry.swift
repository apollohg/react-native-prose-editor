import Foundation

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

private func editorV2UnavailableOperationError() -> FfiError {
    FfiError(
        domain: "lifecycle",
        code: "ENGINE_DESTROYED",
        message: "editor session is not active",
        requestId: nil,
        operationIndex: nil,
        limit: nil,
        actual: nil,
        detailsJson: nil
    )
}

func performEditorV2JsonOperation(
    editorId: String,
    _ operation: () -> FfiJsonResult
) -> FfiJsonResult {
    guard let legacyId = UInt64(editorId),
          let adapter = EditorV2Registry.adapter(forLegacyId: legacyId)
    else {
        return operation()
    }
    return adapter.performRuntimeOperation(
        unavailable: FfiJsonResult(value: nil, error: editorV2UnavailableOperationError()),
        operation
    )
}

func performEditorV2UnitOperation(
    editorId: String,
    _ operation: () -> FfiUnitResult
) -> FfiUnitResult {
    guard let legacyId = UInt64(editorId),
          let adapter = EditorV2Registry.adapter(forLegacyId: legacyId)
    else {
        return operation()
    }
    return adapter.performRuntimeOperation(
        unavailable: FfiUnitResult(value: nil, error: editorV2UnavailableOperationError()),
        operation
    )
}

func performEditorV2LifecycleUnitOperation(
    editorId: String,
    _ operation: () -> FfiUnitResult
) -> FfiUnitResult {
    guard let legacyId = UInt64(editorId),
          let adapter = EditorV2Registry.adapter(forLegacyId: legacyId)
    else {
        return operation()
    }
    return adapter.performRuntimeLifecycleOperation(
        unavailable: FfiUnitResult(value: nil, error: editorV2UnavailableOperationError()),
        operation
    )
}

func performEditorV2SnapshotExportOperation(
    editorId: String,
    _ operation: () -> FfiSnapshotExportResult
) -> FfiSnapshotExportResult {
    guard let legacyId = UInt64(editorId),
          let adapter = EditorV2Registry.adapter(forLegacyId: legacyId)
    else {
        return operation()
    }
    return adapter.performRuntimeOperation(
        unavailable: FfiSnapshotExportResult(value: nil, error: editorV2UnavailableOperationError()),
        operation
    )
}

func performEditorV2OutboundLeaseOperation(
    editorId: String,
    _ operation: () -> FfiOutboundLeaseResult
) -> FfiOutboundLeaseResult {
    guard let legacyId = UInt64(editorId),
          let adapter = EditorV2Registry.adapter(forLegacyId: legacyId)
    else {
        return operation()
    }
    return adapter.performRuntimeOperation(
        unavailable: FfiOutboundLeaseResult(
            value: nil,
            empty: false,
            error: editorV2UnavailableOperationError()
        ),
        operation
    )
}

/// Maps the public (module-visible) editor id to the v2 adapter backing it.
/// The module's create path registers one pairing per editor; views bound
/// to a paired id route every interaction through the adapter.
enum EditorV2Registry {
    static let lock = NSLock()
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
