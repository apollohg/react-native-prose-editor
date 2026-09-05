use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::DocumentValidator;
use crate::yrs_engine;
use crate::yrs_engine::canonical::{CanonicalArtifact, CanonicalSchemaContext};
use crate::yrs_engine::compiler::CompiledDocumentDerivations;
use crate::yrs_engine::{
    EditingLimits, OperationError, OperationResult, TypedOperation, TypedTransaction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedCommandContractKind {
    None,
    RootWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PreparedCommandContractOracle {
    None,
    RootWrap {
        direct_insertion_units: u64,
        direct_growth_bytes: usize,
        replaced_children: u32,
    },
}

impl PreparedCommandContractOracle {
    pub(super) fn admits_transaction(
        self,
        transaction: &TypedTransaction,
        has_candidate_validation: bool,
    ) -> bool {
        match self {
            Self::None => true,
            Self::RootWrap {
                replaced_children, ..
            } => {
                let [TypedOperation::ReplaceStructure(replacement)] =
                    transaction.operations.as_slice()
                else {
                    return false;
                };
                let (from_child, to_child) = replacement.child_window();
                has_candidate_validation
                    && replacement.parent_path().is_empty()
                    && replacement.content().child_count() == 1
                    && replaced_children > 0
                    && to_child.checked_sub(from_child) == Some(replaced_children)
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedSemanticAdmission {
    pub(super) request_id: u64,
    pub(super) document_revision: u64,
    pub(super) state_revision: u64,
    pub(super) yrs_state_epoch: u64,
    pub(super) schema_fingerprint: Box<str>,
    pub(super) transaction: TypedTransaction,
    pub(super) expected_document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) candidate_validation: Option<yrs_engine::derived_state::PreparedCandidateValidation>,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) undo_units: u64,
    pub(super) command_contract_oracle: PreparedCommandContractOracle,
}

pub(super) struct PreparedSemanticConstruction {
    pub(super) request_id: u64,
    pub(super) document_revision: u64,
    pub(super) state_revision: u64,
    pub(super) yrs_state_epoch: u64,
    pub(super) schema_fingerprint: Box<str>,
    pub(super) transaction: TypedTransaction,
    pub(super) expected_document: Document,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) candidate_validation: Option<yrs_engine::derived_state::PreparedCandidateValidation>,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: EditingLimits,
    pub(super) max_length: Option<u32>,
    pub(super) undo_units: u64,
    pub(super) command_contract_oracle: PreparedCommandContractOracle,
}

/// Unforgeable authority for the raw candidate-evidence constructor. The type
/// is visible to `derived_state` so it can require the capability, while its
/// private field keeps construction inside this compiler module.
pub(in crate::yrs_engine) struct CandidateValidationAuthority(());

/// One-shot authority for consuming a deferred semantic admission. Only this
/// compiler module can mint the capability, after staged/live seals pass.
pub(in crate::yrs_engine) struct DeferredAdmissionAuthority(());

#[derive(Clone, Copy)]
pub(crate) struct PreparedSemanticLiveContext<'a> {
    pub(crate) transaction: &'a yrs_engine::TypedTransaction,
    pub(crate) expected_preview: &'a crate::model::Document,
    pub(crate) canonical_schema: &'a yrs_engine::canonical::CanonicalSchemaContext,
}

/// Opaque, compiler-minted candidate input. Callers can request a seed and use
/// it to normalize a simulated selection, but cannot supply or inspect the
/// position map that will become validation evidence.
pub(in crate::yrs_engine) struct PreparedCandidateSeed {
    pub(super) document: Document,
    pub(super) position_map: PositionMap,
    pub(super) schema_fingerprint: Box<str>,
    pub(super) canonical_schema: CanonicalSchemaContext,
    pub(super) resource_limits: ResourceLimits,
    pub(super) editing_limits: EditingLimits,
    pub(super) max_length: Option<u32>,
}

impl PreparedCandidateSeed {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn mint(
        request_id: u64,
        document: &Document,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
    ) -> OperationResult<Self> {
        let schema_fingerprint = crate::schema::schema_fingerprint(schema);
        if schema_fingerprint != canonical_schema.schema_fingerprint() {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "candidate seed schema does not match its canonical context",
            ));
        }
        Ok(Self {
            document: document.clone(),
            position_map: PositionMap::build(document, schema),
            schema_fingerprint: schema_fingerprint.into(),
            canonical_schema: canonical_schema.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
        })
    }

    pub(in crate::yrs_engine) fn normalize_selection(&self, selection: &Selection) -> Selection {
        selection
            .clone()
            .normalized(&self.document, &self.position_map)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn consume(
        self,
        request_id: u64,
        document: &Document,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
    ) -> OperationResult<PositionMap> {
        if !self.document.shares_root_storage_with(document)
            || self.schema_fingerprint.as_ref() != crate::schema::schema_fingerprint(schema)
            || !self.canonical_schema.ptr_eq(canonical_schema)
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(0),
                "prepared candidate seed does not match the admitted document context",
            ));
        }
        Ok(self.position_map)
    }
}

pub(super) fn prepare_command_contract_oracle(
    request_id: u64,
    transaction: &TypedTransaction,
    command_contract_kind: PreparedCommandContractKind,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> OperationResult<PreparedCommandContractOracle> {
    match command_contract_kind {
        PreparedCommandContractKind::None => Ok(PreparedCommandContractOracle::None),
        PreparedCommandContractKind::RootWrap => {
            let [TypedOperation::ReplaceStructure(replacement)] = transaction.operations.as_slice()
            else {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "prepared root wrap has no exact structural replacement",
                ));
            };
            let (from_child, to_child) = replacement.child_window();
            let replaced_children = to_child.checked_sub(from_child).filter(|count| *count > 0);
            let list_node = replacement.content().child(0);
            if !replacement.parent_path().is_empty()
                || replacement.content().child_count() != 1
                || replaced_children.is_none()
                || list_node.is_none()
            {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "prepared root wrap replacement is not one nonempty root window",
                ));
            }
            let metrics = yrs_engine::mutation::direct_root_wrap_metrics(
                request_id,
                0,
                list_node.expect("checked prepared root-wrap child"),
                schema,
                resource_limits,
            )?;
            Ok(PreparedCommandContractOracle::RootWrap {
                direct_insertion_units: metrics.insertion_units,
                direct_growth_bytes: metrics.growth_bytes,
                replaced_children: replaced_children
                    .expect("checked prepared root-wrap child window"),
            })
        }
    }
}

pub(crate) fn finalize_deferred_admission(
    staged: &yrs_engine::prepared_admission::StagedDerivedStateAuthority<'_>,
    deferred: yrs_engine::prepared_admission::DeferredCommandAdmission,
    live: PreparedSemanticLiveContext<'_>,
) -> OperationResult<PreparedSemanticAdmission> {
    deferred.validate_finalization_context(staged, live)?;
    let authority = DeferredAdmissionAuthority(());
    PreparedSemanticAdmission::finalize_from_validated_evidence(&authority, staged, deferred, live)
}

impl PreparedSemanticAdmission {
    pub(super) fn from_post_validation_construction(
        construction: PreparedSemanticConstruction,
    ) -> Self {
        Self {
            request_id: construction.request_id,
            document_revision: construction.document_revision,
            state_revision: construction.state_revision,
            yrs_state_epoch: construction.yrs_state_epoch,
            schema_fingerprint: construction.schema_fingerprint,
            transaction: construction.transaction,
            expected_document: construction.expected_document,
            canonical_artifact: construction.canonical_artifact,
            candidate_validation: construction.candidate_validation,
            resource_limits: construction.resource_limits,
            editing_limits: construction.editing_limits,
            max_length: construction.max_length,
            undo_units: construction.undo_units,
            command_contract_oracle: construction.command_contract_oracle,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_deferred_insert(
        request_id: u64,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema_fingerprint: Box<str>,
        transaction: TypedTransaction,
        expected_document: Document,
        canonical_artifact: CanonicalArtifact,
        canonical_schema: CanonicalSchemaContext,
        validation: crate::transform::DocumentValidationReport,
        candidate_evidence: yrs_engine::derived_state::PreparedCandidateEvidence,
        resource_limits: ResourceLimits,
        editing_limits: EditingLimits,
        max_length: Option<u32>,
        undo_units: u64,
    ) -> OperationResult<Self> {
        if !canonical_artifact.matches_exact_source_document(&expected_document)
            || canonical_artifact.schema_fingerprint() != schema_fingerprint.as_ref()
            || validation.stats.node_count == 0
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(0),
                "deferred insert candidate evidence is not internally consistent",
            ));
        }
        let authority = CandidateValidationAuthority(());
        let candidate_validation = candidate_evidence
            .finalize_deferred(
                &authority,
                &expected_document,
                &canonical_artifact,
                validation,
                &resource_limits,
                &editing_limits,
                max_length,
                schema_fingerprint.as_ref(),
                &canonical_schema,
            )
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "deferred insert validation evidence could not be sealed",
                )
            })?;
        Ok(Self::from_post_validation_construction(
            PreparedSemanticConstruction {
                request_id,
                document_revision,
                state_revision,
                yrs_state_epoch,
                schema_fingerprint,
                transaction,
                expected_document,
                canonical_artifact,
                candidate_validation: Some(candidate_validation),
                resource_limits,
                editing_limits,
                max_length,
                undo_units,
                command_contract_oracle: PreparedCommandContractOracle::None,
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::yrs_engine) fn prepare_single_operation(
        request_id: u64,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        transaction: &TypedTransaction,
        expected_document: &Document,
        candidate_seed: Option<PreparedCandidateSeed>,
        known_canonical_serialized_len: Option<usize>,
        undo_units: u64,
        command_contract_kind: PreparedCommandContractKind,
    ) -> OperationResult<Self> {
        let validation =
            DocumentValidator::validate_report(expected_document, schema, resource_limits)
                .map_err(|error| {
                    OperationError::document_invalid(request_id, None, "command", error.to_string())
                })?;
        if let Some(limit) = max_length {
            let actual = expected_document.root().text_content().chars().count();
            if actual > limit as usize {
                return Err(OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual as u64,
                ));
            }
        }
        let command_contract_oracle = prepare_command_contract_oracle(
            request_id,
            transaction,
            command_contract_kind,
            schema,
            resource_limits,
        )?;
        Self::prepare_single_operation_from_validation(
            request_id,
            document_revision,
            state_revision,
            yrs_state_epoch,
            schema,
            canonical_schema,
            resource_limits,
            editing_limits,
            max_length,
            transaction,
            expected_document,
            validation,
            candidate_seed,
            known_canonical_serialized_len,
            undo_units,
            command_contract_oracle,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_single_operation_from_validation(
        request_id: u64,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        transaction: &TypedTransaction,
        expected_document: &Document,
        validation: crate::transform::DocumentValidationReport,
        candidate_seed: Option<PreparedCandidateSeed>,
        known_canonical_serialized_len: Option<usize>,
        undo_units: u64,
        command_contract_oracle: PreparedCommandContractOracle,
    ) -> OperationResult<Self> {
        let candidate_position_map = candidate_seed
            .map(|seed| {
                seed.consume(
                    request_id,
                    expected_document,
                    schema,
                    canonical_schema,
                    resource_limits,
                    editing_limits,
                    max_length,
                )
            })
            .transpose()?;
        let canonical_artifact = known_canonical_serialized_len
            .map_or_else(
                || canonical_schema.derive(expected_document),
                |serialized_len| {
                    canonical_schema
                        .derive_with_known_serialized_len(expected_document, serialized_len)
                },
            )
            .map_err(|error| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    format!("preview serialization failed: {error}"),
                )
            })?;
        let candidate_validation = candidate_position_map
            .map(|position_map| {
                let authority = CandidateValidationAuthority(());
                yrs_engine::derived_state::PreparedCandidateValidation::prepare(
                    &authority,
                    expected_document,
                    &canonical_artifact,
                    validation,
                    schema,
                    resource_limits,
                    editing_limits,
                    max_length,
                    canonical_schema.schema_fingerprint(),
                    position_map,
                )
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(0),
                        "prepared command candidate validation could not be sealed",
                    )
                })
            })
            .transpose()?;
        Ok(Self::from_post_validation_construction(
            PreparedSemanticConstruction {
                request_id,
                document_revision,
                state_revision,
                yrs_state_epoch,
                schema_fingerprint: canonical_schema.schema_fingerprint().into(),
                transaction: transaction.clone(),
                expected_document: expected_document.clone(),
                canonical_artifact,
                candidate_validation,
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                undo_units,
                command_contract_oracle,
            },
        ))
    }

    pub(crate) fn finalize_from_validated_evidence(
        authority: &DeferredAdmissionAuthority,
        staged: &yrs_engine::prepared_admission::StagedDerivedStateAuthority<'_>,
        deferred: yrs_engine::prepared_admission::DeferredCommandAdmission,
        live: PreparedSemanticLiveContext<'_>,
    ) -> OperationResult<PreparedSemanticAdmission> {
        let parts = deferred.into_finalization_parts(authority, staged, live)?;
        let exact_candidate_len = parts
            .base_canonical_serialized_len
            .checked_add(parts.escaped_body_bytes)
            .filter(|exact| {
                *exact <= parts.candidate_output_upper_bound
                    && *exact <= parts.editing_limits.max_derived_output_bytes
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    parts.request_id,
                    Some(0),
                    "deferred insert exact output exceeded its conservative proof",
                )
            })?;
        let canonical_artifact = parts
            .candidate_canonical
            .seal_with_known_serialized_len(exact_candidate_len)
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    parts.request_id,
                    Some(0),
                    "deferred insert scalar caches diverged from its canonical projection",
                )
            })?;
        if canonical_artifact.serialized_len() != exact_candidate_len
            || !canonical_artifact.matches_exact_source_document(&parts.expected_document)
            || canonical_artifact.schema_fingerprint() != parts.schema_fingerprint.as_ref()
            || canonical_artifact.format_version()
                != yrs_engine::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !canonical_artifact
                .schema_context()
                .ptr_eq(&parts.canonical_schema)
        {
            return Err(OperationError::engine_invariant_failed(
                parts.request_id,
                Some(0),
                "deferred insert canonical projection evidence diverged",
            ));
        }
        let candidate_authority = CandidateValidationAuthority(());
        let candidate_validation = parts
            .candidate_evidence
            .finalize_deferred(
                &candidate_authority,
                &parts.expected_document,
                &canonical_artifact,
                parts.validation_report,
                &parts.resource_limits,
                &parts.editing_limits,
                parts.max_length,
                parts.schema_fingerprint.as_ref(),
                &parts.canonical_schema,
            )
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    parts.request_id,
                    Some(0),
                    "deferred insert saved validation evidence diverged",
                )
            })?;
        let prepared = Self::from_post_validation_construction(PreparedSemanticConstruction {
            request_id: parts.request_id,
            document_revision: parts.document_revision,
            state_revision: parts.state_revision,
            yrs_state_epoch: parts.yrs_state_epoch,
            schema_fingerprint: parts.schema_fingerprint,
            transaction: parts.transaction,
            expected_document: parts.expected_document,
            canonical_artifact,
            candidate_validation: Some(candidate_validation),
            resource_limits: parts.resource_limits,
            editing_limits: parts.editing_limits,
            max_length: parts.max_length,
            undo_units: parts.undo_units,
            command_contract_oracle: PreparedCommandContractOracle::None,
        });
        prepared.pre_admit_seed_independent(
            live.transaction,
            live.expected_preview,
            &prepared.editing_limits,
        )?;
        #[cfg(test)]
        yrs_engine::observability::record_deferred_capsule_finalized();
        Ok(prepared)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn admit(
        &self,
        transaction: &TypedTransaction,
        expected_preview: &Document,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        limits: &yrs_engine::EditingLimits,
        max_length: Option<u32>,
        canonical_schema: &CanonicalSchemaContext,
    ) -> OperationResult<()> {
        self.pre_admit_seed_independent(transaction, expected_preview, limits)?;
        if self.request_id != transaction.request_id
            || self.document_revision != transaction.base_document_revision
            || self.document_revision != document_revision
            || self.state_revision != state_revision
            || self.yrs_state_epoch != yrs_state_epoch
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || !self
                .canonical_artifact
                .matches_exact_source_document(expected_preview)
            || self.canonical_artifact.schema_fingerprint() != schema_fingerprint
            || self.canonical_artifact.format_version()
                != yrs_engine::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !self
                .canonical_artifact
                .schema_context()
                .ptr_eq(canonical_schema)
            || self.resource_limits != *resource_limits
            || self.max_length != max_length
            || self
                .candidate_validation
                .as_ref()
                .is_some_and(|validation| {
                    !validation.admits_context(
                        expected_preview,
                        &self.canonical_artifact,
                        resource_limits,
                        limits,
                        max_length,
                        schema_fingerprint,
                        canonical_schema,
                    )
                })
        {
            return Err(OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "prepared semantic admission does not match the live command context",
            ));
        }
        Ok(())
    }

    pub(crate) fn pre_admit_seed_independent(
        &self,
        transaction: &TypedTransaction,
        expected_preview: &Document,
        editing_limits: &EditingLimits,
    ) -> OperationResult<()> {
        if self.request_id != transaction.request_id
            || self.transaction != *transaction
            || !self.admits_expected_document(expected_preview)
            || self.editing_limits != *editing_limits
            || !self
                .command_contract_oracle
                .admits_transaction(transaction, self.candidate_validation.is_some())
        {
            return Err(OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "prepared semantic admission does not match the live command context",
            ));
        }
        if self.canonical_artifact.serialized_len() > editing_limits.max_derived_output_bytes {
            return Err(OperationError::document_limit_exceeded(
                transaction.request_id,
                Some(0),
                "maxDerivedOutputBytes",
                u64::try_from(editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(self.canonical_artifact.serialized_len()).unwrap_or(u64::MAX),
            ));
        }
        if self.undo_units > editing_limits.max_undo_retained_units {
            return Err(OperationError::operation_limit_exceeded(
                transaction.request_id,
                Some(0),
                "maxUndoRetainedUnits",
                editing_limits.max_undo_retained_units,
                self.undo_units,
            ));
        }
        Ok(())
    }

    pub(crate) fn is_identity_dependent_insert(&self) -> bool {
        matches!(
            self.transaction.operations.as_slice(),
            [TypedOperation::InsertText { .. }]
        )
    }

    pub(crate) fn transaction(&self) -> &TypedTransaction {
        &self.transaction
    }

    pub(crate) fn expected_document(&self) -> &Document {
        &self.expected_document
    }

    pub(crate) fn admits_expected_document(&self, document: &Document) -> bool {
        self.expected_document.shares_root_storage_with(document)
    }

    pub(crate) fn canonical_artifact(&self) -> &CanonicalArtifact {
        &self.canonical_artifact
    }

    pub(crate) fn undo_units(&self) -> u64 {
        self.undo_units
    }

    pub(crate) fn candidate_derivations(&self) -> Option<CompiledDocumentDerivations> {
        self.candidate_validation.as_ref()?.compiled_derivations(
            &self.expected_document,
            &self.canonical_artifact,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
            self.canonical_artifact.schema_context(),
        )
    }

    pub(super) fn candidate_validation(
        &self,
    ) -> Option<yrs_engine::derived_state::PreparedCandidateValidation> {
        self.candidate_validation.clone()
    }

    pub(super) fn candidate_validation_ref(
        &self,
    ) -> Option<&yrs_engine::derived_state::PreparedCandidateValidation> {
        self.candidate_validation.as_ref()
    }

    pub(super) fn command_contract_oracle(&self) -> PreparedCommandContractOracle {
        self.command_contract_oracle
    }

    #[cfg(test)]
    pub(crate) fn replace_candidate_artifact_for_test(
        &mut self,
        canonical_artifact: CanonicalArtifact,
    ) {
        self.candidate_validation
            .as_mut()
            .expect("test tamper requires prepared candidate validation")
            .replace_canonical_artifact_for_test(canonical_artifact);
    }

    #[cfg(test)]
    pub(crate) fn replace_canonical_artifact_for_test(
        &mut self,
        canonical_artifact: CanonicalArtifact,
    ) {
        self.canonical_artifact = canonical_artifact;
    }
}
