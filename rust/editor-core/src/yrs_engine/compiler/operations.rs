mod marks;
mod structure;
mod text;

use crate::model::{Document, Mark};
use crate::position::PositionMap;
use crate::selection::Selection;
use crate::transform::StepMap;
use crate::yrs_engine;
use crate::yrs_engine::canonical::{CanonicalArtifact, CanonicalSchemaContext};
use crate::yrs_engine::compiler::input_limits::validate_preview_marks;
use crate::yrs_engine::compiler::positions::map_position;
use crate::yrs_engine::compiler::preview::{
    charge_canonical_output, charge_localized_preview_output, charge_prepared_preview_output,
    charge_preview_output, prepared_candidate_matches, LocalizedSemanticCompilation,
    LocalizedSemanticDerivations,
};
use crate::yrs_engine::compiler::{
    CompilationContext, HistoryClass, MutationLookupTransition, PreparedSemanticContext,
};
use crate::yrs_engine::editing_limits::CheckedWork;
use crate::yrs_engine::mutation::{
    LocalizedFormatCompiler, LocalizedInsertCompiler, LocalizedRootWindowCompiler,
    MutationCompiler, YrsMutationPlan,
};
use crate::yrs_engine::{
    Affinity, OperationError, OperationResult, TypedOperation, TypedTransaction,
};
use std::borrow::Cow;

pub(super) struct OperationCompiler<'a> {
    pub(super) context: CompilationContext<'a>,
    pub(super) transaction: &'a TypedTransaction,
    pub(super) prepared_semantics: Option<PreparedSemanticContext<'a>>,
    pub(super) localized_semantic: Option<LocalizedSemanticCompilation>,
    pub(super) lowering: Option<MutationCompiler>,
    pub(super) localized_insert: Option<LocalizedInsertCompiler>,
    pub(super) localized_format: Option<LocalizedFormatCompiler>,
    pub(super) localized_root_window: Option<LocalizedRootWindowCompiler>,
    pub(super) prelowered_plan: Option<YrsMutationPlan>,
    pub(super) prelowered_lookup_transition: Option<MutationLookupTransition>,
    pub(super) request_id: u64,
    pub(super) work: CheckedWork,
    pub(super) base_position_map: &'a PositionMap,
    pub(super) rendered_text: &'a str,
    pub(super) preview: Cow<'a, Document>,
    pub(super) composed_map: StepMap,
    pub(super) operation_result: Option<Selection>,
    pub(super) undo_units_bound: u64,
    pub(super) undo_limit_error: Option<OperationError>,
    pub(super) history_class: HistoryClass,
    pub(super) records_history: bool,
    pub(super) canonical_artifact: Option<CanonicalArtifact>,
    pub(super) canonical_schema: &'a CanonicalSchemaContext,
    pub(super) stored_marks_state: Option<Option<Vec<Mark>>>,
    pub(super) split_at_caret_kept_stored_marks: bool,
    pub(super) tracked_caret: Option<(u32, Affinity)>,
    pub(super) localized_derivations: Option<LocalizedSemanticDerivations>,
}

struct OperationOutcome {
    stored_marks_input: Option<(u32, u32)>,
    inherited_marks: Option<Vec<Mark>>,
    operation_changed: bool,
    compatible_text_delete: bool,
}

impl OperationCompiler<'_> {
    #[inline]
    pub(super) fn compile(
        self,
        operation_index: usize,
        operation: &TypedOperation,
    ) -> OperationResult<Self> {
        let tracked_caret_for_operation = self
            .tracked_caret
            .map(|(position, affinity)| map_position(&self.composed_map, position, affinity));
        let (state, outcome) = match operation {
            TypedOperation::InsertText { .. }
            | TypedOperation::DeleteRange { .. }
            | TypedOperation::ReplaceRange { .. } => {
                self.compile_text(operation_index, operation)?
            }
            TypedOperation::AddMark { .. }
            | TypedOperation::RemoveMark { .. }
            | TypedOperation::ReplaceMark { .. } => {
                self.compile_marks(operation_index, operation)?
            }
            TypedOperation::ReplaceStructure(_)
            | TypedOperation::InsertNode { .. }
            | TypedOperation::SplitBlock { .. }
            | TypedOperation::JoinBlocks { .. }
            | TypedOperation::WrapInList { .. }
            | TypedOperation::UnwrapFromList { .. }
            | TypedOperation::IndentListItem { .. }
            | TypedOperation::OutdentListItem { .. }
            | TypedOperation::UpdateNodeAttrs { .. } => {
                self.compile_structure(operation_index, operation)?
            }
        };
        let Self {
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
            mut work,
            base_position_map,
            rendered_text,
            preview,
            composed_map,
            operation_result,
            undo_units_bound,
            undo_limit_error,
            history_class,
            records_history,
            mut canonical_artifact,
            canonical_schema,
            mut stored_marks_state,
            mut split_at_caret_kept_stored_marks,
            tracked_caret,
            localized_derivations,
        } = state;
        let OperationOutcome {
            stored_marks_input,
            mut inherited_marks,
            operation_changed,
            compatible_text_delete,
        } = outcome;
        if let Some(current) = stored_marks_state.as_mut() {
            let operation_at_caret = tracked_caret_for_operation
                .zip(stored_marks_input)
                .is_some_and(|(caret, (from, to))| from == caret && to == caret);
            let deletion_touches_caret = tracked_caret_for_operation
                .zip(stored_marks_input)
                .is_some_and(|(caret, (from, to))| from == caret || to == caret);
            match operation {
                TypedOperation::AddMark { .. }
                | TypedOperation::RemoveMark { .. }
                | TypedOperation::ReplaceMark { .. }
                    if operation_at_caret =>
                {
                    if let Some(marks) = current.as_mut() {
                        yrs_engine::derived_state::apply_stored_mark_operation(
                            marks,
                            operation,
                            context.schema,
                        )
                        .map_err(|mut error| {
                            error.request_id = request_id;
                            error.operation_index = Some(operation_index);
                            error
                        })?;
                    } else {
                        let mut marks = inherited_marks.take().unwrap_or_default();
                        let changed = yrs_engine::derived_state::apply_stored_mark_operation(
                            &mut marks,
                            operation,
                            context.schema,
                        )
                        .map_err(|mut error| {
                            error.request_id = request_id;
                            error.operation_index = Some(operation_index);
                            error
                        })?;
                        let materializes = match operation {
                            TypedOperation::AddMark { .. } | TypedOperation::ReplaceMark { .. } => {
                                changed
                            }
                            TypedOperation::RemoveMark { .. } => true,
                            _ => unreachable!(),
                        };
                        if materializes {
                            *current = Some(marks);
                        }
                    }
                }
                TypedOperation::InsertText { text, marks, .. }
                    if operation_changed && !text.is_empty() && operation_at_caret =>
                {
                    if let Some(effective) = current.as_ref() {
                        if yrs_engine::derived_state::canonical_marks(marks, context.schema)
                            != *effective
                        {
                            *current = None;
                        }
                    }
                }
                TypedOperation::DeleteRange { .. }
                    if operation_changed && deletion_touches_caret && compatible_text_delete =>
                {
                    // A compatible text deletion carries the current stored set
                    // through to the mapped caret without changing it.
                }
                TypedOperation::SplitBlock { .. } if operation_changed && operation_at_caret => {
                    // Pressing Return carries the active formatting onto the new
                    // block, as every comparable editor does: the marks the next
                    // character would have taken before the split are the marks
                    // it takes after it. A split away from the caret is still a
                    // structural edit and falls through to the clearing arm.
                    //
                    // With no explicit stored set the active formatting is
                    // whatever the text before the caret carries, so it has to be
                    // materialised here — the new block is empty and has nothing
                    // for the next character to inherit from.
                    if current.is_none() {
                        let inherited = inherited_marks.take().unwrap_or_default();
                        if !inherited.is_empty() {
                            *current = Some(yrs_engine::derived_state::canonical_marks(
                                &inherited,
                                context.schema,
                            ));
                        }
                    }
                    split_at_caret_kept_stored_marks = true;
                }
                _ if operation_changed => *current = None,
                _ => {}
            }
        }
        let prepared_candidate_matches_preview = prepared_candidate_matches(
            prepared_semantics,
            transaction.operations.len(),
            operation_index,
            &preview,
            context,
            canonical_schema,
        );
        if localized_derivations.is_none() && !prepared_candidate_matches_preview {
            validate_preview_marks(request_id, operation_index, &preview, context.schema)?;
        }
        let next_artifact = if operation_changed {
            if let Some(prepared) = prepared_semantics.filter(|prepared| {
                transaction.operations.len() == 1
                    && operation_index == 0
                    && *preview == *prepared.expected_preview
            }) {
                charge_prepared_preview_output(
                    &mut work,
                    request_id,
                    operation_index,
                    prepared.admission,
                    context,
                )?
            } else if let Some(localized) = localized_derivations.as_ref() {
                charge_localized_preview_output(
                    &mut work,
                    request_id,
                    operation_index,
                    &preview,
                    canonical_schema,
                    context,
                    localized.raw_text_scalars,
                    localized.raw_text_utf8_bytes,
                )?
            } else {
                charge_preview_output(
                    &mut work,
                    request_id,
                    operation_index,
                    &preview,
                    canonical_schema,
                    context,
                )?
            }
        } else if let Some(existing) = canonical_artifact.as_ref() {
            charge_canonical_output(&mut work, request_id, operation_index, existing, context)?;
            existing.clone()
        } else {
            charge_preview_output(
                &mut work,
                request_id,
                operation_index,
                &preview,
                canonical_schema,
                context,
            )?
        };
        canonical_artifact = Some(next_artifact);
        Ok(Self {
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
        })
    }
}
