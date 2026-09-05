mod active_state;
mod candidate_evidence;
mod history_snapshot;
mod insert_admission;
mod localized_index;
mod localized_insert;
mod observability;
mod render_evidence;
mod selection;
mod validation;

use super::canonical::CanonicalArtifact;
use super::compiler::{CachedCompilationView, CompiledDocumentDerivations};
use super::prepared_admission::DerivedStateAuthority;
use super::{RelativeSelection, ResolvedSelection};
use crate::boundary::ResourceLimits;
#[cfg(test)]
use crate::editor_state::ActiveState;
use crate::model::{Document, Mark};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
#[cfg(test)]
use crate::transform::DocumentValidator;
use crate::transform::StepMap;
#[cfg(test)]
use active_state::active_state_retained_bytes;
#[allow(unused_imports)]
pub(crate) use active_state::{
    ActiveStateCertificate, ActiveStateStructuralSeal, CachedActiveState,
    PreparedActiveStateInstall, PreparedActiveStateTransition,
};
pub(crate) use candidate_evidence::{PreparedCandidateEvidence, PreparedCandidateValidation};
#[cfg(test)]
pub(crate) use history_snapshot::prepare_history_candidate_read_for_test;
#[allow(unused_imports)]
pub(crate) use history_snapshot::{
    history_document_snapshot_retained_bytes,
    history_document_snapshot_retained_bytes_with_canonical_charge,
    history_document_snapshot_retained_bytes_with_precomputed_document_charge,
    AdmittedHistoryCandidateRead, AdmittedHistoryMutationLookupProof, HistoryDocumentSnapshot,
    HistoryDocumentSnapshotRetainedBytes, HistoryDocumentSnapshotRetainedInput,
    HistoryMutationLookupCapability, PreparedHistoryCandidateRead, RestoredHistoryDocumentState,
};
pub(crate) use insert_admission::{LocalizedInsertAdmission, ValidatedLocalizedInsertAdmission};
#[cfg(test)]
use localized_index::canonical_marks_sha256;
#[allow(unused_imports)]
pub(crate) use localized_index::{LocalizedTextLeafCertificate, LocalizedTextLeafIndex};
#[cfg(test)]
use observability::FORCE_INITIALIZE_SCALAR_MISMATCH;
#[allow(unused_imports)]
pub(crate) use observability::{
    active_state_cache_hit_fallback_forced, record_active_state_cache_attempt,
    record_active_state_cache_drop, record_active_state_cache_fallback,
    record_active_state_cache_hit, record_active_state_cache_install,
    record_active_state_candidate_build, record_active_state_full_assembly,
    record_active_state_generic_build, record_active_state_public_result_clone,
    record_preview_position_map_derivation, record_preview_rendered_text_derivation,
    record_prewrite_selection_proof_attempt, record_prewrite_selection_proof_fallback,
    record_prewrite_selection_proof_finalization, record_prewrite_selection_proof_install,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use observability::{
    force_active_state_cache_allocation_failure_for_test, force_active_state_cache_budget_for_test,
    force_active_state_cache_hit_fallback_for_test,
    force_active_state_public_materialization_failure_for_test,
    force_history_document_snapshot_fallback_for_test,
    force_history_snapshot_semantic_fallback_for_test,
    force_localized_index_allocation_failure_for_test,
    force_localized_index_allocation_stage_for_test, force_localized_index_budget_for_test,
    reset_active_state_cache_counts_for_test, reset_localized_index_lifecycle_counts_for_test,
    reset_localized_index_metrics_for_test, reset_localized_insert_admission_work_for_test,
    reset_preview_derivation_counts_for_test, reset_prewrite_selection_proof_counts_for_test,
    reset_relative_selection_traversal_counts_for_test, take_active_state_cache_counts_for_test,
    take_localized_index_lifecycle_counts_for_test, take_localized_index_metrics_for_test,
    take_localized_insert_admission_work_for_test, take_preview_derivation_counts_for_test,
    take_prewrite_selection_proof_counts_for_test,
    take_relative_selection_traversal_counts_for_test, ForcedHistoryDocumentSnapshotFallback,
    ForcedHistorySnapshotSemanticFallback, HistorySnapshotSemanticFallbackForTest,
    LocalizedIndexAllocationStage,
};
pub(crate) use render_evidence::{FinalizedDerivedEvidence, PreparedDerivedEvidence};
use selection::preserve_with_mapped_fallback;
pub(crate) use selection::{
    apply_stored_mark_operation, canonical_marks, exact_point_is_representable,
    history_selection_to_relative, marks_at_position, operation_result_to_relative,
    resolve_selection, resolved_from_legacy, resolved_from_legacy_with_view, resolved_to_legacy,
    stored_marks_after_selection_change, FinalizedSelectionState,
};
use std::sync::Arc;
pub(crate) use validation::{
    DocumentValidationCertificate, ValidatedCandidateContext, ValidatedDocumentEvidence,
};
use yrs::types::xml::XmlFragmentRef;
use yrs::ReadTxn;

#[derive(Debug)]
pub(crate) struct DerivedStateCache {
    pub document: Document,
    pub canonical_artifact: CanonicalArtifact,
    pub position_map: PositionMap,
    pub rendered_text: String,
    pub rendered_scalars: u32,
    pub document_text_bytes: usize,
    pub document_node_count: usize,
    pub legacy_selection: Selection,
    pub relative_selection: RelativeSelection,
    pub resolved_selection: ResolvedSelection,
    pub stored_marks: Option<Vec<Mark>>,
    pub document_revision: u64,
    pub state_revision: u64,
    pub schema_fingerprint: String,
    pub render_blocks: Arc<crate::render::incremental::CachedRenderBlocks>,
    pub mutation_lookup_seed: Arc<super::mutation::MutationLookupSeed>,
    pub validation_certificate: DocumentValidationCertificate,
    #[cfg_attr(not(test), allow(dead_code))]
    pub localized_text_index: Option<LocalizedTextLeafIndex>,
    active_state_certificate: Option<Arc<ActiveStateCertificate>>,
}

impl DerivedStateCache {
    // Keep every mutation identity dimension explicit so callers cannot hide a mismatched seal.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn matches_materialized_mutation_identity(
        &self,
        canonical_artifact: &CanonicalArtifact,
        canonical_fingerprint: [u8; 32],
        canonical_serialized_len: usize,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> bool {
        self.document_revision == document_revision
            && self.state_revision == state_revision
            && self.schema_fingerprint == schema_fingerprint
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && self.validation_certificate.matches_materialized_identity(
                canonical_artifact,
                canonical_fingerprint,
                canonical_serialized_len,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
            && self.localized_text_index.as_ref().is_none_or(|index| {
                index.matches_materialized_identity(
                    &self.validation_certificate,
                    canonical_artifact,
                    canonical_fingerprint,
                    canonical_serialized_len,
                    resource_limits,
                    schema_fingerprint,
                    document_revision,
                    state_revision,
                    yrs_state_epoch,
                )
            })
    }

    #[cfg(test)]
    pub(crate) fn materialize_mutation_identity(&mut self) {
        self.validation_certificate.materialize_canonical_artifact();
        if let Some(index) = self.localized_text_index.as_mut() {
            index.materialize_canonical_fingerprint(&self.validation_certificate);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_generic_derived_evidence(
        &self,
        request_id: u64,
        authority: &dyn DerivedStateAuthority,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<FinalizedDerivedEvidence> {
        let installed = authority.installed();
        authority.lookup_seed(request_id).ok()?;
        if !self.document.shares_root_storage_with(&installed.document)
            || !self
                .canonical_artifact
                .ptr_eq(&installed.canonical_artifact)
        {
            return None;
        }
        let validation_certificate = DocumentValidationCertificate::mint(
            document,
            canonical_artifact,
            schema,
            resource_limits,
            schema_fingerprint,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )?;
        let localized_text_index = LocalizedTextLeafIndex::build(
            document,
            &derivations.position_map,
            &derivations.rendered_text,
            &validation_certificate,
            resource_limits,
            schema,
        );
        Some(FinalizedDerivedEvidence {
            validation_certificate,
            localized_text_index,
        })
    }

    pub(crate) fn clone_with_fallible_localized_index(&self) -> Self {
        let localized_text_index = self.localized_text_index.as_ref().and_then(|index| {
            index.try_clone(self.validation_certificate.resource_limits.max_input_bytes)
        });
        Self {
            document: self.document.clone(),
            canonical_artifact: self.canonical_artifact.clone(),
            position_map: self.position_map.clone(),
            rendered_text: self.rendered_text.clone(),
            rendered_scalars: self.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
            legacy_selection: self.legacy_selection.clone(),
            relative_selection: self.relative_selection.clone(),
            resolved_selection: self.resolved_selection.clone(),
            stored_marks: self.stored_marks.clone(),
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            schema_fingerprint: self.schema_fingerprint.clone(),
            render_blocks: Arc::clone(&self.render_blocks),
            mutation_lookup_seed: Arc::clone(&self.mutation_lookup_seed),
            validation_certificate: self.validation_certificate.clone(),
            localized_text_index,
            active_state_certificate: None,
        }
    }

    pub(crate) fn reseal_state_revision(&mut self, state_revision: u64) {
        self.state_revision = state_revision;
        self.validation_certificate
            .reseal_state_revision(state_revision);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialize<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        Self::initialize_with_local_state(
            document,
            canonical_artifact,
            txn,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            None,
            None,
            None,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    }

    /// Initializes a history candidate from the exact local state attached to
    /// the StackItem Yrs popped. Unlike ordinary document initialization, this
    /// does not first manufacture and resolve a default selection only to
    /// clone the complete cache and replace that selection immediately.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_history<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        relative_selection: RelativeSelection,
        stored_marks: Option<Vec<Mark>>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        Self::initialize_with_local_state(
            document,
            canonical_artifact,
            txn,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            Some(relative_selection),
            stored_marks,
            None,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_validated_candidate<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        validated_candidate: ValidatedCandidateContext<'_>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        Self::initialize_with_local_state(
            document,
            canonical_artifact,
            txn,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            None,
            None,
            Some(validated_candidate),
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn initialize_with_local_state<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        initial_relative_selection: Option<RelativeSelection>,
        stored_marks: Option<Vec<Mark>>,
        validated_candidate: Option<ValidatedCandidateContext<'_>>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        if canonical_artifact.schema_fingerprint() != schema_fingerprint
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
        {
            return None;
        }
        let admitted_validation = validated_candidate.as_ref().and_then(|validated| {
            validated.evidence.admitted_validation_report(
                &document,
                &canonical_artifact,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                validated.canonical_schema,
                validated.fragment_name,
                txn,
                fragment,
                validated.engine_epoch,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
        });
        // Render-block construction admits every renderer arithmetic domain,
        // including ordered-list indices. It must happen before PositionMap,
        // whose marker-length walk assumes that admission has succeeded.
        let render_blocks = Arc::new(if let Some(validation) = admitted_validation {
            crate::render::incremental::CachedRenderBlocks::build_validated(
                &document,
                schema,
                resource_limits,
                schema_fingerprint,
                validation.stats.node_count,
                validation.stats.max_depth,
            )
            .ok()?
        } else {
            crate::render::incremental::CachedRenderBlocks::build(
                &document,
                schema,
                resource_limits,
            )
            .ok()?
        });
        if !render_blocks.matches_identity(&document, schema_fingerprint) {
            return None;
        }
        let position_map = PositionMap::build(&document, schema);
        let rendered_text = if admitted_validation.is_some() {
            render_blocks.rendered_text(schema)
        } else {
            crate::render::rendered_text(&document, schema)
        };
        let rendered_scalars = u32::try_from(rendered_text.chars().count()).ok()?;
        #[cfg(test)]
        let rendered_scalars = FORCE_INITIALIZE_SCALAR_MISMATCH.with(|force| {
            if force.get() {
                rendered_scalars.saturating_sub(1)
            } else {
                rendered_scalars
            }
        });
        // Generic non-text-block containers may render direct inline content
        // that is intentionally outside the addressable PositionMap domain.
        // Cache construction only requires that every mapped scalar fits in
        // the rendered buffer; position-dependent compilation separately
        // requires exact domain parity before using the two views together.
        if rendered_scalars < position_map.total_scalars() {
            return None;
        }
        let document_text_bytes = canonical_artifact.text_utf8_bytes();
        let document_node_count = admitted_validation.map_or_else(
            || crate::editor_state::document_node_count(document.root()),
            |validation| validation.stats.node_count,
        );
        let relative_selection = initial_relative_selection.unwrap_or_else(|| {
            let selection = (0..position_map.block_count())
                .filter_map(|index| position_map.block(index))
                .find(|block| !block.is_void_block)
                .map(|block| Selection::cursor(block.doc_start))
                .or_else(|| {
                    position_map
                        .block(0)
                        .map(|block| Selection::node(block.doc_start))
                })
                .unwrap_or_else(Selection::all);
            operation_result_to_relative(txn, fragment, &selection, schema)
        });
        let resolved_selection = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &document,
            &position_map,
            &rendered_text,
        )?;
        let legacy_selection = resolved_to_legacy(&resolved_selection);
        let mutation_lookup_seed = if admitted_validation.is_some() {
            super::mutation::MutationLookupSeed::unavailable_for_validated_import(
                txn,
                fragment,
                &document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
        } else {
            super::mutation::MutationLookupSeed::build(
                0,
                txn,
                fragment,
                schema,
                &document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
            .ok()?
        };
        let mutation_lookup_seed =
            Arc::new(mutation_lookup_seed.with_canonical_artifact(&canonical_artifact));
        let validation_certificate = if let Some(validation) = admitted_validation {
            DocumentValidationCertificate::from_report(
                validation,
                &canonical_artifact,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
        } else {
            DocumentValidationCertificate::mint(
                &document,
                &canonical_artifact,
                schema,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )?
        };
        let localized_text_index = LocalizedTextLeafIndex::build(
            &document,
            &position_map,
            &rendered_text,
            &validation_certificate,
            resource_limits,
            schema,
        );
        Some(Self {
            document,
            canonical_artifact,
            position_map,
            rendered_text,
            rendered_scalars,
            document_text_bytes,
            document_node_count,
            legacy_selection,
            relative_selection,
            resolved_selection,
            stored_marks,
            document_revision,
            state_revision,
            schema_fingerprint: schema_fingerprint.into(),
            render_blocks,
            mutation_lookup_seed,
            validation_certificate,
            localized_text_index,
            active_state_certificate: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn after_document_change<T: ReadTxn>(
        &self,
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        render_blocks: Arc<crate::render::incremental::CachedRenderBlocks>,
        prepared_derivations: Option<CompiledDocumentDerivations>,
        step_map: &StepMap,
        update_mode: UpdateMode,
        affected_top_level_blocks: &[usize],
        explicit_selection: Option<&RelativeSelection>,
        preserved_fallback: Option<&Selection>,
        strict_fallback_affinity: bool,
        promoted_mutation_lookup_seed: Option<Arc<super::mutation::MutationLookupSeed>>,
        finalized_selection: Option<FinalizedSelectionState>,
        finalized_derived_evidence: Option<FinalizedDerivedEvidence>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        if canonical_artifact.schema_fingerprint() != schema_fingerprint
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !render_blocks.matches_identity(&document, schema_fingerprint)
        {
            return None;
        }
        let canonical_document_text_bytes = canonical_artifact.text_utf8_bytes();
        let (
            position_map,
            rendered_text,
            rendered_scalars,
            prepared_document_text_bytes,
            document_node_count,
        ) = if let Some(prepared) = prepared_derivations {
            (
                prepared.position_map,
                prepared.rendered_text,
                prepared.rendered_scalars,
                prepared.document_text_bytes,
                prepared.document_node_count,
            )
        } else {
            let mut position_map = self.position_map.clone();
            record_preview_position_map_derivation();
            let update_mode = if affected_top_level_blocks.is_empty() && self.document != document {
                UpdateMode::Rebuild
            } else {
                update_mode
            };
            position_map.update(step_map, &self.document, &document, update_mode, schema);
            position_map.compact();
            record_preview_rendered_text_derivation();
            let rendered_text = crate::render::rendered_text(&document, schema);
            let rendered_scalars = u32::try_from(rendered_text.chars().count()).ok()?;
            let document_node_count = crate::editor_state::document_node_count(document.root());
            (
                position_map,
                rendered_text,
                rendered_scalars,
                canonical_document_text_bytes,
                document_node_count,
            )
        };
        debug_assert_eq!(prepared_document_text_bytes, canonical_document_text_bytes);
        let document_text_bytes = canonical_document_text_bytes;
        if rendered_scalars < position_map.total_scalars() {
            return None;
        }

        let (relative_selection, resolved_selection, legacy_selection) =
            if let Some(finalized) = finalized_selection {
                record_prewrite_selection_proof_install();
                finalized.into_parts()
            } else {
                let mut relative_selection = explicit_selection
                    .cloned()
                    .unwrap_or_else(|| self.relative_selection.clone());
                let mut resolved_selection = resolve_selection(
                    txn,
                    fragment,
                    &relative_selection,
                    schema,
                    &document,
                    &position_map,
                    &rendered_text,
                );
                if resolved_selection.is_none() {
                    let fallback = preserved_fallback?;
                    relative_selection = preserve_with_mapped_fallback(
                        txn,
                        fragment,
                        &relative_selection,
                        fallback,
                        schema,
                        strict_fallback_affinity,
                    );
                    resolved_selection = resolve_selection(
                        txn,
                        fragment,
                        &relative_selection,
                        schema,
                        &document,
                        &position_map,
                        &rendered_text,
                    );
                }
                let resolved_selection = resolved_selection?;
                let legacy_selection = resolved_to_legacy(&resolved_selection);
                (relative_selection, resolved_selection, legacy_selection)
            };
        let mutation_lookup_seed = if let Some(promoted) = promoted_mutation_lookup_seed {
            promoted
        } else {
            Arc::new(
                super::mutation::MutationLookupSeed::build(
                    0,
                    txn,
                    fragment,
                    schema,
                    &document,
                    resource_limits,
                    editing_limits,
                    max_length,
                    schema_fingerprint,
                    yrs_state_epoch,
                    document_revision,
                )
                .ok()?
                .with_canonical_artifact(&canonical_artifact),
            )
        };
        let mutation_lookup_seed =
            if mutation_lookup_seed.matches_canonical_artifact(&canonical_artifact) {
                mutation_lookup_seed
            } else {
                Arc::new(
                    mutation_lookup_seed
                        .as_ref()
                        .clone()
                        .with_canonical_artifact(&canonical_artifact),
                )
            };
        let (validation_certificate, localized_text_index) =
            if let Some(finalized) = finalized_derived_evidence {
                (
                    finalized.validation_certificate,
                    finalized.localized_text_index,
                )
            } else {
                let validation_certificate = DocumentValidationCertificate::mint(
                    &document,
                    &canonical_artifact,
                    schema,
                    resource_limits,
                    schema_fingerprint,
                    document_revision,
                    state_revision,
                    yrs_state_epoch,
                )?;
                let localized_text_index = LocalizedTextLeafIndex::build(
                    &document,
                    &position_map,
                    &rendered_text,
                    &validation_certificate,
                    resource_limits,
                    schema,
                );
                (validation_certificate, localized_text_index)
            };

        Some(Self {
            document,
            canonical_artifact,
            position_map,
            rendered_text,
            rendered_scalars,
            document_text_bytes,
            document_node_count,
            legacy_selection,
            relative_selection,
            resolved_selection,
            stored_marks: self.stored_marks.clone(),
            document_revision,
            state_revision,
            schema_fingerprint: schema_fingerprint.into(),
            render_blocks,
            mutation_lookup_seed,
            validation_certificate,
            localized_text_index,
            active_state_certificate: None,
        })
    }

    pub fn compilation_view(&self) -> CachedCompilationView<'_> {
        CachedCompilationView {
            document: &self.document,
            position_map: &self.position_map,
            rendered_text: &self.rendered_text,
            rendered_scalars: self.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
            selection: &self.legacy_selection,
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            schema_fingerprint: &self.schema_fingerprint,
            canonical_artifact: &self.canonical_artifact,
        }
    }
}

#[cfg(test)]
#[path = "derived_state_tests.rs"]
mod tests;
