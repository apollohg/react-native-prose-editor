use crate::model::{Document, Node};
use crate::position::PositionMap;
use crate::schema::content_rule::WorkBudget;
use crate::schema::Schema;
use crate::yrs_engine;
use crate::yrs_engine::{OperationError, OperationResult};
use smallvec::SmallVec;

#[derive(Clone, Copy)]
pub(super) struct InsertNodeResolverContext<'a> {
    pub(super) request_id: u64,
    pub(super) operation_index: usize,
    pub(super) document: &'a Document,
    pub(super) node: &'a Node,
    pub(super) schema: &'a Schema,
    pub(super) budget: &'a WorkBudget,
    pub(super) limit: usize,
}

pub(super) fn resolve_insert_node_position(
    context: InsertNodeResolverContext<'_>,
    position: yrs_engine::RevisionedPosition,
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
                yrs_engine::Affinity::Before => {
                    candidates.push((block.node_path.as_slice(), true));
                    candidates.push((next.node_path.as_slice(), false));
                    candidates.push((next.node_path.as_slice(), true));
                }
                yrs_engine::Affinity::After => {
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
        if scalar_offset == content_start && position.affinity == yrs_engine::Affinity::Before {
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
        if scalar_offset == content_end && position.affinity == yrs_engine::Affinity::After {
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
pub(super) fn resolve_insert_node_base_position(
    request_id: u64,
    operation_index: usize,
    position: yrs_engine::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
    resolver_budget: &WorkBudget,
    resolver_limit: usize,
) -> OperationResult<(u32, u32, Option<usize>)> {
    let scalar_offset = match position.kind {
        yrs_engine::EditorOffsetKind::Scalar => Some(position.offset),
        yrs_engine::EditorOffsetKind::Utf16 => utf16_offset_to_scalar_budgeted(
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

pub(super) fn utf16_offset_to_scalar_budgeted(
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

pub(super) fn normalize_current_insert_node_position(
    context: InsertNodeResolverContext<'_>,
    position: u32,
    affinity: yrs_engine::Affinity,
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
            (true, true, yrs_engine::Affinity::After) => [true, false],
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

pub(super) fn insert_node_is_block(node: &Node, schema: &Schema) -> bool {
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

pub(super) fn insertion_is_schema_valid(
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

pub(super) fn resolver_budget_exhausted(
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

pub(super) fn consume_resolver_work(
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

pub(super) struct InsertResolvedPosition<'a> {
    pub(super) node_path: SmallVec<[u32; 8]>,
    pub(super) parent_offset: u32,
    pub(super) parent: &'a Node,
}

pub(super) fn resolve_insert_position_budgeted<'a>(
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
pub(super) fn document_position_at_path_budgeted(
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

pub(super) fn resolver_node_size(
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
