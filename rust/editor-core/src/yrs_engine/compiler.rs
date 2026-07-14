use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::serialize::to_prosemirror_json;
use crate::transform::{DocumentValidator, Step, StepMap};

use super::editing_limits::CheckedWork;
use super::mutation::YrsMutationPlan;
use super::{
    editor_offset_to_doc_pos, Affinity, EditingLimits, HistoryPolicy, OperationError,
    OperationResult, RevisionedRange, SelectionIntent, TransactionOrigin, TypedOperation,
    TypedTransaction,
};

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct CompilationContext<'a> {
    pub document: &'a Document,
    pub selection: Option<&'a Selection>,
    pub schema: &'a Schema,
    pub resource_limits: &'a ResourceLimits,
    pub editing_limits: &'a EditingLimits,
    pub document_revision: u64,
    pub max_length: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum HistoryClass {
    Insert,
    Delete,
    Format,
    Structural,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum SelectionPlan {
    Preserve,
    Mapped(Selection),
    Explicit(Selection),
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct CompiledTransaction {
    pub request_id: u64,
    pub origin: TransactionOrigin,
    pub history_policy: HistoryPolicy,
    pub history_class: HistoryClass,
    pub preview: Document,
    pub selection_plan: SelectionPlan,
    pub affected_top_level_blocks: Vec<usize>,
    pub mutation_plan: YrsMutationPlan,
    pub encoded_growth_bound: usize,
    pub undo_units_bound: u64,
}

pub(super) fn compile_transaction(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
) -> OperationResult<CompiledTransaction> {
    let request_id = transaction.request_id;
    if transaction.base_document_revision != context.document_revision {
        return Err(OperationError::revision_mismatch(
            request_id,
            transaction.base_document_revision,
            context.document_revision,
        ));
    }
    if !matches!(
        transaction.origin,
        TransactionOrigin::LocalInput
            | TransactionOrigin::LocalCommand
            | TransactionOrigin::LocalApi
    ) {
        return Err(OperationError::transaction_invalid(
            request_id,
            "origin",
            "typed editing transactions require a local input, command, or API origin",
        ));
    }

    let mut work = CheckedWork::default();
    work.charge_operations(
        request_id,
        transaction.operations.len(),
        context.editing_limits.max_operations_per_transaction,
    )?;

    let base_position_map = PositionMap::build(context.document, context.schema);
    let rendered_text = crate::render::rendered_text(context.document, context.schema);
    let rendered_scalars = rendered_text.chars().count();
    if u32::try_from(rendered_scalars).ok() != Some(base_position_map.total_scalars()) {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "rendered text and base position map have different scalar lengths",
        ));
    }
    let mut preview = context.document.clone();
    let mut composed_map = StepMap::empty();
    let mut operation_result = None;
    let mut input_bytes = 0usize;
    let mut undo_units_bound = 0u64;
    let mut undo_limit_error = None;
    let mut history_class = HistoryClass::Skip;
    let records_history = transaction.history_policy != HistoryPolicy::Skip;

    for (operation_index, operation) in transaction.operations.iter().enumerate() {
        match operation {
            TypedOperation::InsertText { at, text, marks } => {
                if text.is_empty() {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "text",
                        "insert text must not be empty",
                    ));
                }
                validate_operation_marks(request_id, operation_index, marks, context.schema)?;
                charge_input(
                    &mut input_bytes,
                    text.len().saturating_add(mark_set_input_bytes(marks)),
                    request_id,
                    operation_index,
                    context.resource_limits.max_input_bytes,
                )?;
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    &rendered_text,
                    &base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::InsertText {
                    pos,
                    text: text.clone(),
                    marks: marks.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
                    &rendered_text,
                    &base_position_map,
                    context.document,
                    &composed_map,
                )?;
                let step = Step::DeleteRange { from, to };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                operation_result = Some(Selection::cursor(from));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
                charge_input(
                    &mut input_bytes,
                    fragment_input_bytes(content),
                    request_id,
                    operation_index,
                    context.resource_limits.max_input_bytes,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    &rendered_text,
                    &base_position_map,
                    context.document,
                    &composed_map,
                )?;
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
                operation_result = Some(Selection::cursor(from.saturating_add(content.size())));
                composed_map = composed_map.compose(&step_map);
                preview = next;
                if records_history {
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
                history_class = merge_history_class(history_class, class);
            }
            TypedOperation::AddMark { range, mark } => {
                validate_operation_marks(
                    request_id,
                    operation_index,
                    std::slice::from_ref(mark),
                    context.schema,
                )?;
                charge_input(
                    &mut input_bytes,
                    mark_input_bytes(mark),
                    request_id,
                    operation_index,
                    context.resource_limits.max_input_bytes,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    &rendered_text,
                    &base_position_map,
                    context.document,
                    &composed_map,
                )?;
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
                let operation_changed = next != preview;
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
                charge_input(
                    &mut input_bytes,
                    mark_type.len(),
                    request_id,
                    operation_index,
                    context.resource_limits.max_input_bytes,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    &rendered_text,
                    &base_position_map,
                    context.document,
                    &composed_map,
                )?;
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
                let operation_changed = next != preview;
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
                charge_input(
                    &mut input_bytes,
                    mark_input_bytes(mark),
                    request_id,
                    operation_index,
                    context.resource_limits.max_input_bytes,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    &rendered_text,
                    &base_position_map,
                    context.document,
                    &composed_map,
                )?;
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
                let operation_changed = next != preview;
                let step_map = remove_map.compose(&add_map);
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
            _ => {
                return Err(OperationError::operation_invalid(
                    request_id,
                    operation_index,
                    "operation",
                    "operation is not supported by the text and mark compiler",
                ));
            }
        }
        validate_preview_marks(request_id, operation_index, &preview, context.schema)?;
        charge_preview_output(&mut work, request_id, operation_index, &preview, context)?;
    }

    validate_preview(
        request_id,
        transaction.operations.len().checked_sub(1),
        &preview,
        context,
    )?;
    let selection_plan = selection_plan(
        context,
        &transaction.selection_intent,
        &rendered_text,
        &base_position_map,
        &composed_map,
        operation_result,
        request_id,
        &preview,
    )?;
    let affected_top_level_blocks = affected_top_level_blocks(context.document, &preview);
    if preview == *context.document || transaction.history_policy == HistoryPolicy::Skip {
        history_class = HistoryClass::Skip;
        undo_units_bound = 0;
    } else if let Some(error) = undo_limit_error {
        return Err(error);
    }

    Ok(CompiledTransaction {
        request_id,
        origin: transaction.origin,
        history_policy: transaction.history_policy,
        history_class,
        preview,
        selection_plan,
        affected_top_level_blocks,
        mutation_plan: YrsMutationPlan::default(),
        // Task 5 introduces executable actions and their conservative update-v1 estimator.
        // With an intentionally empty action vocabulary there is no executable growth yet.
        encoded_growth_bound: 0,
        undo_units_bound,
    })
}

fn resolve_position(
    request_id: u64,
    operation_index: Option<usize>,
    field: &'static str,
    position: super::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
) -> OperationResult<u32> {
    editor_offset_to_doc_pos(
        position.offset,
        position.kind,
        rendered_text,
        position_map,
        document,
    )
    .ok_or_else(|| {
        let message = format!("{field} is outside the base document");
        match operation_index {
            Some(operation_index) => {
                OperationError::position_invalid(request_id, operation_index, field, message)
            }
            None => OperationError::selection_position_invalid(request_id, field, message),
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_range(
    request_id: u64,
    operation_index: usize,
    range: RevisionedRange,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
    composed_map: &StepMap,
) -> OperationResult<(u32, u32)> {
    let base_from = resolve_position(
        request_id,
        Some(operation_index),
        "range.from",
        range.from,
        rendered_text,
        position_map,
        document,
    )?;
    let base_to = resolve_position(
        request_id,
        Some(operation_index),
        "range.to",
        range.to,
        rendered_text,
        position_map,
        document,
    )?;
    if base_from > base_to {
        return Err(OperationError::operation_invalid(
            request_id,
            operation_index,
            "range",
            "range.from must not be greater than range.to",
        ));
    }
    Ok((
        map_position(composed_map, base_from, range.from.affinity),
        map_position(composed_map, base_to, range.to.affinity),
    ))
}

fn map_position(map: &StepMap, mut position: u32, affinity: Affinity) -> u32 {
    for &(range_position, deleted, inserted) in map.ranges() {
        if position < range_position {
            continue;
        }
        if position == range_position {
            if inserted > 0 && affinity == Affinity::After {
                position = position.saturating_add(inserted);
            }
        } else if position <= range_position.saturating_add(deleted) {
            position = range_position.saturating_add(inserted);
        } else {
            position = position.saturating_sub(deleted).saturating_add(inserted);
        }
    }
    position
}

fn charge_input(
    charged: &mut usize,
    amount: usize,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<()> {
    let actual = charged.checked_add(amount);
    let overflowed = actual.is_none();
    let actual = actual.unwrap_or(usize::MAX);
    if overflowed || actual > limit {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            Some(operation_index),
            "maxInputBytes",
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        ));
    }
    *charged = actual;
    Ok(())
}

fn charge_undo_bound(
    charged: &mut u64,
    pending_error: &mut Option<OperationError>,
    amount: u64,
    request_id: u64,
    operation_index: usize,
    limit: u64,
) {
    let actual = charged.checked_add(amount);
    let overflowed = actual.is_none();
    let actual = actual.unwrap_or(u64::MAX);
    *charged = actual;
    if pending_error.is_none() && (overflowed || actual > limit) {
        *pending_error = Some(OperationError::operation_limit_exceeded(
            request_id,
            Some(operation_index),
            "maxUndoRetainedUnits",
            limit,
            actual,
        ));
    }
}

fn validate_operation_marks(
    request_id: u64,
    operation_index: usize,
    marks: &[Mark],
    schema: &Schema,
) -> OperationResult<()> {
    crate::transform::validate_input_mark_set(marks, schema).map_err(|error| {
        OperationError::operation_invalid(request_id, operation_index, "marks", error.to_string())
    })
}

fn validate_fragment_marks(
    request_id: u64,
    operation_index: usize,
    fragment: &Fragment,
    schema: &Schema,
) -> OperationResult<()> {
    fn visit(
        request_id: u64,
        operation_index: usize,
        node: &Node,
        schema: &Schema,
    ) -> OperationResult<()> {
        validate_operation_marks(request_id, operation_index, node.marks(), schema)?;
        if let Some(content) = node.content() {
            for child in content.iter() {
                visit(request_id, operation_index, child, schema)?;
            }
        }
        Ok(())
    }

    for node in fragment.iter() {
        visit(request_id, operation_index, node, schema)?;
    }
    Ok(())
}

fn validate_preview_marks(
    request_id: u64,
    operation_index: usize,
    preview: &Document,
    schema: &Schema,
) -> OperationResult<()> {
    crate::transform::validate_canonical_marks(preview, schema).map_err(|error| {
        OperationError::document_invalid(
            request_id,
            Some(operation_index),
            "marks",
            error.to_string(),
        )
    })
}

fn mark_input_bytes(mark: &Mark) -> usize {
    mark.mark_type().len().saturating_add(
        serde_json::to_vec(mark.attrs())
            .map(|attrs| attrs.len())
            .unwrap_or(usize::MAX),
    )
}

fn mark_set_input_bytes(marks: &[Mark]) -> usize {
    marks.iter().fold(0usize, |total, mark| {
        total.saturating_add(mark_input_bytes(mark))
    })
}

fn fragment_input_bytes(fragment: &Fragment) -> usize {
    fn node_bytes(node: &Node) -> usize {
        let mut bytes = node
            .node_type()
            .len()
            .saturating_add(mark_set_input_bytes(node.marks()))
            .saturating_add(
                serde_json::to_vec(node.attrs())
                    .map(|attrs| attrs.len())
                    .unwrap_or(usize::MAX),
            );
        if let Some(text) = node.text_str() {
            bytes = bytes.saturating_add(text.len());
        }
        if let Some(content) = node.content() {
            for child in content.iter() {
                bytes = bytes.saturating_add(node_bytes(child));
            }
        }
        bytes
    }

    fragment
        .iter()
        .fold(0usize, |total, node| total.saturating_add(node_bytes(node)))
}

fn charge_preview_output(
    work: &mut CheckedWork,
    request_id: u64,
    operation_index: usize,
    preview: &Document,
    context: CompilationContext<'_>,
) -> OperationResult<()> {
    let bytes = serde_json::to_vec(&to_prosemirror_json(preview, context.schema))
        .map_err(|error| {
            OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                format!("preview serialization failed: {error}"),
            )
        })?
        .len();
    work.charge_output_bytes(
        request_id,
        operation_index,
        bytes,
        context.editing_limits.max_derived_output_bytes,
    )
}

fn validate_preview(
    request_id: u64,
    operation_index: Option<usize>,
    preview: &Document,
    context: CompilationContext<'_>,
) -> OperationResult<()> {
    DocumentValidator::validate(preview, context.schema, context.resource_limits).map_err(
        |error| {
            let field = if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                "document"
            } else {
                "content"
            };
            if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                OperationError::document_limit_exceeded(
                    request_id,
                    operation_index,
                    field,
                    error.limit.unwrap_or(0) as u64,
                    error.actual.unwrap_or(0) as u64,
                )
            } else {
                OperationError::document_invalid(
                    request_id,
                    operation_index,
                    field,
                    error.to_string(),
                )
            }
        },
    )?;
    if let Some(limit) = context.max_length {
        let actual = preview.root().text_content().chars().count() as u64;
        if actual > limit as u64 {
            return Err(OperationError::document_limit_exceeded(
                request_id,
                operation_index,
                "maxLength",
                limit as u64,
                actual,
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn selection_plan(
    context: CompilationContext<'_>,
    intent: &SelectionIntent,
    rendered_text: &str,
    base_position_map: &PositionMap,
    composed_map: &StepMap,
    operation_result: Option<Selection>,
    request_id: u64,
    preview: &Document,
) -> OperationResult<SelectionPlan> {
    let preview_map = PositionMap::build(preview, context.schema);
    let candidate = match intent {
        SelectionIntent::Preserve => context.selection.map(|selection| {
            selection
                .map(composed_map)
                .normalized(preview, &preview_map)
        }),
        SelectionIntent::UseOperationResult => {
            operation_result.map(|selection| selection.normalized(preview, &preview_map))
        }
        SelectionIntent::Set(input) => Some(
            match input {
                super::SelectionInput::Text { anchor, head } => Selection::text(
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
                super::SelectionInput::Node { at } => Selection::node(map_position(
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
                super::SelectionInput::All => Selection::all(),
            }
            .normalized(preview, &preview_map),
        ),
    };

    match (intent, candidate) {
        (_, None) => Ok(SelectionPlan::Preserve),
        (_, Some(candidate)) if context.selection == Some(&candidate) => {
            Ok(SelectionPlan::Preserve)
        }
        (SelectionIntent::Preserve, Some(candidate)) => Ok(SelectionPlan::Mapped(candidate)),
        (_, Some(candidate)) => Ok(SelectionPlan::Explicit(candidate)),
    }
}

fn affected_top_level_blocks(before: &Document, after: &Document) -> Vec<usize> {
    if before == after {
        return Vec::new();
    }
    let before_children = before
        .root()
        .content()
        .map(|content| content.children())
        .unwrap_or(&[]);
    let after_children = after
        .root()
        .content()
        .map(|content| content.children())
        .unwrap_or(&[]);
    let mut prefix = 0usize;
    while prefix < before_children.len()
        && prefix < after_children.len()
        && before_children[prefix] == after_children[prefix]
    {
        prefix += 1;
    }
    let start = prefix.saturating_sub(1);
    let end = before_children.len().max(after_children.len());
    (start..end).collect()
}

fn merge_history_class(current: HistoryClass, next: HistoryClass) -> HistoryClass {
    match (current, next) {
        (HistoryClass::Skip, next) => next,
        (current, next) if current == next => current,
        (HistoryClass::Structural, _) | (_, HistoryClass::Structural) => HistoryClass::Structural,
        _ => HistoryClass::Structural,
    }
}

fn map_transform_error(
    request_id: u64,
    operation_index: usize,
    field: &'static str,
    error: crate::transform::TransformError,
) -> OperationError {
    match error {
        crate::transform::TransformError::OutOfBounds(message)
        | crate::transform::TransformError::InvalidTarget(message) => {
            OperationError::position_invalid(request_id, operation_index, field, message)
        }
        crate::transform::TransformError::InvalidRange(message) => {
            OperationError::operation_invalid(request_id, operation_index, "range", message)
        }
        crate::transform::TransformError::ContentViolation(message) => {
            OperationError::document_invalid(request_id, Some(operation_index), "content", message)
        }
        crate::transform::TransformError::NotImplemented(message) => {
            OperationError::operation_invalid(request_id, operation_index, "operation", message)
        }
    }
}

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;
