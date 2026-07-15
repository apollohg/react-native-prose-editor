use super::{CommandPlan, PlanningContext, TypedCommand};
use crate::boundary::{BoundedInput, InputKind};
use crate::serialize::{FromHtmlOptions, UnknownTypeMode};
use crate::yrs_engine::{
    Affinity, EditorOffsetKind, HistoryPolicy, OperationError, OperationResult, RevisionedPosition,
    RevisionedRange, SelectionIntent, StructuralReplacement, TransactionOrigin, TypedOperation,
    TypedTransaction,
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

fn transaction(context: &PlanningContext<'_>, operations: Vec<TypedOperation>) -> CommandPlan {
    CommandPlan::Transaction(TypedTransaction {
        request_id: context.request_id,
        base_document_revision: context.revision,
        origin: TransactionOrigin::LocalCommand,
        operations,
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Boundary,
    })
}

pub(super) fn semantic_transaction(
    context: &PlanningContext<'_>,
    selection: &crate::selection::Selection,
    plan: crate::command_planner::SemanticCommandPlan,
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
    let simulated = crate::command_planner::simulate_plan(
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
    })?;
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
    if plan.operations.len() == 1
        && direct_selection_compatible(&plan.operations[0], plan.selection_after.as_ref())
    {
        if let Some(operation) = direct_typed_operation(context, &plan.operations[0]) {
            return Ok(transaction(context, vec![operation]));
        }
    }
    let diff = crate::command_planner::structural_diff_bounded(
        context.document,
        &simulated.document,
        context.resource_limits,
    )
    .map_err(|()| {
        OperationError::operation_resource_exhausted(
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
        &simulated.document,
        &diff,
        context.schema,
        context.resource_limits,
    )
    .map_err(|()| {
        OperationError::operation_resource_exhausted(
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
    Ok(CommandPlan::Transaction(TypedTransaction {
        request_id: context.request_id,
        base_document_revision: context.revision,
        origin: TransactionOrigin::LocalCommand,
        operations: vec![TypedOperation::ReplaceStructure(
            StructuralReplacement::new(
                diff.parent_path,
                diff.from_child,
                diff.to_child,
                diff.content,
                simulated.selection,
            ),
        )],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Boundary,
    }))
}

fn direct_selection_compatible(
    operation: &crate::command_planner::SemanticOperation,
    selection_after: Option<&crate::selection::Selection>,
) -> bool {
    match selection_after {
        None => true,
        Some(crate::selection::Selection::Text { anchor, head }) if anchor == head => {
            let expected = match operation {
                crate::command_planner::SemanticOperation::ReplaceRange {
                    from, content, ..
                } => from.checked_add(content.size()),
                _ => None,
            };
            expected == Some(*anchor)
        }
        _ => false,
    }
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
                let document = context.position_map.scalar_to_doc(scalar, context.document);
                Ok((scalar, document, position.affinity))
            };
            let from = resolve(requested.from, "range.from")?;
            let to = resolve(requested.to, "range.to")?;
            if from.0 == to.0 {
                return Ok(CommandPlan::NotApplicable);
            }
            let (mut from, mut to) = if from.0 < to.0 {
                (from, to)
            } else {
                (to, from)
            };
            if from.0 > 0
                && context
                    .document
                    .resolve(from.1)
                    .is_ok_and(|resolved| resolved.node_path.is_empty())
            {
                from.1 = from.1.saturating_add(1);
                to.1 = to.1.saturating_add(1);
            }
            if from.1 != context.position_map.scalar_to_doc(from.0, context.document)
                || to.1 != context.position_map.scalar_to_doc(to.0, context.document)
            {
                let selection = crate::selection::Selection::text(from.1, to.1);
                return semantic_transaction(
                    &context,
                    &selection,
                    crate::command_planner::SemanticCommandPlan {
                        operations: vec![crate::command_planner::SemanticOperation::DeleteRange {
                            from: from.1,
                            to: to.1,
                        }],
                        selection_after: Some(crate::selection::Selection::cursor(from.1)),
                    },
                );
            }
            return Ok(transaction(
                &context,
                vec![TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: from.0,
                            kind: EditorOffsetKind::Scalar,
                            affinity: from.2,
                        },
                        to: RevisionedPosition {
                            offset: to.0,
                            kind: EditorOffsetKind::Scalar,
                            affinity: to.2,
                        },
                    },
                }],
            ));
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
                OperationError::operation_resource_exhausted(
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
                OperationError::operation_resource_exhausted(
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
                },
            )
        }
        _ => unreachable!("text planner received non-text command"),
    }
}
