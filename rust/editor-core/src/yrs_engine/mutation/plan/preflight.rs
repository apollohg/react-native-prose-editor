impl YrsMutationPlan {
    pub(crate) fn cache_prepared_metrics(&mut self, request_id: u64) -> OperationResult<()> {
        if self.prepared_metrics.len() == self.actions.len() {
            return Ok(());
        }
        let mut cached = Vec::with_capacity(self.actions.len());
        let mut work = 0usize;
        for action in &self.actions {
            if let YrsMutationAction::InsertXmlChildren { nodes, .. } = action {
                let metrics = prepared_nodes_metrics(nodes)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                work = work
                    .checked_add(metrics.work)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                cached.push(Some(PreparedActionMetrics {
                    growth_bytes: metrics.growth_bytes,
                    insertion_units: metrics.insertion_units,
                }));
            } else {
                cached.push(None);
            }
        }
        self.compilation_work = self.compilation_work.checked_add(work).ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::try_from(self.work_limit).unwrap_or(u64::MAX),
                u64::MAX,
            )
        })?;
        if self.compilation_work > self.work_limit {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::try_from(self.work_limit).unwrap_or(u64::MAX),
                u64::try_from(self.compilation_work).unwrap_or(u64::MAX),
            ));
        }
        self.prepared_metrics = cached;
        Ok(())
    }
}

pub(super) fn expected_preflight_work(
    request_id: u64,
    actions: &[YrsMutationAction],
    path_parent_widths: &HashMap<BranchID, usize>,
) -> OperationResult<usize> {
    let mut indexed_work = 0usize;
    let mut materialized_work = 0usize;
    let mut path_cache = HashSet::<BranchID>::new();
    let mut text_targets = HashSet::<BranchID>::new();
    let mut elements = HashSet::<BranchID>::new();
    let mut structural_parents = HashSet::<BranchID>::new();
    let mut created_gaps = HashSet::<BranchID>::new();

    for action in actions {
        match action {
            YrsMutationAction::SetXmlAttribute { signature, .. }
            | YrsMutationAction::RemoveXmlAttribute { signature, .. } => {
                if elements.insert(signature.target.clone()) {
                    indexed_work = indexed_work
                        .checked_add(signature.path.len())
                        .and_then(|work| work.checked_add(element_signature_work(signature)?))
                        .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                    materialize_expected_path(
                        request_id,
                        action.operation_index(),
                        &signature.path,
                        path_parent_widths,
                        &mut path_cache,
                        &mut materialized_work,
                    )?;
                }
                indexed_work = indexed_work
                    .checked_add(1)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::DeleteXmlChildren {
                child_count,
                signature,
                ..
            } => {
                if structural_parents.insert(signature.parent.clone()) {
                    indexed_work = indexed_work
                        .checked_add(signature.path.len())
                        .and_then(|work| work.checked_add(signature.children.len()))
                        .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                    materialize_expected_path(
                        request_id,
                        action.operation_index(),
                        &signature.path,
                        path_parent_widths,
                        &mut path_cache,
                        &mut materialized_work,
                    )?;
                }
                indexed_work = indexed_work
                    .checked_add(usize::try_from(*child_count).map_err(|_| {
                        invalid_action_range(request_id, action.operation_index())
                    })?)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::InsertXmlChildren {
                nodes, signature, ..
            } => {
                if structural_parents.insert(signature.parent.clone()) {
                    indexed_work = indexed_work
                        .checked_add(signature.path.len())
                        .and_then(|work| work.checked_add(signature.children.len()))
                        .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                    materialize_expected_path(
                        request_id,
                        action.operation_index(),
                        &signature.path,
                        path_parent_widths,
                        &mut path_cache,
                        &mut materialized_work,
                    )?;
                }
                indexed_work = indexed_work
                    .checked_add(nodes.len())
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::CreateText { signature, .. } => {
                if !path_cache.contains(&signature.parent) {
                    indexed_work = indexed_work
                        .checked_add(signature.path.len())
                        .and_then(|work| work.checked_add(2))
                        .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                    materialize_expected_path(
                        request_id,
                        action.operation_index(),
                        &signature.path,
                        path_parent_widths,
                        &mut path_cache,
                        &mut materialized_work,
                    )?;
                    if path_cache.insert(signature.parent.clone()) {
                        materialized_work = materialized_work
                            .checked_add(usize::try_from(signature.child_count).map_err(|_| {
                                invalid_action_range(request_id, action.operation_index())
                            })?)
                            .ok_or_else(|| {
                                invalid_action_range(request_id, action.operation_index())
                            })?;
                    }
                } else {
                    indexed_work = indexed_work
                        .checked_add(2)
                        .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                }
                let fenwick_len = usize::try_from(signature.child_count)
                    .ok()
                    .and_then(|len| len.checked_add(2))
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                indexed_work = indexed_work
                    .checked_add(
                        binary_partition_work(fenwick_len)
                            .checked_mul(2)
                            .and_then(|work| {
                                work.checked_add(if created_gaps.insert(signature.parent.clone()) {
                                    fenwick_len
                                } else {
                                    0
                                })
                            })
                            .ok_or_else(|| {
                                invalid_action_range(request_id, action.operation_index())
                            })?,
                    )
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::InsertText { signature, .. }
            | YrsMutationAction::DeleteText { signature, .. }
            | YrsMutationAction::FormatText { signature, .. } => {
                if text_targets.insert(signature.target.clone()) {
                    indexed_work = indexed_work
                        .checked_add(signature.path.len())
                        .and_then(|work| work.checked_add(text_signature_work(&signature.runs)?))
                        .and_then(|work| work.checked_add(signature.capture_work))
                        .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                    materialize_expected_path(
                        request_id,
                        action.operation_index(),
                        &signature.path,
                        path_parent_widths,
                        &mut path_cache,
                        &mut materialized_work,
                    )?;
                }
            }
        }
    }
    indexed_work
        .checked_add(materialized_work)
        .ok_or_else(|| OperationError::engine_invariant_failed(request_id, None, "sealed Yrs preflight work overflow"))
}

fn materialize_expected_path(
    request_id: u64,
    operation_index: usize,
    path: &[(BranchID, u32)],
    path_parent_widths: &HashMap<BranchID, usize>,
    path_cache: &mut HashSet<BranchID>,
    materialized_work: &mut usize,
) -> OperationResult<()> {
    for (parent, _) in path {
        if path_cache.insert(parent.clone()) {
            let width = path_parent_widths.get(parent).copied().ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "sealed Yrs path parent has no recorded child width",
                )
            })?;
            *materialized_work = materialized_work
                .checked_add(width)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        }
    }
    Ok(())
}

fn element_signature_work(signature: &ElementSignature) -> Option<usize> {
    let mut work = 0usize;
    for (key, value) in &signature.attrs {
        work = work
            .checked_add(key.len())?
            .checked_add(any_preflight_work(value)?)?;
    }
    let partitions = binary_partition_work(signature.attrs.len());
    let key_work = signature.attrs.iter().try_fold(0usize, |total, (key, _)| {
        total.checked_add(key.len().checked_mul(partitions)?)
    })?;
    work.checked_add(signature.attrs.len().checked_mul(partitions)?)?
        .checked_add(key_work)
}

pub(crate) fn preflight_mutation_plan<T: ReadTxn>(
    request_id: u64,
    plan: &YrsMutationPlan,
    txn: &T,
) -> OperationResult<()> {
    let total_work = plan
        .compilation_work
        .checked_add(plan.expected_preflight_work)
        .ok_or_else(|| sealed_preflight_limit_error(request_id, plan, u64::MAX))?;
    if total_work > plan.work_limit {
        return Err(sealed_preflight_limit_error(
            request_id,
            plan,
            u64::try_from(total_work).unwrap_or(u64::MAX),
        ));
    }
    if plan.actions.is_empty() {
        return Ok(());
    }
    let guard = plan.document_guard.as_ref().ok_or_else(|| {
        document_guard_error(
            request_id,
            plan,
            "Yrs mutation plan has no sealed document guard",
        )
    })?;
    if txn.store() as *const _ as usize != guard.store_token {
        return Err(document_guard_error(
            request_id,
            plan,
            "Yrs mutation plan belongs to a different document store",
        ));
    }
    if txn.store().pending_update().is_some() || txn.store().pending_ds().is_some() {
        return Err(OperationError::engine_not_ready(request_id));
    }
    let state = txn.state_vector();
    if state != guard.snapshot.state_map
        || snapshot_state_clock_work(request_id, &state)? != guard.state_clock_work
    {
        return Err(document_guard_error(
            request_id,
            plan,
            "Yrs document state changed before mutation preflight",
        ));
    }
    if txn.snapshot() != guard.snapshot {
        return Err(document_guard_error(
            request_id,
            plan,
            "Yrs document snapshot changed before mutation preflight",
        ));
    }
    let measured_work = preflight_mutation_plan_impl(request_id, plan, txn)?;
    debug_assert_eq!(measured_work, plan.expected_preflight_work);
    Ok(())
}

fn document_guard_error(request_id: u64, plan: &YrsMutationPlan, message: &str) -> OperationError {
    OperationError::engine_invariant_failed(
        request_id,
        plan.actions.first().map(YrsMutationAction::operation_index),
        message,
    )
}

fn sealed_preflight_limit_error(
    request_id: u64,
    plan: &YrsMutationPlan,
    actual: u64,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        None,
        "maxActionsPerTransaction",
        u64::try_from(plan.work_limit).unwrap_or(u64::MAX),
        actual,
    )
}

fn preflight_mutation_plan_impl<T: ReadTxn>(
    request_id: u64,
    plan: &YrsMutationPlan,
    txn: &T,
) -> OperationResult<usize> {
    use std::collections::HashMap;

    let mut virtual_lengths = HashMap::<BranchID, u32>::new();
    let mut created_gaps = HashMap::<BranchID, Vec<u32>>::new();
    let mut validated_targets = std::collections::HashSet::<BranchID>::new();
    let mut path_children = HashMap::<BranchID, Vec<BranchID>>::new();
    let mut indexed_work = 0usize;
    let mut structural_parents = HashMap::<BranchID, StructuralPreflightState>::new();
    let mut validated_elements = std::collections::HashSet::<BranchID>::new();
    let mut last_attribute_keys = HashMap::<BranchID, Arc<str>>::new();
    for action in &plan.actions {
        match action {
            YrsMutationAction::SetXmlAttribute {
                target,
                key,
                value,
                signature,
                ..
            } => {
                preflight_attribute_action(
                    request_id,
                    action.operation_index(),
                    target,
                    key,
                    Some(value),
                    signature,
                    txn,
                    &mut validated_elements,
                    &mut last_attribute_keys,
                    &mut path_children,
                    &mut indexed_work,
                )?;
                continue;
            }
            YrsMutationAction::RemoveXmlAttribute {
                target,
                key,
                signature,
                ..
            } => {
                preflight_attribute_action(
                    request_id,
                    action.operation_index(),
                    target,
                    key,
                    None,
                    signature,
                    txn,
                    &mut validated_elements,
                    &mut last_attribute_keys,
                    &mut path_children,
                    &mut indexed_work,
                )?;
                continue;
            }
            YrsMutationAction::DeleteXmlChildren {
                parent,
                child_index,
                child_count,
                signature,
                ..
            } => {
                let state = validate_structural_parent(
                    PreflightActionContext {
                        request_id,
                        operation_index: action.operation_index(),
                    },
                    parent,
                    signature,
                    txn,
                    &mut structural_parents,
                    &mut path_children,
                    &mut indexed_work,
                )?;
                let start = usize::try_from(*child_index)
                    .map_err(|_| invalid_action_range(request_id, action.operation_index()))?;
                let count = usize::try_from(*child_count)
                    .map_err(|_| invalid_action_range(request_id, action.operation_index()))?;
                let end = start
                    .checked_add(count)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                if end > state.virtual_len || count == 0 {
                    return Err(invalid_action_range(request_id, action.operation_index()));
                }
                state.virtual_len = state
                    .virtual_len
                    .checked_sub(count)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                state.last_index = Some(start);
                state.last_was_delete = true;
                indexed_work = indexed_work
                    .checked_add(count)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                continue;
            }
            YrsMutationAction::InsertXmlChildren {
                parent,
                child_index,
                nodes,
                signature,
                ..
            } => {
                let state = validate_structural_parent(
                    PreflightActionContext {
                        request_id,
                        operation_index: action.operation_index(),
                    },
                    parent,
                    signature,
                    txn,
                    &mut structural_parents,
                    &mut path_children,
                    &mut indexed_work,
                )?;
                let index = usize::try_from(*child_index)
                    .map_err(|_| invalid_action_range(request_id, action.operation_index()))?;
                if index > state.virtual_len
                    || nodes.is_empty()
                    || (state.last_index == Some(index) && !state.last_was_delete)
                {
                    return Err(invalid_action_range(request_id, action.operation_index()));
                }
                let mut expected_index = *child_index;
                for child in nodes {
                    if child.index != expected_index {
                        return Err(invalid_action_range(request_id, action.operation_index()));
                    }
                    expected_index = expected_index.checked_add(1).ok_or_else(|| {
                        invalid_action_range(request_id, action.operation_index())
                    })?;
                }
                state.virtual_len = state
                    .virtual_len
                    .checked_add(nodes.len())
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                state.last_index = Some(index);
                state.last_was_delete = false;
                indexed_work = indexed_work
                    .checked_add(nodes.len())
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                continue;
            }
            _ => {}
        }
        if let YrsMutationAction::CreateText {
            parent,
            child_index,
            text,
            scalar_len,
            len_utf16,
            follow_up,
            signature,
            ..
        } = action
        {
            if !path_children.contains_key(&signature.parent) {
                indexed_work = indexed_work
                    .checked_add(signature.path.len())
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                validate_parent_identity(
                    request_id,
                    action.operation_index(),
                    parent,
                    signature,
                    txn,
                    &mut path_children,
                )?;
            }
            validate_parent_gap(
                request_id,
                action.operation_index(),
                signature,
                &path_children[&signature.parent],
            )?;
            indexed_work = indexed_work
                .checked_add(2)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let fenwick_len = usize::try_from(signature.child_count)
                .ok()
                .and_then(|len| len.checked_add(2))
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let gap = usize::try_from(signature.initial_child_index)
                .ok()
                .and_then(|index| index.checked_add(1))
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let creates_index_is_new = !created_gaps.contains_key(&signature.parent);
            let prior = created_gaps
                .entry(signature.parent.clone())
                .or_insert_with(|| vec![0; fenwick_len]);
            let shift = fenwick_prefix(prior, gap)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            fenwick_add(prior, gap)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            indexed_work = indexed_work
                .checked_add(
                    binary_partition_work(fenwick_len)
                        .checked_mul(2)
                        .and_then(|work| {
                            work.checked_add(if creates_index_is_new { fenwick_len } else { 0 })
                        })
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?,
                )
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let expected_execution_index = signature
                .initial_child_index
                .checked_add(shift)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            if *child_index != expected_execution_index {
                return Err(invalid_action_range(request_id, action.operation_index()));
            }
            if text.is_empty() || *scalar_len == 0 || *len_utf16 == 0 {
                return Err(invalid_action_range(request_id, action.operation_index()));
            }
            let mut length = *len_utf16;
            for follow in follow_up {
                let operation_index = follow.operation_index();
                match follow {
                    CreatedTextAction::Insert {
                        index_utf16,
                        len_utf16,
                        ..
                    } => {
                        if *index_utf16 > length || *len_utf16 == 0 {
                            return Err(invalid_action_range(request_id, operation_index));
                        }
                        length = length
                            .checked_add(*len_utf16)
                            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
                    }
                    CreatedTextAction::Delete {
                        index_utf16,
                        len_utf16,
                        ..
                    }
                    | CreatedTextAction::Format {
                        index_utf16,
                        len_utf16,
                        ..
                    } => {
                        let end = index_utf16
                            .checked_add(*len_utf16)
                            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
                        if end > length {
                            return Err(invalid_action_range(request_id, operation_index));
                        }
                        if matches!(follow, CreatedTextAction::Delete { .. }) {
                            length -= *len_utf16;
                        }
                    }
                }
            }
            continue;
        }
        let target = action.target();
        let signature = action.signature();
        if validated_targets.insert(signature.target.clone()) {
            indexed_work = indexed_work
                .checked_add(signature.path.len())
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            let signature_work = validate_signature(
                request_id,
                action.operation_index(),
                target,
                signature,
                txn,
                &mut path_children,
            )?;
            indexed_work = indexed_work
                .checked_add(signature_work)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
        }
        let length = virtual_lengths
            .entry(signature.target.clone())
            .or_insert(signature.initial_len_utf16);
        match action {
            YrsMutationAction::CreateText { .. }
            | YrsMutationAction::DeleteXmlChildren { .. }
            | YrsMutationAction::InsertXmlChildren { .. }
            | YrsMutationAction::SetXmlAttribute { .. }
            | YrsMutationAction::RemoveXmlAttribute { .. } => unreachable!(),
            YrsMutationAction::InsertText {
                index_utf16,
                len_utf16,
                ..
            } => {
                if *index_utf16 > *length {
                    return Err(invalid_action_range(request_id, action.operation_index()));
                }
                *length = length
                    .checked_add(*len_utf16)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::DeleteText {
                index_utf16,
                len_utf16,
                ..
            }
            | YrsMutationAction::FormatText {
                index_utf16,
                len_utf16,
                ..
            } => {
                let end = index_utf16
                    .checked_add(*len_utf16)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                if end > *length {
                    return Err(invalid_action_range(request_id, action.operation_index()));
                }
                if matches!(action, YrsMutationAction::DeleteText { .. }) {
                    *length -= *len_utf16;
                }
            }
        }
    }
    let materialized_children = path_children
        .values()
        .try_fold(0usize, |total, children| total.checked_add(children.len()))
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs preflight child-index work overflow",
            )
        })?;
    let preflight_work = indexed_work
        .checked_add(materialized_children)
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs preflight indexed work overflow",
            )
        })?;
    Ok(preflight_work)
}

#[derive(Debug)]
struct StructuralPreflightState {
    virtual_len: usize,
    last_index: Option<usize>,
    last_was_delete: bool,
}

#[derive(Clone, Copy)]
struct PreflightActionContext {
    request_id: u64,
    operation_index: usize,
}

#[allow(clippy::too_many_arguments)]
fn preflight_attribute_action<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlElementRef,
    key: &Arc<str>,
    set_value: Option<&Any>,
    signature: &ElementSignature,
    txn: &T,
    validated_elements: &mut std::collections::HashSet<BranchID>,
    last_attribute_keys: &mut HashMap<BranchID, Arc<str>>,
    path_children: &mut HashMap<BranchID, Vec<BranchID>>,
    indexed_work: &mut usize,
) -> OperationResult<()> {
    if validated_elements.insert(signature.target.clone()) {
        let attribute_work = validate_element_signature(
            request_id,
            operation_index,
            target,
            signature,
            txn,
            path_children,
        )?;
        *indexed_work = indexed_work
            .checked_add(signature.path.len())
            .and_then(|work| work.checked_add(attribute_work))
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    }
    if last_attribute_keys
        .get(&signature.target)
        .is_some_and(|previous| previous.as_ref() >= key.as_ref())
    {
        return Err(invalid_action_range(request_id, operation_index));
    }
    let previous = signature
        .attrs
        .binary_search_by(|(candidate, _)| candidate.as_ref().cmp(key.as_ref()))
        .ok()
        .map(|index| &signature.attrs[index].1);
    if set_value.is_some_and(|value| previous == Some(value))
        || (set_value.is_none() && previous.is_none())
    {
        return Err(invalid_action_range(request_id, operation_index));
    }
    last_attribute_keys.insert(signature.target.clone(), key.clone());
    *indexed_work = indexed_work
        .checked_add(1)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    Ok(())
}

fn validate_element_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlElementRef,
    signature: &ElementSignature,
    txn: &T,
    path_children: &mut HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<usize> {
    let path_matches = expected_path_matches(
        signature.target.clone(),
        target.parent(),
        &signature.path,
        txn,
        path_children,
    );
    if !path_matches {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "XML attribute target path changed before execution",
        ));
    }
    let mut attrs = Vec::new();
    let mut work = 0usize;
    for (key, value) in target.attributes(txn) {
        let yrs::Out::Any(value) = value else {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "XML attribute resolved to a non-Any shared value",
            ));
        };
        work = work
            .checked_add(key.len())
            .and_then(|work| work.checked_add(any_preflight_work(&value)?))
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        attrs.push((Arc::<str>::from(key), value));
    }
    let sort_partitions = binary_partition_work(attrs.len());
    let sort_key_work = attrs.iter().try_fold(0usize, |total, (key, _)| {
        total.checked_add(key.len().checked_mul(sort_partitions)?)
    });
    work = work
        .checked_add(
            attrs
                .len()
                .checked_mul(sort_partitions)
                .ok_or_else(|| invalid_action_range(request_id, operation_index))?,
        )
        .and_then(|work| work.checked_add(sort_key_work?))
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
    if AsRef::<Branch>::as_ref(target).id() != signature.target
        || target.tag().as_ref() != signature.tag.as_ref()
        || attrs != signature.attrs
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "XML attribute target changed before execution",
        ));
    }
    Ok(work)
}

pub(super) fn any_preflight_work(root: &Any) -> Option<usize> {
    let mut work = 0usize;
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        work = work.checked_add(1)?;
        match value {
            Any::String(value) => work = work.checked_add(value.len())?,
            Any::Buffer(value) => work = work.checked_add(value.len())?,
            Any::Array(values) => {
                work = work.checked_add(values.len())?;
                stack.extend(values.iter());
            }
            Any::Map(values) => {
                work = work.checked_add(values.len())?;
                for (key, value) in values.iter() {
                    work = work.checked_add(key.len())?;
                    stack.push(value);
                }
            }
            Any::Null | Any::Undefined | Any::Bool(_) | Any::Number(_) | Any::BigInt(_) => {}
        }
    }
    Some(work)
}

pub(super) fn capture_text_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: Option<usize>,
    target: &XmlTextRef,
    txn: &T,
) -> OperationResult<(Vec<TextSignatureRun>, usize)> {
    let mut runs = Vec::<TextSignatureRun>::new();
    let mut work = 0usize;
    for diff in target.diff(txn, yrs::types::text::YChange::identity) {
        let yrs::Out::Any(Any::String(value)) = diff.insert else {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                operation_index,
                "Yrs XML text signature contains a non-string value",
            ));
        };
        let text = value.to_string();
        if text.is_empty() {
            continue;
        }
        let mut attrs = Vec::new();
        if let Some(diff_attrs) = diff.attributes {
            work = work
                .checked_add(diff_attrs.len())
                .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
            for (key, value) in diff_attrs.iter() {
                work = work
                    .checked_add(key.len())
                    .and_then(|work| work.checked_add(any_preflight_work(value)?))
                    .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
                attrs.push((key.clone(), value.clone()));
            }
            let sort_partitions = binary_partition_work(attrs.len());
            let sort_key_work = attrs.iter().try_fold(0usize, |total, (key, _)| {
                total.checked_add(key.len().checked_mul(sort_partitions)?)
            });
            let sort_work = attrs
                .len()
                .checked_mul(sort_partitions)
                .and_then(|work| work.checked_add(sort_key_work?))
                .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
            work = work
                .checked_add(sort_work)
                .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
            attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
        }
        work = work
            .checked_add(text.len())
            .and_then(|work| work.checked_add(1))
            .ok_or_else(|| text_signature_overflow(request_id, operation_index))?;
        if let Some(previous) = runs.last_mut().filter(|run| run.attrs == attrs) {
            previous.text.push_str(&text);
        } else {
            runs.push(TextSignatureRun { text, attrs });
        }
    }
    Ok((runs, work))
}

fn text_signature_work(runs: &[TextSignatureRun]) -> Option<usize> {
    runs.iter().try_fold(0usize, |work, run| {
        run.attrs.iter().try_fold(
            work.checked_add(1)?.checked_add(run.text.len())?,
            |work, (key, value)| {
                work.checked_add(key.len())?
                    .checked_add(any_preflight_work(value)?)
            },
        )
    })
}

fn text_signature_overflow(request_id: u64, operation_index: Option<usize>) -> OperationError {
    OperationError::engine_invariant_failed(
        request_id,
        operation_index,
        "Yrs XML text signature work overflow",
    )
}

fn validate_structural_parent<'a, T: ReadTxn>(
    context: PreflightActionContext,
    parent: &XmlParentRef,
    signature: &StructuralParentSignature,
    txn: &T,
    parents: &'a mut HashMap<BranchID, StructuralPreflightState>,
    path_children: &mut HashMap<BranchID, Vec<BranchID>>,
    indexed_work: &mut usize,
) -> OperationResult<&'a mut StructuralPreflightState> {
    let PreflightActionContext {
        request_id,
        operation_index,
    } = context;
    if !parents.contains_key(&signature.parent) {
        // A remotely replaced nested parent may already be detached and
        // garbage-collected. In that state Yrs' Branch::id() panics, while
        // parent() safely reports that the recorded path no longer exists.
        // Validate reachability first and only derive the branch ID after it
        // is known to remain attached to the expected document path.
        if !expected_path_matches(
            signature.parent.clone(),
            parent.parent(),
            &signature.path,
            txn,
            path_children,
        ) || parent.id() != signature.parent
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "structural mutation parent identity changed before execution",
            ));
        }
        let actual = parent.children(txn);
        *indexed_work = indexed_work
            .checked_add(signature.path.len())
            .and_then(|work| work.checked_add(actual.len()))
            .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
        if actual != signature.children {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "structural mutation parent children changed before execution",
            ));
        }
        parents.insert(
            signature.parent.clone(),
            StructuralPreflightState {
                virtual_len: actual.len(),
                last_index: None,
                last_was_delete: false,
            },
        );
    }
    parents
        .get_mut(&signature.parent)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))
}

#[cfg(test)]
pub(crate) fn preflight_mutation_work_for_test<T: ReadTxn>(
    _request_id: u64,
    plan: &YrsMutationPlan,
    _txn: &T,
) -> OperationResult<usize> {
    Ok(plan.expected_preflight_work)
}

#[allow(dead_code)] // Task 7 calls this after installing atomic production application.
fn validate_signature<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    target: &XmlTextRef,
    expected: &TargetSignature,
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<usize> {
    let branch = <XmlTextRef as AsRef<Branch>>::as_ref(target);
    let path_matches = expected_path_matches(
        expected.target.clone(),
        target.parent(),
        &expected.path,
        txn,
        path_children,
    );
    if !path_matches {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved Yrs XML text target path changed before mutation",
        ));
    }
    let actual_len = Some(Text::len(target, txn));
    let (actual_runs, actual_work) =
        capture_text_signature(request_id, Some(operation_index), target, txn)?;
    let compare_work = text_signature_work(&expected.runs)
        .and_then(|work| work.checked_add(actual_work))
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    if branch.is_deleted()
        || branch.id() != expected.target
        || actual_len != Some(expected.initial_len_utf16)
        || actual_runs != expected.runs
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            format!(
                "resolved Yrs XML text target signature changed before mutation (deleted={}, id_match={}, path_match={}, content_match={}, expected_utf16={}, actual_len={actual_len:?})",
                branch.is_deleted(),
                branch.id() == expected.target,
                path_matches,
                actual_runs == expected.runs,
                expected.initial_len_utf16,
            ),
        ));
    }
    Ok(compare_work)
}

fn validate_parent_identity<T: ReadTxn>(
    request_id: u64,
    operation_index: usize,
    parent: &XmlElementRef,
    expected: &ParentSignature,
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> OperationResult<()> {
    let branch = <XmlElementRef as AsRef<Branch>>::as_ref(parent);
    let path_matches = expected_path_matches(
        expected.parent.clone(),
        parent.parent(),
        &expected.path,
        txn,
        path_children,
    );
    if !path_matches {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock path changed before mutation",
        ));
    }
    if branch.is_deleted() || branch.id() != expected.parent || parent.tag() != &expected.tag {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock signature changed before mutation",
        ));
    }
    path_children
        .entry(branch.id())
        .or_insert_with(|| parent.children(txn).map(|child| child.id()).collect());
    Ok(())
}

fn validate_parent_gap(
    request_id: u64,
    operation_index: usize,
    expected: &ParentSignature,
    children: &[BranchID],
) -> OperationResult<()> {
    let actual_child_count = u32::try_from(children.len()).ok();
    let left_neighbor = expected
        .initial_child_index
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| children.get(index))
        .cloned();
    let right_neighbor = usize::try_from(expected.initial_child_index)
        .ok()
        .and_then(|index| children.get(index))
        .cloned();
    if actual_child_count != Some(expected.child_count)
        || expected.initial_child_index > expected.child_count
        || left_neighbor != expected.left_neighbor
        || right_neighbor != expected.right_neighbor
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved empty Yrs textblock gap signature changed before mutation",
        ));
    }
    Ok(())
}

pub(super) fn invalid_action_range(request_id: u64, operation_index: usize) -> OperationError {
    OperationError::engine_invariant_failed(
        request_id,
        Some(operation_index),
        "resolved Yrs mutation action is outside its preflighted UTF-16 target range",
    )
}

fn expected_path_matches<T: ReadTxn>(
    mut child_id: BranchID,
    mut parent: Option<XmlOut>,
    expected: &[(BranchID, u32)],
    txn: &T,
    path_children: &mut std::collections::HashMap<BranchID, Vec<BranchID>>,
) -> bool {
    for (expected_parent, expected_index) in expected {
        let Some(node) = parent else {
            return false;
        };
        if node.id() != *expected_parent {
            return false;
        }
        let children = path_children
            .entry(node.id())
            .or_insert_with(|| match &node {
                XmlOut::Element(element) => element.children(txn).map(|child| child.id()).collect(),
                XmlOut::Fragment(fragment) => {
                    fragment.children(txn).map(|child| child.id()).collect()
                }
                XmlOut::Text(_) => Vec::new(),
            });
        let expected_index = match usize::try_from(*expected_index) {
            Ok(index) => index,
            Err(_) => return false,
        };
        if children.get(expected_index) != Some(&child_id) {
            return false;
        }
        child_id = node.id();
        parent = match node {
            XmlOut::Element(element) => element.parent(),
            XmlOut::Fragment(fragment) => fragment.parent(),
            XmlOut::Text(text) => text.parent(),
        };
    }
    parent.is_none()
}
