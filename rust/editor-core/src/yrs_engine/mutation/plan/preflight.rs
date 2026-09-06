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
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?;
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
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?;
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
                    .checked_add(
                        usize::try_from(*child_count).map_err(|_| {
                            invalid_action_range(request_id, action.operation_index())
                        })?,
                    )
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
            }
            YrsMutationAction::InsertXmlChildren {
                nodes, signature, ..
            } => {
                if structural_parents.insert(signature.parent.clone()) {
                    indexed_work = indexed_work
                        .checked_add(signature.path.len())
                        .and_then(|work| work.checked_add(signature.children.len()))
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?;
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
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?;
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
                    indexed_work = indexed_work.checked_add(2).ok_or_else(|| {
                        invalid_action_range(request_id, action.operation_index())
                    })?;
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
                        .ok_or_else(|| {
                            invalid_action_range(request_id, action.operation_index())
                        })?;
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
    indexed_work.checked_add(materialized_work).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "sealed Yrs preflight work overflow",
        )
    })
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

include!("preflight/signatures.rs");
