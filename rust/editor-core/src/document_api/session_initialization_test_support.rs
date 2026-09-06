use super::*;
use crate::session::{
    CollaborationLimits, EditorInitialization, InitialContent, OperationFailureClass,
    TransportState as InternalTransportState,
};
use crate::yrs_engine::{DocumentSnapshot, EngineRenderState};

#[derive(Debug, Clone, PartialEq)]
pub struct TestError {
    pub domain: &'static str,
    pub code: String,
    pub request_id: Option<u64>,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentState {
    LocalReady,
    AwaitRemote,
    RoomReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    Detached,
    Disconnected,
    Connecting,
    Handshaking,
    Synchronized,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderState {
    Loading,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditAdmission {
    pub can_undo: bool,
}

impl From<SessionError> for TestError {
    fn from(error: SessionError) -> Self {
        Self {
            domain: error.domain.as_str(),
            code: error.code,
            request_id: error.request_id,
            details: error.details,
        }
    }
}

fn local_config(initial_content: InitialContent) -> EditorSessionConfig {
    EditorSessionConfig {
        schema_json: None,
        fragment_name: "prosemirror".into(),
        initialization: EditorInitialization::Local { initial_content },
        resource_limits: crate::boundary::ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        collaboration_limits: CollaborationLimits::default(),
        max_length: None,
        read_only: false,
        input_filter: None,
        allow_base64_images: false,
    }
}

pub fn create_local_empty() -> Result<u64, TestError> {
    DocumentApiFacade::create(local_config(InitialContent::Empty)).map_err(TestError::from)
}

pub fn create_local_json(json: &str) -> Result<u64, TestError> {
    DocumentApiFacade::create(local_config(InitialContent::Json(json.into())))
        .map_err(TestError::from)
}

pub fn create_local_html(html: &str) -> Result<u64, TestError> {
    DocumentApiFacade::create(local_config(InitialContent::Html(html.into())))
        .map_err(TestError::from)
}

pub fn create_local_json_with_options(
    json: &str,
    schema_json: Option<&str>,
    max_length: Option<u32>,
) -> Result<u64, TestError> {
    let mut config = local_config(InitialContent::Json(json.into()));
    config.schema_json = schema_json.map(str::to_owned);
    config.max_length = max_length;
    DocumentApiFacade::create(config).map_err(TestError::from)
}

pub fn create_room_from_json(config: &str) -> Result<u64, TestError> {
    let config = EditorSessionConfig::room_from_json(config).map_err(TestError::from)?;
    DocumentApiFacade::create(config).map_err(TestError::from)
}

pub fn destroy_session(id: u64) {
    registry::destroy_session(id);
}

pub fn registry_count() -> usize {
    registry::session_registry_count()
}

pub fn get_json(id: u64) -> Result<serde_json::Value, TestError> {
    DocumentApiFacade::get_json(id).map_err(TestError::from)
}

pub fn get_html(id: u64) -> Result<String, TestError> {
    DocumentApiFacade::get_html(id).map_err(TestError::from)
}

pub fn get_content_snapshot(id: u64) -> Result<serde_json::Value, TestError> {
    let snapshot = DocumentApiFacade::get_content_snapshot(id).map_err(TestError::from)?;
    serde_json::to_value(snapshot).map_err(|error| TestError {
        domain: "boundary",
        code: format!("SERIALIZATION_FAILED: {error}"),
        request_id: None,
        details: None,
    })
}

fn map_document_state(state: crate::session::DocumentState) -> DocumentState {
    match state {
        crate::session::DocumentState::LocalReady => DocumentState::LocalReady,
        crate::session::DocumentState::AwaitRemote => DocumentState::AwaitRemote,
        crate::session::DocumentState::RoomReady => DocumentState::RoomReady,
    }
}

fn map_transport_state(state: InternalTransportState) -> TransportState {
    match state {
        InternalTransportState::Detached => TransportState::Detached,
        InternalTransportState::Disconnected => TransportState::Disconnected,
        InternalTransportState::Connecting => TransportState::Connecting,
        InternalTransportState::Handshaking => TransportState::Handshaking,
        InternalTransportState::Synchronized => TransportState::Synchronized,
        InternalTransportState::Incompatible => TransportState::Incompatible,
        state @ (InternalTransportState::Destroying | InternalTransportState::Destroyed) => {
            panic!("live sessions never expose transport state {state:?}")
        }
    }
}

pub fn document_state(id: u64) -> Result<DocumentState, TestError> {
    with_live(id, |session| Ok(map_document_state(session.document_state)))
}

pub fn transport_state(id: u64) -> Result<TransportState, TestError> {
    with_live(id, |session| {
        Ok(map_transport_state(session.transport_state()))
    })
}

/// Test-only transport injection, reserved for policy-matrix cells that
/// are unreachable through real transitions (`LocalReady` sessions are
/// permanently `Detached` yet the Task 5 replacement gate deliberately
/// covers every transport state). Room-bound tests must drive
/// [`collaboration_drive`] and its socket directives instead.
pub fn set_transport_state_for_test(id: u64, state: TransportState) -> Result<(), TestError> {
    let internal = match state {
        TransportState::Detached => InternalTransportState::Detached,
        TransportState::Disconnected => InternalTransportState::Disconnected,
        TransportState::Connecting => InternalTransportState::Connecting,
        TransportState::Handshaking => InternalTransportState::Handshaking,
        TransportState::Synchronized => InternalTransportState::Synchronized,
        TransportState::Incompatible => InternalTransportState::Incompatible,
    };
    with_live(id, |session| {
        session.set_transport_state_for_test(internal);
        Ok(())
    })
}

/// Public mirror of the Rust-owned socket-close disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDisposition {
    Retryable,
    Incompatible,
}

/// Public mirror of Rust's complete native-transport directive. Every
/// u64-shaped member stays typed as u64 in test support; the FFI tests
/// pin its canonical decimal-string representation separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollaborationDirective {
    pub transport_state: TransportState,
    pub generation_to_open: Option<u64>,
    pub next_deadline_millis: Option<u64>,
    pub remote_commit_applied: bool,
    pub peers_changed: bool,
    pub renewed_local: bool,
    pub expired_peers: Vec<u64>,
}

/// One retained outbound lease exposed to test scenarios. `frame` is
/// always a complete y-protocols message, including document updates
/// framed at the session/protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundLease {
    pub lease_id: u64,
    pub frame: Vec<u8>,
}

fn map_close_disposition(
    disposition: CloseDisposition,
) -> crate::collaboration_runtime::state::SocketCloseDisposition {
    match disposition {
        CloseDisposition::Retryable => {
            crate::collaboration_runtime::state::SocketCloseDisposition::Retryable
        }
        CloseDisposition::Incompatible => {
            crate::collaboration_runtime::state::SocketCloseDisposition::Incompatible
        }
    }
}

fn map_collaboration_directive(
    directive: crate::session::CollaborationTransportDirective,
) -> CollaborationDirective {
    CollaborationDirective {
        transport_state: map_transport_state(directive.transport_state),
        generation_to_open: directive
            .generation_to_open
            .map(crate::collaboration_runtime::state::TransportGeneration::value),
        next_deadline_millis: directive.next_deadline_millis,
        remote_commit_applied: directive.remote_commit_applied,
        peers_changed: directive.peers_changed,
        renewed_local: directive.renewed_local,
        expired_peers: directive.expired_peers,
    }
}

fn map_outbound_lease(lease: crate::session::OutboundFrameLease) -> OutboundLease {
    OutboundLease {
        lease_id: lease.lease_id,
        frame: lease.frame,
    }
}

/// The one Rust-owned scheduler entry point. It alone issues a due
/// generation and returns all retry/awareness deadlines.
pub fn collaboration_drive(
    id: u64,
    request_id: u64,
    now_millis: u64,
) -> Result<CollaborationDirective, TestError> {
    with_live(id, |session| {
        session
            .collaboration_drive(request_id, now_millis)
            .map(map_collaboration_directive)
    })
}

/// Reports a current socket open; Sync Step 1 is queued into the normal
/// protocol-priority lease path rather than returned directly.
pub fn collaboration_socket_open(
    id: u64,
    request_id: u64,
    generation: u64,
    now_millis: u64,
) -> Result<CollaborationDirective, TestError> {
    with_live(id, |session| {
        session
            .collaboration_socket_open(
                request_id,
                crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
                now_millis,
            )
            .map(map_collaboration_directive)
    })
}

/// Reports one current binary receive and returns the frozen directive.
pub fn collaboration_receive(
    id: u64,
    request_id: u64,
    generation: u64,
    bytes: &[u8],
    now_millis: u64,
) -> Result<CollaborationDirective, TestError> {
    with_live(id, |session| {
        session
            .collaboration_receive(
                request_id,
                crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
                bytes,
                now_millis,
            )
            .map(map_collaboration_directive)
    })
}

/// Reports a current socket close and returns the resulting retry or
/// parked directive.
pub fn collaboration_socket_close(
    id: u64,
    request_id: u64,
    generation: u64,
    disposition: CloseDisposition,
    now_millis: u64,
) -> Result<CollaborationDirective, TestError> {
    with_live(id, |session| {
        session
            .collaboration_socket_close(
                request_id,
                crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
                map_close_disposition(disposition),
                now_millis,
            )
            .map(map_collaboration_directive)
    })
}

/// Retains one current-generation outbox front for an explicit ACK/NACK
/// disposition. `None` is the only empty-queue representation.
pub fn lease_outbound(
    id: u64,
    request_id: u64,
    generation: u64,
) -> Result<Option<OutboundLease>, TestError> {
    with_live(id, |session| {
        session
            .lease_outbound(
                request_id,
                crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
            )
            .map(|lease| lease.map(map_outbound_lease))
    })
}

/// Confirms exactly the current retained lease.
pub fn ack_outbound(
    id: u64,
    request_id: u64,
    generation: u64,
    lease_id: u64,
) -> Result<(), TestError> {
    with_live(id, |session| {
        session.ack_outbound(
            request_id,
            crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
            lease_id,
        )
    })
}

/// Releases exactly the current retained lease without consuming it.
pub fn nack_outbound(
    id: u64,
    request_id: u64,
    generation: u64,
    lease_id: u64,
) -> Result<(), TestError> {
    with_live(id, |session| {
        session.nack_outbound(
            request_id,
            crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
            lease_id,
        )
    })
}

pub fn transport_disconnect(id: u64, request_id: u64) -> Result<(), TestError> {
    with_live(id, |session| session.disconnect(request_id))
}

pub fn transport_detach(id: u64, request_id: u64) -> Result<(), TestError> {
    with_live(id, |session| session.detach(request_id))
}

pub fn transport_reattach(id: u64, request_id: u64) -> Result<(), TestError> {
    with_live(id, |session| session.reattach(request_id))
}

/// Direct exposure of the crate-private Task 9 seam so the matrix can
/// cover `Synchronized` rows without pretending protocol work happened.
pub fn mark_synchronized_for_test(
    id: u64,
    request_id: u64,
    generation: u64,
) -> Result<(), TestError> {
    with_live(id, |session| {
        session.mark_synchronized(
            request_id,
            crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
        )
    })
}

/// Public mirror of one accepted `receive_message` outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceiveOutcome {
    pub frames_decoded: usize,
    pub replies_enqueued: usize,
    pub reply_bytes_enqueued: usize,
    pub remote_commit_applied: bool,
    pub document_promoted: bool,
    pub transport_state: TransportState,
    /// `None` while the generation stays open (`Continue`).
    pub close: Option<ReceiveClose>,
}

/// Public mirror of a generation-closing receive disposition.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceiveClose {
    pub disposition: CloseDisposition,
    pub error: TestError,
}

fn map_socket_close_disposition(
    disposition: crate::collaboration_runtime::state::SocketCloseDisposition,
) -> CloseDisposition {
    match disposition {
        crate::collaboration_runtime::state::SocketCloseDisposition::Retryable => {
            CloseDisposition::Retryable
        }
        crate::collaboration_runtime::state::SocketCloseDisposition::Incompatible => {
            CloseDisposition::Incompatible
        }
    }
}

fn map_receive_outcome(
    outcome: crate::collaboration_runtime::protocol::ReceiveOutcome,
) -> ReceiveOutcome {
    use crate::collaboration_runtime::protocol::ReceiveDisposition;
    ReceiveOutcome {
        frames_decoded: outcome.frames_decoded,
        replies_enqueued: outcome.replies_enqueued,
        reply_bytes_enqueued: outcome.reply_bytes_enqueued,
        remote_commit_applied: outcome.remote_commit_applied,
        document_promoted: outcome.document_promoted,
        transport_state: map_transport_state(outcome.transport_state),
        close: match outcome.disposition {
            ReceiveDisposition::Continue => None,
            ReceiveDisposition::CloseGeneration { close, error } => Some(ReceiveClose {
                disposition: map_socket_close_disposition(close),
                error: TestError::from(error),
            }),
        },
    }
}

/// Task 9 protocol entry point: one bounded inbound y-sync message for
/// the given raw generation value.
pub fn receive_message(
    id: u64,
    request_id: u64,
    generation: u64,
    bytes: &[u8],
) -> Result<ReceiveOutcome, TestError> {
    with_live(id, |session| {
        session.receive_message(
            request_id,
            crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
            bytes,
        )
    })
    .map(map_receive_outcome)
}

/// `(engine retained dependency bytes, runtime charged work units)`.
pub fn remote_dependency_accounting(id: u64) -> Result<(usize, u64), TestError> {
    with_live(id, |session| Ok(session.remote_dependency_accounting()))
}

/// Public mirror of one awareness peer projection.
#[derive(Debug, Clone, PartialEq)]
pub struct AwarenessPeerInfo {
    pub client_id: u64,
    pub clock: u32,
    pub is_local: bool,
    pub state: serde_json::Value,
    /// Resolved `(anchor, head)` document positions, `None` when the
    /// peer carries no resolvable sticky cursor.
    pub cursor: Option<(u32, u32)>,
}

/// Publish the desired local awareness state.
pub fn set_desired_awareness_for_test(
    id: u64,
    request_id: u64,
    state_json: &str,
) -> Result<(), TestError> {
    with_live(id, |session| {
        session.set_desired_awareness_for_test(request_id, state_json)
    })
}

/// Withdraw the desired local awareness state.
pub fn clear_desired_awareness(id: u64, request_id: u64) -> Result<(), TestError> {
    with_live(id, |session| session.clear_desired_awareness(request_id))
}

/// The retained desired local awareness state, if any.
pub fn desired_awareness(id: u64) -> Result<Option<serde_json::Value>, TestError> {
    with_live(id, |session| session.desired_awareness())
}

/// Task 10: awareness peer projections (tombstones excluded, cursors
/// resolved against the current document on every read).
pub fn awareness_peers(id: u64) -> Result<Vec<AwarenessPeerInfo>, TestError> {
    with_live(id, |session| {
        Ok(session
            .awareness_peers()?
            .into_iter()
            .map(|peer| AwarenessPeerInfo {
                client_id: peer.client_id,
                clock: peer.clock,
                is_local: peer.is_local,
                state: peer.state,
                cursor: peer.cursor.map(|cursor| (cursor.anchor, cursor.head)),
            })
            .collect())
    })
}

/// Test-only collaboration-limit override by wire field name, for
/// exact/one-over receive ceiling coverage.
pub fn set_collaboration_limit_for_test(
    id: u64,
    field: &str,
    value: usize,
) -> Result<(), TestError> {
    with_live(id, |session| {
        session.set_collaboration_limit_for_test(field, value);
        Ok(())
    })
}

/// Pre-acquired session handle for destroy-interleaving coverage,
/// mirroring the Task 3 lifecycle-test idiom: it keeps working after
/// the registry entry disappears, so it proves `with_alive` refuses
/// transport transitions during and after destroy.
pub struct TransportHandle {
    slot: std::sync::Arc<crate::registry::EditorSessionSlot>,
}

impl TransportHandle {
    fn with_alive_session<T>(
        &self,
        operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
    ) -> Result<T, TestError> {
        self.slot
            .with_alive(operation)
            .and_then(|value| value)
            .map_err(TestError::from)
    }

    pub fn collaboration_drive(
        &self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<CollaborationDirective, TestError> {
        self.with_alive_session(|session| {
            session
                .collaboration_drive(request_id, now_millis)
                .map(map_collaboration_directive)
        })
    }

    pub fn collaboration_socket_open(
        &self,
        request_id: u64,
        generation: u64,
        now_millis: u64,
    ) -> Result<CollaborationDirective, TestError> {
        self.with_alive_session(|session| {
            session
                .collaboration_socket_open(
                    request_id,
                    crate::collaboration_runtime::state::TransportGeneration::from_value(
                        generation,
                    ),
                    now_millis,
                )
                .map(map_collaboration_directive)
        })
    }

    pub fn collaboration_socket_close(
        &self,
        request_id: u64,
        generation: u64,
        disposition: CloseDisposition,
        now_millis: u64,
    ) -> Result<CollaborationDirective, TestError> {
        self.with_alive_session(|session| {
            session
                .collaboration_socket_close(
                    request_id,
                    crate::collaboration_runtime::state::TransportGeneration::from_value(
                        generation,
                    ),
                    map_close_disposition(disposition),
                    now_millis,
                )
                .map(map_collaboration_directive)
        })
    }

    pub fn disconnect(&self, request_id: u64) -> Result<(), TestError> {
        self.with_alive_session(|session| session.disconnect(request_id))
    }

    pub fn detach(&self, request_id: u64) -> Result<(), TestError> {
        self.with_alive_session(|session| session.detach(request_id))
    }

    pub fn reattach(&self, request_id: u64) -> Result<(), TestError> {
        self.with_alive_session(|session| session.reattach(request_id))
    }

    pub fn mark_synchronized(&self, request_id: u64, generation: u64) -> Result<(), TestError> {
        self.with_alive_session(|session| {
            session.mark_synchronized(
                request_id,
                crate::collaboration_runtime::state::TransportGeneration::from_value(generation),
            )
        })
    }

    /// Hold the session lock (alive) until released, so a concurrent
    /// destroy is pinned in its `Destroying` window.
    pub fn hold_session_lock(
        &self,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::mpsc::Receiver<()>,
    ) -> Result<(), TestError> {
        self.slot
            .with_alive(|_session| {
                entered
                    .send(())
                    .expect("transport lock-hold observer should be present");
                release
                    .recv()
                    .expect("transport lock-hold should be released");
            })
            .map_err(TestError::from)
    }
}

/// A pre-acquired handle to a live session; `None` once destroy has
/// removed the registry entry.
pub fn transport_handle(id: u64) -> Option<TransportHandle> {
    registry::get_session(id).map(|slot| TransportHandle { slot })
}

pub fn render_state(id: u64) -> Result<RenderState, TestError> {
    with_live(id, |session| {
        Ok(match session.render_state() {
            EngineRenderState::Loading => RenderState::Loading,
            EngineRenderState::Ready => RenderState::Ready,
        })
    })
}

pub fn has_document_state(id: u64) -> Result<bool, TestError> {
    with_live(id, |session| Ok(session.engine.has_document_state()))
}

pub fn can_undo(id: u64) -> Result<bool, TestError> {
    with_live(id, |session| Ok(session.engine.can_undo()))
}

pub fn request_edit(id: u64, request_id: u64, text: &str) -> Result<EditAdmission, TestError> {
    with_live(id, |session| {
        session
            .engine
            .apply_command(
                request_id,
                crate::yrs_engine::TypedCommand::InsertText { text: text.into() },
            )
            .map_err(|error| {
                SessionError::from_operation(error, OperationFailureClass::ExistingStableCode)
            })?;
        Ok(EditAdmission {
            can_undo: session.engine.can_undo(),
        })
    })
}

/// Commit surface of a whole-document replacement for staging tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplacementOutcome {
    pub changed: bool,
    pub document_revision: u64,
}

/// Complete session audit used to prove atomic replacement rejection.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionAudit {
    pub document_json: Option<serde_json::Value>,
    pub document_html: Option<String>,
    pub encoded_state: Option<Vec<u8>>,
    pub document_revision: u64,
    pub state_revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selection: Option<String>,
    pub stored_marks: Option<String>,
    pub document_state: DocumentState,
    pub transport_state: TransportState,
}

pub fn write_json(
    id: u64,
    request_id: u64,
    json: &str,
    history: ReplacementHistory,
) -> Result<ReplacementOutcome, TestError> {
    let commit =
        DocumentApiFacade::write_json(id, request_id, json, history).map_err(TestError::from)?;
    Ok(ReplacementOutcome {
        changed: commit.changed,
        document_revision: commit.document_revision,
    })
}

pub fn write_html(
    id: u64,
    request_id: u64,
    html: &str,
    history: ReplacementHistory,
) -> Result<ReplacementOutcome, TestError> {
    let commit =
        DocumentApiFacade::write_html(id, request_id, html, history).map_err(TestError::from)?;
    Ok(ReplacementOutcome {
        changed: commit.changed,
        document_revision: commit.document_revision,
    })
}

pub fn can_redo(id: u64) -> Result<bool, TestError> {
    with_live(id, |session| Ok(session.engine.can_redo()))
}

/// Commit surface of a session-level snapshot restore for staging tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotRestoreOutcome {
    pub changed: bool,
    pub document_revision: u64,
}

/// Task 11: session-level snapshot export (read-only, allowed while
/// connected).
pub fn export_snapshot(id: u64, request_id: u64) -> Result<DocumentSnapshot, TestError> {
    with_live(id, |session| session.export_snapshot(request_id))
}

/// Task 11: session-level snapshot restore behind the lifecycle policy
/// gate.
pub fn restore_snapshot(
    id: u64,
    request_id: u64,
    snapshot: &DocumentSnapshot,
) -> Result<SnapshotRestoreOutcome, TestError> {
    with_live(id, |session| {
        session
            .restore_snapshot(request_id, snapshot)
            .map(|commit| SnapshotRestoreOutcome {
                changed: commit.changed,
                document_revision: commit.revision,
            })
    })
}

/// `(pending protocol reply count, bytes)`; `None` when the session has
/// no attached runtime (and therefore no protocol queue).
pub fn pending_protocol_replies(id: u64) -> Result<Option<(usize, usize)>, TestError> {
    with_live(id, |session| {
        Ok(session.collaboration_outbox().map(|outbox| {
            (
                outbox.pending_protocol_reply_count(),
                outbox.pending_protocol_reply_bytes(),
            )
        }))
    })
}

pub fn undo(id: u64, request_id: u64) -> Result<bool, TestError> {
    with_live(id, |session| {
        session
            .engine
            .undo(request_id)
            .map(|commit| commit.is_some())
            .map_err(|error| {
                SessionError::from_operation(error, OperationFailureClass::ExistingStableCode)
            })
    })
}

pub fn redo(id: u64, request_id: u64) -> Result<bool, TestError> {
    with_live(id, |session| {
        session
            .engine
            .redo(request_id)
            .map(|commit| commit.is_some())
            .map_err(|error| {
                SessionError::from_operation(error, OperationFailureClass::ExistingStableCode)
            })
    })
}

pub fn encoded_state(id: u64) -> Result<Vec<u8>, TestError> {
    with_live(id, |session| {
        session.engine.encoded_state().map_err(SessionError::from)
    })
}

pub fn client_id(id: u64) -> Result<u64, TestError> {
    with_live(id, |session| Ok(session.engine.client_id()))
}

pub fn session_audit(id: u64) -> Result<SessionAudit, TestError> {
    with_live(id, |session| {
        Ok(SessionAudit {
            document_json: session.engine.document_json(),
            document_html: session.engine.document_html(),
            encoded_state: session.engine.encoded_state().ok(),
            document_revision: session.engine.revision(),
            state_revision: session.engine.state_revision(),
            can_undo: session.engine.can_undo(),
            can_redo: session.engine.can_redo(),
            selection: session
                .engine
                .resolved_selection()
                .map(|selection| format!("{selection:?}")),
            stored_marks: session
                .engine
                .stored_marks()
                .map(|marks| format!("{marks:?}")),
            document_state: map_document_state(session.document_state),
            transport_state: map_transport_state(session.transport_state()),
        })
    })
}

fn with_live<T>(
    id: u64,
    operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
) -> Result<T, TestError> {
    with_session(id, operation).map_err(TestError::from)
}
