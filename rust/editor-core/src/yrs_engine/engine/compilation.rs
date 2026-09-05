use super::imports::{RootBoundValidationReport, ValidatedImportDocument};
#[cfg(test)]
use super::test_hooks::{
    record_compiled_commit_authority_validation_for_test, record_compiled_commit_live_view_for_test,
};
use super::transaction_result::affinity_aware_mapped_selection;
use super::YrsDocumentEngine;
use crate::model::Document;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::DocumentValidator;
use crate::yrs_engine;
use crate::yrs_engine::compiler::{
    compile_prepared_transaction_with_yrs_and_stored_marks,
    compile_transaction_with_yrs_and_stored_marks, CompilationContext, CompiledTransaction,
    EngineCompilationView, PreparedSemanticAdmission, PreparedSemanticContext,
    RelativeSelectionPlan, SelectionPlan, StoredMarksCompilationContext, StoredMarksPlan,
};
use crate::yrs_engine::derived_state::{exact_point_is_representable, FinalizedSelectionState};
use crate::yrs_engine::mutation::{YrsMutationAction, YrsMutationPlan};
use yrs::types::xml::XmlFragmentRef;
use yrs::{Assoc, IndexedSequence, ReadTxn, Transact};

fn selection_requires_fallback_proof<T: ReadTxn>(
    plan: &YrsMutationPlan,
    txn: &T,
    fragment: &XmlFragmentRef,
    selection: &yrs_engine::RelativeSelection,
) -> bool {
    match selection {
        yrs_engine::RelativeSelection::Text { anchor, head } => {
            plan.removes_sticky_branch(txn, fragment, &anchor.sticky)
                || plan.removes_sticky_branch(txn, fragment, &head.sticky)
        }
        yrs_engine::RelativeSelection::Node { point } => {
            plan.removes_sticky_branch(txn, fragment, &point.sticky)
        }
        yrs_engine::RelativeSelection::All => false,
    }
}

struct FallbackProofContext<'a, Current, Proof> {
    plan: &'a YrsMutationPlan,
    current_txn: &'a Current,
    current_fragment: &'a XmlFragmentRef,
    proof_txn: &'a Proof,
    proof_fragment: &'a XmlFragmentRef,
    schema: &'a Schema,
}

fn required_fallbacks_are_representable<Current: ReadTxn, Proof: ReadTxn>(
    context: FallbackProofContext<'_, Current, Proof>,
    selection: &Selection,
    relative: &yrs_engine::RelativeSelection,
) -> bool {
    let FallbackProofContext {
        plan,
        current_txn,
        current_fragment,
        proof_txn,
        proof_fragment,
        schema,
    } = context;
    let point_is_valid = |position, point: &yrs_engine::RelativePoint| {
        !plan.removes_sticky_branch(current_txn, current_fragment, &point.sticky)
            || exact_point_is_representable(proof_txn, proof_fragment, position, point, schema)
    };
    match (selection, relative) {
        (
            Selection::Text { anchor, head },
            yrs_engine::RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            },
        ) => point_is_valid(*anchor, relative_anchor) && point_is_valid(*head, relative_head),
        (Selection::Node { pos }, yrs_engine::RelativeSelection::Node { point }) => {
            point_is_valid(*pos, point)
        }
        (Selection::All, yrs_engine::RelativeSelection::All) => true,
        _ => false,
    }
}

impl YrsDocumentEngine {
    #[allow(dead_code)] // exposes the internal compiler through atomic application.
    pub(crate) fn compile_typed_transaction(
        &self,
        transaction: yrs_engine::TypedTransaction,
    ) -> yrs_engine::OperationResult<CompiledTransaction> {
        self.compile_typed_transaction_internal(transaction, None)
    }

    #[cfg(test)]
    fn compile_finalized_prepared_typed_transaction(
        &self,
        transaction: yrs_engine::TypedTransaction,
        semantic_admission: &PreparedSemanticAdmission,
        proof_document: &Document,
        proof_selection: &Selection,
        candidate_derivations: Option<&yrs_engine::compiler::CompiledDocumentDerivations>,
    ) -> yrs_engine::OperationResult<CompiledTransaction> {
        let state = self.derived_state.as_ref().ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "ready Yrs engine has no derived state",
            )
        })?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let authority = yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
        self.compile_finalized_prepared_typed_transaction_with_read_view(
            transaction,
            semantic_admission,
            proof_document,
            proof_selection,
            candidate_derivations,
            &authority,
            &txn,
            &fragment,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn compile_finalized_prepared_typed_transaction_with_read_view<T: ReadTxn>(
        &self,
        transaction: yrs_engine::TypedTransaction,
        semantic_admission: &PreparedSemanticAdmission,
        proof_document: &Document,
        proof_selection: &Selection,
        candidate_derivations: Option<&yrs_engine::compiler::CompiledDocumentDerivations>,
        authority: &dyn yrs_engine::prepared_admission::DerivedStateAuthority,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> yrs_engine::OperationResult<CompiledTransaction> {
        let mut compiled = self.compile_typed_transaction_with_read_view(
            transaction,
            Some((semantic_admission, proof_document)),
            authority,
            txn,
            fragment,
        )?;
        if compiled.preview != *proof_document {
            return Err(yrs_engine::OperationError::engine_invariant_failed(
                compiled.request_id,
                None,
                "prepared command compiler diverged from its simulated document",
            ));
        }
        if let Some(derivations) = candidate_derivations {
            compiled.preview = proof_document.clone();
            compiled.preview_derivations = Some(derivations.clone());
        }
        let state = authority.installed();
        let eligible_admission = compiled
            .localized_insert_admission
            .as_ref()
            .filter(|admission| {
                let current_at_insertion = matches!(
                    &state.resolved_selection,
                    yrs_engine::ResolvedSelection::Text { anchor, head }
                        if anchor == head
                            && anchor.document == admission.inserted_document_position()
                );
                let operation_result = admission.operation_result_selection();
                let operation_result_legacy =
                    yrs_engine::derived_state::resolved_to_legacy(operation_result);
                compiled.origin == yrs_engine::TransactionOrigin::LocalCommand
                    && compiled.history_policy == yrs_engine::HistoryPolicy::Boundary
                    && compiled.history_class == yrs_engine::compiler::HistoryClass::Insert
                    && compiled.localized_semantic_used
                    && admission.inserted_scalars() > 0
                    && current_at_insertion
                    && matches!(
                        &compiled.selection_plan,
                        SelectionPlan::Explicit(selection)
                            if *selection == operation_result_legacy
                                && *selection == *proof_selection
                    )
                    && compiled.relative_selection_plan == RelativeSelectionPlan::OperationResult
                    && matches!(
                        &compiled.stored_marks_plan,
                        StoredMarksPlan::Set(stored_marks)
                            if *stored_marks == state.stored_marks
                    )
                    && compiled.preview == *proof_document
                    && *operation_result
                        == yrs_engine::derived_state::resolved_from_legacy_with_view(
                            &compiled.preview,
                            &operation_result_legacy,
                            &self.schema,
                            compiled
                                .preview_derivations
                                .as_ref()
                                .map(|derivations| &derivations.position_map)
                                .unwrap_or(&state.position_map),
                            compiled
                                .preview_derivations
                                .as_ref()
                                .map(|derivations| derivations.rendered_text.as_str())
                                .unwrap_or(state.rendered_text.as_str()),
                        )
                        .unwrap_or(yrs_engine::ResolvedSelection::All)
            });
        let transition = eligible_admission
            .map(|admission| {
                let StoredMarksPlan::Set(stored_marks) = &compiled.stored_marks_plan else {
                    unreachable!("eligible active-state transition has sealed marks")
                };
                state.prepare_active_state_transition(
                    compiled.request_id,
                    authority,
                    admission,
                    &compiled.preview,
                    admission.operation_result_selection(),
                    stored_marks.as_deref(),
                    &self.resource_limits,
                    &self.editing_limits,
                    self.max_length,
                    self.yrs_state_epoch,
                )
            })
            .transpose()?;
        compiled.prepared_selection_state = eligible_admission.and_then(|admission| {
            yrs_engine::derived_state::record_prewrite_selection_proof_attempt();
            let prepared = self.materialize_prewrite_selection_state(&compiled, admission, txn);
            if prepared.is_some() {
                yrs_engine::derived_state::record_prewrite_selection_proof_finalization();
            } else {
                yrs_engine::derived_state::record_prewrite_selection_proof_fallback();
            }
            prepared
        });
        compiled.prepared_selection_mutation_seal = compiled
            .prepared_selection_state
            .as_ref()
            .and_then(|_| yrs_engine::compiler::PreparedSelectionMutationSeal::capture(&compiled));
        compiled.prepared_active_state_transition = transition;
        Ok(compiled)
    }

    #[cfg(test)]
    pub(super) fn compile_prepared_typed_transaction(
        &self,
        transaction: yrs_engine::TypedTransaction,
        proof: yrs_engine::commands::PreparedCommandProof,
    ) -> yrs_engine::OperationResult<CompiledTransaction> {
        let yrs_engine::commands::PreparedCommandProof {
            document,
            selection,
            execution_admission,
        } = proof;
        let semantic_admission = match execution_admission {
            yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(admission) => {
                admission
            }
            yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(admission) => {
                admission.into_eager()?
            }
        };
        self.compile_finalized_prepared_typed_transaction(
            transaction,
            &semantic_admission,
            &document,
            &selection,
            None,
        )
    }

    pub(super) fn materialize_prewrite_selection_state<T: ReadTxn>(
        &self,
        compiled: &CompiledTransaction,
        admission: &yrs_engine::derived_state::LocalizedInsertAdmission,
        txn: &T,
    ) -> Option<FinalizedSelectionState> {
        let state = self.derived_state.as_ref()?;
        let current_at_insertion = matches!(
            &state.resolved_selection,
            yrs_engine::ResolvedSelection::Text { anchor, head }
                if anchor == head
                    && anchor.document == admission.inserted_document_position()
        );
        if compiled.origin != yrs_engine::TransactionOrigin::LocalCommand
            || compiled.history_policy != yrs_engine::HistoryPolicy::Boundary
            || compiled.history_class != yrs_engine::compiler::HistoryClass::Insert
            || !compiled.localized_semantic_used
            || admission.inserted_scalars() == 0
            || !current_at_insertion
            || compiled.base_state_revision != state.state_revision
            || compiled.yrs_state_epoch != self.yrs_state_epoch
            || !matches!(
                &compiled.stored_marks_plan,
                StoredMarksPlan::Set(stored_marks) if *stored_marks == state.stored_marks
            )
        {
            return None;
        }
        let [YrsMutationAction::InsertText {
            target,
            index_utf16,
            len_utf16,
            signature,
            operation_index,
            ..
        }] = compiled.mutation_plan.actions.as_slice()
        else {
            return None;
        };
        if *operation_index != 0
            || *len_utf16 == 0
            || *len_utf16 != admission.inserted_utf16()
            || *index_utf16 == 0
            || *index_utf16 >= signature.initial_len_utf16()
        {
            return None;
        }
        let sticky = target.sticky_index(txn, *index_utf16, Assoc::After)?;
        let offset = sticky.get_offset(txn)?;
        let exact_target = yrs::branch::BranchPtr::from(<yrs::types::xml::XmlTextRef as AsRef<
            yrs::branch::Branch,
        >>::as_ref(target));
        if offset.index != *index_utf16 || offset.branch != exact_target {
            return None;
        }
        let point = yrs_engine::RelativePoint {
            sticky,
            affinity: yrs_engine::Affinity::After,
        };
        let relative = yrs_engine::RelativeSelection::Text {
            anchor: point.clone(),
            head: point,
        };
        let resolved = admission.operation_result_selection().clone();
        let legacy = yrs_engine::derived_state::resolved_to_legacy(&resolved);
        if !matches!(
            &compiled.selection_plan,
            SelectionPlan::Explicit(selection) if *selection == legacy
        ) || compiled.relative_selection_plan != RelativeSelectionPlan::OperationResult
        {
            return None;
        }
        let preview_derivations = compiled.preview_derivations.as_ref()?;
        if resolved
            != yrs_engine::derived_state::resolved_from_legacy_with_view(
                &compiled.preview,
                &legacy,
                &self.schema,
                &preview_derivations.position_map,
                &preview_derivations.rendered_text,
            )?
        {
            return None;
        }
        FinalizedSelectionState::new(relative, resolved, legacy)
    }

    fn compile_typed_transaction_internal(
        &self,
        transaction: yrs_engine::TypedTransaction,
        prepared_semantics: Option<(&PreparedSemanticAdmission, &Document)>,
    ) -> yrs_engine::OperationResult<CompiledTransaction> {
        let state = self.derived_state.as_ref().ok_or_else(|| {
            yrs_engine::OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "ready Yrs engine has no derived state",
            )
        })?;
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    transaction.request_id,
                    None,
                    "ready Yrs document fragment is missing",
                )
            })?;
        let installed_authority =
            yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
        self.compile_typed_transaction_with_read_view(
            transaction,
            prepared_semantics,
            &installed_authority,
            &txn,
            &fragment,
        )
    }

    pub(super) fn compile_typed_transaction_with_read_view<T: ReadTxn>(
        &self,
        transaction: yrs_engine::TypedTransaction,
        prepared_semantics: Option<(&PreparedSemanticAdmission, &Document)>,
        authority: &dyn yrs_engine::prepared_admission::DerivedStateAuthority,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> yrs_engine::OperationResult<CompiledTransaction> {
        let state = authority.installed();
        let cached = state.compilation_view();
        let document = cached.document;
        let current_selection = cached.selection;
        let current_relative_selection = self.relative_selection().cloned();
        let compilation_context = CompilationContext {
            document,
            selection: Some(current_selection),
            schema: &self.schema,
            resource_limits: &self.resource_limits,
            editing_limits: &self.editing_limits,
            document_revision: self.revision,
            max_length: self.max_length,
        };
        let stored_marks_context = StoredMarksCompilationContext {
            stored_marks: state.stored_marks.as_deref(),
            resolved_selection: &state.resolved_selection,
            relative_selection: &state.relative_selection,
        };
        let engine_view = EngineCompilationView {
            cached,
            authority,
            state_revision: self.state_revision,
            schema_fingerprint: &self.schema_fingerprint,
            yrs_state_epoch: self.yrs_state_epoch,
        };
        let mut compiled = if let Some((semantic_admission, expected_preview)) = prepared_semantics
        {
            compile_prepared_transaction_with_yrs_and_stored_marks(
                compilation_context,
                transaction,
                txn,
                fragment,
                stored_marks_context,
                PreparedSemanticContext {
                    admission: semantic_admission,
                    expected_preview,
                    yrs_state_epoch: self.yrs_state_epoch,
                    state_revision: self.state_revision,
                    schema_fingerprint: &self.schema_fingerprint,
                },
                engine_view,
            )?
        } else {
            compile_transaction_with_yrs_and_stored_marks(
                compilation_context,
                transaction,
                txn,
                fragment,
                stored_marks_context,
                engine_view,
            )?
        };
        if let (
            Some(selection),
            Some(relative),
            SelectionPlan::Mapped(_),
            RelativeSelectionPlan::PreserveWithFallback(fallback),
        ) = (
            Some(current_selection),
            current_relative_selection.as_ref(),
            &compiled.selection_plan,
            &mut compiled.relative_selection_plan,
        ) {
            *fallback = affinity_aware_mapped_selection(
                selection,
                relative,
                &compiled.composed_map,
                &compiled.preview,
                &self.schema,
                compiled
                    .preview_derivations
                    .as_ref()
                    .map(|derivations| &derivations.position_map),
            );
        }
        if let RelativeSelectionPlan::Precomputed { relative, fallback } =
            &compiled.relative_selection_plan
        {
            if compiled.preview != *document
                && selection_requires_fallback_proof(
                    &compiled.mutation_plan,
                    txn,
                    fragment,
                    relative,
                )
            {
                let proof_source = ValidatedImportDocument {
                    document: compiled.preview.clone(),
                    canonical_artifact: compiled.canonical_artifact.as_ref().cloned().ok_or_else(
                        || {
                            yrs_engine::OperationError::engine_invariant_failed(
                                compiled.request_id,
                                None,
                                "changed explicit selection preview has no canonical JSON",
                            )
                        },
                    )?,
                    validation: RootBoundValidationReport {
                        source_root: compiled.preview.root().clone(),
                        report: DocumentValidator::validate_report(
                            &compiled.preview,
                            &self.schema,
                            &self.resource_limits,
                        )
                        .map_err(|error| {
                            yrs_engine::OperationError::engine_invariant_failed(
                                compiled.request_id,
                                None,
                                format!("selection proof preview is invalid: {error}"),
                            )
                        })?,
                    },
                    carry_import_encoded_state_receipt: false,
                };
                let proof = self
                    .build_candidate_from_document(proof_source, compiled.origin)
                    .map_err(|error| {
                        yrs_engine::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            format!("cannot prove committed selection representation: {error}"),
                        )
                    })?;
                let proof_txn = proof.doc.transact();
                let proof_fragment = proof_txn
                    .get_xml_fragment(self.fragment_name.as_str())
                    .ok_or_else(|| {
                        yrs_engine::OperationError::engine_invariant_failed(
                            compiled.request_id,
                            None,
                            "selection proof candidate fragment is missing",
                        )
                    })?;
                if !required_fallbacks_are_representable(
                    FallbackProofContext {
                        plan: &compiled.mutation_plan,
                        current_txn: txn,
                        current_fragment: fragment,
                        proof_txn: &proof_txn,
                        proof_fragment: &proof_fragment,
                        schema: &self.schema,
                    },
                    fallback,
                    relative,
                ) {
                    return Err(yrs_engine::OperationError::selection_position_invalid(
                        compiled.request_id,
                        "selection",
                        "mapped selection cannot preserve the requested Yrs affinity",
                    ));
                }
            }
        }
        compiled.yrs_state_epoch = self.yrs_state_epoch;
        Ok(compiled)
    }

    pub(super) fn with_compiled_base_authority<R>(
        &self,
        request_id: u64,
        context: Option<&yrs_engine::prepared_admission::PreparedMutationContext>,
        use_authority: impl FnOnce(
            &dyn yrs_engine::prepared_admission::DerivedStateAuthority,
            &yrs::Transaction<'_>,
            &XmlFragmentRef,
        ) -> yrs_engine::OperationResult<R>,
    ) -> yrs_engine::OperationResult<R> {
        let state = self
            .derived_state
            .as_ref()
            .ok_or_else(|| yrs_engine::OperationError::engine_not_ready(request_id))?;
        #[cfg(test)]
        record_compiled_commit_live_view_for_test();
        let txn = self.doc.transact();
        let fragment = txn
            .get_xml_fragment(self.fragment_name.as_str())
            .ok_or_else(|| {
                yrs_engine::OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "compiled transaction lost its live Yrs fragment",
                )
            })?;
        if let Some(context) = context {
            #[cfg(test)]
            record_compiled_commit_authority_validation_for_test();
            let authority = context.authority(
                yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                    request_id,
                    installed: state,
                    txn: &txn,
                    fragment: &fragment,
                    fragment_name: &self.fragment_name,
                    schema_fingerprint: &self.schema_fingerprint,
                    resource_limits: &self.resource_limits,
                    editing_limits: &self.editing_limits,
                    max_length: self.max_length,
                    document_revision: self.revision,
                    state_revision: self.state_revision,
                    yrs_state_epoch: self.yrs_state_epoch,
                },
            )?;
            use_authority(&authority, &txn, &fragment)
        } else {
            let authority =
                yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
            use_authority(&authority, &txn, &fragment)
        }
    }
}

pub(super) fn validate_compiled_selection_plans(
    compiled: &CompiledTransaction,
) -> yrs_engine::OperationResult<()> {
    let relative_plan_is_sealed = matches!(
        (&compiled.selection_plan, &compiled.relative_selection_plan),
        (SelectionPlan::Preserve, RelativeSelectionPlan::Preserve)
            | (
                SelectionPlan::Mapped(_),
                RelativeSelectionPlan::PreserveWithFallback(_)
            )
            | (
                SelectionPlan::Explicit(_),
                RelativeSelectionPlan::Precomputed { .. }
            )
            | (
                SelectionPlan::Explicit(_),
                RelativeSelectionPlan::OperationResult
            )
    );
    if !relative_plan_is_sealed {
        return Err(yrs_engine::OperationError::engine_invariant_failed(
            compiled.request_id,
            None,
            "compiled relative selection plan is not sealed",
        ));
    }
    if !matches!(compiled.stored_marks_plan, StoredMarksPlan::Set(_)) {
        return Err(yrs_engine::OperationError::engine_invariant_failed(
            compiled.request_id,
            None,
            "compiled stored-mark plan is not sealed",
        ));
    }
    Ok(())
}
