use crate::model::Mark;
use crate::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, OperationResult, RevisionedPosition,
    RevisionedRange, SelectionIntent, TypedOperation, TypedTransaction,
};

use super::{CommandPlan, PlanningContext, TypedCommand};

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn validate_mark_request(
    context: &PlanningContext<'_>,
    mark_type: &str,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
) -> OperationResult<()> {
    crate::command_planner::validate_mark_request(context.schema, mark_type, attrs).map_err(
        |error| {
            let message = match error {
                crate::command_planner::MarkRequestError::UnknownMark => {
                    format!("unknown mark '{mark_type}'")
                }
                crate::command_planner::MarkRequestError::RequiredAttribute(name) => {
                    format!("'{mark_type}' requires attribute '{name}'")
                }
                crate::command_planner::MarkRequestError::UndeclaredAttribute(name) => {
                    format!("'{mark_type}' contains undeclared attribute '{name}'")
                }
            };
            crate::yrs_engine::OperationError::operation_invalid(
                context.request_id,
                0,
                "mark",
                message,
            )
        },
    )
}

fn state_only_mark_transaction(
    context: &PlanningContext<'_>,
    plan: crate::command_planner::MarkCommandPlan,
) -> CommandPlan {
    let encoded = |position: u32| {
        point(
            context
                .position_map
                .doc_to_scalar(position, context.document),
        )
    };
    let operations = plan
        .semantic
        .operations
        .into_iter()
        .map(|operation| match operation {
            crate::command_planner::SemanticOperation::AddMark { from, to, mark } => {
                TypedOperation::AddMark {
                    range: RevisionedRange {
                        from: encoded(from),
                        to: encoded(to),
                    },
                    mark,
                }
            }
            crate::command_planner::SemanticOperation::RemoveMark {
                from,
                to,
                mark_type,
            } => TypedOperation::RemoveMark {
                range: RevisionedRange {
                    from: encoded(from),
                    to: encoded(to),
                },
                mark_type,
            },
            crate::command_planner::SemanticOperation::ReplaceMark { from, to, mark } => {
                TypedOperation::ReplaceMark {
                    range: RevisionedRange {
                        from: encoded(from),
                        to: encoded(to),
                    },
                    mark,
                }
            }
            _ => unreachable!("stored-mark planner emitted a non-mark operation"),
        })
        .collect();
    CommandPlan::SelectionOnly(TypedTransaction {
        request_id: context.request_id,
        base_document_revision: context.revision,
        origin: context.origin,
        operations,
        selection_intent: context
            .initial_selection
            .cloned()
            .map(SelectionIntent::Set)
            .unwrap_or(SelectionIntent::Preserve),
        history_policy: HistoryPolicy::Boundary,
    })
}

pub(super) fn plan(
    context: PlanningContext<'_>,
    command: TypedCommand,
) -> OperationResult<CommandPlan> {
    let crate::yrs_engine::ResolvedSelection::Text { anchor, head } = context.selection else {
        return Ok(CommandPlan::NotApplicable);
    };
    let selection = crate::selection::Selection::text(
        context
            .position_map
            .scalar_to_doc(anchor.scalar, context.document),
        context
            .position_map
            .scalar_to_doc(head.scalar, context.document),
    );
    match command {
        TypedCommand::ToggleMark { mark_type } => {
            validate_mark_request(&context, &mark_type, &Default::default())?;
            let Some(plan) = crate::command_planner::plan_toggle_mark(
                context.document,
                context.schema,
                &selection,
                context.stored_marks,
                &mark_type,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            if plan.stored_marks_after.is_some() {
                Ok(state_only_mark_transaction(&context, plan))
            } else {
                super::text::semantic_transaction(&context, &selection, plan.semantic)
            }
        }
        TypedCommand::SetMark { mark_type, attrs } => {
            validate_mark_request(&context, &mark_type, &attrs)?;
            let Some(plan) = crate::command_planner::plan_set_mark(
                context.document,
                context.schema,
                &selection,
                context.stored_marks,
                Mark::new(mark_type, attrs),
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            if plan.stored_marks_after.is_some() {
                Ok(state_only_mark_transaction(&context, plan))
            } else {
                super::text::semantic_transaction(&context, &selection, plan.semantic)
            }
        }
        TypedCommand::UnsetMark { mark_type } => {
            crate::command_planner::validate_mark_type(context.schema, &mark_type).map_err(
                |_| {
                    crate::yrs_engine::OperationError::operation_invalid(
                        context.request_id,
                        0,
                        "mark",
                        format!("unknown mark '{mark_type}'"),
                    )
                },
            )?;
            let Some(plan) = crate::command_planner::plan_unset_mark(
                context.document,
                context.schema,
                &selection,
                context.stored_marks,
                &mark_type,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            if plan.stored_marks_after.is_some() {
                Ok(state_only_mark_transaction(&context, plan))
            } else {
                super::text::semantic_transaction(&context, &selection, plan.semantic)
            }
        }
        TypedCommand::ToggleHeading { level } => {
            let Some(plan) = crate::command_planner::plan_toggle_heading(
                context.document,
                context.schema,
                &selection,
                level,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            super::text::semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: plan.from,
                        to: plan.to,
                        content: plan.content,
                    }],
                    selection_after: Some(plan.selection_after),
                    history: crate::command_planner::SemanticCommandHistory::FormatBoundary,
                },
            )
        }
        TypedCommand::ToggleCodeBlock => {
            let Some(plan) = crate::command_planner::plan_toggle_code_block(
                context.document,
                context.schema,
                &selection,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            super::text::semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: plan.from,
                        to: plan.to,
                        content: plan.content,
                    }],
                    selection_after: Some(plan.selection_after),
                    history: crate::command_planner::SemanticCommandHistory::FormatBoundary,
                },
            )
        }
        TypedCommand::ToggleBlockquote => {
            let Some(plan) = crate::command_planner::plan_toggle_blockquote(
                context.document,
                context.schema,
                &selection,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            super::text::semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: plan.from,
                        to: plan.to,
                        content: plan.content,
                    }],
                    selection_after: Some(plan.selection_after),
                    history: crate::command_planner::SemanticCommandHistory::FormatBoundary,
                },
            )
        }
        _ => unreachable!("format planner received non-format command"),
    }
}
