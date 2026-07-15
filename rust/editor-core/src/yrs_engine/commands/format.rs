use crate::model::Mark;
use crate::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, OperationResult, RevisionedPosition,
    RevisionedRange, SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction,
};

use super::{CommandPlan, PlanningContext, TypedCommand};

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

pub(super) fn plan(
    context: PlanningContext<'_>,
    command: TypedCommand,
) -> OperationResult<CommandPlan> {
    let crate::yrs_engine::ResolvedSelection::Text { anchor, head } = context.selection else {
        return Ok(CommandPlan::NotApplicable);
    };
    let from = anchor.scalar.min(head.scalar);
    let to = anchor.scalar.max(head.scalar);
    let selection_range = RevisionedRange {
        from: point(from),
        to: point(to),
    };
    let doc_from = anchor.document.min(head.document);
    let doc_to = anchor.document.max(head.document);
    let scalar_range_for_doc = |doc_from: u32, doc_to: u32| RevisionedRange {
        from: point(
            context
                .position_map
                .doc_to_scalar(doc_from, context.document),
        ),
        to: point(context.position_map.doc_to_scalar(doc_to, context.document)),
    };
    let mut state_only = false;
    let uses_operation_result = false;
    let operations = match command {
        TypedCommand::ToggleMark { mark_type } => {
            let active = if from == to {
                context
                    .stored_marks
                    .map(|marks| marks.iter().any(|mark| mark.mark_type() == mark_type))
                    .unwrap_or_else(|| {
                        crate::editor_state::marks_at_position(context.document, anchor.document)
                            .iter()
                            .any(|mark| mark.mark_type() == mark_type)
                    })
            } else {
                crate::editor_state::range_has_mark(context.document, doc_from, doc_to, &mark_type)
            };
            state_only = from == to;
            if active {
                vec![TypedOperation::RemoveMark {
                    range: selection_range,
                    mark_type,
                }]
            } else {
                vec![TypedOperation::AddMark {
                    range: selection_range,
                    mark: Mark::new(mark_type, Default::default()),
                }]
            }
        }
        TypedCommand::SetMark { mark_type, attrs } if from == to => {
            if let Some((mark_from, mark_to)) = crate::editor_state::mark_range_at_position(
                context.document,
                anchor.document,
                &mark_type,
            ) {
                let range = scalar_range_for_doc(mark_from, mark_to);
                vec![
                    TypedOperation::RemoveMark {
                        range,
                        mark_type: mark_type.clone(),
                    },
                    TypedOperation::AddMark {
                        range,
                        mark: Mark::new(mark_type, attrs),
                    },
                ]
            } else {
                state_only = true;
                vec![TypedOperation::ReplaceMark {
                    range: selection_range,
                    mark: Mark::new(mark_type, attrs),
                }]
            }
        }
        TypedCommand::SetMark { mark_type, attrs } => vec![
            TypedOperation::RemoveMark {
                range: selection_range,
                mark_type: mark_type.clone(),
            },
            TypedOperation::AddMark {
                range: selection_range,
                mark: Mark::new(mark_type, attrs),
            },
        ],
        TypedCommand::UnsetMark { mark_type } if from == to => {
            if let Some((mark_from, mark_to)) = crate::editor_state::mark_range_at_position(
                context.document,
                anchor.document,
                &mark_type,
            ) {
                vec![TypedOperation::RemoveMark {
                    range: scalar_range_for_doc(mark_from, mark_to),
                    mark_type,
                }]
            } else {
                state_only = true;
                vec![TypedOperation::RemoveMark {
                    range: selection_range,
                    mark_type,
                }]
            }
        }
        TypedCommand::UnsetMark { mark_type } => vec![TypedOperation::RemoveMark {
            range: selection_range,
            mark_type,
        }],
        TypedCommand::ToggleHeading { level } => {
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_toggle_heading(
                context.document,
                context.schema,
                &selection,
                level,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            return super::text::semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: plan.from,
                        to: plan.to,
                        content: plan.content,
                    }],
                    selection_after: Some(plan.selection_after),
                },
            );
        }
        TypedCommand::ToggleCodeBlock => {
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_toggle_code_block(
                context.document,
                context.schema,
                &selection,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            return super::text::semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: plan.from,
                        to: plan.to,
                        content: plan.content,
                    }],
                    selection_after: Some(plan.selection_after),
                },
            );
        }
        TypedCommand::ToggleBlockquote => {
            let selection = crate::yrs_engine::derived_state::resolved_to_legacy(context.selection);
            let Some(plan) = crate::command_planner::plan_toggle_blockquote(
                context.document,
                context.schema,
                &selection,
            ) else {
                return Ok(CommandPlan::NotApplicable);
            };
            return super::text::semantic_transaction(
                &context,
                &selection,
                crate::command_planner::SemanticCommandPlan {
                    operations: vec![crate::command_planner::SemanticOperation::ReplaceRange {
                        from: plan.from,
                        to: plan.to,
                        content: plan.content,
                    }],
                    selection_after: Some(plan.selection_after),
                },
            );
        }
        _ => unreachable!("format planner received non-format command"),
    };
    let transaction = TypedTransaction {
        request_id: context.request_id,
        base_document_revision: context.revision,
        origin: TransactionOrigin::LocalCommand,
        operations,
        selection_intent: if uses_operation_result {
            SelectionIntent::UseOperationResult
        } else {
            SelectionIntent::Preserve
        },
        history_policy: HistoryPolicy::Boundary,
    };
    if state_only {
        Ok(CommandPlan::SelectionOnly(transaction))
    } else {
        Ok(CommandPlan::Transaction(transaction))
    }
}
