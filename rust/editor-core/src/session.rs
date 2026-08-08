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

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CollaborationLimitOverrides {
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_frames_per_message: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_frame_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_aggregate_response_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_awareness_peers: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_awareness_peer_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_awareness_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_outbox_messages: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_outbox_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_dependency_update_bytes: Option<usize>,
    #[serde(
        default,
        deserialize_with = "crate::boundary::deserialize_non_null_option"
    )]
    max_pending_dependency_update_work: Option<usize>,
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
    pub(crate) fn resolve(overrides: CollaborationLimitOverrides) -> Result<Self, SessionError> {
        let defaults = Self::default();
        let limits = Self {
            max_frames_per_message: overrides
                .max_frames_per_message
                .unwrap_or(defaults.max_frames_per_message),
            max_frame_bytes: overrides
                .max_frame_bytes
                .unwrap_or(defaults.max_frame_bytes),
            max_aggregate_response_bytes: overrides
                .max_aggregate_response_bytes
                .unwrap_or(defaults.max_aggregate_response_bytes),
            max_awareness_peers: overrides
                .max_awareness_peers
                .unwrap_or(defaults.max_awareness_peers),
            max_awareness_peer_bytes: overrides
                .max_awareness_peer_bytes
                .unwrap_or(defaults.max_awareness_peer_bytes),
            max_awareness_bytes: overrides
                .max_awareness_bytes
                .unwrap_or(defaults.max_awareness_bytes),
            max_pending_outbox_messages: overrides
                .max_pending_outbox_messages
                .unwrap_or(defaults.max_pending_outbox_messages),
            max_pending_outbox_bytes: overrides
                .max_pending_outbox_bytes
                .unwrap_or(defaults.max_pending_outbox_bytes),
            max_pending_dependency_update_bytes: overrides
                .max_pending_dependency_update_bytes
                .unwrap_or(defaults.max_pending_dependency_update_bytes),
            max_pending_dependency_update_work: overrides
                .max_pending_dependency_update_work
                .unwrap_or(defaults.max_pending_dependency_update_work),
        };
        limits.validate()?;
        Ok(limits)
    }

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
    position_epochs: crate::position_epoch::PositionEpochStore,
    native_request_ledgers: std::collections::BTreeMap<u64, NativeRequestLedger>,
    native_render_cursors: std::collections::BTreeMap<u64, NativeRenderCursor>,
}

const NATIVE_REQUEST_CACHE_LIMIT: usize = 256;

#[derive(Debug, Default)]
struct NativeRequestLedger {
    high_water: Option<u64>,
    recent: std::collections::BTreeMap<u64, String>,
    order: std::collections::VecDeque<u64>,
}

#[derive(Clone)]
pub(crate) struct NativeRenderCursor {
    pub(crate) document_revision: u64,
    pub(crate) render_blocks: std::sync::Arc<crate::render::incremental::CachedRenderBlocks>,
}

pub(crate) struct SessionPolicy {
    read_only: bool,
    input_filter: Option<String>,
    /// Lazily compiled `input_filter` pattern (Task 12 tracked Minor: the
    /// legacy `InputFilter` compiles once at construction while the bridge
    /// recompiled per keystroke). The lazy cell preserves the exact
    /// request-time behavior: an invalid pattern surfaces the identical
    /// `CONFIG_INVALID` from the same call sites, replayed from the cache.
    input_filter_regex: std::sync::OnceLock<Result<regex::Regex, String>>,
    allow_base64_images: bool,
}

/// Lifecycle owned by the session until adds transaction translation.
pub(crate) struct NativeBridgeLifecycle {
    active: bool,
    lifecycle_test_calls: usize,
}

/// Lifecycle plus the optionally attached collaboration runtime.
pub(crate) struct CollaborationLifecycle {
    active: bool,
    limits: CollaborationLimits,
    /// The state machine is the single writer of the transport state.
    transport: crate::collaboration_runtime::state::TransportStateMachine,
    runtime: Option<crate::collaboration_runtime::CollaborationRuntime>,
    lifecycle_test_teardowns: usize,
}

/// Frozen Rust-to-native transport directive. Native executes only the
/// explicit open/deadline work it carries; all retry and awareness decisions
/// remain in Rust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CollaborationTransportDirective {
    pub(crate) transport_state: TransportState,
    pub(crate) generation_to_open: Option<crate::collaboration_runtime::state::TransportGeneration>,
    pub(crate) next_deadline_millis: Option<u64>,
    pub(crate) remote_commit_applied: bool,
    pub(crate) peers_changed: bool,
    pub(crate) renewed_local: bool,
    pub(crate) expired_peers: Vec<u64>,
}

/// A complete framed outbound payload retained by its outbox lease until an
/// exact ACK. Document Update-v1 bytes are framed only here, at the
/// session/protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboundFrameLease {
    pub(crate) lease_id: u64,
    pub(crate) frame: Vec<u8>,
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
                lifecycle_test_calls: 0,
            },
            document_state,
            collaboration: CollaborationLifecycle {
                active: true,
                limits: collaboration_limits,
                transport: crate::collaboration_runtime::state::TransportStateMachine::new(
                    transport_state,
                ),
                runtime: None,
                lifecycle_test_teardowns: 0,
            },
            position_epochs: crate::position_epoch::PositionEpochStore::new(
                crate::position_epoch::PositionEpochLimits::default(),
            ),
            native_request_ledgers: std::collections::BTreeMap::new(),
            native_render_cursors: std::collections::BTreeMap::new(),
        })
    }

    pub(crate) fn teardown(&mut self) {
        self.position_epochs.clear();
        self.native_request_ledgers.clear();
        self.native_render_cursors.clear();
        self.native_bridge.teardown();
        self.collaboration.teardown();
    }

    pub(crate) fn transport_state(&self) -> TransportState {
        self.collaboration.transport.state()
    }

    pub(crate) fn render_state(&self) -> EngineRenderState {
        self.engine.render_state()
    }

    pub(crate) fn pin_position_epoch(
        &mut self,
        owner_id: u64,
        document_revision: u64,
    ) -> Result<u64, SessionError> {
        if self.engine.revision() != document_revision {
            return Err(SessionError::new(
                ErrorDomain::Operation,
                "REVISION_MISMATCH",
                "position epoch revision does not match the current document",
            ));
        }
        let count = usize::try_from(
            self.engine
                .position_map()
                .ok_or_else(engine_not_ready)?
                .total_scalars(),
        )
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            SessionError::new(
                ErrorDomain::Boundary,
                "POSITION_EPOCH_LIMIT_EXCEEDED",
                "position epoch boundary count overflowed",
            )
        })?;
        self.position_epochs.admit_boundary_count(count)?;
        let boundaries = self
            .engine
            .build_position_epoch_boundaries()
            .ok_or_else(|| {
                SessionError::new(
                    ErrorDomain::Operation,
                    "ENGINE_INVARIANT_FAILED",
                    "authoritative position epoch could not be built",
                )
            })?;
        let epoch = self.position_epochs.install(
            owner_id,
            self.engine.client_id(),
            document_revision,
            boundaries,
        )?;
        let render_blocks = self
            .engine
            .cached_render_blocks()
            .ok_or_else(engine_not_ready)?;
        self.retain_native_render_cursor(owner_id, document_revision, render_blocks);
        Ok(epoch)
    }

    pub(crate) fn resolve_epoch_range(
        &self,
        owner_id: u64,
        epoch_id: u64,
        anchor: u32,
        head: u32,
    ) -> Result<crate::position_epoch::ResolvedEpochRange, SessionError> {
        let affinity = if anchor == head {
            crate::yrs_engine::Affinity::After
        } else {
            crate::yrs_engine::Affinity::Before
        };
        let (anchor_boundary, _) =
            self.position_epochs
                .boundary(owner_id, epoch_id, self.engine.client_id(), anchor)?;
        let (head_boundary, _) =
            self.position_epochs
                .boundary(owner_id, epoch_id, self.engine.client_id(), head)?;
        let (resolved_anchor, anchor_fallback) = self
            .engine
            .resolve_position_epoch_boundary(anchor_boundary, affinity, anchor)
            .ok_or_else(engine_not_ready)?;
        let (resolved_head, head_fallback) = self
            .engine
            .resolve_position_epoch_boundary(head_boundary, affinity, head)
            .ok_or_else(engine_not_ready)?;
        Ok(crate::position_epoch::ResolvedEpochRange {
            anchor: resolved_anchor,
            head: resolved_head,
            fallback: anchor_fallback || head_fallback,
        })
    }

    pub(crate) fn release_position_epoch_owner(&mut self, owner_id: u64) {
        self.position_epochs.release_owner(owner_id);
        self.native_render_cursors.remove(&owner_id);
    }

    pub(crate) fn native_render_cursor(&self, owner_id: u64) -> Option<NativeRenderCursor> {
        self.native_render_cursors.get(&owner_id).cloned()
    }

    pub(crate) fn retain_native_render_cursor(
        &mut self,
        owner_id: u64,
        document_revision: u64,
        render_blocks: std::sync::Arc<crate::render::incremental::CachedRenderBlocks>,
    ) {
        self.native_render_cursors.insert(
            owner_id,
            NativeRenderCursor {
                document_revision,
                render_blocks,
            },
        );
    }

    pub(crate) fn native_request_outcome(
        &self,
        owner_id: u64,
        request_id: u64,
    ) -> Result<Option<&str>, SessionError> {
        let Some(ledger) = self.native_request_ledgers.get(&owner_id) else {
            return Ok(None);
        };
        if let Some(outcome) = ledger.recent.get(&request_id) {
            return Ok(Some(outcome));
        }
        if ledger
            .high_water
            .is_some_and(|high_water| request_id <= high_water)
        {
            let mut error = SessionError::new(
                ErrorDomain::Boundary,
                "EXPIRED_NATIVE_REQUEST",
                "native request is older than the retained idempotency window",
            );
            error.request_id = Some(request_id);
            return Err(error);
        }
        Ok(None)
    }

    pub(crate) fn retain_native_request_outcome(
        &mut self,
        owner_id: u64,
        request_id: u64,
        outcome: String,
    ) {
        let ledger = self.native_request_ledgers.entry(owner_id).or_default();
        ledger.high_water = Some(
            ledger
                .high_water
                .map_or(request_id, |value| value.max(request_id)),
        );
        ledger.recent.insert(request_id, outcome);
        ledger.order.push_back(request_id);
        while ledger.order.len() > NATIVE_REQUEST_CACHE_LIMIT {
            if let Some(expired) = ledger.order.pop_front() {
                ledger.recent.remove(&expired);
            }
        }
    }

    pub(crate) fn release_native_binding(&mut self, owner_id: u64) {
        self.position_epochs.release_owner(owner_id);
        self.native_request_ledgers.remove(&owner_id);
        self.native_render_cursors.remove(&owner_id);
    }

    pub(crate) fn get_json(&self) -> Result<serde_json::Value, SessionError> {
        self.engine.document_json().ok_or_else(engine_not_ready)
    }

    pub(crate) fn get_json_string(&self) -> Result<String, SessionError> {
        self.engine
            .document_json_string()
            .ok_or_else(engine_not_ready)
    }

    pub(crate) fn get_html(&self) -> Result<String, SessionError> {
        self.engine.document_html().ok_or_else(engine_not_ready)
    }

    /// Same-store whole-document replacement from ProseMirror JSON, behind
    /// the session policy gate. An attached collaboration runtime captures
    /// the replacement's outbound update through its bounded outbox.
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

    /// Session-level snapshot export: read-only and allowed in every
    /// transport state, including connected ones (design "Snapshot and
    /// Persistence Flow": "Export is read-only and available while
    /// connected"). The engine owns manifest construction and its
    /// scope/readiness refusals; the session stamps the request id.
    pub(crate) fn export_snapshot(
        &self,
        request_id: u64,
    ) -> Result<DocumentSnapshot, SessionError> {
        self.engine
            .export_snapshot()
            .map_err(|error| snapshot_session_error(error, request_id))
    }

    /// Session-level snapshot restore behind the lifecycle policy gate
    /// (design "Whole-Document Replacement Policy" / "Snapshot and
    /// Persistence Flow"):
    ///
    /// 1. Transport gate: restore is `Detached`/`Disconnected`-only.
    ///    `Connecting`/`Handshaking`/`Synchronized` reject the frozen
    ///    snapshot-domain `SNAPSHOT_RESTORE_CONNECTED`. `Incompatible`
    ///    rejects with the same code by decision: it is not a live
    ///    transport, but it changes only through an explicit
    ///    detach/reattach (design line 124) and restore must not smuggle
    ///    a transition out of it; the frozen error contract adds exactly
    ///    one restore-gate code, which covers every non-quiescent
    ///    transport. `Destroying`/`Destroyed` are unreachable through
    ///    `with_alive`.
    /// 2. Outbox gate: pending local *document* updates reject
    ///    `SNAPSHOT_OUTBOX_NOT_EMPTY`. Pending protocol/awareness replies
    ///    never block — they are transport-scoped and cleared on success.
    /// 3. Engine restore: every manifest field validates before any decode
    ///    or mutation, the candidate installs atomically under a fresh
    ///    client identity (existing engine contract).
    ///
    /// On success the session clears the prior-store residue:
    /// `AwaitRemote` promotes to `RoomReady`, the transport settles to
    /// `Disconnected` (the design's restore rows end there; stale
    /// generations stay refused and issuance remains monotonic), and the
    /// runtime drops pending protocol replies, dependency-quarantine work
    /// accounting, and peer bookkeeping. Desired local awareness is
    /// retained — the engine's store-swap rebind re-published it under the
    /// fresh identity — and cursor projections recompute against the
    /// restored store on every read. Every failure above leaves session,
    /// engine, outbox, and runtime untouched.
    pub(crate) fn restore_snapshot(
        &mut self,
        request_id: u64,
        snapshot: &DocumentSnapshot,
    ) -> Result<crate::yrs_engine::EngineCommit, SessionError> {
        self.admit_snapshot_restore(request_id)?;
        let commit = self
            .engine
            .restore_snapshot(snapshot)
            .map_err(|error| snapshot_session_error(error, request_id))?;
        self.position_epochs.clear();
        self.native_request_ledgers.clear();
        self.native_render_cursors.clear();
        if self.document_state == DocumentState::AwaitRemote {
            self.document_state = DocumentState::RoomReady;
        }
        self.collaboration.transport.settle_for_restore();
        if let Some(runtime) = self.collaboration.runtime.as_mut() {
            runtime.reset_for_restore();
        }
        Ok(commit)
    }

    /// Document-state x transport-state policy matrix for snapshot restore,
    /// checked before any engine work. Any document state may restore
    /// (`AwaitRemote` promotes on success; `LocalReady` has no scope and
    /// fails inside the engine), so the gate is transport-first, then the
    /// document-outbox check via
    /// [`crate::collaboration_runtime::CollaborationOutbox::has_pending_document_updates`]
    /// without peeking at queue internals.
    fn admit_snapshot_restore(&self, request_id: u64) -> Result<(), SessionError> {
        match self.transport_state() {
            TransportState::Detached | TransportState::Disconnected => {}
            TransportState::Connecting
            | TransportState::Handshaking
            | TransportState::Synchronized
            | TransportState::Incompatible => {
                let mut error = SessionError::new(
                    ErrorDomain::Snapshot,
                    "SNAPSHOT_RESTORE_CONNECTED",
                    format!(
                        "snapshot restore is rejected while the collaboration \
                         transport is {}",
                        self.transport_state().as_str()
                    ),
                );
                error.request_id = Some(request_id);
                return Err(error);
            }
            TransportState::Destroying | TransportState::Destroyed => unreachable!(
                "with_alive rejects destroying/destroyed sessions before policy evaluation"
            ),
        }
        if let Some(outbox) = self.collaboration_outbox() {
            if outbox.has_pending_document_updates() {
                let mut error = SessionError::new(
                    ErrorDomain::Snapshot,
                    "SNAPSHOT_OUTBOX_NOT_EMPTY",
                    "snapshot restore is rejected while unsent local document \
                     updates are pending in the collaboration outbox",
                );
                error.request_id = Some(request_id);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Attach the collaboration runtime (Task 7: the bounded document
    /// outbox) sized from the session's validated collaboration limits.
    /// Idempotent: an already attached runtime and its pending outbox
    /// entries are preserved.
    pub(crate) fn attach_collaboration_runtime(&mut self) {
        if self.collaboration.runtime.is_none() {
            self.collaboration.runtime = Some(
                crate::collaboration_runtime::CollaborationRuntime::new(&self.collaboration.limits),
            );
        }
    }

    /// The attached runtime's outbox, if any. Detached/local-only sessions
    /// return `None`: no outbox exists for them by construction.
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
    fn room_bound(&self) -> bool {
        self.document_state != DocumentState::LocalReady
    }

    /// Rust's unified scheduler. It advances awareness work, issues a due
    /// initial/retry generation, and returns the sole native directive.
    pub(crate) fn collaboration_drive(
        &mut self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.finish_transport_directive(request_id, now_millis, false, false)
    }

    /// Current native socket-open callback. Sync Step 1 is framed and
    /// reserved before the state transitions, then enters the same
    /// protocol-priority lease queue as every other outbound protocol frame.
    pub(crate) fn collaboration_socket_open(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.collaboration
            .transport
            .admit_socket_open(request_id, generation)?;
        self.check_collaboration_now(request_id, now_millis)?;

        let step1 =
            crate::collaboration_runtime::protocol::sync_step1_message(&self.engine, request_id)?;
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        let reservation = runtime
            .outbox_mut()
            .reserve_protocol_replies(1, step1.len())
            .map_err(|error| {
                crate::collaboration_runtime::protocol::protocol_reply_reservation_error(
                    request_id, error,
                )
            })?;
        transport.socket_opened(request_id, generation)?;
        runtime
            .outbox_mut()
            .install_protocol_replies(reservation, request_id, vec![step1]);

        self.finish_transport_directive(request_id, now_millis, false, false)
    }

    /// Current native socket-close callback. The accepted closure retires
    /// its lease without consuming the retained frame and schedules only
    /// Rust's retry deadline.
    pub(crate) fn collaboration_socket_close(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        disposition: crate::collaboration_runtime::state::SocketCloseDisposition,
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.collaboration
            .transport
            .admit_socket_close(request_id, generation)?;
        self.check_collaboration_now(request_id, now_millis)?;
        self.collaboration.transport.socket_closed(
            request_id,
            generation,
            disposition,
            now_millis,
        )?;
        let peers_changed = self.retire_transport_scope();
        self.finish_transport_directive(request_id, now_millis, false, peers_changed)
    }

    /// Test-only local close helper for legacy lifecycle coverage.
    #[cfg(test)]
    pub(crate) fn disconnect(&mut self, request_id: u64) -> Result<(), SessionError> {
        self.collaboration.transport.disconnect(request_id)?;
        self.retire_transport_scope();
        Ok(())
    }

    /// Tear down transport state only (-> `Detached`); the runtime, its
    /// pending outbox entries, and the document are untouched by design.
    /// Remote awareness peers are transport-scoped and clear here too.
    pub(crate) fn detach(&mut self, request_id: u64) -> Result<(), SessionError> {
        self.collaboration.transport.detach(request_id)?;
        self.retire_transport_scope();
        Ok(())
    }

    /// Explicit reattach half of the `Incompatible` escape hatch:
    /// `Detached` -> `Disconnected` on a room-bound session. Peer clearing
    /// is repeated defensively — reattach begins a fresh transport scope.
    pub(crate) fn reattach(&mut self, request_id: u64) -> Result<(), SessionError> {
        let room_bound = self.room_bound();
        self.collaboration
            .transport
            .reattach(request_id, room_bound)?;
        self.retire_transport_scope();
        Ok(())
    }

    /// Task 10 lifecycle rule shared by every accepted generation-closing
    /// transition: remote awareness peers are transport-scoped, desired
    /// local awareness is retained. Sessions without an attached runtime
    /// own no peers by construction.
    fn retire_transport_scope(&mut self) -> bool {
        if let Some(runtime) = self.collaboration.runtime.as_mut() {
            runtime.outbox_mut().release_lease();
            return runtime.clear_transport_peers(&mut self.engine);
        }
        false
    }

    /// Crate-private Task 9 seam: an accepted current-generation Sync
    /// Step 2 turns `Handshaking` into `Synchronized`. Transport-only —
    /// document-state promotion is Task 9's Step 2 handling.
    #[cfg(test)]
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
    pub(crate) fn collaboration_receive(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.collaboration
            .transport
            .admit_receive(request_id, generation)?;
        self.check_collaboration_now(request_id, now_millis)?;
        let outcome = self.receive_message_at(request_id, generation, bytes, now_millis)?;
        self.finish_transport_directive(
            request_id,
            now_millis,
            outcome.remote_commit_applied,
            outcome.peers_changed,
        )
    }

    /// Test-only detailed receive seam retained for protocol matrix tests.
    #[cfg(test)]
    pub(crate) fn receive_message(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
    ) -> Result<crate::collaboration_runtime::protocol::ReceiveOutcome, SessionError> {
        self.receive_message_at(request_id, generation, bytes, 0)
    }

    fn receive_message_at(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
        now_millis: u64,
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
                now_millis,
            },
            bytes,
        )
    }

    /// Retain at most one current-generation frame until exact ACK/NACK.
    pub(crate) fn lease_outbound(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
    ) -> Result<Option<OutboundFrameLease>, SessionError> {
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        transport.admit_outbound_lease(request_id, generation)?;
        let lease = runtime.outbox_mut().lease_next().map_err(|error| {
            outbound_lease_session_error(error, request_id, "leaseOutbound", None)
        })?;
        Ok(lease.map(|lease| {
            let frame = match lease.payload {
                crate::collaboration_runtime::outbox::OutboundLeasePayload::ProtocolReply(
                    frame,
                ) => frame,
                crate::collaboration_runtime::outbox::OutboundLeasePayload::DocumentUpdate(
                    update_v1,
                ) => crate::collaboration_runtime::protocol::frame_sync_update_message(&update_v1),
            };
            OutboundFrameLease {
                lease_id: lease.lease_id.value(),
                frame,
            }
        }))
    }

    /// Consume exactly the retained current-generation lease.
    pub(crate) fn ack_outbound(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        lease_id: u64,
    ) -> Result<(), SessionError> {
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        transport.admit_outbound_lease(request_id, generation)?;
        runtime
            .outbox_mut()
            .ack_lease(crate::collaboration_runtime::outbox::OutboundLeaseId::from_value(lease_id))
            .map_err(|error| {
                outbound_lease_session_error(error, request_id, "ackOutbound", Some(lease_id))
            })
    }

    /// Release exactly the retained current-generation lease without
    /// consuming the queue front.
    pub(crate) fn nack_outbound(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        lease_id: u64,
    ) -> Result<(), SessionError> {
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        transport.admit_outbound_lease(request_id, generation)?;
        runtime
            .outbox_mut()
            .nack_lease(crate::collaboration_runtime::outbox::OutboundLeaseId::from_value(lease_id))
            .map_err(|error| {
                outbound_lease_session_error(error, request_id, "nackOutbound", Some(lease_id))
            })
    }

    fn check_collaboration_now(
        &self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<(), SessionError> {
        match self.collaboration.runtime.as_ref() {
            Some(runtime) => runtime.check_now_millis(request_id, now_millis),
            None => Ok(()),
        }
    }

    fn drive_awareness(
        &mut self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<crate::collaboration_runtime::awareness::TickOutcome, SessionError> {
        let CollaborationLifecycle {
            transport,
            runtime,
            limits,
            ..
        } = &mut self.collaboration;
        let Some(runtime) = runtime.as_mut() else {
            return Ok(crate::collaboration_runtime::awareness::TickOutcome {
                renewed_local: false,
                outbound_changed: false,
                expired_peers: Vec::new(),
                peers_changed: false,
                next_deadline_millis: None,
            });
        };
        runtime.tick(
            request_id,
            now_millis,
            crate::collaboration_runtime::awareness::AwarenessContext {
                engine: &mut self.engine,
                transport_state: transport.state(),
                limits,
            },
        )
    }

    fn finish_transport_directive(
        &mut self,
        request_id: u64,
        now_millis: u64,
        remote_commit_applied: bool,
        peers_changed: bool,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        let awareness = self.drive_awareness(request_id, now_millis)?;
        let room_bound = self.room_bound();
        let generation_to_open = self
            .collaboration
            .transport
            .drive(request_id, room_bound, now_millis)?;
        let next_deadline_millis = minimum_deadline(
            self.collaboration.transport.next_retry_deadline_millis(),
            awareness.next_deadline_millis,
        );
        Ok(CollaborationTransportDirective {
            transport_state: self.collaboration.transport.state(),
            generation_to_open,
            next_deadline_millis,
            remote_commit_applied,
            peers_changed: peers_changed || awareness.peers_changed,
            renewed_local: awareness.renewed_local,
            expired_peers: awareness.expired_peers,
        })
    }

    /// Test-only raw awareness fixture seam. Production callers must use the
    /// typed intent path below.
    #[cfg(test)]
    pub(crate) fn set_desired_awareness_for_test(
        &mut self,
        request_id: u64,
        state_json: &str,
    ) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_desired_awareness_for_test(request_id, state_json, context)
    }

    /// Production local-awareness intent: Rust validates the closed caller
    /// shape and installs an engine-owned sticky cursor before publication.
    pub(crate) fn set_awareness_intent(
        &mut self,
        request_id: u64,
        intent_json: &str,
    ) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_awareness_intent(request_id, intent_json, context)
    }

    pub(crate) fn set_awareness_selection(
        &mut self,
        request_id: u64,
        selection_json: &str,
    ) -> Result<crate::collaboration_runtime::awareness::AwarenessSelectionOutcome, SessionError>
    {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_awareness_selection(request_id, selection_json, context)
    }

    /// Withdraw the desired local awareness state.
    pub(crate) fn clear_desired_awareness(&mut self, request_id: u64) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.clear_desired_awareness(request_id, context)
    }

    /// The retained desired local awareness state, if any.
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

    /// Field-disjoint split borrow for awareness operations, mirroring the
    /// Task 9 receive split: the runtime plus the engine/limits/transport
    /// context it composes.
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
    pub(crate) fn set_collaboration_limit_for_test(&mut self, field: &str, value: usize) {
        self.collaboration.limits.set_for_test(field, value);
    }

    #[cfg(test)]
    pub(crate) fn collaboration_limits(&self) -> &CollaborationLimits {
        &self.collaboration.limits
    }

    /// Test-only transport-state injection, routed through the state
    /// machine so its live-attempt invariant holds (forced states carry no
    /// live attempt). It remains alongside the real transitions for exactly
    /// one reason: policy-matrix cells that are unreachable by construction
    /// — `LocalReady` sessions are permanently `Detached`, yet the Task 5
    /// replacement gate deliberately covers them for every transport state.
    /// Room-bound tests must use the real transitions instead.
    pub(crate) fn set_transport_state_for_test(&mut self, state: TransportState) {
        self.collaboration.transport.set_state_for_test(state);
    }

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
                input_filter_regex: std::sync::OnceLock::new(),
                allow_base64_images: false,
            },
            DocumentState::LocalReady,
            collaboration_limits,
        )
    }

    pub(crate) fn record_lifecycle_test_call(&mut self) -> usize {
        self.native_bridge.lifecycle_test_calls += 1;
        self.native_bridge.lifecycle_test_calls
    }

    pub(crate) fn lifecycle_test_call_count(&self) -> usize {
        self.native_bridge.lifecycle_test_calls
    }

    pub(crate) fn lifecycle_test_teardown_count(&self) -> usize {
        self.collaboration.lifecycle_test_teardowns
    }
}

impl SessionPolicy {
    pub(crate) fn from_config(config: &EditorSessionConfig) -> Self {
        Self {
            read_only: config.read_only,
            input_filter: config.input_filter.clone(),
            input_filter_regex: std::sync::OnceLock::new(),
            allow_base64_images: config.allow_base64_images,
        }
    }

    /// Read-only policy: input and command mutations are rejected while
    /// local-API requests pass, mirroring the legacy `Source::Api` gate.
    pub(crate) fn read_only(&self) -> bool {
        self.read_only
    }

    /// The input filter pattern compiled at most once per policy (Task 12
    /// tracked Minor): the first request compiles and caches the `Regex`;
    /// an invalid pattern caches the compile error message and replays it
    /// verbatim on every request. Semantics are exactly the per-character
    /// `is_match` filter the legacy `InputFilter` applies.
    pub(crate) fn input_filter_regex(&self) -> Option<Result<&regex::Regex, String>> {
        self.input_filter.as_deref().map(|pattern| {
            self.input_filter_regex
                .get_or_init(|| regex::Regex::new(pattern).map_err(|error| error.to_string()))
                .as_ref()
                .map_err(Clone::clone)
        })
    }
}

/// Field-disjoint split of the session into its engine and the optionally
/// attached collaboration outbox, so pre-write reservation can run while the
/// engine is mutably borrowed.
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

fn minimum_deadline(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (deadline @ Some(_), None) | (None, deadline @ Some(_)) => deadline,
        (None, None) => None,
    }
}

fn outbound_lease_session_error(
    error: crate::collaboration_runtime::outbox::OutboundLeaseError,
    request_id: u64,
    action: &'static str,
    lease_id: Option<u64>,
) -> SessionError {
    use crate::collaboration_runtime::outbox::OutboundLeaseError;
    use crate::collaboration_runtime::state::{
        TRANSPORT_GENERATION_EXHAUSTED, TRANSPORT_INVALID_TRANSITION,
    };

    let (code, message) = match error {
        OutboundLeaseError::LeaseIdExhausted => (
            TRANSPORT_GENERATION_EXHAUSTED,
            "outbound lease identifier space is exhausted",
        ),
        OutboundLeaseError::NoActiveLease => (
            TRANSPORT_INVALID_TRANSITION,
            "no outbound lease is active for this disposition",
        ),
        OutboundLeaseError::LeaseMismatch => (
            TRANSPORT_INVALID_TRANSITION,
            "outbound lease identifier does not match the retained queue front",
        ),
    };
    let mut session_error = SessionError::new(ErrorDomain::Transport, code, message);
    session_error.request_id = Some(request_id);
    session_error.details = Some(serde_json::json!({
        "action": action,
        "leaseId": lease_id.map(|value| value.to_string()),
    }));
    session_error
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
            {
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.outbox_mut().release_lease();
                }
                self.transport.teardown_destroyed();
                self.runtime = None;
                self.lifecycle_test_teardowns += 1;
            }
        }
    }
}

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

/// Snapshot export/restore errors stay in the snapshot domain with the
/// engine's exact codes; the session stamps the request id the engine
/// cannot know.
fn snapshot_session_error(error: YrsEngineError, request_id: u64) -> SessionError {
    let mut error = SessionError::snapshot(error);
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
pub(crate) fn no_attached_runtime() -> SessionError {
    SessionError::new(
        ErrorDomain::Boundary,
        "CONFIG_INVALID",
        "session has no attached collaboration runtime",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_with_filter(pattern: &str) -> SessionPolicy {
        SessionPolicy::from_config(&EditorSessionConfig {
            input_filter: Some(pattern.to_string()),
            ..EditorSessionConfig::local_for_test()
        })
    }

    #[test]
    fn input_filter_regex_compiles_once_and_replays_compile_errors() {
        // Compile-once: repeated borrows return the SAME cached Regex
        // allocation, preserving exact per-character `is_match` semantics.
        let policy = policy_with_filter("^[0-9]$");
        let first = policy.input_filter_regex().unwrap().unwrap();
        assert!(first.is_match("7"));
        assert!(!first.is_match("a"));
        let second = policy.input_filter_regex().unwrap().unwrap();
        assert!(
            std::ptr::eq(first, second),
            "the pattern must compile once and be served from the cache",
        );

        // An invalid pattern caches the compile error and replays the
        // identical message (request-time CONFIG_INVALID semantics kept).
        let policy = policy_with_filter("[unclosed");
        let first = policy.input_filter_regex().unwrap().unwrap_err();
        let second = policy.input_filter_regex().unwrap().unwrap_err();
        assert_eq!(first, second, "identical replay of the compile error");

        // No pattern: nothing compiles, nothing is cached.
        let policy = SessionPolicy::from_config(&EditorSessionConfig::local_for_test());
        assert!(policy.input_filter_regex().is_none());
        assert!(policy.input_filter_regex.get().is_none());
    }
}
