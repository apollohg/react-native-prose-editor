import Foundation

extension EditorV2Adapter {
    func contractError(_ message: String) -> FfiError {
        Self.contractError(message)
    }

    static func contractError(_ message: String) -> FfiError {
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

    static func normalizeJsonResult(_ result: FfiJsonResult) -> EditorV2ValueResult {
        switch (result.value, result.error) {
        case let (value?, nil):
            return .success(value)
        case let (nil, error?):
            return .failure(error)
        default:
            return .failure(contractError("v2 result must carry exactly one of value/error"))
        }
    }

    func emit(_ error: FfiError) {
        debugNotes.append("emit \(error.domain)/\(error.code): \(error.message)")
        dispatchAutonomousError(error)
    }

    /// A malformed externally supplied render is a permanent boundary
    /// rejection. Surface it once through the adapter's autonomous-error
    /// channel so the paired view can clear/report it, but do not also add a
    /// diagnostic note: that would expose a second observable emission for
    /// an otherwise atomic no-op.
    func rejectAtomicRenderSnapshot() {
        dispatchAutonomousError(Self.contractError("v2 atomic render snapshot violates the frozen shape"))
    }

    /// View-side envelope failures (missing/mismatched source ids) have no
    /// engine call to classify. Route them through the same boundary error
    /// channel as a rejected external snapshot, once per discarded envelope.
    func rejectExternalRenderEnvelope(_ message: String) {
        guard beginRuntimeOperation() else { return }
        defer { endRuntimeOperation() }
        dispatchAutonomousError(Self.contractError(message))
    }

    /// A later claim replaces the earlier owner. Clearing is conditional on
    /// the same token, so stale views cannot erase a newer binding.
    func bindAutonomousErrorOwner(token: UUID, _ callback: @escaping (FfiError) -> Void) {
        guard beginRuntimeOperation() else { return }
        defer { endRuntimeOperation() }
        claimNativeBinding(token: token, replaceExisting: true)
        autonomousErrorLock.lock()
        autonomousErrorOwnerToken = token
        autonomousErrorCallback = callback
        autonomousErrorLock.unlock()
    }

    func clearAutonomousErrorOwner(token: UUID) {
        guard beginRuntimeOperation() else { return }
        defer { endRuntimeOperation() }
        autonomousErrorLock.lock()
        guard autonomousErrorOwnerToken == token else {
            autonomousErrorLock.unlock()
            return
        }
        autonomousErrorOwnerToken = nil
        autonomousErrorCallback = nil
        autonomousErrorLock.unlock()
        guard nativeOwnerToken == token else { return }
        releaseNativeOwner()
    }

    func clearAutonomousErrorOwner() {
        runtimeLock.lock()
        defer { runtimeLock.unlock() }
        autonomousErrorLock.lock()
        if autonomousErrorOwnerToken != nil {
            autonomousErrorOwnerToken = nil
            autonomousErrorCallback = nil
        }
        autonomousErrorLock.unlock()
        releaseNativeOwner()
    }

    func claimNativeBindingIfUnowned(token: UUID) {
        guard beginRuntimeOperation() else { return }
        defer { endRuntimeOperation() }
        claimNativeBinding(token: token, replaceExisting: false)
    }

    func isNativeBindingOwner(token: UUID) -> Bool {
        guard beginRuntimeOperation() else { return false }
        defer { endRuntimeOperation() }
        return nativeOwnerToken == token
    }

    private func claimNativeBinding(token: UUID, replaceExisting: Bool) {
        if !replaceExisting, nativeOwnerToken != nil { return }
        if nativeOwnerToken == token { return }
        releaseNativeOwner()
        Self.nativeOwnerLock.lock()
        guard Self.nextNativeOwnerId < UInt64.max else {
            Self.nativeOwnerLock.unlock()
            emit(Self.contractError("native owner id counter exhausted"))
            return
        }
        Self.nextNativeOwnerId += 1
        nativeOwnerId = Self.nextNativeOwnerId
        nativeOwnerToken = token
        Self.nativeOwnerLock.unlock()
    }

    func releaseNativeBindingOwner(token: UUID) {
        guard beginRuntimeOperation() else { return }
        defer { endRuntimeOperation() }
        guard nativeOwnerToken == token else { return }
        releaseNativeOwner()
    }

    func releaseCurrentNativeOwnerInRustForTesting() -> Bool {
        guard beginRuntimeOperation() else { return false }
        defer { endRuntimeOperation() }
        guard let ownerId = nativeOwnerId, positionEpoch != nil else { return false }
        let result = editorV2ReleaseNativeBinding(
            editorId: editorId,
            ownerId: String(ownerId)
        )
        return result.error == nil && result.value == true
    }

    func releaseNativeOwner() {
        guard let ownerId = nativeOwnerId else { return }
        nativeOwnerId = nil
        nativeOwnerToken = nil
        positionEpoch = nil
        _ = editorV2ReleaseNativeBinding(editorId: editorId, ownerId: String(ownerId))
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

    func callWithEnvelope(
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
        guard beginRuntimeOperation() else { return }
        defer { endRuntimeOperation() }
        nextRequestId = requestId
    }

    func selectionEnvelope(anchor: UInt32, head: UInt32, affinity: String) -> [String: Any] {
        ["selection": EditorV2PositionBridge.textSelectionEnvelope(anchor: anchor, head: head, affinity: affinity)]
    }

    static func uint64Field(_ object: [String: Any], _ key: String) -> UInt64? {
        guard let string = object[key] as? String,
              isCanonicalDecimalEditorId(string)
        else { return nil }
        return UInt64(string)
    }

    func fetchDocumentJson() -> String? {
        switch Self.normalizeJsonResult(editorV2GetDocumentJson(editorId: editorId)) {
        case .failure(let error):
            emit(error)
            return nil
        case .success(let json):
            return json
        }
    }

}
