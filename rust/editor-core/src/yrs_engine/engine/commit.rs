use super::candidate_cache::{seal_candidate_state_vector, PreparedCandidateCache};
use super::commit_installation::{PreparedCompiledCommit, PreparedCompiledHistory};
use super::compilation::validate_compiled_selection_plans;
use super::history_state::{
    history_document_snapshots_fit, history_local_state, history_snapshot_template,
};
use super::outbound::OutboundUpdateSink;
use super::selection_commit::SelectionCommitContext;
#[cfg(test)]
use super::test_hooks::{
    begin_compiled_commit_preparation_for_test, check_compiled_commit_preparation_stage_for_test,
    record_compiled_commit_authority_validation_for_test,
    record_compiled_commit_live_view_for_test, CompiledCommitPreparationStage,
    COMMIT_CURRENT_STATE_ENCODINGS, COMMIT_SEALED_STATE_REUSES, PREPARED_CANDIDATE_CACHE_HITS,
    PREPARED_CANDIDATE_FULL_BOOTSTRAPS,
};
use super::transaction_result::cached_transition_render_update;
use super::{checked_operation_increment, YrsDocumentEngine};
use crate::yrs_engine;
use crate::yrs_engine::compiler::{
    CompiledTransaction, RelativeSelectionPlan, SelectionPlan, StoredMarksPlan,
};
use crate::yrs_engine::derived_state::operation_result_to_relative;
use crate::yrs_engine::mutation::{execute_mutation_plan, preflight_mutation_plan};
use std::sync::Arc;
use yrs::branch::Branch;
use yrs::types::xml::XmlFragmentRef;
use yrs::updates::decoder::Decode;
use yrs::{ReadTxn, StateVector, Transact, Transaction, Update};

enum CompiledCommitDerivedAuthority<'a> {
    Staged(yrs_engine::prepared_admission::StagedDerivedStateAuthority<'a>),
    Installed(yrs_engine::prepared_admission::InstalledDerivedStateAuthority<'a>),
}

pub(super) struct CompiledCommitAuthority<'a, 'doc> {
    derived: CompiledCommitDerivedAuthority<'a>,
    txn: &'a Transaction<'doc>,
    fragment: &'a XmlFragmentRef,
    state_vector: std::cell::OnceCell<StateVector>,
}

impl CompiledCommitAuthority<'_, '_> {
    pub(super) fn derived(&self) -> &dyn yrs_engine::prepared_admission::DerivedStateAuthority {
        match &self.derived {
            CompiledCommitDerivedAuthority::Staged(authority) => authority,
            CompiledCommitDerivedAuthority::Installed(authority) => authority,
        }
    }

    pub(super) fn txn(&self) -> &Transaction<'_> {
        self.txn
    }

    pub(super) fn fragment(&self) -> &XmlFragmentRef {
        self.fragment
    }

    pub(super) fn state_vector(&self) -> &StateVector {
        self.state_vector.get_or_init(|| self.txn.state_vector())
    }
}

impl YrsDocumentEngine {
    pub(super) fn apply_compiled_transaction_with_history_and_context(
        &mut self,
        mut compiled: CompiledTransaction,
        with_result: bool,
        prepared_history: Option<yrs_engine::prepared_admission::PreparedCommandHistoryAdmission>,
        prepared_context: Option<yrs_engine::prepared_admission::PreparedMutationContext>,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<(
        yrs_engine::TransactionCommit,
        Option<yrs_engine::TypedTransactionResult>,
    )> {
        #[cfg(test)]
        begin_compiled_commit_preparation_for_test();
        // A compiled plan owns Yrs handles after its original read transaction
        // closes. Reject a stale plan in O(1) before no-op classification or
        // any state-vector/snapshot traversal.
        if compiled.yrs_state_epoch != self.yrs_state_epoch
            || compiled.base_state_revision != self.state_revision
        {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled Yrs transaction is stale",
            ));
        }
        let installed = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(compiled.request_id))?;
        let authority_doc = self.doc.clone();
        #[cfg(test)]
        record_compiled_commit_live_view_for_test();
        let authority_txn = authority_doc.transact();
        let authority_fragment = authority_txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "compiled transaction lost its live Yrs fragment",
                )
            })?;
        let derived = if let Some(context) = prepared_context.as_ref() {
            #[cfg(test)]
            record_compiled_commit_authority_validation_for_test();
            CompiledCommitDerivedAuthority::Staged(context.authority(
                yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                    request_id: compiled.request_id,
                    installed,
                    txn: &authority_txn,
                    fragment: &authority_fragment,
                    fragment_name: &self.fragment_name,
                    schema_fingerprint: &self.schema_fingerprint,
                    resource_limits: &self.resource_limits,
                    editing_limits: &self.editing_limits,
                    max_length: self.max_length,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    yrs_state_epoch: self.yrs_state_epoch,
                },
            )?)
        } else {
            CompiledCommitDerivedAuthority::Installed(
                yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(installed),
            )
        };
        let commit_authority = CompiledCommitAuthority {
            derived,
            txn: &authority_txn,
            fragment: &authority_fragment,
            state_vector: std::cell::OnceCell::new(),
        };
        let preview_is_unchanged = compiled.preview
            == *self
                .document()
                .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(compiled.request_id))?;
        if preview_is_unchanged != compiled.mutation_plan.is_empty() {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "compiled preview and Yrs mutation plan disagree about document changes",
            ));
        }
        let mut prepared_history_limits = None;
        let mut prepared_history_before = None;
        let mut prepared_history_after = None;
        let mut prepared_history_render = None;
        if let Some(admission) = prepared_history {
            if preview_is_unchanged
                || !admission
                    .candidate_render
                    .cache
                    .matches_identity(&compiled.preview, &self.schema_fingerprint)
                || admission.candidate_derivations.rendered_scalars
                    != admission.candidate_derivations.position_map.total_scalars()
            {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "prepared command history admission does not match compiled output",
                ));
            }
            compiled.preview_derivations = Some(admission.candidate_derivations);
            prepared_history_limits = Some(admission.limits);
            prepared_history_before = Some(admission.before);
            prepared_history_after = Some(admission.after);
            prepared_history_render = Some(admission.candidate_render);
        }
        validate_compiled_selection_plans(&compiled)?;
        let had_active_state_certificate = self.derived_state.as_ref().is_some_and(
            yrs_engine::derived_state::DerivedStateCache::has_active_state_certificate,
        );
        let render_transition = if preview_is_unchanged {
            None
        } else if let Some(transition) = prepared_history_render.take() {
            Some(transition)
        } else {
            Some(self.prepare_commit_render_transition(&compiled)?)
        };
        let render_update = render_transition
            .as_ref()
            .map(|transition| cached_transition_render_update(&transition.update))
            .unwrap_or(yrs_engine::RenderUpdate::None);
        let prepared_result = with_result
            .then(|| self.prepare_typed_result(&compiled, render_update, &commit_authority))
            .transpose()?;
        let (mut result, prepared_active_cache) = match prepared_result {
            Some((result, cache)) => (Some(result), cache),
            None => (None, None),
        };
        if preview_is_unchanged {
            let prepared = Self::prepare_selection_commit(
                SelectionCommitContext {
                    current: self.derived_state.as_ref(),
                    schema: &self.schema,
                    history: &mut self.history,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                },
                &compiled,
                &commit_authority,
                had_active_state_certificate,
            )?;
            drop(commit_authority);
            drop(authority_txn);
            drop(authority_doc);
            return self.install_selection_commit(&compiled, prepared, result);
        }

        #[cfg(test)]
        yrs_engine::compiler::check_atomic_failpoint(
            compiled.request_id,
            yrs_engine::compiler::AtomicFailpoint::CanonicalOutputAdmission,
        )?;
        let canonical_artifact = compiled.canonical_artifact.take().ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "changed transaction has no admitted canonical artifact",
            )
        })?;

        // Revalidate sealed signatures against one final stable read view.
        let current_encoded_state = {
            #[cfg(test)]
            yrs_engine::compiler::check_atomic_failpoint(
                compiled.request_id,
                yrs_engine::compiler::AtomicFailpoint::FinalPreflight,
            )?;
            match (
                compiled.prepared_selection_state.as_ref(),
                compiled.prepared_selection_mutation_seal.as_ref(),
            ) {
                (Some(prepared), Some(seal)) => {
                    if !seal.matches(&compiled, commit_authority.derived()) {
                        return Err(yrs_engine::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "prepared selection core seal does not match compiled transaction",
                        ));
                    }
                    let rematerialized =
                        compiled
                            .localized_insert_admission
                            .as_ref()
                            .and_then(|admission| {
                                self.materialize_prewrite_selection_state(
                                    &compiled,
                                    admission,
                                    commit_authority.txn(),
                                )
                            });
                    if rematerialized.as_ref() != Some(prepared) {
                        compiled.prepared_selection_state = None;
                        compiled.prepared_selection_mutation_seal = None;
                        yrs_engine::derived_state::record_prewrite_selection_proof_fallback();
                    }
                }
                (Some(_), None) | (None, Some(_)) => {
                    return Err(yrs_engine::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "prepared selection state and core seal lifecycle disagree",
                    ));
                }
                (None, None) => {}
            }
            preflight_mutation_plan(
                compiled.request_id,
                &compiled.mutation_plan,
                commit_authority.txn(),
            )?;
            #[cfg(test)]
            yrs_engine::compiler::check_atomic_failpoint(
                compiled.request_id,
                yrs_engine::compiler::AtomicFailpoint::EncodedAdmission,
            )?;
            if commit_authority.state_vector().is_empty() {
                Vec::new()
            } else if let Some(encoded_state) =
                self.prepared_candidate_cache.as_mut().and_then(|cache| {
                    cache.take_matching_encoded_state(
                        &self.doc,
                        commit_authority.fragment(),
                        &compiled.mutation_plan,
                        self.revision,
                        self.yrs_state_epoch,
                        self.resource_limits.max_encoded_state_bytes,
                    )
                })
            {
                #[cfg(test)]
                COMMIT_SEALED_STATE_REUSES.set(COMMIT_SEALED_STATE_REUSES.get().saturating_add(1));
                encoded_state
            } else {
                #[cfg(test)]
                COMMIT_CURRENT_STATE_ENCODINGS
                    .set(COMMIT_CURRENT_STATE_ENCODINGS.get().saturating_add(1));
                commit_authority
                    .txn()
                    .encode_state_as_update_v1(&StateVector::default())
            }
        };
        let admitted_encoded_bytes = current_encoded_state
            .len()
            .checked_add(compiled.encoded_growth_bound)
            .ok_or_else(|| {
                yrs_engine::OperationError::document_limit_exceeded(
                    compiled.request_id,
                    None,
                    "maxEncodedStateBytes",
                    u64::try_from(self.resource_limits.max_encoded_state_bytes).unwrap_or(u64::MAX),
                    u64::MAX,
                )
            })?;
        if admitted_encoded_bytes > self.resource_limits.max_encoded_state_bytes {
            return Err(yrs_engine::OperationError::document_limit_exceeded(
                compiled.request_id,
                None,
                "maxEncodedStateBytes",
                u64::try_from(self.resource_limits.max_encoded_state_bytes).unwrap_or(u64::MAX),
                u64::try_from(admitted_encoded_bytes).unwrap_or(u64::MAX),
            ));
        }

        #[cfg(test)]
        yrs_engine::compiler::check_atomic_failpoint(
            compiled.request_id,
            yrs_engine::compiler::AtomicFailpoint::RevisionAdmission,
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
        let prepared_active_state_install = compiled
            .prepared_active_state_transition
            .as_ref()
            .zip(prepared_active_cache)
            .map(|(transition, cached)| {
                yrs_engine::derived_state::DerivedStateCache::prepare_active_state_install(
                    transition,
                    cached,
                    next_document_revision,
                    next_state_revision,
                    next_yrs_state_epoch,
                )
            });
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            compiled.request_id,
            CompiledCommitPreparationStage::DocumentValidation,
        )?;
        let had_prepared_candidate_validation = compiled.prepared_candidate_validation.is_some();
        let mut finalized_derived_evidence = compiled
            .prepared_candidate_validation
            .take()
            .and_then(|validation| {
                validation.finalize(
                    &compiled.preview,
                    &canonical_artifact,
                    compiled.preview_derivations.as_ref()?,
                    &self.schema,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    &self.canonical_schema,
                    next_document_revision,
                    next_state_revision,
                    next_yrs_state_epoch,
                )
            });
        if had_prepared_candidate_validation && finalized_derived_evidence.is_none() {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "prepared candidate validation diverged before durable mutation",
            ));
        }
        if finalized_derived_evidence.is_none() {
            finalized_derived_evidence =
                compiled
                    .prepared_derived_evidence
                    .take()
                    .and_then(|evidence| {
                        evidence.finalize(
                            commit_authority.derived(),
                            &compiled.preview,
                            &canonical_artifact,
                            compiled.preview_derivations.as_ref()?,
                            &render_transition.as_ref()?.cache,
                            &self.resource_limits,
                            &self.editing_limits,
                            self.max_length,
                            &self.schema_fingerprint,
                            next_document_revision,
                            next_state_revision,
                            next_yrs_state_epoch,
                        )
                    });
        }
        if compiled.localized_semantic_used && finalized_derived_evidence.is_none() {
            let authority = commit_authority.derived();
            let derivations = compiled.preview_derivations.as_ref().ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "localized derived evidence has no compiled derivations",
                )
            })?;
            finalized_derived_evidence = Some(
                authority
                    .installed()
                    .prepare_generic_derived_evidence(
                        compiled.request_id,
                        authority,
                        &compiled.preview,
                        &canonical_artifact,
                        derivations,
                        &self.schema,
                        &self.resource_limits,
                        &self.schema_fingerprint,
                        next_document_revision,
                        next_state_revision,
                        next_yrs_state_epoch,
                    )
                    .ok_or_else(|| {
                        yrs_engine::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "localized derived evidence could not be rebuilt before mutation",
                        )
                    })?,
            );
        }
        #[cfg(test)]
        yrs_engine::compiler::check_atomic_failpoint(
            compiled.request_id,
            yrs_engine::compiler::AtomicFailpoint::DurableMetadataAdmission,
        )?;
        let mut next_durable_client_ids = self.durable_client_ids.clone();
        if compiled.authored_clock_units > 0 {
            next_durable_client_ids.insert(self.client_id());
        }
        let captures_history = compiled.history_policy != yrs_engine::HistoryPolicy::Skip
            && compiled.history_class != yrs_engine::compiler::HistoryClass::Skip;
        let (history_before, history_after_template) = if captures_history {
            if let (Some(before), Some(after)) = (
                prepared_history_before.take(),
                prepared_history_after.take(),
            ) {
                let current = self
                    .derived_state
                    .as_ref()
                    .expect("captured history has a current derived state");
                if before.canonical_fingerprint != current.canonical_artifact.sha256()
                    || before.derived_output_bytes != current.canonical_artifact.serialized_len()
                    || after.canonical_fingerprint != canonical_artifact.sha256()
                    || after.derived_output_bytes != canonical_artifact.serialized_len()
                {
                    return Err(yrs_engine::OperationError::engine_invariant_failed(
                        compiled.request_id,
                        None,
                        "prepared command history snapshots do not match live artifacts",
                    ));
                }
                (Some(before), Some(after))
            } else {
                let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
                    unreachable!("stored-mark plan was sealed above")
                };
                let before = self
                    .derived_state
                    .as_ref()
                    .expect("captured history has a current derived state");
                // Optional history snapshots are admitted only from the exact
                // precomputed after-map/text derivations that will be installed.
                // If that evidence is unavailable, the normal full restore path
                // remains available and no potentially smaller estimate is used.
                let document_snapshot_retained_bytes = compiled
                    .preview_derivations
                    .as_ref()
                    .and_then(|after_derivations| {
                        history_document_snapshots_fit(
                            before,
                            &compiled.preview,
                            &canonical_artifact,
                            after_derivations,
                            &render_transition.as_ref()?.cache,
                            stored_marks.as_deref(),
                            &self.schema_fingerprint,
                            &self.fragment_name,
                            self.scope.as_ref(),
                            self.editing_limits.max_derived_output_bytes,
                        )
                    });
                let prepared = (
                    Some(history_local_state(
                        before,
                        &self.fragment_name,
                        self.scope.as_ref(),
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        document_snapshot_retained_bytes.map(|bytes| bytes.before),
                    )),
                    Some(history_snapshot_template(
                        &canonical_artifact,
                        stored_marks.as_deref(),
                        &self.fragment_name,
                        document_snapshot_retained_bytes.map(|bytes| bytes.after),
                    )),
                );
                prepared
            }
        } else {
            if prepared_history_limits.is_some()
                || prepared_history_before.is_some()
                || prepared_history_after.is_some()
            {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    compiled.request_id,
                    None,
                    "prepared history admission was supplied for a non-capturing command",
                ));
            }
            (None, None)
        };
        let history_after_metadata_bytes = history_after_template
            .as_ref()
            .map(|template| template.metadata_bytes)
            .unwrap_or(0);

        let outbound_update_upper_bound = compiled.outbound_update_upper_bound();
        let CompiledTransaction {
            request_id,
            origin,
            history_policy,
            history_class,
            undo_units_bound,
            replay_work_units_bound,
            encoded_growth_bound,
            authored_clock_units,
            preview,
            preview_derivations,
            selection_plan,
            relative_selection_plan,
            stored_marks_plan,
            composed_map,
            position_update_mode,
            affected_top_level_blocks,
            mutation_plan,
            mutation_lookup_transition,
            prepared_selection_state,
            ..
        } = compiled;
        let mut prepared_mutation_lookup_seed =
            if let Some(transition) = mutation_lookup_transition.as_ref() {
                #[cfg(test)]
                check_compiled_commit_preparation_stage_for_test(
                    request_id,
                    CompiledCommitPreparationStage::LookupTransition,
                )?;
                Some(self.prepare_mutation_lookup_transition_with_authority(
                    request_id,
                    commit_authority.derived(),
                    transition,
                    commit_authority.txn(),
                    commit_authority.fragment(),
                    &preview,
                    &canonical_artifact,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?)
            } else {
                None
            };
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            request_id,
            CompiledCommitPreparationStage::AllocationProbe,
        )?;
        let next_render_blocks = Arc::new(
            render_transition
                .expect("changed transaction has a prepared render transition")
                .cache,
        );
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            request_id,
            CompiledCommitPreparationStage::HistoryReservation,
        )?;
        // Preserve the baseline lookup/render error precedence while admitting
        // history before all newly introduced candidate-store work.
        let prepared_history = if captures_history {
            PreparedCompiledHistory::Recorded(self.history.pre_admit_recorded(
                request_id,
                origin,
                history_policy,
                history_class,
                undo_units_bound,
                history_before,
                history_after_metadata_bytes,
                &current_encoded_state,
                encoded_growth_bound,
                prepared_history_limits,
            )?)
        } else {
            PreparedCompiledHistory::Excluded(self.history.pre_admit_compiled_excluded(
                request_id,
                origin,
                replay_work_units_bound,
                &current_encoded_state,
                encoded_growth_bound,
            )?)
        };
        #[cfg(test)]
        check_compiled_commit_preparation_stage_for_test(
            request_id,
            CompiledCommitPreparationStage::OperationPreparation,
        )?;
        let cached_candidate = self.prepared_candidate_cache.take();
        let cached_candidate = cached_candidate
            .and_then(|cache| cache.into_matching_doc(self.revision, self.yrs_state_epoch));
        let (candidate_doc, candidate_state_vector) = if let Some(cached) = cached_candidate {
            #[cfg(test)]
            PREPARED_CANDIDATE_CACHE_HITS
                .set(PREPARED_CANDIDATE_CACHE_HITS.get().saturating_add(1));
            cached
        } else {
            #[cfg(test)]
            PREPARED_CANDIDATE_FULL_BOOTSTRAPS
                .set(PREPARED_CANDIDATE_FULL_BOOTSTRAPS.get().saturating_add(1));
            let candidate_doc = self.new_history_candidate_doc();
            // Root shared types are not encoded until they contain structs.
            // Create the configured fragment explicitly so a valid empty root
            // can rebind its first structural mutation.
            let candidate_fragment =
                candidate_doc.get_or_insert_xml_fragment(self.fragment_name.as_str());
            if AsRef::<Branch>::as_ref(&candidate_fragment).id()
                != AsRef::<Branch>::as_ref(commit_authority.fragment()).id()
            {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared commit candidate root identity does not match the live store",
                ));
            }
            if !current_encoded_state.is_empty() {
                let current_update =
                    Update::decode_v1(&current_encoded_state).map_err(|error| {
                        yrs_engine::OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            format!(
                                "admitted current Yrs state cannot seed commit candidate: {error}"
                            ),
                        )
                    })?;
                candidate_doc
                    .transact_mut()
                    .apply_update(current_update)
                    .map_err(|error| {
                        yrs_engine::OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            format!("admitted current Yrs state cannot initialize commit candidate: {error}"),
                        )
                    })?;
            }
            let candidate_state_vector = candidate_doc.transact().state_vector();
            if &candidate_state_vector != commit_authority.state_vector() {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared commit candidate state vector does not exactly match the live base",
                ));
            }
            (candidate_doc, candidate_state_vector)
        };
        if candidate_doc.client_id() != self.doc.client_id()
            || candidate_doc.guid() != self.doc.guid()
            || candidate_doc.offset_kind() != self.doc.offset_kind()
            || candidate_doc.skip_gc() != self.doc.skip_gc()
        {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "prepared commit candidate options do not exactly match the live store",
            ));
        }
        let authored_clock_bound = u32::try_from(authored_clock_units).map_err(|_| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "admitted authored clock bound exceeds the Yrs clock domain",
            )
        })?;
        let (history_update, mut next_derived_state, next_candidate_state_vector) = {
            let candidate_plan = {
                #[cfg(test)]
                check_compiled_commit_preparation_stage_for_test(
                    request_id,
                    CompiledCommitPreparationStage::DocumentValidation,
                )?;
                let txn = candidate_doc.transact();
                let candidate_plan = mutation_plan
                    .clone()
                    .rebind_to_equivalent_store(request_id, &txn)?;
                preflight_mutation_plan(request_id, &candidate_plan, &txn)?;
                candidate_plan
            };
            {
                let mut txn = candidate_doc.transact_mut();
                execute_mutation_plan(candidate_plan, &mut txn);
            }
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "prepared commit candidate lost its configured Yrs fragment",
                    )
                })?;
            #[cfg(test)]
            check_compiled_commit_preparation_stage_for_test(
                request_id,
                CompiledCommitPreparationStage::HistoryUpdateEncoding,
            )?;
            let history_update = txn.encode_state_as_update_v1(&candidate_state_vector);
            if history_update.len() > encoded_growth_bound {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared commit candidate exceeded the admitted encoded growth bound",
                ));
            }
            // Yrs can elide redundant formatting structs when the requested
            // attributes are already active. The compiler's authored units are
            // therefore an admitted hard ceiling, while this private execution
            // supplies the exact next seal. Only the local client may advance.
            let next_candidate_state_vector = seal_candidate_state_vector(
                request_id,
                &candidate_state_vector,
                txn.state_vector(),
                self.doc.client_id(),
                authored_clock_bound,
            )?;
            if prepared_mutation_lookup_seed.is_none() {
                let candidate_seed = yrs_engine::mutation::MutationLookupSeed::build(
                    request_id,
                    &txn,
                    &fragment,
                    &self.schema,
                    &preview,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?
                .with_canonical_artifact(&canonical_artifact);
                prepared_mutation_lookup_seed =
                    Some(Arc::new(candidate_seed.rebind_authoritative_store(
                        commit_authority.txn(),
                        commit_authority.fragment(),
                        &self.schema_fingerprint,
                        next_yrs_state_epoch,
                        next_document_revision,
                    )));
            }
            #[cfg(test)]
            check_compiled_commit_preparation_stage_for_test(
                request_id,
                CompiledCommitPreparationStage::DerivedStateBuild,
            )?;
            let explicit_relative_selection = match (&selection_plan, &prepared_selection_state) {
                (SelectionPlan::Explicit(_), Some(prepared)) => Some(prepared.relative().clone()),
                (SelectionPlan::Explicit(_), None)
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
                (SelectionPlan::Explicit(selection), None) => Some(operation_result_to_relative(
                    &txn,
                    &fragment,
                    selection,
                    &self.schema,
                )),
                (SelectionPlan::Mapped(_), _) | (SelectionPlan::Preserve, _) => None,
            };
            #[cfg(test)]
            check_compiled_commit_preparation_stage_for_test(
                request_id,
                CompiledCommitPreparationStage::SelectionFinalization,
            )?;
            let preserved_fallback = match &relative_selection_plan {
                RelativeSelectionPlan::PreserveWithFallback(selection) => Some(selection),
                RelativeSelectionPlan::Precomputed { fallback, .. } => Some(fallback),
                _ => None,
            };
            let strict_fallback_affinity = matches!(
                relative_selection_plan,
                RelativeSelectionPlan::Precomputed { .. }
            );
            let next = self
                .derived_state
                .as_ref()
                .and_then(|state| {
                    state.after_document_change(
                        preview.clone(),
                        canonical_artifact,
                        &txn,
                        &fragment,
                        &self.schema,
                        &self.schema_fingerprint,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        next_render_blocks,
                        preview_derivations,
                        &composed_map,
                        position_update_mode,
                        &affected_top_level_blocks,
                        explicit_relative_selection.as_ref(),
                        preserved_fallback,
                        strict_fallback_affinity,
                        prepared_mutation_lookup_seed,
                        prepared_selection_state,
                        finalized_derived_evidence,
                        next_document_revision,
                        next_state_revision,
                        next_yrs_state_epoch,
                    )
                })
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "prepared candidate must produce exact next derived editor state",
                    )
                })?;
            (history_update, next, next_candidate_state_vector)
        };
        let StoredMarksPlan::Set(stored_marks) = stored_marks_plan else {
            unreachable!()
        };
        next_derived_state.stored_marks = stored_marks;
        let prepared_active_state_certificate = prepared_active_state_install.and_then(|install| {
            let authority = yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
                &next_derived_state,
            );
            yrs_engine::derived_state::DerivedStateCache::prepare_active_state_certificate(
                install,
                &authority,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                next_yrs_state_epoch,
            )
        });
        let active_state_installed = prepared_active_state_certificate.is_some();
        if let Some(certificate) = prepared_active_state_certificate {
            next_derived_state.install_active_state_certificate(certificate);
        }
        let publish_active_state_drop = had_active_state_certificate && !active_state_installed;
        let history_after = if captures_history {
            let history_after_template = history_after_template
                .expect("captured history has an admitted after-state template");
            let document_snapshot = if let Some(retained_bytes) =
                history_after_template.document_snapshot_retained_bytes
            {
                #[cfg(test)]
                check_compiled_commit_preparation_stage_for_test(
                    request_id,
                    CompiledCommitPreparationStage::HistorySnapshotConstruction,
                )?;
                Some(next_derived_state.capture_history_document_snapshot(
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.fragment_name,
                    self.scope.as_ref(),
                    retained_bytes,
                ))
            } else {
                None
            };
            Some(history_after_template.seal(
                next_derived_state.relative_selection.clone(),
                next_derived_state.resolved_selection.clone(),
                document_snapshot,
            ))
        } else {
            None
        };
        debug_assert_eq!(next_derived_state.document_revision, next_document_revision);
        if let Some(result) = &mut result {
            result.request_id = request_id;
            result.origin = origin;
            result.changed = true;
            result.document_revision = next_document_revision;
            result.state_revision = next_state_revision;
            result.selection = next_derived_state.resolved_selection.clone();
            result.history_state = crate::editor_state::HistoryState {
                can_undo: captures_history || self.can_undo(),
                can_redo: !captures_history && self.can_redo(),
            };
        }
        drop(commit_authority);
        drop(authority_txn);
        drop(authority_doc);
        drop(prepared_context);
        let next_candidate_cache = Some(PreparedCandidateCache {
            doc: candidate_doc,
            state_vector: next_candidate_state_vector,
            staged_lookup_seed: None,
            document_revision: next_document_revision,
            yrs_state_epoch: next_yrs_state_epoch,
            encoded_state_seal: None,
        });
        let mut prepared = PreparedCompiledCommit {
            request_id,
            origin,
            history_policy,
            history: Some(prepared_history),
            mutation_plan: Some(mutation_plan),
            history_update,
            history_after,
            next_derived_state: Some(next_derived_state),
            next_durable_client_ids,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
            publish_active_state_install: active_state_installed,
            publish_active_state_drop,
            result,
            next_candidate_cache,
        };
        // Frozen local mutation flow: reserve bounded outbox count/bytes and
        // stage the candidate-captured Update-v1 from the compiler's
        // conservative bound BEFORE the irreversible Yrs write. Saturation or
        // reservation failure rejects here atomically; the invariant check on
        // `history_update` above already proved `actual <= admitted bound`.
        outbound.reserve_and_stage(
            prepared.request_id,
            outbound_update_upper_bound,
            &prepared.history_update,
        )?;
        self.execute_prepared_yrs_write(&mut prepared);
        let committed = self.install_prepared_changed_commit(prepared);
        outbound.commit_staged();
        Ok(committed)
    }
}
