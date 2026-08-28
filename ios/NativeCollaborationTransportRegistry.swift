import Foundation

enum NativeCollaborationTransportRegistry {
    typealias EventEmitter = ([String: Any]) -> Void

    private static let queue = DispatchQueue(
        label: "com.apollohg.native-editor.collaboration-registry"
    )
    private static var transports: [UInt64: NativeCollaborationTransport] = [:]
    private static var transportOwners: [UInt64: UUID] = [:]
    private static var transportTokens: [UInt64: UUID] = [:]
    private static var eventSequences: [UInt64: UInt64] = [:]
    private static var eventProcessors: [UInt64: CollaborationEventProcessor] = [:]
    private static var remoteRebases: [UInt64: RemoteRebaseState] = [:]
    private static var eventEmitter: EventEmitter?
    private static var eventEmitterOwner: UUID?

    private struct RemoteRebaseState {
        let token: UUID
        var dirty: Bool
    }

    private final class CollaborationEventProcessor {
        let queue: DispatchQueue

        init(editorId: UInt64) {
            queue = DispatchQueue(
                label: "com.apollohg.native-editor.collaboration-events.\(editorId)"
            )
        }
    }

    static func setEventEmitter(owner: UUID, _ emitter: EventEmitter?) {
        queue.sync {
            if let emitter {
                eventEmitter = emitter
                eventEmitterOwner = owner
            } else if eventEmitterOwner == owner {
                eventEmitter = nil
                eventEmitterOwner = nil
            }
        }
    }

    static func configure(owner: UUID, editorId: UInt64, configJSON: String?) -> FfiError? {
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
                transportOwners.removeValue(forKey: editorId)
                transportTokens.removeValue(forKey: editorId)
                eventProcessors.removeValue(forKey: editorId)
                remoteRebases.removeValue(forKey: editorId)
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
                transportOwners[editorId] = owner
                transportTokens[editorId] = token
                eventProcessors[editorId] = CollaborationEventProcessor(editorId: editorId)
                created = true
            }
            transportOwners[editorId] = owner
            let error = transport.configure(parsed)
            if error != nil, created, transports[editorId] === transport {
                // A rejected Rust lifecycle transition must not leave a new
                // unusable owner registered. Existing owners remain intact.
                transports.removeValue(forKey: editorId)
                transportOwners.removeValue(forKey: editorId)
                transportTokens.removeValue(forKey: editorId)
                eventProcessors.removeValue(forKey: editorId)
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

    static func resolveProtocolAdapter(
        editorId: UInt64,
        attemptId: String,
        eventId: String,
        responseJSON: String
    ) -> FfiError? {
        guard editorId > 0,
              !attemptId.isEmpty,
              canonicalProtocolEventId(eventId) != nil,
              let response = parseProtocolAdapterResponse(responseJSON)
        else {
            return contractError("invalid collaboration protocol adapter response")
        }
        return queue.sync {
            guard let transport = transports[editorId] else {
                // A response racing teardown belongs to a retired attempt.
                return nil
            }
            return transport.resolveProtocolAdapter(
                attemptId: attemptId,
                eventId: eventId,
                response: response
            )
        }
    }

    static func destroy(editorId: UInt64) {
        queue.sync {
            transports.removeValue(forKey: editorId)?.destroy()
            transportOwners.removeValue(forKey: editorId)
            transportTokens.removeValue(forKey: editorId)
            eventSequences.removeValue(forKey: editorId)
            eventProcessors.removeValue(forKey: editorId)
            remoteRebases.removeValue(forKey: editorId)
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

    static func destroyAll(owner: UUID) {
        queue.sync {
            let ownedEditorIds = transportOwners.compactMap { editorId, registeredOwner in
                registeredOwner == owner ? editorId : nil
            }
            let owned = ownedEditorIds.compactMap { transports.removeValue(forKey: $0) }
            for editorId in ownedEditorIds {
                transportOwners.removeValue(forKey: editorId)
                transportTokens.removeValue(forKey: editorId)
                eventSequences.removeValue(forKey: editorId)
                eventProcessors.removeValue(forKey: editorId)
                remoteRebases.removeValue(forKey: editorId)
            }
            owned.forEach { $0.destroy() }
            if eventEmitterOwner == owner {
                eventEmitter = nil
                eventEmitterOwner = nil
            }
        }
    }

#if DEBUG
    static var eventEmitterOwnerForTesting: UUID? {
        queue.sync { eventEmitterOwner }
    }

    static func emitForTesting(_ payload: [String: Any]) {
        queue.sync { eventEmitter?(payload) }
    }
#endif

    private static func enqueue(
        event: NativeCollaborationTransportEvent,
        editorId: UInt64,
        token: UUID
    ) {
        queue.async {
            guard transports[editorId] != nil,
                  transportTokens[editorId] == token,
                  let processor = eventProcessors[editorId]
            else { return }
            let next = (eventSequences[editorId] ?? 0).addingReportingOverflow(1)
            guard !next.overflow, next.partialValue > 0 else {
                return
            }
            eventSequences[editorId] = next.partialValue
            let sequence = next.partialValue
            processor.queue.async {
                var payload: [String: Any] = [
                    "editorId": String(editorId),
                    "eventSequence": String(sequence),
                ]
                switch event {
                case let .directive(directive, generation, wakeReason):
                    guard let state = state(editorId: editorId) else { return }
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
                case let .protocolAdapter(adapterEvent):
                    payload["kind"] = "protocolAdapter"
                    payload["generation"] = adapterEvent.generation
                    payload["attemptId"] = adapterEvent.attemptId
                    payload["eventId"] = adapterEvent.eventId
                    payload["negotiatedProtocol"] =
                        adapterEvent.negotiatedProtocol ?? NSNull()
                    switch adapterEvent.phase {
                    case .open:
                        payload["phase"] = "open"
                    case .message(.text(let text)):
                        payload["phase"] = "message"
                        payload["frame"] = ["type": "text", "data": text]
                    case .message(.binary(let data)):
                        payload["phase"] = "message"
                        payload["frame"] = [
                            "type": "binary",
                            "data": data.base64EncodedString(),
                        ]
                    }
                }
                queue.async {
                    guard transports[editorId] != nil,
                          transportTokens[editorId] == token,
                          eventProcessors[editorId] === processor
                    else { return }
                    if case let .directive(directive, _, _) = event,
                       directive.remoteCommitApplied {
                        scheduleRemoteRebase(editorId: editorId, token: token)
                    }
                    eventEmitter?(payload)
                }
            }
        }
    }

    private static func scheduleRemoteRebase(editorId: UInt64, token: UUID) {
        if var pending = remoteRebases[editorId], pending.token == token {
            pending.dirty = true
            remoteRebases[editorId] = pending
            return
        }
        remoteRebases[editorId] = RemoteRebaseState(token: token, dirty: false)
        dispatchRemoteRebase(editorId: editorId, token: token)
    }

    private static func dispatchRemoteRebase(editorId: UInt64, token: UUID) {
        DispatchQueue.main.async {
            NativeEditorViewRegistry.shared.applyRemoteCommitRefresh(editorId: editorId)
            queue.async {
                guard var pending = remoteRebases[editorId], pending.token == token else { return }
                if pending.dirty {
                    pending.dirty = false
                    remoteRebases[editorId] = pending
                    dispatchRemoteRebase(editorId: editorId, token: token)
                } else {
                    remoteRebases.removeValue(forKey: editorId)
                }
            }
        }
    }

    private static func state(editorId: UInt64) -> [String: Any]? {
        let handle = String(editorId)
        let result = performEditorV2JsonOperation(editorId: handle) {
            editorV2GetState(editorId: handle)
        }
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
        let handle = String(editorId)
        let result = performEditorV2JsonOperation(editorId: handle) {
            editorV2CollaborationPeers(editorId: handle)
        }
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
              Set(object.keys).isSubset(of: Set(["url", "connect", "protocolAdapter"])),
              object.keys.contains("url"),
              object.keys.contains("connect"),
              let rawURL = object["url"] as? String,
              let connectNumber = object["connect"] as? NSNumber,
              CFGetTypeID(connectNumber) == CFBooleanGetTypeID(),
              let url = URL(string: rawURL)
        else {
            return .failure(contractError("invalid collaboration transport configuration"))
        }
        let protocolAdapter: NativeCollaborationProtocolAdapterConfig?
        if let rawAdapter = object["protocolAdapter"] {
            guard let adapter = parseProtocolAdapterConfig(rawAdapter) else {
                return .failure(contractError("invalid collaboration protocol adapter"))
            }
            protocolAdapter = adapter
        } else {
            protocolAdapter = nil
        }
        do {
            return .success(try NativeCollaborationTransportConfig(
                url: url,
                connect: connectNumber.boolValue,
                protocolAdapter: protocolAdapter
            ))
        } catch {
            return .failure(contractError("invalid collaboration WebSocket URL"))
        }
    }

    private static func parseProtocolAdapterConfig(
        _ raw: Any
    ) -> NativeCollaborationProtocolAdapterConfig? {
        guard let object = raw as? [String: Any],
              Set(object.keys).isSubset(of: Set([
                "protocols",
                "timeoutMillis",
                "terminalCloseCodes",
              ])),
              let protocols = object["protocols"] as? [String],
              !protocols.isEmpty,
              protocols.count <= 16,
              Set(protocols).count == protocols.count,
              protocols.allSatisfy(validWebSocketProtocol)
        else {
            return nil
        }
        let timeoutMillis: UInt64
        if let rawTimeout = object["timeoutMillis"] {
            guard let number = rawTimeout as? NSNumber,
                  CFGetTypeID(number) != CFBooleanGetTypeID(),
                  number.doubleValue.rounded(.towardZero) == number.doubleValue,
                  number.doubleValue >= 1,
                  number.doubleValue <= 60_000
            else {
                return nil
            }
            timeoutMillis = number.uint64Value
        } else {
            timeoutMillis = NativeCollaborationProtocolAdapterConfig.defaultTimeoutMillis
        }
        let terminalCloseCodes: Set<UInt32>
        if let rawCodes = object["terminalCloseCodes"] {
            guard let values = rawCodes as? [Any] else { return nil }
            var codes = Set<UInt32>()
            for rawCode in values {
                guard let number = rawCode as? NSNumber,
                      CFGetTypeID(number) != CFBooleanGetTypeID(),
                      number.doubleValue.rounded(.towardZero) == number.doubleValue,
                      number.doubleValue >= 1_000,
                      number.doubleValue <= 4_999
                else {
                    return nil
                }
                guard codes.insert(number.uint32Value).inserted else { return nil }
            }
            terminalCloseCodes = codes
        } else {
            terminalCloseCodes = []
        }
        return NativeCollaborationProtocolAdapterConfig(
            protocols: protocols,
            timeoutMillis: timeoutMillis,
            terminalCloseCodes: terminalCloseCodes
        )
    }

    private static func validWebSocketProtocol(_ value: String) -> Bool {
        guard let bytes = value.data(using: .utf8),
              !bytes.isEmpty,
              bytes.count <= 128
        else {
            return false
        }
        let allowed = CharacterSet(
            charactersIn: "!#$%&'*+-.^_`|~0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
        )
        return value.unicodeScalars.allSatisfy(allowed.contains)
    }

    private static func parseProtocolAdapterResponse(
        _ json: String
    ) -> NativeCollaborationProtocolAdapterResponse? {
        guard let data = json.data(using: .utf8),
              data.count <= 1_500_000,
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              Set(object.keys).isSubset(of: Set(["action", "frames"])),
              let rawAction = object["action"] as? String
        else {
            return nil
        }
        let action: NativeCollaborationProtocolAdapterAction
        switch rawAction {
        case "continue": action = .continue
        case "ready": action = .ready
        case "reject": action = .reject
        default: return nil
        }
        let rawFrames = object["frames"] as? [Any] ?? []
        guard rawFrames.count <= 16 else { return nil }
        var frames: [NativeCollaborationProtocolFrame] = []
        frames.reserveCapacity(rawFrames.count)
        for rawFrame in rawFrames {
            guard let frame = rawFrame as? [String: Any],
                  Set(frame.keys) == Set(["type", "data"]),
                  let type = frame["type"] as? String,
                  let value = frame["data"] as? String
            else {
                return nil
            }
            switch type {
            case "text":
                guard (value.data(using: .utf8)?.count ?? Int.max) <= 64 * 1_024 else {
                    return nil
                }
                frames.append(.text(value))
            case "binary":
                guard let decoded = Data(base64Encoded: value),
                      decoded.count <= 64 * 1_024
                else {
                    return nil
                }
                frames.append(.binary(decoded))
            default:
                return nil
            }
        }
        return NativeCollaborationProtocolAdapterResponse(action: action, frames: frames)
    }

    private static func canonicalProtocolEventId(_ raw: String) -> UInt64? {
        guard !raw.isEmpty,
              raw.allSatisfy({ $0 >= "0" && $0 <= "9" }),
              raw == "0" || raw.first != "0"
        else {
            return nil
        }
        return UInt64(raw)
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
