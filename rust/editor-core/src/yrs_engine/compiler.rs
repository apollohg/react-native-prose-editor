use crate::boundary::{JsonMeterDimension, JsonMeterError, JsonValueMeter, ResourceLimits};
use crate::model::{Document, Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::schema::content_rule::WorkBudget;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::serialize::to_prosemirror_json;
use crate::transform::{DocumentValidator, Step, StepMap};
use smallvec::SmallVec;

use super::editing_limits::CheckedWork;
use super::mutation::{
    crdt_clock_scan_reservation, crdt_envelope, estimate_undo_units, estimate_update_v1_growth,
    mark_attr, preflight_mutation_plan, removed_mark_attr, CrdtEnvelope, MutationCompiler,
    MutationDocumentContext, ReplacementInput, TextRangeDisposition, YrsMutationPlan,
};
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
    admit_transaction_envelope(context, &transaction)?;
    compile_transaction_impl(context, transaction, None, None)
}

pub(super) fn compile_transaction_with_yrs<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
) -> OperationResult<CompiledTransaction> {
    let request_id = transaction.request_id;
    let admitted_input_bytes = admit_transaction_envelope(context, &transaction)?;
    let action_multiplier = context
        .editing_limits
        .max_operations_per_transaction
        .checked_add(context.resource_limits.max_document_depth)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::MAX,
                u64::MAX,
            )
        })?;
    let action_limit = action_multiplier
        .checked_mul(context.resource_limits.max_document_nodes)
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::MAX,
                u64::MAX,
            )
        })?;
    let document_text_bytes = document_text_bytes(context.document).ok_or_else(|| {
        OperationError::operation_limit_exceeded(
            request_id,
            None,
            "maxInputBytes",
            u64::try_from(context.resource_limits.max_input_bytes).unwrap_or(u64::MAX),
            u64::MAX,
        )
    })?;
    let crdt_clock_work = crdt_clock_scan_reservation(
        request_id,
        txn,
        context.resource_limits.max_encoded_state_bytes,
    )?;
    // One pass materializes each Yrs text and one pass builds its scalar/UTF-16 index.
    // Reserve both before constructing MutationCompiler, so invalid envelopes never
    // traverse Yrs and admitted traversal cannot outrun its transaction-wide budget.
    let initial_scan_work = document_text_bytes
        .checked_mul(2)
        .and_then(|work| work.checked_add(crdt_clock_work.checked_mul(2)?))
        .ok_or_else(|| {
            input_limit_error(
                request_id,
                None,
                context.resource_limits.max_input_bytes,
                usize::MAX,
            )
        })?;
    let charged_scan_work = admitted_input_bytes
        .checked_add(initial_scan_work)
        .ok_or_else(|| {
            input_limit_error(
                request_id,
                None,
                context.resource_limits.max_input_bytes,
                usize::MAX,
            )
        })?;
    if charged_scan_work > context.resource_limits.max_input_bytes {
        return Err(input_limit_error(
            request_id,
            None,
            context.resource_limits.max_input_bytes,
            charged_scan_work,
        ));
    }
    let lowering = MutationCompiler::new(
        request_id,
        txn,
        fragment,
        context.schema,
        action_limit,
        context.resource_limits.max_input_bytes,
        charged_scan_work,
    )?;
    let mut load_crdt_envelope = |mutation_scan_work: usize| {
        let envelope = crdt_envelope(
            request_id,
            txn,
            context.resource_limits.max_encoded_state_bytes,
        )?;
        let reconciled = mutation_scan_work
            .checked_add(envelope.scan_work)
            .ok_or_else(|| {
                input_limit_error(
                    request_id,
                    None,
                    context.resource_limits.max_input_bytes,
                    usize::MAX,
                )
            })?;
        if reconciled > context.resource_limits.max_input_bytes {
            return Err(input_limit_error(
                request_id,
                None,
                context.resource_limits.max_input_bytes,
                reconciled,
            ));
        }
        Ok(envelope)
    };
    let compiled = compile_transaction_impl(
        context,
        transaction,
        Some(lowering),
        Some(&mut load_crdt_envelope),
    )?;
    // The server owns this read view through compilation and preflight. The
    // plan's document guard was captured only after the CRDT clock scan and
    // its input-work reservation above admitted full snapshot construction.
    // Preflight checks that sealed snapshot before any eager Yrs target reads.
    preflight_mutation_plan(request_id, &compiled.mutation_plan, txn)?;
    Ok(compiled)
}

fn admit_transaction_envelope(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
) -> OperationResult<usize> {
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

    let mut input_bytes = 0usize;
    for (operation_index, operation) in transaction.operations.iter().enumerate() {
        let amount = match operation {
            TypedOperation::InsertText { text, marks, .. } => {
                if text.is_empty() {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "text",
                        "insert text must not be empty",
                    ));
                }
                text.len().checked_add(checked_mark_set_input_bytes(
                    request_id,
                    operation_index,
                    marks,
                    context.resource_limits,
                    input_bytes.saturating_add(text.len()),
                )?)
            }
            TypedOperation::DeleteRange { .. } => Some(0),
            TypedOperation::ReplaceRange { content, .. } => Some(checked_fragment_input_bytes(
                request_id,
                operation_index,
                content,
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::AddMark { mark, .. } | TypedOperation::ReplaceMark { mark, .. } => {
                Some(checked_mark_input_bytes(
                    request_id,
                    operation_index,
                    mark,
                    context.resource_limits,
                    input_bytes,
                )?)
            }
            TypedOperation::RemoveMark { mark_type, .. } => Some(mark_type.len()),
            TypedOperation::InsertNode { node, .. } => Some(checked_node_input_bytes(
                request_id,
                operation_index,
                node,
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::UpdateNodeAttrs { attrs, .. } => Some(checked_attrs_input_bytes(
                request_id,
                operation_index,
                attrs,
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::SplitBlock {
                node_type, attrs, ..
            } => node_type.len().checked_add(checked_attrs_input_bytes(
                request_id,
                operation_index,
                attrs,
                context.resource_limits,
                input_bytes.saturating_add(node_type.len()),
            )?),
            TypedOperation::JoinBlocks { .. } => Some(0),
            TypedOperation::WrapInList {
                list_type,
                item_type,
                attrs,
                item_attrs,
                ..
            } => {
                let attrs_bytes = checked_attrs_input_bytes(
                    request_id,
                    operation_index,
                    attrs,
                    context.resource_limits,
                    input_bytes
                        .saturating_add(list_type.len())
                        .saturating_add(item_type.len()),
                )?;
                let item_attrs_bytes = checked_attrs_input_bytes(
                    request_id,
                    operation_index,
                    item_attrs,
                    context.resource_limits,
                    input_bytes
                        .saturating_add(list_type.len())
                        .saturating_add(item_type.len())
                        .saturating_add(attrs_bytes),
                )?;
                list_type
                    .len()
                    .checked_add(item_type.len())
                    .and_then(|amount| amount.checked_add(attrs_bytes))
                    .and_then(|amount| amount.checked_add(item_attrs_bytes))
            }
            TypedOperation::UnwrapFromList { .. }
            | TypedOperation::IndentListItem { .. }
            | TypedOperation::OutdentListItem { .. } => Some(0),
        }
        .ok_or_else(|| input_work_overflow(request_id, operation_index, context))?;
        charge_input(
            &mut input_bytes,
            amount,
            request_id,
            operation_index,
            context.resource_limits.max_input_bytes,
        )?;
    }
    Ok(input_bytes)
}

fn input_limit_error(
    request_id: u64,
    operation_index: Option<usize>,
    limit: usize,
    actual: usize,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        operation_index,
        "maxInputBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::try_from(actual).unwrap_or(u64::MAX),
    )
}

fn document_text_bytes(document: &Document) -> Option<usize> {
    fn node_bytes(node: &Node) -> Option<usize> {
        let mut total = node.text_str().map_or(0, str::len);
        if let Some(content) = node.content() {
            for child in content.iter() {
                total = total.checked_add(node_bytes(child)?)?;
            }
        }
        Some(total)
    }
    node_bytes(document.root())
}

fn compile_transaction_impl(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
    mut lowering: Option<MutationCompiler>,
    mut crdt_envelope_loader: Option<&mut dyn FnMut(usize) -> OperationResult<CrdtEnvelope>>,
) -> OperationResult<CompiledTransaction> {
    let request_id = transaction.request_id;
    let mut work = CheckedWork::default();
    // Revision, origin, operation count and aggregate input were admitted by the
    // single shared envelope path before any optional Yrs target traversal.
    debug_assert_eq!(
        transaction.base_document_revision,
        context.document_revision
    );
    debug_assert!(matches!(
        transaction.origin,
        TransactionOrigin::LocalInput
            | TransactionOrigin::LocalCommand
            | TransactionOrigin::LocalApi
    ));
    debug_assert!(
        transaction.operations.len() <= context.editing_limits.max_operations_per_transaction
    );

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
    let mut undo_units_bound = 0u64;
    let mut undo_limit_error = None;
    let mut history_class = HistoryClass::Skip;
    let records_history = transaction.history_policy != HistoryPolicy::Skip;

    for (operation_index, operation) in transaction.operations.iter().enumerate() {
        match operation {
            TypedOperation::InsertText { at, text, marks } => {
                validate_operation_marks(request_id, operation_index, marks, context.schema)?;
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
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.insert(operation_index, pos, text, marks)?;
                }
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
                    if lowering.delete(operation_index, from, to, &boundaries)?
                        == TextRangeDisposition::Structural
                    {
                        lowering.delete_structural_range(operation_index, &preview, from, to)?;
                    }
                }
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
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                let operation_changed = next != preview;
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
                preview = next;
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
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                let operation_changed = next != preview;
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
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                let operation_changed = next != preview;
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
                    }
                }
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
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                let operation_changed = next != preview;
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
                    &rendered_text,
                    &base_position_map,
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
                preview = next;
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
                    &rendered_text,
                    &base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
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
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
                    &rendered_text,
                    &base_position_map,
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
                preview = next;
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
                    &rendered_text,
                    &base_position_map,
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
                preview = next;
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::UnwrapFromList { at } => {
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
                preview = next;
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::IndentListItem { at } => {
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
                let step = Step::IndentListItem { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                let operation_changed = next != preview;
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
                preview = next;
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
                    &rendered_text,
                    &base_position_map,
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
                let operation_changed = next != preview;
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
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
                    &rendered_text,
                    &base_position_map,
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
                let operation_changed = next != preview;
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
                operation_result = Some(Selection::node(pos));
                composed_map = composed_map.compose(&step_map);
                preview = next;
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
        undo_limit_error = None;
    }

    let yrs_lowered = lowering.is_some();
    let lowered_plan = lowering
        .map(|compiler| compiler.finish(transaction.operations.len().checked_sub(1)))
        .transpose()?
        .unwrap_or_default();
    let mut mutation_plan = if preview == *context.document {
        YrsMutationPlan::default()
    } else {
        lowered_plan
    };
    mutation_plan.cache_prepared_metrics(request_id)?;
    let crdt_envelope = if mutation_plan.requires_crdt_envelope() {
        Some(crdt_envelope_loader.as_mut().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs live deletion plan has no snapshot envelope loader",
            )
        })?(mutation_plan.scan_work)?)
    } else {
        None
    };
    if yrs_lowered && history_class != HistoryClass::Skip {
        undo_units_bound = estimate_undo_units(request_id, &mutation_plan, crdt_envelope.as_ref())?;
        if undo_units_bound > context.editing_limits.max_undo_retained_units {
            // The semantic pass records the first operation that crosses the
            // aggregate limit. Preserve only that attribution when the exact
            // Yrs estimator confirms the failure: the reported `actual` must
            // always be the exact plan-derived bound.
            let operation_index = undo_limit_error
                .as_ref()
                .and_then(|error| error.operation_index)
                .or_else(|| transaction.operations.len().checked_sub(1));
            undo_limit_error = Some(OperationError::operation_limit_exceeded(
                request_id,
                operation_index,
                "maxUndoRetainedUnits",
                context.editing_limits.max_undo_retained_units,
                undo_units_bound,
            ));
        } else {
            undo_limit_error = None;
        }
    }
    if let Some(error) = undo_limit_error {
        return Err(error);
    }
    let encoded_growth_bound =
        estimate_update_v1_growth(request_id, &mutation_plan, crdt_envelope.as_ref())?;

    Ok(CompiledTransaction {
        request_id,
        origin: transaction.origin,
        history_policy: transaction.history_policy,
        history_class,
        preview,
        selection_plan,
        affected_top_level_blocks,
        mutation_plan,
        encoded_growth_bound,
        undo_units_bound,
    })
}

fn text_boundaries(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    schema: &Schema,
    lowering: &mut MutationCompiler,
) -> OperationResult<Vec<u32>> {
    fn visit(
        request_id: u64,
        operation_index: usize,
        node: &Node,
        schema: &Schema,
        lowering: &mut MutationCompiler,
        position: &mut u32,
        output: &mut Vec<u32>,
    ) -> OperationResult<()> {
        lowering.charge_boundary_node(operation_index)?;
        if let Some(text) = node.text_str() {
            output.push(*position);
            lowering.charge_boundary_text(operation_index, text.len())?;
            let len = u32::try_from(text.chars().count()).map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview text scalar length exceeds u32",
                )
            })?;
            *position = position.checked_add(len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview text boundary overflow",
                )
            })?;
            output.push(*position);
            return Ok(());
        }
        if let Some(content) = node.content() {
            let is_document = node.node_type() == schema.doc_node_type();
            if !is_document {
                *position = position.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "preview node boundary overflow",
                    )
                })?;
            }
            for child in content.iter() {
                visit(
                    request_id,
                    operation_index,
                    child,
                    schema,
                    lowering,
                    position,
                    output,
                )?;
            }
            if !is_document {
                *position = position.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "preview node boundary overflow",
                    )
                })?;
            }
        } else {
            *position = position.checked_add(1).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview leaf boundary overflow",
                )
            })?;
        }
        Ok(())
    }

    let mut output = Vec::new();
    let mut position = 0u32;
    visit(
        request_id,
        operation_index,
        document.root(),
        schema,
        lowering,
        &mut position,
        &mut output,
    )?;
    output.sort_unstable();
    output.dedup();
    Ok(output)
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

#[derive(Clone, Copy)]
struct InsertNodeResolverContext<'a> {
    request_id: u64,
    operation_index: usize,
    document: &'a Document,
    node: &'a Node,
    schema: &'a Schema,
    budget: &'a WorkBudget,
    limit: usize,
}

fn resolve_insert_node_position(
    context: InsertNodeResolverContext<'_>,
    position: super::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
) -> OperationResult<u32> {
    let InsertNodeResolverContext {
        request_id,
        operation_index,
        document,
        node,
        schema,
        budget: resolver_budget,
        limit: resolver_limit,
    } = context;
    let (fallback, scalar_offset, block_index) = resolve_insert_node_base_position(
        request_id,
        operation_index,
        position,
        rendered_text,
        position_map,
        document,
        resolver_budget,
        resolver_limit,
    )?;
    if !insert_node_is_block(node, schema) {
        return Ok(fallback);
    }
    consume_resolver_work(
        resolver_budget,
        1,
        request_id,
        operation_index,
        resolver_limit,
    )?;
    let Some(index) = block_index else {
        return Ok(fallback);
    };
    if let Some(block) = position_map.block(index) {
        let break_start = block
            .scalar_start
            .checked_add(block.scalar_prefix_len)
            .and_then(|start| start.checked_add(block.scalar_len));
        let break_end = break_start.and_then(|start| start.checked_add(block.rendered_break_after));
        if break_start
            .zip(break_end)
            .is_some_and(|(start, end)| scalar_offset >= start && scalar_offset < end)
        {
            let Some(next) = position_map.block(index + 1) else {
                return Ok(fallback);
            };
            let mut common_depth = 0usize;
            for (left, right) in block.node_path.iter().zip(next.node_path.iter()) {
                consume_resolver_work(
                    resolver_budget,
                    1,
                    request_id,
                    operation_index,
                    resolver_limit,
                )?;
                if left != right {
                    break;
                }
                common_depth += 1;
            }
            let direct_path = common_depth
                .checked_add(1)
                .filter(|depth| *depth <= block.node_path.len())
                .map(|depth| &block.node_path[..depth]);
            let mut candidates = Vec::with_capacity(4);
            if let Some(path) = direct_path {
                candidates.push((path, true));
            }
            match position.affinity {
                super::Affinity::Before => {
                    candidates.push((block.node_path.as_slice(), true));
                    candidates.push((next.node_path.as_slice(), false));
                    candidates.push((next.node_path.as_slice(), true));
                }
                super::Affinity::After => {
                    candidates.push((next.node_path.as_slice(), false));
                    candidates.push((next.node_path.as_slice(), true));
                    candidates.push((block.node_path.as_slice(), true));
                }
            }
            for (path, after_child) in candidates {
                let candidate = document_position_at_path_budgeted(
                    document,
                    path,
                    after_child,
                    resolver_budget,
                    request_id,
                    operation_index,
                    resolver_limit,
                )?;
                if let Some(candidate) = candidate {
                    let valid = insertion_is_schema_valid(context, candidate)?;
                    if valid {
                        return Ok(candidate);
                    }
                }
            }
            return Ok(fallback);
        }
        let Some(content_start) = block.scalar_start.checked_add(block.scalar_prefix_len) else {
            return Ok(fallback);
        };
        let Some(content_end) = content_start.checked_add(block.scalar_len) else {
            return Ok(fallback);
        };
        if scalar_offset == content_start && position.affinity == super::Affinity::Before {
            if let Some(candidate) = document_position_at_path_budgeted(
                document,
                &block.node_path,
                false,
                resolver_budget,
                request_id,
                operation_index,
                resolver_limit,
            )? {
                let valid = insertion_is_schema_valid(context, candidate)?;
                if valid {
                    return Ok(candidate);
                }
            }
        }
        if scalar_offset == content_end && position.affinity == super::Affinity::After {
            if let Some(candidate) = document_position_at_path_budgeted(
                document,
                &block.node_path,
                true,
                resolver_budget,
                request_id,
                operation_index,
                resolver_limit,
            )? {
                let valid = insertion_is_schema_valid(context, candidate)?;
                if valid {
                    return Ok(candidate);
                }
            }
        }
    }
    Ok(fallback)
}

#[allow(clippy::too_many_arguments)]
fn resolve_insert_node_base_position(
    request_id: u64,
    operation_index: usize,
    position: super::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
    resolver_budget: &WorkBudget,
    resolver_limit: usize,
) -> OperationResult<(u32, u32, Option<usize>)> {
    let scalar_offset = match position.kind {
        super::EditorOffsetKind::Scalar => Some(position.offset),
        super::EditorOffsetKind::Utf16 => utf16_offset_to_scalar_budgeted(
            rendered_text,
            position.offset,
            resolver_budget,
            request_id,
            operation_index,
            resolver_limit,
        )?,
    }
    .filter(|offset| *offset <= position_map.total_scalars())
    .ok_or_else(|| {
        OperationError::position_invalid(
            request_id,
            operation_index,
            "at",
            "at is outside the base document",
        )
    })?;

    let mut exhausted = false;
    let fallback = position_map.scalar_to_doc_metered(scalar_offset, document, |amount| {
        let admitted = resolver_budget.consume_n(amount);
        exhausted |= !admitted;
        admitted
    });
    if exhausted {
        return Err(resolver_budget_exhausted(
            request_id,
            operation_index,
            resolver_limit,
        ));
    }
    let (fallback, block_index) = fallback.ok_or_else(|| {
        OperationError::position_invalid(
            request_id,
            operation_index,
            "at",
            "at is outside the base document",
        )
    })?;
    Ok((
        fallback,
        scalar_offset,
        position_map.block(block_index).map(|_| block_index),
    ))
}

fn utf16_offset_to_scalar_budgeted(
    value: &str,
    utf16_offset: u32,
    resolver_budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    resolver_limit: usize,
) -> OperationResult<Option<u32>> {
    if utf16_offset == 0 {
        return Ok(Some(0));
    }
    let mut utf16_seen = 0u32;
    let mut scalars_seen = 0u32;
    for character in value.chars() {
        consume_resolver_work(
            resolver_budget,
            1,
            request_id,
            operation_index,
            resolver_limit,
        )?;
        let next_utf16 = utf16_seen.saturating_add(character.len_utf16() as u32);
        if utf16_offset < next_utf16 {
            return Ok(None);
        }
        scalars_seen = scalars_seen.saturating_add(1);
        if utf16_offset == next_utf16 {
            return Ok(Some(scalars_seen));
        }
        utf16_seen = next_utf16;
    }
    Ok(None)
}

fn normalize_current_insert_node_position(
    context: InsertNodeResolverContext<'_>,
    position: u32,
    affinity: super::Affinity,
) -> OperationResult<u32> {
    let InsertNodeResolverContext {
        request_id,
        operation_index,
        document,
        node,
        schema,
        budget: resolver_budget,
        limit: resolver_limit,
    } = context;
    let valid = insertion_is_schema_valid(context, position)?;
    if valid || !insert_node_is_block(node, schema) {
        return Ok(position);
    }
    let Some(resolved) = resolve_insert_position_budgeted(
        document,
        position,
        resolver_budget,
        request_id,
        operation_index,
        resolver_limit,
    )?
    else {
        return Ok(position);
    };
    let Some(content) = resolved.parent.content() else {
        return Ok(position);
    };
    let at_start = resolved.parent_offset == 0;
    let at_end = resolved.parent_offset == content.size();
    if !at_start && !at_end {
        return Ok(position);
    }
    for depth in (1..=resolved.node_path.len()).rev() {
        let path = &resolved.node_path[..depth];
        let boundary_order = match (at_start, at_end, affinity) {
            (true, true, super::Affinity::After) => [true, false],
            (true, _, _) => [false, true],
            _ => [true, false],
        };
        for after_child in boundary_order {
            let Some(candidate) = document_position_at_path_budgeted(
                document,
                path,
                after_child,
                resolver_budget,
                request_id,
                operation_index,
                resolver_limit,
            )?
            else {
                continue;
            };
            let valid = insertion_is_schema_valid(context, candidate)?;
            if valid {
                return Ok(candidate);
            }
        }
    }
    Ok(position)
}

fn insert_node_is_block(node: &Node, schema: &Schema) -> bool {
    schema.node(node.node_type()).map_or_else(
        || {
            matches!(node.node_type(), "__opaque" | "__opaque_json")
                && node
                    .attrs()
                    .get("opaque_placement")
                    .and_then(serde_json::Value::as_str)
                    == Some("block")
        },
        |spec| {
            matches!(
                spec.role,
                crate::schema::NodeRole::TextBlock
                    | crate::schema::NodeRole::List { .. }
                    | crate::schema::NodeRole::ListItem
                    | crate::schema::NodeRole::Block
            )
        },
    )
}

fn insertion_is_schema_valid(
    context: InsertNodeResolverContext<'_>,
    position: u32,
) -> OperationResult<bool> {
    let InsertNodeResolverContext {
        request_id,
        operation_index,
        document,
        node: inserted,
        schema,
        budget: resolver_budget,
        limit: resolver_limit,
    } = context;
    enum Child<'a> {
        Existing(&'a Node),
        Inserted(&'a Node),
    }
    let Some(resolved) = resolve_insert_position_budgeted(
        document,
        position,
        resolver_budget,
        request_id,
        operation_index,
        resolver_limit,
    )?
    else {
        return Ok(false);
    };
    let parent = resolved.parent;
    let Some(content) = parent.content() else {
        return Ok(false);
    };
    let mut offset = 0u32;
    let mut insertion_index = None;
    for (index, child) in content.iter().enumerate() {
        consume_resolver_work(
            resolver_budget,
            1,
            request_id,
            operation_index,
            resolver_limit,
        )?;
        if offset == resolved.parent_offset {
            insertion_index = Some(index);
            break;
        }
        let child_size = resolver_node_size(
            child,
            resolver_budget,
            request_id,
            operation_index,
            resolver_limit,
        )?;
        let Some(end) = offset.checked_add(child_size) else {
            return Ok(false);
        };
        if resolved.parent_offset < end {
            return Ok(false);
        }
        offset = end;
    }
    let insertion_index = insertion_index
        .or_else(|| (offset == resolved.parent_offset).then_some(content.child_count()));
    let Some(insertion_index) = insertion_index else {
        return Ok(false);
    };
    let Some(parent_spec) = schema.node(parent.node_type()) else {
        return Ok(false);
    };
    let assembly_work = content
        .child_count()
        .checked_add(1)
        .ok_or_else(|| resolver_budget_exhausted(request_id, operation_index, resolver_limit))?;
    consume_resolver_work(
        resolver_budget,
        assembly_work,
        request_id,
        operation_index,
        resolver_limit,
    )?;
    let children = content
        .iter()
        .enumerate()
        .flat_map(|(index, child)| {
            (index == insertion_index)
                .then_some(Child::Inserted(inserted))
                .into_iter()
                .chain(std::iter::once(Child::Existing(child)))
        })
        .chain((insertion_index == content.child_count()).then_some(Child::Inserted(inserted)))
        .collect::<Vec<_>>();
    let valid = parent_spec
        .content
        .matches_with_budget(
            &children,
            |child, symbol| {
                let node = match child {
                    Child::Existing(node) | Child::Inserted(node) => node,
                };
                if matches!(node.node_type(), "__opaque" | "__opaque_json") {
                    return node
                        .attrs()
                        .get("opaque_placement")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|placement| {
                            schema.symbol_accepts_opaque_placement(symbol, placement)
                        });
                }
                schema.node_matches_symbol(node.node_type(), symbol)
            },
            resolver_budget,
        )
        .map_err(|_| resolver_budget_exhausted(request_id, operation_index, resolver_limit))?;
    Ok(valid)
}

fn resolver_budget_exhausted(
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxInputBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1),
    )
}

fn consume_resolver_work(
    budget: &WorkBudget,
    amount: usize,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<()> {
    if budget.consume_n(amount) {
        Ok(())
    } else {
        Err(resolver_budget_exhausted(
            request_id,
            operation_index,
            limit,
        ))
    }
}

struct InsertResolvedPosition<'a> {
    node_path: SmallVec<[u32; 8]>,
    parent_offset: u32,
    parent: &'a Node,
}

fn resolve_insert_position_budgeted<'a>(
    document: &'a Document,
    position: u32,
    budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<Option<InsertResolvedPosition<'a>>> {
    consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
    if position > document.content_size() {
        return Ok(None);
    }

    let mut parent = document.root();
    let mut relative_position = position;
    let mut path = SmallVec::<[u32; 8]>::new();
    loop {
        let Some(content) = parent.content() else {
            return Ok(None);
        };
        if relative_position > content.size() {
            return Ok(None);
        }

        let mut offset = 0u32;
        let mut descended = false;
        for (child_index, child) in content.iter().enumerate() {
            consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
            let child_size = resolver_node_size(child, budget, request_id, operation_index, limit)?;
            if child.is_text() || child.is_void() {
                if relative_position < offset.saturating_add(child_size) {
                    return Ok(Some(InsertResolvedPosition {
                        node_path: path,
                        parent_offset: relative_position,
                        parent,
                    }));
                }
                let Some(next_offset) = offset.checked_add(child_size) else {
                    return Ok(None);
                };
                offset = next_offset;
                continue;
            }

            if relative_position == offset {
                return Ok(Some(InsertResolvedPosition {
                    node_path: path,
                    parent_offset: relative_position,
                    parent,
                }));
            }
            let Some(inner_start) = offset.checked_add(1) else {
                return Ok(None);
            };
            let Some(inner_end) = offset
                .checked_add(child_size)
                .and_then(|end| end.checked_sub(1))
            else {
                return Ok(None);
            };
            if relative_position >= inner_start && relative_position <= inner_end {
                let Ok(child_index) = u32::try_from(child_index) else {
                    return Ok(None);
                };
                path.push(child_index);
                relative_position -= inner_start;
                parent = child;
                descended = true;
                break;
            }
            let Some(next_offset) = offset.checked_add(child_size) else {
                return Ok(None);
            };
            offset = next_offset;
        }
        if descended {
            continue;
        }
        return Ok(
            (relative_position == offset).then_some(InsertResolvedPosition {
                node_path: path,
                parent_offset: relative_position,
                parent,
            }),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn document_position_at_path_budgeted(
    document: &Document,
    path: &[u32],
    after_child: bool,
    budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<Option<u32>> {
    let mut parent = document.root();
    let mut position = 0u32;
    for (depth, &child_index) in path.iter().enumerate() {
        consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
        let Some(content) = parent.content() else {
            return Ok(None);
        };
        let Ok(child_index) = usize::try_from(child_index) else {
            return Ok(None);
        };
        for child in content.iter().take(child_index) {
            consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
            let child_size = resolver_node_size(child, budget, request_id, operation_index, limit)?;
            let Some(next_position) = position.checked_add(child_size) else {
                return Ok(None);
            };
            position = next_position;
        }
        let Some(child) = content.child(child_index) else {
            return Ok(None);
        };
        if depth + 1 == path.len() {
            return Ok(if after_child {
                position.checked_add(resolver_node_size(
                    child,
                    budget,
                    request_id,
                    operation_index,
                    limit,
                )?)
            } else {
                Some(position)
            });
        }
        let Some(next_position) = position.checked_add(1) else {
            return Ok(None);
        };
        position = next_position;
        parent = child;
    }
    Ok(Some(position))
}

fn resolver_node_size(
    node: &Node,
    budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<u32> {
    if let Some(text) = node.text_str() {
        consume_resolver_work(budget, text.len(), request_id, operation_index, limit)?;
    }
    Ok(node.node_size())
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

fn resolve_attribute_target_position(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    position: u32,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
    schema: &Schema,
) -> OperationResult<u32> {
    let accepts = |node: &Node| {
        schema.node(node.node_type()).is_some_and(|spec| {
            (spec.allow_undeclared_attrs || attrs.keys().all(|key| spec.attrs.contains_key(key)))
                && (!attrs.is_empty() || !spec.attrs.is_empty() || !node.attrs().is_empty())
        })
    };
    if direct_attribute_target(document, position).is_some_and(&accepts) {
        return Ok(position);
    }
    let resolved = document.resolve(position).map_err(|message| {
        OperationError::position_invalid(request_id, operation_index, "at", message)
    })?;
    for depth in (1..=resolved.node_path.len()).rev() {
        let path = &resolved.node_path[..depth];
        let Some(node) = document.node_at(path) else {
            continue;
        };
        if accepts(node) {
            return node_boundary_position(document.root(), path).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "attribute target path has no document boundary",
                )
            });
        }
    }
    Err(OperationError::position_invalid(
        request_id,
        operation_index,
        "at",
        "position does not resolve to a node accepting the supplied attributes",
    ))
}

fn resolve_join_target_position(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    position: u32,
) -> OperationResult<u32> {
    let resolved = document.resolve(position).map_err(|message| {
        OperationError::position_invalid(request_id, operation_index, "at", message)
    })?;
    if resolved.node_path.is_empty() {
        return Ok(position);
    }
    let deepest = document.node_at(&resolved.node_path).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "join position path has no containing node",
        )
    })?;
    if resolved.parent_offset != deepest.content().map(Fragment::size).unwrap_or(0) {
        return Ok(position);
    }
    for depth in (1..=resolved.node_path.len()).rev() {
        let path = &resolved.node_path[..depth];
        let parent_path = &path[..depth - 1];
        let child_index = usize::try_from(path[depth - 1]).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "join child index exceeds usize",
            )
        })?;
        let parent = if parent_path.is_empty() {
            document.root()
        } else {
            document.node_at(parent_path).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "join ancestor path has no parent node",
                )
            })?
        };
        let content = parent.content().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "join ancestor parent has no content",
            )
        })?;
        if let Some(next) = content.child(child_index.saturating_add(1)) {
            let candidate = document.node_at(path).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "join candidate path has no node",
                )
            })?;
            if candidate.is_element() && next.is_element() {
                let start = node_boundary_position(document.root(), path).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "join candidate path has no document boundary",
                    )
                })?;
                return start.checked_add(candidate.node_size()).ok_or_else(|| {
                    OperationError::position_invalid(
                        request_id,
                        operation_index,
                        "at",
                        "join boundary position overflow",
                    )
                });
            }
            break;
        }
    }
    Ok(position)
}

fn direct_attribute_target(document: &Document, position: u32) -> Option<&Node> {
    let resolved = document.resolve(position).ok()?;
    let parent = resolved.parent(document);
    let mut offset = 0u32;
    parent.content()?.iter().find(|child| {
        let matches = !child.is_text() && resolved.parent_offset == offset;
        if !matches {
            offset = offset.saturating_add(child.node_size());
        }
        matches
    })
}

fn node_boundary_position(root: &Node, path: &[u32]) -> Option<u32> {
    let mut node = root;
    let mut content_start = 0u32;
    for (depth, child_index) in path.iter().copied().enumerate() {
        let content = node.content()?;
        let index = usize::try_from(child_index).ok()?;
        let preceding = content
            .iter()
            .take(index)
            .try_fold(0u32, |total, child| total.checked_add(child.node_size()))?;
        let boundary = content_start.checked_add(preceding)?;
        node = content.child(index)?;
        if depth + 1 == path.len() {
            return Some(boundary);
        }
        content_start = boundary.checked_add(1)?;
    }
    None
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

fn input_work_overflow(
    request_id: u64,
    operation_index: usize,
    context: CompilationContext<'_>,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxInputBytes",
        u64::try_from(context.resource_limits.max_input_bytes).unwrap_or(u64::MAX),
        u64::MAX,
    )
}

fn checked_mark_input_bytes(
    request_id: u64,
    operation_index: usize,
    mark: &Mark,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    counter.charge_bytes(mark.mark_type().len())?;
    counter.count_attrs(mark.attrs())?;
    Ok(counter.delta())
}

fn checked_mark_set_input_bytes(
    request_id: u64,
    operation_index: usize,
    marks: &[Mark],
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    for mark in marks {
        counter.charge_bytes(mark.mark_type().len())?;
        counter.count_attrs(mark.attrs())?;
    }
    Ok(counter.delta())
}

fn checked_fragment_input_bytes(
    request_id: u64,
    operation_index: usize,
    fragment: &Fragment,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    count_node_forest(&mut counter, fragment.children())?;
    Ok(counter.delta())
}

fn checked_node_input_bytes(
    request_id: u64,
    operation_index: usize,
    node: &Node,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    count_node_forest(&mut counter, std::slice::from_ref(node))?;
    Ok(counter.delta())
}

fn checked_attrs_input_bytes(
    request_id: u64,
    operation_index: usize,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    counter.count_attrs(attrs)?;
    Ok(counter.delta())
}

fn count_node_forest(
    counter: &mut StructuredInputCounter<'_>,
    roots: &[Node],
) -> OperationResult<()> {
    enum Frame<'a> {
        Node(&'a Node, usize),
        Children(std::slice::Iter<'a, Node>, usize),
    }
    let mut stack = vec![Frame::Children(roots.iter(), 1)];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Node(node, depth) => {
                counter.charge_bytes(node.node_type().len())?;
                counter.count_attrs(node.attrs())?;
                for mark in node.marks() {
                    counter.charge_bytes(mark.mark_type().len())?;
                    counter.count_attrs(mark.attrs())?;
                }
                if let Some(text) = node.text_str() {
                    counter.charge_bytes(text.len())?;
                }
                if let Some(content) = node.content() {
                    let child_depth = depth.checked_add(1).ok_or_else(|| counter.depth_error())?;
                    stack.push(Frame::Children(content.children().iter(), child_depth));
                }
            }
            Frame::Children(mut children, depth) => {
                if let Some(child) = children.next() {
                    counter.admit_item(depth)?;
                    stack.push(Frame::Children(children, depth));
                    stack.push(Frame::Node(child, depth));
                }
            }
        }
    }
    Ok(())
}

struct StructuredInputCounter<'a> {
    request_id: u64,
    operation_index: usize,
    limits: &'a ResourceLimits,
    base_bytes: usize,
    json_meter: JsonValueMeter,
    items: usize,
}

impl<'a> StructuredInputCounter<'a> {
    fn new(
        request_id: u64,
        operation_index: usize,
        limits: &'a ResourceLimits,
        base_bytes: usize,
    ) -> Self {
        Self {
            request_id,
            operation_index,
            limits,
            base_bytes,
            json_meter: JsonValueMeter::new(
                limits.max_input_bytes,
                limits.max_document_nodes,
                limits.max_document_depth,
                base_bytes,
            ),
            items: 0,
        }
    }

    fn delta(&self) -> usize {
        self.json_meter.bytes() - self.base_bytes
    }

    fn charge_bytes(&mut self, amount: usize) -> OperationResult<()> {
        self.json_meter
            .charge_bytes(amount)
            .map_err(|error| self.map_json_meter_error(error))
    }

    fn admit_item(&mut self, depth: usize) -> OperationResult<()> {
        if depth > self.limits.max_document_depth {
            return Err(self.depth_error());
        }
        let actual = self.items.saturating_add(1);
        if actual > self.limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                Some(self.operation_index),
                "maxDocumentNodes",
                u64::try_from(self.limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            ));
        }
        self.items = actual;
        Ok(())
    }

    fn depth_error(&self) -> OperationError {
        OperationError::operation_limit_exceeded(
            self.request_id,
            Some(self.operation_index),
            "maxDocumentDepth",
            u64::try_from(self.limits.max_document_depth).unwrap_or(u64::MAX),
            u64::try_from(self.limits.max_document_depth)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
    }

    fn count_attrs(
        &mut self,
        attrs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> OperationResult<()> {
        self.json_meter
            .admit_object(attrs)
            .map_err(|error| self.map_json_meter_error(error))
    }

    fn map_json_meter_error(&self, error: JsonMeterError) -> OperationError {
        match error.dimension {
            JsonMeterDimension::Bytes => input_limit_error(
                self.request_id,
                Some(self.operation_index),
                error.limit,
                error.actual,
            ),
            JsonMeterDimension::Work => OperationError::operation_limit_exceeded(
                self.request_id,
                Some(self.operation_index),
                "maxDocumentNodes",
                u64::try_from(error.limit).unwrap_or(u64::MAX),
                u64::try_from(error.actual).unwrap_or(u64::MAX),
            ),
            JsonMeterDimension::Depth => OperationError::operation_limit_exceeded(
                self.request_id,
                Some(self.operation_index),
                "maxDocumentDepth",
                u64::try_from(error.limit).unwrap_or(u64::MAX),
                u64::try_from(error.actual).unwrap_or(u64::MAX),
            ),
        }
    }
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
