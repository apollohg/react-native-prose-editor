import Foundation

enum NativeCollaborationTransportRegistry {
    typealias EventEmitter = ([String: Any]) -> Void

    private static let queue = DispatchQueue(
        label: "com.apollohg.native-editor.collaboration-registry"
    )
    private static var transports: [UInt64: NativeCollaborationTransport] = [:]
    private static var transportTokens: [UInt64: UUID] = [:]
    private static var eventSequences: [UInt64: UInt64] = [:]
    private static var eventEmitter: EventEmitter?

    static func setEventEmitter(_ emitter: EventEmitter?) {
        queue.sync {
            eventEmitter = emitter
        }
    }

    static func configure(editorId: UInt64, configJSON: String?) -> FfiError? {
        guard editorId > 0 else {
            return contractError("invalid editorId")
        }
        let parsed: NativeCollaborationTransportConfig?
        if let configJSON {
            switch parseConfig(configJSON) {
            case .success(let config):
                parsed = config
            case .failure(let error):
                return error
            }
        } else {
            parsed = nil
        }

        return queue.sync {
            if parsed == nil {
                let existing = transports.removeValue(forKey: editorId)
                transportTokens.removeValue(forKey: editorId)
                existing?.destroy()
                return nil
            }

            let transport: NativeCollaborationTransport
            let created: Bool
            if let existing = transports[editorId] {
                transport = existing
                created = false
            } else {
                let token = UUID()
                transport = NativeCollaborationTransport(
                    editorId: String(editorId),
                    eventSink: { event in
                        enqueue(event: event, editorId: editorId, token: token)
                    }
                )
                transports[editorId] = transport
                transportTokens[editorId] = token
                created = true
            }
            let error = transport.configure(parsed)
            if error != nil, created, transports[editorId] === transport {
                // A rejected Rust lifecycle transition must not leave a new
                // unusable owner registered. Existing owners remain intact.
                transports.removeValue(forKey: editorId)
                transportTokens.removeValue(forKey: editorId)
                transport.destroy()
            }
            return error
        }
    }

    static func notifyOutboundAvailable(editorId: UInt64, reason: CollaborationWakeReason) {
        queue.async {
            transports[editorId]?.notifyOutboundAvailable(reason: reason)
        }
    }

    static func destroy(editorId: UInt64) {
        queue.sync {
            transports.removeValue(forKey: editorId)?.destroy()
            transportTokens.removeValue(forKey: editorId)
            eventSequences.removeValue(forKey: editorId)
        }
    }

    static func enterBackground() {
        queue.sync {
            transports.values.forEach { $0.enterBackground() }
        }
    }

    static func enterForeground() {
        queue.sync {
            transports.values.forEach { $0.enterForeground() }
        }
    }

    static func destroyAll() {
        queue.sync {
            let owned = transports.values
            transports.removeAll()
            transportTokens.removeAll()
            eventSequences.removeAll()
            owned.forEach { $0.destroy() }
            eventEmitter = nil
        }
    }

    private static func enqueue(
        event: NativeCollaborationTransportEvent,
        editorId: UInt64,
        token: UUID
    ) {
        queue.async {
            guard transports[editorId] != nil, transportTokens[editorId] == token else { return }
            let next = (eventSequences[editorId] ?? 0).addingReportingOverflow(1)
            guard !next.overflow, next.partialValue > 0 else {
                return
            }
            eventSequences[editorId] = next.partialValue

            var payload: [String: Any] = [
                "editorId": String(editorId),
                "eventSequence": String(next.partialValue),
            ]
            switch event {
            case let .directive(directive, generation, wakeReason):
                guard let state = state(editorId: editorId) else {
                    return
                }
                payload["kind"] = "state"
                payload["generation"] = generation ?? NSNull()
                payload["state"] = state
                payload["peers"] = peers(editorId: editorId)
                payload["diagnostics"] = [
                    "wakeReason": wakeReason.rawValue,
                    "transportState": directive.transportState,
                    "nextDeadlineMillis": directive.nextDeadlineMillis ?? NSNull(),
                    "remoteCommitApplied": directive.remoteCommitApplied,
                    "peersChanged": directive.peersChanged,
                    "renewedLocal": directive.renewedLocal,
                    "expiredPeerCount": directive.expiredPeers.count,
                ]
            case let .error(error, generation):
                payload["kind"] = "error"
                payload["generation"] = generation ?? NSNull()
                payload["error"] = errorDictionary(error)
            }
            eventEmitter?(payload)
        }
    }

    private static func state(editorId: UInt64) -> [String: Any]? {
        let result = editorV2GetState(editorId: String(editorId))
        guard result.error == nil,
              let value = result.value,
              let data = value.data(using: .utf8),
              let decoded = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return nil
        }
        return decoded
    }

    private static func peers(editorId: UInt64) -> Any {
        let result = editorV2CollaborationPeers(editorId: String(editorId))
        guard result.error == nil,
              let value = result.value,
              let data = value.data(using: .utf8),
              let decoded = try? JSONSerialization.jsonObject(with: data)
        else {
            return []
        }
        if let object = decoded as? [String: Any],
           let peers = object["peers"] {
            return peers
        }
        return []
    }

    private static func parseConfig(
        _ json: String
    ) -> Result<NativeCollaborationTransportConfig, FfiError> {
        guard let data = json.data(using: .utf8),
              data.count <= 32_768,
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys).isSubset(of: Set(["url", "connect", "connectionInit"])),
              object.keys.contains("url"),
              object.keys.contains("connect"),
              let rawURL = object["url"] as? String,
              let connectNumber = object["connect"] as? NSNumber,
              CFGetTypeID(connectNumber) == CFBooleanGetTypeID(),
              let url = URL(string: rawURL)
        else {
            return .failure(contractError("invalid collaboration transport configuration"))
        }
        let connectionInitJWT: String?
        if let rawConnectionInit = object["connectionInit"] {
            guard let connectionInit = rawConnectionInit as? [String: Any],
                  Set(connectionInit.keys) == Set(["jwt"]),
                  let jwt = connectionInit["jwt"] as? String
            else {
                return .failure(contractError("invalid collaboration connection_init configuration"))
            }
            connectionInitJWT = jwt
        } else {
            connectionInitJWT = nil
        }
        do {
            return .success(try NativeCollaborationTransportConfig(
                url: url,
                connect: connectNumber.boolValue,
                connectionInitJWT: connectionInitJWT
            ))
        } catch {
            return .failure(contractError("invalid collaboration WebSocket URL"))
        }
    }

    private static func errorDictionary(_ error: FfiError) -> [String: Any] {
        var value: [String: Any] = [
            "domain": error.domain,
            "code": error.code,
            "message": error.message,
        ]
        if let requestId = error.requestId { value["requestId"] = requestId }
        if let operationIndex = error.operationIndex { value["operationIndex"] = operationIndex }
        if let limit = error.limit { value["limit"] = limit }
        if let actual = error.actual { value["actual"] = actual }
        if let detailsJson = error.detailsJson { value["detailsJson"] = detailsJson }
        return value
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
}
