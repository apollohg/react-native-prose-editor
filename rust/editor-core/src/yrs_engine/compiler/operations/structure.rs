use crate::schema::content_rule::WorkBudget;
use crate::selection::Selection;
use crate::transform::Step;
use crate::yrs_engine;
use crate::yrs_engine::compiler::input_limits::{
    charge_undo_bound, checked_attrs_input_bytes, validate_fragment_marks,
};
use crate::yrs_engine::compiler::insert_position::{
    normalize_current_insert_node_position, resolve_insert_node_position, InsertNodeResolverContext,
};
use crate::yrs_engine::compiler::operations::{OperationCompiler, OperationOutcome};
use crate::yrs_engine::compiler::positions::{
    map_position, resolve_attribute_target_position, resolve_join_target_position,
    resolve_position, resolve_range, resolve_structural_window,
};
use crate::yrs_engine::compiler::preview::{prepared_candidate_matches, validate_preview};
use crate::yrs_engine::compiler::selection::selectable_void_at;
use crate::yrs_engine::compiler::text_boundaries::text_boundaries;
use crate::yrs_engine::compiler::{
    map_transform_error, merge_history_class, HistoryClass, MutationLookupTransition,
};
use crate::yrs_engine::mutation::{MutationCompiler, MutationDocumentContext, ReplacementInput};
use crate::yrs_engine::{OperationError, OperationResult, TypedOperation};
use std::borrow::Cow;

impl OperationCompiler<'_> {
    #[inline]
    pub(super) fn compile_structure(
        self,
        operation_index: usize,
        operation: &TypedOperation,
    ) -> OperationResult<(Self, OperationOutcome)> {
        let Self {
            context,
            transaction,
            prepared_semantics,
            localized_semantic,
            mut lowering,
            localized_insert,
            localized_format,
            mut localized_root_window,
            mut prelowered_plan,
            mut prelowered_lookup_transition,
            request_id,
            work,
            base_position_map,
            rendered_text,
            mut preview,
            mut composed_map,
            operation_result: _previous_operation_result,
            mut undo_units_bound,
            mut undo_limit_error,
            mut history_class,
            records_history,
            canonical_artifact,
            canonical_schema,
            stored_marks_state,
            split_at_caret_kept_stored_marks,
            tracked_caret,
            localized_derivations,
        } = self;
        let mut stored_marks_input = None;
        let mut inherited_marks = None;
        let mut operation_changed;
        let compatible_text_delete = false;
        let operation_result;
        match operation {
            TypedOperation::ReplaceStructure(replacement) => {
                if transaction.operations.len() != 1 {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "structure",
                        "sealed structural replacement must be the transaction's only operation",
                    ));
                }
                validate_fragment_marks(
                    request_id,
                    operation_index,
                    replacement.content(),
                    context.schema,
                )?;
                let (from, to) = resolve_structural_window(
                    request_id,
                    operation_index,
                    &preview,
                    replacement,
                    context.resource_limits,
                )?;
                stored_marks_input = Some((from, to));
                let step = Step::ReplaceRange {
                    from,
                    to,
                    content: replacement.content().clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "structure", error)
                    })?;
                if lowering.is_some()
                    && !prepared_candidate_matches(
                        prepared_semantics,
                        transaction.operations.len(),
                        operation_index,
                        &next,
                        context,
                        canonical_schema,
                    )
                {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            lowering,
                        )?;
                        lowering.replace_structural_range(
                            operation_index,
                            MutationDocumentContext {
                                before: &preview,
                                after: &next,
                                schema: context.schema,
                                limits: context.resource_limits,
                            },
                            ReplacementInput {
                                from,
                                to,
                                boundaries: &boundaries,
                                content: replacement.content(),
                            },
                        )?;
                    } else if localized_root_window.is_some() {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            localized_root_window
                                .as_mut()
                                .expect("localized root-window compiler was checked"),
                        )?;
                        let plan = localized_root_window
                            .take()
                            .expect("localized root-window compiler was checked")
                            .replace_structural_range(
                                operation_index,
                                MutationDocumentContext {
                                    before: &preview,
                                    after: &next,
                                    schema: context.schema,
                                    limits: context.resource_limits,
                                },
                                ReplacementInput {
                                    from,
                                    to,
                                    boundaries: &boundaries,
                                    content: replacement.content(),
                                },
                            )?;
                        prelowered_plan = Some(plan);
                        prelowered_lookup_transition =
                            Some(MutationLookupTransition::Invalidate { request_id });
                    }
                }
                operation_result = Some(replacement.selection_after().clone());
                composed_map = composed_map.compose(&step_map);
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from)
                            .saturating_add(u64::from(replacement.content().size())),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            TypedOperation::InsertNode { at, node } => {
                let resolver_limit = lowering.as_ref().map_or(
                    context.resource_limits.max_input_bytes,
                    MutationCompiler::remaining_scan_work,
                );
                let resolver_budget = WorkBudget::new(resolver_limit);
                let base_pos = resolve_insert_node_position(
                    InsertNodeResolverContext {
                        request_id,
                        operation_index,
                        document: context.document,
                        node,
                        schema: context.schema,
                        budget: &resolver_budget,
                        limit: context.resource_limits.max_input_bytes,
                    },
                    *at,
                    rendered_text,
                    base_position_map,
                )?;
                let mapped_pos = map_position(&composed_map, base_pos, at.affinity);
                let pos = normalize_current_insert_node_position(
                    InsertNodeResolverContext {
                        request_id,
                        operation_index,
                        document: &preview,
                        node,
                        schema: context.schema,
                        budget: &resolver_budget,
                        limit: context.resource_limits.max_input_bytes,
                    },
                    mapped_pos,
                    at.affinity,
                )?;
                if let Some(lowering) = &mut lowering {
                    lowering.charge_position_resolver_work(
                        operation_index,
                        resolver_budget.consumed(resolver_limit),
                    )?;
                }
                let step = Step::InsertNode {
                    pos,
                    node: node.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.insert_structural_node(
                        operation_index,
                        MutationDocumentContext {
                            before: &preview,
                            after: &next,
                            schema: context.schema,
                            limits: context.resource_limits,
                        },
                        pos,
                        node,
                    )?;
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(node.node_size()),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::SplitBlock {
                at,
                node_type,
                attrs,
            } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                // Captured against the pre-split document: these are the marks
                // the next typed character would have inherited had Return not
                // been pressed.
                inherited_marks = Some(yrs_engine::derived_state::marks_at_position(&preview, pos));
                let step = Step::SplitBlock {
                    pos,
                    node_type: node_type.clone(),
                    attrs: attrs.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.split_block(
                        operation_index,
                        &preview,
                        &next,
                        pos,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                stored_marks_input = Some((pos, pos));
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        2,
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::JoinBlocks { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let pos = resolve_join_target_position(request_id, operation_index, &preview, pos)?;
                let step = Step::JoinBlocks { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.join_blocks(
                        operation_index,
                        &preview,
                        pos,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::WrapInList {
                range,
                list_type,
                item_type,
                attrs,
                item_attrs,
            } => {
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                let step = Step::WrapInList {
                    from,
                    to,
                    list_type: list_type.clone(),
                    item_type: item_type.clone(),
                    attrs: attrs.clone(),
                    item_attrs: item_attrs.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.wrap_in_list(
                        operation_index,
                        &preview,
                        &next,
                        from,
                        to,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                operation_result = Some(Selection::text(from, step_map.map_pos(to)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::UnwrapFromList { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::UnwrapFromList { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.unwrap_from_list(
                        operation_index,
                        &preview,
                        &next,
                        pos,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::IndentListItem { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::IndentListItem { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        lowering.indent_list_item(
                            operation_index,
                            &preview,
                            &next,
                            pos,
                            context.schema,
                            context.resource_limits,
                        )?;
                    }
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            TypedOperation::OutdentListItem { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::OutdentListItem { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        lowering.outdent_list_item(
                            operation_index,
                            &preview,
                            &next,
                            pos,
                            context.schema,
                            context.resource_limits,
                        )?;
                    }
                }
                let mapped_pos = crate::command_planner::outdented_list_item_position(
                    &preview,
                    &next,
                    pos,
                    context.schema,
                )
                .unwrap_or_else(|| step_map.map_pos(pos));
                operation_result = Some(Selection::cursor(mapped_pos));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            TypedOperation::UpdateNodeAttrs { at, attrs } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let pos = resolve_attribute_target_position(
                    request_id,
                    operation_index,
                    &preview,
                    pos,
                    attrs,
                    context.schema,
                )?;
                let step = Step::UpdateNodeAttrs {
                    pos,
                    attrs: attrs.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        lowering.update_node_attrs(
                            operation_index,
                            &preview,
                            pos,
                            attrs,
                            context.schema,
                            context.resource_limits,
                        )?;
                    }
                }
                operation_result = selectable_void_at(preview.root(), pos, 0, context.schema)
                    .then(|| Selection::node(pos));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::try_from(checked_attrs_input_bytes(
                            request_id,
                            operation_index,
                            attrs,
                            context.resource_limits,
                            0,
                        )?)
                        .unwrap_or(u64::MAX),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            _ => unreachable!("operation was dispatched to its matching compiler"),
        }
        Ok((
            Self {
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
            },
            OperationOutcome {
                stored_marks_input,
                inherited_marks,
                operation_changed,
                compatible_text_delete,
            },
        ))
    }
}
