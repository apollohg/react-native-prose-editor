use crate::model::{Document, Node};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::StepMap;
use crate::yrs_engine;
use crate::yrs_engine::compiler::positions::{map_position, resolve_position};
use crate::yrs_engine::compiler::{
    base_position_map, base_rendered_text, CachedCompilationView, CompilationContext, SelectionPlan,
};
use crate::yrs_engine::{
    OperationError, OperationResult, SelectionIntent, TypedOperation, TypedTransaction,
};

pub(super) fn planned_relative_selection<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    cached_view: Option<CachedCompilationView<'_>>,
) -> OperationResult<Option<yrs_engine::RelativeSelection>> {
    let owned_rendered;
    let owned_map;
    let (rendered, map) = if let Some(cached) = cached_view {
        (cached.rendered_text, cached.position_map)
    } else {
        owned_rendered = base_rendered_text(context.document, context.schema);
        owned_map = base_position_map(context.document, context.schema);
        (owned_rendered.as_str(), &owned_map)
    };
    let relative_point = |field: &'static str, point: yrs_engine::RevisionedPosition| {
        yrs_engine::revisioned_position_to_relative_point(
            txn,
            fragment,
            point,
            rendered,
            map,
            context.document,
            context.schema,
        )
        .ok_or_else(|| {
            OperationError::selection_position_invalid(
                transaction.request_id,
                field,
                "selection cannot be represented with the requested Yrs affinity",
            )
        })
    };
    let text = |anchor, head| {
        Ok(yrs_engine::RelativeSelection::Text {
            anchor: relative_point("selection.anchor", anchor)?,
            head: relative_point("selection.head", head)?,
        })
    };
    let relative = match &transaction.selection_intent {
        SelectionIntent::Preserve => return Ok(None),
        SelectionIntent::Set(yrs_engine::SelectionInput::Text { anchor, head }) => {
            text(*anchor, *head)?
        }
        SelectionIntent::Set(yrs_engine::SelectionInput::Node { at }) => {
            yrs_engine::RelativeSelection::Node {
                point: relative_point("selection.at", *at)?,
            }
        }
        SelectionIntent::Set(yrs_engine::SelectionInput::All) => yrs_engine::RelativeSelection::All,
        SelectionIntent::UseOperationResult => return Ok(None),
    };
    Ok(Some(relative))
}

pub(super) fn position_update_mode(operations: &[TypedOperation]) -> UpdateMode {
    if operations.iter().all(|operation| {
        matches!(
            operation,
            TypedOperation::AddMark { .. }
                | TypedOperation::RemoveMark { .. }
                | TypedOperation::ReplaceMark { .. }
        )
    }) {
        UpdateMode::MarksOnly
    } else if operations.iter().all(|operation| {
        matches!(
            operation,
            TypedOperation::InsertText { .. } | TypedOperation::DeleteRange { .. }
        )
    }) {
        UpdateMode::InlineTextOnly
    } else {
        UpdateMode::Rebuild
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn selection_plan(
    context: CompilationContext<'_>,
    intent: &SelectionIntent,
    rendered_text: &str,
    base_position_map: &PositionMap,
    composed_map: &StepMap,
    operation_result: Option<Selection>,
    request_id: u64,
    preview: &Document,
    prepared_preview_map: Option<&PositionMap>,
) -> OperationResult<SelectionPlan> {
    let owned_preview_map;
    let preview_map = if let Some(prepared) = prepared_preview_map {
        prepared
    } else {
        yrs_engine::derived_state::record_preview_position_map_derivation();
        owned_preview_map = PositionMap::build(preview, context.schema);
        &owned_preview_map
    };
    let uses_preserved_fallback =
        matches!(intent, SelectionIntent::UseOperationResult) && operation_result.is_none();
    let mut candidate = match intent {
        SelectionIntent::Preserve => context
            .selection
            .map(|selection| selection.map(composed_map).normalized(preview, preview_map)),
        SelectionIntent::UseOperationResult => operation_result
            .or_else(|| {
                context
                    .selection
                    .map(|selection| selection.map(composed_map))
            })
            .map(|selection| selection.normalized(preview, preview_map)),
        SelectionIntent::Set(input) => Some(
            match input {
                yrs_engine::SelectionInput::Text { anchor, head } => Selection::text(
                    map_position(
                        composed_map,
                        resolve_position(
                            request_id,
                            None,
                            "selection.anchor",
                            *anchor,
                            rendered_text,
                            base_position_map,
                            context.document,
                        )?,
                        anchor.affinity,
                    ),
                    map_position(
                        composed_map,
                        resolve_position(
                            request_id,
                            None,
                            "selection.head",
                            *head,
                            rendered_text,
                            base_position_map,
                            context.document,
                        )?,
                        head.affinity,
                    ),
                ),
                yrs_engine::SelectionInput::Node { at } => Selection::node(map_position(
                    composed_map,
                    resolve_position(
                        request_id,
                        None,
                        "selection.at",
                        *at,
                        rendered_text,
                        base_position_map,
                        context.document,
                    )?,
                    at.affinity,
                )),
                yrs_engine::SelectionInput::All => Selection::all(),
            }
            .normalized(preview, preview_map),
        ),
    };

    if let Some(Selection::Node { pos }) = candidate.as_ref() {
        let pos = *pos;
        if !selectable_void_at(preview.root(), pos, 0, context.schema) {
            match intent {
                SelectionIntent::Set(yrs_engine::SelectionInput::Node { .. }) => {
                    return Err(OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                SelectionIntent::Preserve => {
                    candidate = Some(Selection::cursor(pos).normalized(preview, preview_map));
                }
                SelectionIntent::UseOperationResult if uses_preserved_fallback => {
                    candidate = Some(Selection::cursor(pos).normalized(preview, preview_map));
                }
                SelectionIntent::UseOperationResult => {
                    return Err(OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "operation result produced a node selection for a non-selectable node",
                    ));
                }
                SelectionIntent::Set(_) => {
                    return Err(OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "non-node explicit selection compiled to an invalid node selection",
                    ));
                }
            }
        }
    } else if matches!(
        intent,
        SelectionIntent::Set(yrs_engine::SelectionInput::Node { .. })
    ) {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "node selection did not compile to a node selection",
        ));
    }

    match (intent, candidate) {
        (_, None) => Ok(SelectionPlan::Preserve),
        (SelectionIntent::Preserve, Some(_)) if preview == context.document => {
            Ok(SelectionPlan::Preserve)
        }
        (SelectionIntent::Preserve, Some(candidate)) => Ok(SelectionPlan::Mapped(candidate)),
        (SelectionIntent::UseOperationResult, Some(_))
            if uses_preserved_fallback && preview == context.document =>
        {
            Ok(SelectionPlan::Preserve)
        }
        (SelectionIntent::UseOperationResult, Some(candidate)) if uses_preserved_fallback => {
            Ok(SelectionPlan::Mapped(candidate))
        }
        (_, Some(candidate)) => Ok(SelectionPlan::Explicit(candidate)),
    }
}

pub(crate) fn selectable_void_at(
    node: &Node,
    target: u32,
    content_start: u32,
    schema: &Schema,
) -> bool {
    let Some(content) = node.content() else {
        return false;
    };
    let mut offset = content_start;
    for child in content.iter() {
        let selectable = child.is_void()
            || schema
                .node(child.node_type())
                .is_some_and(|spec| spec.is_void);
        if selectable && target == offset {
            return true;
        }
        if child.content().is_some()
            && target > offset
            && target < offset.saturating_add(child.node_size())
            && selectable_void_at(child, target, offset.saturating_add(1), schema)
        {
            return true;
        }
        offset = offset.saturating_add(child.node_size());
    }
    false
}
