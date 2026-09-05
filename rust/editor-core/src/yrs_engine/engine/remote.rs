use super::candidate_cache::{
    encode_candidate_state_bounded, encode_state_bounded, fresh_utf16_doc_excluding,
};
use super::history_state::history_operation_error;
#[cfg(test)]
use super::test_hooks::FAIL_QUARANTINED_UPDATE_RESERVATION;
use super::transaction_result::{affinity_aware_mapped_selection, cached_render_operation_error};
use super::{
    checked_operation_increment, merge_operation_details, EngineCommit, YrsDocumentEngine,
};
use crate::position::update::UpdateMode;
use crate::serialize::{
    from_prosemirror_json_with_limits, rehydrate_reserved_html_opaque, JsonParseError,
    UnknownTypeMode,
};
use crate::transform::{DocumentValidator, StepMap};
use crate::yrs_engine;
use crate::yrs_engine::derived_state::{stored_marks_after_selection_change, DerivedStateCache};
use crate::yrs_engine::update_preflight::preflight_update_v1;
use crate::yrs_engine::{TransactionOrigin, YrsDocumentCodec, YrsEngineError};
use std::collections::HashSet;
use std::sync::Arc;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, ReadTxn, StateVector, Transact, Update};

struct PreparedRemoteSeal {
    request_id: u64,
    sealed_doc: Doc,
    admitted_revision: u64,
    admitted_state_revision: u64,
    admitted_epoch: u64,
    sealed_generation: u64,
}

enum PreparedDocumentOutcome {
    Unchanged,
    Changed(Box<PreparedRemoteInstall>),
}

enum PreparedDependencyState {
    Clear,
    Retain(Vec<u8>),
}

/// A one-shot remote Update-v1 admission, sealed to the exact store identity,
/// document revision, state revision, epoch, and quarantine generation that
/// admitted it. Everything fallible (decode preflight, dependency
/// classification, candidate admission, ceilings, and derived-state
/// preparation) already happened during preparation; committing atomically
/// installs the document and dependency candidates. Deliberately non-`Clone`
/// and consumed by value.
pub struct PreparedRemoteUpdate {
    seal: PreparedRemoteSeal,
    document: PreparedDocumentOutcome,
    dependencies: PreparedDependencyState,
}

impl PreparedRemoteUpdate {
    pub fn retained_dependency_bytes(&self) -> usize {
        match &self.dependencies {
            PreparedDependencyState::Clear => 0,
            PreparedDependencyState::Retain(bytes) => bytes.len(),
        }
    }

    pub fn has_pending_dependencies(&self) -> bool {
        matches!(self.dependencies, PreparedDependencyState::Retain(_))
    }
}

/// The proven installation payload for a changed remote update. Every field
/// was validated against the live store during preparation.
struct PreparedRemoteInstall {
    live_update: Update,
    accepted_update: Vec<u8>,
    history_admission: yrs_engine::history::PreparedExcludedHistoryAdmission,
    next_state: DerivedStateCache,
    prepared_live_seed: Arc<yrs_engine::mutation::MutationLookupSeed>,
    durable_client_ids: HashSet<u64>,
    next_revision: u64,
    next_state_revision: u64,
    next_epoch: u64,
}

impl YrsDocumentEngine {
    /// Merge one standard Yjs/Yrs Update-v1 into the authoritative document.
    ///
    /// The update is fully decoded, applied, derived, validated, and admitted
    /// on an isolated candidate before the live CRDT is opened for mutation.
    /// Remote-origin structs remain outside the local undo scope.
    ///
    /// This is exactly [`Self::prepare_remote_update_internal`] followed by
    /// [`Self::commit_prepared_remote_update_internal`], so the one-shot and
    /// prepare/commit paths cannot drift.
    #[allow(dead_code)]
    pub fn apply_remote_update_v1(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> yrs_engine::OperationResult<EngineCommit> {
        let prepared = self.prepare_remote_update_internal(request_id, update)?;
        self.commit_prepared_remote_update_internal(prepared)
    }

    /// Everything fallible in the remote-update pipeline, up to (and
    /// excluding) the live installation. Preparation is observationally pure:
    /// dependency classification produces an owned candidate while the live
    /// quarantine remains untouched until commit.
    fn prepare_remote_update_internal(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> yrs_engine::OperationResult<PreparedRemoteUpdate> {
        let admitted_revision = self.revision;
        let admitted_state_revision = self.state_revision;
        let admitted_epoch = self.yrs_state_epoch;
        admit_max_encoded_state_len(
            request_id,
            update.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        preflight_update_v1(update, &self.resource_limits)
            .map_err(|error| remote_ingress_error(request_id, error))?;
        let incoming_update = Update::decode_v1(update).map_err(|error| {
            yrs_engine::OperationError::document_invalid(
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
                yrs_engine::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxEncodedStateBytes",
                    self.resource_limits.max_encoded_state_bytes as u64,
                    u64::MAX,
                )
            })?;
            admit_max_encoded_state_len(
                request_id,
                combined_len,
                self.resource_limits.max_encoded_state_bytes,
            )?;
            let quarantined_update = Update::decode_v1(quarantined).map_err(|error| {
                yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    format!("quarantined Update-v1 cannot decode: {error}"),
                )
            })?;
            let merged = Update::merge_updates(vec![quarantined_update, incoming_update]);
            let merged_bytes = merged.encode_v1();
            admit_max_encoded_state_len(
                request_id,
                merged_bytes.len(),
                self.resource_limits.max_encoded_state_bytes,
            )?;
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
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("current encoded state cannot decode: {error}"),
                    )
                })?;
                txn.apply_update(current).map_err(|error| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("candidate cannot seed current state: {error}"),
                    )
                })?;
            }
            txn.apply_update(candidate_update).map_err(|error| {
                yrs_engine::OperationError::document_invalid(
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
                #[cfg(test)]
                if FAIL_QUARANTINED_UPDATE_RESERVATION.with(std::cell::Cell::get) {
                    return Err(yrs_engine::OperationError::operation_resource_exhausted(
                        request_id,
                        "remoteUpdate",
                        "injected quarantined remote update reservation failure",
                    ));
                }
                let mut admitted = Vec::new();
                admitted.try_reserve_exact(update.len()).map_err(|error| {
                    yrs_engine::OperationError::operation_resource_exhausted(
                        request_id,
                        "remoteUpdate",
                        format!("cannot reserve quarantined remote update: {error}"),
                    )
                })?;
                admitted.extend_from_slice(update);
                admitted
            };
            return Ok(self.seal_remote_outcome(
                request_id,
                PreparedDocumentOutcome::Unchanged,
                PreparedDependencyState::Retain(quarantined),
            ));
        }
        let candidate_encoded =
            encode_candidate_state_bounded(&candidate_doc, &self.resource_limits)
                .map_err(|error| history_operation_error(request_id, error))?;
        if candidate_encoded == current_encoded {
            return Ok(self.seal_remote_outcome(
                request_id,
                PreparedDocumentOutcome::Unchanged,
                PreparedDependencyState::Clear,
            ));
        }

        let codec = YrsDocumentCodec::new(&self.schema, &self.resource_limits);
        let candidate_json = {
            let txn = candidate_doc.transact();
            let fragment = txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    yrs_engine::OperationError::document_invalid(
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
                return Err(yrs_engine::OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual as u64,
                ));
            }
        }
        let canonical_artifact =
            self.canonical_schema
                .derive(&candidate_document)
                .map_err(|error| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        format!("remote document serialization failed: {error}"),
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
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "validated remote candidate lost its document fragment",
                    )
                })?;
            if let Some(current) = self.derived_state.as_ref() {
                let candidate_render_blocks = Arc::new(
                    crate::render::incremental::CachedRenderBlocks::build(
                        &candidate_document,
                        &self.schema,
                        &self.resource_limits,
                    )
                    .map_err(|error| {
                        cached_render_operation_error(request_id, &self.resource_limits, error)
                    })?,
                );
                let fallback = affinity_aware_mapped_selection(
                    &current.legacy_selection(),
                    &current.relative_selection,
                    &StepMap::empty(),
                    &candidate_document,
                    &self.schema,
                    None,
                );
                let mut next = current
                    .after_document_change(
                        candidate_document.clone(),
                        canonical_artifact.clone(),
                        &txn,
                        &fragment,
                        &self.schema,
                        &self.schema_fingerprint,
                        &self.resource_limits,
                        &self.editing_limits,
                        self.max_length,
                        Arc::clone(&candidate_render_blocks),
                        None,
                        &StepMap::empty(),
                        UpdateMode::Rebuild,
                        &[],
                        None,
                        Some(&fallback),
                        false,
                        None,
                        None,
                        None,
                        next_revision,
                        next_state_revision,
                        next_epoch,
                    )
                    .ok_or_else(|| {
                        yrs_engine::OperationError::selection_position_invalid(
                            request_id,
                            "selection",
                            "local relative selection cannot resolve after remote update",
                        )
                    })?;
                next.stored_marks = match (&current.resolved_selection, &next.resolved_selection) {
                    (
                        yrs_engine::ResolvedSelection::Text {
                            anchor: current_anchor,
                            head: current_head,
                        },
                        yrs_engine::ResolvedSelection::Text {
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
                    canonical_artifact.clone(),
                    &txn,
                    &fragment,
                    &self.schema,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_revision,
                    next_state_revision,
                    next_epoch,
                )
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
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
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("candidate-produced incremental update cannot decode: {error}"),
            )
        })?;
        #[cfg(test)]
        yrs_engine::compiler::check_atomic_failpoint(
            request_id,
            yrs_engine::compiler::AtomicFailpoint::RemoteHistoryAdmission,
        )?;
        // Replay retention charges remote work in encoded-byte units: this is
        // the exact admitted incremental payload, not redundant caller input.
        let replay_byte_units = u64::try_from(accepted_update.len())
            .unwrap_or(u64::MAX)
            .max(1);
        let history_admission = self.history.pre_admit_excluded(
            request_id,
            TransactionOrigin::RemoteSync,
            replay_byte_units,
            &current_encoded,
            accepted_update.len(),
        )?;
        if (self.revision, self.state_revision, self.yrs_state_epoch)
            != (admitted_revision, admitted_state_revision, admitted_epoch)
        {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "remote update engine state changed during candidate admission",
            ));
        }
        let prepared_live_seed = {
            let candidate_txn = candidate_doc.transact();
            let candidate_fragment = candidate_txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "accepted remote candidate lost its Yrs fragment before seed rebind",
                    )
                })?;
            let live_txn = self.doc.transact();
            let live_fragment = live_txn
                .get_xml_fragment(self.fragment_name.as_str())
                .ok_or_else(|| {
                    yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "accepted remote update lost its live Yrs fragment before seed rebind",
                    )
                })?;
            let prepared = next_state
                .mutation_lookup_seed
                .prepare_authoritative_store_rebind(
                    request_id,
                    &candidate_txn,
                    &candidate_fragment,
                    &next_state.document,
                    &next_state.canonical_artifact,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_epoch,
                    next_revision,
                    &live_txn,
                    &live_fragment,
                )?;
            if !prepared.matches_canonical_artifact(&next_state.canonical_artifact)
                || !prepared.matches(
                    &live_txn,
                    &live_fragment,
                    &next_state.document,
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    &self.schema_fingerprint,
                    next_epoch,
                    next_revision,
                )
            {
                return Err(yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared authoritative-store mutation lookup seed is stale",
                ));
            }
            prepared
        };
        #[cfg(test)]
        yrs_engine::observability::record_staged_seed_preparation();
        Ok(self.seal_remote_outcome(
            request_id,
            PreparedDocumentOutcome::Changed(Box::new(PreparedRemoteInstall {
                live_update,
                accepted_update,
                history_admission,
                next_state,
                prepared_live_seed,
                durable_client_ids,
                next_revision,
                next_state_revision,
                next_epoch,
            })),
            PreparedDependencyState::Clear,
        ))
    }

    /// Seals prepared document and dependency candidates to the engine state
    /// that admitted them without mutating that state.
    fn seal_remote_outcome(
        &self,
        request_id: u64,
        document: PreparedDocumentOutcome,
        dependencies: PreparedDependencyState,
    ) -> PreparedRemoteUpdate {
        PreparedRemoteUpdate {
            seal: PreparedRemoteSeal {
                request_id,
                sealed_doc: self.doc.clone(),
                admitted_revision: self.revision,
                admitted_state_revision: self.state_revision,
                admitted_epoch: self.yrs_state_epoch,
                sealed_generation: self.remote_seal_generation,
            },
            document,
            dependencies,
        }
    }

    /// The already-proven live installation of a prepared remote update.
    /// Rejects with `ENGINE_INVARIANT_FAILED` — leaving the engine untouched —
    /// if anything mutated the engine after preparation (local edit, another
    /// remote commit, snapshot restore, replacement, or a newly quarantined
    /// dependency payload).
    fn commit_prepared_remote_update_internal(
        &mut self,
        prepared: PreparedRemoteUpdate,
    ) -> yrs_engine::OperationResult<EngineCommit> {
        let PreparedRemoteUpdate {
            seal,
            document,
            dependencies,
        } = prepared;
        let PreparedRemoteSeal {
            request_id,
            sealed_doc,
            admitted_revision,
            admitted_state_revision,
            admitted_epoch,
            sealed_generation,
        } = seal;
        if !Doc::ptr_eq(&sealed_doc, &self.doc)
            || (self.revision, self.state_revision, self.yrs_state_epoch)
                != (admitted_revision, admitted_state_revision, admitted_epoch)
            || self.remote_seal_generation != sealed_generation
        {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "prepared remote update is sealed to a superseded engine state",
            ));
        }
        let dependency_candidate = match dependencies {
            PreparedDependencyState::Clear => None,
            PreparedDependencyState::Retain(bytes) => Some(bytes),
        };
        match document {
            PreparedDocumentOutcome::Unchanged => {
                if self.quarantined_remote_update != dependency_candidate {
                    self.quarantined_remote_update = dependency_candidate;
                    self.remote_seal_generation = self.remote_seal_generation.wrapping_add(1);
                }
                Ok(EngineCommit {
                    changed: false,
                    revision: self.revision,
                })
            }
            PreparedDocumentOutcome::Changed(install) => {
                let PreparedRemoteInstall {
                    live_update,
                    accepted_update,
                    history_admission,
                    mut next_state,
                    prepared_live_seed,
                    durable_client_ids,
                    next_revision,
                    next_state_revision,
                    next_epoch,
                } = *install;
                {
                    let mut txn = self.doc.transact_mut_with(history_admission.yrs_origin());
                    txn.apply_update(live_update).expect(
                        "candidate-proved remote update must apply to identical live state",
                    );
                }
                next_state.mutation_lookup_seed = prepared_live_seed;
                self.history
                    .finish_prepared_excluded(history_admission, accepted_update);
                self.quarantined_remote_update = dependency_candidate;
                self.derived_state = Some(next_state);
                self.durable_client_ids = durable_client_ids;
                self.revision = next_revision;
                self.state_revision = next_state_revision;
                self.yrs_state_epoch = next_epoch;
                self.last_committed_origin = Some(TransactionOrigin::RemoteSync);
                self.document_origin = yrs_engine::DocumentOrigin::RemoteCollaboration;
                self.prepared_candidate_cache = None;
                Ok(EngineCommit {
                    changed: true,
                    revision: self.revision,
                })
            }
        }
    }

    /// Production surface: admit a remote Update-v1 without installing it, so a
    /// coupled protocol reply can be reserved between preparation and
    /// installation. See [`Self::apply_remote_update_v1`], which is this
    /// method followed by [`Self::commit_prepared_remote_update`].
    pub fn prepare_remote_update_v1(
        &mut self,
        request_id: u64,
        update: &[u8],
    ) -> yrs_engine::OperationResult<PreparedRemoteUpdate> {
        self.prepare_remote_update_internal(request_id, update)
    }

    /// Production surface: install a prepared remote update. One-shot; the
    /// prepared value is consumed whether or not installation is admitted.
    pub fn commit_prepared_remote_update(
        &mut self,
        prepared: PreparedRemoteUpdate,
    ) -> yrs_engine::OperationResult<EngineCommit> {
        self.commit_prepared_remote_update_internal(prepared)
    }

    /// Production surface: the authoritative store's state vector, encoded v1.
    /// Read-only: no revision, epoch, state, or history effect.
    pub fn encode_state_vector_v1(&self, request_id: u64) -> yrs_engine::OperationResult<Vec<u8>> {
        let encoded = self.doc.transact().state_vector().encode_v1();
        // Defensive symmetry with every other encoded artifact: a consistent
        // engine cannot produce a state vector above this ceiling (its full
        // encoded state is strictly larger and is bounded by the same limit
        // on every admission path), so the gate itself is unit-tested at the
        // exact/one-over boundary instead.
        admit_max_encoded_state_len(
            request_id,
            encoded.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        Ok(encoded)
    }

    /// Production surface: the incremental Update-v1 that brings a peer at
    /// `remote_state_vector_v1` up to the authoritative store. Read-only.
    /// Malformed input rejects with a structured error, bounded before any
    /// decode work by the encoded-state byte ceiling.
    pub fn encode_diff_v1(
        &self,
        request_id: u64,
        remote_state_vector_v1: &[u8],
    ) -> yrs_engine::OperationResult<Vec<u8>> {
        admit_max_encoded_state_len(
            request_id,
            remote_state_vector_v1.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        let remote_state_vector =
            StateVector::decode_v1(remote_state_vector_v1).map_err(|error| {
                yrs_engine::OperationError::document_invalid(
                    request_id,
                    None,
                    "stateVector",
                    format!("invalid state vector: {error}"),
                )
            })?;
        let diff = self
            .doc
            .transact()
            .encode_state_as_update_v1(&remote_state_vector);
        admit_max_encoded_state_len(
            request_id,
            diff.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        Ok(diff)
    }

    /// Production surface: the bounded structural classification half of the
    /// remote ingress pipeline, exactly the byte-ceiling admission plus
    /// Update-v1 preflight that [`Self::prepare_remote_update_v1`] runs
    /// first. Read-only; the protocol layer uses it to classify a rejected
    /// payload as malformed encoding (preflight also fails) versus
    /// admitted-but-inadmissible content (preflight passes).
    pub fn preflight_remote_update_v1(
        &self,
        request_id: u64,
        update: &[u8],
    ) -> yrs_engine::OperationResult<()> {
        admit_max_encoded_state_len(
            request_id,
            update.len(),
            self.resource_limits.max_encoded_state_bytes,
        )?;
        preflight_update_v1(update, &self.resource_limits)
            .map_err(|error| remote_ingress_error(request_id, error))?;
        Ok(())
    }

    /// Production surface: byte accounting for the engine-owned dependency
    /// quarantine (`0` when no update is pending). The engine is the sole
    /// owner of the pending bytes themselves; the collaboration runtime
    /// charges this figure against its configured ceilings without ever
    /// holding a second payload copy.
    pub fn pending_remote_dependency_bytes(&self) -> usize {
        self.quarantined_remote_update
            .as_deref()
            .map_or(0, <[u8]>::len)
    }
}

/// The shared `maxEncodedStateBytes` admission gate used by the remote-update
/// pipeline and the sealed state-vector/diff encoders: exact length is
/// admitted, one over rejects with the structured limit error.
pub(super) fn admit_max_encoded_state_len(
    request_id: u64,
    actual_len: usize,
    max_encoded_state_bytes: usize,
) -> yrs_engine::OperationResult<()> {
    if actual_len > max_encoded_state_bytes {
        return Err(yrs_engine::OperationError::document_limit_exceeded(
            request_id,
            None,
            "maxEncodedStateBytes",
            max_encoded_state_bytes as u64,
            actual_len as u64,
        ));
    }
    Ok(())
}

fn remote_ingress_error(request_id: u64, error: YrsEngineError) -> yrs_engine::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "encodedState"
        };
        let mut mapped = yrs_engine::OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        yrs_engine::OperationError::document_invalid(request_id, None, "update", error.message)
    }
}

fn remote_engine_error(request_id: u64, error: YrsEngineError) -> yrs_engine::OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let mut mapped = yrs_engine::OperationError::document_limit_exceeded(
            request_id,
            None,
            "update",
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        yrs_engine::OperationError::document_invalid(
            request_id,
            None,
            "update",
            format!("remote document cannot be decoded: {}", error.message),
        )
    }
}

fn remote_json_error(request_id: u64, error: JsonParseError) -> yrs_engine::OperationError {
    match error {
        JsonParseError::ResourceLimit { limit, actual } => {
            yrs_engine::OperationError::document_limit_exceeded(
                request_id,
                None,
                "update",
                u64::try_from(limit).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            )
        }
        error => yrs_engine::OperationError::document_invalid(
            request_id,
            None,
            "update",
            error.to_string(),
        ),
    }
}

fn remote_validation_error(
    request_id: u64,
    error: crate::boundary::BoundaryError,
) -> yrs_engine::OperationError {
    if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
        let mut mapped = yrs_engine::OperationError::document_limit_exceeded(
            request_id,
            None,
            "update",
            u64::try_from(error.limit.unwrap_or(0)).unwrap_or(u64::MAX),
            u64::try_from(error.actual.unwrap_or(0)).unwrap_or(u64::MAX),
        );
        merge_operation_details(&mut mapped, error.details);
        mapped
    } else {
        yrs_engine::OperationError::document_invalid(request_id, None, "update", error.to_string())
    }
}
