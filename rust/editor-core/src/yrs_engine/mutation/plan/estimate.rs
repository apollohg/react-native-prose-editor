pub(crate) fn estimate_update_v1_growth(
    request_id: u64,
    plan: &YrsMutationPlan,
    envelope: Option<&CrdtEnvelope>,
) -> OperationResult<usize> {
    let mut total = 0usize;
    for (action_index, action) in plan.actions.iter().enumerate() {
        // Includes worst-case client/clock varints, parent/type refs, item headers,
        // content lengths, format sentinels and delete-set clocks.
        let mut action_bytes = 512usize;
        match action {
            YrsMutationAction::DeleteXmlChildren { child_count, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    usize::try_from(*child_count)
                        .ok()
                        .and_then(|count| count.checked_mul(64)),
                )?;
            }
            YrsMutationAction::InsertXmlChildren { .. } => {
                let inserted = plan
                    .prepared_metrics
                    .get(action_index)
                    .and_then(Option::as_ref)
                    .map(|metrics| metrics.growth_bytes)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    Some(inserted),
                )?;
            }
            YrsMutationAction::SetXmlAttribute { key, value, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    key.len()
                        .checked_mul(2)
                        .and_then(|bytes| bytes.checked_add(any_growth(value)?))
                        .and_then(|bytes| bytes.checked_add(128)),
                )?;
            }
            YrsMutationAction::RemoveXmlAttribute { key, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    key.len()
                        .checked_mul(2)
                        .and_then(|bytes| bytes.checked_add(128)),
                )?;
            }
            YrsMutationAction::CreateText {
                text,
                attrs,
                follow_up,
                ..
            } => {
                // XML type creation needs a parent reference and its own item header in
                // addition to the subsequently inserted attributed text items.
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    Some(512),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    text.len().checked_mul(4),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(8)),
                )?;
                for follow in follow_up {
                    action_bytes = estimate_created_text_action(request_id, action_bytes, follow)?;
                }
            }
            YrsMutationAction::InsertText { text, attrs, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    text.len().checked_mul(4),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(8)),
                )?;
            }
            YrsMutationAction::DeleteText { len_utf16, .. } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    usize::try_from(*len_utf16)
                        .ok()
                        .and_then(|len| len.checked_mul(16)),
                )?;
            }
            YrsMutationAction::FormatText {
                len_utf16, attrs, ..
            } => {
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    usize::try_from(*len_utf16)
                        .ok()
                        .and_then(|len| len.checked_mul(16)),
                )?;
                action_bytes = checked_estimate_add(
                    request_id,
                    action.operation_index(),
                    action_bytes,
                    attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(16)),
                )?;
            }
        }
        total = checked_estimate_add(
            request_id,
            action.operation_index(),
            total,
            Some(action_bytes),
        )?;
    }
    if plan_has_deletions(plan) {
        let inserted_units = planned_insertion_units(request_id, plan)?;
        let live_units = envelope.map_or(0, |envelope| envelope.live_clock_units);
        if plan_may_delete_live(plan) && envelope.is_none() {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs live deletion estimate requires a transaction snapshot envelope",
            ));
        }
        let delete_units = live_units
            .checked_add(inserted_units)
            .ok_or_else(|| invalid_action_range(request_id, 0))?;
        let possible_clients = u64::try_from(envelope.map_or(0, |value| value.client_count))
            .unwrap_or(u64::MAX)
            .saturating_add(1)
            .min(delete_units);
        // Update-v1 delete sets encode a client count, then for every client a
        // client id and range count, and for every range a clock and length.
        // Ten bytes bounds a u64 client varint; five bounds every u32 varint.
        // One deleted clock per range is the maximally fragmented case.
        let delete_set_bytes = 5u64
            .checked_add(
                possible_clients
                    .checked_mul(15)
                    .ok_or_else(|| invalid_action_range(request_id, 0))?,
            )
            .and_then(|bytes| bytes.checked_add(delete_units.checked_mul(10)?))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or_else(|| invalid_action_range(request_id, 0))?;
        total = checked_estimate_add(request_id, 0, total, Some(delete_set_bytes))?;
    }
    Ok(total)
}
pub(crate) fn estimate_undo_units(
    request_id: u64,
    plan: &YrsMutationPlan,
    envelope: Option<&CrdtEnvelope>,
) -> OperationResult<u64> {
    let inserted = planned_insertion_units(request_id, plan)?;
    if !plan_has_deletions(plan) {
        return Ok(inserted);
    }
    if plan_may_delete_live(plan) && envelope.is_none() {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "Yrs live deletion estimate requires a transaction snapshot envelope",
        ));
    }
    // A plan can delete every currently-live clock plus clocks it inserted
    // earlier in the same transaction. Count insertions separately because an
    // UndoManager stack item retains both its insertion and deletion IdSets.
    inserted
        .checked_add(envelope.map_or(0, |value| value.live_clock_units))
        .and_then(|units| units.checked_add(inserted))
        .ok_or_else(|| invalid_action_range(request_id, 0))
}

fn plan_has_deletions(plan: &YrsMutationPlan) -> bool {
    plan.actions.iter().any(|action| match action {
        YrsMutationAction::DeleteXmlChildren { .. }
        | YrsMutationAction::RemoveXmlAttribute { .. }
        | YrsMutationAction::DeleteText { .. }
        | YrsMutationAction::FormatText { .. } => true,
        YrsMutationAction::SetXmlAttribute { key, signature, .. } => signature
            .attrs
            .binary_search_by(|(candidate, _)| candidate.as_ref().cmp(key.as_ref()))
            .is_ok(),
        YrsMutationAction::CreateText { follow_up, .. } => follow_up.iter().any(|follow| {
            matches!(
                follow,
                CreatedTextAction::Delete { .. } | CreatedTextAction::Format { .. }
            )
        }),
        YrsMutationAction::InsertXmlChildren { .. } | YrsMutationAction::InsertText { .. } => false,
    })
}

fn plan_may_delete_live(plan: &YrsMutationPlan) -> bool {
    plan.actions.iter().any(|action| match action {
        YrsMutationAction::DeleteXmlChildren { .. }
        | YrsMutationAction::RemoveXmlAttribute { .. }
        | YrsMutationAction::DeleteText { .. } => true,
        YrsMutationAction::SetXmlAttribute { key, signature, .. } => signature
            .attrs
            .binary_search_by(|(candidate, _)| candidate.as_ref().cmp(key.as_ref()))
            .is_ok(),
        YrsMutationAction::FormatText { .. } => true,
        YrsMutationAction::CreateText { .. }
        | YrsMutationAction::InsertXmlChildren { .. }
        | YrsMutationAction::InsertText { .. } => false,
    })
}

fn planned_insertion_units(request_id: u64, plan: &YrsMutationPlan) -> OperationResult<u64> {
    fn format_units(attrs: &Attrs) -> Option<u64> {
        u64::try_from(attrs.len()).ok()?.checked_mul(2)
    }

    let mut total = 0u64;
    for (index, action) in plan.actions.iter().enumerate() {
        let units = match action {
            YrsMutationAction::DeleteXmlChildren { .. }
            | YrsMutationAction::RemoveXmlAttribute { .. }
            | YrsMutationAction::DeleteText { .. } => 0,
            YrsMutationAction::InsertXmlChildren { .. } => plan
                .prepared_metrics
                .get(index)
                .and_then(Option::as_ref)
                .map(|metrics| metrics.insertion_units)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?,
            YrsMutationAction::SetXmlAttribute { .. } => 1,
            YrsMutationAction::CreateText {
                len_utf16,
                attrs,
                follow_up,
                ..
            } => {
                let mut units = 1u64
                    .checked_add(u64::from(*len_utf16))
                    .and_then(|units| units.checked_add(format_units(attrs)?))
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
                for follow in follow_up {
                    let added = match follow {
                        CreatedTextAction::Insert {
                            len_utf16, attrs, ..
                        } => {
                            let formats = format_units(attrs).ok_or_else(|| {
                                invalid_action_range(request_id, follow.operation_index())
                            })?;
                            u64::from(*len_utf16).checked_add(formats).ok_or_else(|| {
                                invalid_action_range(request_id, follow.operation_index())
                            })?
                        }
                        CreatedTextAction::Delete { .. } => 0,
                        CreatedTextAction::Format { attrs, .. } => {
                            format_units(attrs).ok_or_else(|| {
                                invalid_action_range(request_id, follow.operation_index())
                            })?
                        }
                    };
                    units = units.checked_add(added).ok_or_else(|| {
                        invalid_action_range(request_id, follow.operation_index())
                    })?;
                }
                units
            }
            YrsMutationAction::InsertText {
                len_utf16, attrs, ..
            } => {
                u64::from(*len_utf16)
                    .checked_add(format_units(attrs).ok_or_else(|| {
                        invalid_action_range(request_id, action.operation_index())
                    })?)
                    .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?
            }
            YrsMutationAction::FormatText { attrs, .. } => format_units(attrs)
                .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?,
        };
        total = total
            .checked_add(units)
            .ok_or_else(|| invalid_action_range(request_id, action.operation_index()))?;
    }
    Ok(total)
}

fn checked_estimate_add(
    request_id: u64,
    operation_index: usize,
    current: usize,
    amount: Option<usize>,
) -> OperationResult<usize> {
    amount
        .and_then(|amount| current.checked_add(amount))
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "estimatedUpdateV1Growth",
                u64::MAX,
                u64::MAX,
            )
        })
}

fn estimate_created_text_action(
    request_id: u64,
    mut current: usize,
    action: &CreatedTextAction,
) -> OperationResult<usize> {
    current = checked_estimate_add(request_id, action.operation_index(), current, Some(512))?;
    match action {
        CreatedTextAction::Insert { text, attrs, .. } => {
            current = checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                text.len().checked_mul(4),
            )?;
            checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(8)),
            )
        }
        CreatedTextAction::Delete { len_utf16, .. } => checked_estimate_add(
            request_id,
            action.operation_index(),
            current,
            usize::try_from(*len_utf16)
                .ok()
                .and_then(|len| len.checked_mul(16)),
        ),
        CreatedTextAction::Format {
            len_utf16, attrs, ..
        } => {
            current = checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                usize::try_from(*len_utf16)
                    .ok()
                    .and_then(|len| len.checked_mul(16)),
            )?;
            checked_estimate_add(
                request_id,
                action.operation_index(),
                current,
                attrs_estimate(attrs).and_then(|bytes| bytes.checked_mul(16)),
            )
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct PreparedMetrics {
    growth_bytes: usize,
    insertion_units: u64,
    work: usize,
}

fn prepared_nodes_metrics(nodes: &[PreparedXmlChild]) -> Option<PreparedMetrics> {
    fn node_metrics(node: &PreparedXmlNode) -> Option<PreparedMetrics> {
        match node {
            PreparedXmlNode::Text { runs } => {
                let mut metrics = PreparedMetrics {
                    growth_bytes: 128,
                    insertion_units: 1,
                    work: 1,
                };
                for run in runs {
                    metrics.growth_bytes = metrics
                        .growth_bytes
                        .checked_add(run.text.len().checked_mul(4)?)?
                        .checked_add(64)?;
                    metrics.insertion_units = metrics
                        .insertion_units
                        .checked_add(u64::try_from(run.text.encode_utf16().count()).ok()?)?
                        .checked_add(u64::try_from(run.attrs.len()).ok()?.checked_mul(2)?)?;
                    metrics.work = metrics.work.checked_add(1)?.checked_add(run.text.len())?;
                    for (key, value) in run.attrs.iter() {
                        metrics.growth_bytes = metrics
                            .growth_bytes
                            .checked_add(key.len().checked_mul(2)?)?
                            .checked_add(any_growth(value)?)?
                            .checked_add(64)?;
                        metrics.work = metrics
                            .work
                            .checked_add(key.len())?
                            .checked_add(any_preflight_work(value)?)?;
                    }
                }
                Some(metrics)
            }
            PreparedXmlNode::Element {
                tag,
                attrs,
                children,
            } => {
                let mut metrics = PreparedMetrics {
                    growth_bytes: tag.len().checked_mul(2)?.checked_add(128)?,
                    insertion_units: 1u64.checked_add(u64::try_from(attrs.len()).ok()?)?,
                    work: 1usize.checked_add(tag.len())?,
                };
                for (key, value) in attrs {
                    metrics.growth_bytes = metrics
                        .growth_bytes
                        .checked_add(key.len().checked_mul(2)?)?
                        .checked_add(any_growth(value)?)?
                        .checked_add(64)?;
                    metrics.work = metrics
                        .work
                        .checked_add(key.len())?
                        .checked_add(any_preflight_work(value)?)?;
                }
                for child in children {
                    let child = node_metrics(&child.node)?;
                    metrics.growth_bytes = metrics.growth_bytes.checked_add(child.growth_bytes)?;
                    metrics.insertion_units =
                        metrics.insertion_units.checked_add(child.insertion_units)?;
                    metrics.work = metrics.work.checked_add(child.work)?;
                }
                Some(metrics)
            }
        }
    }

    nodes
        .iter()
        .try_fold(PreparedMetrics::default(), |mut total, child| {
            let child = node_metrics(&child.node)?;
            total.growth_bytes = total.growth_bytes.checked_add(child.growth_bytes)?;
            total.insertion_units = total.insertion_units.checked_add(child.insertion_units)?;
            total.work = total.work.checked_add(child.work)?;
            Some(total)
        })
}

fn any_growth(value: &Any) -> Option<usize> {
    match value {
        Any::Null | Any::Undefined | Any::Bool(_) | Any::Number(_) | Any::BigInt(_) => Some(16),
        Any::String(value) => value.len().checked_mul(2)?.checked_add(16),
        Any::Buffer(value) => value.len().checked_mul(2)?.checked_add(16),
        Any::Array(values) => values.iter().try_fold(24usize, |total, value| {
            total.checked_add(any_growth(value)?)
        }),
        Any::Map(values) => values.iter().try_fold(24usize, |total, (key, value)| {
            total
                .checked_add(key.len().checked_mul(2)?)?
                .checked_add(any_growth(value)?)
        }),
    }
}

fn attrs_estimate(attrs: &Attrs) -> Option<usize> {
    attrs.iter().try_fold(0usize, |total, (key, value)| {
        total
            .checked_add(key.len())
            .and_then(|bytes| bytes.checked_add(value.to_string().len()))
            .and_then(|bytes| bytes.checked_add(32))
    })
}

pub(super) fn attrs_work(attrs: &Attrs) -> usize {
    attrs
        .iter()
        .try_fold(0usize, |total, (key, value)| {
            total
                .checked_add(1)
                .and_then(|work| work.checked_add(key.len()))
                .and_then(|work| work.checked_add(value.to_string().len()))
        })
        .unwrap_or(usize::MAX)
}
