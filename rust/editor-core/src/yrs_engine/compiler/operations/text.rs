use crate::selection::Selection;
use crate::transform::Step;
use crate::yrs_engine::compiler::input_limits::{
    charge_undo_bound, validate_fragment_marks, validate_operation_marks,
};
use crate::yrs_engine::compiler::operations::{OperationCompiler, OperationOutcome};
use crate::yrs_engine::compiler::positions::{map_position, resolve_position, resolve_range};
use crate::yrs_engine::compiler::preview::{validate_preview, LocalizedSemanticCompilation};
use crate::yrs_engine::compiler::text_boundaries::text_boundaries;
use crate::yrs_engine::compiler::{
    map_transform_error, merge_history_class, HistoryClass, MutationLookupTransition,
};
use crate::yrs_engine::mutation::{
    MutationDocumentContext, ReplacementInput, TextRangeDisposition,
};
use crate::yrs_engine::{OperationResult, TypedOperation};
use std::borrow::Cow;

impl OperationCompiler<'_> {
    #[inline]
    pub(super) fn compile_text(
        self,
        operation_index: usize,
        operation: &TypedOperation,
    ) -> OperationResult<(Self, OperationOutcome)> {
        let Self {
            context,
            transaction,
            prepared_semantics,
            mut localized_semantic,
            mut lowering,
            mut localized_insert,
            localized_format,
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
            mut localized_derivations,
        } = self;
        let stored_marks_input;
        let inherited_marks = None;
        let mut operation_changed;
        let mut compatible_text_delete = false;
        let operation_result;
        match operation {
            TypedOperation::InsertText { at, text, marks } => {
                let localized = localized_semantic.take();
                let (pos, next, step_map) = if let Some(localized) = localized {
                    let LocalizedSemanticCompilation {
                        position,
                        preview,
                        step_map,
                        derivations,
                    } = localized;
                    localized_derivations = Some(derivations);
                    (position, preview, step_map)
                } else {
                    validate_operation_marks(request_id, operation_index, marks, context.schema)?;
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
                    let step = Step::InsertText {
                        pos,
                        text: text.clone(),
                        marks: marks.clone(),
                    };
                    let (next, step_map) = crate::transform::apply_step_canonical_marks(
                        &preview,
                        &step,
                        context.schema,
                    )
                    .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                    if lowering.is_some() || localized_insert.is_some() {
                        validate_preview(request_id, Some(operation_index), &next, context)?;
                    }
                    (pos, next, step_map)
                };
                stored_marks_input = Some((pos, pos));
                if let Some(lowering) = &mut lowering {
                    lowering.insert(operation_index, pos, text, marks)?;
                } else if let Some(localized) = localized_insert.take() {
                    let (plan, promotion) =
                        localized.compile_with_promotion(operation_index, pos, text, marks)?;
                    prelowered_plan = Some(plan);
                    prelowered_lookup_transition =
                        Some(MutationLookupTransition::Promote(promotion));
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        text.chars().count() as u64,
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Insert);
            }
            TypedOperation::DeleteRange { range } => {
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
                let step = Step::DeleteRange { from, to };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    let boundaries = text_boundaries(
                        request_id,
                        operation_index,
                        &preview,
                        context.schema,
                        lowering,
                    )?;
                    match lowering.delete(operation_index, from, to, &boundaries)? {
                        TextRangeDisposition::Applied => compatible_text_delete = true,
                        TextRangeDisposition::Structural => {
                            lowering.delete_structural_range(
                                operation_index,
                                &preview,
                                from,
                                to,
                            )?;
                        }
                    }
                }
                operation_result = Some(Selection::cursor(from));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Delete);
            }
            TypedOperation::ReplaceRange { range, content } => {
                validate_fragment_marks(request_id, operation_index, content, context.schema)?;
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
                let step = Step::ReplaceRange {
                    from,
                    to,
                    content: content.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
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
                        lowering.replace(
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
                                content,
                            },
                        )?;
                    }
                }
                operation_result = Some(Selection::cursor(from.saturating_add(content.size())));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from).saturating_add(u64::from(content.size())),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                let class = if content.size() == 0 {
                    HistoryClass::Delete
                } else if from == to {
                    HistoryClass::Insert
                } else {
                    HistoryClass::Structural
                };
                if operation_changed {
                    history_class = merge_history_class(history_class, class);
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
