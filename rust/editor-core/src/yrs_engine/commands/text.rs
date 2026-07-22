use super::{CommandPlan, PlanningContext, TypedCommand};
use crate::boundary::{BoundedInput, InputKind};
use crate::serialize::{FromHtmlOptions, UnknownTypeMode};
use crate::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, OperationError, OperationResult, RevisionedPosition,
    RevisionedRange, SelectionInput, SelectionIntent, StructuralReplacement, TransactionOrigin,
    TypedOperation, TypedTransaction,
};

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn bounded_json_bytes(
    request_id: u64,
    value: &serde_json::Value,
    limit: usize,
) -> OperationResult<usize> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            let actual = self.bytes.saturating_add(buffer.len());
            if actual > self.limit {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::FileTooLarge,
                    "JSON input exceeds its byte limit",
                ));
            }
            self.bytes = actual;
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).map_err(|error| {
        if counter.bytes >= limit || error.io_error_kind() == Some(std::io::ErrorKind::FileTooLarge)
        {
            OperationError::document_limit_exceeded(
                request_id,
                None,
                "maxInputBytes",
                limit as u64,
                counter.bytes.saturating_add(1) as u64,
            )
        } else {
            OperationError::document_invalid(request_id, None, "json", error.to_string())
        }
    })?;
    Ok(counter.bytes)
}

fn selection_range(
    _request_id: u64,
    selection: &crate::yrs_engine::ResolvedSelection,
) -> OperationResult<(u32, u32)> {
    match selection {
        crate::yrs_engine::ResolvedSelection::Text { anchor, head } => Ok((
            anchor.scalar.min(head.scalar),
            anchor.scalar.max(head.scalar),
        )),
        _ => Err(OperationError::transaction_invalid(
            _request_id,
            "selection",
            "text command requires a text selection",
        )),
    }
}

fn transaction_with_selection(
    context: &PlanningContext<'_>,
    operations: Vec<TypedOperation>,
    selection_intent: SelectionIntent,
    history: crate::command_planner::SemanticCommandHistory,
) -> TypedTransaction {
    let history_policy = semantic_history_policy(history);
    TypedTransaction {
        request_id: context.request_id,
        base_document_revision: context.revision,
        origin: TransactionOrigin::LocalCommand,
        operations,
        selection_intent,
        history_policy,
    }
}

fn semantic_history_policy(
    history: crate::command_planner::SemanticCommandHistory,
) -> HistoryPolicy {
    match history {
        crate::command_planner::SemanticCommandHistory::InputBoundary
        | crate::command_planner::SemanticCommandHistory::FormatBoundary => HistoryPolicy::Boundary,
    }
}

pub(super) fn semantic_transaction(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
    plan: crate::command_planner::SemanticCommandPlan,
) -> OperationResult<CommandPlan> {
    semantic_transaction_impl(context, selection, plan, None)
}

pub(super) fn admitted_semantic_transaction(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
    admitted: crate::command_planner::AdmittedSemanticCommandPlan,
) -> OperationResult<CommandPlan> {
    semantic_transaction_impl(context, selection, admitted.plan, Some(admitted.simulated))
}

fn semantic_transaction_impl(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
    plan: crate::command_planner::SemanticCommandPlan,
    admitted_simulation: Option<crate::command_planner::SimulatedCommandPlan>,
) -> OperationResult<CommandPlan> {
    if plan.operations.len() > context.editing_limits.max_operations_per_transaction {
        return Err(OperationError::operation_limit_exceeded(
            context.request_id,
            None,
            "maxOperationsPerTransaction",
            context.editing_limits.max_operations_per_transaction as u64,
            plan.operations.len() as u64,
        ));
    }
    let simulated = match admitted_simulation {
        Some(simulated) => simulated,
        None => crate::command_planner::simulate_plan(
            context.document,
            context.schema,
            selection,
            &plan,
            context.resource_limits,
        )
        .map_err(|()| {
            OperationError::operation_invalid(
                context.request_id,
                0,
                "command",
                "command simulation failed",
            )
        })?,
    };
    if let Some(preparation) = context
        .preparation
        .filter(|_| is_single_compile_prepared_plan(&plan))
    {
        let prepares_candidate_validation = plan.operations.iter().any(|operation| {
            matches!(
                operation,
                crate::command_planner::SemanticOperation::AddMark { .. }
                    | crate::command_planner::SemanticOperation::RemoveMark { .. }
                    | crate::command_planner::SemanticOperation::WrapInList { .. }
            )
        });
        let candidate_seed = prepares_candidate_validation
            .then(|| {
                crate::yrs_engine::compiler::PreparedCandidateSeed::mint(
                    context.request_id,
                    &simulated.document,
                    context.schema,
                    context.canonical_schema,
                    context.resource_limits,
                    context.editing_limits,
                    context.max_length,
                )
            })
            .transpose()?;
        let prepared = if plan.operations.iter().any(|operation| {
            matches!(
                operation,
                crate::command_planner::SemanticOperation::WrapInList { .. }
            )
        }) {
            let prepared_selection = candidate_seed
                .as_ref()
                .expect("prepared wrap retains candidate seed")
                .normalize_selection(&simulated.selection);
            let transaction = structural_fallback_transaction(
                context,
                plan.history,
                &simulated.document,
                &prepared_selection,
            )?;
            is_prepared_root_wrap_shape(&transaction).then_some((transaction, prepared_selection))
        } else {
            preferred_direct_transaction(context, selection, &plan)
                .map(|transaction| (transaction, simulated.selection.clone()))
        };
        if let Some((transaction, prepared_selection)) = prepared {
            let deferred_shape =
                crate::yrs_engine::prepared_admission::DeferredInsertShapeProof::prepare(
                    context.document,
                    plan.operations.as_slice(),
                );
            let eager_known_serialized_len = (!context.allow_deferred_admission)
                .then(|| {
                    deferred_shape.as_ref().and_then(|shape| {
                        context
                            .canonical_artifact
                            .serialized_len()
                            .checked_add(shape.escaped_body_bytes())
                    })
                })
                .flatten();
            let execution_admission = match (
                context.allow_deferred_admission,
                deferred_shape,
                context
                    .canonical_artifact
                    .admitted_serialized_upper_bound_option(),
            ) {
                (true, Some(shape), Some(base_upper))
                    if base_upper
                        .checked_add(shape.escaped_body_bytes())
                        .is_some_and(|candidate| {
                            candidate <= context.editing_limits.max_derived_output_bytes
                        }) =>
                {
                    let admission =
                        crate::yrs_engine::prepared_admission::DeferredCommandAdmission::prepare(
                            context,
                            &transaction,
                            &simulated,
                            shape,
                        )?;
                    admission.pre_admit_seed_independent()?;
                    crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Deferred(
                        admission,
                    )
                }
                _ => {
                    let admission = prepare_eager_semantic_admission(
                        context,
                        &plan,
                        &transaction,
                        &simulated,
                        candidate_seed,
                        eager_known_serialized_len,
                    )?;
                    admission.pre_admit_seed_independent(
                        &transaction,
                        &simulated.document,
                        context.editing_limits,
                    )?;
                    crate::yrs_engine::prepared_admission::ExecutionSemanticAdmission::Eager(
                        admission,
                    )
                }
            };
            preparation.replace(Some(super::PreparedCommandProof {
                document: simulated.document,
                selection: prepared_selection,
                execution_admission,
            }));
            return Ok(CommandPlan::Transaction(transaction));
        }
    }
    crate::transform::DocumentValidator::validate(
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
    if let Some(transaction) = direct_transaction(
        context,
        selection,
        &plan,
        &simulated.document,
        &simulated.selection,
    )? {
        return Ok(CommandPlan::Transaction(transaction));
    }
    let transaction = structural_fallback_transaction(
        context,
        plan.history,
        &simulated.document,
        &simulated.selection,
    )?;
    Ok(CommandPlan::Transaction(transaction))
}

fn is_prepared_root_wrap_shape(transaction: &TypedTransaction) -> bool {
    let [TypedOperation::ReplaceStructure(replacement)] = transaction.operations.as_slice() else {
        return false;
    };
    let (from_child, to_child) = replacement.child_window();
    replacement.parent_path().is_empty()
        && replacement.content().child_count() == 1
        && from_child < to_child
}

fn is_single_compile_prepared_plan(plan: &crate::command_planner::SemanticCommandPlan) -> bool {
    plan.operations.len() == 1
        && plan.operations.iter().all(|operation| {
            matches!(
                operation,
                crate::command_planner::SemanticOperation::InsertText { .. }
                    | crate::command_planner::SemanticOperation::AddMark { .. }
                    | crate::command_planner::SemanticOperation::RemoveMark { .. }
                    | crate::command_planner::SemanticOperation::WrapInList { .. }
            )
        })
}

fn prepare_eager_semantic_admission(
    context: &PlanningContext<'_>,
    plan: &crate::command_planner::SemanticCommandPlan,
    transaction: &TypedTransaction,
    simulated: &crate::command_planner::SimulatedCommandPlan,
    candidate_seed: Option<crate::yrs_engine::compiler::PreparedCandidateSeed>,
    known_canonical_serialized_len: Option<usize>,
) -> OperationResult<crate::yrs_engine::compiler::PreparedSemanticAdmission> {
    let (undo_units, command_contract_kind) = if simulated.document == *context.document {
        (
            0,
            crate::yrs_engine::compiler::PreparedCommandContractKind::None,
        )
    } else {
        match &plan.operations[0] {
            crate::command_planner::SemanticOperation::InsertText { text, .. } => (
                text.chars().count() as u64,
                crate::yrs_engine::compiler::PreparedCommandContractKind::None,
            ),
            crate::command_planner::SemanticOperation::AddMark { from, to, .. }
            | crate::command_planner::SemanticOperation::RemoveMark { from, to, .. } => (
                u64::from(to.saturating_sub(*from)),
                crate::yrs_engine::compiler::PreparedCommandContractKind::None,
            ),
            crate::command_planner::SemanticOperation::WrapInList { .. } => (
                0,
                crate::yrs_engine::compiler::PreparedCommandContractKind::RootWrap,
            ),
            _ => unreachable!("prepared eligibility admits only one direct operation"),
        }
    };
    crate::yrs_engine::compiler::PreparedSemanticAdmission::prepare_single_operation(
        context.request_id,
        context.revision,
        context.state_revision,
        context.yrs_state_epoch,
        context.schema,
        context.canonical_schema,
        context.resource_limits,
        context.editing_limits,
        context.max_length,
        transaction,
        &simulated.document,
        candidate_seed,
        known_canonical_serialized_len,
        undo_units,
        command_contract_kind,
    )
}

pub(super) fn structural_fallback_transaction(
    context: &PlanningContext<'_>,
    history: crate::command_planner::SemanticCommandHistory,
    simulated_document: &crate::model::Document,
    simulated_selection: &crate::selection::Selection,
) -> OperationResult<TypedTransaction> {
    let diff = crate::command_planner::structural_diff_bounded(
        context.document,
        simulated_document,
        context.resource_limits,
    )
    .map_err(|()| {
        OperationError::operation_work_budget_exceeded(
            context.request_id,
            "commandPlanningWork",
            "structural diff exceeded its bounded work budget",
        )
    })?
    .ok_or_else(|| {
        OperationError::operation_invalid(
            context.request_id,
            0,
            "command",
            "command produced no document change",
        )
    })?;
    if !crate::command_planner::prove_structural_diff(
        context.document,
        simulated_document,
        &diff,
        context.schema,
        context.resource_limits,
    )
    .map_err(|()| {
        OperationError::operation_work_budget_exceeded(
            context.request_id,
            "commandPlanningWork",
            "structural replacement proof exceeded its bounded work budget",
        )
    })? {
        return Err(OperationError::operation_invalid(
            context.request_id,
            0,
            "command",
            "structural replacement did not reproduce the simulated candidate",
        ));
    }
    Ok(TypedTransaction {
        request_id: context.request_id,
        base_document_revision: context.revision,
        origin: TransactionOrigin::LocalCommand,
        operations: vec![TypedOperation::ReplaceStructure(
            StructuralReplacement::new(
                diff.parent_path,
                diff.from_child,
                diff.to_child,
                diff.content,
                simulated_selection.clone(),
            ),
        )],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: semantic_history_policy(history),
    })
}

fn direct_typed_operations(
    context: &PlanningContext<'_>,
    operations: &[crate::command_planner::SemanticOperation],
) -> Option<Vec<TypedOperation>> {
    operations
        .iter()
        .map(|operation| direct_typed_operation(context, operation))
        .collect()
}

fn direct_transaction(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
    plan: &crate::command_planner::SemanticCommandPlan,
    simulated_document: &crate::model::Document,
    simulated_selection: &crate::selection::Selection,
) -> OperationResult<Option<TypedTransaction>> {
    let Some(operations) = direct_typed_operations(context, &plan.operations) else {
        return Ok(None);
    };
    let preferred = if plan.selection_after.as_ref() == Some(selection) {
        SelectionIntent::Preserve
    } else {
        SelectionIntent::UseOperationResult
    };
    let compile = |selection_intent| {
        let transaction =
            transaction_with_selection(context, operations.clone(), selection_intent, plan.history);
        let compiled = crate::yrs_engine::compiler::compile_transaction(
            crate::yrs_engine::compiler::CompilationContext {
                document: context.document,
                selection: Some(selection),
                schema: context.schema,
                resource_limits: context.resource_limits,
                editing_limits: context.editing_limits,
                document_revision: context.revision,
                max_length: context.max_length,
            },
            transaction.clone(),
        )?;
        let compiled_selection = match compiled.selection_plan {
            crate::yrs_engine::compiler::SelectionPlan::Preserve => selection.clone(),
            crate::yrs_engine::compiler::SelectionPlan::Mapped(selection)
            | crate::yrs_engine::compiler::SelectionPlan::Explicit(selection) => selection,
        };
        Ok((
            compiled.preview == *simulated_document && compiled_selection == *simulated_selection,
            transaction,
        ))
    };
    let (proved, transaction) = compile(preferred.clone())?;
    if proved {
        return Ok(Some(transaction));
    }

    let mut alternatives = Vec::new();
    let structural_selection_change = plan.selection_after.as_ref().unwrap_or(simulated_selection)
        != selection
        && plan.operations.iter().any(|operation| {
            matches!(
                operation,
                crate::command_planner::SemanticOperation::ReplaceRange { .. }
                    | crate::command_planner::SemanticOperation::SplitBlock { .. }
                    | crate::command_planner::SemanticOperation::JoinBlocks { .. }
                    | crate::command_planner::SemanticOperation::UnwrapFromList { .. }
                    | crate::command_planner::SemanticOperation::OutdentListItem { .. }
                    | crate::command_planner::SemanticOperation::WrapInList { .. }
                    | crate::command_planner::SemanticOperation::IndentListItem { .. }
                    | crate::command_planner::SemanticOperation::InsertNode { .. }
            )
        });
    if !matches!(preferred, SelectionIntent::UseOperationResult) {
        alternatives.push(SelectionIntent::UseOperationResult);
    }
    if !matches!(preferred, SelectionIntent::Preserve) && !structural_selection_change {
        alternatives.push(SelectionIntent::Preserve);
    }
    if let Some(explicit) = direct_selection_input(context, simulated_selection) {
        alternatives.push(SelectionIntent::Set(explicit));
    }
    for intent in alternatives {
        if let Ok((true, transaction)) = compile(intent) {
            return Ok(Some(transaction));
        }
    }
    Ok(None)
}

fn preferred_direct_transaction(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
    plan: &crate::command_planner::SemanticCommandPlan,
) -> Option<TypedTransaction> {
    let operations = direct_typed_operations(context, &plan.operations)?;
    let preferred = if plan.selection_after.as_ref() == Some(selection) {
        SelectionIntent::Preserve
    } else {
        SelectionIntent::UseOperationResult
    };
    Some(transaction_with_selection(
        context,
        operations,
        preferred,
        plan.history,
    ))
}

fn direct_selection_input(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
) -> Option<SelectionInput> {
    let encoded = |position: u32| {
        let scalar = context
            .position_map
            .doc_to_scalar(position, context.document);
        (context.position_map.scalar_to_doc(scalar, context.document) == position)
            .then_some(point(scalar))
    };
    Some(match selection {
        crate::selection::Selection::Text { anchor, head } => SelectionInput::Text {
            anchor: encoded(*anchor)?,
            head: encoded(*head)?,
        },
        crate::selection::Selection::Node { pos } => SelectionInput::Node { at: encoded(*pos)? },
        crate::selection::Selection::All => SelectionInput::All,
    })
}

fn direct_typed_operation(
    context: &PlanningContext<'_>,
    operation: &crate::command_planner::SemanticOperation,
) -> Option<TypedOperation> {
    let encoded = |position: u32| {
        let scalar = context
            .position_map
            .doc_to_scalar(position, context.document);
        (context.position_map.scalar_to_doc(scalar, context.document) == position)
            .then_some(point(scalar))
    };
    Some(match operation {
        crate::command_planner::SemanticOperation::InsertText { pos, text, marks } => {
            TypedOperation::InsertText {
                at: encoded(*pos)?,
                text: text.clone(),
                marks: marks.clone(),
            }
        }
        crate::command_planner::SemanticOperation::DeleteRange { from, to } => {
            TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: encoded(*from)?,
                    to: encoded(*to)?,
                },
            }
        }
        crate::command_planner::SemanticOperation::AddMark { from, to, mark } => {
            TypedOperation::AddMark {
                range: RevisionedRange {
                    from: encoded(*from)?,
                    to: encoded(*to)?,
                },
                mark: mark.clone(),
            }
        }
        crate::command_planner::SemanticOperation::RemoveMark {
            from,
            to,
            mark_type,
        } => TypedOperation::RemoveMark {
            range: RevisionedRange {
                from: encoded(*from)?,
                to: encoded(*to)?,
            },
            mark_type: mark_type.clone(),
        },
        crate::command_planner::SemanticOperation::ReplaceMark { from, to, mark } => {
            TypedOperation::ReplaceMark {
                range: RevisionedRange {
                    from: encoded(*from)?,
                    to: encoded(*to)?,
                },
                mark: mark.clone(),
            }
        }
        crate::command_planner::SemanticOperation::ReplaceRange { from, to, content } => {
            // The concrete Yrs ReplaceRange lowering writes into existing XML text
            // targets. Block fragments need the structural lowering below instead.
            if content.iter().any(|node| node.text_str().is_none()) {
                return None;
            }
            TypedOperation::ReplaceRange {
                range: RevisionedRange {
                    from: encoded(*from)?,
                    to: encoded(*to)?,
                },
                content: content.clone(),
            }
        }
        crate::command_planner::SemanticOperation::SplitBlock {
            pos,
            node_type,
            attrs,
        } => TypedOperation::SplitBlock {
            at: encoded(*pos)?,
            node_type: node_type.clone(),
            attrs: attrs.clone(),
        },
        crate::command_planner::SemanticOperation::JoinBlocks { pos } => {
            TypedOperation::JoinBlocks { at: encoded(*pos)? }
        }
        crate::command_planner::SemanticOperation::UnwrapFromList { pos } => {
            TypedOperation::UnwrapFromList { at: encoded(*pos)? }
        }
        crate::command_planner::SemanticOperation::OutdentListItem { pos } => {
            TypedOperation::OutdentListItem { at: encoded(*pos)? }
        }
        crate::command_planner::SemanticOperation::WrapInList {
            from,
            to,
            list_type,
            item_type,
            attrs,
            item_attrs,
        } => TypedOperation::WrapInList {
            range: RevisionedRange {
                from: encoded(*from)?,
                to: encoded(*to)?,
            },
            list_type: list_type.clone(),
            item_type: item_type.clone(),
            attrs: attrs.clone(),
            item_attrs: item_attrs.clone(),
        },
        crate::command_planner::SemanticOperation::IndentListItem { pos } => {
            TypedOperation::IndentListItem { at: encoded(*pos)? }
        }
        crate::command_planner::SemanticOperation::InsertNode { pos, node } => {
            TypedOperation::InsertNode {
                at: encoded(*pos)?,
                node: node.clone(),
            }
        }
        crate::command_planner::SemanticOperation::UpdateNodeAttrs { pos, attrs } => {
            TypedOperation::UpdateNodeAttrs {
                at: encoded(*pos)?,
                attrs: attrs.clone(),
            }
        }
    })
}

pub(super) fn plan(
    context: PlanningContext<'_>,
    command: TypedCommand,
) -> OperationResult<CommandPlan> {
    let command = match command {
        TypedCommand::DeleteRange { range: requested } => {
            let rendered = crate::render::rendered_text(context.document, context.schema);
            let resolve = |position: RevisionedPosition, field| {
                let scalar = crate::yrs_engine::position::editor_offset_to_scalar(
                    position.offset,
                    position.kind,
                    &rendered,
                    context.position_map,
                )
                .ok_or_else(|| {
                    OperationError::position_invalid(
                        context.request_id,
                        0,
                        field,
                        format!("{field} is outside the rendered document"),
                    )
                })?;
                Ok(scalar)
            };
            let from = resolve(requested.from, "range.from")?;
            let to = resolve(requested.to, "range.to")?;
            if from == to {
                return Ok(CommandPlan::NotApplicable);
            }
            let (from, to) = if from < to { (from, to) } else { (to, from) };
            let Some(plan) = crate::command_planner::plan_delete_scalar_range(
                context.document,
                context.position_map,
                context.schema,
                from,
                to,
            )
            .map_err(|()| {
                OperationError::operation_invalid(
                    context.request_id,
                    0,
                    "command",
                    "delete command planning failed",
                )
            })?
            else {
                return Ok(CommandPlan::NotApplicable);
            };
            let selection = crate::selection::Selection::text(
                context.position_map.scalar_to_doc(from, context.document),
                context.position_map.scalar_to_doc(to, context.document),
            );
            return semantic_transaction(&context, &selection, plan);
        }
        TypedCommand::InsertText { text } => {
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_insert_text(
                context.document,
                context.schema,
                &selection,
                context.stored_marks,
                &text,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            return semantic_transaction(&context, &selection, plan);
        }
        TypedCommand::ReplaceSelectionText { text } => {
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_replace_selection_text(
                context.document,
                context.schema,
                &selection,
                context.stored_marks,
                &text,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            return semantic_transaction(&context, &selection, plan);
        }
        command => command,
    };
    let Ok((_from, _to)) = selection_range(context.request_id, context.selection) else {
        return Ok(CommandPlan::NotApplicable);
    };
    match command {
        TypedCommand::DeleteBackward => {
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_delete_backward(
                context.document,
                context.position_map,
                context.schema,
                &selection,
                context.resource_limits,
            )
            .map_err(|()| {
                OperationError::operation_work_budget_exceeded(
                    context.request_id,
                    "commandPlanningWork",
                    "command planning exceeded its bounded work budget",
                )
            })?
            else {
                return Ok(CommandPlan::NotApplicable);
            };
            semantic_transaction(&context, &selection, plan)
        }
        TypedCommand::SplitBlock | TypedCommand::DeleteAndSplit => {
            let delete_selection = matches!(command, TypedCommand::DeleteAndSplit);
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_split(
                context.document,
                context.position_map,
                context.schema,
                &selection,
                delete_selection,
                context.resource_limits,
            )
            .map_err(|()| {
                OperationError::operation_work_budget_exceeded(
                    context.request_id,
                    "commandPlanningWork",
                    "command planning exceeded its bounded work budget",
                )
            })?
            else {
                return Ok(CommandPlan::NotApplicable);
            };
            semantic_transaction(&context, &selection, plan)
        }
        TypedCommand::InsertContentJson { .. } | TypedCommand::InsertContentHtml { .. } => {
            let parsed = match command {
                TypedCommand::InsertContentJson { json } => {
                    bounded_json_bytes(
                        context.request_id,
                        &json,
                        context.resource_limits.max_input_bytes,
                    )?;
                    crate::serialize::from_prosemirror_json_with_limits(
                        &json,
                        context.schema,
                        UnknownTypeMode::Preserve,
                        context.resource_limits,
                    )
                    .map_err(|error| {
                        OperationError::document_invalid(
                            context.request_id,
                            None,
                            "json",
                            error.to_string(),
                        )
                    })?
                }
                TypedCommand::InsertContentHtml { html } => {
                    BoundedInput::new(&html, InputKind::Html, context.resource_limits).map_err(
                        |error| {
                            OperationError::document_limit_exceeded(
                                context.request_id,
                                None,
                                "maxHtmlBytes",
                                error.limit.unwrap_or(0) as u64,
                                error.actual.unwrap_or(0) as u64,
                            )
                        },
                    )?;
                    crate::serialize::from_html_with_limits(
                        &html,
                        context.schema,
                        &FromHtmlOptions {
                            strict: false,
                            allow_base64_images: false,
                        },
                        context.resource_limits,
                    )
                    .map_err(|error| {
                        OperationError::document_invalid(
                            context.request_id,
                            None,
                            "html",
                            error.to_string(),
                        )
                    })?
                }
                _ => unreachable!(),
            };
            let Some(content) = parsed.root().content().cloned() else {
                return Ok(CommandPlan::NotApplicable);
            };
            if content.size() == 0 {
                return Ok(CommandPlan::NotApplicable);
            }
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(replacement) = crate::editor_state::plan_content_insertion(
                context.document,
                context.schema,
                &selection,
                &content,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: replacement.from,
                        to: replacement.to,
                        content: replacement.content,
                    }],
                    selection_after: Some(replacement.selection_after),
                    history: crate::command_planner::SemanticCommandHistory::InputBoundary,
                },
            )
        }
        _ => unreachable!("text planner received non-text command"),
    }
}

#[cfg(test)]
mod tests {
    use crate::boundary::ResourceLimits;
    use crate::position::PositionMap;
    use crate::schema::presets::tiptap_schema;
    use crate::selection::Selection;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use crate::yrs_engine::canonical::CanonicalSchemaContext;
    use crate::yrs_engine::commands::PlanningContext;
    use crate::yrs_engine::{EditingLimits, ResolvedPoint, ResolvedSelection};

    const BEFORE: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]}]}"#;
    const AFTER: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]},{"type":"paragraph","content":[{"type":"text","text":"World"}]}]}"#;

    // Task 16B: a bounded command-planning work budget is a deterministic
    // operation limit, not allocation-class resource exhaustion.
    #[test]
    fn structural_diff_work_budget_excess_is_an_operation_limit_not_resource_exhaustion() {
        let schema = tiptap_schema();
        let before = from_prosemirror_json(
            &serde_json::from_str(BEFORE).unwrap(),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let after = from_prosemirror_json(
            &serde_json::from_str(AFTER).unwrap(),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        // The structural diff work budget is max_document_nodes * 4; the two
        // fixture trees exceed a budget of 4 without any allocation failure.
        let limits = ResourceLimits {
            max_document_nodes: 1,
            ..ResourceLimits::default()
        };
        let editing_limits = EditingLimits::default();
        let canonical_schema = CanonicalSchemaContext::new(&schema);
        let canonical_artifact = canonical_schema.derive(&before).unwrap();
        let position_map = PositionMap::build(&before, &schema);
        let rendered_text = crate::render::rendered_text(&before, &schema);
        let point = ResolvedPoint {
            document: 0,
            scalar: 0,
            utf16: 0,
        };
        let selection = ResolvedSelection::Text {
            anchor: point,
            head: point,
        };
        let context = PlanningContext {
            request_id: 7,
            revision: 0,
            state_revision: 0,
            document: &before,
            position_map: &position_map,
            rendered_text: &rendered_text,
            selection: &selection,
            stored_marks: None,
            schema: &schema,
            resource_limits: &limits,
            editing_limits: &editing_limits,
            max_length: None,
            yrs_state_epoch: 0,
            canonical_schema: &canonical_schema,
            canonical_artifact: &canonical_artifact,
            allow_deferred_admission: false,
            preparation: None,
        };
        let error = super::structural_fallback_transaction(
            &context,
            crate::command_planner::SemanticCommandHistory::InputBoundary,
            &after,
            &Selection::cursor(0),
        )
        .unwrap_err();
        assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "field": "commandPlanningWork" }))
        );
    }
}
