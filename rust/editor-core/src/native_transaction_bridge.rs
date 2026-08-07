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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    text: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    command: CommandEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SelectionRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    selection: SelectionEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalApiRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    base_document_revision: u64,
    #[serde(default)]
    set_json: Option<serde_json::Value>,
    #[serde(default)]
    set_html: Option<String>,
    history: HistoryModeEnvelope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HistoryModeEnvelope {
    UndoableBoundary,
    ResetAndClear,
}

impl From<HistoryModeEnvelope> for ReplacementHistory {
    fn from(mode: HistoryModeEnvelope) -> Self {
        match mode {
            HistoryModeEnvelope::UndoableBoundary => Self::UndoableBoundary,
            HistoryModeEnvelope::ResetAndClear => Self::ResetAndClear,
        }
    }
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PositionEnvelope {
    offset: u32,
    kind: OffsetKindEnvelope,
    #[serde(default)]
    affinity: Option<AffinityEnvelope>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum OffsetKindEnvelope {
    Scalar,
    Utf16,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum AffinityEnvelope {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RangeEnvelope {
    from: PositionEnvelope,
    to: PositionEnvelope,
}

impl From<PositionEnvelope> for RevisionedPosition {
    fn from(position: PositionEnvelope) -> Self {
        Self {
            offset: position.offset,
            kind: match position.kind {
                OffsetKindEnvelope::Scalar => EditorOffsetKind::Scalar,
                OffsetKindEnvelope::Utf16 => EditorOffsetKind::Utf16,
            },
            affinity: match position.affinity {
                Some(AffinityEnvelope::Before) => Affinity::Before,
                Some(AffinityEnvelope::After) => Affinity::After,
                None => DEFAULT_POSITION_AFFINITY,
            },
        }
    }
}

impl From<RangeEnvelope> for RevisionedRange {
    fn from(range: RangeEnvelope) -> Self {
        Self {
            from: range.from.into(),
            to: range.to.into(),
        }
    }
}

/// Data-only mirror of the full [`TypedCommand`] surface. The outer request
/// envelope denies unknown fields (serde cannot enforce that inside an
/// internally tagged enum, but no variant carries an origin or policy field,
/// so nothing privileged can be injected here).
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum CommandEnvelope {
    InsertText {
        text: String,
    },
    DeleteRange {
        range: RangeEnvelope,
    },
    DeleteBackward,
    ReplaceSelectionText {
        text: String,
    },
    SplitBlock,
    DeleteAndSplit,
    InsertContentJson {
        json: serde_json::Value,
    },
    InsertContentHtml {
        html: String,
    },
    ToggleMark {
        #[serde(rename = "markType")]
        mark_type: String,
    },
    SetMark {
        #[serde(rename = "markType")]
        mark_type: String,
        #[serde(default)]
        attrs: HashMap<String, serde_json::Value>,
    },
    UnsetMark {
        #[serde(rename = "markType")]
        mark_type: String,
    },
    ToggleHeading {
        level: u8,
    },
    ToggleCodeBlock,
    ToggleBlockquote,
    ApplyListType {
        #[serde(rename = "listType")]
        list_type: String,
    },
    WrapInList {
        #[serde(rename = "listType")]
        list_type: String,
        #[serde(rename = "itemType")]
        item_type: String,
    },
    UnwrapFromList,
    IndentListItem,
    OutdentListItem,
    ToggleTaskItemChecked,
    InsertNode {
        #[serde(rename = "nodeType")]
        node_type: String,
    },
    ResizeImage {
        at: PositionEnvelope,
        width: u32,
        height: u32,
    },
}

impl From<CommandEnvelope> for TypedCommand {
    fn from(command: CommandEnvelope) -> Self {
        match command {
            CommandEnvelope::InsertText { text } => Self::InsertText { text },
            CommandEnvelope::DeleteRange { range } => Self::DeleteRange {
                range: range.into(),
            },
            CommandEnvelope::DeleteBackward => Self::DeleteBackward,
            CommandEnvelope::ReplaceSelectionText { text } => Self::ReplaceSelectionText { text },
            CommandEnvelope::SplitBlock => Self::SplitBlock,
            CommandEnvelope::DeleteAndSplit => Self::DeleteAndSplit,
            CommandEnvelope::InsertContentJson { json } => Self::InsertContentJson { json },
            CommandEnvelope::InsertContentHtml { html } => Self::InsertContentHtml { html },
            CommandEnvelope::ToggleMark { mark_type } => Self::ToggleMark { mark_type },
            CommandEnvelope::SetMark { mark_type, attrs } => Self::SetMark { mark_type, attrs },
            CommandEnvelope::UnsetMark { mark_type } => Self::UnsetMark { mark_type },
            CommandEnvelope::ToggleHeading { level } => Self::ToggleHeading { level },
            CommandEnvelope::ToggleCodeBlock => Self::ToggleCodeBlock,
            CommandEnvelope::ToggleBlockquote => Self::ToggleBlockquote,
            CommandEnvelope::ApplyListType { list_type } => Self::ApplyListType { list_type },
            CommandEnvelope::WrapInList {
                list_type,
                item_type,
            } => Self::WrapInList {
                list_type,
                item_type,
            },
            CommandEnvelope::UnwrapFromList => Self::UnwrapFromList,
            CommandEnvelope::IndentListItem => Self::IndentListItem,
            CommandEnvelope::OutdentListItem => Self::OutdentListItem,
            CommandEnvelope::ToggleTaskItemChecked => Self::ToggleTaskItemChecked,
            CommandEnvelope::InsertNode { node_type } => Self::InsertNode { node_type },
            CommandEnvelope::ResizeImage { at, width, height } => Self::ResizeImage {
                at: at.into(),
                width,
                height,
            },
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SelectionEnvelope {
    Text {
        anchor: PositionEnvelope,
        head: PositionEnvelope,
    },
    Node {
        at: PositionEnvelope,
    },
    All,
}

impl From<SelectionEnvelope> for SelectionInput {
    fn from(selection: SelectionEnvelope) -> Self {
        match selection {
            SelectionEnvelope::Text { anchor, head } => Self::Text {
                anchor: anchor.into(),
                head: head.into(),
            },
            SelectionEnvelope::Node { at } => Self::Node { at: at.into() },
            SelectionEnvelope::All => Self::All,
        }
    }
}

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
        let transaction = TypedTransaction {
            request_id,
            base_document_revision: envelope.base_document_revision,
            origin: TransactionOrigin::LocalInput,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(envelope.selection.into()),
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
pub mod native_bridge_test_support {
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
                .ack_lease(
                    crate::collaboration_runtime::outbox::OutboundLeaseId::from_value(lease_id),
                )
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
                LoweredInput::Transaction(transaction) if transaction.operations.is_empty() => {
                    Ok(None)
                }
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
                outbox_pending_updates: outbox
                    .map(crate::collaboration_runtime::CollaborationOutbox::pending_document_update_count),
                outbox_pending_bytes: outbox
                    .map(crate::collaboration_runtime::CollaborationOutbox::pending_document_update_bytes),
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
}
