use crate::boundary::{JsonMeterDimension, JsonMeterError, JsonValueMeter, ResourceLimits};
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::Schema;
use crate::yrs_engine::compiler::CompilationContext;
use crate::yrs_engine::editing_limits::CheckedWork;
use crate::yrs_engine::mutation::crdt_clock_scan_reservation;
use crate::yrs_engine::{
    OperationError, OperationResult, TransactionOrigin, TypedOperation, TypedTransaction,
};

pub(crate) fn admit_transaction_envelope(
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
            TypedOperation::ReplaceStructure(replacement) => Some(checked_fragment_input_bytes(
                request_id,
                operation_index,
                replacement.content(),
                context.resource_limits,
                input_bytes,
            )?),
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

pub(super) fn input_limit_error(
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

pub(crate) fn admit_yrs_scan_work<T: yrs::ReadTxn>(
    request_id: u64,
    admitted_input_bytes: usize,
    document_text_bytes: usize,
    txn: &T,
    resource_limits: &ResourceLimits,
) -> OperationResult<usize> {
    let crdt_clock_work =
        crdt_clock_scan_reservation(request_id, txn, resource_limits.max_encoded_state_bytes)?;
    // One pass materializes each Yrs text and one pass builds its scalar/UTF-16 index.
    // Reserve both before any selection or mutation traversal. Selection-only callers
    // may supply the exact document-text metric cached at the last document change.
    let initial_scan_work = document_text_bytes
        .checked_mul(2)
        .and_then(|work| work.checked_add(crdt_clock_work.checked_mul(2)?))
        .ok_or_else(|| {
            input_limit_error(
                request_id,
                None,
                resource_limits.max_input_bytes,
                usize::MAX,
            )
        })?;
    let charged_scan_work = admitted_input_bytes
        .checked_add(initial_scan_work)
        .ok_or_else(|| {
            input_limit_error(
                request_id,
                None,
                resource_limits.max_input_bytes,
                usize::MAX,
            )
        })?;
    if charged_scan_work > resource_limits.max_input_bytes {
        return Err(input_limit_error(
            request_id,
            None,
            resource_limits.max_input_bytes,
            charged_scan_work,
        ));
    }
    Ok(charged_scan_work)
}

pub(super) fn charge_input(
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

pub(super) fn charge_undo_bound(
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

pub(super) fn validate_operation_marks(
    request_id: u64,
    operation_index: usize,
    marks: &[Mark],
    schema: &Schema,
) -> OperationResult<()> {
    crate::transform::validate_input_mark_set(marks, schema).map_err(|error| {
        OperationError::operation_invalid(request_id, operation_index, "marks", error.to_string())
    })
}

pub(super) fn add_mark_conflicts_with_existing_attrs(
    document: &Document,
    from: u32,
    to: u32,
    mark: &Mark,
) -> bool {
    if from >= to {
        return false;
    }
    let (Ok(resolved_from), Ok(resolved_to)) = (document.resolve(from), document.resolve(to))
    else {
        return false;
    };
    if resolved_from.node_path != resolved_to.node_path {
        return false;
    }
    let parent = resolved_from.parent(document);
    let Some(content) = parent.content() else {
        return false;
    };
    let mut offset = 0u32;
    for child in content.iter() {
        let child_end = offset.saturating_add(child.node_size());
        if child.is_text()
            && child_end > resolved_from.parent_offset
            && offset < resolved_to.parent_offset
            && child
                .marks()
                .iter()
                .any(|existing| existing.mark_type() == mark.mark_type() && existing != mark)
        {
            return true;
        }
        offset = child_end;
    }
    false
}

pub(super) fn validate_fragment_marks(
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

pub(super) fn validate_preview_marks(
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

pub(super) fn input_work_overflow(
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

pub(super) fn checked_mark_input_bytes(
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

pub(super) fn checked_mark_set_input_bytes(
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

pub(super) fn checked_fragment_input_bytes(
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

pub(super) fn checked_node_input_bytes(
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

pub(super) fn checked_attrs_input_bytes(
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

pub(super) fn count_node_forest(
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

pub(super) struct StructuredInputCounter<'a> {
    pub(super) request_id: u64,
    pub(super) operation_index: usize,
    pub(super) limits: &'a ResourceLimits,
    pub(super) base_bytes: usize,
    pub(super) json_meter: JsonValueMeter,
    pub(super) items: usize,
}

impl<'a> StructuredInputCounter<'a> {
    pub(super) fn new(
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

    pub(super) fn delta(&self) -> usize {
        self.json_meter.bytes() - self.base_bytes
    }

    pub(super) fn charge_bytes(&mut self, amount: usize) -> OperationResult<()> {
        self.json_meter
            .charge_bytes(amount)
            .map_err(|error| self.map_json_meter_error(error))
    }

    pub(super) fn admit_item(&mut self, depth: usize) -> OperationResult<()> {
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

    pub(super) fn depth_error(&self) -> OperationError {
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

    pub(super) fn count_attrs(
        &mut self,
        attrs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> OperationResult<()> {
        self.json_meter
            .admit_object(attrs)
            .map_err(|error| self.map_json_meter_error(error))
    }

    pub(super) fn map_json_meter_error(&self, error: JsonMeterError) -> OperationError {
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
