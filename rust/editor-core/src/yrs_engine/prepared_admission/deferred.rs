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
