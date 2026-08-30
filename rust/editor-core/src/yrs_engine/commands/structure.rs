use super::{CommandPlan, PlanningContext, TypedCommand};
use crate::yrs_engine::{OperationError, OperationResult, RevisionedPosition};

fn selection(context: &PlanningContext<'_>) -> crate::selection::Selection {
    match context.selection {
        crate::yrs_engine::ResolvedSelection::Text { anchor, head } => {
            crate::selection::Selection::text(anchor.document, head.document)
        }
        crate::yrs_engine::ResolvedSelection::Node { at } => {
            crate::selection::Selection::node(at.document)
        }
        crate::yrs_engine::ResolvedSelection::All => crate::selection::Selection::all(),
    }
}

fn explicit_doc_position(
    context: &PlanningContext<'_>,
    position: RevisionedPosition,
) -> OperationResult<u32> {
    let rendered = crate::render::rendered_text(context.document, context.schema);
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
            "at",
            "at is outside the rendered document",
        )
    })?;
    Ok(context.position_map.scalar_to_doc(scalar, context.document))
}

pub(super) fn plan(
    context: PlanningContext<'_>,
    command: TypedCommand,
) -> OperationResult<CommandPlan> {
    let selection = selection(&context);
    let semantic = match command {
        TypedCommand::ApplyListType { list_type } => crate::command_planner::plan_apply_list_type(
            context.document,
            context.schema,
            &selection,
            &list_type,
            context.resource_limits,
        ),
        TypedCommand::WrapInList {
            list_type,
            item_type,
        } => {
            let admitted = crate::command_planner::plan_wrap_in_list_admitted(
                context.document,
                context.schema,
                &selection,
                &list_type,
                &item_type,
                context.resource_limits,
            );
            return match admitted {
                Some(admitted) => {
                    super::text::admitted_semantic_transaction(&context, &selection, admitted)
                }
                None => Ok(CommandPlan::NotApplicable),
            };
        }
        TypedCommand::UnwrapFromList => crate::command_planner::plan_unwrap_from_list(
            context.document,
            context.schema,
            &selection,
            context.resource_limits,
        ),
        TypedCommand::IndentListItem => crate::command_planner::plan_indent_list_item(
            context.document,
            context.schema,
            &selection,
            context.resource_limits,
        ),
        TypedCommand::OutdentListItem => crate::command_planner::plan_outdent_list_item(
            context.document,
            context.schema,
            &selection,
            context.resource_limits,
        ),
        TypedCommand::ToggleTaskItemChecked => {
            crate::command_planner::plan_toggle_task_item_checked(
                context.document,
                context.schema,
                &selection,
                context.resource_limits,
            )
        }
        TypedCommand::InsertNode { node_type } => crate::command_planner::plan_insert_node(
            context.document,
            context.schema,
            &selection,
            &node_type,
            context.resource_limits,
        ),
        TypedCommand::UpdateNodeAttrs { doc_pos, attrs } => {
            crate::command_planner::plan_update_node_attrs(
                context.document,
                context.position_map,
                context.schema,
                &selection,
                doc_pos,
                attrs,
                context.resource_limits,
            )
        }
        TypedCommand::ResizeImage { at, width, height } => {
            let doc_position = explicit_doc_position(&context, at)?;
            crate::command_planner::plan_resize_image(
                context.document,
                context.position_map,
                context.schema,
                &selection,
                crate::command_planner::ResizeImageRequest {
                    doc_position,
                    width,
                    height,
                },
                context.resource_limits,
            )
        }
        _ => unreachable!("structural planner received non-structural command"),
    };
    match semantic {
        Some(plan) => super::text::semantic_transaction(&context, &selection, plan),
        None => Ok(CommandPlan::NotApplicable),
    }
}
