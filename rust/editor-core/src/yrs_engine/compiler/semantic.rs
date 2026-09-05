use crate::model::Mark;
use crate::transform::StepMap;
use crate::yrs_engine;
use crate::yrs_engine::canonical::CanonicalSchemaContext;
use crate::yrs_engine::compiler::admission::PreparedCommandContractOracle;
#[cfg(test)]
use crate::yrs_engine::compiler::observability::SEMANTIC_COMPILATION_COUNT;
use crate::yrs_engine::compiler::operations::OperationCompiler;
use crate::yrs_engine::compiler::positions::map_position;
use crate::yrs_engine::compiler::preview::{
    affected_top_level_blocks, derive_localized_preview_document, derive_preview_document,
    prepared_candidate_matches, validate_preview,
};
use crate::yrs_engine::compiler::selection::{position_update_mode, selection_plan};
use crate::yrs_engine::compiler::{
    base_position_map, base_rendered_text, CachedCompilationView, CompilationContext,
    CompiledTransaction, HistoryClass, RelativeSelectionPlan, SelectionPlan,
    SemanticCompilationShortcuts, StoredMarksCompilationContext, StoredMarksPlan,
    TransactionMutationLowering,
};
use crate::yrs_engine::editing_limits::CheckedWork;
use crate::yrs_engine::mutation::{
    estimate_undo_units, estimate_update_v1_growth, planned_insertion_units, CrdtEnvelope,
    YrsMutationPlan,
};
use crate::yrs_engine::{
    HistoryPolicy, OperationError, OperationResult, SelectionIntent, TransactionOrigin,
    TypedTransaction,
};
use std::borrow::Cow;

pub(super) fn compile_transaction_impl(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
    mutation_lowering: Option<TransactionMutationLowering>,
    mut crdt_envelope_loader: Option<&mut dyn FnMut(usize) -> OperationResult<CrdtEnvelope>>,
    stored_marks_context: Option<StoredMarksCompilationContext<'_>>,
    cached_view: Option<CachedCompilationView<'_>>,
    semantic_shortcuts: SemanticCompilationShortcuts<'_>,
) -> OperationResult<CompiledTransaction> {
    let SemanticCompilationShortcuts {
        prepared: prepared_semantics,
        localized: localized_semantic,
    } = semantic_shortcuts;
    let (
        lowering,
        localized_insert,
        localized_format,
        localized_root_window,
        prelowered_plan,
        prelowered_lookup_transition,
    ) = match mutation_lowering {
        Some(TransactionMutationLowering::Eager(compiler)) => {
            (Some(*compiler), None, None, None, None, None)
        }
        Some(TransactionMutationLowering::LocalizedInsert(localized)) => {
            (None, Some(*localized), None, None, None, None)
        }
        Some(TransactionMutationLowering::LocalizedFormat(localized)) => {
            (None, None, Some(*localized), None, None, None)
        }
        Some(TransactionMutationLowering::LocalizedRootWindow(localized)) => {
            (None, None, None, Some(*localized), None, None)
        }
        None => (None, None, None, None, None, None),
    };
    #[cfg(test)]
    SEMANTIC_COMPILATION_COUNT.set(SEMANTIC_COMPILATION_COUNT.get().saturating_add(1));
    let request_id = transaction.request_id;
    let work = CheckedWork::default();
    // Revision, origin, operation count and aggregate input were admitted by the
    // single shared envelope path before any optional Yrs target traversal.
    debug_assert_eq!(
        transaction.base_document_revision,
        context.document_revision
    );
    debug_assert!(matches!(
        transaction.origin,
        TransactionOrigin::LocalInput
            | TransactionOrigin::LocalCommand
            | TransactionOrigin::LocalApi
    ));
    debug_assert!(
        transaction.operations.len() <= context.editing_limits.max_operations_per_transaction
    );

    let owned_base_position_map;
    let owned_rendered_text;
    let (base_position_map, rendered_text, rendered_scalars) = if let Some(cached) = cached_view {
        (
            cached.position_map,
            cached.rendered_text,
            cached.rendered_scalars,
        )
    } else {
        owned_base_position_map = base_position_map(context.document, context.schema);
        owned_rendered_text = base_rendered_text(context.document, context.schema);
        let rendered_scalars = u32::try_from(owned_rendered_text.chars().count()).ok();
        (
            &owned_base_position_map,
            owned_rendered_text.as_str(),
            rendered_scalars.unwrap_or(u32::MAX),
        )
    };
    if rendered_scalars != base_position_map.total_scalars() {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "rendered text and base position map have different scalar lengths",
        ));
    }
    let yrs_lowering_requested = lowering.is_some()
        || localized_insert.is_some()
        || localized_format.is_some()
        || localized_root_window.is_some();
    let preview = Cow::Borrowed(context.document);
    let composed_map = StepMap::empty();
    let operation_result = None;
    let undo_units_bound = 0u64;
    let undo_limit_error = None;
    let history_class = HistoryClass::Skip;
    let records_history = transaction.history_policy != HistoryPolicy::Skip;
    let canonical_artifact = (!transaction.operations.is_empty())
        .then(|| cached_view.map(|cached| cached.canonical_artifact.clone()))
        .flatten();
    let owned_canonical_schema;
    let canonical_schema = if let Some(cached) = cached_view {
        cached.canonical_artifact.schema_context()
    } else {
        owned_canonical_schema = CanonicalSchemaContext::new(context.schema);
        &owned_canonical_schema
    };
    let stored_marks_state = stored_marks_context
        .as_ref()
        .map(|state| state.stored_marks.map(<[Mark]>::to_vec));
    // Set when a caret-anchored SplitBlock carried the stored marks through.
    // Return moves the caret across the new block boundary, so the generic
    // "caret mapped to the same place" compatibility check cannot see it.
    let split_at_caret_kept_stored_marks = false;
    let tracked_caret = stored_marks_context.as_ref().and_then(|state| {
        let yrs_engine::ResolvedSelection::Text { anchor, head } = state.resolved_selection else {
            return None;
        };
        let yrs_engine::RelativeSelection::Text {
            head: relative_head,
            ..
        } = state.relative_selection
        else {
            return None;
        };
        (anchor.document == head.document).then_some((head.document, relative_head.affinity))
    });
    let localized_derivations = None;
    let mut operations = OperationCompiler {
        context,
        transaction,
        prepared_semantics,
        localized_semantic,
        lowering,
        localized_insert,
        localized_format,
        localized_root_window,
        prelowered_plan,
        prelowered_lookup_transition,
        request_id,
        work,
        base_position_map,
        rendered_text,
        preview,
        composed_map,
        operation_result,
        undo_units_bound,
        undo_limit_error,
        history_class,
        records_history,
        canonical_artifact,
        canonical_schema,
        stored_marks_state,
        split_at_caret_kept_stored_marks,
        tracked_caret,
        localized_derivations,
    };
    for (operation_index, operation) in transaction.operations.iter().enumerate() {
        operations = operations.compile(operation_index, operation)?;
    }
    let OperationCompiler {
        context,
        transaction,
        prepared_semantics,
        localized_semantic: _localized_semantic,
        lowering,
        localized_insert: _localized_insert,
        localized_format: _localized_format,
        localized_root_window: _localized_root_window,
        prelowered_plan,
        prelowered_lookup_transition,
        request_id,
        work: _work,
        base_position_map,
        rendered_text,
        mut preview,
        composed_map,
        operation_result,
        mut undo_units_bound,
        mut undo_limit_error,
        mut history_class,
        records_history: _records_history,
        canonical_artifact,
        canonical_schema,
        stored_marks_state,
        split_at_caret_kept_stored_marks,
        tracked_caret,
        mut localized_derivations,
    } = operations;

    let prepared_candidate_matches_preview = transaction
        .operations
        .len()
        .checked_sub(1)
        .is_some_and(|operation_index| {
            prepared_candidate_matches(
                prepared_semantics,
                transaction.operations.len(),
                operation_index,
                &preview,
                context,
                canonical_schema,
            )
        });
    if localized_derivations.is_none() && !prepared_candidate_matches_preview {
        validate_preview(
            request_id,
            transaction.operations.len().checked_sub(1),
            &preview,
            context,
        )?;
    }
    let prepared_candidate_validation = prepared_semantics
        .filter(|_| prepared_candidate_matches_preview)
        .and_then(|prepared| {
            preview = Cow::Borrowed(prepared.expected_preview);
            prepared.admission.candidate_validation()
        });
    let localized_semantic_used = localized_derivations.is_some();
    let affected_top_level_blocks = if let Some(localized) = localized_derivations.as_mut() {
        std::mem::take(&mut localized.affected_top_level_blocks)
    } else {
        affected_top_level_blocks(context.document, &preview)
    };
    let position_update_mode = position_update_mode(&transaction.operations);
    let preview_derivations = if let Some(prepared) = prepared_candidate_validation.as_ref() {
        let canonical_artifact = canonical_artifact.as_ref().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "prepared candidate has no canonical artifact",
            )
        })?;
        Some(
            prepared
                .compiled_derivations(
                    &preview,
                    canonical_artifact,
                    context.resource_limits,
                    context.editing_limits,
                    context.max_length,
                    prepared_semantics
                        .expect("prepared candidate retains semantic context")
                        .schema_fingerprint,
                    canonical_schema,
                )
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "prepared candidate derivations do not match the live command context",
                    )
                })?,
        )
    } else if yrs_lowering_requested && *preview != *context.document {
        Some(if let Some(localized) = localized_derivations.take() {
            derive_localized_preview_document(
                request_id,
                context,
                base_position_map,
                &preview,
                &composed_map,
                position_update_mode,
                &affected_top_level_blocks,
                localized,
            )?
        } else {
            derive_preview_document(
                request_id,
                context,
                base_position_map,
                &preview,
                &composed_map,
                position_update_mode,
                &affected_top_level_blocks,
            )?
        })
    } else {
        None
    };
    let use_operation_result_falls_back_to_preserve = operation_result.is_none();
    let selection_plan = selection_plan(
        context,
        &transaction.selection_intent,
        rendered_text,
        base_position_map,
        &composed_map,
        operation_result,
        request_id,
        &preview,
        preview_derivations
            .as_ref()
            .map(|derivations| &derivations.position_map),
    )?;
    let stored_marks_plan = if let (Some(mut stored), Some(initial)) =
        (stored_marks_state, stored_marks_context.as_ref())
    {
        let after = match &selection_plan {
            SelectionPlan::Preserve => initial.resolved_selection.clone(),
            SelectionPlan::Mapped(selection) | SelectionPlan::Explicit(selection) => {
                let resolved = if let Some(derivations) = preview_derivations.as_ref() {
                    yrs_engine::derived_state::resolved_from_legacy_with_view(
                        &preview,
                        selection,
                        context.schema,
                        &derivations.position_map,
                        &derivations.rendered_text,
                    )
                } else {
                    yrs_engine::derived_state::resolved_from_legacy(
                        &preview,
                        selection,
                        context.schema,
                    )
                };
                resolved.ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "compiled selection cannot produce resolved stored-mark state",
                    )
                })?
            }
        };
        let mapped_tracked_caret = tracked_caret
            .map(|(position, affinity)| map_position(&composed_map, position, affinity));
        let after_is_mapped_tracked_caret = mapped_tracked_caret.is_some_and(|mapped| {
            matches!(
                &after,
                yrs_engine::ResolvedSelection::Text { anchor, head }
                    if anchor.document == head.document && head.document == mapped
            )
        });
        let compatible_moved_selection = match transaction.selection_intent {
            SelectionIntent::Preserve => tracked_caret.is_some(),
            SelectionIntent::UseOperationResult if use_operation_result_falls_back_to_preserve => {
                tracked_caret.is_some()
            }
            SelectionIntent::UseOperationResult => {
                after_is_mapped_tracked_caret || split_at_caret_kept_stored_marks
            }
            SelectionIntent::Set(_) => false,
        };
        let after_is_collapsed_text = matches!(
            &after,
            yrs_engine::ResolvedSelection::Text { anchor, head }
                if anchor.document == head.document
        );
        if !after_is_collapsed_text
            || (initial.resolved_selection != &after && !compatible_moved_selection)
        {
            stored = None;
        } else if matches!(transaction.selection_intent, SelectionIntent::Set(_))
            || transaction.operations.is_empty()
        {
            stored = yrs_engine::derived_state::stored_marks_after_selection_change(
                stored.as_deref(),
                initial.resolved_selection,
                &after,
                &preview,
                context.schema,
            );
        }
        StoredMarksPlan::Set(stored)
    } else {
        StoredMarksPlan::Unsealed
    };
    if *preview == *context.document || transaction.history_policy == HistoryPolicy::Skip {
        history_class = HistoryClass::Skip;
        undo_units_bound = 0;
        undo_limit_error = None;
    }

    let yrs_lowered = yrs_lowering_requested;
    let lowered_plan = lowering
        .map(|compiler| compiler.finish(transaction.operations.len().checked_sub(1)))
        .transpose()?
        .or(prelowered_plan)
        .unwrap_or_default();
    let mut mutation_plan = if *preview == *context.document {
        YrsMutationPlan::default()
    } else {
        lowered_plan
    };
    mutation_plan.cache_prepared_metrics(request_id)?;
    let authored_clock_units = planned_insertion_units(request_id, &mutation_plan)?;
    let crdt_envelope = if mutation_plan.requires_crdt_envelope() {
        Some(crdt_envelope_loader.as_mut().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs live deletion plan has no snapshot envelope loader",
            )
        })?(mutation_plan.scan_work)?)
    } else {
        None
    };
    let replay_work_units_bound = if yrs_lowered {
        estimate_undo_units(request_id, &mutation_plan, crdt_envelope.as_ref())?
    } else {
        undo_units_bound
    };
    let admitted_undo_units_bound = match prepared_semantics
        .map(|prepared| prepared.admission.command_contract_oracle())
        .unwrap_or(PreparedCommandContractOracle::None)
    {
        PreparedCommandContractOracle::None => replay_work_units_bound,
        PreparedCommandContractOracle::RootWrap {
            direct_insertion_units,
            direct_growth_bytes: _,
            replaced_children: _,
        } => {
            let envelope = crdt_envelope.as_ref().ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    transaction.operations.len().checked_sub(1),
                    "prepared root-wrap undo oracle has no live deletion envelope",
                )
            })?;
            replay_work_units_bound.max(yrs_engine::mutation::deleting_plan_undo_units(
                request_id,
                transaction.operations.len().saturating_sub(1),
                direct_insertion_units,
                envelope,
            )?)
        }
    };
    if yrs_lowered && history_class != HistoryClass::Skip {
        undo_units_bound = replay_work_units_bound;
        if admitted_undo_units_bound > context.editing_limits.max_undo_retained_units {
            // The semantic pass records the first operation that crosses the
            // aggregate limit. Preserve only that attribution when the exact
            // Yrs estimator confirms the failure: the reported `actual` must
            // always be the exact plan-derived bound.
            let operation_index = undo_limit_error
                .as_ref()
                .and_then(|error| error.operation_index)
                .or_else(|| transaction.operations.len().checked_sub(1));
            undo_limit_error = Some(OperationError::operation_limit_exceeded(
                request_id,
                operation_index,
                "maxUndoRetainedUnits",
                context.editing_limits.max_undo_retained_units,
                admitted_undo_units_bound,
            ));
        } else {
            undo_limit_error = None;
        }
    }
    if let Some(error) = undo_limit_error {
        return Err(error);
    }
    let encoded_growth_bound =
        estimate_update_v1_growth(request_id, &mutation_plan, crdt_envelope.as_ref())?;
    let encoded_growth_bound = match prepared_semantics
        .map(|prepared| prepared.admission.command_contract_oracle())
        .unwrap_or(PreparedCommandContractOracle::None)
    {
        PreparedCommandContractOracle::None => encoded_growth_bound,
        PreparedCommandContractOracle::RootWrap {
            direct_insertion_units,
            direct_growth_bytes,
            replaced_children,
        } => {
            let envelope = crdt_envelope.as_ref().ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    transaction.operations.len().checked_sub(1),
                    "prepared root-wrap growth oracle has no live deletion envelope",
                )
            })?;
            encoded_growth_bound.max(yrs_engine::mutation::direct_xml_replacement_growth(
                request_id,
                transaction.operations.len().saturating_sub(1),
                replaced_children,
                direct_growth_bytes,
                direct_insertion_units,
                envelope,
            )?)
        }
    };
    let mutation_lookup_transition = (!mutation_plan.is_empty())
        .then_some(prelowered_lookup_transition)
        .flatten();

    Ok(CompiledTransaction {
        request_id,
        base_state_revision: cached_view.map_or(0, |cached| cached.state_revision),
        origin: transaction.origin,
        history_policy: transaction.history_policy,
        history_class,
        preview: preview.into_owned(),
        canonical_artifact,
        preview_derivations,
        selection_plan,
        affected_top_level_blocks,
        composed_map,
        position_update_mode,
        relative_selection_plan: RelativeSelectionPlan::Unsealed,
        stored_marks_plan,
        mutation_plan,
        mutation_lookup_transition,
        encoded_growth_bound,
        undo_units_bound,
        replay_work_units_bound,
        authored_clock_units,
        // Standalone compiler tests do not own an engine epoch. The engine
        // seals its current epoch onto the compiled plan before it can leave
        // the stable read view.
        yrs_state_epoch: 0,
        localized_insert_admission: None,
        prepared_derived_evidence: None,
        prepared_candidate_validation,
        prepared_active_state_transition: None,
        prepared_selection_state: None,
        prepared_selection_mutation_seal: None,
        localized_semantic_used,
    })
}
