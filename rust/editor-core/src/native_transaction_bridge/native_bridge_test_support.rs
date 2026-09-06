use super::*;
use crate::boundary::ResourceLimits;
use crate::registry;
use crate::session::{EditorInitialization, EditorSessionConfig, InitialContent};
use crate::yrs_engine::EditingLimits;

pub use crate::collaboration_runtime::outbox::set_reservation_allocation_failure_for_test as set_outbox_allocation_failure;
pub use crate::document_api::session_initialization_test_support::TestError;

/// Structured mirror of [`NativeBridgeOutcome`] for integration tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeTestOutcome {
    Transaction {
        changed: bool,
        document_revision: u64,
        state_revision: u64,
        can_undo: bool,
        can_redo: bool,
    },
    NotApplicable,
    Replacement {
        changed: bool,
        document_revision: u64,
    },
}

/// Complete before/after audit for atomic-rejection comparisons,
/// including the collaboration outbox accounting.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeSessionAudit {
    pub document_json: Option<serde_json::Value>,
    pub document_html: Option<String>,
    pub encoded_state: Option<Vec<u8>>,
    pub state_vector: Option<Vec<u8>>,
    pub document_revision: u64,
    pub state_revision: u64,
    pub yrs_state_epoch: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub selection: Option<String>,
    pub stored_marks: Option<String>,
    pub last_committed_origin: Option<String>,
    pub outbox_pending_updates: Option<usize>,
    pub outbox_pending_bytes: Option<usize>,
    pub last_reserved_upper_bound: Option<usize>,
}

/// Session construction knobs for bridge/outbox coverage.
#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    pub read_only: bool,
    pub input_filter: Option<String>,
    pub initial_json: Option<String>,
    pub attach_runtime: bool,
}

pub fn create_session(options: SessionOptions) -> Result<u64, TestError> {
    let config = EditorSessionConfig {
        schema_json: None,
        fragment_name: "prosemirror".into(),
        initialization: EditorInitialization::Local {
            initial_content: match options.initial_json {
                Some(json) => InitialContent::Json(json),
                None => InitialContent::Empty,
            },
        },
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        collaboration_limits: crate::session::CollaborationLimits::default(),
        max_length: None,
        read_only: options.read_only,
        input_filter: options.input_filter,
        allow_base64_images: false,
    };
    let id = crate::document_api::DocumentApiFacade::create(config).map_err(TestError::from)?;
    if options.attach_runtime {
        attach_runtime(id)?;
    }
    Ok(id)
}

pub fn destroy_session(id: u64) {
    registry::destroy_session(id);
}

pub fn attach_runtime(id: u64) -> Result<(), TestError> {
    with_live_session(id, |session| {
        session.attach_collaboration_runtime();
        Ok(())
    })
}

pub fn submit_input(id: u64, envelope: &str) -> Result<BridgeTestOutcome, TestError> {
    submit(id, envelope, |bridge, envelope| {
        bridge.submit_input(envelope)
    })
}

pub fn submit_command(id: u64, envelope: &str) -> Result<BridgeTestOutcome, TestError> {
    submit(id, envelope, |bridge, envelope| {
        bridge.submit_command(envelope)
    })
}

pub fn submit_selection(id: u64, envelope: &str) -> Result<BridgeTestOutcome, TestError> {
    submit(id, envelope, |bridge, envelope| {
        bridge.submit_selection(envelope)
    })
}

pub fn submit_local_api(id: u64, envelope: &str) -> Result<BridgeTestOutcome, TestError> {
    submit(id, envelope, |bridge, envelope| {
        bridge.submit_local_api(envelope)
    })
}

pub fn undo(id: u64, request_id: u64) -> Result<bool, TestError> {
    with_live_session(id, |session| {
        let (engine, outbox) = session.engine_and_outbox();
        engine
            .undo_with_outbox(request_id, outbox)
            .map(|commit| commit.is_some())
            .map_err(super::operation_error)
    })
}

pub fn redo(id: u64, request_id: u64) -> Result<bool, TestError> {
    with_live_session(id, |session| {
        let (engine, outbox) = session.engine_and_outbox();
        engine
            .redo_with_outbox(request_id, outbox)
            .map(|commit| commit.is_some())
            .map_err(super::operation_error)
    })
}

/// `(pending update count, pending bytes)`; `None` when the session has
/// no attached collaboration runtime (and therefore no outbox).
pub fn outbox_pending(id: u64) -> Result<Option<(usize, usize)>, TestError> {
    with_live_session(id, |session| {
        Ok(session.collaboration_outbox().map(|outbox| {
            (
                outbox.pending_document_update_count(),
                outbox.pending_document_update_bytes(),
            )
        }))
    })
}

pub fn last_reserved_upper_bound(id: u64) -> Result<Option<usize>, TestError> {
    with_live_session(id, |session| {
        Ok(session
            .collaboration_outbox()
            .and_then(|outbox| outbox.last_reserved_upper_bound_for_test()))
    })
}

/// Retain the next raw document update for test transport simulation.
/// The entry stays charged until `ack_leased_update`; protocol frames are
/// intentionally not exposed by this local-mutation fixture seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeasedDocumentUpdate {
    pub lease_id: u64,
    pub request_id: u64,
    pub update_v1: Vec<u8>,
}

pub fn lease_next_update(id: u64) -> Result<Option<LeasedDocumentUpdate>, TestError> {
    with_live_session(id, |session| {
        let (_, outbox) = session.engine_and_outbox();
        let Some(outbox) = outbox else {
            return Ok(None);
        };
        match outbox.lease_next().map_err(|error| {
            crate::session::SessionError::new(
                crate::session::ErrorDomain::Transport,
                "TRANSPORT_INVALID_TRANSITION",
                format!("native bridge test lease failed: {error:?}"),
            )
        })? {
            None => Ok(None),
            Some(crate::collaboration_runtime::outbox::OutboundLease {
                lease_id,
                payload:
                    crate::collaboration_runtime::outbox::OutboundLeasePayload::DocumentUpdate(
                        update_v1,
                    ),
            }) => {
                let request_id = outbox
                    .pending_document_update_request_id_for_leased_front()
                    .expect("a leased document front has its original request id");
                Ok(Some(LeasedDocumentUpdate {
                    lease_id: lease_id.value(),
                    request_id,
                    update_v1,
                }))
            }
            Some(crate::collaboration_runtime::outbox::OutboundLease {
                lease_id: _,
                payload:
                    crate::collaboration_runtime::outbox::OutboundLeasePayload::ProtocolReply(_),
            }) => {
                outbox.release_lease();
                Err(crate::session::SessionError::new(
                    crate::session::ErrorDomain::Transport,
                    "TRANSPORT_INVALID_TRANSITION",
                    "native bridge document fixture cannot lease a protocol reply",
                ))
            }
        }
    })
}

/// The kind of queue front one [`ack_next_outbound`] drain step retired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainedOutboundKind {
    DocumentUpdate,
    ProtocolReply,
}

/// Lease and acknowledge the current outbound front whatever its kind —
/// the ordered drain the platform transports perform. Unlike
/// [`lease_next_update`], an awareness broadcast or protocol reply ahead
/// of a document update is retired rather than refused. `None` once the
/// queue is empty.
pub fn ack_next_outbound(id: u64) -> Result<Option<DrainedOutboundKind>, TestError> {
    with_live_session(id, |session| {
        let (_, outbox) = session.engine_and_outbox();
        let Some(outbox) = outbox else {
            return Ok(None);
        };
        let lease = match outbox.lease_next().map_err(|error| {
            crate::session::SessionError::new(
                crate::session::ErrorDomain::Transport,
                "TRANSPORT_INVALID_TRANSITION",
                format!("native bridge test drain lease failed: {error:?}"),
            )
        })? {
            None => return Ok(None),
            Some(lease) => lease,
        };
        let kind = match lease.payload {
            crate::collaboration_runtime::outbox::OutboundLeasePayload::DocumentUpdate(_) => {
                DrainedOutboundKind::DocumentUpdate
            }
            crate::collaboration_runtime::outbox::OutboundLeasePayload::ProtocolReply(_) => {
                DrainedOutboundKind::ProtocolReply
            }
        };
        outbox.ack_lease(lease.lease_id).map_err(|error| {
            crate::session::SessionError::new(
                crate::session::ErrorDomain::Transport,
                "TRANSPORT_INVALID_TRANSITION",
                format!("native bridge test drain ACK failed: {error:?}"),
            )
        })?;
        Ok(Some(kind))
    })
}

pub fn ack_leased_update(id: u64, lease_id: u64) -> Result<(), TestError> {
    with_live_session(id, |session| {
        let (_, outbox) = session.engine_and_outbox();
        let outbox = outbox.ok_or_else(crate::session::no_attached_runtime)?;
        outbox
            .ack_lease(crate::collaboration_runtime::outbox::OutboundLeaseId::from_value(lease_id))
            .map_err(|error| {
                crate::session::SessionError::new(
                    crate::session::ErrorDomain::Transport,
                    "TRANSPORT_INVALID_TRANSITION",
                    format!("native bridge test lease ACK failed: {error:?}"),
                )
            })
    })
}

pub fn set_outbox_ceilings(id: u64, messages: usize, bytes: usize) -> Result<(), TestError> {
    with_live_session(id, |session| {
        let (_, outbox) = session.engine_and_outbox();
        let outbox = outbox.ok_or_else(crate::session::no_attached_runtime)?;
        outbox.set_ceilings_for_test(messages, bytes);
        Ok(())
    })
}

/// Probe the conservative bound the next input commit would reserve;
/// `None` when the commit lowers to a reservation-free no-op.
pub fn probe_input_upper_bound(id: u64, envelope: &str) -> Result<Option<usize>, TestError> {
    with_live_session(id, |session| {
        let parsed: InputRequestEnvelope = parse_envelope(session, envelope)?;
        admit_version(parsed.version, parsed.request_id)?;
        let filtered = apply_input_filter(
            session.policy.input_filter_regex(),
            &parsed.text,
            parsed.request_id,
        )?;
        match lower_input(&session.engine, parsed.request_id, filtered)? {
            LoweredInput::NotApplicable => Ok(None),
            LoweredInput::Transaction(transaction) if transaction.operations.is_empty() => Ok(None),
            LoweredInput::Transaction(transaction) => session
                .engine
                .probe_transaction_outbound_upper_bound(transaction)
                .map(Some)
                .map_err(super::operation_error),
        }
    })
}

pub fn probe_command_upper_bound(id: u64, envelope: &str) -> Result<Option<usize>, TestError> {
    with_live_session(id, |session| {
        let parsed: CommandRequestEnvelope = parse_envelope(session, envelope)?;
        admit_version(parsed.version, parsed.request_id)?;
        session
            .engine
            .probe_command_outbound_upper_bound(parsed.request_id, parsed.command.into())
            .map_err(super::operation_error)
    })
}

pub fn probe_history_pop_bytes(
    id: u64,
    request_id: u64,
    undoing: bool,
) -> Result<Option<usize>, TestError> {
    with_live_session(id, |session| {
        session
            .engine
            .probe_history_pop_outbound_bytes(request_id, undoing)
            .map_err(super::operation_error)
    })
}

pub fn probe_replace_json_upper_bound(
    id: u64,
    request_id: u64,
    json: &str,
    reset: bool,
) -> Result<usize, TestError> {
    let history = if reset {
        ReplacementHistory::ResetAndClear
    } else {
        ReplacementHistory::UndoableBoundary
    };
    with_live_session(id, |session| {
        session
            .engine
            .probe_root_replacement_json_outbound_upper_bound(request_id, json, history)
            .map_err(|error| crate::session::replacement_session_error(error, request_id))
    })
}

/// One-shot remote update through the engine; never an outbox entry.
pub fn apply_remote_update(id: u64, request_id: u64, update: &[u8]) -> Result<bool, TestError> {
    with_live_session(id, |session| {
        session
            .engine
            .apply_remote_update_v1(request_id, update)
            .map(|commit| commit.changed)
            .map_err(super::operation_error)
    })
}

/// Sealed prepare/commit remote update; never an outbox entry.
pub fn apply_prepared_remote_update(
    id: u64,
    request_id: u64,
    update: &[u8],
) -> Result<bool, TestError> {
    with_live_session(id, |session| {
        let prepared = session
            .engine
            .prepare_remote_update_v1(request_id, update)
            .map_err(super::operation_error)?;
        session
            .engine
            .commit_prepared_remote_update(prepared)
            .map(|commit| commit.changed)
            .map_err(super::operation_error)
    })
}

pub fn session_audit(id: u64) -> Result<NativeSessionAudit, TestError> {
    with_live_session(id, |session| {
        let outbox = session.collaboration_outbox();
        Ok(NativeSessionAudit {
            document_json: session.engine.document_json(),
            document_html: session.engine.document_html(),
            encoded_state: session.engine.encoded_state().ok(),
            state_vector: session.engine.encode_state_vector_v1(0).ok(),
            document_revision: session.engine.revision(),
            state_revision: session.engine.state_revision(),
            yrs_state_epoch: session.engine.yrs_state_epoch(),
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
            last_committed_origin: session
                .engine
                .last_committed_origin()
                .map(|origin| origin.as_tag().to_string()),
            outbox_pending_updates: outbox.map(
                crate::collaboration_runtime::CollaborationOutbox::pending_document_update_count,
            ),
            outbox_pending_bytes: outbox.map(
                crate::collaboration_runtime::CollaborationOutbox::pending_document_update_bytes,
            ),
            last_reserved_upper_bound: outbox
                .and_then(|outbox| outbox.last_reserved_upper_bound_for_test()),
        })
    })
}

fn submit(
    id: u64,
    envelope: &str,
    entry: impl FnOnce(
        &mut NativeTransactionBridge<'_>,
        &str,
    ) -> Result<NativeBridgeOutcome, SessionError>,
) -> Result<BridgeTestOutcome, TestError> {
    with_live_session(id, |session| {
        let mut bridge = NativeTransactionBridge::new(session);
        entry(&mut bridge, envelope).map(|outcome| match outcome {
            NativeBridgeOutcome::Transaction(result) => BridgeTestOutcome::Transaction {
                changed: result.changed,
                document_revision: result.document_revision,
                state_revision: result.state_revision,
                can_undo: result.history_state.can_undo,
                can_redo: result.history_state.can_redo,
            },
            NativeBridgeOutcome::NotApplicable => BridgeTestOutcome::NotApplicable,
            NativeBridgeOutcome::Replacement(commit) => BridgeTestOutcome::Replacement {
                changed: commit.changed,
                document_revision: commit.document_revision,
            },
        })
    })
}

fn with_live_session<T>(
    id: u64,
    operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
) -> Result<T, TestError> {
    let slot = registry::get_session(id).ok_or_else(|| {
        TestError::from(SessionError::new(
            crate::session::ErrorDomain::Lifecycle,
            "ENGINE_DESTROYED",
            "editor session is not registered",
        ))
    })?;
    slot.with_alive(|session| operation(session))
        .and_then(|value| value)
        .map_err(TestError::from)
}
