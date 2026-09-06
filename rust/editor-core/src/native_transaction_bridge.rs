//! `NativeTransactionBridge` — the only production local mutation entrance.
//!
//! The bridge borrows one live [`EditorSession`] and owns no document state.
//! It accepts versioned, bounded, data-only request envelopes with DISTINCT
//! entry points — input (composition/typing), command, selection, and
//! local-API — so callers can never opt into or out of local undo tracking.
//! For every request it:
//!
//! 1. validates the envelope (size, shape, version, fields) before any
//!    proportional engine work;
//! 2. assigns the trusted transaction origin itself — no envelope carries an
//!    origin field, and a caller-supplied `origin` is a CONFIG_INVALID-class
//!    unknown-field rejection;
//! 3. enforces the session's read-only and input-filter policy;
//! 4. invokes exactly one engine planner/typed-transaction path, threading
//!    the optionally attached collaboration outbox for bounded pre-write
//!    reservation;
//! 5. returns one structured result or one frozen-domain error envelope.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed six-domain boundary envelope"
)]

use std::collections::HashMap;

use crate::boundary::{BoundaryError, BoundedInput, InputKind};
use crate::ffi_v2::types::{deserialize_canonical_u64, recover_request_id};
use crate::session::{EditorSession, ErrorDomain, OperationFailureClass, SessionError};
use crate::yrs_engine::{
    Affinity, CommandPlan, EditorOffsetKind, HistoryPolicy, OperationError, ReplacementHistory,
    RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent, TransactionCommit,
    TransactionOrigin, TypedCommand, TypedTransaction, TypedTransactionResult, YrsDocumentEngine,
};

/// The one supported native bridge envelope version.
const NATIVE_BRIDGE_ENVELOPE_VERSION: u32 = 1;

/// Affinity used when a data-only position envelope omits it: typing and
/// selection anchors stick after the addressed position.
const DEFAULT_POSITION_AFFINITY: Affinity = Affinity::After;

/// One structured bridge result. Command planning that finds nothing
/// applicable is a structured outcome, never a fabricated engine result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NativeBridgeOutcome {
    Transaction(Box<TypedTransactionResult>),
    NotApplicable,
    Replacement(TransactionCommit),
}

/// The production local mutation entrance. Borrows the (already
/// lifecycle-checked and locked) session for the duration of one request.
pub(crate) struct NativeTransactionBridge<'session> {
    session: &'session mut EditorSession,
}

include!("native_transaction_bridge/envelopes.rs");

/// Lowered input commit: either nothing was applicable, or exactly one
/// typed local-input transaction.
enum LoweredInput {
    NotApplicable,
    Transaction(TypedTransaction),
}

impl<'session> NativeTransactionBridge<'session> {
    pub(crate) fn new(session: &'session mut EditorSession) -> Self {
        Self { session }
    }

    pub(crate) fn submit_native_intent(&mut self, envelope: &str) -> Result<String, SessionError> {
        let envelope: NativeIntentRequestEnvelope = parse_envelope(self.session, envelope)?;
        admit_version(envelope.version, envelope.request_id)?;
        if let Some(cached) = self
            .session
            .native_request_outcome(envelope.owner_id, envelope.request_id)?
        {
            return Ok(cached.to_owned());
        }

        let (anchor, head) = envelope.intent.offsets();
        let resolved = self.session.resolve_epoch_range(
            envelope.owner_id,
            envelope.position_epoch,
            anchor,
            head,
        )?;
        let scalar_limit = self
            .session
            .engine
            .position_map()
            .ok_or_else(|| operation_error(OperationError::engine_not_ready(envelope.request_id)))?
            .total_scalars();
        let selection = scalar_selection(resolved.anchor, resolved.head, scalar_limit);
        let request_id = envelope.request_id;
        let document_revision_before = self.session.engine.revision();
        let outcome = match envelope.intent {
            NativeIntentEnvelope::SetSelection { .. } => {
                let (engine, outbox) = self.session.engine_and_outbox();
                let mut outbox = outbox;
                let transaction =
                    native_selection_transaction(request_id, engine.revision(), selection);
                let applied = engine.apply_typed_transaction_with_outbox(
                    transaction,
                    true,
                    outbox.as_deref_mut(),
                );
                let (_, result) = match applied {
                    Err(error)
                        if error.code == "POSITION_INVALID" && resolved.anchor == resolved.head =>
                    {
                        engine.apply_typed_transaction_with_outbox(
                            native_selection_transaction(
                                request_id,
                                engine.revision(),
                                scalar_selection_with_affinity(
                                    resolved.anchor,
                                    resolved.head,
                                    scalar_limit,
                                    Affinity::Before,
                                ),
                            ),
                            true,
                            outbox.as_deref_mut(),
                        )
                    }
                    result => result,
                }
                .map_err(operation_error)?;
                typed_outcome(request_id, result)?
            }
            intent => {
                self.admit_writable(request_id)?;
                let intent = match intent {
                    NativeIntentEnvelope::InsertText { anchor, head, text } => apply_input_filter(
                        self.session.policy.input_filter_regex(),
                        &text,
                        request_id,
                    )?
                    .map(|text| NativeIntentEnvelope::InsertText { anchor, head, text }),
                    NativeIntentEnvelope::ReplaceSelectionText { anchor, head, text } => {
                        apply_input_filter(
                            self.session.policy.input_filter_regex(),
                            &text,
                            request_id,
                        )?
                        .map(|text| {
                            NativeIntentEnvelope::ReplaceSelectionText { anchor, head, text }
                        })
                    }
                    intent => Some(intent),
                };
                match intent {
                    None => {
                        let (engine, outbox) = self.session.engine_and_outbox();
                        let transaction = match lower_input(engine, request_id, None)? {
                            LoweredInput::Transaction(transaction) => transaction,
                            LoweredInput::NotApplicable => {
                                return Err(operation_error(
                                    OperationError::engine_invariant_failed(
                                        request_id,
                                        None,
                                        "fully filtered native input did not lower to a transaction",
                                    ),
                                ));
                            }
                        };
                        let (_, result) = engine
                            .apply_typed_transaction_with_outbox(transaction, true, outbox)
                            .map_err(operation_error)?;
                        typed_outcome(request_id, result)?
                    }
                    Some(intent) => {
                        let command = lower_native_intent(
                            intent,
                            resolved.anchor,
                            resolved.head,
                            request_id,
                            &self.session.engine,
                        )?;
                        let (engine, outbox) = self.session.engine_and_outbox();
                        let mut outbox = outbox;
                        let applied = engine.apply_command_at_selection_with_outbox(
                            request_id,
                            command.clone(),
                            selection,
                            TransactionOrigin::LocalInput,
                            outbox.as_deref_mut(),
                        );
                        let result = match applied {
                            Err(error)
                                if error.code == "POSITION_INVALID"
                                    && resolved.anchor == resolved.head =>
                            {
                                engine.apply_command_at_selection_with_outbox(
                                    request_id,
                                    command,
                                    scalar_selection_with_affinity(
                                        resolved.anchor,
                                        resolved.head,
                                        scalar_limit,
                                        Affinity::Before,
                                    ),
                                    TransactionOrigin::LocalInput,
                                    outbox.as_deref_mut(),
                                )
                            }
                            result => result,
                        }
                        .map_err(operation_error)?;
                        match result {
                            Some(result) => NativeBridgeOutcome::Transaction(Box::new(result)),
                            None => NativeBridgeOutcome::NotApplicable,
                        }
                    }
                }
            }
        };
        let document_changed = self.session.engine.revision() != document_revision_before;
        if document_changed {
            self.session.engine.mark_document_origin_native_view();
        }
        let serialized = serialize_native_outcome(outcome, resolved.fallback, document_changed);
        self.session.retain_native_request_outcome(
            envelope.owner_id,
            request_id,
            serialized.clone(),
        );
        Ok(serialized)
    }

    /// Composition/typing commit: exactly one typed local-input transaction
    /// through the planner, replacing the current selection with the
    /// (policy-filtered) committed text.
    pub(crate) fn submit_input(
        &mut self,
        envelope: &str,
    ) -> Result<NativeBridgeOutcome, SessionError> {
        let envelope: InputRequestEnvelope = parse_envelope(self.session, envelope)?;
        admit_version(envelope.version, envelope.request_id)?;
        let request_id = envelope.request_id;
        if envelope.text.is_empty() {
            return Err(config_invalid(
                request_id,
                "input commits require non-empty text",
            ));
        }
        self.admit_writable(request_id)?;
        self.admit_base_revision(request_id, envelope.base_document_revision)?;
        let filtered = apply_input_filter(
            self.session.policy.input_filter_regex(),
            &envelope.text,
            request_id,
        )?;
        let (engine, outbox) = self.session.engine_and_outbox();
        let lowered = lower_input(engine, request_id, filtered)?;
        let transaction = match lowered {
            LoweredInput::NotApplicable => return Ok(NativeBridgeOutcome::NotApplicable),
            LoweredInput::Transaction(transaction) => transaction,
        };
        let (_, result) = engine
            .apply_typed_transaction_with_outbox(transaction, true, outbox)
            .map_err(operation_error)?;
        typed_outcome(request_id, result)
    }

    /// Named editor command through the planner (trusted local-command
    /// origin, planner-owned undo tracking).
    pub(crate) fn submit_command(
        &mut self,
        envelope: &str,
    ) -> Result<NativeBridgeOutcome, SessionError> {
        let envelope: CommandRequestEnvelope = parse_envelope(self.session, envelope)?;
        admit_version(envelope.version, envelope.request_id)?;
        let request_id = envelope.request_id;
        self.admit_writable(request_id)?;
        self.admit_base_revision(request_id, envelope.base_document_revision)?;
        let (engine, outbox) = self.session.engine_and_outbox();
        let result = engine
            .apply_command_with_outbox(request_id, envelope.command.into(), outbox)
            .map_err(operation_error)?;
        match result {
            Some(result) => Ok(NativeBridgeOutcome::Transaction(Box::new(result))),
            None => Ok(NativeBridgeOutcome::NotApplicable),
        }
    }

    /// Selection/state-only request: one empty skip transaction. Reserves
    /// nothing and enqueues nothing in the collaboration outbox.
    pub(crate) fn submit_selection(
        &mut self,
        envelope: &str,
    ) -> Result<NativeBridgeOutcome, SessionError> {
        let envelope: SelectionRequestEnvelope = parse_envelope(self.session, envelope)?;
        admit_version(envelope.version, envelope.request_id)?;
        let request_id = envelope.request_id;
        self.admit_base_revision(request_id, envelope.base_document_revision)?;
        let selection = match envelope.selection {
            SelectionEnvelope::Atom { doc_pos, edge } => {
                let engine = &self.session.engine;
                let map = engine
                    .position_map()
                    .ok_or_else(|| operation_error(OperationError::engine_not_ready(request_id)))?;
                let document = engine
                    .document()
                    .ok_or_else(|| operation_error(OperationError::engine_not_ready(request_id)))?;
                let Some(block_index) = map.find_block_for_doc_pos(doc_pos) else {
                    return Ok(NativeBridgeOutcome::NotApplicable);
                };
                let block = map.block(block_index).expect("mapped block exists");
                let scalar = map.doc_to_scalar(doc_pos, document);
                if !block.is_void_block || map.effective_doc_start(block_index) != doc_pos {
                    return Ok(NativeBridgeOutcome::NotApplicable);
                }
                let (offset, affinity) = match edge {
                    AtomSelectionEdge::Node => (scalar, Affinity::After),
                    AtomSelectionEdge::Before => (
                        scalar.saturating_sub(1),
                        if scalar == 0 {
                            Affinity::After
                        } else {
                            Affinity::Before
                        },
                    ),
                    AtomSelectionEdge::After => (
                        (scalar + block.scalar_len + block.rendered_break_after)
                            .min(map.total_scalars()),
                        Affinity::After,
                    ),
                };
                let point = scalar_position(offset, map.total_scalars(), affinity);
                match edge {
                    AtomSelectionEdge::Node => SelectionInput::Node { at: point },
                    _ => SelectionInput::Text {
                        anchor: point,
                        head: point,
                    },
                }
            }
            selection => selection.into(),
        };
        let transaction = TypedTransaction {
            request_id,
            base_document_revision: envelope.base_document_revision,
            origin: TransactionOrigin::LocalInput,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(selection),
            history_policy: HistoryPolicy::Skip,
        };
        let (engine, outbox) = self.session.engine_and_outbox();
        let (_, result) = engine
            .apply_typed_transaction_with_outbox(transaction, true, outbox)
            .map_err(operation_error)?;
        typed_outcome(request_id, result)
    }

    /// Local-API request: whole-document replacement through the session
    /// policy gate. Passes under read-only policy (the legacy `Source::Api`
    /// pass-through) and carries the trusted local-API origin.
    pub(crate) fn submit_local_api(
        &mut self,
        envelope: &str,
    ) -> Result<NativeBridgeOutcome, SessionError> {
        let envelope: LocalApiRequestEnvelope = parse_envelope(self.session, envelope)?;
        admit_version(envelope.version, envelope.request_id)?;
        let request_id = envelope.request_id;
        self.admit_base_revision(request_id, envelope.base_document_revision)?;
        let history = ReplacementHistory::from(envelope.history);
        let commit = match (envelope.set_json, envelope.set_html) {
            (Some(json), None) => {
                self.session
                    .replace_document_json(request_id, &json.to_string(), history)?
            }
            (None, Some(html)) => self
                .session
                .replace_document_html(request_id, &html, history)?,
            _ => {
                return Err(config_invalid(
                    request_id,
                    "local-API requests carry exactly one of setJson or setHtml",
                ));
            }
        };
        Ok(NativeBridgeOutcome::Replacement(commit))
    }

    /// Read-only policy for the mutating input/command entry points; the
    /// selection and local-API entries pass (legacy interceptor semantics).
    fn admit_writable(&self, request_id: u64) -> Result<(), SessionError> {
        if self.session.policy.read_only() {
            let mut error = SessionError::new(
                ErrorDomain::Boundary,
                "MUTATION_REJECTED",
                "document is read-only; only selection and local-API requests are allowed",
            );
            error.request_id = Some(request_id);
            return Err(error);
        }
        Ok(())
    }

    /// Undo: one history walk through the engine with the optionally
    /// attached outbox. Read-only policy covers history mutations (Task 12
    /// tracked Minor: the legacy locked `ReadOnly` rejects
    /// `Source::History`); the rejection is structured and atomic — no
    /// engine work happens after the policy check fails.
    pub(crate) fn undo(&mut self, request_id: u64) -> Result<bool, SessionError> {
        self.admit_history_writable(request_id)?;
        let (engine, outbox) = self.session.engine_and_outbox();
        engine
            .undo_with_outbox(request_id, outbox)
            .map(|commit| commit.is_some())
            .map_err(operation_error)
    }

    /// Redo: the mirror history walk under the same read-only coverage.
    pub(crate) fn redo(&mut self, request_id: u64) -> Result<bool, SessionError> {
        self.admit_history_writable(request_id)?;
        let (engine, outbox) = self.session.engine_and_outbox();
        engine
            .redo_with_outbox(request_id, outbox)
            .map(|commit| commit.is_some())
            .map_err(operation_error)
    }

    /// Read-only policy for the history entry points (Task 12 tracked
    /// Minor): the legacy locked `ReadOnly` rejects `Source::History`
    /// transactions, so undo/redo refuse with the frozen policy code before
    /// any engine work.
    fn admit_history_writable(&self, request_id: u64) -> Result<(), SessionError> {
        if self.session.policy.read_only() {
            let mut error = SessionError::new(
                ErrorDomain::Boundary,
                "MUTATION_REJECTED",
                "document is read-only; history mutations are not allowed",
            );
            error.request_id = Some(request_id);
            return Err(error);
        }
        Ok(())
    }

    /// Stale native requests reject deterministically; native code refreshes
    /// from engine state instead of retrying against guessed positions.
    fn admit_base_revision(&self, request_id: u64, base: u64) -> Result<(), SessionError> {
        let actual = self.session.engine.revision();
        if base != actual {
            return Err(operation_error(OperationError::revision_mismatch(
                request_id, base, actual,
            )));
        }
        Ok(())
    }
}

impl NativeIntentEnvelope {
    fn offsets(&self) -> (u32, u32) {
        match self {
            Self::SetSelection { anchor, head }
            | Self::InsertText { anchor, head, .. }
            | Self::ReplaceSelectionText { anchor, head, .. }
            | Self::DeleteBackward { anchor, head }
            | Self::DeleteForward { anchor, head }
            | Self::DeleteSurroundingText { anchor, head, .. }
            | Self::DeleteRange { anchor, head }
            | Self::SplitBlock { anchor, head }
            | Self::DeleteAndSplit { anchor, head }
            | Self::InsertContentHtml { anchor, head, .. }
            | Self::InsertContentJson { anchor, head, .. }
            | Self::Command { anchor, head, .. } => (*anchor, *head),
        }
    }
}

fn scalar_position(offset: u32, scalar_limit: u32, affinity: Affinity) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: if offset >= scalar_limit {
            Affinity::Before
        } else {
            affinity
        },
    }
}

fn scalar_selection(anchor: u32, head: u32, scalar_limit: u32) -> SelectionInput {
    let affinity = if anchor == head {
        Affinity::After
    } else {
        Affinity::Before
    };
    scalar_selection_with_affinity(anchor, head, scalar_limit, affinity)
}

fn scalar_selection_with_affinity(
    anchor: u32,
    head: u32,
    scalar_limit: u32,
    affinity: Affinity,
) -> SelectionInput {
    SelectionInput::Text {
        anchor: scalar_position(anchor, scalar_limit, affinity),
        head: scalar_position(head, scalar_limit, affinity),
    }
}

fn scalar_range(from: u32, to: u32, scalar_limit: u32) -> RevisionedRange {
    RevisionedRange {
        from: scalar_position(from, scalar_limit, Affinity::Before),
        to: scalar_position(to, scalar_limit, Affinity::Before),
    }
}

fn native_selection_transaction(
    request_id: u64,
    document_revision: u64,
    selection: SelectionInput,
) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: document_revision,
        origin: TransactionOrigin::LocalInput,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Set(selection),
        history_policy: HistoryPolicy::Skip,
    }
}

fn lower_native_intent(
    intent: NativeIntentEnvelope,
    anchor: u32,
    head: u32,
    request_id: u64,
    engine: &YrsDocumentEngine,
) -> Result<TypedCommand, SessionError> {
    let command = match intent {
        NativeIntentEnvelope::SetSelection { .. } => {
            return Err(SessionError::new(
                ErrorDomain::Operation,
                "ENGINE_INVARIANT_FAILED",
                "selection intent reached command lowering",
            ));
        }
        NativeIntentEnvelope::InsertText { text, .. } => TypedCommand::InsertText { text },
        NativeIntentEnvelope::ReplaceSelectionText { text, .. } => {
            TypedCommand::ReplaceSelectionText { text }
        }
        NativeIntentEnvelope::DeleteBackward { .. } => TypedCommand::DeleteBackward,
        NativeIntentEnvelope::DeleteForward { .. } => {
            let limit = engine
                .position_map()
                .ok_or_else(|| operation_error(OperationError::engine_not_ready(request_id)))?
                .total_scalars();
            let (from, to) = if anchor == head {
                (anchor, anchor.saturating_add(1).min(limit))
            } else {
                (anchor.min(head), anchor.max(head))
            };
            TypedCommand::DeleteRange {
                range: scalar_range(from, to, limit),
            }
        }
        NativeIntentEnvelope::DeleteSurroundingText { before, after, .. } => {
            let limit = engine
                .position_map()
                .ok_or_else(|| operation_error(OperationError::engine_not_ready(request_id)))?
                .total_scalars();
            let from = anchor.min(head).saturating_sub(before);
            let to = anchor.max(head).saturating_add(after).min(limit);
            TypedCommand::DeleteRange {
                range: scalar_range(from, to, limit),
            }
        }
        NativeIntentEnvelope::DeleteRange { .. } => {
            let limit = engine
                .position_map()
                .ok_or_else(|| operation_error(OperationError::engine_not_ready(request_id)))?
                .total_scalars();
            TypedCommand::DeleteRange {
                range: scalar_range(anchor, head, limit),
            }
        }
        NativeIntentEnvelope::SplitBlock { .. } => TypedCommand::SplitBlock,
        NativeIntentEnvelope::DeleteAndSplit { .. } => TypedCommand::DeleteAndSplit,
        NativeIntentEnvelope::InsertContentHtml { html, .. } => {
            TypedCommand::InsertContentHtml { html }
        }
        NativeIntentEnvelope::InsertContentJson { json, .. } => {
            TypedCommand::InsertContentJson { json }
        }
        NativeIntentEnvelope::Command { command, .. } => command.into(),
    };
    Ok(command)
}

pub(crate) fn serialize_native_outcome(
    outcome: NativeBridgeOutcome,
    position_fallback: bool,
    document_changed: bool,
) -> String {
    match outcome {
        NativeBridgeOutcome::Transaction(result) => serde_json::json!({
            "type": "transaction",
            "changed": result.changed,
            "documentChanged": document_changed,
            "documentRevision": result.document_revision.to_string(),
            "stateRevision": result.state_revision.to_string(),
            "canUndo": result.history_state.can_undo,
            "canRedo": result.history_state.can_redo,
            "positionFallback": position_fallback,
        })
        .to_string(),
        NativeBridgeOutcome::NotApplicable => serde_json::json!({
            "type": "notApplicable",
            "positionFallback": position_fallback,
        })
        .to_string(),
        NativeBridgeOutcome::Replacement(commit) => serde_json::json!({
            "type": "replacement",
            "changed": commit.changed,
            "documentChanged": document_changed,
            "documentRevision": commit.document_revision.to_string(),
            "positionFallback": position_fallback,
        })
        .to_string(),
    }
}

/// Bounded-size admission plus data-only parse, both before any
/// proportional engine work. Unknown fields — including any caller-supplied
/// `origin` — reject as CONFIG_INVALID.
fn parse_envelope<T: serde::de::DeserializeOwned>(
    session: &EditorSession,
    envelope: &str,
) -> Result<T, SessionError> {
    let input = BoundedInput::new(
        envelope,
        InputKind::Config,
        session.engine.resource_limits(),
    )?;
    serde_json::from_str(input.as_str()).map_err(|error| {
        let mut error = SessionError::from(BoundaryError::parse("CONFIG_INVALID", error));
        error.request_id = recover_request_id(input.as_str());
        error
    })
}

fn admit_version(version: u32, request_id: u64) -> Result<(), SessionError> {
    if version != NATIVE_BRIDGE_ENVELOPE_VERSION {
        return Err(config_invalid(
            request_id,
            format!(
                "unsupported native bridge envelope version {version}; \
                 supported version is {NATIVE_BRIDGE_ENVELOPE_VERSION}"
            ),
        ));
    }
    Ok(())
}

fn config_invalid(request_id: u64, message: impl Into<String>) -> SessionError {
    let mut error = SessionError::new(ErrorDomain::Boundary, "CONFIG_INVALID", message);
    error.request_id = Some(request_id);
    error
}

/// Frozen mapping: the engine emits `OPERATION_RESOURCE_EXHAUSTED` only for
/// allocation/reservation failures, which preserve their code; everything
/// else keeps its existing stable code.
fn operation_error(error: OperationError) -> SessionError {
    let failure_class = if error.code == "OPERATION_RESOURCE_EXHAUSTED" {
        OperationFailureClass::AllocationOrReservation
    } else {
        OperationFailureClass::ExistingStableCode
    };
    SessionError::from_operation(error, failure_class)
}

/// A changed/unchanged typed transaction always carries a result envelope;
/// its absence is an engine invariant failure, never a silent success.
fn typed_outcome(
    request_id: u64,
    result: Option<TypedTransactionResult>,
) -> Result<NativeBridgeOutcome, SessionError> {
    result
        .map(Box::new)
        .map(NativeBridgeOutcome::Transaction)
        .ok_or_else(|| {
            operation_error(OperationError::engine_invariant_failed(
                request_id,
                None,
                "bridge transaction produced no result envelope",
            ))
        })
}

/// Per-character input filter with exact legacy `InputFilter` semantics:
/// each committed character is kept only if it matches the pattern; a fully
/// filtered commit drops the insertion entirely. The pattern arrives
/// pre-compiled from the session policy's once-per-policy cache; a cached
/// compile failure replays the identical `CONFIG_INVALID` on every request.
fn apply_input_filter(
    compiled: Option<Result<&regex::Regex, String>>,
    text: &str,
    request_id: u64,
) -> Result<Option<String>, SessionError> {
    let Some(compiled) = compiled else {
        return Ok(Some(text.to_string()));
    };
    let regex = compiled.map_err(|message| {
        config_invalid(
            request_id,
            format!("invalid input filter pattern: {message}"),
        )
    })?;
    let filtered: String = text
        .chars()
        .filter(|character| regex.is_match(&character.to_string()))
        .collect();
    Ok((!filtered.is_empty()).then_some(filtered))
}

/// Lower one (already filtered) input commit to exactly one typed
/// local-input transaction. A fully filtered commit lowers to the empty
/// skip transaction: a real engine no-op result with no reservation.
/// Shared verbatim by the bound probe so probed bounds are commit bounds.
fn lower_input(
    engine: &YrsDocumentEngine,
    request_id: u64,
    filtered: Option<String>,
) -> Result<LoweredInput, SessionError> {
    let Some(text) = filtered else {
        return Ok(LoweredInput::Transaction(TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        }));
    };
    let plan = engine
        .plan_command(request_id, TypedCommand::InsertText { text })
        .map_err(operation_error)?;
    Ok(match plan {
        CommandPlan::NotApplicable => LoweredInput::NotApplicable,
        CommandPlan::SelectionOnly(mut transaction) | CommandPlan::Transaction(mut transaction) => {
            // The bridge assigns the trusted origin: input commits are
            // local-input regardless of how the planner lowered them.
            transaction.origin = TransactionOrigin::LocalInput;
            LoweredInput::Transaction(transaction)
        }
    })
}

/// Test support for the native bridge, the collaboration outbox
/// attachment, and the reservation-before-write saturation matrices. Mirrors
/// the `session_initialization_test_support` idiom: registry-backed session
/// ids, structured `TestError` envelopes, and full session audits.
#[cfg(test)]
#[path = "native_transaction_bridge/native_bridge_test_support.rs"]
pub mod native_bridge_test_support;
