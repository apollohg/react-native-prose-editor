import Foundation

/// One serialized native owner for one authentic Rust editor handle.
final class NativeCollaborationTransport {
    typealias EventSink = (NativeCollaborationTransportEvent) -> Void

    let editorId: String
    let backend: NativeCollaborationTransportBackend
    let socketFactory: CollaborationSocketFactory
    let clock: CollaborationMonotonicClock
    let eventSink: EventSink
    let queue: DispatchQueue
    private let queueKey = DispatchSpecificKey<UInt8>()

    var config: NativeCollaborationTransportConfig?
    var socket: CollaborationSocket?
    var generation: String?
    var socketToken = UUID()
    var timer: DispatchWorkItem?
    var protocolAdapterTimer: DispatchWorkItem?
    var inFlightLease: FfiOutboundLease?
    var networkSocketOpened = false
    var socketOpened = false
    var protocolAttemptId: String?
    var protocolEventSequence: UInt64 = 0
    var pendingProtocolEventId: String?
    var negotiatedProtocol: String?
    var closeReported = false
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

    var canDrive: Bool {
        !destroyed && !backgrounded && config?.connect == true
    }

    func schedule(deadline: String?) {
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

    func scheduleProtocolAdapterTimeout(token: UUID, generation: String) {
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

    func emitProtocolAdapterEvent(
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

    func retireNativeResources() {
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

    func isCurrent(token: UUID, generation callbackGeneration: String) -> Bool {
        !destroyed && token == socketToken && generation == callbackGeneration
    }

    func emit(_ error: FfiError, generation: String? = nil) {
        eventSink(.error(error, generation: generation))
    }

    private func onQueueSync<T>(_ operation: () -> T) -> T {
        if DispatchQueue.getSpecific(key: queueKey) != nil {
            return operation()
        }
        return queue.sync(execute: operation)
    }
}
