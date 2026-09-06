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

include!("prepared_admission/deferred.rs");

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
