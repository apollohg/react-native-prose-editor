import Foundation

extension NativeCollaborationTransport {
    func normalizeDirective(
        _ result: FfiJsonResult
    ) -> Result<NativeCollaborationTransportDirective, FfiError> {
        switch (result.value, result.error) {
        case let (nil, error?):
            return .failure(error)
        case let (value?, nil):
            guard let data = value.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let transportState = object["transportState"] as? String,
                  let remoteCommitApplied = object["remoteCommitApplied"] as? Bool,
                  let peersChanged = object["peersChanged"] as? Bool,
                  let renewedLocal = object["renewedLocal"] as? Bool,
                  let expiredPeers = object["expiredPeers"] as? [String],
                  expiredPeers.allSatisfy({ canonicalUInt64($0) != nil })
            else {
                return .failure(contractError("collaboration directive violates the frozen shape"))
            }
            let generationToOpen = object["generationToOpen"] as? String
            let nextDeadlineMillis = object["nextDeadlineMillis"] as? String
            guard (object["generationToOpen"] is NSNull || generationToOpen != nil),
                  (object["nextDeadlineMillis"] is NSNull || nextDeadlineMillis != nil),
                  generationToOpen.map({ canonicalUInt64($0) != nil }) ?? true,
                  nextDeadlineMillis.map({ canonicalUInt64($0) != nil }) ?? true
            else {
                return .failure(contractError("collaboration directive contains a non-canonical u64"))
            }
            return .success(NativeCollaborationTransportDirective(
                transportState: transportState,
                generationToOpen: generationToOpen,
                nextDeadlineMillis: nextDeadlineMillis,
                remoteCommitApplied: remoteCommitApplied,
                peersChanged: peersChanged,
                renewedLocal: renewedLocal,
                expiredPeers: expiredPeers
            ))
        default:
            return .failure(contractError("v2 result must carry exactly one of value/error"))
        }
    }

    func normalizeUnit(_ result: FfiUnitResult) -> FfiError? {
        switch (result.value, result.error) {
        case (true?, nil):
            return nil
        case let (nil, error?):
            return error
        default:
            return contractError("v2 unit result violates the frozen shape")
        }
    }

    func normalizeJsonUnit(_ result: FfiJsonResult) -> FfiError? {
        switch (result.value, result.error) {
        case (_?, nil):
            return nil
        case let (nil, error?):
            return error
        default:
            return contractError("v2 JSON result violates the frozen shape")
        }
    }

    func canonicalUInt64(_ raw: String) -> UInt64? {
        guard !raw.isEmpty,
              raw.allSatisfy({ $0 >= "0" && $0 <= "9" }),
              raw == "0" || raw.first != "0"
        else {
            return nil
        }
        return UInt64(raw)
    }

    func contractError(_ message: String) -> FfiError {
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

    func lifecycleError(_ code: String, _ message: String) -> FfiError {
        FfiError(
            domain: "lifecycle",
            code: code,
            message: message,
            requestId: nil,
            operationIndex: nil,
            limit: nil,
            actual: nil,
            detailsJson: nil
        )
    }

}
