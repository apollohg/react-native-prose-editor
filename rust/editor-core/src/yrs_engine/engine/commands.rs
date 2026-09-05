use super::history_state::{
    history_document_snapshots_fit, history_document_snapshots_fit_with_precomputed_after_charge,
    history_local_state, history_snapshot_template, history_snapshot_template_from_identity,
};
use super::outbound::OutboundUpdateSink;
use super::YrsDocumentEngine;
use crate::model::Document;
use crate::selection::Selection;
use crate::yrs_engine;
use crate::yrs_engine::compiler::{
    selectable_void_at, CompiledTransaction, PreparedSemanticAdmission, SelectionPlan,
};
use crate::yrs_engine::derived_state::DerivedStateCache;
use std::sync::Arc;

impl YrsDocumentEngine {
    pub fn plan_command(
        &self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
    ) -> yrs_engine::OperationResult<yrs_engine::CommandPlan> {
        self.plan_command_internal(request_id, command, None)
    }

    pub(super) fn plan_command_internal<'a>(
        &'a self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
        preparation: Option<
            &'a std::cell::RefCell<Option<yrs_engine::commands::PreparedCommandProof>>,
        >,
    ) -> yrs_engine::OperationResult<yrs_engine::CommandPlan> {
        self.plan_command_internal_at_selection(
            request_id,
            command,
            preparation,
            None,
            None,
            yrs_engine::TransactionOrigin::LocalCommand,
        )
    }

    fn plan_command_internal_at_selection<'a>(
        &'a self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
        preparation: Option<
            &'a std::cell::RefCell<Option<yrs_engine::commands::PreparedCommandProof>>,
        >,
        selection: Option<&'a yrs_engine::ResolvedSelection>,
        initial_selection: Option<&'a yrs_engine::SelectionInput>,
        origin: yrs_engine::TransactionOrigin,
    ) -> yrs_engine::OperationResult<yrs_engine::CommandPlan> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
        let allow_deferred_admission =
            preparation.is_some() && state.mutation_lookup_seed.is_unavailable() && {
                let canonical_fingerprint = state.canonical_artifact.sha256();
                let canonical_serialized_len = state.canonical_artifact.serialized_len();
                state.matches_materialized_mutation_identity(
                    &state.canonical_artifact,
                    canonical_fingerprint,
                    canonical_serialized_len,
                    &self.resource_limits,
                    &self.schema_fingerprint,
                    self.revision,
                    self.state_revision,
                    self.yrs_state_epoch,
                )
            };
        yrs_engine::commands::plan(
            yrs_engine::commands::PlanningContext {
                request_id,
                revision: self.revision,
                state_revision: self.state_revision,
                document: &state.document,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                selection: selection.unwrap_or(&state.resolved_selection),
                initial_selection,
                origin,
                stored_marks: state.stored_marks.as_deref(),
                schema: &self.schema,
                resource_limits: &self.resource_limits,
                editing_limits: &self.editing_limits,
                max_length: self.max_length,
                yrs_state_epoch: self.yrs_state_epoch,
                canonical_schema: &self.canonical_schema,
                canonical_artifact: &state.canonical_artifact,
                allow_deferred_admission,
                preparation,
            },
            command,
        )
    }

    #[allow(dead_code)]
    pub fn apply_command(
        &mut self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TypedTransactionResult>> {
        self.apply_command_with_sink(request_id, command, &mut OutboundUpdateSink::detached())
    }

    /// Production surface: [`Self::apply_command`] with an optionally attached
    /// collaboration outbox for outbound update capture.
    pub(crate) fn apply_command_with_outbox(
        &mut self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TypedTransactionResult>> {
        self.apply_command_with_sink(
            request_id,
            command,
            &mut OutboundUpdateSink::from_optional_outbox(outbox),
        )
    }

    pub(crate) fn apply_command_at_selection_with_outbox(
        &mut self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
        selection: yrs_engine::SelectionInput,
        origin: yrs_engine::TransactionOrigin,
        outbox: Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TypedTransactionResult>> {
        let resolved = self.resolve_selection_input_for_planning(request_id, &selection)?;
        let preparation = std::cell::RefCell::new(None);
        let mut outbound = OutboundUpdateSink::from_optional_outbox(outbox);
        let (_, result) = match self.plan_command_internal_at_selection(
            request_id,
            command,
            Some(&preparation),
            Some(&resolved),
            Some(&selection),
            origin,
        )? {
            yrs_engine::CommandPlan::NotApplicable => return Ok(None),
            yrs_engine::CommandPlan::SelectionOnly(transaction) => {
                let compiled = self.compile_typed_transaction(transaction)?;
                self.apply_compiled_transaction_with_history(compiled, true, None, &mut outbound)?
            }
            yrs_engine::CommandPlan::Transaction(transaction) => {
                if let Some(proof) = preparation.into_inner() {
                    self.apply_prepared_command_transaction(
                        transaction,
                        proof,
                        true,
                        &mut outbound,
                    )?
                } else {
                    self.apply_typed_transaction_with_staged_context(
                        transaction,
                        true,
                        &mut outbound,
                    )?
                }
            }
        };
        result.map(Some).ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "anchored command produced no result envelope",
            )
        })
    }

    fn resolve_selection_input_for_planning(
        &self,
        request_id: u64,
        selection: &yrs_engine::SelectionInput,
    ) -> yrs_engine::OperationResult<yrs_engine::ResolvedSelection> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
        let resolve = |field: &'static str,
                       point: yrs_engine::RevisionedPosition|
         -> yrs_engine::OperationResult<u32> {
            yrs_engine::position::editor_offset_to_doc_pos(
                point.offset,
                point.kind,
                &state.rendered_text,
                &state.position_map,
                &state.document,
            )
            .ok_or_else(|| {
                yrs_engine::OperationError::selection_position_invalid(
                    request_id,
                    field,
                    format!("{field} is outside the current document"),
                )
            })
        };
        let resolved_point =
            |document: u32| -> yrs_engine::OperationResult<yrs_engine::ResolvedPoint> {
                let scalar = state.position_map.doc_to_scalar(document, &state.document);
                let utf16 =
                    yrs_engine::position::scalar_offset_to_utf16(&state.rendered_text, scalar)
                        .ok_or_else(|| {
                            yrs_engine::OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "resolved selection is not representable as UTF-16",
                            )
                        })?;
                Ok(yrs_engine::ResolvedPoint {
                    document,
                    scalar,
                    utf16,
                })
            };
        match selection {
            yrs_engine::SelectionInput::Text { anchor, head } => {
                let anchor = resolve("selection.anchor", *anchor)?;
                let head = resolve("selection.head", *head)?;
                let normalized =
                    Selection::text(anchor, head).normalized(&state.document, &state.position_map);
                let Selection::Text { anchor, head } = normalized else {
                    return Err(yrs_engine::OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "text selection normalized to a non-text selection",
                    ));
                };
                Ok(yrs_engine::ResolvedSelection::Text {
                    anchor: resolved_point(anchor)?,
                    head: resolved_point(head)?,
                })
            }
            yrs_engine::SelectionInput::Node { at } => {
                let at = resolve("selection.at", *at)?;
                let Selection::Node { pos } =
                    Selection::node(at).normalized(&state.document, &state.position_map)
                else {
                    return Err(yrs_engine::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection did not resolve to a selectable node",
                    ));
                };
                if !selectable_void_at(state.document.root(), pos, 0, &self.schema) {
                    return Err(yrs_engine::OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                Ok(yrs_engine::ResolvedSelection::Node {
                    at: resolved_point(pos)?,
                })
            }
            yrs_engine::SelectionInput::All => Ok(yrs_engine::ResolvedSelection::All),
        }
    }

    fn apply_command_with_sink(
        &mut self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
        outbound: &mut OutboundUpdateSink<'_>,
    ) -> yrs_engine::OperationResult<Option<yrs_engine::TypedTransactionResult>> {
        let preparation = std::cell::RefCell::new(None);
        let (_, result) =
            match self.plan_command_internal(request_id, command, Some(&preparation))? {
                yrs_engine::CommandPlan::NotApplicable => return Ok(None),
                yrs_engine::CommandPlan::SelectionOnly(transaction) => {
                    let compiled = self.compile_typed_transaction(transaction)?;
                    self.apply_compiled_transaction_with_history(compiled, true, None, outbound)?
                }
                yrs_engine::CommandPlan::Transaction(transaction) => {
                    if let Some(proof) = preparation.into_inner() {
                        self.apply_prepared_command_transaction(transaction, proof, true, outbound)?
                    } else {
                        self.apply_typed_transaction_with_staged_context(
                            transaction,
                            true,
                            outbound,
                        )?
                    }
                }
            };
        result.map(Some).ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                request_id,
                None,
                "rich prepared command produced no result envelope",
            )
        })
    }

    fn prepare_command_history_admission(
        &self,
        semantic: &PreparedSemanticAdmission,
    ) -> yrs_engine::OperationResult<
        Option<yrs_engine::prepared_admission::PreparedCommandHistoryAdmission>,
    > {
        let transaction = semantic.transaction();
        let expected_document = semantic.expected_document();
        if transaction.history_policy == yrs_engine::HistoryPolicy::Skip
            || expected_document
                == self.document().ok_or_else(|| {
                    yrs_engine::OperationError::engine_not_ready(transaction.request_id)
                })?
        {
            return Ok(None);
        }
        let class = transaction.operations.iter().fold(
            yrs_engine::compiler::HistoryClass::Skip,
            |class, operation| {
                use yrs_engine::compiler::HistoryClass;
                let next = match operation {
                    yrs_engine::TypedOperation::InsertText { .. }
                    | yrs_engine::TypedOperation::InsertNode { .. } => HistoryClass::Insert,
                    yrs_engine::TypedOperation::DeleteRange { .. } => HistoryClass::Delete,
                    yrs_engine::TypedOperation::AddMark { .. }
                    | yrs_engine::TypedOperation::RemoveMark { .. }
                    | yrs_engine::TypedOperation::ReplaceMark { .. }
                    | yrs_engine::TypedOperation::UpdateNodeAttrs { .. } => HistoryClass::Format,
                    _ => HistoryClass::Structural,
                };
                match (class, next) {
                    (HistoryClass::Skip, value) => value,
                    (left, right) if left == right => left,
                    _ => HistoryClass::Structural,
                }
            },
        );
        if class == yrs_engine::compiler::HistoryClass::Skip {
            return Ok(None);
        }
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(transaction.request_id))?;
        let candidate_artifact = semantic.canonical_artifact();
        let candidate_derivations = if let Some(derivations) = semantic.candidate_derivations() {
            derivations
        } else {
            let mut position_map =
                crate::position::PositionMap::build(expected_document, &self.schema);
            position_map.compact();
            let rendered_text = crate::render::rendered_text(expected_document, &self.schema);
            let rendered_scalars = u32::try_from(rendered_text.chars().count()).map_err(|_| {
                yrs_engine::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "prepared history rendered text exceeds the position domain",
                )
            })?;
            let mut document_text_bytes = 0usize;
            let mut stack = vec![expected_document.root()];
            while let Some(node) = stack.pop() {
                if let Some(text) = node.text_str() {
                    document_text_bytes =
                        document_text_bytes.checked_add(text.len()).ok_or_else(|| {
                            yrs_engine::OperationError::engine_invariant_failed(
                                transaction.request_id,
                                None,
                                "prepared history text byte metric overflowed",
                            )
                        })?;
                }
                if let Some(content) = node.content() {
                    stack.extend(content.iter());
                }
            }
            yrs_engine::compiler::CompiledDocumentDerivations {
                identity_seal: Arc::new(()),
                position_map,
                rendered_text,
                rendered_scalars,
                document_text_bytes,
                document_node_count: crate::editor_state::document_node_count(
                    expected_document.root(),
                ),
            }
        };
        let candidate_render = state
            .render_blocks
            .transition(
                &state.document,
                expected_document,
                &self.schema,
                &[],
                &self.resource_limits,
            )
            .map_err(|error| {
                yrs_engine::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    format!("prepared history render transition failed: {error:?}"),
                )
            })?;
        let retained = history_document_snapshots_fit(
            state,
            expected_document,
            candidate_artifact,
            &candidate_derivations,
            &candidate_render.cache,
            state.stored_marks.as_deref(),
            &self.schema_fingerprint,
            &self.fragment_name,
            self.scope.as_ref(),
            self.editing_limits.max_derived_output_bytes,
        );
        let before = history_local_state(
            state,
            &self.fragment_name,
            self.scope.as_ref(),
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            retained.map(|pair| pair.before),
        );
        let after = history_snapshot_template(
            candidate_artifact,
            state.stored_marks.as_deref(),
            &self.fragment_name,
            retained.map(|pair| pair.after),
        );
        let limits = self.history.pre_admit_capture_limits(
            transaction.request_id,
            transaction.origin,
            transaction.history_policy,
            class,
            semantic.undo_units(),
            before.metadata_bytes,
            after.metadata_bytes,
        )?;
        Ok(Some(
            yrs_engine::prepared_admission::PreparedCommandHistoryAdmission {
                limits,
                before,
                after,
                candidate_derivations,
                candidate_render,
            },
        ))
    }

    pub(super) fn prepare_execution_command_history_admission(
        &self,
        semantic: &yrs_engine::prepared_admission::ExecutionSemanticAdmission,
    ) -> yrs_engine::OperationResult<
        Option<yrs_engine::prepared_admission::PreparedCommandHistoryAdmission>,
    > {
        match semantic {
            yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(admission) => {
                self.prepare_command_history_admission(admission)
            }
            yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(admission) => {
                self.prepare_deferred_command_history_admission(admission)
            }
        }
    }

    fn prepare_deferred_command_history_admission(
        &self,
        deferred: &yrs_engine::prepared_admission::DeferredCommandAdmission,
    ) -> yrs_engine::OperationResult<
        Option<yrs_engine::prepared_admission::PreparedCommandHistoryAdmission>,
    > {
        let transaction = deferred.transaction();
        let expected_document = deferred.expected_document();
        if transaction.history_policy == yrs_engine::HistoryPolicy::Skip
            || expected_document
                == self.document().ok_or_else(|| {
                    yrs_engine::OperationError::engine_not_ready(transaction.request_id)
                })?
        {
            return Ok(None);
        }
        let class = transaction.operations.iter().fold(
            yrs_engine::compiler::HistoryClass::Skip,
            |class, operation| {
                use yrs_engine::compiler::HistoryClass;
                let next = match operation {
                    yrs_engine::TypedOperation::InsertText { .. }
                    | yrs_engine::TypedOperation::InsertNode { .. } => HistoryClass::Insert,
                    yrs_engine::TypedOperation::DeleteRange { .. } => HistoryClass::Delete,
                    yrs_engine::TypedOperation::AddMark { .. }
                    | yrs_engine::TypedOperation::RemoveMark { .. }
                    | yrs_engine::TypedOperation::ReplaceMark { .. }
                    | yrs_engine::TypedOperation::UpdateNodeAttrs { .. } => HistoryClass::Format,
                    _ => HistoryClass::Structural,
                };
                match (class, next) {
                    (HistoryClass::Skip, value) => value,
                    (left, right) if left == right => left,
                    _ => HistoryClass::Structural,
                }
            },
        );
        if class == yrs_engine::compiler::HistoryClass::Skip {
            return Ok(None);
        }
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(transaction.request_id))?;
        let evidence = deferred.prepare_history_evidence()?;
        let generic_render = || {
            state.render_blocks.transition(
                &state.document,
                expected_document,
                &self.schema,
                &[],
                &self.resource_limits,
            )
        };
        crate::render::incremental::record_localized_render_transition_attempt();
        let specialized = deferred.prepare_history_render_transition(
            state,
            &evidence.candidate_derivations,
            &self.schema,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
        );
        let candidate_render = match specialized {
            Some(Ok(transition)) => {
                crate::render::incremental::record_localized_render_transition_success();
                Ok(transition)
            }
            Some(Err(_)) | None => {
                crate::render::incremental::record_localized_render_transition_fallback();
                generic_render()
            }
        }
        .map_err(|error| {
            yrs_engine::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                format!("prepared history render transition failed: {error:?}"),
            )
        })?;
        let retained = history_document_snapshots_fit_with_precomputed_after_charge(
            state,
            evidence.canonical_retained_bytes,
            evidence.source_document_retained_bytes,
            &evidence.candidate_derivations,
            &candidate_render.cache,
            state.stored_marks.as_deref(),
            &self.schema_fingerprint,
            &self.fragment_name,
            self.scope.as_ref(),
            self.editing_limits.max_derived_output_bytes,
        );
        let before = history_local_state(
            state,
            &self.fragment_name,
            self.scope.as_ref(),
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            retained.map(|pair| pair.before),
        );
        let after = history_snapshot_template_from_identity(
            evidence.canonical_text_scalar_len,
            evidence.canonical_fingerprint,
            evidence.canonical_serialized_len,
            state.stored_marks.as_deref(),
            &self.fragment_name,
            retained.map(|pair| pair.after),
        );
        let limits = self.history.pre_admit_capture_limits(
            transaction.request_id,
            transaction.origin,
            transaction.history_policy,
            class,
            deferred.undo_units(),
            before.metadata_bytes,
            after.metadata_bytes,
        )?;
        Ok(Some(
            yrs_engine::prepared_admission::PreparedCommandHistoryAdmission {
                limits,
                before,
                after,
                candidate_derivations: evidence.candidate_derivations,
                candidate_render,
            },
        ))
    }

    pub(super) fn compiled_command_matches_proof(
        &self,
        compiled: &CompiledTransaction,
        document: &Document,
        selection: &Selection,
    ) -> yrs_engine::OperationResult<bool> {
        let current_selection = self
            .derived_state
            .as_ref()
            .map(DerivedStateCache::legacy_selection)
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(compiled.request_id))?;
        let compiled_selection = match &compiled.selection_plan {
            SelectionPlan::Preserve => current_selection,
            SelectionPlan::Mapped(selection) | SelectionPlan::Explicit(selection) => {
                selection.clone()
            }
        };
        Ok(compiled.preview == *document && compiled_selection == *selection)
    }

    /// Production probe: the conservative outbound bound one planned command
    /// would reserve; `None` when the command is not applicable or lowers to
    /// a selection-only plan (which reserves nothing).
    #[allow(dead_code)]
    pub(crate) fn probe_command_outbound_upper_bound(
        &self,
        request_id: u64,
        command: yrs_engine::TypedCommand,
    ) -> yrs_engine::OperationResult<Option<usize>> {
        match self.plan_command(request_id, command)? {
            yrs_engine::CommandPlan::NotApplicable | yrs_engine::CommandPlan::SelectionOnly(_) => {
                Ok(None)
            }
            yrs_engine::CommandPlan::Transaction(transaction) => self
                .probe_transaction_outbound_upper_bound(transaction)
                .map(Some),
        }
    }
}
