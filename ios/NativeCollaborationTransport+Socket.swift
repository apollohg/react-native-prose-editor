import Foundation

extension NativeCollaborationTransport {
    func drive(reason: CollaborationWakeReason) {
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
            },
            didFail: { [weak self] in
                self?.queue.async {
                    self?.failCurrentSocket(
                        token: token,
                        generation: newGeneration,
                        code: nil
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

    func activateYjs(token: UUID, generation callbackGeneration: String) {
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

    func receiveNext(token: UUID, generation callbackGeneration: String) {
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

    func failCurrentSocket(token: UUID, generation: String, code: UInt32?) {
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

}
