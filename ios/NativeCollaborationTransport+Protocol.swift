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
        performEditorV2JsonOperation(editorId: editorId) {
            editorV2CollaborationDrive(editorId: editorId, nowMillis: nowMillis)
        }
    }

    func socketOpen(editorId: String, generation: String, nowMillis: String) -> FfiJsonResult {
        performEditorV2JsonOperation(editorId: editorId) {
            editorV2CollaborationSocketOpen(
                editorId: editorId,
                generation: generation,
                nowMillis: nowMillis
            )
        }
    }

    func receive(
        editorId: String,
        generation: String,
        message: Data,
        nowMillis: String
    ) -> FfiJsonResult {
        performEditorV2JsonOperation(editorId: editorId) {
            editorV2CollaborationReceive(
                editorId: editorId,
                generation: generation,
                message: message,
                nowMillis: nowMillis
            )
        }
    }

    func socketClose(
        editorId: String,
        generation: String,
        code: UInt32?,
        reason: String?,
        nowMillis: String
    ) -> FfiJsonResult {
        performEditorV2JsonOperation(editorId: editorId) {
            editorV2CollaborationSocketClose(
                editorId: editorId,
                generation: generation,
                code: code,
                reason: reason,
                nowMillis: nowMillis
            )
        }
    }

    func leaseOutbound(editorId: String, generation: String) -> FfiOutboundLeaseResult {
        performEditorV2OutboundLeaseOperation(editorId: editorId) {
            editorV2CollaborationLeaseOutbound(editorId: editorId, generation: generation)
        }
    }

    func ackOutbound(editorId: String, generation: String, leaseId: String) -> FfiJsonResult {
        performEditorV2JsonOperation(editorId: editorId) {
            editorV2CollaborationAckOutbound(
                editorId: editorId,
                generation: generation,
                leaseId: leaseId
            )
        }
    }

    func nackOutbound(editorId: String, generation: String, leaseId: String) -> FfiJsonResult {
        performEditorV2JsonOperation(editorId: editorId) {
            editorV2CollaborationNackOutbound(
                editorId: editorId,
                generation: generation,
                leaseId: leaseId
            )
        }
    }

    func detach(editorId: String) -> FfiUnitResult {
        performEditorV2LifecycleUnitOperation(editorId: editorId) {
            editorV2CollaborationDetach(editorId: editorId)
        }
    }

    func reattach(editorId: String) -> FfiUnitResult {
        performEditorV2UnitOperation(editorId: editorId) {
            editorV2CollaborationReattach(editorId: editorId)
        }
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
