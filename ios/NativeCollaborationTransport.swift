import Foundation

extension FfiError: Error {}

struct NativeCollaborationTransportConfig: Equatable {
    static let maximumURLBytes = 4_096

    let url: URL
    let connect: Bool
    let protocolAdapter: NativeCollaborationProtocolAdapterConfig?
    let diagnosticEndpoint: String

    init(
        url: URL,
        connect: Bool,
        protocolAdapter: NativeCollaborationProtocolAdapterConfig?
    ) throws {
        guard let absolute = url.absoluteString.data(using: .utf8),
              !absolute.isEmpty,
              absolute.count <= Self.maximumURLBytes,
              let scheme = url.scheme?.lowercased(),
              scheme == "ws" || scheme == "wss",
              url.host?.isEmpty == false,
              url.user == nil,
              url.password == nil,
              url.fragment == nil
        else {
            throw NativeCollaborationTransportConfigurationError.invalidURL
        }

        guard let liveComponents = URLComponents(
            url: url,
            resolvingAgainstBaseURL: false
        ) else {
            throw NativeCollaborationTransportConfigurationError.invalidURL
        }
        var diagnostic = URLComponents()
        diagnostic.scheme = scheme
        diagnostic.host = url.host
        diagnostic.port = url.port
        diagnostic.percentEncodedPath = liveComponents.percentEncodedPath
        guard let endpoint = diagnostic.string else {
            throw NativeCollaborationTransportConfigurationError.invalidURL
        }
        self.url = url
        self.connect = connect
        self.protocolAdapter = protocolAdapter
        diagnosticEndpoint = endpoint
    }

    static func == (
        lhs: NativeCollaborationTransportConfig,
        rhs: NativeCollaborationTransportConfig
    ) -> Bool {
        lhs.url.absoluteString == rhs.url.absoluteString
            && lhs.connect == rhs.connect
            && lhs.protocolAdapter == rhs.protocolAdapter
    }
}

enum NativeCollaborationTransportConfigurationError: Error {
    case invalidURL
}

struct NativeCollaborationProtocolAdapterConfig: Equatable {
    static let defaultTimeoutMillis: UInt64 = 10_000
    static let maximumFrameBytes = 64 * 1_024

    let protocols: [String]
    let timeoutMillis: UInt64
    let terminalCloseCodes: Set<UInt32>
}

enum NativeCollaborationProtocolFrame {
    case text(String)
    case binary(Data)
}

enum NativeCollaborationProtocolAdapterAction {
    case `continue`
    case ready
    case reject
}

struct NativeCollaborationProtocolAdapterResponse {
    let action: NativeCollaborationProtocolAdapterAction
    let frames: [NativeCollaborationProtocolFrame]
}

enum NativeCollaborationProtocolAdapterPhase {
    case open
    case message(NativeCollaborationProtocolFrame)
}

struct NativeCollaborationProtocolAdapterEvent {
    let attemptId: String
    let eventId: String
    let generation: String
    let negotiatedProtocol: String?
    let phase: NativeCollaborationProtocolAdapterPhase
}

enum CollaborationWakeReason: String {
    case localMutation
    case moduleMutation
    case receive
    case timer
    case open
    case reattach
    case awareness
}

struct NativeCollaborationTransportDirective: Equatable {
    let transportState: String
    let generationToOpen: String?
    let nextDeadlineMillis: String?
    let remoteCommitApplied: Bool
    let peersChanged: Bool
    let renewedLocal: Bool
    let expiredPeers: [String]
}

enum NativeCollaborationTransportEvent {
    case directive(
        directive: NativeCollaborationTransportDirective,
        generation: String?,
        wakeReason: CollaborationWakeReason
    )
    case error(FfiError, generation: String?)
    case protocolAdapter(NativeCollaborationProtocolAdapterEvent)
}

protocol NativeCollaborationTransportBackend {
    func drive(editorId: String, nowMillis: String) -> FfiJsonResult
    func socketOpen(editorId: String, generation: String, nowMillis: String) -> FfiJsonResult
    func receive(
        editorId: String,
        generation: String,
        message: Data,
        nowMillis: String
    ) -> FfiJsonResult
    func socketClose(
        editorId: String,
        generation: String,
        code: UInt32?,
        reason: String?,
        nowMillis: String
    ) -> FfiJsonResult
    func leaseOutbound(editorId: String, generation: String) -> FfiOutboundLeaseResult
    func ackOutbound(editorId: String, generation: String, leaseId: String) -> FfiJsonResult
    func nackOutbound(editorId: String, generation: String, leaseId: String) -> FfiJsonResult
    func detach(editorId: String) -> FfiUnitResult
    func reattach(editorId: String) -> FfiUnitResult
}

struct RustNativeCollaborationTransportBackend: NativeCollaborationTransportBackend {
    func drive(editorId: String, nowMillis: String) -> FfiJsonResult {
        editorV2CollaborationDrive(editorId: editorId, nowMillis: nowMillis)
    }

    func socketOpen(editorId: String, generation: String, nowMillis: String) -> FfiJsonResult {
        editorV2CollaborationSocketOpen(
            editorId: editorId,
            generation: generation,
            nowMillis: nowMillis
        )
    }

    func receive(
        editorId: String,
        generation: String,
        message: Data,
        nowMillis: String
    ) -> FfiJsonResult {
        editorV2CollaborationReceive(
            editorId: editorId,
            generation: generation,
            message: message,
            nowMillis: nowMillis
        )
    }

    func socketClose(
        editorId: String,
        generation: String,
        code: UInt32?,
        reason: String?,
        nowMillis: String
    ) -> FfiJsonResult {
        editorV2CollaborationSocketClose(
            editorId: editorId,
            generation: generation,
            code: code,
            reason: reason,
            nowMillis: nowMillis
        )
    }

    func leaseOutbound(editorId: String, generation: String) -> FfiOutboundLeaseResult {
        editorV2CollaborationLeaseOutbound(editorId: editorId, generation: generation)
    }

    func ackOutbound(editorId: String, generation: String, leaseId: String) -> FfiJsonResult {
        editorV2CollaborationAckOutbound(
            editorId: editorId,
            generation: generation,
            leaseId: leaseId
        )
    }

    func nackOutbound(editorId: String, generation: String, leaseId: String) -> FfiJsonResult {
        editorV2CollaborationNackOutbound(
            editorId: editorId,
            generation: generation,
            leaseId: leaseId
        )
    }

    func detach(editorId: String) -> FfiUnitResult {
        editorV2CollaborationDetach(editorId: editorId)
    }

    func reattach(editorId: String) -> FfiUnitResult {
        editorV2CollaborationReattach(editorId: editorId)
    }
}

protocol CollaborationMonotonicClock {
    func nowMillis() -> UInt64
}

struct SystemCollaborationMonotonicClock: CollaborationMonotonicClock {
    func nowMillis() -> UInt64 {
        let milliseconds = ProcessInfo.processInfo.systemUptime * 1_000
        if milliseconds >= Double(UInt64.max) {
            return UInt64.max
        }
        return UInt64(milliseconds.rounded(.down))
    }
}

/// One serialized native owner for one authentic Rust editor handle.
final class NativeCollaborationTransport {
    typealias EventSink = (NativeCollaborationTransportEvent) -> Void

    private let editorId: String
    private let backend: NativeCollaborationTransportBackend
    private let socketFactory: CollaborationSocketFactory
    private let clock: CollaborationMonotonicClock
    private let eventSink: EventSink
    private let queue: DispatchQueue
    private let queueKey = DispatchSpecificKey<UInt8>()

    private var config: NativeCollaborationTransportConfig?
    private var socket: CollaborationSocket?
    private var generation: String?
    private var socketToken = UUID()
    private var timer: DispatchWorkItem?
    private var protocolAdapterTimer: DispatchWorkItem?
    private var inFlightLease: FfiOutboundLease?
    private var networkSocketOpened = false
    private var socketOpened = false
    private var protocolAttemptId: String?
    private var protocolEventSequence: UInt64 = 0
    private var pendingProtocolEventId: String?
    private var negotiatedProtocol: String?
    private var closeReported = false
    private var backgrounded = false
    private var destroyed = false

    init(
        editorId: String,
        backend: NativeCollaborationTransportBackend = RustNativeCollaborationTransportBackend(),
        socketFactory: CollaborationSocketFactory = URLSessionCollaborationSocketFactory(),
        clock: CollaborationMonotonicClock = SystemCollaborationMonotonicClock(),
        eventSink: @escaping EventSink = { _ in }
    ) {
        self.editorId = editorId
        self.backend = backend
        self.socketFactory = socketFactory
        self.clock = clock
        self.eventSink = eventSink
        queue = DispatchQueue(label: "com.apollohg.native-editor.collaboration.\(editorId)")
        queue.setSpecific(key: queueKey, value: 1)
    }

    @discardableResult
    func configure(_ newConfig: NativeCollaborationTransportConfig?) -> FfiError? {
        onQueueSync {
            guard !destroyed else {
                return lifecycleError("ENGINE_DESTROYED", "collaboration transport is destroyed")
            }
            if config == newConfig {
                if newConfig?.connect == true, !backgrounded {
                    drive(reason: .reattach)
                }
                return nil
            }

            retireNativeResources()
            if let error = normalizeUnit(backend.detach(editorId: editorId)) {
                emit(error)
                return error
            }
            config = newConfig

            guard newConfig?.connect == true, !backgrounded else {
                return nil
            }
            if let error = normalizeUnit(backend.reattach(editorId: editorId)) {
                emit(error)
                return error
            }
            drive(reason: .reattach)
            return nil
        }
    }

    func notifyOutboundAvailable(reason: CollaborationWakeReason) {
        queue.async { [weak self] in
            guard let self, self.canDrive else { return }
            self.drive(reason: reason)
        }
    }

    @discardableResult
    func resolveProtocolAdapter(
        attemptId: String,
        eventId: String,
        response: NativeCollaborationProtocolAdapterResponse
    ) -> FfiError? {
        onQueueSync {
            guard !destroyed else {
                return lifecycleError("ENGINE_DESTROYED", "collaboration transport is destroyed")
            }
            guard protocolAttemptId == attemptId,
                  pendingProtocolEventId == eventId,
                  let generation,
                  let socket
            else {
                // RN promises from a retired attempt are intentionally harmless.
                return nil
            }
            pendingProtocolEventId = nil
            let token = socketToken
            sendProtocolAdapterFrames(
                response.frames,
                index: 0,
                socket: socket,
                token: token,
                generation: generation,
                completion: { [weak self] success in
                    guard let self,
                          self.isCurrent(token: token, generation: generation),
                          self.protocolAttemptId == attemptId
                    else {
                        return
                    }
                    guard success else {
                        self.failCurrentSocket(
                            token: token,
                            generation: generation,
                            code: nil
                        )
                        return
                    }
                    switch response.action {
                    case .continue:
                        self.receiveNext(token: token, generation: generation)
                    case .ready:
                        self.activateYjs(token: token, generation: generation)
                    case .reject:
                        self.failCurrentSocket(
                            token: token,
                            generation: generation,
                            code: UInt32(URLSessionWebSocketTask.CloseCode.policyViolation.rawValue)
                        )
                    }
                }
            )
            return nil
        }
    }

    func enterBackground() {
        onQueueSync {
            guard !destroyed, !backgrounded else { return }
            backgrounded = true
            retireNativeResources()
            if let error = normalizeUnit(backend.detach(editorId: editorId)) {
                emit(error)
            }
        }
    }

    func enterForeground() {
        onQueueSync {
            guard !destroyed, backgrounded else { return }
            backgrounded = false
            guard config?.connect == true else { return }
            if let error = normalizeUnit(backend.reattach(editorId: editorId)) {
                emit(error)
                return
            }
            drive(reason: .reattach)
        }
    }

    func destroy() {
        onQueueSync {
            guard !destroyed else { return }
            destroyed = true
            retireNativeResources()
            if let error = normalizeUnit(backend.detach(editorId: editorId)) {
                emit(error)
            }
            config = nil
        }
    }

    private var canDrive: Bool {
        !destroyed && !backgrounded && config?.connect == true
    }

    private func drive(reason: CollaborationWakeReason) {
        guard canDrive else { return }
        consumeDirective(
            backend.drive(editorId: editorId, nowMillis: String(clock.nowMillis())),
            generation: generation,
            reason: reason
        )
    }

    @discardableResult
    private func consumeDirective(
        _ result: FfiJsonResult,
        generation eventGeneration: String?,
        reason: CollaborationWakeReason
    ) -> Bool {
        switch normalizeDirective(result) {
        case .failure(let error):
            emit(error, generation: eventGeneration)
            return false
        case .success(let directive):
            eventSink(.directive(
                directive: directive,
                generation: eventGeneration,
                wakeReason: reason
            ))
            schedule(deadline: directive.nextDeadlineMillis)
            if let generationToOpen = directive.generationToOpen {
                openSocket(generation: generationToOpen)
                return true
            }
            driveOutboundIfPossible()
            return true
        }
    }

    private func openSocket(generation newGeneration: String) {
        guard canDrive, let config else { return }
        retireNativeResources()

        generation = newGeneration
        closeReported = false
        networkSocketOpened = false
        socketOpened = false
        protocolAttemptId = config.protocolAdapter == nil ? nil : UUID().uuidString
        protocolEventSequence = 0
        pendingProtocolEventId = nil
        negotiatedProtocol = nil
        let token = UUID()
        socketToken = token
        let callbacks = CollaborationSocketCallbacks(
            didOpen: { [weak self] negotiatedProtocol in
                self?.queue.async {
                    self?.socketDidOpen(
                        token: token,
                        generation: newGeneration,
                        negotiatedProtocol: negotiatedProtocol
                    )
                }
            },
            didClose: { [weak self] code, _ in
                self?.queue.async {
                    self?.socketDidClose(
                        token: token,
                        generation: newGeneration,
                        code: UInt32(code.rawValue)
                    )
                }
            }
        )
        let newSocket = socketFactory.makeSocket(
            url: config.url,
            protocols: config.protocolAdapter?.protocols ?? [],
            callbacks: callbacks
        )
        socket = newSocket
        newSocket.resume()
    }

    private func socketDidOpen(
        token: UUID,
        generation callbackGeneration: String,
        negotiatedProtocol: String?
    ) {
        guard isCurrent(token: token, generation: callbackGeneration),
              !networkSocketOpened
        else {
            return
        }
        networkSocketOpened = true
        self.negotiatedProtocol = negotiatedProtocol
        guard config?.protocolAdapter != nil else {
            activateYjs(token: token, generation: callbackGeneration)
            return
        }
        scheduleProtocolAdapterTimeout(token: token, generation: callbackGeneration)
        emitProtocolAdapterEvent(
            phase: .open,
            token: token,
            generation: callbackGeneration
        )
    }

    private func activateYjs(token: UUID, generation callbackGeneration: String) {
        guard isCurrent(token: token, generation: callbackGeneration),
              networkSocketOpened,
              !socketOpened
        else {
            return
        }
        protocolAdapterTimer?.cancel()
        protocolAdapterTimer = nil
        pendingProtocolEventId = nil
        socketOpened = true
        let accepted = consumeDirective(
            backend.socketOpen(
                editorId: editorId,
                generation: callbackGeneration,
                nowMillis: String(clock.nowMillis())
            ),
            generation: callbackGeneration,
            reason: .open
        )
        guard accepted else {
            failCurrentSocket(token: token, generation: callbackGeneration, code: nil)
            return
        }
        receiveNext(token: token, generation: callbackGeneration)
    }

    private func receiveNext(token: UUID, generation callbackGeneration: String) {
        guard isCurrent(token: token, generation: callbackGeneration),
              networkSocketOpened,
              let socket
        else {
            return
        }
        socket.receive { [weak self] result in
            self?.queue.async {
                guard let self,
                      self.isCurrent(token: token, generation: callbackGeneration)
                else {
                    return
                }
                switch result {
                case .success(.text(let text)):
                    if !self.socketOpened, self.config?.protocolAdapter != nil {
                        guard self.protocolAdapterFrameIsWithinLimit(.text(text)) else {
                            self.failCurrentSocket(
                                token: token,
                                generation: callbackGeneration,
                                code: 1008
                            )
                            return
                        }
                        self.emitProtocolAdapterEvent(
                            phase: .message(.text(text)),
                            token: token,
                            generation: callbackGeneration
                        )
                    } else {
                        self.failCurrentSocket(
                            token: token,
                            generation: callbackGeneration,
                            code: 1008
                        )
                    }
                case .success(.binary(let data)):
                    guard self.socketOpened else {
                        if self.config?.protocolAdapter != nil {
                            guard self.protocolAdapterFrameIsWithinLimit(.binary(data)) else {
                                self.failCurrentSocket(
                                    token: token,
                                    generation: callbackGeneration,
                                    code: 1008
                                )
                                return
                            }
                            self.emitProtocolAdapterEvent(
                                phase: .message(.binary(data)),
                                token: token,
                                generation: callbackGeneration
                            )
                        } else {
                            self.failCurrentSocket(
                                token: token,
                                generation: callbackGeneration,
                                code: 1008
                            )
                        }
                        return
                    }
                    if self.consumeDirective(
                        self.backend.receive(
                            editorId: self.editorId,
                            generation: callbackGeneration,
                            message: data,
                            nowMillis: String(self.clock.nowMillis())
                        ),
                        generation: callbackGeneration,
                        reason: .receive
                    ) {
                        self.receiveNext(token: token, generation: callbackGeneration)
                    } else {
                        self.failCurrentSocket(
                            token: token,
                            generation: callbackGeneration,
                            code: nil
                        )
                    }
                case .failure:
                    self.failCurrentSocket(
                        token: token,
                        generation: callbackGeneration,
                        code: nil
                    )
                }
            }
        }
    }

    private func protocolAdapterFrameIsWithinLimit(
        _ frame: NativeCollaborationProtocolFrame
    ) -> Bool {
        switch frame {
        case .text(let text):
            return (text.data(using: .utf8)?.count ?? Int.max)
                <= NativeCollaborationProtocolAdapterConfig.maximumFrameBytes
        case .binary(let data):
            return data.count <= NativeCollaborationProtocolAdapterConfig.maximumFrameBytes
        }
    }

    private func driveOutboundIfPossible() {
        guard canDrive,
              socketOpened,
              inFlightLease == nil,
              let generation,
              let socket
        else {
            return
        }

        let result = backend.leaseOutbound(editorId: editorId, generation: generation)
        switch (result.value, result.empty, result.error) {
        case let (lease?, false, nil):
            inFlightLease = lease
            let token = socketToken
            socket.sendBinary(lease.frame) { [weak self] sendResult in
                self?.queue.async {
                    self?.sendCompleted(
                        sendResult,
                        token: token,
                        generation: generation,
                        lease: lease
                    )
                }
            }
        case (nil, true, nil):
            return
        case let (nil, false, error?):
            emit(error, generation: generation)
            failCurrentSocket(
                token: socketToken,
                generation: generation,
                code: UInt32(URLSessionWebSocketTask.CloseCode.policyViolation.rawValue)
            )
        default:
            emit(contractError("outbound lease result violates the frozen shape"), generation: generation)
            failCurrentSocket(
                token: socketToken,
                generation: generation,
                code: UInt32(URLSessionWebSocketTask.CloseCode.policyViolation.rawValue)
            )
        }
    }

    private func sendCompleted(
        _ result: Result<Void, Error>,
        token: UUID,
        generation callbackGeneration: String,
        lease: FfiOutboundLease
    ) {
        guard isCurrent(token: token, generation: callbackGeneration),
              inFlightLease?.leaseId == lease.leaseId
        else {
            return
        }
        inFlightLease = nil

        switch result {
        case .success:
            let ack = backend.ackOutbound(
                editorId: editorId,
                generation: callbackGeneration,
                leaseId: lease.leaseId
            )
            if let error = normalizeJsonUnit(ack) {
                emit(error, generation: callbackGeneration)
                failCurrentSocket(
                    token: token,
                    generation: callbackGeneration,
                    code: UInt32(URLSessionWebSocketTask.CloseCode.policyViolation.rawValue)
                )
                return
            }
            drive(reason: .localMutation)
        case .failure:
            let nack = backend.nackOutbound(
                editorId: editorId,
                generation: callbackGeneration,
                leaseId: lease.leaseId
            )
            if let error = normalizeJsonUnit(nack) {
                emit(error, generation: callbackGeneration)
            }
            failCurrentSocket(token: token, generation: callbackGeneration, code: nil)
        }
    }

    private func socketDidClose(token: UUID, generation: String, code: UInt32?) {
        guard isCurrent(token: token, generation: generation) else { return }
        let backendCode: UInt32?
        if let code,
           config?.protocolAdapter?.terminalCloseCodes.contains(code) == true
        {
            backendCode = UInt32(URLSessionWebSocketTask.CloseCode.policyViolation.rawValue)
        } else {
            backendCode = code
        }
        reportCurrentSocketClose(generation: generation, code: backendCode)
    }

    private func failCurrentSocket(token: UUID, generation: String, code: UInt32?) {
        guard isCurrent(token: token, generation: generation) else { return }
        socket?.cancel(code: .goingAway, reason: nil)
        reportCurrentSocketClose(generation: generation, code: code)
    }

    private func reportCurrentSocketClose(generation closingGeneration: String, code: UInt32?) {
        guard !closeReported else { return }
        closeReported = true
        timer?.cancel()
        timer = nil
        protocolAdapterTimer?.cancel()
        protocolAdapterTimer = nil
        socketToken = UUID()
        socket = nil
        networkSocketOpened = false
        socketOpened = false
        protocolAttemptId = nil
        pendingProtocolEventId = nil
        negotiatedProtocol = nil
        inFlightLease = nil
        generation = nil
        consumeDirective(
            backend.socketClose(
                editorId: editorId,
                generation: closingGeneration,
                code: code,
                reason: nil,
                nowMillis: String(clock.nowMillis())
            ),
            generation: closingGeneration,
            reason: .timer
        )
    }

    private func schedule(deadline: String?) {
        timer?.cancel()
        timer = nil
        guard canDrive,
              let deadline,
              let deadlineValue = canonicalUInt64(deadline)
        else {
            return
        }

        let now = clock.nowMillis()
        let delayMillis = deadlineValue > now ? deadlineValue - now : 0
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.timer = nil
            self.drive(reason: .timer)
        }
        timer = work
        let nanoseconds = min(delayMillis, UInt64(Int.max / 1_000_000)) * 1_000_000
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(nanoseconds)), execute: work)
    }

    private func scheduleProtocolAdapterTimeout(token: UUID, generation: String) {
        protocolAdapterTimer?.cancel()
        guard let timeoutMillis = config?.protocolAdapter?.timeoutMillis else { return }
        let work = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.protocolAdapterTimer = nil
            self.failCurrentSocket(token: token, generation: generation, code: 1008)
        }
        protocolAdapterTimer = work
        let nanoseconds = min(timeoutMillis, UInt64(Int.max / 1_000_000)) * 1_000_000
        queue.asyncAfter(deadline: .now() + .nanoseconds(Int(nanoseconds)), execute: work)
    }

    private func emitProtocolAdapterEvent(
        phase: NativeCollaborationProtocolAdapterPhase,
        token: UUID,
        generation: String
    ) {
        guard isCurrent(token: token, generation: generation),
              config?.protocolAdapter != nil,
              let attemptId = protocolAttemptId,
              pendingProtocolEventId == nil,
              protocolEventSequence < UInt64.max
        else {
            failCurrentSocket(token: token, generation: generation, code: 1008)
            return
        }
        protocolEventSequence += 1
        let eventId = String(protocolEventSequence)
        pendingProtocolEventId = eventId
        eventSink(.protocolAdapter(NativeCollaborationProtocolAdapterEvent(
            attemptId: attemptId,
            eventId: eventId,
            generation: generation,
            negotiatedProtocol: negotiatedProtocol,
            phase: phase
        )))
    }

    private func sendProtocolAdapterFrames(
        _ frames: [NativeCollaborationProtocolFrame],
        index: Int,
        socket: CollaborationSocket,
        token: UUID,
        generation: String,
        completion: @escaping (Bool) -> Void
    ) {
        guard isCurrent(token: token, generation: generation) else { return }
        guard index < frames.count else {
            completion(true)
            return
        }
        let callback: (Result<Void, Error>) -> Void = { [weak self] result in
            self?.queue.async {
                guard let self,
                      self.isCurrent(token: token, generation: generation)
                else {
                    return
                }
                switch result {
                case .success:
                    self.sendProtocolAdapterFrames(
                        frames,
                        index: index + 1,
                        socket: socket,
                        token: token,
                        generation: generation,
                        completion: completion
                    )
                case .failure:
                    completion(false)
                }
            }
        }
        switch frames[index] {
        case .text(let text):
            socket.sendText(text, completion: callback)
        case .binary(let data):
            socket.sendBinary(data, completion: callback)
        }
    }

    private func retireNativeResources() {
        timer?.cancel()
        timer = nil
        protocolAdapterTimer?.cancel()
        protocolAdapterTimer = nil
        socketToken = UUID()
        socket?.cancel(code: .goingAway, reason: nil)
        socket = nil
        generation = nil
        inFlightLease = nil
        networkSocketOpened = false
        socketOpened = false
        protocolAttemptId = nil
        pendingProtocolEventId = nil
        negotiatedProtocol = nil
        closeReported = true
    }

    private func isCurrent(token: UUID, generation callbackGeneration: String) -> Bool {
        !destroyed && token == socketToken && generation == callbackGeneration
    }

    private func normalizeDirective(
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

    private func normalizeUnit(_ result: FfiUnitResult) -> FfiError? {
        switch (result.value, result.error) {
        case (true?, nil):
            return nil
        case let (nil, error?):
            return error
        default:
            return contractError("v2 unit result violates the frozen shape")
        }
    }

    private func normalizeJsonUnit(_ result: FfiJsonResult) -> FfiError? {
        switch (result.value, result.error) {
        case (_?, nil):
            return nil
        case let (nil, error?):
            return error
        default:
            return contractError("v2 JSON result violates the frozen shape")
        }
    }

    private func canonicalUInt64(_ raw: String) -> UInt64? {
        guard !raw.isEmpty,
              raw.allSatisfy({ $0 >= "0" && $0 <= "9" }),
              raw == "0" || raw.first != "0"
        else {
            return nil
        }
        return UInt64(raw)
    }

    private func contractError(_ message: String) -> FfiError {
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

    private func lifecycleError(_ code: String, _ message: String) -> FfiError {
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

    private func emit(_ error: FfiError, generation: String? = nil) {
        eventSink(.error(error, generation: generation))
    }

    private func onQueueSync<T>(_ operation: () -> T) -> T {
        if DispatchQueue.getSpecific(key: queueKey) != nil {
            return operation()
        }
        return queue.sync(execute: operation)
    }
}
