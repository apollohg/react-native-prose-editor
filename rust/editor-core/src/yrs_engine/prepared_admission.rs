#![allow(dead_code)] // The all-consumer cutover lands in the following redesign tasks.

use std::sync::Arc;

use crate::yrs_engine::{OperationError, OperationResult};

pub(crate) struct MaterializedMutationIdentity {
    pub(crate) canonical_fingerprint: [u8; 32],
    pub(crate) canonical_serialized_len: usize,
}

pub(crate) struct PreparedMutationContext {
    request_id: u64,
    base_document: crate::model::Document,
    canonical_artifact: super::canonical::CanonicalArtifact,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    schema_fingerprint: Box<str>,
    fragment_name: Box<str>,
    resource_limits: crate::boundary::ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    lookup_seed: Arc<super::mutation::MutationLookupSeed>,
    identity: Option<MaterializedMutationIdentity>,
}

pub(crate) struct LiveMutationAuthorityContext<'a, T: yrs::ReadTxn> {
    pub(crate) request_id: u64,
    pub(crate) installed: &'a super::derived_state::DerivedStateCache,
    pub(crate) txn: &'a T,
    pub(crate) fragment: &'a yrs::types::xml::XmlFragmentRef,
    pub(crate) fragment_name: &'a str,
    pub(crate) schema_fingerprint: &'a str,
    pub(crate) resource_limits: &'a crate::boundary::ResourceLimits,
    pub(crate) editing_limits: &'a super::EditingLimits,
    pub(crate) max_length: Option<u32>,
    pub(crate) document_revision: u64,
    pub(crate) state_revision: u64,
    pub(crate) yrs_state_epoch: u64,
}

pub(crate) struct StagedDerivedStateAuthority<'a> {
    installed: &'a super::derived_state::DerivedStateCache,
    prepared: &'a PreparedMutationContext,
}

pub(crate) struct InstalledDerivedStateAuthority<'a> {
    installed: &'a super::derived_state::DerivedStateCache,
}

pub(crate) trait DerivedStateAuthority {
    fn installed(&self) -> &super::derived_state::DerivedStateCache;

    fn lookup_seed(
        &self,
        request_id: u64,
    ) -> OperationResult<&Arc<super::mutation::MutationLookupSeed>>;

    fn materialized_identity(&self) -> Option<&MaterializedMutationIdentity>;
}

impl PreparedMutationContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        request_id: u64,
        base_document: crate::model::Document,
        canonical_artifact: super::canonical::CanonicalArtifact,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema_fingerprint: Box<str>,
        fragment_name: Box<str>,
        resource_limits: crate::boundary::ResourceLimits,
        editing_limits: super::EditingLimits,
        max_length: Option<u32>,
        lookup_seed: Arc<super::mutation::MutationLookupSeed>,
    ) -> Self {
        Self {
            request_id,
            base_document,
            canonical_artifact,
            document_revision,
            state_revision,
            yrs_state_epoch,
            schema_fingerprint,
            fragment_name,
            resource_limits,
            editing_limits,
            max_length,
            lookup_seed,
            identity: None,
        }
    }

    pub(crate) fn lookup_seed(&self) -> &Arc<super::mutation::MutationLookupSeed> {
        &self.lookup_seed
    }

    pub(crate) fn materialized_identity(&self) -> Option<&MaterializedMutationIdentity> {
        self.identity.as_ref()
    }

    pub(crate) fn set_materialized_identity(&mut self, identity: MaterializedMutationIdentity) {
        self.identity = Some(identity);
    }

    pub(crate) fn canonical_artifact(&self) -> &super::canonical::CanonicalArtifact {
        &self.canonical_artifact
    }

    pub(crate) fn request_id(&self) -> u64 {
        self.request_id
    }

    pub(crate) fn authority<'a, T: yrs::ReadTxn>(
        &'a self,
        live: LiveMutationAuthorityContext<'a, T>,
    ) -> super::OperationResult<StagedDerivedStateAuthority<'a>> {
        if self.request_id != live.request_id
            || self.document_revision != live.document_revision
            || self.state_revision != live.state_revision
            || self.yrs_state_epoch != live.yrs_state_epoch
            || self.fragment_name.as_ref() != live.fragment_name
            || self.schema_fingerprint.as_ref() != live.schema_fingerprint
            || self.resource_limits != *live.resource_limits
            || self.editing_limits != *live.editing_limits
            || self.max_length != live.max_length
            || live.installed.document_revision != live.document_revision
            || live.installed.state_revision != live.state_revision
            || live.installed.schema_fingerprint != live.schema_fingerprint
            || !self
                .base_document
                .shares_root_storage_with(&live.installed.document)
            || !self
                .canonical_artifact
                .ptr_eq(&live.installed.canonical_artifact)
            || self.canonical_artifact.schema_fingerprint() != live.schema_fingerprint
            || !self
                .lookup_seed
                .matches_canonical_artifact(&live.installed.canonical_artifact)
            || !self.lookup_seed.matches(
                live.txn,
                live.fragment,
                &live.installed.document,
                live.resource_limits,
                live.editing_limits,
                live.max_length,
                live.schema_fingerprint,
                live.yrs_state_epoch,
                live.document_revision,
            )
            || self.identity.as_ref().is_some_and(|identity| {
                !live.installed.matches_materialized_mutation_identity(
                    &self.canonical_artifact,
                    identity.canonical_fingerprint,
                    identity.canonical_serialized_len,
                    live.resource_limits,
                    live.schema_fingerprint,
                    live.document_revision,
                    live.state_revision,
                    live.yrs_state_epoch,
                )
            })
        {
            return Err(super::OperationError::engine_invariant_failed(
                self.request_id,
                None,
                "prepared mutation context does not match installed derived state",
            ));
        }
        Ok(StagedDerivedStateAuthority {
            installed: live.installed,
            prepared: self,
        })
    }
}

impl StagedDerivedStateAuthority<'_> {
    pub(crate) fn lookup_seed(&self) -> &Arc<super::mutation::MutationLookupSeed> {
        self.prepared.lookup_seed()
    }

    pub(crate) fn materialized_identity(&self) -> Option<&MaterializedMutationIdentity> {
        self.prepared.materialized_identity()
    }

    pub(crate) fn admit_existing_text_insert<T: yrs::ReadTxn>(
        &self,
        transaction: &super::TypedTransaction,
        allow_prepared_command_boundary: bool,
        document_position: u32,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
    ) -> Option<super::derived_state::LocalizedInsertAdmission> {
        if transaction.request_id != self.prepared.request_id {
            return None;
        }
        self.installed.admit_existing_text_insert_with_authority(
            transaction,
            allow_prepared_command_boundary,
            document_position,
            txn,
            fragment,
            self.lookup_seed(),
            self.materialized_identity(),
            &self.prepared.schema_fingerprint,
            &self.prepared.resource_limits,
            &self.prepared.editing_limits,
            self.prepared.max_length,
            self.prepared.yrs_state_epoch,
        )
    }

    pub(crate) fn validate_localized_insert<'a, T: yrs::ReadTxn>(
        &'a self,
        admission: &'a super::derived_state::LocalizedInsertAdmission,
        transaction: &super::TypedTransaction,
        document_position: u32,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
    ) -> Option<super::derived_state::ValidatedLocalizedInsertAdmission<'a>> {
        if transaction.request_id != self.prepared.request_id {
            return None;
        }
        admission.validate_current_with_authority(
            self.installed,
            transaction,
            document_position,
            txn,
            fragment,
            self.lookup_seed(),
            self.materialized_identity(),
            &self.prepared.resource_limits,
            &self.prepared.editing_limits,
            self.prepared.max_length,
            self.prepared.yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_lookup_transition<T: yrs::ReadTxn>(
        &self,
        transition: &super::compiler::MutationLookupTransition,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
        preview: &crate::model::Document,
        canonical_artifact: &super::canonical::CanonicalArtifact,
        next_yrs_state_epoch: u64,
        next_document_revision: u64,
    ) -> super::OperationResult<Arc<super::mutation::MutationLookupSeed>> {
        if transition.request_id() != self.prepared.request_id {
            return Err(super::OperationError::engine_invariant_failed(
                self.prepared.request_id,
                None,
                "prepared mutation lookup transition belongs to another request",
            ));
        }
        let prepared = match transition {
            super::compiler::MutationLookupTransition::Promote(promotion) => {
                self.lookup_seed().prepare_promotion(
                    txn,
                    fragment,
                    promotion,
                    &self.installed.document,
                    preview,
                    &self.prepared.resource_limits,
                    &self.prepared.editing_limits,
                    self.prepared.max_length,
                    &self.prepared.schema_fingerprint,
                    self.prepared.yrs_state_epoch,
                    self.prepared.document_revision,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?
            }
            super::compiler::MutationLookupTransition::Invalidate { .. } => {
                self.lookup_seed().prepare_unavailable_transition(
                    self.prepared.request_id,
                    txn,
                    fragment,
                    &self.installed.document,
                    preview,
                    &self.prepared.resource_limits,
                    &self.prepared.editing_limits,
                    self.prepared.max_length,
                    &self.prepared.schema_fingerprint,
                    self.prepared.yrs_state_epoch,
                    self.prepared.document_revision,
                    next_yrs_state_epoch,
                    next_document_revision,
                )?
            }
        };
        Ok(Arc::new(
            prepared.with_canonical_artifact(canonical_artifact),
        ))
    }
}

impl DerivedStateAuthority for StagedDerivedStateAuthority<'_> {
    fn installed(&self) -> &super::derived_state::DerivedStateCache {
        self.installed
    }

    fn lookup_seed(
        &self,
        request_id: u64,
    ) -> OperationResult<&Arc<super::mutation::MutationLookupSeed>> {
        if request_id != self.prepared.request_id || self.lookup_seed().is_unavailable() {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "staged derived-state authority has no ready matching mutation lookup seed",
            ));
        }
        Ok(self.lookup_seed())
    }

    fn materialized_identity(&self) -> Option<&MaterializedMutationIdentity> {
        self.materialized_identity()
    }
}

impl<'a> InstalledDerivedStateAuthority<'a> {
    pub(crate) fn new(installed: &'a super::derived_state::DerivedStateCache) -> Self {
        Self { installed }
    }

    pub(crate) fn lookup_seed(&self) -> &Arc<super::mutation::MutationLookupSeed> {
        &self.installed.mutation_lookup_seed
    }

    // Keep every live authority input explicit so the localized admission seal
    // cannot accidentally substitute values bundled by a different context.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_existing_text_insert<T: yrs::ReadTxn>(
        &self,
        transaction: &super::TypedTransaction,
        allow_prepared_command_boundary: bool,
        document_position: u32,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
        schema_fingerprint: &str,
        resource_limits: &crate::boundary::ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<super::derived_state::LocalizedInsertAdmission> {
        self.installed.admit_existing_text_insert_with_authority(
            transaction,
            allow_prepared_command_boundary,
            document_position,
            txn,
            fragment,
            self.lookup_seed(),
            None,
            schema_fingerprint,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_localized_insert<'b, T: yrs::ReadTxn>(
        &'b self,
        admission: &'b super::derived_state::LocalizedInsertAdmission,
        transaction: &super::TypedTransaction,
        document_position: u32,
        txn: &T,
        fragment: &yrs::types::xml::XmlFragmentRef,
        resource_limits: &crate::boundary::ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<super::derived_state::ValidatedLocalizedInsertAdmission<'b>> {
        admission.validate_current_with_authority(
            self.installed,
            transaction,
            document_position,
            txn,
            fragment,
            self.lookup_seed(),
            None,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }
}

impl DerivedStateAuthority for InstalledDerivedStateAuthority<'_> {
    fn installed(&self) -> &super::derived_state::DerivedStateCache {
        self.installed
    }

    fn lookup_seed(
        &self,
        request_id: u64,
    ) -> OperationResult<&Arc<super::mutation::MutationLookupSeed>> {
        let seed = self.lookup_seed();
        if seed.is_unavailable()
            || !seed.matches_canonical_artifact(&self.installed.canonical_artifact)
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "installed derived-state authority requires a ready matching mutation lookup seed",
            ));
        }
        Ok(seed)
    }

    fn materialized_identity(&self) -> Option<&MaterializedMutationIdentity> {
        None
    }
}

// Both variants stay inline to avoid adding an unadmitted heap allocation to
// the prepared-command hot path solely to reduce this private enum's size.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ExecutionSemanticAdmission {
    Eager(super::compiler::PreparedSemanticAdmission),
    Deferred(DeferredCommandAdmission),
}

impl ExecutionSemanticAdmission {
    pub(crate) fn requires_materialized_identity(&self) -> bool {
        matches!(self, Self::Deferred(_))
            || matches!(
                self,
                Self::Eager(admission) if admission.is_identity_dependent_insert()
            )
    }

    pub(crate) fn transaction(&self) -> &super::TypedTransaction {
        match self {
            Self::Eager(admission) => admission.transaction(),
            Self::Deferred(admission) => &admission.transaction,
        }
    }

    pub(crate) fn expected_document(&self) -> &crate::model::Document {
        match self {
            Self::Eager(admission) => admission.expected_document(),
            Self::Deferred(admission) => &admission.expected_document,
        }
    }

    pub(crate) fn pre_admit_seed_independent(
        &self,
        transaction: &super::TypedTransaction,
        expected_document: &crate::model::Document,
        editing_limits: &super::EditingLimits,
    ) -> OperationResult<()> {
        if self.transaction() != transaction
            || !self
                .expected_document()
                .shares_root_storage_with(expected_document)
        {
            return Err(OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "prepared command proof does not match its planned transaction",
            ));
        }
        match self {
            Self::Eager(admission) => {
                admission.pre_admit_seed_independent(transaction, expected_document, editing_limits)
            }
            Self::Deferred(admission) => admission.pre_admit_seed_independent(),
        }
    }
}

pub(crate) struct DeferredCommandHistoryEvidence {
    pub(crate) candidate_derivations: super::compiler::CompiledDocumentDerivations,
    pub(crate) canonical_fingerprint: [u8; 32],
    pub(crate) canonical_serialized_len: usize,
    pub(crate) canonical_text_scalar_len: u64,
    pub(crate) canonical_retained_bytes: usize,
    pub(crate) source_document_retained_bytes: usize,
}

pub(crate) struct DeferredCommandAdmission {
    request_id: u64,
    transaction: super::TypedTransaction,
    base_document: crate::model::Document,
    expected_document: crate::model::Document,
    prepared_selection: crate::selection::Selection,
    prepared_selection_seal: crate::selection::Selection,
    canonical_artifact: super::canonical::CanonicalArtifact,
    canonical_schema: super::canonical::CanonicalSchemaContext,
    canonical_format_version: u8,
    schema_fingerprint: Box<str>,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    resource_limits: crate::boundary::ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    validation_report: crate::transform::DocumentValidationReport,
    candidate_evidence: super::derived_state::PreparedCandidateEvidence,
    candidate_canonical: super::canonical::PreparedCanonicalCandidate,
    shape: DeferredInsertShapeProof,
    shape_seal: DeferredInsertShapeSeal,
    position_proof: DeferredInsertPositionProof,
    base_output_upper_bound: usize,
    candidate_output_upper_bound: usize,
    undo_units: u64,
    command_contract_kind: super::compiler::PreparedCommandContractKind,
}

pub(crate) struct DeferredInsertShapeProof {
    base_document: crate::model::Document,
    position: u32,
    inserted_text: Box<str>,
    inserted_marks: Vec<crate::model::Mark>,
    escaped_body_bytes: usize,
    target_top_level_index: usize,
    inserted_scalars: u32,
}

struct DeferredInsertShapeSeal {
    base_document_root: crate::model::Node,
    preview_document_root: crate::model::Node,
    position: u32,
    inserted_text: Box<str>,
    inserted_marks: Vec<crate::model::Mark>,
    escaped_body_bytes: usize,
    target_top_level_index: usize,
    inserted_scalars: u32,
}

struct DeferredInsertPositionProof {
    base_document_root: crate::model::Node,
    document_revision: u64,
    transaction_at: super::RevisionedPosition,
    document_position: u32,
}

impl DeferredInsertShapeProof {
    pub(crate) fn prepare(
        document: &crate::model::Document,
        operations: &[crate::command_planner::SemanticOperation],
    ) -> Option<Self> {
        let [crate::command_planner::SemanticOperation::InsertText { pos, text, marks }] =
            operations
        else {
            return None;
        };
        if text.is_empty() {
            return None;
        }
        let resolved = document.resolve(*pos).ok()?;
        let target_top_level_index = usize::try_from(*resolved.node_path.first()?).ok()?;
        let inserted_scalars = u32::try_from(text.chars().count()).ok()?;
        let parent = resolved.parent(document);
        let mut child_start = 0_u32;
        for child in parent.content()?.iter() {
            let child_end = child_start.checked_add(child.node_size())?;
            if child.is_text()
                && child_start < resolved.parent_offset
                && resolved.parent_offset < child_end
                && child.marks() == marks
            {
                return Some(Self {
                    base_document: document.clone(),
                    position: *pos,
                    inserted_text: text.clone().into_boxed_str(),
                    inserted_marks: marks.clone(),
                    escaped_body_bytes: checked_json_string_body_len(text)?,
                    target_top_level_index,
                    inserted_scalars,
                });
            }
            child_start = child_end;
        }
        None
    }

    pub(crate) fn escaped_body_bytes(&self) -> usize {
        self.escaped_body_bytes
    }
}

fn checked_json_string_body_len(text: &str) -> Option<usize> {
    text.chars().try_fold(0usize, |bytes, character| {
        let amount = match character {
            '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            other => other.len_utf8(),
        };
        bytes.checked_add(amount)
    })
}

impl DeferredCommandAdmission {
    pub(crate) fn transaction(&self) -> &super::TypedTransaction {
        &self.transaction
    }

    pub(crate) fn expected_document(&self) -> &crate::model::Document {
        &self.expected_document
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_history_render_transition(
        &self,
        state: &super::derived_state::DerivedStateCache,
        derivations: &super::compiler::CompiledDocumentDerivations,
        schema: &crate::schema::Schema,
        resource_limits: &crate::boundary::ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
    ) -> Option<
        Result<
            crate::render::incremental::CachedRenderTransition,
            crate::render::incremental::CachedRenderError,
        >,
    > {
        self.candidate_evidence.prepare_history_render_transition(
            state,
            &self.expected_document,
            derivations,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
        )
    }

    #[cfg(test)]
    pub(crate) fn history_render_tamper_cases_for_test() -> &'static [&'static str] {
        super::derived_state::PreparedCandidateEvidence::history_render_tamper_cases_for_test()
    }

    #[cfg(test)]
    pub(crate) fn tamper_history_render_for_test(&mut self, case: &str) {
        self.candidate_evidence.tamper_history_render_for_test(case);
    }

    #[cfg(test)]
    pub(crate) fn tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "requestId",
            "transaction",
            "baseDocument",
            "expectedDocument",
            "preparedSelection",
            "canonicalArtifact",
            "canonicalSchema",
            "canonicalFormat",
            "schemaFingerprint",
            "documentRevision",
            "stateRevision",
            "yrsStateEpoch",
            "resourceLimits",
            "editingLimits",
            "maxLength",
            "validationReport",
            "positionEvidence",
            "positionEvidenceSameSummary",
            "renderEvidence",
            "renderEvidenceSameSummary",
            "shapeBaseDocument",
            "shapePosition",
            "shapeText",
            "shapeMarks",
            "shapeDelta",
            "shapeTargetIndex",
            "shapeScalarDelta",
            "baseOutputProof",
            "candidateOutputProof",
            "undoUnits",
            "contractOracle",
        ]
    }

    #[cfg(test)]
    pub(crate) fn tamper_for_test(&mut self, case: &str) {
        match case {
            "requestId" => self.request_id = self.request_id.saturating_add(1),
            "transaction" => {
                self.transaction.base_document_revision =
                    self.transaction.base_document_revision.saturating_add(1)
            }
            "baseDocument" => self.base_document = self.expected_document.clone(),
            "expectedDocument" => self.expected_document = self.base_document.clone(),
            "preparedSelection" => self.prepared_selection = crate::selection::Selection::All,
            "canonicalArtifact" => {
                self.canonical_artifact = self
                    .canonical_artifact
                    .with_admission_upper_bound_for_test(self.base_output_upper_bound)
            }
            "canonicalSchema" => {
                self.canonical_schema =
                    super::canonical::CanonicalSchemaContext::new(self.canonical_schema.schema())
            }
            "canonicalFormat" => {
                self.canonical_format_version = self.canonical_format_version.saturating_add(1)
            }
            "schemaFingerprint" => self.schema_fingerprint = "tampered".into(),
            "documentRevision" => self.document_revision = self.document_revision.saturating_add(1),
            "stateRevision" => self.state_revision = self.state_revision.saturating_add(1),
            "yrsStateEpoch" => self.yrs_state_epoch = self.yrs_state_epoch.saturating_add(1),
            "resourceLimits" => {
                self.resource_limits.max_input_bytes =
                    self.resource_limits.max_input_bytes.saturating_add(1)
            }
            "editingLimits" => {
                self.editing_limits.max_undo_groups =
                    self.editing_limits.max_undo_groups.saturating_add(1)
            }
            "maxLength" => {
                self.max_length = Some(self.max_length.unwrap_or_default().saturating_add(1))
            }
            "validationReport" => {
                self.validation_report.stats.node_count =
                    self.validation_report.stats.node_count.saturating_add(1)
            }
            "positionEvidence" => self.candidate_evidence.tamper_position_for_test(),
            "positionEvidenceSameSummary" => self
                .candidate_evidence
                .tamper_same_summary_for_test("position"),
            "renderEvidence" => self.candidate_evidence.tamper_render_for_test(),
            "renderEvidenceSameSummary" => self
                .candidate_evidence
                .tamper_same_summary_for_test("render"),
            "shapeBaseDocument" => self.shape.base_document = self.expected_document.clone(),
            "shapePosition" => self.shape.position = self.shape.position.saturating_add(1),
            "shapeText" => {
                self.shape.inserted_text = format!("{}x", self.shape.inserted_text).into_boxed_str()
            }
            "shapeMarks" => {
                self.shape.inserted_marks = vec![crate::model::Mark::new(
                    "bold".into(),
                    std::collections::HashMap::new(),
                )]
            }
            "shapeDelta" => {
                self.shape.escaped_body_bytes = self.shape.escaped_body_bytes.saturating_add(1)
            }
            "shapeTargetIndex" => {
                self.shape.target_top_level_index =
                    self.shape.target_top_level_index.saturating_add(1)
            }
            "shapeScalarDelta" => {
                self.shape.inserted_scalars = self.shape.inserted_scalars.saturating_add(1)
            }
            "baseOutputProof" => {
                self.base_output_upper_bound = self.base_output_upper_bound.saturating_add(1)
            }
            "candidateOutputProof" => {
                self.candidate_output_upper_bound =
                    self.candidate_output_upper_bound.saturating_add(1)
            }
            "undoUnits" => self.undo_units = self.undo_units.saturating_add(1),
            "contractOracle" => {
                self.command_contract_kind = super::compiler::PreparedCommandContractKind::RootWrap
            }
            _ => panic!("unknown deferred admission tamper case {case}"),
        }
    }

    #[cfg(test)]
    pub(crate) fn tamper_same_summary_evidence_for_test(&mut self, case: &str) {
        self.candidate_evidence.tamper_same_summary_for_test(case);
    }

    #[cfg(test)]
    pub(crate) fn tamper_matching_transaction_position_for_test(
        &mut self,
        live: &mut super::TypedTransaction,
    ) {
        let [super::TypedOperation::InsertText { at: sealed, .. }] =
            self.transaction.operations.as_mut_slice()
        else {
            panic!("position tamper requires deferred insert transaction")
        };
        let [super::TypedOperation::InsertText { at: live, .. }] = live.operations.as_mut_slice()
        else {
            panic!("position tamper requires live insert transaction")
        };
        sealed.offset = sealed.offset.saturating_add(1);
        live.offset = live.offset.saturating_add(1);
        assert_eq!(
            sealed, live,
            "position tamper keeps transaction copies equal"
        );
    }

    #[cfg(test)]
    pub(crate) fn warm_candidate_caches_for_test(&self) -> (usize, [u8; 32]) {
        self.candidate_canonical.warm_scalar_caches_for_test()
    }

    #[cfg(test)]
    pub(crate) fn tamper_candidate_cache_for_test(&mut self, case: &str) {
        self.candidate_canonical.tamper_scalar_cache_for_test(case);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        context: &super::commands::PlanningContext<'_>,
        transaction: &super::TypedTransaction,
        simulated: &crate::command_planner::SimulatedCommandPlan,
        shape: DeferredInsertShapeProof,
    ) -> OperationResult<Self> {
        let base_output_upper_bound = context
            .canonical_artifact
            .admitted_serialized_upper_bound_option()
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    context.request_id,
                    Some(0),
                    "deferred insert lost its admitted base output bound",
                )
            })?;
        let candidate_output_upper_bound = base_output_upper_bound
            .checked_add(shape.escaped_body_bytes())
            .filter(|candidate| *candidate <= context.editing_limits.max_derived_output_bytes)
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    context.request_id,
                    Some(0),
                    "deferred insert output proof is not admissible",
                )
            })?;
        let validation_report = crate::transform::DocumentValidator::validate_report(
            &simulated.document,
            context.schema,
            context.resource_limits,
        )
        .map_err(|error| {
            OperationError::document_invalid(context.request_id, None, "command", error.to_string())
        })?;
        if let Some(limit) = context.max_length {
            let actual = simulated.document.root().text_content().chars().count();
            if actual > limit as usize {
                return Err(OperationError::document_limit_exceeded(
                    context.request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual as u64,
                ));
            }
        }
        let undo_units = shape.inserted_text.chars().count() as u64;
        if undo_units > context.editing_limits.max_undo_retained_units {
            return Err(OperationError::operation_limit_exceeded(
                context.request_id,
                Some(0),
                "maxUndoRetainedUnits",
                context.editing_limits.max_undo_retained_units,
                undo_units,
            ));
        }
        let candidate_canonical = super::canonical::PreparedCanonicalCandidate::prepare(
            &simulated.document,
            context.canonical_schema,
            candidate_output_upper_bound,
        );
        let candidate_raw_text_scalars = context
            .canonical_artifact
            .text_scalar_len()
            .checked_add(undo_units)
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    context.request_id,
                    Some(0),
                    "deferred insert scalar evidence overflowed",
                )
            })?;
        let candidate_raw_text_utf8_bytes = context
            .canonical_artifact
            .text_utf8_bytes()
            .checked_add(shape.inserted_text.len())
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    context.request_id,
                    Some(0),
                    "deferred insert text-byte evidence overflowed",
                )
            })?;
        let candidate_evidence = super::derived_state::PreparedCandidateEvidence::prepare_deferred(
            context.document,
            context.position_map,
            context.rendered_text,
            &simulated.document,
            validation_report,
            context.schema,
            context.canonical_schema,
            context.resource_limits,
            context.editing_limits,
            context.max_length,
            candidate_raw_text_scalars,
            candidate_raw_text_utf8_bytes,
            shape.position,
            &shape.inserted_text,
            shape.target_top_level_index,
            shape.inserted_scalars,
        )
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                context.request_id,
                Some(0),
                "deferred insert candidate evidence could not be prepared",
            )
        })?;
        let shape_seal = DeferredInsertShapeSeal {
            base_document_root: shape.base_document.root().clone(),
            preview_document_root: simulated.document.root().clone(),
            position: shape.position,
            inserted_text: shape.inserted_text.clone(),
            inserted_marks: shape.inserted_marks.clone(),
            escaped_body_bytes: shape.escaped_body_bytes,
            target_top_level_index: shape.target_top_level_index,
            inserted_scalars: shape.inserted_scalars,
        };
        let [super::TypedOperation::InsertText { at, .. }] = transaction.operations.as_slice()
        else {
            return Err(OperationError::engine_invariant_failed(
                context.request_id,
                Some(0),
                "deferred insert lost its exact transaction position",
            ));
        };
        if at.kind != super::EditorOffsetKind::Scalar
            || at.affinity != super::Affinity::After
            || context
                .position_map
                .scalar_to_doc(at.offset, context.document)
                != shape.position
            || context
                .position_map
                .doc_to_scalar(shape.position, context.document)
                != at.offset
        {
            return Err(OperationError::engine_invariant_failed(
                context.request_id,
                Some(0),
                "deferred insert transaction position does not match its strict-interior shape",
            ));
        }
        let position_proof = DeferredInsertPositionProof {
            base_document_root: context.document.root().clone(),
            document_revision: context.revision,
            transaction_at: *at,
            document_position: shape.position,
        };
        let admission = Self {
            request_id: context.request_id,
            transaction: transaction.clone(),
            base_document: context.document.clone(),
            expected_document: simulated.document.clone(),
            prepared_selection: simulated.selection.clone(),
            prepared_selection_seal: simulated.selection.clone(),
            canonical_artifact: context.canonical_artifact.clone(),
            canonical_schema: context.canonical_schema.clone(),
            canonical_format_version: context.canonical_schema.format_version(),
            schema_fingerprint: context.canonical_schema.schema_fingerprint().into(),
            document_revision: context.revision,
            state_revision: context.state_revision,
            yrs_state_epoch: context.yrs_state_epoch,
            resource_limits: context.resource_limits.clone(),
            editing_limits: context.editing_limits.clone(),
            max_length: context.max_length,
            validation_report,
            candidate_evidence,
            candidate_canonical,
            shape,
            shape_seal,
            position_proof,
            base_output_upper_bound,
            candidate_output_upper_bound,
            undo_units,
            command_contract_kind: super::compiler::PreparedCommandContractKind::None,
        };
        #[cfg(test)]
        super::observability::record_deferred_capsule_created();
        Ok(admission)
    }

    pub(crate) fn pre_admit_seed_independent(&self) -> OperationResult<()> {
        if self.request_id != self.transaction.request_id
            || self.document_revision != self.transaction.base_document_revision
            || !self
                .shape
                .base_document
                .shares_root_storage_with(&self.base_document)
            || !self
                .canonical_artifact
                .matches_exact_source_document(&self.base_document)
            || self
                .canonical_artifact
                .admitted_serialized_upper_bound_option()
                != Some(self.base_output_upper_bound)
            || self.candidate_canonical.admission_upper_bound() != self.candidate_output_upper_bound
            || self.prepared_selection != self.prepared_selection_seal
            || self.canonical_format_version != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !self
                .shape
                .base_document
                .root()
                .shares_storage_with(&self.shape_seal.base_document_root)
            || !self
                .expected_document
                .root()
                .shares_storage_with(&self.shape_seal.preview_document_root)
            || self.shape.position != self.shape_seal.position
            || self.shape.inserted_text != self.shape_seal.inserted_text
            || self.shape.inserted_marks != self.shape_seal.inserted_marks
            || self.shape.escaped_body_bytes != self.shape_seal.escaped_body_bytes
            || self.shape.target_top_level_index != self.shape_seal.target_top_level_index
            || self.shape.inserted_scalars != self.shape_seal.inserted_scalars
            || self.command_contract_kind != super::compiler::PreparedCommandContractKind::None
        {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                None,
                "deferred semantic admission does not match its sealed command context",
            ));
        }
        Ok(())
    }

    pub(crate) fn prepare_history_evidence(
        &self,
    ) -> OperationResult<DeferredCommandHistoryEvidence> {
        self.pre_admit_seed_independent()?;
        let (canonical_serialized_len, canonical_fingerprint) =
            self.candidate_canonical.exact_history_identity();
        if canonical_serialized_len > self.candidate_output_upper_bound
            || canonical_serialized_len > self.editing_limits.max_derived_output_bytes
        {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(0),
                "deferred insert exact output exceeded its conservative proof",
            ));
        }
        let candidate_derivations = self
            .candidate_evidence
            .derivations_for_prepared_history(
                &self.expected_document,
                self.validation_report,
                &self.resource_limits,
                &self.editing_limits,
                self.max_length,
                &self.schema_fingerprint,
                &self.canonical_schema,
                self.candidate_canonical.text_scalar_len(),
                self.candidate_canonical.text_utf8_bytes(),
            )
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(0),
                    "deferred insert history evidence does not match its sealed candidate",
                )
            })?;
        if !self
            .candidate_canonical
            .matches_exact_source_document(&self.expected_document)
        {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(0),
                "deferred insert history evidence does not match its candidate document",
            ));
        }
        let Some(retained_charge) = self.candidate_canonical.history_snapshot_retained_charge()
        else {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(0),
                "deferred insert history retention accounting overflowed",
            ));
        };
        Ok(DeferredCommandHistoryEvidence {
            candidate_derivations,
            canonical_fingerprint,
            canonical_serialized_len,
            canonical_text_scalar_len: self.candidate_canonical.text_scalar_len(),
            canonical_retained_bytes: retained_charge.canonical_retained_bytes,
            source_document_retained_bytes: retained_charge.source_document_retained_bytes,
        })
    }

    pub(crate) fn undo_units(&self) -> u64 {
        self.undo_units
    }

    pub(crate) fn into_eager(self) -> OperationResult<super::compiler::PreparedSemanticAdmission> {
        self.pre_admit_seed_independent()?;
        let exact_len = self.candidate_canonical.serialized_len();
        if exact_len > self.candidate_output_upper_bound {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(0),
                "deferred insert exact output exceeded its conservative proof",
            ));
        }
        let canonical_artifact = self
            .candidate_canonical
            .seal_with_known_serialized_len(exact_len)
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(0),
                    "deferred insert scalar caches diverged from its canonical projection",
                )
            })?;
        super::compiler::PreparedSemanticAdmission::from_deferred_insert(
            self.request_id,
            self.document_revision,
            self.state_revision,
            self.yrs_state_epoch,
            self.schema_fingerprint,
            self.transaction,
            self.expected_document,
            canonical_artifact,
            self.canonical_schema,
            self.validation_report,
            self.candidate_evidence,
            self.resource_limits,
            self.editing_limits,
            self.max_length,
            self.undo_units,
        )
    }

    pub(super) fn validate_finalization_context(
        &self,
        staged: &StagedDerivedStateAuthority<'_>,
        live: super::compiler::PreparedSemanticLiveContext<'_>,
    ) -> OperationResult<()> {
        let prepared = staged.prepared;
        let identity = prepared.materialized_identity().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                self.request_id,
                None,
                "deferred semantic admission requires materialized base identity",
            )
        })?;
        let [super::TypedOperation::InsertText { at, text, marks }] =
            self.transaction.operations.as_slice()
        else {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                None,
                "deferred semantic admission lost its exact insert transaction",
            ));
        };
        if self.request_id != prepared.request_id
            || self.request_id != live.transaction.request_id
            || self.transaction != *live.transaction
            || self.document_revision != prepared.document_revision
            || self.document_revision != live.transaction.base_document_revision
            || self.state_revision != prepared.state_revision
            || self.yrs_state_epoch != prepared.yrs_state_epoch
            || !self
                .base_document
                .shares_root_storage_with(&prepared.base_document)
            || !self
                .base_document
                .shares_root_storage_with(&staged.installed.document)
            || !self
                .expected_document
                .shares_root_storage_with(live.expected_preview)
            || self.prepared_selection != self.prepared_selection_seal
            || !self.canonical_artifact.ptr_eq(&prepared.canonical_artifact)
            || !self
                .canonical_artifact
                .ptr_eq(&staged.installed.canonical_artifact)
            || !self
                .canonical_artifact
                .matches_exact_source_document(&self.base_document)
            || self.canonical_artifact.schema_fingerprint() != self.schema_fingerprint.as_ref()
            || self.canonical_artifact.format_version() != self.canonical_format_version
            || self.canonical_format_version != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !self.canonical_schema.ptr_eq(live.canonical_schema)
            || !self
                .canonical_artifact
                .schema_context()
                .ptr_eq(&self.canonical_schema)
            || self.schema_fingerprint.as_ref() != prepared.schema_fingerprint.as_ref()
            || self.schema_fingerprint.as_ref() != self.canonical_schema.schema_fingerprint()
            || self.resource_limits != prepared.resource_limits
            || self.editing_limits != prepared.editing_limits
            || self.max_length != prepared.max_length
            || self.validation_report.stats.node_count == 0
            || !self
                .shape
                .base_document
                .root()
                .shares_storage_with(&self.shape_seal.base_document_root)
            || !self
                .expected_document
                .root()
                .shares_storage_with(&self.shape_seal.preview_document_root)
            || self.shape.position != self.shape_seal.position
            || self.shape.inserted_text != self.shape_seal.inserted_text
            || self.shape.inserted_marks != self.shape_seal.inserted_marks
            || self.shape.escaped_body_bytes != self.shape_seal.escaped_body_bytes
            || self.shape.target_top_level_index != self.shape_seal.target_top_level_index
            || self.shape.inserted_scalars != self.shape_seal.inserted_scalars
            || !self
                .base_document
                .root()
                .shares_storage_with(&self.position_proof.base_document_root)
            || self.position_proof.document_revision != self.document_revision
            || self.position_proof.transaction_at != *at
            || self.position_proof.transaction_at.kind != super::EditorOffsetKind::Scalar
            || self.position_proof.transaction_at.affinity != super::Affinity::After
            || self.position_proof.document_position != self.shape.position
            || self.shape.inserted_text.as_ref() != text
            || self.shape.inserted_marks != *marks
            || self.undo_units != self.shape.inserted_text.chars().count() as u64
            || self.undo_units > self.editing_limits.max_undo_retained_units
            || self
                .canonical_artifact
                .admitted_serialized_upper_bound_option()
                != Some(self.base_output_upper_bound)
            || self.candidate_canonical.admission_upper_bound() != self.candidate_output_upper_bound
            || self
                .base_output_upper_bound
                .checked_add(self.shape.escaped_body_bytes)
                != Some(self.candidate_output_upper_bound)
            || self.candidate_output_upper_bound > self.editing_limits.max_derived_output_bytes
            || self.command_contract_kind != super::compiler::PreparedCommandContractKind::None
            || identity.canonical_fingerprint != self.canonical_artifact.sha256()
            || identity.canonical_serialized_len != self.canonical_artifact.serialized_len()
        {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                None,
                "deferred semantic admission does not match staged live authority",
            ));
        }
        Ok(())
    }

    pub(super) fn into_finalization_parts(
        self,
        _authority: &super::compiler::DeferredAdmissionAuthority,
        staged: &StagedDerivedStateAuthority<'_>,
        live: super::compiler::PreparedSemanticLiveContext<'_>,
    ) -> OperationResult<DeferredFinalizationParts> {
        self.validate_finalization_context(staged, live)?;
        let base_canonical_serialized_len = staged
            .materialized_identity()
            .expect("validated deferred context has materialized identity")
            .canonical_serialized_len;
        Ok(DeferredFinalizationParts {
            request_id: self.request_id,
            transaction: self.transaction,
            expected_document: self.expected_document,
            canonical_schema: self.canonical_schema,
            schema_fingerprint: self.schema_fingerprint,
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            yrs_state_epoch: self.yrs_state_epoch,
            resource_limits: self.resource_limits,
            editing_limits: self.editing_limits,
            max_length: self.max_length,
            validation_report: self.validation_report,
            candidate_evidence: self.candidate_evidence,
            candidate_canonical: self.candidate_canonical,
            base_canonical_serialized_len,
            escaped_body_bytes: self.shape.escaped_body_bytes,
            candidate_output_upper_bound: self.candidate_output_upper_bound,
            undo_units: self.undo_units,
        })
    }
}

pub(super) struct DeferredFinalizationParts {
    pub(super) request_id: u64,
    pub(super) transaction: super::TypedTransaction,
    pub(super) expected_document: crate::model::Document,
    pub(super) canonical_schema: super::canonical::CanonicalSchemaContext,
    pub(super) schema_fingerprint: Box<str>,
    pub(super) document_revision: u64,
    pub(super) state_revision: u64,
    pub(super) yrs_state_epoch: u64,
    pub(super) resource_limits: crate::boundary::ResourceLimits,
    pub(super) editing_limits: super::EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) validation_report: crate::transform::DocumentValidationReport,
    pub(super) candidate_evidence: super::derived_state::PreparedCandidateEvidence,
    pub(super) candidate_canonical: super::canonical::PreparedCanonicalCandidate,
    pub(super) base_canonical_serialized_len: usize,
    pub(super) escaped_body_bytes: usize,
    pub(super) candidate_output_upper_bound: usize,
    pub(super) undo_units: u64,
}

pub(crate) struct PreparedCommandHistoryAdmission {
    pub(crate) limits: super::history::PreparedHistoryLimits,
    pub(crate) before: super::history::HistoryLocalState,
    pub(crate) after: super::history::HistorySnapshotTemplate,
    pub(crate) candidate_derivations: super::compiler::CompiledDocumentDerivations,
    pub(crate) candidate_render: crate::render::incremental::CachedRenderTransition,
}

pub(crate) struct PreparedExecutionAdmission {
    semantic: ExecutionSemanticAdmission,
    history: Option<PreparedCommandHistoryAdmission>,
}

impl PreparedExecutionAdmission {
    pub(crate) fn new(
        semantic: ExecutionSemanticAdmission,
        history: Option<PreparedCommandHistoryAdmission>,
    ) -> Self {
        Self { semantic, history }
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ExecutionSemanticAdmission,
        Option<PreparedCommandHistoryAdmission>,
    ) {
        (self.semantic, self.history)
    }
}
