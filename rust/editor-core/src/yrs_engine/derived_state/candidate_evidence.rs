use super::localized_index::LocalizedTextLeafIndex;
use super::observability::{
    record_preview_position_map_derivation, record_preview_rendered_text_derivation,
};
use super::render_evidence::FinalizedDerivedEvidence;
use super::validation::DocumentValidationCertificate;
use super::DerivedStateCache;
use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::transform::{DocumentValidationReport, StepMap};
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalArtifact;
use crate::yrs_engine::compiler::CompiledDocumentDerivations;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(crate) struct PreparedCandidateValidation {
    pub(super) document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) validation: DocumentValidationReport,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: yrs_engine::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) derivations: CompiledDocumentDerivations,
}

#[derive(Debug)]
pub(crate) struct PreparedCandidateEvidence {
    pub(super) document: Document,
    pub(super) validation_seal: DocumentValidationReport,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: yrs_engine::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) canonical_schema: yrs_engine::canonical::CanonicalSchemaContext,
    pub(super) position: PreparedPositionEvidence,
    pub(super) position_identity_seal: Arc<()>,
    pub(super) render: PreparedRenderEvidence,
    pub(super) render_identity_seal: Arc<()>,
    pub(super) document_text_bytes: usize,
    pub(super) document_node_count: usize,
    pub(super) raw_text_scalars: u64,
    pub(super) raw_text_utf8_bytes: usize,
    pub(super) history_render: Option<PreparedHistoryRenderEvidence>,
}

#[derive(Debug)]
pub(super) struct PreparedPositionEvidence {
    pub(super) position_map: PositionMap,
    pub(super) total_scalars: u32,
    pub(super) block_count: usize,
    pub(super) identity: Arc<()>,
}

#[derive(Debug)]
pub(super) struct PreparedRenderEvidence {
    pub(super) rendered_text: String,
    pub(super) rendered_scalars: u32,
    pub(super) identity: Arc<()>,
}

#[derive(Debug)]
pub(super) struct PreparedHistoryRenderEvidence {
    pub(super) base_document_root: crate::model::Node,
    pub(super) target_top_level_index: usize,
    pub(super) inserted_scalar_delta: u32,
    pub(super) candidate_render_identity: Arc<()>,
}

impl PreparedCandidateEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn prepare_deferred(
        base_document: &Document,
        base_position_map: &PositionMap,
        base_rendered_text: &str,
        document: &Document,
        validation: DocumentValidationReport,
        schema: &Schema,
        canonical_schema: &yrs_engine::canonical::CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        raw_text_scalars: u64,
        raw_text_utf8_bytes: usize,
        inserted_document_position: u32,
        inserted_text: &str,
        target_top_level_index: usize,
        inserted_scalar_delta: u32,
    ) -> Option<Self> {
        let schema_fingerprint = crate::schema::schema_fingerprint(schema);
        if schema_fingerprint != canonical_schema.schema_fingerprint()
            || validation.stats.node_count == 0
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || max_length.is_some_and(|limit| raw_text_scalars > u64::from(limit))
            || inserted_scalar_delta == 0
            || base_document.root().child_count() != document.root().child_count()
            || target_top_level_index >= base_document.root().child_count()
        {
            return None;
        }
        let old_target = base_document.root().child(target_top_level_index)?;
        let new_target = document.root().child(target_top_level_index)?;
        if old_target.node_size().checked_add(inserted_scalar_delta)? != new_target.node_size() {
            return None;
        }
        crate::transform::validate_canonical_marks(document, schema).ok()?;
        record_preview_position_map_derivation();
        #[cfg(test)]
        yrs_engine::observability::record_position_map_clone();
        let mut position_map = base_position_map.clone();
        let step_map = StepMap::try_from_insert(inserted_document_position, inserted_scalar_delta)?;
        position_map.update(
            &step_map,
            base_document,
            document,
            UpdateMode::InlineTextOnly,
            schema,
        );
        #[cfg(test)]
        yrs_engine::observability::record_position_map_compaction();
        position_map.compact();

        let rendered_scalar =
            base_position_map.doc_to_scalar(inserted_document_position, base_document);
        if base_position_map.scalar_to_doc(rendered_scalar, base_document)
            != inserted_document_position
            || u32::try_from(inserted_text.chars().count()).ok()? != inserted_scalar_delta
        {
            return None;
        }
        let rendered_byte = if rendered_scalar == base_position_map.total_scalars() {
            base_rendered_text.len()
        } else {
            base_rendered_text
                .char_indices()
                .nth(usize::try_from(rendered_scalar).ok()?)?
                .0
        };
        let rendered_capacity = base_rendered_text.len().checked_add(inserted_text.len())?;
        let mut rendered_text = String::new();
        rendered_text.try_reserve_exact(rendered_capacity).ok()?;
        rendered_text.push_str(base_rendered_text.get(..rendered_byte)?);
        rendered_text.push_str(inserted_text);
        rendered_text.push_str(base_rendered_text.get(rendered_byte..)?);
        let rendered_scalars = base_position_map
            .total_scalars()
            .checked_add(inserted_scalar_delta)?;
        if rendered_scalars != position_map.total_scalars() {
            return None;
        }
        let position_identity_seal = Arc::new(());
        let render_identity_seal = Arc::new(());
        Some(Self {
            document: document.clone(),
            validation_seal: validation,
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: schema_fingerprint.into(),
            canonical_schema: canonical_schema.clone(),
            position: PreparedPositionEvidence {
                total_scalars: position_map.total_scalars(),
                block_count: position_map.block_count(),
                position_map,
                identity: Arc::clone(&position_identity_seal),
            },
            position_identity_seal,
            render: PreparedRenderEvidence {
                rendered_text,
                rendered_scalars,
                identity: Arc::clone(&render_identity_seal),
            },
            render_identity_seal: Arc::clone(&render_identity_seal),
            document_text_bytes: raw_text_utf8_bytes,
            document_node_count: validation.stats.node_count,
            raw_text_scalars,
            raw_text_utf8_bytes,
            history_render: Some(PreparedHistoryRenderEvidence {
                base_document_root: base_document.root().clone(),
                target_top_level_index,
                inserted_scalar_delta,
                candidate_render_identity: Arc::clone(&render_identity_seal),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn prepare_history_render_transition(
        &self,
        state: &DerivedStateCache,
        document: &Document,
        derivations: &CompiledDocumentDerivations,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
    ) -> Option<
        Result<
            crate::render::incremental::CachedRenderTransition,
            crate::render::incremental::CachedRenderError,
        >,
    > {
        let proof = self.history_render.as_ref()?;
        let expected_rendered_scalars = state
            .rendered_scalars
            .checked_add(proof.inserted_scalar_delta)?;
        let expected_raw_scalars = state
            .validation_certificate
            .raw_text_scalars
            .checked_add(u64::from(proof.inserted_scalar_delta))?;
        if !state
            .document
            .root()
            .shares_storage_with(&proof.base_document_root)
            || !self.document.shares_root_storage_with(document)
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || self.validation_seal.stats.node_count != derivations.document_node_count
            || self.validation_seal.stats.node_count > resource_limits.max_document_nodes
            || self.validation_seal.stats.max_depth > resource_limits.max_document_depth
            || !Arc::ptr_eq(&proof.candidate_render_identity, &self.render_identity_seal)
            || !Arc::ptr_eq(&derivations.identity_seal, &self.position_identity_seal)
            || derivations.rendered_text != self.render.rendered_text
            || derivations.rendered_scalars != self.render.rendered_scalars
            || derivations.rendered_scalars != expected_rendered_scalars
            || self.raw_text_scalars != expected_raw_scalars
            || state.document.root().child_count() != document.root().child_count()
            || proof.target_top_level_index >= document.root().child_count()
            || !state
                .render_blocks
                .matches_identity(&state.document, schema_fingerprint)
        {
            return None;
        }
        Some(state.render_blocks.transition_localized_insert(
            &state.document,
            document,
            schema,
            proof.target_top_level_index,
            proof.inserted_scalar_delta,
            resource_limits,
        ))
    }

    #[cfg(test)]
    pub(in crate::yrs_engine) fn history_render_tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "missing",
            "baseDocument",
            "targetIndex",
            "scalarDelta",
            "renderIdentity",
        ]
    }

    #[cfg(test)]
    pub(in crate::yrs_engine) fn tamper_history_render_for_test(&mut self, case: &str) {
        if case == "missing" {
            self.history_render = None;
            return;
        }
        let proof = self
            .history_render
            .as_mut()
            .expect("history render tamper fixture retains its proof");
        match case {
            "baseDocument" => proof.base_document_root = self.document.root().clone(),
            "targetIndex" => {
                proof.target_top_level_index = proof.target_top_level_index.saturating_add(1)
            }
            "scalarDelta" => {
                proof.inserted_scalar_delta = proof.inserted_scalar_delta.saturating_add(1)
            }
            "renderIdentity" => proof.candidate_render_identity = Arc::new(()),
            _ => panic!("unknown history render evidence tamper case {case}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn finalize_deferred(
        self,
        _authority: &yrs_engine::compiler::CandidateValidationAuthority,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        validation: DocumentValidationReport,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &yrs_engine::canonical::CanonicalSchemaContext,
    ) -> Option<PreparedCandidateValidation> {
        if !self.document.shares_root_storage_with(document)
            || self.validation_seal != validation
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || !self.canonical_schema.ptr_eq(canonical_schema)
            || !canonical_artifact.matches_exact_source_document(document)
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !canonical_artifact.schema_context().ptr_eq(canonical_schema)
            || validation.stats.node_count == 0
            || validation.stats.node_count != self.document_node_count
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || !Arc::ptr_eq(&self.position.identity, &self.position_identity_seal)
            || !Arc::ptr_eq(&self.render.identity, &self.render_identity_seal)
            || self.position.position_map.total_scalars() != self.position.total_scalars
            || self.position.position_map.block_count() != self.position.block_count
            || self.render.rendered_scalars != self.position.total_scalars
            || canonical_artifact.text_scalar_len() != self.raw_text_scalars
            || canonical_artifact.text_utf8_bytes() != self.raw_text_utf8_bytes
            || self.document_text_bytes != self.raw_text_utf8_bytes
            || max_length.is_some_and(|limit| self.raw_text_scalars > u64::from(limit))
        {
            return None;
        }
        let derivations = CompiledDocumentDerivations {
            identity_seal: self.position.identity,
            position_map: self.position.position_map,
            rendered_text: self.render.rendered_text,
            rendered_scalars: self.render.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
        };
        Some(PreparedCandidateValidation {
            document: self.document,
            canonical_artifact: canonical_artifact.clone(),
            validation,
            resource_limits: self.resource_limits,
            editing_limits: self.editing_limits,
            max_length: self.max_length,
            schema_fingerprint: self.schema_fingerprint,
            derivations,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn derivations_for_prepared_history(
        &self,
        document: &Document,
        validation: DocumentValidationReport,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &yrs_engine::canonical::CanonicalSchemaContext,
        raw_text_scalars: u64,
        raw_text_utf8_bytes: usize,
    ) -> Option<CompiledDocumentDerivations> {
        if !self.document.shares_root_storage_with(document)
            || self.validation_seal != validation
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || !self.canonical_schema.ptr_eq(canonical_schema)
            || validation.stats.node_count == 0
            || validation.stats.node_count != self.document_node_count
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || !Arc::ptr_eq(&self.position.identity, &self.position_identity_seal)
            || !Arc::ptr_eq(&self.render.identity, &self.render_identity_seal)
            || self.position.position_map.total_scalars() != self.position.total_scalars
            || self.position.position_map.block_count() != self.position.block_count
            || self.render.rendered_scalars != self.position.total_scalars
            || self.raw_text_scalars != raw_text_scalars
            || self.raw_text_utf8_bytes != raw_text_utf8_bytes
            || self.document_text_bytes != raw_text_utf8_bytes
            || max_length.is_some_and(|limit| raw_text_scalars > u64::from(limit))
        {
            return None;
        }
        Some(CompiledDocumentDerivations {
            identity_seal: Arc::clone(&self.position.identity),
            position_map: self.position.position_map.clone(),
            rendered_text: self.render.rendered_text.clone(),
            rendered_scalars: self.render.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
        })
    }

    #[cfg(test)]
    pub(in crate::yrs_engine) fn tamper_position_for_test(&mut self) {
        self.position.total_scalars = self.position.total_scalars.saturating_add(1);
    }

    #[cfg(test)]
    pub(in crate::yrs_engine) fn tamper_render_for_test(&mut self) {
        self.render.rendered_scalars = self.render.rendered_scalars.saturating_add(1);
    }

    #[cfg(test)]
    pub(in crate::yrs_engine) fn tamper_same_summary_for_test(&mut self, case: &str) {
        match case {
            "position" => {
                self.position = PreparedPositionEvidence {
                    position_map: self.position.position_map.clone(),
                    total_scalars: self.position.total_scalars,
                    block_count: self.position.block_count,
                    identity: Arc::new(()),
                };
            }
            "render" => {
                let mut rendered_text = self.render.rendered_text.clone();
                let first = rendered_text
                    .chars()
                    .next()
                    .expect("render replacement fixture is nonempty");
                assert!(
                    first.is_ascii(),
                    "render replacement fixture starts in ASCII"
                );
                rendered_text
                    .replace_range(0..first.len_utf8(), if first == 'z' { "y" } else { "z" });
                self.render = PreparedRenderEvidence {
                    rendered_text,
                    rendered_scalars: self.render.rendered_scalars,
                    identity: Arc::new(()),
                };
            }
            _ => panic!("unknown same-summary evidence tamper case {case}"),
        }
    }
}

impl PreparedCandidateValidation {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn prepare(
        _authority: &yrs_engine::compiler::CandidateValidationAuthority,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        validation: DocumentValidationReport,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        position_map: PositionMap,
    ) -> Option<Self> {
        if crate::schema::schema_fingerprint(schema) != schema_fingerprint
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !canonical_artifact.matches_exact_source_document(document)
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || max_length
                .is_some_and(|limit| canonical_artifact.text_scalar_len() > u64::from(limit))
        {
            return None;
        }
        crate::transform::validate_canonical_marks(document, schema).ok()?;
        record_preview_position_map_derivation();
        record_preview_rendered_text_derivation();
        let rendered_text = crate::render::rendered_text(document, schema);
        let rendered_scalars = u32::try_from(rendered_text.chars().count()).ok()?;
        if rendered_scalars != position_map.total_scalars() {
            return None;
        }
        let derivations = CompiledDocumentDerivations {
            identity_seal: Arc::new(()),
            position_map,
            rendered_text,
            rendered_scalars,
            document_text_bytes: canonical_artifact.text_utf8_bytes(),
            document_node_count: validation.stats.node_count,
        };
        Some(Self {
            document: document.clone(),
            canonical_artifact: canonical_artifact.clone(),
            validation,
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: schema_fingerprint.into(),
            derivations,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admits_context(
        &self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &yrs_engine::canonical::CanonicalSchemaContext,
    ) -> bool {
        self.document.shares_root_storage_with(document)
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && self
                .canonical_artifact
                .matches_exact_source_document(document)
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self
                .canonical_artifact
                .schema_context()
                .ptr_eq(canonical_schema)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compiled_derivations(
        &self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &yrs_engine::canonical::CanonicalSchemaContext,
    ) -> Option<CompiledDocumentDerivations> {
        self.admits_context(
            document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            canonical_schema,
        )
        .then(|| self.derivations.clone())
    }

    #[cfg(test)]
    pub(crate) fn replace_canonical_artifact_for_test(
        &mut self,
        canonical_artifact: CanonicalArtifact,
    ) {
        self.canonical_artifact = canonical_artifact;
    }
}

impl PreparedCandidateValidation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finalize(
        self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &yrs_engine::canonical::CanonicalSchemaContext,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<FinalizedDerivedEvidence> {
        if !self.admits_context(
            document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            canonical_schema,
        ) || !Arc::ptr_eq(&derivations.identity_seal, &self.derivations.identity_seal)
            || derivations.position_map.total_scalars()
                != self.derivations.position_map.total_scalars()
            || derivations.position_map.block_count() != self.derivations.position_map.block_count()
            || derivations.rendered_text != self.derivations.rendered_text
            || derivations.rendered_scalars != self.derivations.rendered_scalars
            || derivations.document_text_bytes != self.derivations.document_text_bytes
            || derivations.document_node_count != self.validation.stats.node_count
            || canonical_artifact.text_utf8_bytes() != derivations.document_text_bytes
            || u64::from(derivations.rendered_scalars) < canonical_artifact.text_scalar_len()
        {
            return None;
        }
        let validation_certificate = DocumentValidationCertificate {
            stats: self.validation.stats,
            metrics: self.validation.metrics,
            resource_limits: self.resource_limits,
            schema_fingerprint: self.schema_fingerprint,
            canonical_artifact: canonical_artifact.clone(),
            canonical_fingerprint: canonical_artifact.sha256(),
            canonical_serialized_len: canonical_artifact.serialized_len(),
            canonical_fingerprint_materialized: true,
            raw_text_scalars: canonical_artifact.text_scalar_len(),
            raw_text_utf8_bytes: canonical_artifact.text_utf8_bytes(),
            document_revision,
            state_revision,
            yrs_state_epoch,
        };
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
}
