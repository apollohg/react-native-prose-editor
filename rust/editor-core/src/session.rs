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

include!("session/config.rs");

include!("session/errors.rs");

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

include!("session/native.rs");
include!("session/content.rs");
include!("session/collaboration.rs");

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
#[path = "session/tests.rs"]
mod tests;
