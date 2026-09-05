use crate::selection::Selection;
use crate::transform::Step;
use crate::yrs_engine;
use crate::yrs_engine::compiler::input_limits::{
    add_mark_conflicts_with_existing_attrs, charge_undo_bound, validate_operation_marks,
};
use crate::yrs_engine::compiler::operations::{OperationCompiler, OperationOutcome};
use crate::yrs_engine::compiler::positions::resolve_range;
use crate::yrs_engine::compiler::preview::{prepared_candidate_matches, validate_preview};
use crate::yrs_engine::compiler::text_boundaries::text_boundaries;
use crate::yrs_engine::compiler::{
    map_transform_error, merge_history_class, HistoryClass, MutationLookupTransition,
};
use crate::yrs_engine::mutation::{mark_attr, removed_mark_attr};
use crate::yrs_engine::{OperationError, OperationResult, TypedOperation};
use std::borrow::Cow;

impl OperationCompiler<'_> {
    #[inline]
    pub(super) fn compile_marks(
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
            mut localized_format,
            localized_root_window,
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
        let stored_marks_input;
        let mut inherited_marks = None;
        let mut operation_changed;
        let compatible_text_delete = false;
        let operation_result;
        match operation {
            TypedOperation::AddMark { range, mark } => {
                validate_operation_marks(
                    request_id,
                    operation_index,
                    std::slice::from_ref(mark),
                    context.schema,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                if from == to {
                    inherited_marks =
                        Some(yrs_engine::derived_state::marks_at_position(&preview, from));
                }
                if add_mark_conflicts_with_existing_attrs(&preview, from, to, mark) {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "mark",
                        "AddMark conflicts with an existing same-type mark; use ReplaceMark",
                    ));
                }
                let step = Step::AddMark {
                    from,
                    to,
                    mark: mark.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
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
                        lowering.format(operation_index, from, to, &boundaries, mark_attr(mark))?;
                    } else if localized_format.is_some() {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            localized_format
                                .as_mut()
                                .expect("localized format capability is present"),
                        )?;
                        let (plan, promotion) = localized_format
                            .take()
                            .expect("localized format capability is present")
                            .format(operation_index, from, to, &boundaries, mark_attr(mark))?;
                        prelowered_plan = Some(plan);
                        prelowered_lookup_transition =
                            Some(MutationLookupTransition::Promote(promotion));
                    }
                }
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Format);
            }
            TypedOperation::RemoveMark { range, mark_type } => {
                if context.schema.mark(mark_type).is_none() {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "markType",
                        format!("unknown mark '{mark_type}'"),
                    ));
                }
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                if from == to {
                    inherited_marks =
                        Some(yrs_engine::derived_state::marks_at_position(&preview, from));
                }
                let step = Step::RemoveMark {
                    from,
                    to,
                    mark_type: mark_type.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
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
                        lowering.format(
                            operation_index,
                            from,
                            to,
                            &boundaries,
                            removed_mark_attr(mark_type),
                        )?;
                    } else if localized_format.is_some() {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            localized_format
                                .as_mut()
                                .expect("localized format capability is present"),
                        )?;
                        let (plan, promotion) = localized_format
                            .take()
                            .expect("localized format capability is present")
                            .format(
                                operation_index,
                                from,
                                to,
                                &boundaries,
                                removed_mark_attr(mark_type),
                            )?;
                        prelowered_plan = Some(plan);
                        prelowered_lookup_transition =
                            Some(MutationLookupTransition::Promote(promotion));
                    }
                }
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Format);
            }
            TypedOperation::ReplaceMark { range, mark } => {
                validate_operation_marks(
                    request_id,
                    operation_index,
                    std::slice::from_ref(mark),
                    context.schema,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                if from == to {
                    inherited_marks =
                        Some(yrs_engine::derived_state::marks_at_position(&preview, from));
                }
                let remove = Step::RemoveMark {
                    from,
                    to,
                    mark_type: mark.mark_type().to_string(),
                };
                let (without, remove_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &remove, context.schema)
                        .map_err(|error| {
                            map_transform_error(request_id, operation_index, "range", error)
                        })?;
                let add = Step::AddMark {
                    from,
                    to,
                    mark: mark.clone(),
                };
                let (next, add_map) =
                    crate::transform::apply_step_canonical_marks(&without, &add, context.schema)
                        .map_err(|error| {
                            map_transform_error(request_id, operation_index, "range", error)
                        })?;
                if lowering.is_some() {
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
                        lowering.format(operation_index, from, to, &boundaries, mark_attr(mark))?;
                    }
                }
                let step_map = remove_map.compose(&add_map);
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    for _ in 0..2 {
                        charge_undo_bound(
                            &mut undo_units_bound,
                            &mut undo_limit_error,
                            u64::from(to - from),
                            request_id,
                            operation_index,
                            context.editing_limits.max_undo_retained_units,
                        );
                    }
                }
                history_class = merge_history_class(history_class, HistoryClass::Format);
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
