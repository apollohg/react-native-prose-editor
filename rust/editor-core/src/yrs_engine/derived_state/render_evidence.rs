use super::localized_index::LocalizedTextLeafIndex;
use super::validation::DocumentValidationCertificate;
use super::DerivedStateCache;
use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::schema::Schema;
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalArtifact;
use crate::yrs_engine::compiler::CompiledDocumentDerivations;
use crate::yrs_engine::prepared_admission::DerivedStateAuthority;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct LocalizedRenderTransitionProof {
    pub(super) base_document_root: crate::model::Node,
    pub(super) preview_root: crate::model::Node,
    pub(super) base_render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    pub(super) resource_limits: ResourceLimits,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) max_operations_per_transaction: usize,
    pub(super) max_undo_groups: usize,
    pub(super) max_derived_output_bytes: usize,
    pub(super) max_undo_retained_units: u64,
    pub(super) max_length: Option<u32>,
    pub(super) derivation_identity_seal: Arc<()>,
    pub(super) target_top_level_index: usize,
    pub(super) inserted_scalar_delta: u32,
    pub(super) top_level_cardinality: usize,
    pub(super) operation_kind: LocalizedRenderOperationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalizedRenderOperationKind {
    ExistingTextInsert,
    #[cfg(test)]
    Unsupported,
}

#[derive(Debug)]
pub(crate) struct PreparedDerivedEvidence {
    pub(super) request_id: u64,
    pub(super) base_document_root: crate::model::Node,
    pub(super) preview_root: crate::model::Node,
    pub(super) base_validation: DocumentValidationCertificate,
    pub(super) base_render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    pub(super) base_lookup_seal: Arc<yrs_engine::mutation::MutationLookupSeed>,
    pub(super) max_operations_per_transaction: usize,
    pub(super) max_undo_groups: usize,
    pub(super) max_derived_output_bytes: usize,
    pub(super) max_undo_retained_units: u64,
    pub(super) max_length: Option<u32>,
    pub(super) derivation_identity_seal: Arc<()>,
    pub(super) preview_rendered_scalars: u32,
    pub(super) preview_document_text_bytes: usize,
    pub(super) preview_document_node_count: usize,
    pub(super) preview_position_total_scalars: u32,
    pub(super) preview_position_block_count: usize,
    pub(super) canonical_fingerprint: [u8; 32],
    pub(super) canonical_serialized_len: usize,
    pub(super) validation_certificate: DocumentValidationCertificate,
    pub(super) localized_text_index: Option<LocalizedTextLeafIndex>,
    pub(super) localized_render_transition_proof: Option<LocalizedRenderTransitionProof>,
}

#[derive(Debug)]
pub(crate) struct FinalizedDerivedEvidence {
    pub(super) validation_certificate: DocumentValidationCertificate,
    pub(super) localized_text_index: Option<LocalizedTextLeafIndex>,
}

impl PreparedDerivedEvidence {
    #[cfg(test)]
    pub(crate) fn localized_render_tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "missing",
            "baseDocument",
            "previewDocument",
            "renderSeal",
            "resourceLimits",
            "schemaFingerprint",
            "maxOperations",
            "maxUndoGroups",
            "maxDerivedOutput",
            "maxUndo",
            "maxLength",
            "derivationIdentity",
            "targetIndex",
            "scalarDelta",
            "cardinality",
            "operationKind",
        ]
    }

    #[cfg(test)]
    pub(crate) fn tamper_localized_render_for_test(&mut self, case: &str) {
        if case == "missing" {
            self.localized_render_transition_proof = None;
            return;
        }
        let proof = self
            .localized_render_transition_proof
            .as_mut()
            .expect("localized render tamper fixture retains its proof");
        match case {
            "baseDocument" => proof.base_document_root = proof.preview_root.clone(),
            "previewDocument" => proof.preview_root = proof.base_document_root.clone(),
            "renderSeal" => proof.base_render_seal = Arc::new((*proof.base_render_seal).clone()),
            "resourceLimits" => {
                proof.resource_limits.max_input_bytes =
                    proof.resource_limits.max_input_bytes.saturating_add(1)
            }
            "schemaFingerprint" => proof.schema_fingerprint = Arc::<str>::from("tampered"),
            "maxOperations" => {
                proof.max_operations_per_transaction =
                    proof.max_operations_per_transaction.saturating_add(1)
            }
            "maxUndoGroups" => proof.max_undo_groups = proof.max_undo_groups.saturating_add(1),
            "maxDerivedOutput" => {
                proof.max_derived_output_bytes = proof.max_derived_output_bytes.saturating_add(1)
            }
            "maxUndo" => {
                proof.max_undo_retained_units = proof.max_undo_retained_units.saturating_add(1)
            }
            "maxLength" => proof.max_length = Some(proof.max_length.unwrap_or(1).saturating_add(1)),
            "derivationIdentity" => proof.derivation_identity_seal = Arc::new(()),
            "targetIndex" => {
                proof.target_top_level_index = proof.target_top_level_index.saturating_add(1)
            }
            "scalarDelta" => {
                proof.inserted_scalar_delta = proof.inserted_scalar_delta.saturating_add(1)
            }
            "cardinality" => {
                proof.top_level_cardinality = proof.top_level_cardinality.saturating_add(1)
            }
            "operationKind" => proof.operation_kind = LocalizedRenderOperationKind::Unsupported,
            _ => panic!("unknown localized render proof tamper case {case}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_localized_render_transition(
        &self,
        state: &DerivedStateCache,
        preview: &Document,
        derivations: &CompiledDocumentDerivations,
        affected_top_level_blocks: &[usize],
        schema: &Schema,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
    ) -> Option<
        Result<
            crate::render::incremental::CachedRenderTransition,
            crate::render::incremental::CachedRenderError,
        >,
    > {
        let proof = self.localized_render_transition_proof.as_ref()?;
        let base_raw_scalars = state.validation_certificate.raw_text_scalars;
        let expected_raw_scalars =
            base_raw_scalars.checked_add(u64::from(proof.inserted_scalar_delta))?;
        let expected_rendered_scalars = state
            .rendered_scalars
            .checked_add(proof.inserted_scalar_delta)?;
        let expected_affected_start = proof.target_top_level_index.saturating_sub(1);
        let expected_affected_len = proof
            .top_level_cardinality
            .checked_sub(expected_affected_start)?;
        let validation_stats = self.validation_certificate.stats;
        let affected_range_matches = affected_top_level_blocks.len() == expected_affected_len
            && affected_top_level_blocks
                .iter()
                .copied()
                .eq(expected_affected_start..proof.top_level_cardinality);
        if !state
            .document
            .root()
            .shares_storage_with(&proof.base_document_root)
            || !preview.root().shares_storage_with(&proof.preview_root)
            || !Arc::ptr_eq(&state.render_blocks, &proof.base_render_seal)
            || self.base_validation != state.validation_certificate
            || proof.resource_limits != *resource_limits
            || self.validation_certificate.resource_limits != proof.resource_limits
            || self.validation_certificate.schema_fingerprint.as_ref()
                != proof.schema_fingerprint.as_ref()
            || proof.schema_fingerprint.as_ref() != schema_fingerprint
            || proof.max_operations_per_transaction != editing_limits.max_operations_per_transaction
            || proof.max_undo_groups != editing_limits.max_undo_groups
            || proof.max_derived_output_bytes != editing_limits.max_derived_output_bytes
            || proof.max_undo_retained_units != editing_limits.max_undo_retained_units
            || proof.max_length != max_length
            || !Arc::ptr_eq(&proof.derivation_identity_seal, &derivations.identity_seal)
            || proof.operation_kind != LocalizedRenderOperationKind::ExistingTextInsert
            || proof.inserted_scalar_delta == 0
            || state.document.root().child_count() != proof.top_level_cardinality
            || preview.root().child_count() != proof.top_level_cardinality
            || !affected_range_matches
            || proof.target_top_level_index >= proof.top_level_cardinality
            || derivations.rendered_scalars != expected_rendered_scalars
            || derivations.document_node_count != validation_stats.node_count
            || validation_stats.node_count > proof.resource_limits.max_document_nodes
            || validation_stats.max_depth > proof.resource_limits.max_document_depth
            || proof.top_level_cardinality > validation_stats.node_count
            || self.validation_certificate.raw_text_scalars != expected_raw_scalars
            || proof
                .max_length
                .is_some_and(|limit| expected_raw_scalars > u64::from(limit))
            || !state
                .render_blocks
                .matches_identity(&state.document, schema_fingerprint)
        {
            return None;
        }
        Some(state.render_blocks.transition_localized_insert(
            &state.document,
            preview,
            schema,
            proof.target_top_level_index,
            proof.inserted_scalar_delta,
            resource_limits,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finalize(
        mut self,
        authority: &dyn DerivedStateAuthority,
        preview: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        next_render_blocks: &crate::render::incremental::CachedRenderBlocks,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        next_document_revision: u64,
        next_state_revision: u64,
        next_yrs_state_epoch: u64,
    ) -> Option<FinalizedDerivedEvidence> {
        let state = authority.installed();
        let authority_lookup_seed = authority.lookup_seed(self.request_id).ok()?;
        if self
            .localized_text_index
            .as_ref()
            .is_some_and(|index| !index.matches(&self.validation_certificate))
            || !state
                .document
                .root()
                .shares_storage_with(&self.base_document_root)
            || !preview.root().shares_storage_with(&self.preview_root)
            || state.validation_certificate != self.base_validation
            || !Arc::ptr_eq(&state.render_blocks, &self.base_render_seal)
            || !Arc::ptr_eq(authority_lookup_seed, &self.base_lookup_seal)
            || state.validation_certificate.resource_limits != *resource_limits
            || state.schema_fingerprint != schema_fingerprint
            || editing_limits.max_operations_per_transaction != self.max_operations_per_transaction
            || editing_limits.max_undo_groups != self.max_undo_groups
            || editing_limits.max_derived_output_bytes != self.max_derived_output_bytes
            || editing_limits.max_undo_retained_units != self.max_undo_retained_units
            || max_length != self.max_length
            || !Arc::ptr_eq(&derivations.identity_seal, &self.derivation_identity_seal)
            || canonical_artifact.sha256() != self.canonical_fingerprint
            || canonical_artifact.serialized_len() != self.canonical_serialized_len
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !self
                .validation_certificate
                .canonical_artifact
                .ptr_eq(canonical_artifact)
            || !self
                .validation_certificate
                .canonical_fingerprint_materialized
            || self.validation_certificate.canonical_fingerprint != canonical_artifact.sha256()
            || self.validation_certificate.canonical_serialized_len
                != canonical_artifact.serialized_len()
            || self.validation_certificate.raw_text_scalars != canonical_artifact.text_scalar_len()
            || self.validation_certificate.raw_text_utf8_bytes
                != canonical_artifact.text_utf8_bytes()
            || derivations.rendered_scalars != self.preview_rendered_scalars
            || derivations.document_text_bytes != self.preview_document_text_bytes
            || derivations.document_node_count != self.preview_document_node_count
            || derivations.position_map.total_scalars() != self.preview_position_total_scalars
            || derivations.position_map.block_count() != self.preview_position_block_count
            || !next_render_blocks.matches_identity(preview, schema_fingerprint)
            || next_document_revision != self.base_validation.document_revision.checked_add(1)?
            || next_state_revision != self.base_validation.state_revision.checked_add(1)?
            || next_yrs_state_epoch != self.base_validation.yrs_state_epoch.checked_add(1)?
        {
            return None;
        }
        self.validation_certificate.document_revision = next_document_revision;
        self.validation_certificate.state_revision = next_state_revision;
        self.validation_certificate.yrs_state_epoch = next_yrs_state_epoch;
        if let Some(index) = self.localized_text_index.as_mut() {
            index.document_revision = next_document_revision;
            index.canonical_fingerprint = canonical_artifact.sha256();
            index.schema_fingerprint = Arc::clone(&self.validation_certificate.schema_fingerprint);
        }
        Some(FinalizedDerivedEvidence {
            validation_certificate: self.validation_certificate,
            localized_text_index: self.localized_text_index,
        })
    }

    #[cfg(test)]
    pub(crate) fn tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "baseDocument",
            "previewDocument",
            "baseValidation",
            "renderSeal",
            "lookupSeal",
            "maxOperations",
            "maxUndoGroups",
            "maxDerivedOutput",
            "maxUndo",
            "maxLength",
            "derivationIdentity",
            "renderedScalars",
            "documentBytes",
            "documentNodes",
            "positionScalars",
            "positionBlocks",
            "canonicalFingerprint",
            "canonicalLength",
            "promotedValidation",
            "promotedIndex",
        ]
    }

    #[cfg(test)]
    pub(crate) fn tamper_for_test(&mut self, case: &str) {
        match case {
            "baseDocument" => self.base_document_root = self.preview_root.clone(),
            "previewDocument" => self.preview_root = self.base_document_root.clone(),
            "baseValidation" => {
                self.base_validation.state_revision =
                    self.base_validation.state_revision.saturating_add(1)
            }
            "renderSeal" => self.base_render_seal = Arc::new((*self.base_render_seal).clone()),
            "lookupSeal" => self.base_lookup_seal = Arc::new((*self.base_lookup_seal).clone()),
            "maxOperations" => {
                self.max_operations_per_transaction =
                    self.max_operations_per_transaction.saturating_add(1)
            }
            "maxUndoGroups" => self.max_undo_groups = self.max_undo_groups.saturating_add(1),
            "maxDerivedOutput" => {
                self.max_derived_output_bytes = self.max_derived_output_bytes.saturating_add(1)
            }
            "maxUndo" => {
                self.max_undo_retained_units = self.max_undo_retained_units.saturating_add(1)
            }
            "maxLength" => self.max_length = Some(self.max_length.unwrap_or(1).saturating_add(1)),
            "derivationIdentity" => self.derivation_identity_seal = Arc::new(()),
            "renderedScalars" => {
                self.preview_rendered_scalars = self.preview_rendered_scalars.saturating_add(1)
            }
            "documentBytes" => {
                self.preview_document_text_bytes =
                    self.preview_document_text_bytes.saturating_add(1)
            }
            "documentNodes" => {
                self.preview_document_node_count =
                    self.preview_document_node_count.saturating_add(1)
            }
            "positionScalars" => {
                self.preview_position_total_scalars =
                    self.preview_position_total_scalars.saturating_add(1)
            }
            "positionBlocks" => {
                self.preview_position_block_count =
                    self.preview_position_block_count.saturating_add(1)
            }
            "canonicalFingerprint" => self.canonical_fingerprint[0] ^= 1,
            "canonicalLength" => {
                self.canonical_serialized_len = self.canonical_serialized_len.saturating_add(1)
            }
            "promotedValidation" => self.validation_certificate.canonical_fingerprint[0] ^= 1,
            "promotedIndex" => {
                if let Some(index) = self.localized_text_index.as_mut() {
                    index.canonical_fingerprint[0] ^= 1;
                }
            }
            _ => panic!("unknown prepared evidence tamper case {case}"),
        }
    }
}
