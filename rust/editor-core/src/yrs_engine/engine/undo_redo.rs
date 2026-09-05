use super::candidate_cache::encode_state_bounded;
use super::history_state::history_operation_error;
use super::outbound::OutboundUpdateSink;
use super::transaction_result::cached_transition_render_update;
use super::{checked_operation_increment, YrsDocumentEngine};
use crate::serialize::{
    from_prosemirror_json_with_limits, rehydrate_reserved_html_opaque, UnknownTypeMode,
};
use crate::transform::{canonicalize_yrs_document, DocumentValidator};
use crate::yrs_engine;
use crate::yrs_engine::derived_state::{history_selection_to_relative, DerivedStateCache};
use crate::yrs_engine::{TransactionOrigin, YrsDocumentCodec};
use std::sync::Arc;
use yrs::{Doc, OffsetKind, Options, ReadTxn, Transact};

struct PreparedHistoryCandidateState {
    state: DerivedStateCache,
    encoded_state: Vec<u8>,
    candidate_publication: Option<yrs_engine::derived_state::HistoryMutationLookupCapability>,
}

struct PreparedHistoryPop {
    request_id: u64,
    candidate_doc: Doc,
    candidate_history: yrs_engine::history::YrsHistory,
    candidate_state: DerivedStateCache,
    candidate_publication: Option<yrs_engine::derived_state::HistoryMutationLookupCapability>,
    next_document_revision: u64,
    next_state_revision: u64,
    next_yrs_state_epoch: u64,
    result: Option<yrs_engine::TypedTransactionResult>,
}

impl YrsDocumentEngine {
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    #[allow(dead_code)]
    pub fn undo(
        &mut self,
        request_id: u64,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TransactionCommit>> {
        Ok(self
            .apply_history_pop(request_id, true, false, &mut OutboundUpdateSink::detached())?
            .map(|(commit, _)| commit))
    }

    #[allow(dead_code)]
    pub fn redo(
        &mut self,
        request_id: u64,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TransactionCommit>> {
        Ok(self
            .apply_history_pop(
                request_id,
                false,
                false,
                &mut OutboundUpdateSink::detached(),
            )?
            .map(|(commit, _)| commit))
    }

    /// Production surface: [`Self::undo`] with an optionally attached
    /// collaboration outbox. The pop's outbound update is captured on the
    /// prepared candidate and reserved before the infallible install.
    pub(crate) fn undo_with_outbox(
        &mut self,
        request_id: u64,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TransactionCommit>> {
        Ok(self
            .apply_history_pop(
                request_id,
                true,
                false,
                &mut OutboundUpdateSink::from_optional_outbox(outbox),
            )?
            .map(|(commit, _)| commit))
    }

    /// Production surface: [`Self::redo`] with an optionally attached
    /// collaboration outbox.
    pub(crate) fn redo_with_outbox(
        &mut self,
        request_id: u64,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TransactionCommit>> {
        Ok(self
            .apply_history_pop(
                request_id,
                false,
                false,
                &mut OutboundUpdateSink::from_optional_outbox(outbox),
            )?
            .map(|(commit, _)| commit))
    }

    #[allow(dead_code)]
    pub fn undo_with_result(
        &mut self,
        request_id: u64,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TypedTransactionResult>> {
        Ok(self
            .apply_history_pop(request_id, true, true, &mut OutboundUpdateSink::detached())?
            .and_then(|(_, result)| result))
    }

    #[allow(dead_code)]
    pub fn redo_with_result(
        &mut self,
        request_id: u64,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TypedTransactionResult>> {
        Ok(self
            .apply_history_pop(request_id, false, true, &mut OutboundUpdateSink::detached())?
            .and_then(|(_, result)| result))
    }

    pub(super) fn apply_history_pop(
        &mut self,
        request_id: u64,
        undoing: bool,
        with_result: bool,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<
        Option<(
            yrs_engine::TransactionCommit,
            Option<yrs_engine::TypedTransactionResult>,
        )>,
    > {
        let Some(prepared) = self.prepare_history_pop(request_id, undoing, with_result)? else {
            return Ok(None);
        };
        self.commit_prepared_history_pop(prepared, outbound)
            .map(Some)
    }

    fn prepare_history_pop(
        &self,
        request_id: u64,
        undoing: bool,
        with_result: bool,
    ) -> yrs_engine::OperationResult<Option<PreparedHistoryPop>> {
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
            yrs_engine::history::HistoryAction::Undo
        } else {
            yrs_engine::history::HistoryAction::Redo
        };
        let candidate_doc = self.new_history_candidate_doc();
        self.history.seed_candidate(request_id, &candidate_doc)?;
        let candidate_fragment =
            candidate_doc.get_or_insert_xml_fragment(self.fragment_name.as_str());
        let mut candidate_history =
            self.history
                .replay_into(request_id, &candidate_doc, &candidate_fragment)?;
        let candidate_pop = match action {
            yrs_engine::history::HistoryAction::Undo => candidate_history.undo(),
            yrs_engine::history::HistoryAction::Redo => candidate_history.redo(),
        };
        if !candidate_pop.changed {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "bounded history replay cannot reproduce the next live pop",
            ));
        }
        let restored_slot = candidate_pop.restored.as_ref().ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "changed history candidate supplied no restoration metadata",
            )
        })?;
        let restored = restored_slot.get().ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "changed history candidate supplied an unsealed restoration snapshot",
            )
        })?;
        let PreparedHistoryCandidateState {
            state: candidate_state,
            encoded_state: candidate_encoded_state,
            candidate_publication,
        } = self.derive_history_candidate_state(
            request_id,
            &candidate_doc,
            restored,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
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
        Ok(Some(PreparedHistoryPop {
            request_id,
            candidate_doc,
            candidate_history,
            candidate_state,
            candidate_publication,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
            result,
        }))
    }

    fn commit_prepared_history_pop(
        &mut self,
        mut prepared: PreparedHistoryPop,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        let ready_seed = {
            let txn = prepared.candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        prepared.request_id,
                        None,
                        "prepared history candidate lost its configured fragment",
                    )
                })?;
            let seed = if let Some(capability) = prepared.candidate_publication.take() {
                capability.prepare_candidate_publication(
                    prepared.request_id,
                    &txn,
                    &fragment,
                    &self.schema,
                    &prepared.candidate_state.document,
                    &prepared.candidate_state.canonical_artifact,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    prepared.next_yrs_state_epoch,
                    prepared.next_document_revision,
                )?
            } else {
                Arc::clone(&prepared.candidate_state.mutation_lookup_seed)
            };
            if !seed.matches_canonical_artifact(&prepared.candidate_state.canonical_artifact)
                || !seed.matches(
                    &txn,
                    &fragment,
                    &prepared.candidate_state.document,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    prepared.next_yrs_state_epoch,
                    prepared.next_document_revision,
                )
            {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    prepared.request_id,
                    None,
                    "prepared history candidate has no matching ready mutation seed",
                ));
            }
            seed
        };
        prepared.candidate_state.mutation_lookup_seed = ready_seed;
        if outbound.is_attached() {
            // Standard reconciliation update for the pop: the candidate's
            // new pop-authored structs against the live store's state vector
            // plus the candidate's complete delete set (redundant deletes are
            // idempotent for peers). It is captured on the fully prepared
            // candidate BEFORE the infallible install, so its exact length is
            // the admitted conservative bound (actual == bound).
            let outbound_update = {
                let live_state_vector = self.doc.transact().state_vector();
                prepared
                    .candidate_doc
                    .transact()
                    .encode_state_as_update_v1(&live_state_vector)
            };
            outbound.reserve_and_stage(
                prepared.request_id,
                outbound_update.len(),
                &outbound_update,
            )?;
        }
        let installed = self.install_prepared_history_pop(prepared);
        outbound.commit_staged();
        Ok(installed)
    }

    fn install_prepared_history_pop(
        &mut self,
        prepared: PreparedHistoryPop,
    ) -> (
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    ) {
        // All recoverable candidate work is complete. Installation below is
        // infallible ownership transfer plus admitted scalar assignments.
        self.doc = prepared.candidate_doc;
        // Same client identity and logical session: awareness migrates every
        // live state (local and remote) with clocks intact.
        if let Some(awareness) = self.awareness.as_mut() {
            awareness.rebind_preserving_peers(&self.doc);
        }
        self.history = prepared.candidate_history;
        self.derived_state = Some(prepared.candidate_state);
        self.revision = prepared.next_document_revision;
        self.state_revision = prepared.next_state_revision;
        self.yrs_state_epoch = prepared.next_yrs_state_epoch;
        self.last_committed_origin = Some(TransactionOrigin::UndoRedo);
        self.document_origin = yrs_engine::DocumentOrigin::History;
        self.prepared_candidate_cache = None;
        (
            yrs_engine::TransactionCommit {
                request_id: prepared.request_id,
                changed: true,
                document_revision: self.revision,
                state_revision: self.state_revision,
                origin: TransactionOrigin::UndoRedo,
            },
            prepared.result,
        )
    }

    fn prepare_history_result(
        &self,
        request_id: u64,
        candidate: &DerivedStateCache,
    ) -> yrs_engine::OperationResult<yrs_engine::TypedTransactionResult> {
        let current = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
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
        let render_update =
            cached_transition_render_update(&current.render_blocks.classify_transition_to(
                &current.document,
                &candidate.document,
                &candidate.render_blocks,
                &[],
            ));
        let result = yrs_engine::TypedTransactionResult {
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

    pub(super) fn new_history_candidate_doc(&self) -> Doc {
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
        restored: &yrs_engine::history::HistoryLocalState,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> yrs_engine::OperationResult<PreparedHistoryCandidateState> {
        let txn = doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history result fragment is missing",
                )
            })?;
        let (derived_json, history_admission) =
            if let Some(document_snapshot) = restored.document_snapshot.as_deref() {
                document_snapshot
                    .prepare_candidate_read(
                        request_id,
                        &txn,
                        &fragment,
                        &self.schema,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        &self.schema_fingerprint,
                        &self.fragment_name,
                        self.scope.as_ref(),
                        yrs_state_epoch,
                        document_revision,
                    )?
                    .into_parts()
            } else {
                let json = YrsDocumentCodec::new(&self.schema, &self.resource_limits)
                    .read_json(&fragment, &txn)
                    .map_err(|error| history_operation_error(request_id, error))?;
                (json, None)
            };
        crate::transform::validate_input_mark_set(
            restored.stored_marks.as_deref().unwrap_or_default(),
            &self.schema,
        )
        .map_err(|error| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("history metadata contains invalid stored marks: {error}"),
            )
        })?;
        if let (Some(document_snapshot), Some(admission)) =
            (restored.document_snapshot.as_deref(), history_admission)
        {
            let restored_state = DerivedStateCache::restore_history_document_snapshot(
                request_id,
                document_snapshot,
                admission,
                &txn,
                &fragment,
                &self.schema,
                &restored.relative_selection,
                &restored.resolved_selection,
                restored.stored_marks.clone(),
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )?;
            if let Some(restored) = restored_state {
                drop(txn);
                let encoded = encode_state_bounded(doc, &self.resource_limits)
                    .map_err(|error| history_operation_error(request_id, error))?;
                return Ok(PreparedHistoryCandidateState {
                    state: restored.state,
                    encoded_state: encoded,
                    candidate_publication: Some(restored.candidate_publication),
                });
            }
        }
        let document = from_prosemirror_json_with_limits(
            &derived_json,
            &self.schema,
            UnknownTypeMode::Preserve,
            &self.resource_limits,
        )
        .map_err(|error| {
            yrs_engine::OperationError::document_invalid(
                request_id,
                None,
                "document",
                error.to_string(),
            )
        })?;
        let document =
            canonicalize_yrs_document(&rehydrate_reserved_html_opaque(&document), &self.schema);
        DocumentValidator::validate(&document, &self.schema, &self.resource_limits).map_err(
            |error| {
                if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                    yrs_engine::OperationError::document_limit_exceeded(
                        request_id,
                        None,
                        "document",
                        error.limit.unwrap_or(0) as u64,
                        error.actual.unwrap_or(0) as u64,
                    )
                } else {
                    yrs_engine::OperationError::document_invalid(
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
                return Err(yrs_engine::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual,
                ));
            }
        }
        let canonical_artifact = self.canonical_schema.derive(&document).map_err(|error| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("history result serialization failed: {error}"),
            )
        })?;
        if canonical_artifact.serialized_len() > self.editing_limits.max_derived_output_bytes {
            return Err(yrs_engine::OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxDerivedOutputBytes",
                u64::try_from(self.editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(canonical_artifact.serialized_len()).unwrap_or(u64::MAX),
            ));
        }
        let canonical_fingerprint = canonical_artifact.sha256();
        // Yrs recreates inserted structs with new IDs during redo, so the
        // original relative cursor can remain valid yet resolve beside the
        // redone content. Preserve the document-relative snapshot as the CRDT
        // metadata and reseal it from the exact resolved fallback on restore.
        let restored_relative = if canonical_fingerprint == restored.canonical_fingerprint {
            history_selection_to_relative(
                &txn,
                &fragment,
                &restored.relative_selection,
                &restored.resolved_selection,
                &self.schema,
            )
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history selection affinity is not exactly representable in the candidate",
                )
            })?
        } else {
            restored.relative_selection.clone()
        };
        let stored_marks = restored
            .stored_marks
            .as_deref()
            .map(|marks| yrs_engine::derived_state::canonical_marks(marks, &self.schema));
        let state = DerivedStateCache::initialize_history(
            document,
            canonical_artifact,
            &txn,
            &fragment,
            &self.schema,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
            restored_relative,
            stored_marks,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
        .ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "history result cannot initialize derived editor state or resolve restoration selection",
            )
        })?;
        if !state
            .mutation_lookup_seed
            .matches_canonical_artifact(&state.canonical_artifact)
            || !state.mutation_lookup_seed.matches(
                &txn,
                &fragment,
                &state.document,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
        {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "generic history candidate has no matching ready mutation seed",
            ));
        }
        drop(txn);
        let encoded = encode_state_bounded(doc, &self.resource_limits)
            .map_err(|error| history_operation_error(request_id, error))?;
        Ok(PreparedHistoryCandidateState {
            state,
            encoded_state: encoded,
            candidate_publication: None,
        })
    }

    /// Production probe: the exact outbound Update-v1 length the next history
    /// pop would capture and reserve (`None` when nothing can pop). The pop
    /// path's conservative bound is this exact captured length.
    #[allow(dead_code)]
    pub(crate) fn probe_history_pop_outbound_bytes(
        &self,
        request_id: u64,
        undoing: bool,
    ) -> yrs_engine::OperationResult<Option<usize>> {
        let Some(prepared) = self.prepare_history_pop(request_id, undoing, false)? else {
            return Ok(None);
        };
        let live_state_vector = self.doc.transact().state_vector();
        let captured_len = {
            let candidate_txn = prepared.candidate_doc.transact();
            candidate_txn
                .encode_state_as_update_v1(&live_state_vector)
                .len()
        };
        Ok(Some(captured_len))
    }
}
