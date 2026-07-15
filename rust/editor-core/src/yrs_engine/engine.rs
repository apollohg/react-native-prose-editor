use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;
use yrs::sync::time::{Clock, SystemClock};
use yrs::types::xml::XmlFragmentRef;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::Update;
use yrs::{Doc, OffsetKind, Options, ReadTxn, StateVector, Transact, WriteTxn};

use crate::boundary::{BoundedInput, InputKind, ResourceLimits};
use crate::model::Document;
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::{schema_fingerprint, NodeRole, Schema};
use crate::selection::Selection;
use crate::serialize::{
    from_html_with_limits, from_prosemirror_json_with_limits, rehydrate_reserved_html_opaque,
    to_html, to_prosemirror_json, FromHtmlOptions, JsonParseError, ParseError, UnknownTypeMode,
};
use crate::transform::{
    canonicalize_yrs_document, validate_canonical_marks, DocumentValidator, StepMap,
};

use super::compiler::{
    compile_transaction_with_yrs_and_stored_marks, map_position, selectable_void_at,
    CompilationContext, CompiledTransaction,
};
use super::compiler::{
    RelativeSelectionPlan, SelectionPlan, StoredMarksCompilationContext, StoredMarksPlan,
};
use super::derived_state::{
    exact_point_is_representable, operation_result_to_relative,
    stored_marks_after_selection_change, DerivedStateCache,
};
use super::mutation::{execute_mutation_plan, preflight_mutation_plan, YrsMutationPlan};
use super::update_preflight::preflight_update_v1;
use super::{
    DocumentScope, DocumentSnapshot, EditingLimits, TransactionOrigin, YrsDocumentCodec,
    YrsEngineError, YrsEngineResult, SNAPSHOT_FORMAT_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitializationMode {
    LocalEmpty,
    AwaitRemote,
}

#[derive(Debug, Clone)]
pub struct YrsEngineConfig {
    pub schema: Schema,
    pub fragment_name: String,
    pub initialization_mode: InitializationMode,
    pub resource_limits: ResourceLimits,
    pub editing_limits: EditingLimits,
    pub max_length: Option<u32>,
    pub scope: Option<DocumentScope>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineCommit {
    pub changed: bool,
    pub revision: u64,
}

fn selection_requires_fallback_proof<T: ReadTxn>(
    plan: &YrsMutationPlan,
    txn: &T,
    fragment: &XmlFragmentRef,
    selection: &super::RelativeSelection,
) -> bool {
    match selection {
        super::RelativeSelection::Text { anchor, head } => {
            plan.removes_sticky_branch(txn, fragment, &anchor.sticky)
                || plan.removes_sticky_branch(txn, fragment, &head.sticky)
        }
        super::RelativeSelection::Node { point } => {
            plan.removes_sticky_branch(txn, fragment, &point.sticky)
        }
        super::RelativeSelection::All => false,
    }
}

struct FallbackProofContext<'a, Current, Proof> {
    plan: &'a YrsMutationPlan,
    current_txn: &'a Current,
    current_fragment: &'a XmlFragmentRef,
    proof_txn: &'a Proof,
    proof_fragment: &'a XmlFragmentRef,
    schema: &'a Schema,
}

fn required_fallbacks_are_representable<Current: ReadTxn, Proof: ReadTxn>(
    context: FallbackProofContext<'_, Current, Proof>,
    selection: &Selection,
    relative: &super::RelativeSelection,
) -> bool {
    let FallbackProofContext {
        plan,
        current_txn,
        current_fragment,
        proof_txn,
        proof_fragment,
        schema,
    } = context;
    let point_is_valid = |position, point: &super::RelativePoint| {
        !plan.removes_sticky_branch(current_txn, current_fragment, &point.sticky)
            || exact_point_is_representable(proof_txn, proof_fragment, position, point, schema)
    };
    match (selection, relative) {
        (
            Selection::Text { anchor, head },
            super::RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            },
        ) => point_is_valid(*anchor, relative_anchor) && point_is_valid(*head, relative_head),
        (Selection::Node { pos }, super::RelativeSelection::Node { point }) => {
            point_is_valid(*pos, point)
        }
        (Selection::All, super::RelativeSelection::All) => true,
        _ => false,
    }
}

enum EngineDocumentState {
    AwaitingRemote,
    Ready {
        document: Document,
        canonical_json: serde_json::Value,
    },
}

struct CandidateDocument {
    doc: Doc,
    state: EngineDocumentState,
    durable_client_ids: HashSet<u64>,
}

struct ValidatedImportDocument {
    document: Document,
    canonical_json: serde_json::Value,
}

impl ValidatedImportDocument {
    fn new(
        document: Document,
        schema: &Schema,
        resource_limits: &ResourceLimits,
    ) -> YrsEngineResult<Self> {
        if contains_reserved_public_json_forge(document.root()) {
            return Err(candidate_invariant_parse_error(
                "public JSON cannot construct reserved opaque HTML metadata",
                "candidate codec round-trip changed the document",
            ));
        }
        validate_yrs_mark_representation(&document, schema)?;
        validate_import_document(&document, schema, resource_limits)?;
        let document = canonicalize_yrs_document(&document, schema);
        let canonical_json = to_prosemirror_json(&document, schema);
        Ok(Self {
            document,
            canonical_json,
        })
    }
}

fn contains_reserved_public_json_forge(node: &crate::model::Node) -> bool {
    if node.node_type() == "__opaque_json"
        && node
            .attrs()
            .get("original_type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|node_type| matches!(node_type, "__opaque" | "__opaque_json" | "__skip"))
    {
        return true;
    }
    node.content()
        .is_some_and(|content| content.iter().any(contains_reserved_public_json_forge))
}

pub struct YrsDocumentEngine {
    doc: Doc,
    fragment_name: String,
    schema: Schema,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
    scope: Option<DocumentScope>,
    schema_fingerprint: String,
    derived_state: Option<DerivedStateCache>,
    revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    last_committed_origin: Option<TransactionOrigin>,
    durable_client_ids: HashSet<u64>,
    /// Dependency-pending standard updates are quarantined outside the live
    /// authoritative Doc until their complete merged state can be validated.
    quarantined_remote_update: Option<Vec<u8>>,
    history: super::history::YrsHistory,
}

impl YrsDocumentEngine {
    pub fn new(config: YrsEngineConfig) -> YrsEngineResult<Self> {
        Self::new_with_history_clock(config, Arc::new(SystemClock))
    }

    pub fn new_with_history_clock(
        config: YrsEngineConfig,
        history_clock: Arc<dyn Clock>,
    ) -> YrsEngineResult<Self> {
        let YrsEngineConfig {
            schema,
            fragment_name,
            initialization_mode,
            resource_limits,
            editing_limits,
            max_length,
            scope,
        } = config;
        resource_limits.validate()?;
        editing_limits.validate()?;
        validate_config_metadata(&fragment_name, scope.as_ref(), &resource_limits)?;
        let schema_fingerprint = schema_fingerprint(&schema);
        let candidate = match initialization_mode {
            InitializationMode::LocalEmpty => {
                build_local_empty_candidate(&schema, &fragment_name, &resource_limits)?
            }
            InitializationMode::AwaitRemote => {
                build_await_remote_candidate(&fragment_name, &resource_limits)?
            }
        };
        let derived_state =
            build_derived_state_for_candidate(&candidate, &schema, &fragment_name, 0, 0)?;
        let history_fragment = {
            let txn = candidate.doc.transact();
            txn.get_xml_fragment(fragment_name.as_str())
                .ok_or_else(|| {
                    YrsEngineError::new(
                        "CODEC_INVARIANT_FAILED",
                        "initialized Yrs fragment is missing while binding history",
                    )
                })?
        };
        let history = super::history::YrsHistory::new(
            &candidate.doc,
            &history_fragment,
            editing_limits.clone(),
            resource_limits.max_encoded_state_bytes,
            history_clock,
        );

        Ok(Self {
            doc: candidate.doc,
            fragment_name,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            scope,
            schema_fingerprint,
            derived_state,
            revision: 0,
            state_revision: 0,
            yrs_state_epoch: 0,
            last_committed_origin: None,
            durable_client_ids: candidate.durable_client_ids,
            quarantined_remote_update: None,
            history,
        })
    }

    pub fn is_ready(&self) -> bool {
        self.derived_state.is_some()
    }

    pub fn plan_command(
        &self,
        request_id: u64,
        command: super::TypedCommand,
    ) -> super::OperationResult<super::CommandPlan> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        super::commands::plan(
            super::commands::PlanningContext {
                request_id,
                revision: self.revision,
                document: &state.document,
                position_map: &state.position_map,
                selection: &state.resolved_selection,
                stored_marks: state.stored_marks.as_deref(),
                schema: &self.schema,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                max_length: self.max_length,
            },
            command,
        )
    }

    pub fn apply_command(
        &mut self,
        request_id: u64,
        command: super::TypedCommand,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        match self.plan_command(request_id, command)? {
            super::CommandPlan::NotApplicable => Ok(None),
            super::CommandPlan::Transaction(transaction)
            | super::CommandPlan::SelectionOnly(transaction) => self
                .apply_typed_transaction_with_result(transaction)
                .map(Some),
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TransactionCommit>> {
        Ok(self
            .apply_history_pop(request_id, true, false)?
            .map(|(commit, _)| commit))
    }

    pub fn redo(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TransactionCommit>> {
        Ok(self
            .apply_history_pop(request_id, false, false)?
            .map(|(commit, _)| commit))
    }

    pub fn undo_with_result(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        Ok(self
            .apply_history_pop(request_id, true, true)?
            .and_then(|(_, result)| result))
    }

    pub fn redo_with_result(
        &mut self,
        request_id: u64,
    ) -> super::OperationResult<Option<super::TypedTransactionResult>> {
        Ok(self
            .apply_history_pop(request_id, false, true)?
            .and_then(|(_, result)| result))
    }

    /// Merge one standard Yjs/Yrs Update-v1 into the authoritative document.
    ///
    /// The update is fully decoded, applied, derived, validated, and admitted
    /// on an isolated candidate before the live CRDT is opened for mutation.
    /// Remote-origin structs remain outside the local undo scope.
    pub fn apply_remote_update_v1(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> super::OperationResult<EngineCommit> {
        let admitted_epoch = self.yrs_state_epoch;
        if update.len() > self.resource_limits.max_encoded_state_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxEncodedStateBytes",
                self.resource_limits.max_encoded_state_bytes as u64,
                update.len() as u64,
            ));
        }
        preflight_update_v1(update, &self.resource_limits)
            .map_err(|error| remote_ingress_error(request_id, error))?;
        let incoming_update = Update::decode_v1(update).map_err(|error| {
            super::OperationError::document_invalid(
                request_id,
                None,
                "update",
                format!("invalid Update-v1: {error}"),
            )
        })?;
        let (candidate_update, merged_update_bytes) = if let Some(quarantined) =
            self.quarantined_remote_update.as_deref()
        {
            let combined_len = quarantined.len().checked_add(update.len()).ok_or_else(|| {
                super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxEncodedStateBytes",
                    self.resource_limits.max_encoded_state_bytes as u64,
                    u64::MAX,
                )
            })?;
            if combined_len > self.resource_limits.max_encoded_state_bytes {
                return Err(super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxEncodedStateBytes",
                    self.resource_limits.max_encoded_state_bytes as u64,
                    combined_len as u64,
                ));
            }
            let quarantined_update = Update::decode_v1(quarantined).map_err(|error| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    format!("quarantined Update-v1 cannot decode: {error}"),
                )
            })?;
            let merged = Update::merge_updates(vec![quarantined_update, incoming_update]);
            let merged_bytes = merged.encode_v1();
            if merged_bytes.len() > self.resource_limits.max_encoded_state_bytes {
                return Err(super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxEncodedStateBytes",
                    self.resource_limits.max_encoded_state_bytes as u64,
                    merged_bytes.len() as u64,
                ));
            }
            preflight_update_v1(&merged_bytes, &self.resource_limits)
                .map_err(|error| remote_ingress_error(request_id, error))?;
            (merged, Some(merged_bytes))
        } else {
            (incoming_update, None)
        };
        let current_encoded = encode_state_bounded(&self.doc, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        let candidate_doc = fresh_utf16_doc_excluding(&self.durable_client_ids, self.client_id());
        {
            let mut txn =
                candidate_doc.transact_mut_with(TransactionOrigin::RemoteSync.as_yrs_origin());
            if !current_encoded.is_empty() {
                let current = Update::decode_v1(&current_encoded).map_err(|error| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("current encoded state cannot decode: {error}"),
                    )
                })?;
                txn.apply_update(current).map_err(|error| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("candidate cannot seed current state: {error}"),
                    )
                })?;
            }
            txn.apply_update(candidate_update).map_err(|error| {
                super::OperationError::document_invalid(
                    request_id,
                    None,
                    "update",
                    format!("candidate rejected Update-v1: {error}"),
                )
            })?;
        }
        let candidate_has_missing_updates = {
            let txn = candidate_doc.transact();
            txn.has_missing_updates()
        };
        if candidate_has_missing_updates {
            let quarantined = if let Some(merged) = merged_update_bytes {
                merged
            } else {
                let mut admitted = Vec::new();
                admitted.try_reserve_exact(update.len()).map_err(|error| {
                    super::OperationError::operation_resource_exhausted(
                        request_id,
                        "remoteUpdate",
                        format!("cannot reserve quarantined remote update: {error}"),
                    )
                })?;
                admitted.extend_from_slice(update);
                admitted
            };
            self.quarantined_remote_update = Some(quarantined);
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }
        // Temporarily remove the completed dependency set while its document
        // content is validated. Invalid/over-limit content is poison and stays
        // discarded; once content admission succeeds, retain these bytes until
        // every fallible operational reservation has completed.
        let completed_quarantine = self.quarantined_remote_update.take();
        let candidate_encoded =
            encode_candidate_state_bounded(&candidate_doc, &self.resource_limits)
                .map_err(|error| history_operation_error(request_id, error))?;
        if candidate_encoded == current_encoded {
            self.quarantined_remote_update = None;
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }

        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let candidate_json = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::document_invalid(
                        request_id,
                        None,
                        "update",
                        "remote update does not contain the configured document fragment",
                    )
                })?;
            codec
                .read_json(&fragment, &txn)
                .map_err(|error| remote_engine_error(request_id, error))?
        };
        let candidate_document = from_prosemirror_json_with_limits(
            &candidate_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| remote_json_error(request_id, error))?;
        let candidate_document = rehydrate_reserved_html_opaque(&candidate_document);
        DocumentValidator::validate(&candidate_document, &self.schema, &self.resource_limits)
            .map_err(|error| remote_validation_error(request_id, error))?;
        if let Some(limit) = self.max_length {
            let actual = candidate_document.root().text_content().chars().count();
            if actual > limit as usize {
                return Err(super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual as u64,
                ));
            }
        }
        let canonical_json = to_prosemirror_json(&candidate_document, &self.schema);
        let canonical_bytes = serde_json::to_vec(&canonical_json).map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("remote document serialization failed: {error}"),
            )
        })?;
        if canonical_bytes.len() > self.editing_limits.max_derived_output_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(canonical_bytes.len()).unwrap_or(u64::MAX),
            ));
        }
        self.quarantined_remote_update = completed_quarantine;
        let next_revision =
            checked_operation_increment(request_id, self.revision, "documentRevision")?;
        let next_state_revision =
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?;
        let next_epoch =
            checked_operation_increment(request_id, self.yrs_state_epoch, "yrsStateEpoch")?;
        let next_state = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "validated remote candidate lost its document fragment",
                    )
                })?;
            if let Some(current) = self.derived_state.as_ref() {
                let fallback = affinity_aware_mapped_selection(
                    &current.legacy_selection(),
                    &current.relative_selection,
                    &StepMap::empty(),
                    &candidate_document,
                    &self.schema,
                );
                let mut next = current
                    .after_document_change(
                        candidate_document.clone(),
                        canonical_json.clone(),
                        &txn,
                        &fragment,
                        &self.schema,
                        &StepMap::empty(),
                        UpdateMode::Rebuild,
                        &[],
                        None,
                        Some(&fallback),
                        false,
                        next_revision,
                        next_state_revision,
                    )
                    .ok_or_else(|| {
                        super::OperationError::selection_position_invalid(
                            request_id,
                            "selection",
                            "local relative selection cannot resolve after remote update",
                        )
                    })?;
                next.stored_marks = match (&current.resolved_selection, &next.resolved_selection) {
                    (
                        super::ResolvedSelection::Text {
                            anchor: current_anchor,
                            head: current_head,
                        },
                        super::ResolvedSelection::Text {
                            anchor: next_anchor,
                            head: next_head,
                        },
                    ) if current.relative_selection == next.relative_selection
                        && current_anchor.document == current_head.document
                        && next_anchor.document == next_head.document =>
                    {
                        current.stored_marks.clone()
                    }
                    _ => stored_marks_after_selection_change(
                        current.stored_marks.as_deref(),
                        &current.resolved_selection,
                        &next.resolved_selection,
                        &next.document,
                        &self.schema,
                    ),
                };
                next
            } else {
                DerivedStateCache::initialize(
                    candidate_document.clone(),
                    canonical_json.clone(),
                    &txn,
                    &fragment,
                    &self.schema,
                    next_revision,
                    next_state_revision,
                )
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "remote candidate cannot initialize derived state",
                    )
                })?
            }
        };
        let durable_client_ids = {
            let txn = candidate_doc.transact();
            txn.state_vector()
                .iter()
                .map(|(client, _)| client.get())
                .collect::<HashSet<_>>()
        };
        let accepted_update = {
            let current_state_vector = self.doc.transact().state_vector();
            candidate_doc
                .transact()
                .encode_state_as_update_v1(&current_state_vector)
        };
        preflight_update_v1(&accepted_update, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        let live_update = Update::decode_v1(&accepted_update).map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("candidate-produced incremental update cannot decode: {error}"),
            )
        })?;
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            request_id,
            super::compiler::AtomicFailpoint::RemoteHistoryAdmission,
        )?;
        // Replay retention charges remote work in encoded-byte units: this is
        // the exact admitted incremental payload, not redundant caller input.
        let replay_byte_units = u64::try_from(accepted_update.len())
            .unwrap_or(u64::MAX)
            .max(1);
        let origin = self.history.prepare_excluded(
            request_id,
            TransactionOrigin::RemoteSync,
            replay_byte_units,
            &current_encoded,
            accepted_update.len(),
        )?;
        assert_eq!(
            admitted_epoch, self.yrs_state_epoch,
            "remote update engine state changed during candidate admission"
        );
        {
            let mut txn = self.doc.transact_mut_with(origin);
            txn.apply_update(live_update)
                .expect("candidate-proved remote update must apply to identical live state");
        }
        self.history.finish_excluded(accepted_update);
        self.quarantined_remote_update = None;
        self.derived_state = Some(next_state);
        self.durable_client_ids = durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_epoch;
        self.last_committed_origin = Some(TransactionOrigin::RemoteSync);
        Ok(EngineCommit {
            changed: true,
            revision: self.revision,
        })
    }

    fn apply_history_pop(
        &mut self,
        request_id: u64,
        undoing: bool,
        with_result: bool,
    ) -> super::OperationResult<
        Option<(
            super::TransactionCommit,
            Option<super::TypedTransactionResult>,
        )>,
    > {
        if if undoing {
            !self.history.can_undo()
        } else {
            !self.history.can_redo()
        } {
            return Ok(None);
        }

        let next_document_revision =
            checked_operation_increment(request_id, self.revision, "documentRevision")?;
        let next_state_revision =
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?;
        let next_yrs_state_epoch =
            checked_operation_increment(request_id, self.yrs_state_epoch, "yrsStateEpoch")?;

        let action = if undoing {
            super::history::HistoryAction::Undo
        } else {
            super::history::HistoryAction::Redo
        };
        let candidate_doc = self.new_history_candidate_doc();
        self.history.seed_candidate(request_id, &candidate_doc)?;
        let candidate_fragment =
            candidate_doc.get_or_insert_xml_fragment(self.fragment_name.as_str());
        let mut candidate_history =
            self.history
                .replay_into(request_id, &candidate_doc, &candidate_fragment)?;
        let candidate_pop = match action {
            super::history::HistoryAction::Undo => candidate_history.undo(),
            super::history::HistoryAction::Redo => candidate_history.redo(),
        };
        if !candidate_pop.changed {
            return Err(super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "bounded history replay cannot reproduce the next live pop",
            ));
        }
        let restored_slot = candidate_pop.restored.as_ref().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "changed history candidate supplied no restoration metadata",
            )
        })?;
        let restored = restored_slot.get().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "changed history candidate supplied an unsealed restoration snapshot",
            )
        })?;
        let (candidate_state, candidate_encoded_state) = self.derive_history_candidate_state(
            request_id,
            &candidate_doc,
            restored,
            next_document_revision,
            next_state_revision,
        )?;

        let mut result = with_result
            .then(|| self.prepare_history_result(request_id, &candidate_state))
            .transpose()?;

        candidate_history.accept_action(request_id, action, candidate_encoded_state)?;
        if let Some(result) = &mut result {
            result.history_state = crate::editor_state::HistoryState {
                can_undo: candidate_history.can_undo(),
                can_redo: candidate_history.can_redo(),
            };
        }
        self.doc = candidate_doc;
        self.history = candidate_history;
        self.derived_state = Some(candidate_state);
        self.revision = next_document_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(TransactionOrigin::UndoRedo);
        Ok(Some((
            super::TransactionCommit {
                request_id,
                changed: true,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: TransactionOrigin::UndoRedo,
            },
            result,
        )))
    }

    fn prepare_history_result(
        &self,
        request_id: u64,
        candidate: &DerivedStateCache,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let selection = candidate.resolved_selection.clone();
        let legacy_selection = candidate.legacy_selection();
        let commands = crate::editor_state::command_applicability(
            &candidate.document,
            &self.schema,
            &legacy_selection,
            &self.resource_limits,
        );
        let active_state = crate::editor_state::active_state(
            &candidate.document,
            &self.schema,
            &legacy_selection,
            candidate.stored_marks.as_deref(),
            commands,
            &self.resource_limits,
        );
        let render_update = match crate::render::incremental::safe_contiguous_render_blocks_patch(
            &current.document,
            &candidate.document,
            &self.schema,
            &[],
        ) {
            Ok(Some(patch)) => super::RenderUpdate::Patch(patch),
            Ok(None) => super::RenderUpdate::None,
            Err(full) => super::RenderUpdate::Full(full),
        };
        let result = super::TypedTransactionResult {
            request_id,
            origin: TransactionOrigin::UndoRedo,
            changed: true,
            document_revision: candidate.document_revision,
            state_revision: candidate.state_revision,
            selection,
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update,
        };
        self.admit_typed_result(request_id, &result)?;
        Ok(result)
    }

    fn new_history_candidate_doc(&self) -> Doc {
        Doc::with_options(Options {
            client_id: self.doc.client_id(),
            guid: self.doc.guid(),
            offset_kind: OffsetKind::Utf16,
            // History StackItems reference deleted structs which are present in
            // the full update but whose live keep flags are not encoded.
            skip_gc: true,
            ..Options::default()
        })
    }

    fn derive_history_candidate_state(
        &self,
        request_id: u64,
        doc: &Doc,
        restored: &super::history::HistoryLocalState,
        document_revision: u64,
        state_revision: u64,
    ) -> super::OperationResult<(DerivedStateCache, Vec<u8>)> {
        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let txn = doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history result fragment is missing",
                )
            })?;
        let derived_json = codec
            .read_json(&fragment, &txn)
            .map_err(|error| history_operation_error(request_id, error))?;
        let document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| {
            super::OperationError::document_invalid(request_id, None, "document", error.to_string())
        })?;
        let document =
            canonicalize_yrs_document(&rehydrate_reserved_html_opaque(&document), &self.schema);
        DocumentValidator::validate(&document, &self.schema, &self.resource_limits).map_err(
            |error| {
                if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                    super::OperationError::document_limit_exceeded(
                        request_id,
                        None,
                        "document",
                        error.limit.unwrap_or(0) as u64,
                        error.actual.unwrap_or(0) as u64,
                    )
                } else {
                    super::OperationError::document_invalid(
                        request_id,
                        None,
                        "document",
                        error.to_string(),
                    )
                }
            },
        )?;
        if let Some(limit) = self.max_length {
            let actual = document.root().text_content().chars().count() as u64;
            if actual > u64::from(limit) {
                return Err(super::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual,
                ));
            }
        }
        crate::transform::validate_input_mark_set(
            restored.stored_marks.as_deref().unwrap_or_default(),
            &self.schema,
        )
        .map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("history metadata contains invalid stored marks: {error}"),
            )
        })?;
        let canonical_json = to_prosemirror_json(&document, &self.schema);
        let canonical_bytes = serde_json::to_vec(&canonical_json).map_err(|error| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("history result serialization failed: {error}"),
            )
        })?;
        if canonical_bytes.len() > self.editing_limits.max_derived_output_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(canonical_bytes.len()).unwrap_or(u64::MAX),
            ));
        }
        let canonical_fingerprint: [u8; 32] = Sha256::digest(&canonical_bytes).into();
        let base = DerivedStateCache::initialize(
            document,
            canonical_json,
            &txn,
            &fragment,
            &self.schema,
            document_revision,
            state_revision,
        )
        .ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "history result cannot initialize derived editor state",
            )
        })?;
        // Yrs recreates inserted structs with new IDs during redo, so the
        // original relative cursor can remain valid yet resolve beside the
        // redone content. Preserve the document-relative snapshot as the CRDT
        // metadata and reseal it from the exact resolved fallback on restore.
        let restored_relative = if canonical_fingerprint == restored.canonical_fingerprint {
            operation_result_to_relative(
                &txn,
                &fragment,
                &super::derived_state::resolved_to_legacy(&restored.resolved_selection),
                &self.schema,
            )
        } else {
            restored.relative_selection.clone()
        };
        let mut state = base
            .with_relative_selection(
                restored_relative,
                &txn,
                &fragment,
                &self.schema,
                state_revision,
            )
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history restoration selection cannot resolve",
                )
            })?;
        state.stored_marks = restored
            .stored_marks
            .as_deref()
            .map(|marks| super::derived_state::canonical_marks(marks, &self.schema));
        drop(txn);
        let encoded = encode_state_bounded(doc, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        Ok((state, encoded))
    }

    pub fn document(&self) -> Option<&Document> {
        self.debug_assert_derived_revision_keys();
        let state = self.derived_state.as_ref()?;
        Some(&state.document)
    }

    pub fn document_json(&self) -> Option<serde_json::Value> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .map(|state| state.canonical_json.clone())
    }

    pub fn document_html(&self) -> Option<String> {
        self.document()
            .map(|document| to_html(document, &self.schema))
    }

    pub fn encoded_state(&self) -> YrsEngineResult<Vec<u8>> {
        encode_state_bounded(&self.doc, &self.resource_limits)
    }

    pub fn revision(&self) -> u64 {
        self.debug_assert_derived_revision_keys();
        self.revision
    }

    pub fn state_revision(&self) -> u64 {
        self.debug_assert_derived_revision_keys();
        self.state_revision
    }

    pub fn position_map(&self) -> Option<&PositionMap> {
        self.debug_assert_derived_revision_keys();
        self.derived_state.as_ref().map(|state| &state.position_map)
    }

    pub fn relative_selection(&self) -> Option<&super::RelativeSelection> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .map(|state| &state.relative_selection)
    }

    pub fn resolved_selection(&self) -> Option<&super::ResolvedSelection> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .map(|state| &state.resolved_selection)
    }

    pub fn stored_marks(&self) -> Option<&[crate::model::Mark]> {
        self.debug_assert_derived_revision_keys();
        self.derived_state
            .as_ref()
            .and_then(|state| state.stored_marks.as_deref())
    }

    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    pub fn fragment_name(&self) -> &str {
        &self.fragment_name
    }

    pub fn schema_fingerprint(&self) -> &str {
        &self.schema_fingerprint
    }

    pub fn scope(&self) -> Option<&DocumentScope> {
        self.scope.as_ref()
    }

    pub fn last_committed_origin(&self) -> Option<TransactionOrigin> {
        self.last_committed_origin
    }

    pub fn resource_limits(&self) -> &ResourceLimits {
        &self.resource_limits
    }

    pub fn editing_limits(&self) -> &EditingLimits {
        &self.editing_limits
    }

    pub fn max_length(&self) -> Option<u32> {
        self.max_length
    }

    fn debug_assert_derived_revision_keys(&self) {
        if let Some(state) = &self.derived_state {
            debug_assert_eq!(state.document_revision, self.revision);
            debug_assert_eq!(state.state_revision, self.state_revision);
            debug_assert_eq!(
                state.document_node_count,
                crate::editor_state::document_node_count(state.document.root())
            );
        }
    }

    #[allow(dead_code)] // Task 7 exposes the internal compiler through atomic application.
    pub(crate) fn compile_typed_transaction(
        &self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<CompiledTransaction> {
        let document = self
            .document()
            .ok_or_else(|| super::OperationError::engine_not_ready(transaction.request_id))?;
        let state = self.derived_state.as_ref().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "ready Yrs engine has no derived state",
            )
        })?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let current_selection = self
            .derived_state
            .as_ref()
            .map(DerivedStateCache::legacy_selection);
        let current_relative_selection = self.relative_selection().cloned();
        let mut compiled = compile_transaction_with_yrs_and_stored_marks(
            CompilationContext {
                document,
                selection: current_selection.as_ref(),
                schema: &self.schema,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                document_revision: self.revision,
                max_length: self.max_length,
            },
            transaction,
            &txn,
            &fragment,
            StoredMarksCompilationContext {
                stored_marks: state.stored_marks.as_deref(),
                resolved_selection: &state.resolved_selection,
                relative_selection: &state.relative_selection,
            },
        )?;
        if let (
            Some(selection),
            Some(relative),
            SelectionPlan::Mapped(_),
            RelativeSelectionPlan::PreserveWithFallback(fallback),
        ) = (
            current_selection.as_ref(),
            current_relative_selection.as_ref(),
            &compiled.selection_plan,
            &mut compiled.relative_selection_plan,
        ) {
            *fallback = affinity_aware_mapped_selection(
                selection,
                relative,
                &compiled.composed_map,
                &compiled.preview,
                &self.schema,
            );
        }
        if let RelativeSelectionPlan::Precomputed { relative, fallback } =
            &compiled.relative_selection_plan
        {
            if compiled.preview != *document
                && selection_requires_fallback_proof(
                    &compiled.mutation_plan,
                    &txn,
                    &fragment,
                    relative,
                )
            {
                let proof_source = ValidatedImportDocument {
                    document: compiled.preview.clone(),
                    canonical_json: compiled.canonical_json.clone().ok_or_else(|| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "changed explicit selection preview has no canonical JSON",
                        )
                    })?,
                };
                let proof = self
                    .build_candidate_from_document(proof_source, compiled.origin)
                    .map_err(|error| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            format!("cannot prove committed selection representation: {error}"),
                        )
                    })?;
                let proof_txn = proof.doc.transact();
                let proof_fragment = proof_txn
                    .get_xml_fragment(self.fragment_name.as_str())
                    .ok_or_else(|| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "selection proof candidate fragment is missing",
                        )
                    })?;
                if !required_fallbacks_are_representable(
                    FallbackProofContext {
                        plan: &compiled.mutation_plan,
                        current_txn: &txn,
                        current_fragment: &fragment,
                        proof_txn: &proof_txn,
                        proof_fragment: &proof_fragment,
                        schema: &self.schema,
                    },
                    fallback,
                    relative,
                ) {
                    return Err(super::OperationError::selection_position_invalid(
                        compiled.request_id,
                        "selection",
                        "mapped selection cannot preserve the requested Yrs affinity",
                    ));
                }
            }
        }
        compiled.yrs_state_epoch = self.yrs_state_epoch;
        Ok(compiled)
    }

    pub fn apply_typed_transaction(
        &mut self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<super::TransactionCommit> {
        if transaction.operations.is_empty()
            && transaction.history_policy == super::HistoryPolicy::Skip
        {
            return self
                .apply_empty_skip_transaction(transaction, false)
                .map(|(commit, _)| commit);
        }
        let compiled = self.compile_typed_transaction(transaction)?;
        let (commit, _) = self.apply_compiled_transaction(compiled, false)?;
        Ok(commit)
    }

    fn apply_empty_skip_transaction(
        &mut self,
        transaction: super::TypedTransaction,
        with_result: bool,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        debug_assert!(transaction.operations.is_empty());
        debug_assert_eq!(transaction.history_policy, super::HistoryPolicy::Skip);
        let request_id = transaction.request_id;
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let context = CompilationContext {
            document: &current.document,
            selection: None,
            schema: &self.schema,
            resource_limits: &self.resource_limits,
            editing_limits: &self.editing_limits,
            document_revision: self.revision,
            max_length: self.max_length,
        };
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            request_id,
            super::compiler::AtomicFailpoint::EnvelopeAdmission,
        )?;
        let admitted_input_bytes =
            super::compiler::admit_transaction_envelope(context, &transaction)?;
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            request_id,
            super::compiler::AtomicFailpoint::SemanticCompilation,
        )?;

        let txn = self.doc.transact();
        super::compiler::admit_yrs_scan_work(
            request_id,
            admitted_input_bytes,
            current.document_text_bytes,
            &txn,
            &self.resource_limits,
        )?;
        let needs_rendered_text = match &transaction.selection_intent {
            super::SelectionIntent::Set(super::SelectionInput::Text { anchor, head }) => {
                anchor.kind == super::EditorOffsetKind::Utf16
                    || head.kind == super::EditorOffsetKind::Utf16
            }
            super::SelectionIntent::Set(super::SelectionInput::Node { at }) => {
                at.kind == super::EditorOffsetKind::Utf16
            }
            _ => false,
        };
        let rendered_text = if needs_rendered_text {
            current.rendered_text.as_str()
        } else {
            ""
        };
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let resolve_point = |field: &'static str,
                             point: super::RevisionedPosition|
         -> super::OperationResult<u32> {
            super::position::editor_offset_to_doc_pos(
                point.offset,
                point.kind,
                rendered_text,
                &current.position_map,
                &current.document,
            )
            .ok_or_else(|| {
                super::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    format!("{field} is outside the base document"),
                )
            })
        };
        let relative_point = |field: &'static str,
                              point: super::RevisionedPosition|
         -> super::OperationResult<super::RelativePoint> {
            let document_position = resolve_point(field, point)?;
            super::position::doc_pos_to_relative_point(
                &txn,
                &fragment,
                document_position,
                point.affinity,
                &self.schema,
            )
            .ok_or_else(|| {
                super::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    "selection cannot be represented with the requested Yrs affinity",
                )
            })
        };
        let next_relative = match &transaction.selection_intent {
            super::SelectionIntent::Preserve | super::SelectionIntent::UseOperationResult => {
                current.relative_selection.clone()
            }
            super::SelectionIntent::Set(super::SelectionInput::Text { anchor, head }) => {
                let anchor_document = resolve_point("selection.anchor", *anchor)?;
                let head_document = resolve_point("selection.head", *head)?;
                let normalized = Selection::text(anchor_document, head_document)
                    .normalized(&current.document, &current.position_map);
                debug_assert!(matches!(normalized, Selection::Text { .. }));
                super::RelativeSelection::Text {
                    anchor: relative_point("selection.anchor", *anchor)?,
                    head: relative_point("selection.head", *head)?,
                }
            }
            super::SelectionIntent::Set(super::SelectionInput::Node { at }) => {
                let document_position = resolve_point("selection.at", *at)?;
                let normalized = Selection::node(document_position)
                    .normalized(&current.document, &current.position_map);
                let Selection::Node { pos } = normalized else {
                    return Err(super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "node selection did not compile to a node selection",
                    ));
                };
                if !selectable_void_at(current.document.root(), pos, 0, &self.schema) {
                    return Err(super::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                super::RelativeSelection::Node {
                    point: relative_point("selection.at", *at)?,
                }
            }
            super::SelectionIntent::Set(super::SelectionInput::All) => {
                super::RelativeSelection::All
            }
        };
        let next_selection = current
            .resolve_relative_selection(&next_relative, &txn, &fragment, &self.schema)
            .ok_or_else(|| {
                super::OperationError::selection_position_invalid(
                    request_id,
                    "selection",
                    "selection cannot be represented in the Yrs document",
                )
            })?;
        drop(txn);
        let next_stored_marks = stored_marks_after_selection_change(
            current.stored_marks.as_deref(),
            &current.resolved_selection,
            &next_selection,
            &current.document,
            &self.schema,
        );
        let changed = next_relative != current.relative_selection
            || next_selection != current.resolved_selection
            || next_stored_marks != current.stored_marks;
        let next_state_revision = if changed {
            checked_operation_increment(request_id, self.state_revision, "stateRevision")?
        } else {
            self.state_revision
        };
        let result = with_result
            .then(|| {
                self.prepare_empty_skip_result(
                    request_id,
                    transaction.origin,
                    &next_selection,
                    next_stored_marks.as_deref(),
                    changed,
                    next_state_revision,
                )
            })
            .transpose()?;

        if changed {
            let current = self.derived_state.as_mut().ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "ready Yrs engine lost derived state during selection admission",
                )
            })?;
            current.relative_selection = next_relative;
            current.resolved_selection = next_selection;
            current.stored_marks = next_stored_marks;
            current.state_revision = next_state_revision;
            self.state_revision = next_state_revision;
            self.last_committed_origin = Some(transaction.origin);
        }
        let commit = super::TransactionCommit {
            request_id,
            changed,
            document_revision: self.revision,
            state_revision: self.state_revision,
            origin: transaction.origin,
        };
        Ok((commit, result))
    }

    fn prepare_empty_skip_result(
        &self,
        request_id: u64,
        origin: TransactionOrigin,
        selection: &super::ResolvedSelection,
        stored_marks: Option<&[crate::model::Mark]>,
        changed: bool,
        state_revision: u64,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(request_id))?;
        let legacy_selection = super::derived_state::resolved_to_legacy(selection);
        let commands = crate::editor_state::command_applicability_with_known_node_count(
            &current.document,
            &self.schema,
            &legacy_selection,
            &self.resource_limits,
            current.document_node_count,
        );
        let active_state = crate::editor_state::active_state(
            &current.document,
            &self.schema,
            &legacy_selection,
            stored_marks,
            commands,
            &self.resource_limits,
        );
        let result = super::TypedTransactionResult {
            request_id,
            origin,
            changed,
            document_revision: self.revision,
            state_revision,
            selection: selection.clone(),
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update: super::RenderUpdate::None,
        };
        self.admit_typed_result(request_id, &result)?;
        Ok(result)
    }

    fn prepare_typed_result(
        &self,
        compiled: &CompiledTransaction,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
        let selection = match &compiled.selection_plan {
            SelectionPlan::Preserve => current.resolved_selection.clone(),
            SelectionPlan::Explicit(selection) | SelectionPlan::Mapped(selection) => {
                super::derived_state::resolved_from_legacy(
                    &compiled.preview,
                    selection,
                    &self.schema,
                )
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "compiled result selection cannot be resolved",
                    )
                })?
            }
        };
        let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled result stored-mark plan is not sealed",
            ));
        };
        let legacy_selection = super::derived_state::resolved_to_legacy(&selection);
        let commands = crate::editor_state::command_applicability(
            &compiled.preview,
            &self.schema,
            &legacy_selection,
            &self.resource_limits,
        );
        let active_state = crate::editor_state::active_state(
            &compiled.preview,
            &self.schema,
            &legacy_selection,
            stored_marks.as_deref(),
            commands,
            &self.resource_limits,
        );
        let render_update = if current.document == compiled.preview {
            super::RenderUpdate::None
        } else {
            match crate::render::incremental::safe_contiguous_render_blocks_patch(
                &current.document,
                &compiled.preview,
                &self.schema,
                &compiled.affected_top_level_blocks,
            ) {
                Ok(Some(patch)) => super::RenderUpdate::Patch(patch),
                Ok(None) => super::RenderUpdate::None,
                Err(full) => super::RenderUpdate::Full(full),
            }
        };
        let result = super::TypedTransactionResult {
            request_id: compiled.request_id,
            origin: compiled.origin,
            changed: current.document != compiled.preview,
            document_revision: self.revision,
            state_revision: self.state_revision,
            selection,
            active_state,
            history_state: crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            },
            render_update,
        };
        self.admit_typed_result(compiled.request_id, &result)?;
        Ok(result)
    }

    fn admit_typed_result(
        &self,
        request_id: u64,
        result: &super::TypedTransactionResult,
    ) -> super::OperationResult<()> {
        let actual = result.derived_output_bytes();
        if actual > self.editing_limits.max_derived_output_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            ));
        }
        let render_elements = match &result.render_update {
            super::RenderUpdate::None => 0,
            super::RenderUpdate::Patch(patch) => patch.blocks.iter().map(Vec::len).sum(),
            super::RenderUpdate::Full(blocks) => blocks.iter().map(Vec::len).sum(),
        };
        let render_element_limit = self.resource_limits.max_document_nodes.saturating_mul(3);
        if render_elements > render_element_limit {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDocumentNodes",
                u64::try_from(render_element_limit).unwrap_or(u64::MAX),
                u64::try_from(render_elements).unwrap_or(u64::MAX),
            ));
        }
        let schema_marks = self.schema.all_marks().count();
        let schema_nodes = self.schema.all_nodes().count();
        let active_is_bounded = result.active_state.marks.len() <= schema_marks
            && result.active_state.mark_attrs.len() <= schema_marks
            && result.active_state.allowed_marks.len() <= schema_marks
            && result.active_state.nodes.len()
                <= self.resource_limits.max_document_depth.saturating_add(1)
            && result.active_state.insertable_nodes.len() <= schema_nodes
            && result.active_state.commands.len() <= 16;
        if !active_is_bounded {
            return Err(super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxSchemaNodes",
                u64::try_from(schema_nodes.max(schema_marks)).unwrap_or(u64::MAX),
                u64::MAX,
            ));
        }
        Ok(())
    }

    pub fn apply_typed_transaction_with_result(
        &mut self,
        transaction: super::TypedTransaction,
    ) -> super::OperationResult<super::TypedTransactionResult> {
        let request_id = transaction.request_id;
        if transaction.operations.is_empty()
            && transaction.history_policy == super::HistoryPolicy::Skip
        {
            return self
                .apply_empty_skip_transaction(transaction, true)?
                .1
                .ok_or_else(|| {
                    super::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "rich empty Skip transaction produced no result envelope",
                    )
                });
        }
        let compiled = self.compile_typed_transaction(transaction)?;
        let (_, result) = self.apply_compiled_transaction(compiled, true)?;
        result.ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                request_id,
                None,
                "rich typed transaction produced no result envelope",
            )
        })
    }

    fn apply_compiled_transaction(
        &mut self,
        mut compiled: CompiledTransaction,
        with_result: bool,
    ) -> super::OperationResult<(
        super::TransactionCommit,
        Option<super::TypedTransactionResult>,
    )> {
        // A compiled plan owns Yrs handles after its original read transaction
        // closes. Reject a stale plan in O(1) before no-op classification or
        // any state-vector/snapshot traversal.
        if compiled.yrs_state_epoch != self.yrs_state_epoch {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled Yrs transaction is stale",
            ));
        }
        let preview_is_unchanged = compiled.preview
            == *self
                .document()
                .ok_or_else(|| super::OperationError::engine_not_ready(compiled.request_id))?;
        if preview_is_unchanged != compiled.mutation_plan.is_empty() {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled preview and Yrs mutation plan disagree about document changes",
            ));
        }
        let relative_plan_is_sealed = matches!(
            (&compiled.selection_plan, &compiled.relative_selection_plan),
            (SelectionPlan::Preserve, RelativeSelectionPlan::Preserve)
                | (
                    SelectionPlan::Mapped(_),
                    RelativeSelectionPlan::PreserveWithFallback(_)
                )
                | (
                    SelectionPlan::Explicit(_),
                    RelativeSelectionPlan::Precomputed { .. }
                )
                | (
                    SelectionPlan::Explicit(_),
                    RelativeSelectionPlan::OperationResult
                )
        );
        if !relative_plan_is_sealed {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled relative selection plan is not sealed",
            ));
        }
        if !matches!(compiled.stored_marks_plan, StoredMarksPlan::Set(_)) {
            return Err(super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled stored-mark plan is not sealed",
            ));
        }
        let mut result = with_result
            .then(|| self.prepare_typed_result(&compiled))
            .transpose()?;
        if preview_is_unchanged {
            let boundary_state =
                (compiled.history_policy == super::HistoryPolicy::Boundary).then(|| {
                    let txn = self.doc.transact();
                    if txn.state_vector().is_empty() {
                        Vec::new()
                    } else {
                        txn.encode_state_as_update_v1(&StateVector::default())
                    }
                });
            let current = self.derived_state.as_ref().ok_or_else(|| {
                super::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "ready Yrs engine has no derived state",
                )
            })?;
            let mut next = if matches!(compiled.selection_plan, SelectionPlan::Preserve) {
                current.clone()
            } else {
                let selection = match &compiled.selection_plan {
                    SelectionPlan::Explicit(selection) | SelectionPlan::Mapped(selection) => {
                        selection
                    }
                    SelectionPlan::Preserve => unreachable!(),
                };
                let planned_relative_selection = match &compiled.relative_selection_plan {
                    RelativeSelectionPlan::Precomputed { relative, .. } => relative.clone(),
                    RelativeSelectionPlan::OperationResult => {
                        let txn = self.doc.transact();
                        let fragment = txn
                            .get_xml_fragment(self.fragment_name.as_str())
                            .ok_or_else(|| {
                                super::OperationError::engine_invariant_failed(
                                    compiled.request_id,
                                    None,
                                    "ready Yrs document fragment is missing",
                                )
                            })?;
                        operation_result_to_relative(&txn, &fragment, selection, &self.schema)
                    }
                    RelativeSelectionPlan::Unsealed
                    | RelativeSelectionPlan::Preserve
                    | RelativeSelectionPlan::PreserveWithFallback(_) => {
                        return Err(super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "selection-only transaction has no materializable relative selection",
                        ));
                    }
                };
                let txn = self.doc.transact();
                let fragment = txn
                    .get_xml_fragment(self.fragment_name.as_str())
                    .ok_or_else(|| {
                        super::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "ready Yrs document fragment is missing",
                        )
                    })?;
                current
                    .with_relative_selection(
                        planned_relative_selection,
                        &txn,
                        &fragment,
                        &self.schema,
                        self.state_revision,
                    )
                    .ok_or_else(|| {
                        super::OperationError::selection_position_invalid(
                            compiled.request_id,
                            "selection",
                            "selection cannot be represented in the Yrs document",
                        )
                    })?
            };
            let StoredMarksPlan::Set(planned_stored_marks) = &compiled.stored_marks_plan else {
                unreachable!()
            };
            next.stored_marks = planned_stored_marks.clone();
            let prepared_boundary = boundary_state
                .map(|encoded| self.history.prepare_boundary(compiled.request_id, encoded))
                .transpose()?;
            if next.relative_selection == current.relative_selection
                && next.resolved_selection == current.resolved_selection
                && next.stored_marks == current.stored_marks
            {
                if let Some(prepared) = prepared_boundary {
                    self.history.commit_boundary(prepared);
                }
                let commit = super::TransactionCommit {
                    request_id: compiled.request_id,
                    changed: false,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    origin: compiled.origin,
                };
                if let Some(result) = &mut result {
                    result.changed = false;
                    result.document_revision = self.revision;
                    result.state_revision = self.state_revision;
                    result.history_state = crate::editor_state::HistoryState {
                        can_undo: self.can_undo(),
                        can_redo: self.can_redo(),
                    };
                }
                return Ok((commit, result));
            }
            let next_state_revision = checked_operation_increment(
                compiled.request_id,
                self.state_revision,
                "stateRevision",
            )?;
            next.state_revision = next_state_revision;
            debug_assert_eq!(next.document_revision, self.revision);
            self.derived_state = Some(next);
            self.state_revision = next_state_revision;
            self.last_committed_origin = Some(compiled.origin);
            if let Some(prepared) = prepared_boundary {
                self.history.commit_boundary(prepared);
            }
            let commit = super::TransactionCommit {
                request_id: compiled.request_id,
                changed: true,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: compiled.origin,
            };
            if let Some(result) = &mut result {
                result.changed = true;
                result.document_revision = self.revision;
                result.state_revision = self.state_revision;
                result.selection = self
                    .derived_state
                    .as_ref()
                    .expect("selection-only result retains derived state")
                    .resolved_selection
                    .clone();
                result.history_state = crate::editor_state::HistoryState {
                    can_undo: self.can_undo(),
                    can_redo: self.can_redo(),
                };
            }
            return Ok((commit, result));
        }

        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::CanonicalOutputAdmission,
        )?;
        let canonical_json = compiled.canonical_json.take().ok_or_else(|| {
            super::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "changed transaction has no admitted canonical JSON",
            )
        })?;

        // Revalidate sealed signatures against one final stable read view.
        let current_encoded_state = {
            let txn = self.doc.transact();
            #[cfg(test)]
            super::compiler::check_atomic_failpoint(
                compiled.request_id,
                super::compiler::AtomicFailpoint::FinalPreflight,
            )?;
            preflight_mutation_plan(compiled.request_id, &compiled.mutation_plan, &txn)?;
            #[cfg(test)]
            super::compiler::check_atomic_failpoint(
                compiled.request_id,
                super::compiler::AtomicFailpoint::EncodedAdmission,
            )?;
            if txn.state_vector().is_empty() {
                Vec::new()
            } else {
                txn.encode_state_as_update_v1(&StateVector::default())
            }
        };
        let admitted_encoded_bytes = current_encoded_state
            .len()
            .checked_add(compiled.encoded_growth_bound)
            .ok_or_else(|| {
                super::OperationError::document_limit_exceeded(
                    compiled.request_id,
                    None,
                    "maxEncodedStateBytes",
                    u64::try_from(self.resource_limits.max_encoded_state_bytes).unwrap_or(u64::MAX),
                    u64::MAX,
                )
            })?;
        if admitted_encoded_bytes > self.resource_limits.max_encoded_state_bytes {
            return Err(super::OperationError::document_limit_exceeded(
                compiled.request_id,
                None,
                "maxEncodedStateBytes",
                u64::try_from(self.resource_limits.max_encoded_state_bytes).unwrap_or(u64::MAX),
                u64::try_from(admitted_encoded_bytes).unwrap_or(u64::MAX),
            ));
        }

        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::RevisionAdmission,
        )?;
        let next_document_revision =
            checked_operation_increment(compiled.request_id, self.revision, "documentRevision")?;
        let next_state_revision =
            checked_operation_increment(compiled.request_id, self.state_revision, "stateRevision")?;
        let next_yrs_state_epoch = checked_operation_increment(
            compiled.request_id,
            self.yrs_state_epoch,
            "yrsStateEpoch",
        )?;
        #[cfg(test)]
        super::compiler::check_atomic_failpoint(
            compiled.request_id,
            super::compiler::AtomicFailpoint::DurableMetadataAdmission,
        )?;
        let mut next_durable_client_ids = self.durable_client_ids.clone();
        if compiled.authored_clock_units > 0 {
            next_durable_client_ids.insert(self.client_id());
        }
        let history_before = self
            .derived_state
            .as_ref()
            .map(|state| history_local_state(state, &self.fragment_name));
        let captures_history = compiled.history_policy != super::HistoryPolicy::Skip
            && compiled.history_class != super::compiler::HistoryClass::Skip;
        let history_after_template = if captures_history {
            let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
                unreachable!("stored-mark plan was sealed above")
            };
            Some(history_snapshot_template(
                &compiled.preview,
                &canonical_json,
                stored_marks.as_deref(),
                &self.fragment_name,
            ))
        } else {
            None
        };
        let history_after_metadata_bytes = history_after_template
            .as_ref()
            .map(|template| template.metadata_bytes)
            .unwrap_or(0);

        let CompiledTransaction {
            request_id,
            origin,
            history_policy,
            history_class,
            undo_units_bound,
            replay_work_units_bound,
            encoded_growth_bound,
            preview,
            selection_plan,
            relative_selection_plan,
            stored_marks_plan,
            composed_map,
            position_update_mode,
            affected_top_level_blocks,
            mutation_plan,
            ..
        } = compiled;
        let history_origin = if captures_history {
            self.history.prepare_capture(
                request_id,
                origin,
                history_policy,
                history_class,
                undo_units_bound,
                history_before,
                history_after_metadata_bytes,
                &current_encoded_state,
                encoded_growth_bound,
            )?
        } else {
            self.history.prepare_excluded(
                request_id,
                origin,
                replay_work_units_bound,
                &current_encoded_state,
                encoded_growth_bound,
            )?
        };
        let history_state_vector = self.doc.transact().state_vector();
        {
            let mut txn = self.doc.transact_mut_with(history_origin);
            execute_mutation_plan(mutation_plan, &mut txn);
        }
        let history_update = self
            .doc
            .transact()
            .encode_state_as_update_v1(&history_state_vector);

        let canonical_json_for_cache = canonical_json.clone();
        let next_derived_state = {
            let txn = self.doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .expect("committed Yrs mutation retains the document fragment");
            let explicit_relative_selection = match &selection_plan {
                SelectionPlan::Explicit(_)
                    if matches!(
                        relative_selection_plan,
                        RelativeSelectionPlan::Precomputed { .. }
                    ) =>
                {
                    let RelativeSelectionPlan::Precomputed { relative, .. } =
                        &relative_selection_plan
                    else {
                        unreachable!()
                    };
                    Some(relative.clone())
                }
                SelectionPlan::Explicit(selection) => Some(operation_result_to_relative(
                    &txn,
                    &fragment,
                    selection,
                    &self.schema,
                )),
                SelectionPlan::Mapped(_) => None,
                SelectionPlan::Preserve => None,
            };
            let preserved_fallback = match &relative_selection_plan {
                RelativeSelectionPlan::PreserveWithFallback(selection) => Some(selection),
                RelativeSelectionPlan::Precomputed { fallback, .. } => Some(fallback),
                _ => None,
            };
            let strict_fallback_affinity = matches!(
                relative_selection_plan,
                RelativeSelectionPlan::Precomputed { .. }
            );
            let mut next = self
                .derived_state
                .as_ref()
                .and_then(|state| {
                    state.after_document_change(
                        preview.clone(),
                        canonical_json_for_cache,
                        &txn,
                        &fragment,
                        &self.schema,
                        &composed_map,
                        position_update_mode,
                        &affected_top_level_blocks,
                        explicit_relative_selection.as_ref(),
                        preserved_fallback,
                        strict_fallback_affinity,
                        next_document_revision,
                        next_state_revision,
                    )
                })
                .expect("committed Yrs state must produce derived editor state");
            let StoredMarksPlan::Set(stored_marks) = stored_marks_plan else {
                unreachable!()
            };
            next.stored_marks = stored_marks;
            next
        };
        if captures_history {
            let history_after_template = history_after_template
                .expect("captured history has an admitted after-state template");
            self.history.finish_capture(
                history_after_template.seal(
                    next_derived_state.relative_selection.clone(),
                    next_derived_state.resolved_selection.clone(),
                ),
                history_update,
            );
        } else {
            self.history.finish_excluded(history_update);
            if history_policy == super::HistoryPolicy::Boundary {
                self.history.force_next_capture_boundary();
            }
        }

        debug_assert_eq!(next_derived_state.document_revision, next_document_revision);
        self.derived_state = Some(next_derived_state);
        self.durable_client_ids = next_durable_client_ids;
        self.revision = next_document_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(origin);
        let commit = super::TransactionCommit {
            request_id,
            changed: true,
            document_revision: self.revision,
            state_revision: self.state_revision,
            origin,
        };
        if let Some(result) = &mut result {
            result.request_id = request_id;
            result.origin = origin;
            result.changed = true;
            result.document_revision = self.revision;
            result.state_revision = self.state_revision;
            result.selection = self
                .derived_state
                .as_ref()
                .expect("durable result retains derived state")
                .resolved_selection
                .clone();
            result.history_state = crate::editor_state::HistoryState {
                can_undo: self.can_undo(),
                can_redo: self.can_redo(),
            };
        }
        Ok((commit, result))
    }

    pub fn export_snapshot(&self) -> YrsEngineResult<DocumentSnapshot> {
        let scope = self.scope.as_ref().ok_or_else(|| {
            snapshot_error(
                "SNAPSHOT_SCOPE_MISMATCH",
                "document scope is required to export a snapshot",
                "documentId",
            )
        })?;
        if !self.is_ready() {
            return Err(snapshot_error(
                "DOCUMENT_INVALID",
                "an awaiting document cannot be exported as a snapshot",
                "encodedState",
            ));
        }
        let encoded_state =
            encode_state_bounded(&self.doc, &self.resource_limits).map_err(|error| {
                YrsEngineError::limit(
                    "DOCUMENT_LIMIT_EXCEEDED",
                    error
                        .limit
                        .unwrap_or(self.resource_limits.max_encoded_state_bytes),
                    error
                        .actual
                        .unwrap_or(self.resource_limits.max_encoded_state_bytes),
                )
                .with_details(json!({ "field": "encodedState" }))
            })?;
        validate_snapshot_envelope_output(
            scope,
            &self.fragment_name,
            &self.schema_fingerprint,
            encoded_state.len(),
            &self.resource_limits,
        )?;

        Ok(DocumentSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            document_id: scope.document_id.clone(),
            lineage_id: scope.lineage_id.clone(),
            fragment_name: self.fragment_name.clone(),
            schema_fingerprint: self.schema_fingerprint.clone(),
            encoded_state,
        })
    }

    pub fn restore_snapshot(
        &mut self,
        snapshot: &DocumentSnapshot,
    ) -> YrsEngineResult<EngineCommit> {
        self.validate_snapshot_manifest(snapshot)?;

        let current_state = encode_state_bounded(&self.doc, &self.resource_limits)?;
        if self.is_ready() && current_state == snapshot.encoded_state {
            self.quarantined_remote_update = None;
            self.reset_history_binding();
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }

        preflight_update_v1(&snapshot.encoded_state, &self.resource_limits)?;
        let candidate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.build_snapshot_candidate(snapshot)
        }));
        let candidate = match candidate {
            Ok(result) => result?,
            Err(_) => {
                return Err(snapshot_error(
                    "COLLABORATION_DECODE_FAILED",
                    "Yrs rejected the encoded snapshot state",
                    "encodedState",
                ))
            }
        };

        let (next_revision, next_state_revision, next_yrs_state_epoch) =
            self.next_durable_revisions()?;
        let next_derived_state = build_derived_state_for_candidate(
            &candidate,
            &self.schema,
            &self.fragment_name,
            next_revision,
            next_state_revision,
        )?;
        self.doc = candidate.doc;
        let history_fragment = {
            let txn = self.doc.transact();
            txn.get_xml_fragment(self.fragment_name.as_str())
                .expect("validated snapshot candidate retains the history fragment")
        };
        self.history.rebind(&self.doc, &history_fragment);
        self.quarantined_remote_update = None;
        debug_assert_eq!(
            next_derived_state
                .as_ref()
                .map(|state| state.document_revision),
            Some(next_revision)
        );
        self.derived_state = next_derived_state;
        self.durable_client_ids = candidate.durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(TransactionOrigin::SnapshotRestore);
        Ok(EngineCommit {
            changed: true,
            revision: self.revision,
        })
    }

    fn build_snapshot_candidate(
        &self,
        snapshot: &DocumentSnapshot,
    ) -> YrsEngineResult<CandidateDocument> {
        let (candidate_doc, durable_client_ids) = {
            let update = Update::decode_v1(&snapshot.encoded_state).map_err(|error| {
                snapshot_parse_error("COLLABORATION_DECODE_FAILED", error, "encodedState")
            })?;
            let durable_state = update.state_vector();
            let durable_client_ids = durable_state
                .iter()
                .map(|(client, _)| client.get())
                .collect();
            let candidate_doc = fresh_utf16_doc_excluding(&durable_client_ids, self.client_id());
            candidate_doc
                .transact_mut_with(TransactionOrigin::SnapshotRestore.as_yrs_origin())
                .apply_update(update)
                .map_err(|error| {
                    snapshot_parse_error("COLLABORATION_DECODE_FAILED", error, "encodedState")
                })?;
            if candidate_doc.transact().has_missing_updates() {
                return Err(snapshot_error(
                    "COLLABORATION_DECODE_FAILED",
                    "encoded snapshot contains unresolved Update-v1 dependencies",
                    "encodedState",
                ));
            }
            (candidate_doc, durable_client_ids)
        };

        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let derived_json = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    snapshot_error(
                        "CODEC_INVARIANT_FAILED",
                        "snapshot Yrs fragment is missing",
                        "fragmentName",
                    )
                })?;
            codec
                .read_json(&fragment, &txn)
                .map_err(|error| snapshot_derived_error(error, "encodedState"))?
        };
        let derived_document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(map_json_import_error)
        .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        let derived_document = rehydrate_reserved_html_opaque(&derived_document);
        validate_import_document(&derived_document, &self.schema, &self.resource_limits)
            .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        encode_candidate_state_bounded(&candidate_doc, &self.resource_limits)
            .map_err(|error| snapshot_derived_error(error, "encodedState"))?;
        let canonical_json = to_prosemirror_json(&derived_document, &self.schema);
        Ok(CandidateDocument {
            doc: candidate_doc,
            state: EngineDocumentState::Ready {
                document: derived_document,
                canonical_json,
            },
            durable_client_ids,
        })
    }

    fn validate_snapshot_manifest(&self, snapshot: &DocumentSnapshot) -> YrsEngineResult<()> {
        if snapshot.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(snapshot_error(
                "SNAPSHOT_VERSION_UNSUPPORTED",
                format!(
                    "unsupported snapshot format version {}",
                    snapshot.format_version
                ),
                "formatVersion",
            ));
        }
        let scope = self.scope.as_ref().ok_or_else(|| {
            snapshot_error(
                "SNAPSHOT_SCOPE_MISMATCH",
                "document scope is required to restore a snapshot",
                "documentId",
            )
        })?;
        if snapshot.document_id != scope.document_id {
            return Err(snapshot_error(
                "SNAPSHOT_SCOPE_MISMATCH",
                "snapshot document ID does not match the engine scope",
                "documentId",
            ));
        }
        if snapshot.lineage_id != scope.lineage_id {
            return Err(snapshot_error(
                "SNAPSHOT_LINEAGE_MISMATCH",
                "snapshot lineage ID does not match the engine scope",
                "lineageId",
            ));
        }
        if snapshot.fragment_name != self.fragment_name {
            return Err(snapshot_error(
                "SNAPSHOT_FRAGMENT_MISMATCH",
                "snapshot fragment name does not match the engine fragment",
                "fragmentName",
            ));
        }
        if snapshot.schema_fingerprint != self.schema_fingerprint {
            return Err(snapshot_error(
                "SNAPSHOT_SCHEMA_MISMATCH",
                "snapshot schema fingerprint does not match the engine schema",
                "schemaFingerprint",
            ));
        }
        let metadata_bytes = snapshot.metadata_byte_len();
        if metadata_bytes > self.resource_limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.resource_limits.max_input_bytes,
                metadata_bytes,
            )
            .with_details(json!({ "field": "metadata" })));
        }
        if snapshot.encoded_state.len() > self.resource_limits.max_encoded_state_bytes {
            return Err(YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                self.resource_limits.max_encoded_state_bytes,
                snapshot.encoded_state.len(),
            )
            .with_details(json!({ "field": "encodedState" })));
        }
        Ok(())
    }

    pub fn import_json(
        &mut self,
        input: &str,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let input = BoundedInput::new(input, InputKind::DocumentJson, &self.resource_limits)?;
        let value = serde_json::from_str(input.as_str())
            .map_err(|error| YrsEngineError::parse("DOCUMENT_INVALID", error))?;
        if let Some(state) = &self.derived_state {
            if state.canonical_json == value {
                self.quarantined_remote_update = None;
                self.reset_history_binding();
                return Ok(EngineCommit {
                    changed: false,
                    revision: self.revision,
                });
            }
        }
        let document = from_prosemirror_json_with_limits(
            &value,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(map_json_import_error)?;
        let source = ValidatedImportDocument::new(document, &self.schema, &self.resource_limits)?;

        let candidate = self.build_candidate_from_document(source, origin)?;
        self.commit_candidate(candidate, origin)
    }

    pub fn import_html(
        &mut self,
        input: &str,
        options: &FromHtmlOptions,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let input = BoundedInput::new(input, InputKind::Html, &self.resource_limits)?;
        let document =
            from_html_with_limits(input.as_str(), &self.schema, options, &self.resource_limits)
                .map_err(map_html_import_error)?;
        let source = ValidatedImportDocument::new(document, &self.schema, &self.resource_limits)?;

        let candidate = self.build_candidate_from_document(source, origin)?;
        self.commit_candidate(candidate, origin)
    }

    fn build_candidate_from_document(
        &self,
        source: ValidatedImportDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<CandidateDocument> {
        let ValidatedImportDocument {
            document: source_document,
            canonical_json,
        } = source;
        let empty_json = json!({
            "type": self.schema.doc_node_type(),
            "content": [],
        });
        let doc = fresh_utf16_doc_excluding(&self.durable_client_ids, self.client_id());
        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        {
            let mut txn = doc.transact_mut_with(origin.as_yrs_origin());
            let fragment = txn.get_or_insert_xml_fragment(self.fragment_name.as_str());
            codec.apply_json(&fragment, &mut txn, &empty_json, &canonical_json)?;
        }

        let derived_json = {
            let txn = doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(candidate_invariant_error)?;
            codec.read_json_from_validated_source(&fragment, &txn)?
        };
        let derived_document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| candidate_invariant_parse_error(error, "derived document is invalid"))?;
        let derived_document = rehydrate_reserved_html_opaque(&derived_document);
        DocumentValidator::validate(&derived_document, &self.schema, &self.resource_limits)
            .map_err(|error| {
                candidate_invariant_parse_error(error, "derived document is invalid")
            })?;
        if derived_document != source_document {
            return Err(candidate_invariant_parse_error(
                "derived document does not match the validated import",
                "candidate codec round-trip changed the document",
            ));
        }
        encode_candidate_state_bounded(&doc, &self.resource_limits)?;

        let durable_client_ids = HashSet::from([doc.client_id().get()]);
        Ok(CandidateDocument {
            doc,
            state: EngineDocumentState::Ready {
                document: derived_document,
                canonical_json,
            },
            durable_client_ids,
        })
    }

    fn commit_candidate(
        &mut self,
        candidate: CandidateDocument,
        origin: TransactionOrigin,
    ) -> YrsEngineResult<EngineCommit> {
        let candidate_document = match &candidate.state {
            EngineDocumentState::Ready { document, .. } => document,
            EngineDocumentState::AwaitingRemote => {
                unreachable!("imports always build ready candidates")
            }
        };
        let unchanged = self.document() == Some(candidate_document);
        if unchanged {
            self.quarantined_remote_update = None;
            self.reset_history_binding();
            return Ok(EngineCommit {
                changed: false,
                revision: self.revision,
            });
        }

        let (next_revision, next_state_revision, next_yrs_state_epoch) =
            self.next_durable_revisions()?;
        let next_derived_state = build_derived_state_for_candidate(
            &candidate,
            &self.schema,
            &self.fragment_name,
            next_revision,
            next_state_revision,
        )?;
        self.doc = candidate.doc;
        let history_fragment = {
            let txn = self.doc.transact();
            txn.get_xml_fragment(self.fragment_name.as_str())
                .expect("validated import candidate retains the history fragment")
        };
        self.history.rebind(&self.doc, &history_fragment);
        self.quarantined_remote_update = None;
        debug_assert_eq!(
            next_derived_state
                .as_ref()
                .map(|state| state.document_revision),
            Some(next_revision)
        );
        self.derived_state = next_derived_state;
        self.durable_client_ids = candidate.durable_client_ids;
        self.revision = next_revision;
        self.state_revision = next_state_revision;
        self.yrs_state_epoch = next_yrs_state_epoch;
        self.last_committed_origin = Some(origin);
        Ok(EngineCommit {
            changed: true,
            revision: self.revision,
        })
    }

    fn next_revision(&self) -> YrsEngineResult<u64> {
        self.revision.checked_add(1).ok_or_else(|| {
            YrsEngineError::new(
                "REVISION_OVERFLOW",
                "document revision cannot be incremented",
            )
            .with_details(json!({ "field": "revision" }))
        })
    }

    fn reset_history_binding(&mut self) {
        let fragment = {
            let txn = self.doc.transact();
            txn.get_xml_fragment(self.fragment_name.as_str())
                .expect("ready Yrs document retains the history fragment")
        };
        self.history.rebind(&self.doc, &fragment);
    }

    fn next_durable_revisions(&self) -> YrsEngineResult<(u64, u64, u64)> {
        let document_revision = self.next_revision()?;
        let state_revision = self.state_revision.checked_add(1).ok_or_else(|| {
            YrsEngineError::new("REVISION_OVERFLOW", "state revision cannot be incremented")
                .with_details(json!({ "field": "stateRevision" }))
        })?;
        let yrs_state_epoch = self.yrs_state_epoch.checked_add(1).ok_or_else(|| {
            YrsEngineError::new("REVISION_OVERFLOW", "Yrs state epoch cannot be incremented")
                .with_details(json!({ "field": "yrsStateEpoch" }))
        })?;
        Ok((document_revision, state_revision, yrs_state_epoch))
    }
}

fn affinity_aware_mapped_selection(
    selection: &crate::selection::Selection,
    relative: &super::RelativeSelection,
    map: &crate::transform::StepMap,
    preview: &Document,
    schema: &Schema,
) -> crate::selection::Selection {
    let mapped = match (selection, relative) {
        (
            crate::selection::Selection::Text { anchor, head },
            super::RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            },
        ) => crate::selection::Selection::text(
            map_position(map, *anchor, relative_anchor.affinity),
            map_position(map, *head, relative_head.affinity),
        ),
        (crate::selection::Selection::Node { pos }, super::RelativeSelection::Node { point }) => {
            crate::selection::Selection::node(map_position(map, *pos, point.affinity))
        }
        (crate::selection::Selection::All, super::RelativeSelection::All) => {
            crate::selection::Selection::all()
        }
        _ => selection.map(map),
    };
    let position_map = PositionMap::build(preview, schema);
    let normalized = mapped.normalized(preview, &position_map);
    match normalized {
        crate::selection::Selection::Node { pos }
            if !selectable_void_at(preview.root(), pos, 0, schema) =>
        {
            crate::selection::Selection::cursor(pos).normalized(preview, &position_map)
        }
        selection => selection,
    }
}

fn checked_operation_increment(
    request_id: u64,
    value: u64,
    field: &'static str,
) -> super::OperationResult<u64> {
    value
        .checked_add(1)
        .ok_or_else(|| super::OperationError::revision_overflow(request_id, field))
}

fn build_derived_state_for_candidate(
    candidate: &CandidateDocument,
    schema: &Schema,
    fragment_name: &str,
    document_revision: u64,
    state_revision: u64,
) -> YrsEngineResult<Option<DerivedStateCache>> {
    let EngineDocumentState::Ready {
        document,
        canonical_json,
    } = &candidate.state
    else {
        return Ok(None);
    };
    let txn = candidate.doc.transact();
    let fragment = txn.get_xml_fragment(fragment_name).ok_or_else(|| {
        YrsEngineError::new(
            "CODEC_INVARIANT_FAILED",
            "ready Yrs document fragment is missing while deriving editor state",
        )
    })?;
    DerivedStateCache::initialize(
        document.clone(),
        canonical_json.clone(),
        &txn,
        &fragment,
        schema,
        document_revision,
        state_revision,
    )
    .map(Some)
    .ok_or_else(|| {
        YrsEngineError::new(
            "CODEC_INVARIANT_FAILED",
            "ready Yrs document cannot initialize derived editor state",
        )
    })
}

fn snapshot_error(
    code: &'static str,
    message: impl Into<String>,
    field: &'static str,
) -> YrsEngineError {
    YrsEngineError::new(code, message).with_details(json!({ "field": field }))
}

fn history_local_state(
    state: &DerivedStateCache,
    fragment_name: &str,
) -> super::history::HistoryLocalState {
    let canonical_bytes = serde_json::to_vec(&state.canonical_json)
        .expect("validated canonical history JSON remains serializable");
    super::history::HistoryLocalState {
        relative_selection: state.relative_selection.clone(),
        resolved_selection: state.resolved_selection.clone(),
        stored_marks: state.stored_marks.clone(),
        text_length: u64::try_from(state.document.root().text_content().chars().count())
            .unwrap_or(u64::MAX),
        canonical_fingerprint: Sha256::digest(&canonical_bytes).into(),
        derived_output_bytes: canonical_bytes.len(),
        metadata_bytes: history_metadata_bytes(state.stored_marks.as_deref(), fragment_name),
    }
}

fn history_snapshot_template(
    document: &Document,
    canonical_json: &serde_json::Value,
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
) -> super::history::HistorySnapshotTemplate {
    let canonical_bytes = serde_json::to_vec(canonical_json)
        .expect("validated canonical history JSON remains serializable");
    super::history::HistorySnapshotTemplate {
        stored_marks: stored_marks.map(<[crate::model::Mark]>::to_vec),
        text_length: document_text_length(document.root()),
        canonical_fingerprint: Sha256::digest(&canonical_bytes).into(),
        derived_output_bytes: canonical_bytes.len(),
        metadata_bytes: history_metadata_bytes(stored_marks, fragment_name),
    }
}

fn document_text_length(node: &crate::model::Node) -> u64 {
    if let Some(text) = node.text_str() {
        return u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
    }
    node.content()
        .map(|content| {
            content.iter().fold(0u64, |total, child| {
                total.saturating_add(document_text_length(child))
            })
        })
        .unwrap_or(0)
}

fn history_metadata_bytes(
    stored_marks: Option<&[crate::model::Mark]>,
    fragment_name: &str,
) -> usize {
    const FIXED_SELECTION_BYTES: usize = 512;
    let marks = stored_marks
        .unwrap_or_default()
        .iter()
        .map(|mark| {
            json!({
                "type": mark.mark_type(),
                "attrs": mark.attrs(),
            })
        })
        .collect::<Vec<_>>();
    let mark_bytes = serde_json::to_vec(&marks)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    FIXED_SELECTION_BYTES
        .checked_add(fragment_name.len())
        .and_then(|bytes| bytes.checked_add(mark_bytes))
        .unwrap_or(usize::MAX)
}

fn history_operation_error(request_id: u64, error: YrsEngineError) -> super::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "document"
        };
        super::OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        )
    } else {
        super::OperationError::engine_invariant_failed(request_id, None, error.message)
    }
}

fn remote_ingress_error(request_id: u64, error: YrsEngineError) -> super::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "encodedState"
        };
        let mut mapped = super::OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        super::OperationError::document_invalid(request_id, None, "update", error.message)
    }
}

fn remote_engine_error(request_id: u64, error: YrsEngineError) -> super::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let mut mapped = super::OperationError::document_limit_exceeded(
            request_id,
            None,
            "update",
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        super::OperationError::document_invalid(
            request_id,
            None,
            "update",
            format!("remote document cannot be decoded: {}", error.message),
        )
    }
}

fn remote_json_error(request_id: u64, error: JsonParseError) -> super::OperationError {
    match error {
        JsonParseError::ResourceLimit { limit, actual } => {
            super::OperationError::document_limit_exceeded(
                request_id,
                None,
                "update",
                u64::try_from(limit).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            )
        }
        error => {
            super::OperationError::document_invalid(request_id, None, "update", error.to_string())
        }
    }
}

fn remote_validation_error(
    request_id: u64,
    error: crate::boundary::BoundaryError,
) -> super::OperationError {
    if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
        let mut mapped = super::OperationError::document_limit_exceeded(
            request_id,
            None,
            "update",
            u64::try_from(error.limit.unwrap_or(0)).unwrap_or(u64::MAX),
            u64::try_from(error.actual.unwrap_or(0)).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        super::OperationError::document_invalid(request_id, None, "update", error.to_string())
    }
}

fn merge_operation_details(mapped: &mut super::OperationError, source: Option<serde_json::Value>) {
    let Some(serde_json::Value::Object(source)) = source else {
        return;
    };
    let target = mapped
        .details
        .get_or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let serde_json::Value::Object(target) = target else {
        return;
    };
    for (key, value) in source {
        if key != "field" {
            target.insert(key, value);
        }
    }
}

fn snapshot_parse_error(
    code: &'static str,
    error: impl std::fmt::Display,
    field: &'static str,
) -> YrsEngineError {
    YrsEngineError::parse(code, error).with_details(json!({ "field": field }))
}

fn snapshot_derived_error(mut error: YrsEngineError, field: &'static str) -> YrsEngineError {
    let mut details = match error.details.take() {
        Some(serde_json::Value::Object(details)) => details,
        _ => serde_json::Map::new(),
    };
    details.insert("field".into(), serde_json::Value::String(field.into()));
    error.details = Some(serde_json::Value::Object(details));
    error
}

fn validate_yrs_mark_representation(document: &Document, schema: &Schema) -> YrsEngineResult<()> {
    validate_canonical_marks(document, schema).map_err(|error| YrsEngineError {
        code: error.code,
        message: error.message,
        limit: error.limit,
        actual: error.actual,
        details: error.details,
    })
}

fn validate_config_metadata(
    fragment_name: &str,
    scope: Option<&DocumentScope>,
    limits: &ResourceLimits,
) -> YrsEngineResult<()> {
    let fields = [
        ("fragmentName", fragment_name.len()),
        (
            "documentId",
            scope.map(|scope| scope.document_id.len()).unwrap_or(0),
        ),
        (
            "lineageId",
            scope.map(|scope| scope.lineage_id.len()).unwrap_or(0),
        ),
    ];
    for (field, actual) in fields {
        if actual > limits.max_input_bytes {
            return Err(YrsEngineError::limit(
                "INPUT_LIMIT_EXCEEDED",
                limits.max_input_bytes,
                actual,
            )
            .with_details(json!({ "field": field })));
        }
    }
    let total = fields
        .into_iter()
        .fold(0usize, |total, (_, bytes)| total.saturating_add(bytes));
    if total > limits.max_input_bytes {
        return Err(
            YrsEngineError::limit("INPUT_LIMIT_EXCEEDED", limits.max_input_bytes, total)
                .with_details(json!({ "field": "metadata" })),
        );
    }
    Ok(())
}

fn validate_snapshot_envelope_output(
    scope: &DocumentScope,
    fragment_name: &str,
    schema_fingerprint: &str,
    encoded_state_bytes: usize,
    limits: &ResourceLimits,
) -> YrsEngineResult<()> {
    let metadata_bytes = scope
        .document_id
        .len()
        .saturating_add(scope.lineage_id.len())
        .saturating_add(fragment_name.len())
        .saturating_add(schema_fingerprint.len());
    let actual = metadata_bytes.saturating_add(encoded_state_bytes);
    let limit = limits
        .max_input_bytes
        .saturating_add(limits.max_encoded_state_bytes);
    if actual > limit {
        return Err(
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
                .with_details(json!({ "phase": "snapshotExport" })),
        );
    }
    Ok(())
}

fn validate_import_document(
    document: &Document,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<()> {
    let root_has_doc_role = schema
        .node(document.root().node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::Doc));
    if !root_has_doc_role {
        return Err(YrsEngineError::new(
            "DOCUMENT_INVALID",
            format!(
                "document root '{}' does not have the doc role",
                document.root().node_type()
            ),
        ));
    }
    DocumentValidator::validate(document, schema, resource_limits)
        .map(|_| ())
        .map_err(map_import_validation_error)
}

fn map_json_import_error(error: JsonParseError) -> YrsEngineError {
    match error {
        JsonParseError::ResourceLimit { limit, actual } => {
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
        }
        other => YrsEngineError::parse("DOCUMENT_INVALID", other),
    }
}

fn map_html_import_error(error: ParseError) -> YrsEngineError {
    match error {
        ParseError::ResourceLimit { limit, actual } => {
            YrsEngineError::limit("DOCUMENT_LIMIT_EXCEEDED", limit, actual)
        }
        other => YrsEngineError::parse("DOCUMENT_INVALID", other),
    }
}

fn map_import_validation_error(error: crate::boundary::BoundaryError) -> YrsEngineError {
    if error.code == "DOCUMENT_LIMIT_EXCEEDED" {
        error.into()
    } else {
        YrsEngineError {
            code: "DOCUMENT_INVALID",
            message: error.message,
            limit: error.limit,
            actual: error.actual,
            details: error.details,
        }
    }
}

fn candidate_invariant_error() -> YrsEngineError {
    candidate_invariant_parse_error(
        "candidate Yrs fragment is missing",
        "candidate Yrs fragment is missing",
    )
}

fn candidate_invariant_parse_error(
    error: impl std::fmt::Display,
    message: &'static str,
) -> YrsEngineError {
    YrsEngineError::new("CODEC_INVARIANT_FAILED", format!("{message}: {error}"))
        .with_details(json!({ "phase": "candidateDerivation" }))
}

fn build_local_empty_candidate(
    schema: &Schema,
    fragment_name: &str,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<CandidateDocument> {
    let default_document = schema
        .default_document()
        .map_err(|error| YrsEngineError::parse("DOCUMENT_INVALID", error))?;
    DocumentValidator::validate(&default_document, schema, resource_limits)?;
    let canonical_json = to_prosemirror_json(&default_document, schema);

    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(schema, resource_limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment(fragment_name);
        codec.apply_json(
            &fragment,
            &mut txn,
            &json!({
                "type": schema.doc_node_type(),
                "content": [],
            }),
            &canonical_json,
        )?;
    }

    let derived_json = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment(fragment_name).ok_or_else(|| {
            YrsEngineError::new(
                "CODEC_INVARIANT_FAILED",
                "initialized Yrs fragment is missing",
            )
        })?;
        codec.read_json(&fragment, &txn)?
    };
    let document = from_prosemirror_json_with_limits(
        &derived_json,
        schema,
        UnknownTypeMode::Error,
        resource_limits,
    )
    .map_err(|error| YrsEngineError::parse("CODEC_INVARIANT_FAILED", error))?;
    DocumentValidator::validate(&document, schema, resource_limits)?;
    let canonical_json = to_prosemirror_json(&document, schema);
    encode_state_bounded(&doc, resource_limits)?;

    let durable_client_ids = HashSet::from([doc.client_id().get()]);
    Ok(CandidateDocument {
        doc,
        state: EngineDocumentState::Ready {
            document,
            canonical_json,
        },
        durable_client_ids,
    })
}

fn build_await_remote_candidate(
    fragment_name: &str,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<CandidateDocument> {
    let doc = utf16_doc();
    doc.get_or_insert_xml_fragment(fragment_name);
    encode_state_bounded(&doc, resource_limits)?;
    Ok(CandidateDocument {
        doc,
        state: EngineDocumentState::AwaitingRemote,
        durable_client_ids: HashSet::new(),
    })
}

fn utf16_doc() -> Doc {
    let options = Options {
        offset_kind: OffsetKind::Utf16,
        // Yrs history StackItems refer to deleted structs. Keep them available
        // in both live and candidate stores for the lifetime of an epoch.
        skip_gc: true,
        ..Options::default()
    };
    Doc::with_options(options)
}

fn fresh_utf16_doc_excluding(durable_client_ids: &HashSet<u64>, previous_client_id: u64) -> Doc {
    fresh_utf16_doc_excluding_with(durable_client_ids, previous_client_id, utf16_doc)
}

fn fresh_utf16_doc_excluding_with(
    durable_client_ids: &HashSet<u64>,
    previous_client_id: u64,
    mut candidate: impl FnMut() -> Doc,
) -> Doc {
    loop {
        let doc = candidate();
        let client_id = doc.client_id().get();
        if client_id != previous_client_id && !durable_client_ids.contains(&client_id) {
            return doc;
        }
    }
}

fn encode_state_bounded(doc: &Doc, resource_limits: &ResourceLimits) -> YrsEngineResult<Vec<u8>> {
    let txn = doc.transact();
    let encoded_state = if txn.state_vector().is_empty() {
        Vec::new()
    } else {
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    if encoded_state.len() > resource_limits.max_encoded_state_bytes {
        return Err(YrsEngineError::limit(
            "INPUT_LIMIT_EXCEEDED",
            resource_limits.max_encoded_state_bytes,
            encoded_state.len(),
        ));
    }
    Ok(encoded_state)
}

fn encode_candidate_state_bounded(
    doc: &Doc,
    resource_limits: &ResourceLimits,
) -> YrsEngineResult<Vec<u8>> {
    encode_state_bounded(doc, resource_limits).map_err(|error| {
        if error.code == "INPUT_LIMIT_EXCEEDED" {
            YrsEngineError::limit(
                "DOCUMENT_LIMIT_EXCEEDED",
                error
                    .limit
                    .unwrap_or(resource_limits.max_encoded_state_bytes),
                error
                    .actual
                    .unwrap_or(resource_limits.max_encoded_state_bytes),
            )
            .with_details(json!({ "phase": "candidateDerivation" }))
        } else {
            error
        }
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use crate::boundary::ResourceLimits;
    use crate::model::Mark;
    use crate::schema::presets::tiptap_schema;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use serde_json::json;
    use yrs::OffsetKind;

    use yrs::branch::{Branch, BranchPtr};
    use yrs::types::xml::{XmlFragment, XmlOut, XmlTextRef};
    use yrs::{updates::decoder::Decode, Update};
    use yrs::{Assoc, ClientID, Doc, Options, ReadTxn, StickyIndex, Transact};

    use crate::yrs_engine::{
        Affinity, CommandPlan, EditorOffsetKind, HistoryPolicy, RevisionedPosition,
        RevisionedRange, SelectionInput, SelectionIntent, TransactionOrigin, TypedCommand,
        TypedOperation, TypedTransaction,
    };

    use super::{
        fresh_utf16_doc_excluding_with, utf16_doc, ValidatedImportDocument, YrsDocumentEngine,
        YrsEngineConfig,
    };

    #[derive(Debug, PartialEq)]
    struct AtomicAudit {
        encoded: Vec<u8>,
        json: Option<serde_json::Value>,
        html: Option<String>,
        revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        client_id: u64,
        durable_client_ids: HashSet<u64>,
        origin: Option<TransactionOrigin>,
        scope: Option<crate::yrs_engine::DocumentScope>,
        fragment: String,
        fingerprint: String,
        selection: Option<crate::yrs_engine::ResolvedSelection>,
        stored_marks: Option<Vec<crate::model::Mark>>,
        can_undo: bool,
        can_redo: bool,
        retained_history_units: u64,
        replay_audit: (usize, usize, bool),
    }

    fn atomic_audit(engine: &YrsDocumentEngine) -> AtomicAudit {
        AtomicAudit {
            encoded: engine.encoded_state().unwrap(),
            json: engine.document_json(),
            html: engine.document_html(),
            revision: engine.revision,
            state_revision: engine.state_revision,
            yrs_state_epoch: engine.yrs_state_epoch,
            client_id: engine.client_id(),
            durable_client_ids: engine.durable_client_ids.clone(),
            origin: engine.last_committed_origin,
            scope: engine.scope.clone(),
            fragment: engine.fragment_name.clone(),
            fingerprint: engine.schema_fingerprint.clone(),
            selection: engine.resolved_selection().cloned(),
            stored_marks: engine.stored_marks().map(<[_]>::to_vec),
            can_undo: engine.can_undo(),
            can_redo: engine.can_redo(),
            retained_history_units: engine.history.retained_units(0).unwrap(),
            replay_audit: engine.history.replay_audit_for_test(),
        }
    }

    fn transaction_engine() -> YrsDocumentEngine {
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap()
    }

    fn insert_transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        }
    }

    #[test]
    fn derived_state_node_count_refreshes_and_empty_results_use_equivalent_commands() {
        let mut engine = transaction_engine();
        let initial = engine.derived_state.as_ref().unwrap();
        assert_eq!(
            initial.document_node_count,
            crate::editor_state::document_node_count(initial.document.root())
        );

        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let refreshed = engine.derived_state.as_ref().unwrap();
        assert_eq!(refreshed.document_revision, engine.revision());
        assert_eq!(
            refreshed.document_node_count,
            crate::editor_state::document_node_count(refreshed.document.root())
        );

        let transaction = TypedTransaction {
            request_id: 991,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
                head: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
            }),
            history_policy: HistoryPolicy::Skip,
        };
        let result = engine
            .apply_typed_transaction_with_result(transaction)
            .unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let selection = state.legacy_selection();
        assert_eq!(
            result.active_state.commands,
            crate::editor_state::command_applicability(
                &state.document,
                &engine.schema,
                &selection,
                &engine.resource_limits,
            )
        );
    }

    #[test]
    fn utf16_doc_preserves_fresh_client_ids_and_uses_utf16_offsets() {
        let first = utf16_doc();
        let second = utf16_doc();

        assert_eq!(first.offset_kind(), OffsetKind::Utf16);
        assert_eq!(second.offset_kind(), OffsetKind::Utf16);
        assert_ne!(first.client_id(), second.client_id());
    }

    #[test]
    fn validated_import_source_reuses_one_schema_ranked_canonical_result() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let input = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "ordered",
                    "marks": [{ "type": "bold" }, { "type": "italic" }]
                }]
            }]
        });
        let parsed = from_prosemirror_json(&input, &schema, UnknownTypeMode::Preserve).unwrap();

        let validated = ValidatedImportDocument::new(parsed, &schema, &limits).unwrap();

        assert_eq!(
            validated.canonical_json,
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "ordered",
                        "marks": [{ "type": "bold" }, { "type": "italic" }]
                    }]
                }]
            })
        );
        assert_eq!(
            validated.canonical_json,
            crate::serialize::to_prosemirror_json(&validated.document, &schema)
        );
    }

    #[test]
    fn collision_excluding_candidate_selection_retries_live_and_durable_ids() {
        let durable = HashSet::from([7_u64]);
        let mut ids = [5_u64, 7_u64, 11_u64].into_iter();
        let selected = fresh_utf16_doc_excluding_with(&durable, 5, || {
            Doc::with_options(Options {
                client_id: ClientID::new(ids.next().unwrap()),
                offset_kind: OffsetKind::Utf16,
                ..Options::default()
            })
        });

        assert_eq!(selected.client_id().get(), 11);
    }

    #[test]
    fn restored_and_local_candidates_cache_all_relevant_durable_clients() {
        let config = || crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        };
        let source = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let expected = Update::decode_v1(&snapshot.encoded_state)
            .unwrap()
            .state_vector()
            .iter()
            .map(|(client, _)| client.get())
            .collect::<HashSet<_>>();
        let mut target = crate::yrs_engine::YrsDocumentEngine::new(config()).unwrap();

        target.restore_snapshot(&snapshot).unwrap();
        assert_eq!(target.durable_client_ids, expected);

        target
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"local"}]}]}"#,
                crate::yrs_engine::TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert_eq!(
            target.durable_client_ids,
            HashSet::from([target.client_id()])
        );
    }

    #[test]
    fn revision_overflow_rejects_before_candidate_swap() {
        let mut engine =
            crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
                resource_limits: ResourceLimits::default(),
                editing_limits: crate::yrs_engine::EditingLimits::default(),
                max_length: None,
                scope: None,
            })
            .unwrap();
        engine.revision = u64::MAX;
        engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
        let before_client = engine.client_id();
        let before_json = engine.document_json();
        let before_state = engine.encoded_state().unwrap();

        let error = engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                crate::yrs_engine::TransactionOrigin::DocumentImport,
            )
            .unwrap_err();

        assert_eq!(error.code, "REVISION_OVERFLOW");
        assert_eq!(engine.revision(), u64::MAX);
        assert_eq!(engine.client_id(), before_client);
        assert_eq!(engine.document_json(), before_json);
        assert_eq!(engine.encoded_state().unwrap(), before_state);
    }

    #[test]
    fn candidate_state_revision_and_epoch_overflow_reject_before_swap() {
        for field in ["stateRevision", "yrsStateEpoch"] {
            let mut engine = transaction_engine();
            if field == "stateRevision" {
                engine.state_revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
            } else {
                engine.yrs_state_epoch = u64::MAX;
            }
            let before = atomic_audit(&engine);

            let error = engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap_err();

            assert_eq!(error.code, "REVISION_OVERFLOW", "{field}");
            assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
            assert_eq!(atomic_audit(&engine), before, "{field}");
        }
    }

    #[test]
    fn identical_selection_is_no_op_even_when_state_revision_is_max() {
        let mut engine = transaction_engine();
        engine.state_revision = u64::MAX;
        if let Some(state) = &mut engine.derived_state {
            state.state_revision = u64::MAX;
        }
        let before = atomic_audit(&engine);
        let transaction = TypedTransaction {
            request_id: 90_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(crate::yrs_engine::SelectionInput::Text {
                anchor: RevisionedPosition {
                    offset: 0,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
                head: RevisionedPosition {
                    offset: 0,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::Before,
                },
            }),
            history_policy: HistoryPolicy::Skip,
        };
        let commit = engine.apply_typed_transaction(transaction).unwrap();
        assert!(!commit.changed);
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn snapshot_export_envelope_budget_has_exact_and_over_boundaries_without_mutation() {
        let mut engine =
            crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
                schema: tiptap_schema(),
                fragment_name: "prosemirror".into(),
                initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
                resource_limits: ResourceLimits::default(),
                editing_limits: crate::yrs_engine::EditingLimits::default(),
                max_length: None,
                scope: Some(crate::yrs_engine::DocumentScope {
                    document_id: "doc".into(),
                    lineage_id: "lineage".into(),
                }),
            })
            .unwrap();
        let state = engine.encoded_state().unwrap();
        let metadata_bytes =
            "doc".len() + "lineage".len() + "prosemirror".len() + engine.schema_fingerprint().len();
        engine.resource_limits.max_input_bytes = metadata_bytes;
        engine.resource_limits.max_encoded_state_bytes = state.len();
        assert!(engine.export_snapshot().is_ok());

        let before_revision = engine.revision();
        let before_client = engine.client_id();
        let before_json = engine.document_json();
        engine.resource_limits.max_input_bytes = metadata_bytes - 1;
        let error = engine.export_snapshot().unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(serde_json::json!({"phase": "snapshotExport"}))
        );
        assert_eq!(engine.revision(), before_revision);
        assert_eq!(engine.client_id(), before_client);
        assert_eq!(engine.document_json(), before_json);
        assert_eq!(engine.encoded_state().unwrap(), state);
    }

    #[test]
    fn typed_transaction_rejects_every_revision_or_epoch_overflow_before_mutation() {
        for field in ["documentRevision", "stateRevision", "yrsStateEpoch"] {
            let mut engine = transaction_engine();
            match field {
                "documentRevision" => {
                    engine.revision = u64::MAX;
                    engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
                }
                "stateRevision" => {
                    engine.state_revision = u64::MAX;
                    engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
                }
                "yrsStateEpoch" => engine.yrs_state_epoch = u64::MAX,
                _ => unreachable!(),
            }
            let transaction = insert_transaction(&engine, 71);
            let before = atomic_audit(&engine);

            let error = engine.apply_typed_transaction(transaction).unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{field}");
            assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
            assert_eq!(atomic_audit(&engine), before, "{field}");
        }
    }

    #[test]
    fn compiled_transaction_epoch_is_checked_before_yrs_metadata_revalidation() {
        for changed in [true, false] {
            let mut engine = transaction_engine();
            let transaction = if changed {
                insert_transaction(&engine, 72)
            } else {
                TypedTransaction {
                    request_id: 72,
                    base_document_revision: engine.revision(),
                    origin: TransactionOrigin::LocalApi,
                    operations: vec![],
                    selection_intent: SelectionIntent::Preserve,
                    history_policy: HistoryPolicy::Skip,
                }
            };
            let compiled = engine.compile_typed_transaction(transaction).unwrap();
            engine.yrs_state_epoch += 1;
            let before = atomic_audit(&engine);

            let error = engine
                .apply_compiled_transaction(compiled, false)
                .unwrap_err();

            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
            assert!(error.message.contains("stale"), "changed={changed}");
            assert_eq!(atomic_audit(&engine), before, "changed={changed}");
        }
    }

    #[test]
    fn projected_encoded_ceiling_accepts_exact_and_rejects_one_under_without_new_clock() {
        let mut exact = transaction_engine();
        let exact_transaction = insert_transaction(&exact, 73);
        let exact_compiled = exact
            .compile_typed_transaction(exact_transaction.clone())
            .unwrap();
        let exact_limit = exact
            .encoded_state()
            .unwrap()
            .len()
            .checked_add(exact_compiled.encoded_growth_bound)
            .unwrap();
        exact.resource_limits.max_encoded_state_bytes = exact_limit;

        let commit = exact.apply_typed_transaction(exact_transaction).unwrap();

        assert!(commit.changed);
        assert!(exact.encoded_state().unwrap().len() <= exact_limit);

        let mut one_under = transaction_engine();
        let rejected_transaction = insert_transaction(&one_under, 74);
        let rejected_compiled = one_under
            .compile_typed_transaction(rejected_transaction.clone())
            .unwrap();
        let rejected_limit = one_under
            .encoded_state()
            .unwrap()
            .len()
            .checked_add(rejected_compiled.encoded_growth_bound)
            .unwrap()
            - 1;
        one_under.resource_limits.max_encoded_state_bytes = rejected_limit;
        let before = atomic_audit(&one_under);

        let error = one_under
            .apply_typed_transaction(rejected_transaction)
            .unwrap_err();

        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxEncodedStateBytes" }))
        );
        assert_eq!(error.limit, Some(rejected_limit as u64));
        assert_eq!(error.actual, Some((rejected_limit + 1) as u64));
        assert_eq!(atomic_audit(&one_under), before);
    }

    #[test]
    fn canonical_cache_output_accepts_exact_rejects_one_under_and_reuses_empty_noop_cache() {
        let expected = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "x" }]
            }]
        });
        let exact_bytes = serde_json::to_vec(&expected).unwrap().len();

        let mut exact = transaction_engine();
        exact.editing_limits.max_derived_output_bytes = exact_bytes;
        let transaction = insert_transaction(&exact, 77);
        exact.apply_typed_transaction(transaction).unwrap();
        assert_eq!(exact.document_json(), Some(expected));

        let mut one_under = transaction_engine();
        one_under.editing_limits.max_derived_output_bytes = exact_bytes - 1;
        let transaction = insert_transaction(&one_under, 78);
        let before = atomic_audit(&one_under);
        let error = one_under.apply_typed_transaction(transaction).unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some((exact_bytes - 1) as u64));
        assert_eq!(error.actual, Some(exact_bytes as u64));
        assert_eq!(atomic_audit(&one_under), before);

        let mut empty_noop = transaction_engine();
        empty_noop.editing_limits.max_derived_output_bytes = 1;
        let transaction = TypedTransaction {
            request_id: 79,
            base_document_revision: empty_noop.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        };
        let before = atomic_audit(&empty_noop);
        let commit = empty_noop.apply_typed_transaction(transaction).unwrap();
        assert!(!commit.changed);
        assert_eq!(atomic_audit(&empty_noop), before);
    }

    #[test]
    fn typed_commit_installs_local_client_origin_and_candidate_revisions() {
        let mut source = transaction_engine();
        let imported = source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(imported.changed);
        assert_eq!(
            (
                source.revision,
                source.state_revision,
                source.yrs_state_epoch
            ),
            (1, 1, 1)
        );
        let snapshot = source.export_snapshot().unwrap();
        let mut target = transaction_engine();
        target.restore_snapshot(&snapshot).unwrap();
        let local_client = target.client_id();
        assert!(!target.durable_client_ids.contains(&local_client));
        assert_eq!(
            (
                target.revision,
                target.state_revision,
                target.yrs_state_epoch
            ),
            (1, 1, 1)
        );

        let transaction = insert_transaction(&target, 75);
        let commit = target.apply_typed_transaction(transaction).unwrap();

        assert!(commit.changed);
        assert!(target.durable_client_ids.contains(&local_client));
        assert_eq!(
            target.last_committed_origin,
            Some(TransactionOrigin::LocalApi)
        );
        assert_eq!(
            (
                target.revision,
                target.state_revision,
                target.yrs_state_epoch
            ),
            (2, 2, 2)
        );

        let unchanged = target.document_json().unwrap();
        let commit = target
            .import_json(
                &serde_json::to_string(&unchanged).unwrap(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        assert!(!commit.changed);
        assert_eq!(
            (
                target.revision,
                target.state_revision,
                target.yrs_state_epoch
            ),
            (2, 2, 2)
        );
    }

    #[test]
    fn restored_deletion_only_commit_does_not_claim_an_unauthored_local_client() {
        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let mut target = transaction_engine();
        target.restore_snapshot(&snapshot).unwrap();
        let local_client = target.client_id();
        assert!(!target.durable_client_ids.contains(&local_client));
        let from = RevisionedPosition {
            offset: 0,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let to = RevisionedPosition { offset: 1, ..from };
        let transaction = TypedTransaction {
            request_id: 80,
            base_document_revision: target.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange { from, to },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        };

        let compiled = target
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        assert_eq!(compiled.authored_clock_units, 0);
        target.apply_typed_transaction(transaction).unwrap();

        assert!(!target.durable_client_ids.contains(&local_client));
        let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
            .unwrap()
            .state_vector();
        assert!(durable_clients.get(&ClientID::new(local_client)) == 0);
    }

    #[test]
    fn restored_format_only_commit_records_its_authored_local_clock() {
        let mut source = transaction_engine();
        source
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let snapshot = source.export_snapshot().unwrap();
        let mut target = transaction_engine();
        target.restore_snapshot(&snapshot).unwrap();
        let local_client = target.client_id();
        let from = RevisionedPosition {
            offset: 0,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        let to = RevisionedPosition { offset: 1, ..from };
        let transaction = TypedTransaction {
            request_id: 81,
            base_document_revision: target.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::AddMark {
                range: RevisionedRange { from, to },
                mark: Mark::new("bold".into(), HashMap::new()),
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        };

        let compiled = target
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        assert!(compiled.authored_clock_units > 0);
        target.apply_typed_transaction(transaction).unwrap();

        assert!(target.durable_client_ids.contains(&local_client));
        let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
            .unwrap()
            .state_vector();
        assert!(durable_clients.get(&ClientID::new(local_client)) > 0);
    }

    fn select_text(engine: &mut YrsDocumentEngine, request_id: u64, anchor: u32, head: u32) {
        let point = |offset| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point(anchor),
                    head: point(head),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
    }

    fn unaffected_text_sticky(
        engine: &YrsDocumentEngine,
        text_child: u32,
        utf16_index: u32,
    ) -> (crate::yrs_engine::RelativePoint, BranchPtr, u32) {
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("expected paragraph")
        };
        let XmlOut::Text(text) = paragraph.get(&txn, text_child).unwrap() else {
            panic!("expected text child")
        };
        let branch = BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text));
        let sticky = StickyIndex::at(&txn, branch, utf16_index, Assoc::After).unwrap();
        let point = crate::yrs_engine::RelativePoint {
            sticky,
            affinity: Affinity::After,
        };
        let Some(offset) = point.sticky.get_offset(&txn) else {
            panic!("sticky must resolve")
        };
        let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
            &txn,
            &fragment,
            &point,
            &engine.schema,
        )
        .unwrap();
        let scalar = engine
            .position_map()
            .unwrap()
            .doc_to_scalar(doc_pos, engine.document().unwrap());
        (point, offset.branch, scalar)
    }

    fn assert_unaffected_sticky(
        engine: &YrsDocumentEngine,
        point: &crate::yrs_engine::RelativePoint,
        branch: BranchPtr,
        expected_scalar: u32,
    ) {
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let offset = point.sticky.get_offset(&txn).unwrap();
        assert_eq!(
            offset.branch, branch,
            "unaffected Yrs branch identity changed"
        );
        let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
            &txn,
            &fragment,
            point,
            &engine.schema,
        )
        .unwrap();
        assert_eq!(
            engine
                .position_map()
                .unwrap()
                .doc_to_scalar(doc_pos, engine.document().unwrap()),
            expected_scalar,
            "unaffected sticky point moved to the wrong rendered position"
        );
    }

    #[test]
    fn granular_command_lowering_preserves_classification_locality_and_unaffected_sticky_identity()
    {
        let mut format = transaction_engine();
        format
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},{"type":"text","text":"bc tail"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let (format_sticky, format_branch, format_scalar) = unaffected_text_sticky(&format, 1, 5);
        select_text(&mut format, 100, 2, 0);
        let CommandPlan::Transaction(format_transaction) = format
            .plan_command(
                101,
                TypedCommand::SetMark {
                    mark_type: "link".into(),
                    attrs: HashMap::from([("href".into(), json!("new"))]),
                },
            )
            .unwrap()
        else {
            panic!("range format must plan")
        };
        assert!(matches!(
            format_transaction.operations.as_slice(),
            [
                TypedOperation::RemoveMark { .. },
                TypedOperation::AddMark { .. }
            ]
        ));
        let compiled = format
            .compile_typed_transaction(format_transaction.clone())
            .unwrap();
        assert_eq!(
            compiled.history_class,
            crate::yrs_engine::compiler::HistoryClass::Format
        );
        assert_eq!(
            compiled.position_update_mode,
            crate::position::update::UpdateMode::MarksOnly
        );
        assert_eq!(compiled.affected_top_level_blocks, vec![0]);
        let format_result = format
            .apply_typed_transaction_with_result(format_transaction)
            .unwrap();
        let crate::yrs_engine::RenderUpdate::Patch(format_patch) = format_result.render_update
        else {
            panic!("range format must produce a local render patch")
        };
        assert_eq!(
            (
                format_patch.start_index,
                format_patch.delete_count,
                format_patch.blocks.len(),
            ),
            (0, 1, 1)
        );
        assert_unaffected_sticky(&format, &format_sticky, format_branch, format_scalar);
        assert!(format.can_undo());

        let mut replace = transaction_engine();
        replace
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"left target right"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let (replace_sticky, replace_branch, replace_scalar) =
            unaffected_text_sticky(&replace, 0, 13);
        select_text(&mut replace, 102, 11, 5);
        let CommandPlan::Transaction(replace_transaction) = replace
            .plan_command(
                103,
                TypedCommand::ReplaceSelectionText { text: "new".into() },
            )
            .unwrap()
        else {
            panic!("range replacement must plan")
        };
        assert!(matches!(
            replace_transaction.operations.as_slice(),
            [
                TypedOperation::DeleteRange { .. },
                TypedOperation::InsertText { .. }
            ]
        ));
        let compiled = replace
            .compile_typed_transaction(replace_transaction.clone())
            .unwrap();
        assert_eq!(
            compiled.history_class,
            crate::yrs_engine::compiler::HistoryClass::Structural
        );
        assert_eq!(
            compiled.position_update_mode,
            crate::position::update::UpdateMode::InlineTextOnly
        );
        assert_eq!(compiled.affected_top_level_blocks, vec![0]);
        let replace_result = replace
            .apply_typed_transaction_with_result(replace_transaction)
            .unwrap();
        let crate::yrs_engine::RenderUpdate::Patch(replace_patch) = replace_result.render_update
        else {
            panic!("range replacement must produce a local render patch")
        };
        assert_eq!(
            (
                replace_patch.start_index,
                replace_patch.delete_count,
                replace_patch.blocks.len(),
            ),
            (0, 1, 1)
        );
        assert_unaffected_sticky(
            &replace,
            &replace_sticky,
            replace_branch,
            replace_scalar - 3,
        );
        assert!(replace.can_undo());
    }

    #[test]
    fn direct_command_admission_error_is_not_replanned_as_structure() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"target"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 104, 6, 0);
        engine.resource_limits.max_input_bytes = 0;
        let before = atomic_audit(&engine);

        let error = engine
            .plan_command(105, TypedCommand::ReplaceSelectionText { text: "x".into() })
            .unwrap_err();

        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
        assert_eq!(atomic_audit(&engine), before);
    }

    #[test]
    fn every_recoverable_atomic_stage_failpoint_is_pre_open_and_read_only() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

        let failpoints = [
            AtomicFailpoint::EnvelopeAdmission,
            AtomicFailpoint::SemanticCompilation,
            AtomicFailpoint::MutationPreflight,
            AtomicFailpoint::FinalPreflight,
            AtomicFailpoint::EncodedAdmission,
            AtomicFailpoint::CanonicalOutputAdmission,
            AtomicFailpoint::RevisionAdmission,
            AtomicFailpoint::DurableMetadataAdmission,
        ];
        for failpoint in failpoints {
            let mut engine = transaction_engine();
            let transaction = insert_transaction(&engine, 76);
            let before = atomic_audit(&engine);
            set_atomic_failpoint_for_test(Some(failpoint));

            let error = engine.apply_typed_transaction(transaction).unwrap_err();

            set_atomic_failpoint_for_test(None);
            assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
            assert_eq!(
                error.details,
                Some(json!({ "failpoint": failpoint.field_name() })),
                "{failpoint:?}"
            );
            assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        }
    }

    #[test]
    fn empty_skip_selection_bypasses_mutation_preflight_but_not_admission_or_boundaries() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

        let selection_transaction =
            |engine: &YrsDocumentEngine, request_id, history_policy| TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy,
            };

        let mut skip = transaction_engine();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
        let result = skip
            .apply_typed_transaction_with_result(selection_transaction(
                &skip,
                760,
                HistoryPolicy::Skip,
            ))
            .expect("empty Skip selection must not enter mutation preflight");
        set_atomic_failpoint_for_test(None);
        assert!(result.changed);
        assert_eq!(skip.revision(), 0);
        assert_eq!(skip.state_revision(), 1);

        let mut boundary = transaction_engine();
        let before_boundary = atomic_audit(&boundary);
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
        let boundary_error = boundary
            .apply_typed_transaction(selection_transaction(
                &boundary,
                761,
                HistoryPolicy::Boundary,
            ))
            .unwrap_err();
        set_atomic_failpoint_for_test(None);
        assert_eq!(
            boundary_error.details,
            Some(json!({ "failpoint": "mutationPreflight" }))
        );
        assert_eq!(atomic_audit(&boundary), before_boundary);

        let mut rejected = transaction_engine();
        let before_rejected = atomic_audit(&rejected);
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::EnvelopeAdmission));
        let admission_error = rejected
            .apply_typed_transaction(selection_transaction(&rejected, 762, HistoryPolicy::Skip))
            .unwrap_err();
        set_atomic_failpoint_for_test(None);
        assert_eq!(
            admission_error.details,
            Some(json!({ "failpoint": "envelopeAdmission" }))
        );
        assert_eq!(atomic_audit(&rejected), before_rejected);
    }

    #[test]
    fn empty_skip_fast_path_matches_full_compiler_at_yrs_scan_work_boundary() {
        fn populated_engine() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"scan boundary"}]}]}"#,
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn scan_work(engine: &YrsDocumentEngine) -> usize {
            let document_text_bytes = engine.document().unwrap().root().text_content().len();
            let txn = engine.doc.transact();
            let crdt_clock_work = txn
                .state_vector()
                .iter()
                .map(|(_, clock)| usize::try_from(*clock).unwrap() + 1)
                .sum::<usize>();
            document_text_bytes * 2 + crdt_clock_work * 2
        }

        fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            }
        }

        let required = scan_work(&populated_engine());

        let mut exact_fast = populated_engine();
        exact_fast.resource_limits.max_input_bytes = required;
        let exact_fast_result = exact_fast
            .apply_typed_transaction_with_result(transaction(&exact_fast, 763))
            .unwrap();
        let mut exact_slow = populated_engine();
        exact_slow.resource_limits.max_input_bytes = required;
        let exact_slow_transaction = transaction(&exact_slow, 763);
        let exact_slow_compiled = exact_slow
            .compile_typed_transaction(exact_slow_transaction)
            .unwrap();
        let exact_slow_result = exact_slow
            .apply_compiled_transaction(exact_slow_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(exact_fast_result, exact_slow_result);
        assert_eq!(exact_fast.document_json(), exact_slow.document_json());
        assert_eq!(exact_fast.document_html(), exact_slow.document_html());
        assert_eq!(exact_fast.revision(), exact_slow.revision());
        assert_eq!(exact_fast.state_revision(), exact_slow.state_revision());
        assert_eq!(
            exact_fast.resolved_selection(),
            exact_slow.resolved_selection()
        );
        assert_eq!(exact_fast.stored_marks(), exact_slow.stored_marks());
        assert_eq!(exact_fast.can_undo(), exact_slow.can_undo());
        assert_eq!(exact_fast.can_redo(), exact_slow.can_redo());

        let mut one_under_slow = populated_engine();
        one_under_slow.resource_limits.max_input_bytes = required - 1;
        let before_slow = atomic_audit(&one_under_slow);
        let slow_error = one_under_slow
            .compile_typed_transaction(transaction(&one_under_slow, 764))
            .unwrap_err();
        assert_eq!(atomic_audit(&one_under_slow), before_slow);

        let mut one_under_fast = populated_engine();
        one_under_fast.resource_limits.max_input_bytes = required - 1;
        let before_fast = atomic_audit(&one_under_fast);
        let fast_error = one_under_fast
            .apply_typed_transaction_with_result(transaction(&one_under_fast, 764))
            .unwrap_err();
        assert_eq!(fast_error, slow_error);
        assert_eq!(atomic_audit(&one_under_fast), before_fast);

        let mut changed_document = populated_engine();
        changed_document
            .apply_command(765, TypedCommand::InsertText { text: "é".into() })
            .unwrap()
            .unwrap();
        let cached_text_bytes = changed_document
            .derived_state
            .as_ref()
            .unwrap()
            .document_text_bytes;
        assert_eq!(
            cached_text_bytes,
            changed_document
                .document()
                .unwrap()
                .root()
                .text_content()
                .len()
        );
        let changed_required = scan_work(&changed_document);
        changed_document.resource_limits.max_input_bytes = changed_required - 1;
        let before_changed = atomic_audit(&changed_document);
        let changed_error = changed_document
            .apply_typed_transaction_with_result(transaction(&changed_document, 766))
            .unwrap_err();
        assert_eq!(changed_error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(
            changed_error.limit,
            Some(u64::try_from(changed_required - 1).unwrap())
        );
        assert_eq!(
            changed_error.actual,
            Some(u64::try_from(changed_required).unwrap())
        );
        assert_eq!(atomic_audit(&changed_document), before_changed);

        let invalid_selection = |engine: &YrsDocumentEngine| TypedTransaction {
            request_id: 767,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: RevisionedPosition {
                    offset: u32::MAX,
                    kind: EditorOffsetKind::Utf16,
                    affinity: Affinity::Before,
                },
                head: RevisionedPosition {
                    offset: u32::MAX,
                    kind: EditorOffsetKind::Utf16,
                    affinity: Affinity::After,
                },
            }),
            history_policy: HistoryPolicy::Skip,
        };
        let invalid_slow = populated_engine();
        let before_invalid_slow = atomic_audit(&invalid_slow);
        let invalid_slow_error = invalid_slow
            .compile_typed_transaction(invalid_selection(&invalid_slow))
            .unwrap_err();
        assert_eq!(atomic_audit(&invalid_slow), before_invalid_slow);
        let mut invalid_fast = populated_engine();
        let before_invalid_fast = atomic_audit(&invalid_fast);
        let invalid_fast_error = invalid_fast
            .apply_typed_transaction_with_result(invalid_selection(&invalid_fast))
            .unwrap_err();
        assert_eq!(invalid_fast_error, invalid_slow_error);
        assert_eq!(atomic_audit(&invalid_fast), before_invalid_fast);
    }

    #[test]
    fn empty_skip_fast_path_matches_full_compiler_for_selection_forms_and_local_state() {
        fn populated_engine() -> YrsDocumentEngine {
            let mut engine = transaction_engine();
            engine
                .import_json(
                    &json!({
                        "type": "doc",
                        "content": [
                            {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                            {"type": "horizontalRule"},
                            {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                        ]
                    })
                    .to_string(),
                    TransactionOrigin::DocumentImport,
                )
                .unwrap();
            engine
        }

        fn transaction(
            engine: &YrsDocumentEngine,
            request_id: u64,
            selection_intent: SelectionIntent,
        ) -> TypedTransaction {
            TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent,
                history_policy: HistoryPolicy::Skip,
            }
        }

        fn slow_result(
            engine: &mut YrsDocumentEngine,
            transaction: TypedTransaction,
        ) -> crate::yrs_engine::TypedTransactionResult {
            let compiled = engine.compile_typed_transaction(transaction).unwrap();
            engine
                .apply_compiled_transaction(compiled, true)
                .unwrap()
                .1
                .unwrap()
        }

        let scalar = |offset, affinity| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity,
        };
        let utf16 = |offset, affinity| RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Utf16,
            affinity,
        };
        let intents = [
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(2, Affinity::Before),
                head: scalar(2, Affinity::After),
            }),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: utf16(3, Affinity::Before),
                head: utf16(3, Affinity::After),
            }),
            SelectionIntent::Set(SelectionInput::Node {
                at: scalar(4, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::All),
            SelectionIntent::Preserve,
            SelectionIntent::UseOperationResult,
        ];

        for (index, intent) in intents.into_iter().enumerate() {
            let mut fast = populated_engine();
            let mut slow = populated_engine();
            let fast_before = atomic_audit(&fast);
            let slow_before = atomic_audit(&slow);
            let fast_transaction = transaction(&fast, 770 + index as u64, intent.clone());
            let slow_transaction = transaction(&slow, 770 + index as u64, intent.clone());

            let fast_result = fast
                .apply_typed_transaction_with_result(fast_transaction)
                .unwrap();
            let slow_result = slow_result(&mut slow, slow_transaction);

            assert_eq!(fast_result, slow_result, "intent={intent:?}");
            assert_eq!(
                fast.document_json(),
                slow.document_json(),
                "intent={intent:?}"
            );
            assert_eq!(
                fast.document_html(),
                slow.document_html(),
                "intent={intent:?}"
            );
            assert_eq!(fast.revision(), slow.revision(), "intent={intent:?}");
            assert_eq!(
                fast.state_revision(),
                slow.state_revision(),
                "intent={intent:?}"
            );
            assert_eq!(
                fast.resolved_selection(),
                slow.resolved_selection(),
                "intent={intent:?}"
            );
            assert_eq!(
                fast.stored_marks(),
                slow.stored_marks(),
                "intent={intent:?}"
            );
            assert_eq!(fast.can_undo(), slow.can_undo(), "intent={intent:?}");
            assert_eq!(fast.can_redo(), slow.can_redo(), "intent={intent:?}");
            assert_eq!(fast.encoded_state().unwrap(), fast_before.encoded);
            assert_eq!(slow.encoded_state().unwrap(), slow_before.encoded);
            assert_eq!(fast.yrs_state_epoch, fast_before.yrs_state_epoch);
            assert_eq!(slow.yrs_state_epoch, slow_before.yrs_state_epoch);
            assert_eq!(
                fast.history.replay_audit_for_test(),
                fast_before.replay_audit
            );
            assert_eq!(
                slow.history.replay_audit_for_test(),
                slow_before.replay_audit
            );
        }

        let stored_mark_intents = [
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(1, Affinity::Before),
                head: scalar(1, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: scalar(2, Affinity::Before),
                head: scalar(2, Affinity::Before),
            }),
            SelectionIntent::Set(SelectionInput::Node {
                at: scalar(4, Affinity::Before),
            }),
        ];
        for (index, intent) in stored_mark_intents.into_iter().enumerate() {
            let mut fast = populated_engine();
            let mut slow = populated_engine();
            select_text(&mut fast, 780, 1, 1);
            select_text(&mut slow, 780, 1, 1);
            for engine in [&mut fast, &mut slow] {
                engine
                    .apply_command(
                        781,
                        TypedCommand::ToggleMark {
                            mark_type: "bold".into(),
                        },
                    )
                    .unwrap()
                    .unwrap();
                assert!(engine.stored_marks().is_some());
            }
            let fast_transaction = transaction(&fast, 782 + index as u64, intent.clone());
            let slow_transaction = transaction(&slow, 782 + index as u64, intent.clone());

            let fast_result = fast
                .apply_typed_transaction_with_result(fast_transaction)
                .unwrap();
            let slow_result = slow_result(&mut slow, slow_transaction);

            assert_eq!(fast_result, slow_result, "stored intent={intent:?}");
            assert_eq!(
                fast.resolved_selection(),
                slow.resolved_selection(),
                "stored intent={intent:?}"
            );
            assert_eq!(
                fast.stored_marks(),
                slow.stored_marks(),
                "stored intent={intent:?}"
            );
            if index == 0 {
                assert!(fast.stored_marks().is_some());
            } else {
                assert!(fast.stored_marks().is_none());
            }
        }
    }

    #[test]
    fn remote_history_admission_failure_retains_dependency_quarantine_for_retry() {
        use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
        use yrs::{diff_updates_v1, encode_state_vector_from_update_v1};

        let mut source = transaction_engine();
        let base = source.encoded_state().unwrap();
        source
            .apply_command(200, TypedCommand::InsertText { text: "a".into() })
            .unwrap();
        let after_a = source.encoded_state().unwrap();
        source
            .apply_command(201, TypedCommand::InsertText { text: "b".into() })
            .unwrap();
        let after_b = source.encoded_state().unwrap();
        let base_sv = encode_state_vector_from_update_v1(&base).unwrap();
        let after_a_sv = encode_state_vector_from_update_v1(&after_a).unwrap();
        let delta_a = diff_updates_v1(&after_a, &base_sv).unwrap();
        let delta_b = diff_updates_v1(&after_b, &after_a_sv).unwrap();

        let mut target = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
        assert!(
            !target
                .apply_remote_update_v1(202, &delta_b)
                .unwrap()
                .changed
        );
        assert!(
            !target
                .apply_remote_update_v1(203, &delta_a)
                .unwrap()
                .changed
        );
        let before = atomic_audit(&target);

        set_atomic_failpoint_for_test(Some(AtomicFailpoint::RemoteHistoryAdmission));
        let error = target.apply_remote_update_v1(204, &base).unwrap_err();
        set_atomic_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": "remoteHistoryAdmission" }))
        );
        assert_eq!(atomic_audit(&target), before);
        let retry = target.apply_remote_update_v1(205, &base).unwrap();
        assert!(retry.changed);
        assert_eq!(target.document().unwrap().root().text_content(), "ab");
        assert_eq!(target.encoded_state().unwrap(), after_b);
    }

    #[test]
    fn state_only_boundary_reservation_failure_is_fully_atomic() {
        use crate::yrs_engine::history::set_boundary_reservation_failure_for_test;

        let mut engine = transaction_engine();
        let before = atomic_audit(&engine);
        set_boundary_reservation_failure_for_test(true);

        let error = engine
            .apply_command(
                90,
                crate::yrs_engine::TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap_err();

        set_boundary_reservation_failure_for_test(false);
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(atomic_audit(&engine), before);
    }
}
