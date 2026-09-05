use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Node};
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::transform::StepMap;
use crate::yrs_engine;
use crate::yrs_engine::{
    editor_offset_to_doc_pos, Affinity, OperationError, OperationResult, RevisionedRange,
};

pub(super) fn resolve_position(
    request_id: u64,
    operation_index: Option<usize>,
    field: &'static str,
    position: yrs_engine::RevisionedPosition,
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

pub(super) fn resolve_structural_window(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    replacement: &yrs_engine::StructuralReplacement,
    limits: &ResourceLimits,
) -> OperationResult<(u32, u32)> {
    if replacement.parent_path().len() > limits.max_document_depth {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            Some(operation_index),
            "maxDocumentDepth",
            u64::try_from(limits.max_document_depth).unwrap_or(u64::MAX),
            u64::try_from(replacement.parent_path().len()).unwrap_or(u64::MAX),
        ));
    }
    let mut node = document.root();
    let mut content_start = 0u32;
    let mut work = 0usize;
    for path_index in replacement.parent_path().iter().copied() {
        work = work.saturating_add(1);
        if work > limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "maxDocumentNodes",
                u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(work).unwrap_or(u64::MAX),
            ));
        }
        let content = node.content().ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target parent has no child content",
            )
        })?;
        let index = usize::try_from(path_index).map_err(|_| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target path index is not representable",
            )
        })?;
        for sibling in content.iter().take(index) {
            work = work.saturating_add(1);
            if work > limits.max_document_nodes {
                return Err(OperationError::operation_limit_exceeded(
                    request_id,
                    Some(operation_index),
                    "maxDocumentNodes",
                    u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                    u64::try_from(work).unwrap_or(u64::MAX),
                ));
            }
            content_start = content_start
                .checked_add(sibling.node_size())
                .ok_or_else(|| {
                    OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "structure",
                        "structural target position overflowed",
                    )
                })?;
        }
        content_start = content_start.checked_add(1).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target position overflowed",
            )
        })?;
        node = content.child(index).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target path is outside the document",
            )
        })?;
    }
    let content = node.content().ok_or_else(|| {
        OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural target parent has no child content",
        )
    })?;
    let (from_child, to_child) = replacement.child_window();
    let from_child = usize::try_from(from_child).map_err(|_| {
        OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural child window is not representable",
        )
    })?;
    let to_child = usize::try_from(to_child).map_err(|_| {
        OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural child window is not representable",
        )
    })?;
    if from_child > to_child || to_child > content.child_count() {
        return Err(OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural child window is outside its parent",
        ));
    }
    let mut from = content_start;
    for sibling in content.iter().take(from_child) {
        work = work.saturating_add(1);
        if work > limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "maxDocumentNodes",
                u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(work).unwrap_or(u64::MAX),
            ));
        }
        from = from.checked_add(sibling.node_size()).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target position overflowed",
            )
        })?;
    }
    let mut to = from;
    for sibling in content.iter().skip(from_child).take(to_child - from_child) {
        work = work.saturating_add(1);
        if work > limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "maxDocumentNodes",
                u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(work).unwrap_or(u64::MAX),
            ));
        }
        to = to.checked_add(sibling.node_size()).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target position overflowed",
            )
        })?;
    }
    Ok((from, to))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_range(
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

pub(crate) fn map_position(map: &StepMap, mut position: u32, affinity: Affinity) -> u32 {
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

pub(super) fn resolve_attribute_target_position(
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

pub(super) fn resolve_join_target_position(
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

pub(super) fn direct_attribute_target(document: &Document, position: u32) -> Option<&Node> {
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

pub(super) fn node_boundary_position(root: &Node, path: &[u32]) -> Option<u32> {
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
