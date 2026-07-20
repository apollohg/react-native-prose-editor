#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed FFI and session error envelope"
)]

use crate::boundary::{BoundaryError, BoundedInput, InputKind, ResourceLimits};
use crate::yrs_engine::{
    DocumentSnapshot, EditingLimits, EngineRenderState, OperationError, YrsDocumentEngine,
    YrsEngineError,
};

pub(crate) use crate::yrs_engine::DocumentScope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentState {
    LocalReady,
    AwaitRemote,
    RoomReady,
}

impl DocumentState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::LocalReady => "LocalReady",
            Self::AwaitRemote => "AwaitRemote",
            Self::RoomReady => "RoomReady",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportState {
    Detached,
    Disconnected,
    Connecting,
    Handshaking,
    Synchronized,
    Incompatible,
    Destroying,
    Destroyed,
}

impl TransportState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Detached => "Detached",
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting",
            Self::Handshaking => "Handshaking",
            Self::Synchronized => "Synchronized",
            Self::Incompatible => "Incompatible",
            Self::Destroying => "Destroying",
            Self::Destroyed => "Destroyed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InitialContent {
    Empty,
    Json(String),
    Html(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EditorInitialization {
    Local {
        initial_content: InitialContent,
    },
    Room {
        scope: DocumentScope,
        snapshot: Option<DocumentSnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollaborationLimits {
    pub(crate) max_frames_per_message: usize,
    pub(crate) max_frame_bytes: usize,
    pub(crate) max_aggregate_response_bytes: usize,
    pub(crate) max_awareness_peers: usize,
    pub(crate) max_awareness_peer_bytes: usize,
    pub(crate) max_awareness_bytes: usize,
    pub(crate) max_pending_outbox_messages: usize,
    pub(crate) max_pending_outbox_bytes: usize,
    pub(crate) max_pending_dependency_update_bytes: usize,
    pub(crate) max_pending_dependency_update_work: usize,
}

impl Default for CollaborationLimits {
    fn default() -> Self {
        Self {
            max_frames_per_message: 64,
            max_frame_bytes: 10 * 1024 * 1024,
            max_aggregate_response_bytes: 10 * 1024 * 1024,
            max_awareness_peers: 1_024,
            max_awareness_peer_bytes: 64 * 1024,
            max_awareness_bytes: 10 * 1024 * 1024,
            max_pending_outbox_messages: 256,
            max_pending_outbox_bytes: 10 * 1024 * 1024,
            max_pending_dependency_update_bytes: 10 * 1024 * 1024,
            max_pending_dependency_update_work: 1_000_000,
        }
    }
}

impl CollaborationLimits {
    pub(crate) const fn hard_ceiling() -> Self {
        Self {
            max_frames_per_message: 1_024,
            max_frame_bytes: 64 * 1024 * 1024,
            max_aggregate_response_bytes: 64 * 1024 * 1024,
            max_awareness_peers: 10_000,
            max_awareness_peer_bytes: 1024 * 1024,
            max_awareness_bytes: 64 * 1024 * 1024,
            max_pending_outbox_messages: 4_096,
            max_pending_outbox_bytes: 64 * 1024 * 1024,
            max_pending_dependency_update_bytes: 64 * 1024 * 1024,
            max_pending_dependency_update_work: 8_000_000,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), SessionError> {
        let ceilings = Self::hard_ceiling();
        for (field, actual, ceiling) in [
            (
                "maxFramesPerMessage",
                self.max_frames_per_message,
                ceilings.max_frames_per_message,
            ),
            (
                "maxFrameBytes",
                self.max_frame_bytes,
                ceilings.max_frame_bytes,
            ),
            (
                "maxAggregateResponseBytes",
                self.max_aggregate_response_bytes,
                ceilings.max_aggregate_response_bytes,
            ),
            (
                "maxAwarenessPeers",
                self.max_awareness_peers,
                ceilings.max_awareness_peers,
            ),
            (
                "maxAwarenessPeerBytes",
                self.max_awareness_peer_bytes,
                ceilings.max_awareness_peer_bytes,
            ),
            (
                "maxAwarenessBytes",
                self.max_awareness_bytes,
                ceilings.max_awareness_bytes,
            ),
            (
                "maxPendingOutboxMessages",
                self.max_pending_outbox_messages,
                ceilings.max_pending_outbox_messages,
            ),
            (
                "maxPendingOutboxBytes",
                self.max_pending_outbox_bytes,
                ceilings.max_pending_outbox_bytes,
            ),
            (
                "maxPendingDependencyUpdateBytes",
                self.max_pending_dependency_update_bytes,
                ceilings.max_pending_dependency_update_bytes,
            ),
            (
                "maxPendingDependencyUpdateWork",
                self.max_pending_dependency_update_work,
                ceilings.max_pending_dependency_update_work,
            ),
        ] {
            if actual == 0 || actual > ceiling {
                return Err(SessionError::new(
                    ErrorDomain::Boundary,
                    "INVALID_RESOURCE_LIMIT",
                    format!("{field} must be a positive integer no greater than {ceiling}"),
                )
                .with_limit(ceiling, actual)
                .with_details(serde_json::json!({ "field": field })));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) const fn fields() -> [&'static str; 10] {
        [
            "maxFramesPerMessage",
            "maxFrameBytes",
            "maxAggregateResponseBytes",
            "maxAwarenessPeers",
            "maxAwarenessPeerBytes",
            "maxAwarenessBytes",
            "maxPendingOutboxMessages",
            "maxPendingOutboxBytes",
            "maxPendingDependencyUpdateBytes",
            "maxPendingDependencyUpdateWork",
        ]
    }

    #[cfg(test)]
    pub(crate) fn value(&self, field: &str) -> usize {
        match field {
            "maxFramesPerMessage" => self.max_frames_per_message,
            "maxFrameBytes" => self.max_frame_bytes,
            "maxAggregateResponseBytes" => self.max_aggregate_response_bytes,
            "maxAwarenessPeers" => self.max_awareness_peers,
            "maxAwarenessPeerBytes" => self.max_awareness_peer_bytes,
            "maxAwarenessBytes" => self.max_awareness_bytes,
            "maxPendingOutboxMessages" => self.max_pending_outbox_messages,
            "maxPendingOutboxBytes" => self.max_pending_outbox_bytes,
            "maxPendingDependencyUpdateBytes" => self.max_pending_dependency_update_bytes,
            "maxPendingDependencyUpdateWork" => self.max_pending_dependency_update_work,
            _ => unreachable!("unknown collaboration limit field"),
        }
    }

    /// Test-only field mutation by wire name, shared by the in-crate limits
    /// matrix and the staging integration-test support (which drives the
    /// Task 9 receive ceilings to their exact/one-over boundaries).
    #[cfg(any(test, feature = "ffi-v2-staging"))]
    pub(crate) fn set_for_test(&mut self, field: &str, value: usize) {
        match field {
            "maxFramesPerMessage" => self.max_frames_per_message = value,
            "maxFrameBytes" => self.max_frame_bytes = value,
            "maxAggregateResponseBytes" => self.max_aggregate_response_bytes = value,
            "maxAwarenessPeers" => self.max_awareness_peers = value,
            "maxAwarenessPeerBytes" => self.max_awareness_peer_bytes = value,
            "maxAwarenessBytes" => self.max_awareness_bytes = value,
            "maxPendingOutboxMessages" => self.max_pending_outbox_messages = value,
            "maxPendingOutboxBytes" => self.max_pending_outbox_bytes = value,
            "maxPendingDependencyUpdateBytes" => self.max_pending_dependency_update_bytes = value,
            "maxPendingDependencyUpdateWork" => self.max_pending_dependency_update_work = value,
            _ => unreachable!("unknown collaboration limit field"),
        }
    }

    #[cfg(test)]
    pub(crate) fn as_pairs_json(&self) -> serde_json::Value {
        serde_json::json!({
            "maxFramesPerMessage": self.max_frames_per_message,
            "maxFrameBytes": self.max_frame_bytes,
            "maxAggregateResponseBytes": self.max_aggregate_response_bytes,
            "maxAwarenessPeers": self.max_awareness_peers,
            "maxAwarenessPeerBytes": self.max_awareness_peer_bytes,
            "maxAwarenessBytes": self.max_awareness_bytes,
            "maxPendingOutboxMessages": self.max_pending_outbox_messages,
            "maxPendingOutboxBytes": self.max_pending_outbox_bytes,
            "maxPendingDependencyUpdateBytes": self.max_pending_dependency_update_bytes,
            "maxPendingDependencyUpdateWork": self.max_pending_dependency_update_work,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EditorSessionConfig {
    pub(crate) schema_json: Option<String>,
    pub(crate) fragment_name: String,
    pub(crate) initialization: EditorInitialization,
    pub(crate) resource_limits: ResourceLimits,
    pub(crate) editing_limits: EditingLimits,
    pub(crate) collaboration_limits: CollaborationLimits,
    pub(crate) max_length: Option<u32>,
    pub(crate) read_only: bool,
    pub(crate) input_filter: Option<String>,
    pub(crate) allow_base64_images: bool,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomInitializationEnvelope {
    document_id: String,
    lineage_id: String,
    snapshot: Option<DocumentSnapshot>,
}

impl EditorSessionConfig {
    pub(crate) fn room_from_json(input: &str) -> Result<Self, SessionError> {
        let limits = ResourceLimits::default();
        let input = BoundedInput::new(input, InputKind::Config, &limits)?;
        let envelope: RoomInitializationEnvelope = serde_json::from_str(input.as_str())
            .map_err(|error| BoundaryError::parse("CONFIG_INVALID", error))?;
        Ok(Self {
            schema_json: None,
            fragment_name: "prosemirror".into(),
            initialization: EditorInitialization::Room {
                scope: DocumentScope {
                    document_id: envelope.document_id,
                    lineage_id: envelope.lineage_id,
                },
                snapshot: envelope.snapshot,
            },
            resource_limits: limits,
            editing_limits: EditingLimits::default(),
            collaboration_limits: CollaborationLimits::default(),
            max_length: None,
            read_only: false,
            input_filter: None,
            allow_base64_images: false,
        })
    }
}

#[cfg(test)]
impl EditorSessionConfig {
    pub(crate) fn local_for_test() -> Self {
        Self {
            schema_json: None,
            fragment_name: "prosemirror".into(),
            initialization: EditorInitialization::Local {
                initial_content: InitialContent::Empty,
            },
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            collaboration_limits: CollaborationLimits::default(),
            max_length: None,
            read_only: false,
            input_filter: None,
            allow_base64_images: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorDomain {
    Boundary,
    Document,
    Operation,
    Lifecycle,
    Snapshot,
    Transport,
}

impl ErrorDomain {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Boundary => "boundary",
            Self::Document => "document",
            Self::Operation => "operation",
            Self::Lifecycle => "lifecycle",
            Self::Snapshot => "snapshot",
            Self::Transport => "transport",
        }
    }
}

fn legacy_error_domain(code: &str) -> ErrorDomain {
    match code {
        "DOCUMENT_PARSE_FAILED"
        | "DOCUMENT_INVALID"
        | "DOCUMENT_LIMIT_EXCEEDED"
        | "POSITION_LIMIT_EXCEEDED"
        | "SCHEMA_INVALID"
        | "REQUIRED_ATTRIBUTE_MISSING"
        | "UNKNOWN_MARK"
        | "MAX_LENGTH_EXCEEDED" => ErrorDomain::Document,
        "ENGINE_NOT_READY"
        | "REVISION_MISMATCH"
        | "POSITION_INVALID"
        | "TRANSACTION_INVALID"
        | "OPERATION_INVALID"
        | "OPERATION_LIMIT_EXCEEDED"
        | "OPERATION_RESOURCE_EXHAUSTED"
        | "ENGINE_INVARIANT_FAILED" => ErrorDomain::Operation,
        _ => ErrorDomain::Boundary,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationFailureClass {
    ExistingStableCode,
    DeterministicOperationLimit,
    DeterministicDocumentLimit,
    AllocationOrReservation,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SessionError {
    pub(crate) domain: ErrorDomain,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) request_id: Option<u64>,
    pub(crate) operation_index: Option<usize>,
    pub(crate) limit: Option<u64>,
    pub(crate) actual: Option<u64>,
    pub(crate) details: Option<serde_json::Value>,
}

impl SessionError {
    pub(crate) fn new(
        domain: ErrorDomain,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            domain,
            code: code.into(),
            message: message.into(),
            request_id: None,
            operation_index: None,
            limit: None,
            actual: None,
            details: None,
        }
    }

    pub(crate) fn from_operation(
        error: OperationError,
        failure_class: OperationFailureClass,
    ) -> Self {
        let (domain, code) = match (error.code, failure_class) {
            (
                "OPERATION_RESOURCE_EXHAUSTED",
                OperationFailureClass::DeterministicOperationLimit,
            ) => (ErrorDomain::Operation, "OPERATION_LIMIT_EXCEEDED"),
            ("OPERATION_RESOURCE_EXHAUSTED", OperationFailureClass::DeterministicDocumentLimit) => {
                (ErrorDomain::Document, "DOCUMENT_LIMIT_EXCEEDED")
            }
            ("OPERATION_RESOURCE_EXHAUSTED", OperationFailureClass::ExistingStableCode) => {
                (ErrorDomain::Operation, "OPERATION_LIMIT_EXCEEDED")
            }
            _ => (legacy_error_domain(error.code), error.code),
        };
        Self {
            domain,
            code: code.into(),
            message: error.message.into(),
            request_id: Some(error.request_id),
            operation_index: error.operation_index,
            limit: error.limit,
            actual: error.actual,
            details: error.details,
        }
    }

    pub(crate) fn lifecycle(error: YrsEngineError) -> Self {
        Self::from_engine_error(error, ErrorDomain::Lifecycle)
    }

    pub(crate) fn snapshot(error: YrsEngineError) -> Self {
        Self::from_engine_error(error, ErrorDomain::Snapshot)
    }

    pub(crate) fn transport(error: YrsEngineError) -> Self {
        Self::from_engine_error(error, ErrorDomain::Transport)
    }

    fn from_engine_error(error: YrsEngineError, domain: ErrorDomain) -> Self {
        Self {
            domain,
            code: error.code.into(),
            message: error.message,
            request_id: None,
            operation_index: None,
            limit: error.limit.and_then(|value| u64::try_from(value).ok()),
            actual: error.actual.and_then(|value| u64::try_from(value).ok()),
            details: error.details.filter(serde_json::Value::is_object),
        }
    }

    fn with_limit(mut self, limit: usize, actual: usize) -> Self {
        self.limit = u64::try_from(limit).ok();
        self.actual = u64::try_from(actual).ok();
        self
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details.is_object().then_some(details);
        self
    }
}

impl From<BoundaryError> for SessionError {
    fn from(error: BoundaryError) -> Self {
        let domain = legacy_error_domain(error.code);
        Self {
            domain,
            code: error.code.into(),
            message: error.message,
            request_id: None,
            operation_index: None,
            limit: error.limit.and_then(|value| u64::try_from(value).ok()),
            actual: error.actual.and_then(|value| u64::try_from(value).ok()),
            details: error.details.filter(serde_json::Value::is_object),
        }
    }
}

impl From<YrsEngineError> for SessionError {
    fn from(error: YrsEngineError) -> Self {
        let domain = legacy_error_domain(error.code);
        Self::from_engine_error(error, domain)
    }
}

pub(crate) struct EditorSession {
    pub(crate) engine: YrsDocumentEngine,
    pub(crate) policy: SessionPolicy,
    pub(crate) native_bridge: NativeBridgeLifecycle,
    pub(crate) document_state: DocumentState,
    pub(crate) collaboration: CollaborationLifecycle,
}

pub(crate) struct SessionPolicy {
    read_only: bool,
    input_filter: Option<String>,
    allow_base64_images: bool,
}

/// Lifecycle owned by the session until Task 7 adds transaction translation.
pub(crate) struct NativeBridgeLifecycle {
    active: bool,
    #[cfg(feature = "ffi-v2-staging")]
    lifecycle_test_calls: usize,
}

/// Lifecycle plus the optionally attached collaboration runtime (Task 7:
/// outbox host; Task 8: the generation-owned transport state machine).
pub(crate) struct CollaborationLifecycle {
    active: bool,
    limits: CollaborationLimits,
    /// Shipped default builds keep the plain field (constructor and
    /// teardown are its only writers); under staging the state machine is
    /// the single writer of the transport state.
    #[cfg(not(feature = "ffi-v2-staging"))]
    transport_state: TransportState,
    #[cfg(feature = "ffi-v2-staging")]
    transport: crate::collaboration_runtime::state::TransportStateMachine,
    #[cfg(feature = "ffi-v2-staging")]
    runtime: Option<crate::collaboration_runtime::CollaborationRuntime>,
    #[cfg(feature = "ffi-v2-staging")]
    lifecycle_test_teardowns: usize,
}

impl EditorSession {
    pub(crate) fn new(
        engine: YrsDocumentEngine,
        policy: SessionPolicy,
        document_state: DocumentState,
        collaboration_limits: CollaborationLimits,
    ) -> Result<Self, SessionError> {
        collaboration_limits.validate()?;
        let transport_state = match document_state {
            DocumentState::LocalReady => TransportState::Detached,
            DocumentState::AwaitRemote | DocumentState::RoomReady => TransportState::Disconnected,
        };
        Ok(Self {
            engine,
            policy,
            native_bridge: NativeBridgeLifecycle {
                active: true,
                #[cfg(feature = "ffi-v2-staging")]
                lifecycle_test_calls: 0,
            },
            document_state,
            collaboration: CollaborationLifecycle {
                active: true,
                limits: collaboration_limits,
                #[cfg(not(feature = "ffi-v2-staging"))]
                transport_state,
                #[cfg(feature = "ffi-v2-staging")]
                transport: crate::collaboration_runtime::state::TransportStateMachine::new(
                    transport_state,
                ),
                #[cfg(feature = "ffi-v2-staging")]
                runtime: None,
                #[cfg(feature = "ffi-v2-staging")]
                lifecycle_test_teardowns: 0,
            },
        })
    }

    pub(crate) fn teardown(&mut self) {
        self.native_bridge.teardown();
        self.collaboration.teardown();
    }

    pub(crate) fn transport_state(&self) -> TransportState {
        #[cfg(feature = "ffi-v2-staging")]
        {
            self.collaboration.transport.state()
        }
        #[cfg(not(feature = "ffi-v2-staging"))]
        {
            self.collaboration.transport_state
        }
    }

    pub(crate) fn render_state(&self) -> EngineRenderState {
        self.engine.render_state()
    }

    pub(crate) fn get_json(&self) -> Result<serde_json::Value, SessionError> {
        self.engine.document_json().ok_or_else(engine_not_ready)
    }

    pub(crate) fn get_html(&self) -> Result<String, SessionError> {
        self.engine.document_html().ok_or_else(engine_not_ready)
    }

    /// Same-store whole-document replacement from ProseMirror JSON, behind
    /// the session policy gate. An attached collaboration runtime captures
    /// the replacement's outbound update through its bounded outbox.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn replace_document_json(
        &mut self,
        request_id: u64,
        json: &str,
        history: crate::yrs_engine::ReplacementHistory,
    ) -> Result<crate::yrs_engine::TransactionCommit, SessionError> {
        self.admit_whole_document_replacement(request_id)?;
        let (engine, outbox) = split_engine_and_outbox(&mut self.engine, &mut self.collaboration);
        engine
            .prepare_root_replacement_json_with_outbox(request_id, json, history, outbox)
            .map_err(|error| replacement_session_error(error, request_id))
    }

    /// Same-store whole-document replacement from HTML, behind the session
    /// policy gate.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn replace_document_html(
        &mut self,
        request_id: u64,
        html: &str,
        history: crate::yrs_engine::ReplacementHistory,
    ) -> Result<crate::yrs_engine::TransactionCommit, SessionError> {
        self.admit_whole_document_replacement(request_id)?;
        let options = crate::serialize::FromHtmlOptions {
            strict: false,
            allow_base64_images: self.policy.allow_base64_images,
        };
        let (engine, outbox) = split_engine_and_outbox(&mut self.engine, &mut self.collaboration);
        engine
            .prepare_root_replacement_html_with_outbox(request_id, html, &options, history, outbox)
            .map_err(|error| replacement_session_error(error, request_id))
    }

    /// Attach the collaboration runtime (Task 7: the bounded document
    /// outbox) sized from the session's validated collaboration limits.
    /// Idempotent: an already attached runtime and its pending outbox
    /// entries are preserved.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn attach_collaboration_runtime(&mut self) {
        if self.collaboration.runtime.is_none() {
            self.collaboration.runtime = Some(
                crate::collaboration_runtime::CollaborationRuntime::new(&self.collaboration.limits),
            );
        }
    }

    /// The attached runtime's outbox, if any. Detached/local-only sessions
    /// return `None`: no outbox exists for them by construction.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn collaboration_outbox(
        &self,
    ) -> Option<&crate::collaboration_runtime::CollaborationOutbox> {
        self.collaboration
            .runtime
            .as_ref()
            .map(crate::collaboration_runtime::CollaborationRuntime::outbox)
    }

    /// Split borrow used by every durable local mutation path: the engine
    /// plus the optionally attached outbox for pre-write reservation.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn engine_and_outbox(
        &mut self,
    ) -> (
        &mut YrsDocumentEngine,
        Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) {
        split_engine_and_outbox(&mut self.engine, &mut self.collaboration)
    }

    /// Document-state x transport-state policy matrix for whole-document
    /// replacement, checked before any engine work:
    ///
    /// - `AwaitRemote` + any transport rejects `ENGINE_NOT_READY`.
    /// - Any document state + `Connecting`/`Handshaking`/`Synchronized`
    ///   rejects the frozen lifecycle `WHOLE_DOCUMENT_REPLACEMENT_CONNECTED`.
    /// - `LocalReady`/`RoomReady` + `Detached`/`Disconnected`/`Incompatible`
    ///   is allowed. `Destroying`/`Destroyed` are unreachable via `with_alive`.
    #[cfg(feature = "ffi-v2-staging")]
    fn admit_whole_document_replacement(&self, request_id: u64) -> Result<(), SessionError> {
        if self.document_state == DocumentState::AwaitRemote {
            let mut error = engine_not_ready();
            error.request_id = Some(request_id);
            return Err(error);
        }
        match self.transport_state() {
            TransportState::Connecting
            | TransportState::Handshaking
            | TransportState::Synchronized => {
                let mut error = SessionError::new(
                    ErrorDomain::Lifecycle,
                    "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED",
                    format!(
                        "whole-document replacement is rejected while the collaboration \
                         transport is {}",
                        self.transport_state().as_str()
                    ),
                );
                error.request_id = Some(request_id);
                Err(error)
            }
            TransportState::Detached
            | TransportState::Disconnected
            | TransportState::Incompatible => Ok(()),
            TransportState::Destroying | TransportState::Destroyed => unreachable!(
                "with_alive rejects destroying/destroyed sessions before policy evaluation"
            ),
        }
    }

    /// Whether this session is bound to a collaboration room. Local-only
    /// sessions (`LocalReady`) have no scope to connect to, so every
    /// connection-shaped transport action on them is refused.
    #[cfg(feature = "ffi-v2-staging")]
    fn room_bound(&self) -> bool {
        self.document_state != DocumentState::LocalReady
    }

    /// `Disconnected` -> `Connecting`; returns the generation JavaScript
    /// must carry through its socket callbacks. Runs under the session lock
    /// like every transition (callers only reach sessions via `with_alive`).
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn begin_connect(
        &mut self,
        request_id: u64,
    ) -> Result<crate::collaboration_runtime::state::TransportGeneration, SessionError> {
        let room_bound = self.room_bound();
        self.collaboration
            .transport
            .begin_connect(request_id, room_bound)
    }

    /// Current `Connecting` -> `Handshaking`; the caller owes a Sync Step 1
    /// send (framed in Task 9).
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn socket_opened(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
    ) -> Result<crate::collaboration_runtime::state::HandshakeDirective, SessionError> {
        self.collaboration
            .transport
            .socket_opened(request_id, generation)
    }

    /// Current-generation close with a Rust-owned disposition; stale
    /// generations are observable no-ops. An accepted close clears the
    /// transport-scoped awareness peers (Task 10); desired local awareness
    /// survives by construction.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn socket_closed(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        disposition: crate::collaboration_runtime::state::SocketCloseDisposition,
    ) -> Result<TransportState, SessionError> {
        let state =
            self.collaboration
                .transport
                .socket_closed(request_id, generation, disposition)?;
        self.clear_transport_scoped_peers();
        Ok(state)
    }

    /// Local intent: close the live attempt -> `Disconnected`. Clears the
    /// transport-scoped awareness peers like every generation close.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn disconnect(&mut self, request_id: u64) -> Result<(), SessionError> {
        self.collaboration.transport.disconnect(request_id)?;
        self.clear_transport_scoped_peers();
        Ok(())
    }

    /// Tear down transport state only (-> `Detached`); the runtime, its
    /// pending outbox entries, and the document are untouched by design.
    /// Remote awareness peers are transport-scoped and clear here too.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn detach(&mut self, request_id: u64) -> Result<(), SessionError> {
        self.collaboration.transport.detach(request_id)?;
        self.clear_transport_scoped_peers();
        Ok(())
    }

    /// Explicit reattach half of the `Incompatible` escape hatch:
    /// `Detached` -> `Disconnected` on a room-bound session. Peer clearing
    /// is repeated defensively — reattach begins a fresh transport scope.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn reattach(&mut self, request_id: u64) -> Result<(), SessionError> {
        let room_bound = self.room_bound();
        self.collaboration
            .transport
            .reattach(request_id, room_bound)?;
        self.clear_transport_scoped_peers();
        Ok(())
    }

    /// Task 10 lifecycle rule shared by every accepted generation-closing
    /// transition: remote awareness peers are transport-scoped, desired
    /// local awareness is retained. Sessions without an attached runtime
    /// own no peers by construction.
    #[cfg(feature = "ffi-v2-staging")]
    fn clear_transport_scoped_peers(&mut self) {
        if let Some(runtime) = self.collaboration.runtime.as_mut() {
            runtime.clear_transport_peers(&mut self.engine);
        }
    }

    /// Crate-private Task 9 seam: an accepted current-generation Sync
    /// Step 2 turns `Handshaking` into `Synchronized`. Transport-only —
    /// document-state promotion is Task 9's Step 2 handling.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn mark_synchronized(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
    ) -> Result<(), SessionError> {
        self.collaboration
            .transport
            .mark_synchronized(request_id, generation)
    }

    /// Task 9 protocol entry point: one bounded inbound y-sync message for
    /// the given generation. The runtime composes the sealed seams (engine
    /// state-vector/diff and prepare/commit, outbox reservation, transport
    /// generation discipline); the session only performs the field-disjoint
    /// split borrow. Sessions without an attached runtime own no protocol
    /// surface and refuse like every other runtime-shaped operation.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn receive_message(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
    ) -> Result<crate::collaboration_runtime::protocol::ReceiveOutcome, SessionError> {
        let CollaborationLifecycle {
            transport,
            runtime,
            limits,
            ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        runtime.receive_message(
            request_id,
            generation,
            crate::collaboration_runtime::protocol::ReceiveContext {
                transport,
                engine: &mut self.engine,
                document_state: &mut self.document_state,
                limits,
            },
            bytes,
        )
    }

    /// The standard framed Sync Step 1 message the caller owes after
    /// `socket_opened` (read-only; framing lives in the protocol module).
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn sync_step1_message(&self, request_id: u64) -> Result<Vec<u8>, SessionError> {
        crate::collaboration_runtime::protocol::sync_step1_message(&self.engine, request_id)
    }

    /// Task 10: publish the desired local awareness state (runtime-owned,
    /// codec-validated). Requires an attached runtime like every other
    /// runtime-shaped operation.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn set_desired_awareness(
        &mut self,
        request_id: u64,
        state_json: &str,
    ) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_desired_awareness(request_id, state_json, context)
    }

    /// Task 10: withdraw the desired local awareness state.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn clear_desired_awareness(&mut self, request_id: u64) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.clear_desired_awareness(request_id, context)
    }

    /// Task 10: the retained desired local awareness state, if any.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn desired_awareness(&self) -> Result<Option<serde_json::Value>, SessionError> {
        let runtime = self
            .collaboration
            .runtime
            .as_ref()
            .ok_or_else(no_attached_runtime)?;
        Ok(runtime.desired_awareness().cloned())
    }

    /// Task 10: public awareness peer projections with cursors resolved
    /// against the current document (recomputed on every read).
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn awareness_peers(
        &mut self,
    ) -> Result<Vec<crate::collaboration_runtime::awareness::AwarenessPeerProjection>, SessionError>
    {
        let runtime = self
            .collaboration
            .runtime
            .as_ref()
            .ok_or_else(no_attached_runtime)?;
        Ok(runtime.peers(&mut self.engine))
    }

    /// Task 10: deterministic awareness clock work; `now_millis` is the only
    /// time source (JavaScript schedules the returned deadline).
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn awareness_tick(
        &mut self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<crate::collaboration_runtime::awareness::TickOutcome, SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.tick(request_id, now_millis, context)
    }

    /// Field-disjoint split borrow for awareness operations, mirroring the
    /// Task 9 receive split: the runtime plus the engine/limits/transport
    /// context it composes.
    #[cfg(feature = "ffi-v2-staging")]
    fn awareness_runtime_and_context(
        &mut self,
    ) -> Result<
        (
            &mut crate::collaboration_runtime::CollaborationRuntime,
            crate::collaboration_runtime::awareness::AwarenessContext<'_>,
        ),
        SessionError,
    > {
        let CollaborationLifecycle {
            transport,
            runtime,
            limits,
            ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        Ok((
            runtime,
            crate::collaboration_runtime::awareness::AwarenessContext {
                engine: &mut self.engine,
                transport_state: transport.state(),
                limits,
            },
        ))
    }

    /// Engine-owned retained dependency bytes plus the runtime's charged
    /// work units (`0` work without an attached runtime by construction).
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn remote_dependency_accounting(&self) -> (usize, u64) {
        let work = self.collaboration.runtime.as_ref().map_or(
            0,
            crate::collaboration_runtime::CollaborationRuntime::remote_dependency_work,
        );
        (self.engine.pending_remote_dependency_bytes(), work)
    }

    /// Test-only collaboration-limit override by wire field name, mirroring
    /// the outbox `set_ceilings_for_test` idiom for exact/one-over receive
    /// ceiling coverage.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn set_collaboration_limit_for_test(&mut self, field: &str, value: usize) {
        self.collaboration.limits.set_for_test(field, value);
    }

    /// Test-only transport-state injection, routed through the state
    /// machine so its live-attempt invariant holds (forced states carry no
    /// live attempt). It remains alongside the real transitions for exactly
    /// one reason: policy-matrix cells that are unreachable by construction
    /// — `LocalReady` sessions are permanently `Detached`, yet the Task 5
    /// replacement gate deliberately covers them for every transport state.
    /// Room-bound tests must use the real transitions instead.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn set_transport_state_for_test(&mut self, state: TransportState) {
        self.collaboration.transport.set_state_for_test(state);
    }

    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn lifecycle_test_session(reject_admission: bool) -> Result<Self, SessionError> {
        use crate::schema::presets::tiptap_schema;
        use crate::yrs_engine::{InitializationMode, YrsEngineConfig};

        let mut collaboration_limits = CollaborationLimits::default();
        if reject_admission {
            collaboration_limits.max_frames_per_message = 0;
        }
        collaboration_limits.validate()?;

        let engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .map_err(SessionError::from)?;

        Self::new(
            engine,
            SessionPolicy {
                read_only: false,
                input_filter: None,
                allow_base64_images: false,
            },
            DocumentState::LocalReady,
            collaboration_limits,
        )
    }

    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn record_lifecycle_test_call(&mut self) -> usize {
        self.native_bridge.lifecycle_test_calls += 1;
        self.native_bridge.lifecycle_test_calls
    }

    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn lifecycle_test_call_count(&self) -> usize {
        self.native_bridge.lifecycle_test_calls
    }

    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn lifecycle_test_teardown_count(&self) -> usize {
        self.collaboration.lifecycle_test_teardowns
    }
}

impl SessionPolicy {
    pub(crate) fn from_config(config: &EditorSessionConfig) -> Self {
        Self {
            read_only: config.read_only,
            input_filter: config.input_filter.clone(),
            allow_base64_images: config.allow_base64_images,
        }
    }

    /// Read-only policy: input and command mutations are rejected while
    /// local-API requests pass, mirroring the legacy `Source::Api` gate.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn read_only(&self) -> bool {
        self.read_only
    }

    /// Per-character input filter pattern applied to local-input text.
    #[cfg(feature = "ffi-v2-staging")]
    pub(crate) fn input_filter(&self) -> Option<&str> {
        self.input_filter.as_deref()
    }
}

/// Field-disjoint split of the session into its engine and the optionally
/// attached collaboration outbox, so pre-write reservation can run while the
/// engine is mutably borrowed.
#[cfg(feature = "ffi-v2-staging")]
fn split_engine_and_outbox<'session>(
    engine: &'session mut YrsDocumentEngine,
    collaboration: &'session mut CollaborationLifecycle,
) -> (
    &'session mut YrsDocumentEngine,
    Option<&'session mut crate::collaboration_runtime::CollaborationOutbox>,
) {
    let outbox = collaboration
        .runtime
        .as_mut()
        .map(crate::collaboration_runtime::CollaborationRuntime::outbox_mut);
    (engine, outbox)
}

impl NativeBridgeLifecycle {
    fn teardown(&mut self) {
        self.active = false;
    }
}

impl CollaborationLifecycle {
    fn teardown(&mut self) {
        if self.active {
            self.active = false;
            #[cfg(not(feature = "ffi-v2-staging"))]
            {
                self.transport_state = TransportState::Destroyed;
            }
            #[cfg(feature = "ffi-v2-staging")]
            {
                self.transport.teardown_destroyed();
                self.runtime = None;
                self.lifecycle_test_teardowns += 1;
            }
        }
    }
}

#[cfg(feature = "ffi-v2-staging")]
pub(crate) fn replacement_session_error(
    error: crate::yrs_engine::RootReplacementError,
    request_id: u64,
) -> SessionError {
    let mut error = match error {
        crate::yrs_engine::RootReplacementError::Admission(admission) => {
            SessionError::from(admission)
        }
        crate::yrs_engine::RootReplacementError::Transaction(transaction) => {
            // Frozen Task 1 mapping: the engine emits
            // OPERATION_RESOURCE_EXHAUSTED only for allocation/reservation
            // failures (Task 7: outbox reservation), which preserve their
            // code; deterministic ceilings keep their existing stable codes.
            let failure_class = if transaction.code == "OPERATION_RESOURCE_EXHAUSTED" {
                OperationFailureClass::AllocationOrReservation
            } else {
                OperationFailureClass::ExistingStableCode
            };
            SessionError::from_operation(transaction, failure_class)
        }
    };
    error.request_id.get_or_insert(request_id);
    error
}

fn engine_not_ready() -> SessionError {
    SessionError::new(
        ErrorDomain::Operation,
        "ENGINE_NOT_READY",
        "document engine is awaiting remote initialization",
    )
}

/// Runtime-shaped operations on sessions without an attached collaboration
/// runtime refuse with the established configuration error.
#[cfg(feature = "ffi-v2-staging")]
pub(crate) fn no_attached_runtime() -> SessionError {
    SessionError::new(
        ErrorDomain::Boundary,
        "CONFIG_INVALID",
        "session has no attached collaboration runtime",
    )
}
