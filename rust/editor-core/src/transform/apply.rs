//! Step application logic.
//!
//! Each step produces a new `Document` (immutable tree transformation) and a
//! `StepMap` recording how positions shifted.

use std::collections::HashMap;

use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};

use super::mapping::StepMap;
use super::steps::{
    add_mark_to_set, merge_adjacent_text_nodes, rebuild_element, remove_mark_from_set,
    split_text_node,
};
use super::{Step, TransformError};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Apply a single step to a document, producing a new document and step map.
///
/// This does NOT validate the resulting document against the schema — that is
/// done once after all steps in a transaction have been applied.
pub fn apply_step(
    doc: &Document,
    step: &Step,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    match step {
        Step::InsertText { pos, text, marks } => apply_insert_text(doc, *pos, text, marks, schema),
        Step::DeleteRange { from, to } => apply_delete_range(doc, *from, *to),
        Step::AddMark { from, to, mark } => apply_add_mark(doc, *from, *to, mark),
        Step::RemoveMark {
            from,
            to,
            mark_type,
        } => apply_remove_mark(doc, *from, *to, mark_type),

        Step::SplitBlock {
            pos,
            node_type,
            attrs,
        } => apply_split_block(doc, *pos, node_type, attrs, schema),
        Step::JoinBlocks { pos } => apply_join_blocks(doc, *pos),
        Step::WrapInList {
            from,
            to,
            list_type,
            item_type,
            attrs,
        } => apply_wrap_in_list(doc, *from, *to, list_type, item_type, attrs, schema),
        Step::UnwrapFromList { pos } => apply_unwrap_from_list(doc, *pos, schema),

        Step::IndentListItem { pos } => apply_indent_list_item(doc, *pos, schema),

        Step::OutdentListItem { pos } => apply_outdent_list_item(doc, *pos, schema),
        Step::InsertNode { pos, node } => apply_insert_node(doc, *pos, node, schema),
        Step::ReplaceRange { from, to, content } => {
            apply_replace_range(doc, *from, *to, content, schema)
        }
    }
}

// ---------------------------------------------------------------------------
// InsertText
// ---------------------------------------------------------------------------

fn apply_insert_text(
    doc: &Document,
    pos: u32,
    insert_text: &str,
    marks: &[Mark],
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc
        .resolve(pos)
        .map_err(|e| TransformError::OutOfBounds(e))?;
    let parent = resolved.parent(doc);

    // The parent must be a text block (e.g. paragraph). Check via schema.
    let parent_spec = schema.node(parent.node_type());
    match parent_spec {
        Some(spec) => match spec.role {
            NodeRole::TextBlock => {} // OK
            _ => {
                return Err(TransformError::InvalidTarget(format!(
                    "cannot insert text into '{}' (role {:?}); text can only be inserted into text blocks",
                    parent.node_type(),
                    spec.role
                )));
            }
        },
        None => {
            // If the node type isn't in the schema, we can still proceed if
            // it has inline content, but be strict for now.
            return Err(TransformError::InvalidTarget(format!(
                "node type '{}' not found in schema",
                parent.node_type()
            )));
        }
    }

    let parent_offset = resolved.parent_offset;
    let insert_len = insert_text.chars().count() as u32;

    // Rebuild the parent node's children with the inserted text.
    let new_children = insert_text_in_children(parent, parent_offset, insert_text, marks);
    let new_parent = rebuild_element(parent, new_children);

    // Reconstruct the document by replacing the parent node at its path.
    let new_root = replace_node_at_path(doc.root(), &resolved.node_path, &new_parent);
    let new_doc = Document::new(new_root);
    let map = StepMap::from_insert(pos, insert_len);

    Ok((new_doc, map))
}

/// Insert text into a parent node's children at the given parent-content offset.
fn insert_text_in_children(
    parent: &Node,
    offset: u32,
    insert_text: &str,
    marks: &[Mark],
) -> Vec<Node> {
    let content = parent.content().expect("parent should be an element node");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count() + 2);
    let mut remaining_offset = offset;

    // If the parent has no children (empty paragraph), just insert the text.
    if content.child_count() == 0 {
        new_children.push(Node::text(insert_text.to_string(), marks.to_vec()));
        return merge_adjacent_text_nodes(new_children);
    }

    let mut inserted = false;

    for child in content.iter() {
        if inserted {
            new_children.push(child.clone());
            continue;
        }

        let child_size = child.node_size();

        if child.is_text() {
            if remaining_offset <= child_size {
                // Insert point is within (or at boundary of) this text node.
                let (left, right) = split_text_node(child, remaining_offset);

                if let Some(l) = left {
                    new_children.push(l);
                }

                new_children.push(Node::text(insert_text.to_string(), marks.to_vec()));

                if let Some(r) = right {
                    new_children.push(r);
                }

                inserted = true;
                continue;
            }
            new_children.push(child.clone());
            remaining_offset -= child_size;
        } else if child.is_void() {
            if remaining_offset == 0 {
                // Insert before this void node.
                new_children.push(Node::text(insert_text.to_string(), marks.to_vec()));
                new_children.push(child.clone());
                inserted = true;
                continue;
            }
            remaining_offset -= 1;
            new_children.push(child.clone());
        } else {
            // Nested element — for InsertText we don't descend into nested
            // elements here; the resolved position should already point to
            // the correct parent. Just skip this child.
            if remaining_offset == 0 {
                new_children.push(Node::text(insert_text.to_string(), marks.to_vec()));
                new_children.push(child.clone());
                inserted = true;
                continue;
            }
            remaining_offset -= child_size;
            new_children.push(child.clone());
        }
    }

    // If we haven't inserted yet, the offset is at the end.
    if !inserted {
        new_children.push(Node::text(insert_text.to_string(), marks.to_vec()));
    }

    merge_adjacent_text_nodes(new_children)
}

// ---------------------------------------------------------------------------
// DeleteRange
// ---------------------------------------------------------------------------

fn apply_delete_range(
    doc: &Document,
    from: u32,
    to: u32,
) -> Result<(Document, StepMap), TransformError> {
    if from > to {
        return Err(TransformError::InvalidRange(format!(
            "delete range from ({from}) is greater than to ({to})"
        )));
    }
    if from == to {
        // No-op deletion.
        return Ok((doc.clone(), StepMap::empty()));
    }

    let resolved_from = doc
        .resolve(from)
        .map_err(|e| TransformError::OutOfBounds(e))?;
    let resolved_to = doc
        .resolve(to)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    // If both endpoints are in the same parent, do the simple in-parent delete.
    if resolved_from.node_path == resolved_to.node_path {
        let parent = resolved_from.parent(doc);
        let from_offset = resolved_from.parent_offset;
        let to_offset = resolved_to.parent_offset;

        let new_children = delete_in_children(parent, from_offset, to_offset);
        let new_parent = rebuild_element(parent, new_children);

        let new_root = replace_node_at_path(doc.root(), &resolved_from.node_path, &new_parent);
        let new_doc = Document::new(new_root);
        let deleted_len = to - from;
        let map = StepMap::from_delete(from, deleted_len);

        return Ok((new_doc, map));
    }

    // Cross-parent deletion: endpoints resolve to different parents.
    // Handle the common case: both endpoints are in sibling blocks under
    // the same grandparent. Delete content from `from` to end of first
    // block, remove all intermediate blocks, delete content from start
    // of last block to `to`, then join the first and last blocks.
    apply_cross_parent_delete(doc, from, to, &resolved_from, &resolved_to)
}

/// Delete content in a parent node's children between `from_offset` and
/// `to_offset` (both relative to the parent's content start).
fn delete_in_children(parent: &Node, from_offset: u32, to_offset: u32) -> Vec<Node> {
    let content = parent.content().expect("parent should be an element node");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count());
    let mut offset: u32 = 0;

    for child in content.iter() {
        let child_size = child.node_size();
        let child_start = offset;
        let child_end = offset + child_size;

        if child_end <= from_offset || child_start >= to_offset {
            // Child is entirely outside the delete range — keep it.
            new_children.push(child.clone());
        } else if child.is_text() {
            // Child overlaps with the delete range. Keep the parts outside.
            let chars: Vec<char> = child.text_str().unwrap().chars().collect();

            let keep_left_end = if from_offset > child_start {
                (from_offset - child_start) as usize
            } else {
                0
            };
            let keep_right_start = if to_offset < child_end {
                (to_offset - child_start) as usize
            } else {
                chars.len()
            };

            let mut kept = String::new();
            if keep_left_end > 0 {
                kept.extend(&chars[..keep_left_end]);
            }
            if keep_right_start < chars.len() {
                kept.extend(&chars[keep_right_start..]);
            }

            if !kept.is_empty() {
                new_children.push(Node::text(kept, child.marks().to_vec()));
            }
        } else if child.is_void() {
            // Void node is inside the delete range — remove it.
            // (It's only 1 token, and it overlaps with the range.)
        } else {
            // Element node overlapping with delete range — for now, if it's
            // fully contained, remove it. If partially, this is an error we
            // don't handle yet (cross-node deletion).
            if child_start >= from_offset && child_end <= to_offset {
                // Fully inside — remove.
            } else {
                // Partially overlapping element — keep it as-is for now.
                // A more sophisticated implementation would handle this.
                new_children.push(child.clone());
            }
        }

        offset = child_end;
    }

    merge_adjacent_text_nodes(new_children)
}

// ---------------------------------------------------------------------------
// AddMark
// ---------------------------------------------------------------------------

fn apply_add_mark(
    doc: &Document,
    from: u32,
    to: u32,
    mark: &Mark,
) -> Result<(Document, StepMap), TransformError> {
    if from >= to {
        // No-op: empty range.
        return Ok((doc.clone(), StepMap::empty()));
    }

    let resolved_from = doc
        .resolve(from)
        .map_err(|e| TransformError::OutOfBounds(e))?;
    let resolved_to = doc
        .resolve(to)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    if resolved_from.node_path != resolved_to.node_path {
        return Err(TransformError::InvalidRange(
            "mark range spans different parent nodes".to_string(),
        ));
    }

    let parent = resolved_from.parent(doc);
    let from_offset = resolved_from.parent_offset;
    let to_offset = resolved_to.parent_offset;

    let new_children = add_mark_in_children(parent, from_offset, to_offset, mark);
    let new_parent = rebuild_element(parent, new_children);

    let new_root = replace_node_at_path(doc.root(), &resolved_from.node_path, &new_parent);
    let new_doc = Document::new(new_root);

    // Mark operations don't change positions.
    Ok((new_doc, StepMap::empty()))
}

/// Add a mark to all text within `[from_offset, to_offset)` in a parent's children.
fn add_mark_in_children(parent: &Node, from_offset: u32, to_offset: u32, mark: &Mark) -> Vec<Node> {
    let content = parent.content().expect("parent should be an element node");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count() + 2);
    let mut offset: u32 = 0;

    for child in content.iter() {
        let child_size = child.node_size();
        let child_start = offset;
        let child_end = offset + child_size;

        if !child.is_text() || child_end <= from_offset || child_start >= to_offset {
            // Non-text or entirely outside the mark range — keep as-is.
            new_children.push(child.clone());
        } else {
            // Text node overlaps with the mark range.
            let text_str = child.text_str().unwrap();
            let chars: Vec<char> = text_str.chars().collect();

            // How much of this text node is before, inside, and after the range.
            let mark_start_in_child = if from_offset > child_start {
                (from_offset - child_start) as usize
            } else {
                0
            };
            let mark_end_in_child = if to_offset < child_end {
                (to_offset - child_start) as usize
            } else {
                chars.len()
            };

            // Part before the mark range.
            if mark_start_in_child > 0 {
                let before_str: String = chars[..mark_start_in_child].iter().collect();
                new_children.push(Node::text(before_str, child.marks().to_vec()));
            }

            // Part inside the mark range — add the mark.
            if mark_start_in_child < mark_end_in_child {
                let inside_str: String = chars[mark_start_in_child..mark_end_in_child]
                    .iter()
                    .collect();
                let new_marks = add_mark_to_set(child.marks(), mark);
                new_children.push(Node::text(inside_str, new_marks));
            }

            // Part after the mark range.
            if mark_end_in_child < chars.len() {
                let after_str: String = chars[mark_end_in_child..].iter().collect();
                new_children.push(Node::text(after_str, child.marks().to_vec()));
            }
        }

        offset = child_end;
    }

    merge_adjacent_text_nodes(new_children)
}

// ---------------------------------------------------------------------------
// RemoveMark
// ---------------------------------------------------------------------------

fn apply_remove_mark(
    doc: &Document,
    from: u32,
    to: u32,
    mark_type: &str,
) -> Result<(Document, StepMap), TransformError> {
    if from >= to {
        return Ok((doc.clone(), StepMap::empty()));
    }

    let resolved_from = doc
        .resolve(from)
        .map_err(|e| TransformError::OutOfBounds(e))?;
    let resolved_to = doc
        .resolve(to)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    if resolved_from.node_path != resolved_to.node_path {
        return Err(TransformError::InvalidRange(
            "mark range spans different parent nodes".to_string(),
        ));
    }

    let parent = resolved_from.parent(doc);
    let from_offset = resolved_from.parent_offset;
    let to_offset = resolved_to.parent_offset;

    let new_children = remove_mark_in_children(parent, from_offset, to_offset, mark_type);
    let new_parent = rebuild_element(parent, new_children);

    let new_root = replace_node_at_path(doc.root(), &resolved_from.node_path, &new_parent);
    let new_doc = Document::new(new_root);

    Ok((new_doc, StepMap::empty()))
}

/// Remove a mark type from all text within `[from_offset, to_offset)`.
fn remove_mark_in_children(
    parent: &Node,
    from_offset: u32,
    to_offset: u32,
    mark_type: &str,
) -> Vec<Node> {
    let content = parent.content().expect("parent should be an element node");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count() + 2);
    let mut offset: u32 = 0;

    for child in content.iter() {
        let child_size = child.node_size();
        let child_start = offset;
        let child_end = offset + child_size;

        if !child.is_text() || child_end <= from_offset || child_start >= to_offset {
            new_children.push(child.clone());
        } else {
            let text_str = child.text_str().unwrap();
            let chars: Vec<char> = text_str.chars().collect();

            let range_start = if from_offset > child_start {
                (from_offset - child_start) as usize
            } else {
                0
            };
            let range_end = if to_offset < child_end {
                (to_offset - child_start) as usize
            } else {
                chars.len()
            };

            // Part before the removal range — keep original marks.
            if range_start > 0 {
                let before_str: String = chars[..range_start].iter().collect();
                new_children.push(Node::text(before_str, child.marks().to_vec()));
            }

            // Part inside the removal range — remove the mark type.
            if range_start < range_end {
                let inside_str: String = chars[range_start..range_end].iter().collect();
                let new_marks = remove_mark_from_set(child.marks(), mark_type);
                new_children.push(Node::text(inside_str, new_marks));
            }

            // Part after the removal range — keep original marks.
            if range_end < chars.len() {
                let after_str: String = chars[range_end..].iter().collect();
                new_children.push(Node::text(after_str, child.marks().to_vec()));
            }
        }

        offset = child_end;
    }

    merge_adjacent_text_nodes(new_children)
}

// ---------------------------------------------------------------------------
// SplitBlock
// ---------------------------------------------------------------------------

fn apply_split_block(
    doc: &Document,
    pos: u32,
    new_node_type: &str,
    new_attrs: &HashMap<String, serde_json::Value>,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc
        .resolve(pos)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    // The resolved position should be inside a text block (e.g. paragraph).
    // We need to find the text block in the path and determine what to split.
    let text_block = resolved.parent(doc);
    let text_block_spec = schema.node(text_block.node_type());

    match text_block_spec {
        Some(spec) => match spec.role {
            NodeRole::TextBlock => {}
            _ => {
                return Err(TransformError::InvalidTarget(format!(
                    "cannot split non-text-block '{}' (role {:?})",
                    text_block.node_type(),
                    spec.role
                )));
            }
        },
        None => {
            return Err(TransformError::InvalidTarget(format!(
                "node type '{}' not found in schema",
                text_block.node_type()
            )));
        }
    }

    let parent_offset = resolved.parent_offset;

    // Split the text block's children at parent_offset into left and right.
    let (left_children, right_children) = split_children_at(text_block, parent_offset);

    // Build the two new blocks.
    // First block: same type as the original text block.
    let left_block = rebuild_element(text_block, left_children);
    // Second block: uses the specified node_type and attrs.
    let right_block = Node::element(
        new_node_type.to_string(),
        new_attrs.clone(),
        Fragment::from(right_children),
    );

    // Now we need to determine what the grandparent is and how to splice.
    // The text block's path in the tree is `resolved.node_path`.
    // If the text block is directly inside doc (path len == 1), we replace
    // the text block with the two new blocks in doc's children.
    // If the text block is inside a list item (path len > 1), we may need
    // to split the list item as well.
    let text_block_path = &resolved.node_path;

    if text_block_path.is_empty() {
        // Position resolved to the doc level itself — shouldn't happen for text blocks.
        return Err(TransformError::InvalidTarget(
            "cannot split at document level".to_string(),
        ));
    }

    // Check if the grandparent is a list item. If so, we split the list item too.
    if text_block_path.len() >= 2 {
        let grandparent_path = &text_block_path[..text_block_path.len() - 1];
        let grandparent = doc
            .node_at(grandparent_path)
            .ok_or_else(|| TransformError::OutOfBounds("grandparent path invalid".to_string()))?;

        if let Some(gp_spec) = schema.node(grandparent.node_type()) {
            if matches!(gp_spec.role, NodeRole::ListItem) {
                // We're inside a list item. Split the list item into two.
                let text_block_idx = *text_block_path.last().unwrap() as usize;
                let gp_content = grandparent
                    .content()
                    .expect("list item should have content");

                // Distribute the list item's children between the two new list items.
                // Children before the split text block go into the first list item
                // (along with left_block). Children after go into the second
                // (along with right_block).
                let mut li1_children: Vec<Node> = Vec::new();
                let mut li2_children: Vec<Node> = Vec::new();

                for (i, child) in gp_content.iter().enumerate() {
                    if i < text_block_idx {
                        li1_children.push(child.clone());
                    } else if i == text_block_idx {
                        li1_children.push(left_block.clone());
                        li2_children.push(right_block.clone());
                    } else {
                        li2_children.push(child.clone());
                    }
                }

                let li1 = rebuild_element(grandparent, li1_children);
                let li2 = Node::element(
                    grandparent.node_type().to_string(),
                    grandparent.attrs().clone(),
                    Fragment::from(li2_children),
                );

                // Replace the grandparent (list item) with the two new list items
                // in the great-grandparent.
                let new_root = replace_node_with_two(doc.root(), grandparent_path, &li1, &li2);
                let new_doc = Document::new(new_root);
                // Splitting inside a list item adds both the standard block split
                // tokens (+2 for the new right block) and a second list-item
                // wrapper (+2 for the new listItem open/close), so the cursor at
                // the split point must advance by 4 into the new item.
                let map = StepMap::from_insert(pos, 4);

                return Ok((new_doc, map));
            }
        }
    }

    // Standard case: replace the text block with two blocks in the parent.
    let new_root = replace_node_with_two(doc.root(), text_block_path, &left_block, &right_block);
    let new_doc = Document::new(new_root);
    let map = StepMap::from_insert(pos, 2);

    Ok((new_doc, map))
}

/// Split a parent node's children at the given content offset into two vecs.
/// Text nodes straddling the split point are themselves split.
fn split_children_at(parent: &Node, offset: u32) -> (Vec<Node>, Vec<Node>) {
    let content = match parent.content() {
        Some(c) => c,
        None => return (vec![], vec![]),
    };

    let mut left: Vec<Node> = Vec::new();
    let mut right: Vec<Node> = Vec::new();
    let mut current_offset: u32 = 0;
    let mut split_done = false;

    for child in content.iter() {
        if split_done {
            right.push(child.clone());
            continue;
        }

        let child_size = child.node_size();

        if current_offset + child_size <= offset {
            // Entire child is on the left side.
            left.push(child.clone());
            current_offset += child_size;
        } else if current_offset >= offset {
            // Entire child is on the right side.
            right.push(child.clone());
            split_done = true;
        } else {
            // The split point is inside this child.
            let inner_offset = offset - current_offset;

            if child.is_text() {
                let (left_part, right_part) = split_text_node(child, inner_offset);
                if let Some(l) = left_part {
                    left.push(l);
                }
                if let Some(r) = right_part {
                    right.push(r);
                }
            } else {
                // Non-text child straddling the split — shouldn't happen for
                // inline content, but keep the child on the left side.
                left.push(child.clone());
            }
            split_done = true;
            current_offset += child_size;
        }
    }

    // Merge adjacent text nodes within each side.
    (
        merge_adjacent_text_nodes(left),
        merge_adjacent_text_nodes(right),
    )
}

// ---------------------------------------------------------------------------
// JoinBlocks
// ---------------------------------------------------------------------------

fn apply_join_blocks(doc: &Document, pos: u32) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc
        .resolve(pos)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    // The position should be at a block boundary in the parent.
    // That means it should resolve to a parent (e.g. doc or list) and the
    // parent_offset should sit exactly between two element children.
    let parent = resolved.parent(doc);
    let parent_offset = resolved.parent_offset;

    // Walk the parent's children to find which boundary we're at.
    let content = parent.content().ok_or_else(|| {
        TransformError::InvalidTarget("join position parent has no content".to_string())
    })?;

    let mut offset: u32 = 0;
    let mut boundary_idx: Option<usize> = None;

    for (i, child) in content.iter().enumerate() {
        let child_size = child.node_size();

        if offset == parent_offset && i > 0 {
            // We're at the start of child `i`, meaning between child `i-1` and child `i`.
            boundary_idx = Some(i);
            break;
        }

        offset += child_size;
    }

    let idx = boundary_idx.ok_or_else(|| {
        TransformError::InvalidTarget(format!(
            "position {} (parent_offset {}) is not at a block boundary in '{}'",
            pos,
            parent_offset,
            parent.node_type()
        ))
    })?;

    // Get the two adjacent blocks.
    let first = content.child(idx - 1).unwrap();
    let second = content.child(idx).unwrap();

    if !first.is_element() || !second.is_element() {
        return Err(TransformError::InvalidTarget(
            "JoinBlocks requires two adjacent element nodes at the boundary".to_string(),
        ));
    }

    // Merge the children of both blocks.
    let first_content = first.content().unwrap();
    let second_content = second.content().unwrap();

    let mut merged_children: Vec<Node> =
        Vec::with_capacity(first_content.child_count() + second_content.child_count());

    for child in first_content.iter() {
        merged_children.push(child.clone());
    }
    for child in second_content.iter() {
        merged_children.push(child.clone());
    }

    let merged_children = merge_adjacent_text_nodes(merged_children);

    // Build the merged block using the first block's type and attrs.
    let merged_block = Node::element(
        first.node_type().to_string(),
        first.attrs().clone(),
        Fragment::from(merged_children),
    );

    // Rebuild the parent with the merged block replacing the two.
    let mut new_parent_children: Vec<Node> = Vec::with_capacity(content.child_count() - 1);
    for (i, child) in content.iter().enumerate() {
        if i == idx - 1 {
            new_parent_children.push(merged_block.clone());
        } else if i == idx {
            // Skip the second block — it's been merged into the first.
        } else {
            new_parent_children.push(child.clone());
        }
    }

    let new_parent = rebuild_element(parent, new_parent_children);

    // Replace the parent in the tree.
    let new_root = replace_node_at_path(doc.root(), &resolved.node_path, &new_parent);
    let new_doc = Document::new(new_root);

    // The join removes 2 tokens: one close tag + one open tag.
    let map = StepMap::from_delete(pos, 2);

    Ok((new_doc, map))
}

// ---------------------------------------------------------------------------
// WrapInList
// ---------------------------------------------------------------------------

fn apply_wrap_in_list(
    doc: &Document,
    from: u32,
    to: u32,
    list_type: &str,
    item_type: &str,
    list_attrs: &HashMap<String, serde_json::Value>,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    if from > to {
        return Err(TransformError::InvalidRange(format!(
            "wrap range from ({from}) is greater than to ({to})"
        )));
    }

    // Validate the list_type is actually a list node in the schema.
    let list_spec = schema.node(list_type).ok_or_else(|| {
        TransformError::InvalidTarget(format!("list_type '{}' not found in schema", list_type))
    })?;
    if !matches!(list_spec.role, NodeRole::List { .. }) {
        return Err(TransformError::InvalidTarget(format!(
            "'{}' is not a list node (role {:?}); expected a node with NodeRole::List",
            list_type, list_spec.role
        )));
    }

    // Validate the item_type is a list item.
    let item_spec = schema.node(item_type).ok_or_else(|| {
        TransformError::InvalidTarget(format!("item_type '{}' not found in schema", item_type))
    })?;
    if !matches!(item_spec.role, NodeRole::ListItem) {
        return Err(TransformError::InvalidTarget(format!(
            "'{}' is not a list item node (role {:?})",
            item_type, item_spec.role
        )));
    }

    // The from/to range must select complete block nodes at the doc level.
    // Walk the doc's children to find which blocks are covered.
    let doc_content = doc
        .root()
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("document root has no content".to_string()))?;

    let mut offset: u32 = 0;
    let mut first_block_idx: Option<usize> = None;
    let mut last_block_idx: Option<usize> = None;

    for (i, child) in doc_content.iter().enumerate() {
        let child_size = child.node_size();
        let child_start = offset;
        let child_end = offset + child_size;

        // A block is "in range" if its span overlaps with [from, to).
        if child_end > from && child_start < to {
            // Validate that we're not trying to wrap something that is already
            // a list node (wrapping a list in a list is not supported).
            if let Some(spec) = schema.node(child.node_type()) {
                if matches!(spec.role, NodeRole::List { .. }) {
                    return Err(TransformError::InvalidTarget(format!(
                        "cannot wrap '{}' (already a list) in another list",
                        child.node_type()
                    )));
                }
            }

            if first_block_idx.is_none() {
                first_block_idx = Some(i);
            }
            last_block_idx = Some(i);
        }

        offset += child_size;
    }

    let first_idx = first_block_idx.ok_or_else(|| {
        TransformError::InvalidRange(format!("no block nodes found in range [{}..{}]", from, to))
    })?;
    let last_idx = last_block_idx.unwrap(); // safe: set whenever first_idx is set

    // Build the list items: one per block in the range.
    let mut list_items: Vec<Node> = Vec::with_capacity(last_idx - first_idx + 1);
    for i in first_idx..=last_idx {
        let block = doc_content.child(i).unwrap();
        let li = Node::element(
            item_type.to_string(),
            HashMap::new(),
            Fragment::from(vec![block.clone()]),
        );
        list_items.push(li);
    }

    // Build the list node.
    let list_node = Node::element(
        list_type.to_string(),
        list_attrs.clone(),
        Fragment::from(list_items),
    );

    // Rebuild the doc's children: children before the range, the list, children after.
    let mut new_children: Vec<Node> =
        Vec::with_capacity(doc_content.child_count() - (last_idx - first_idx));
    for (i, child) in doc_content.iter().enumerate() {
        if i == first_idx {
            new_children.push(list_node.clone());
        } else if i > first_idx && i <= last_idx {
            // Skip — these are now inside the list.
        } else {
            new_children.push(child.clone());
        }
    }

    let new_root = rebuild_element(doc.root(), new_children);
    let new_doc = Document::new(new_root);

    // Wrapping inserts the list open tag plus the first list-item open tag
    // before the wrapped content, then the remaining close/open boundaries at
    // the end of the wrapped range.
    let num_blocks = (last_idx - first_idx + 1) as u32;
    let total_added = 2 + 2 * num_blocks; // list open/close + li open/close per block
    let map_start = StepMap::from_insert(from, 2);
    let map_end = StepMap::from_insert(to + 2, total_added - 2);
    let map = map_start.compose(&map_end);

    Ok((new_doc, map))
}

// ---------------------------------------------------------------------------
// UnwrapFromList
// ---------------------------------------------------------------------------

fn apply_unwrap_from_list(
    doc: &Document,
    pos: u32,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    // Resolve the position to find which list item we're in.
    let resolved = doc
        .resolve(pos)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    // Walk up the path to find the list item and the list.
    // The node_path gives us indices from root. We need to find a node in the
    // path that is a ListItem, and its parent should be a List.
    //
    // For a typical structure: doc > bulletList > listItem > paragraph
    // node_path would be [0, 0, 0] (indices at each level).
    // - path[0] = bulletList index in doc
    // - path[1] = listItem index in bulletList
    // - path[2] = paragraph index in listItem
    //
    // We need to find the ListItem level.
    let path = &resolved.node_path;

    let mut list_item_depth: Option<usize> = None;

    // Check each node in the path (from root down) to find the list item.
    // path[i] is the child index at depth i+1. The node at depth i+1 is
    // doc.root().child(path[0]).child(path[1])...child(path[i]).
    let mut current_node = doc.root();
    for (depth_idx, &child_idx) in path.iter().enumerate() {
        let child = current_node.child(child_idx as usize).ok_or_else(|| {
            TransformError::OutOfBounds(format!(
                "invalid path index {} at depth {}",
                child_idx, depth_idx
            ))
        })?;

        if let Some(spec) = schema.node(child.node_type()) {
            if matches!(spec.role, NodeRole::ListItem) {
                list_item_depth = Some(depth_idx);
            }
        }

        current_node = child;
    }

    let li_depth = list_item_depth.ok_or_else(|| {
        TransformError::InvalidTarget("position is not inside a list item".to_string())
    })?;

    // li_depth is the index in `path` where the list item is.
    // The list is the parent at li_depth - 0 in the tree perspective.
    // Actually, path[li_depth] is the index of the list item in the list.
    // The list itself is found by following path[0..li_depth].
    let list_item_idx = path[li_depth] as usize;

    // Get the list node.
    let list_path = &path[..li_depth];
    let list_node = doc
        .node_at(list_path)
        .ok_or_else(|| TransformError::OutOfBounds("list node path invalid".to_string()))?;

    // Verify the list node is actually a list.
    let list_spec = schema.node(list_node.node_type()).ok_or_else(|| {
        TransformError::InvalidTarget(format!(
            "parent of list item ('{}') not found in schema",
            list_node.node_type()
        ))
    })?;
    if !matches!(list_spec.role, NodeRole::List { .. }) {
        return Err(TransformError::InvalidTarget(format!(
            "parent of list item is '{}' (role {:?}), expected a list",
            list_node.node_type(),
            list_spec.role
        )));
    }

    let list_content = list_node
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("list node has no content".to_string()))?;

    let list_item_node = list_content.child(list_item_idx).ok_or_else(|| {
        TransformError::OutOfBounds(format!(
            "list item index {} out of bounds in list with {} items",
            list_item_idx,
            list_content.child_count()
        ))
    })?;

    // Extract the content of the list item (the paragraph(s) inside it).
    let li_content = list_item_node
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("list item has no content".to_string()))?;
    let extracted_blocks: Vec<Node> = li_content.iter().cloned().collect();

    let total_list_items = list_content.child_count();

    // Build replacement nodes for where the list currently sits.
    // Three cases:
    //   1. Only child → remove the entire list, replace with extracted blocks
    //   2. First or last child → keep remaining items in a shortened list
    //   3. Middle child → split the list into two with extracted blocks between

    let mut replacement_nodes: Vec<Node> = Vec::new();

    if total_list_items == 1 {
        // Case 1: Only list item — replace entire list with extracted blocks.
        replacement_nodes.extend(extracted_blocks);
    } else if list_item_idx == 0 {
        // Case 2a: First item — extracted blocks come first, then remaining list.
        replacement_nodes.extend(extracted_blocks);

        let remaining_items: Vec<Node> = (1..total_list_items)
            .map(|i| list_content.child(i).unwrap().clone())
            .collect();
        let remaining_list = Node::element(
            list_node.node_type().to_string(),
            list_node.attrs().clone(),
            Fragment::from(remaining_items),
        );
        replacement_nodes.push(remaining_list);
    } else if list_item_idx == total_list_items - 1 {
        // Case 2b: Last item — remaining list comes first, then extracted blocks.
        let remaining_items: Vec<Node> = (0..list_item_idx)
            .map(|i| list_content.child(i).unwrap().clone())
            .collect();
        let remaining_list = Node::element(
            list_node.node_type().to_string(),
            list_node.attrs().clone(),
            Fragment::from(remaining_items),
        );
        replacement_nodes.push(remaining_list);
        replacement_nodes.extend(extracted_blocks);
    } else {
        // Case 3: Middle item — split into two lists with extracted blocks between.
        let before_items: Vec<Node> = (0..list_item_idx)
            .map(|i| list_content.child(i).unwrap().clone())
            .collect();
        let after_items: Vec<Node> = ((list_item_idx + 1)..total_list_items)
            .map(|i| list_content.child(i).unwrap().clone())
            .collect();

        let list_before = Node::element(
            list_node.node_type().to_string(),
            list_node.attrs().clone(),
            Fragment::from(before_items),
        );
        let list_after = Node::element(
            list_node.node_type().to_string(),
            list_node.attrs().clone(),
            Fragment::from(after_items),
        );

        replacement_nodes.push(list_before);
        replacement_nodes.extend(extracted_blocks);
        replacement_nodes.push(list_after);
    }

    // Now replace the list node in its parent with the replacement nodes.
    // The list's parent is found by following list_path[..last].
    // If list_path is empty, the list is a direct child of doc root.
    let new_root = replace_node_with_many(doc.root(), list_path, &replacement_nodes);
    let new_doc = Document::new(new_root);

    // StepMap: We removed the list open/close (2 tokens) and the list item
    // open/close (2 tokens) = 4 tokens removed for the simple case (only item).
    // For first/last item: remove li open/close (2 tokens), list stays.
    // For middle item: remove li open/close (2) but add list close/open (2) for the split = net 0?
    // Actually let's think about this more carefully:
    //
    // Only item: removed list_open + li_open + li_close + list_close = 4 tokens
    // First/last item: removed li_open + li_close = 2 tokens
    // Middle item: removed li_open + li_close = 2 tokens, but added list_close + list_open = 2
    //   net = 0 tokens change for middle case
    //
    // For position mapping, positions before the list are unchanged.
    // Positions inside the unwrapped content shift by the number of wrapper tokens removed.

    // Calculate the absolute position of the list start in the document.
    let mut list_abs_pos: u32 = 0;
    {
        let mut node = doc.root();
        for &idx in list_path.iter() {
            let content = node.content().unwrap();
            for i in 0..(idx as usize) {
                list_abs_pos += content.child(i).unwrap().node_size();
            }
            list_abs_pos += 1; // open tag of this node
            node = content.child(idx as usize).unwrap();
        }
    }

    if total_list_items == 1 {
        // Removed 4 tokens: list_open at list_abs_pos, li_open at list_abs_pos+1,
        // li_close before list_close. Model as delete of 2 at start + 2 at end.
        let map_start = StepMap::from_delete(list_abs_pos, 2);
        // After removing 2 at start, the li_close + list_close are at the end.
        // Original end position of the content = list_abs_pos + 2 + li_content_size
        let li_content_size = li_content.size();
        let close_pos = list_abs_pos + li_content_size; // after removing the 2 opens
        let map_end = StepMap::from_delete(close_pos, 2);
        let map = map_start.compose(&map_end);
        Ok((new_doc, map))
    } else if list_item_idx == 0 {
        // First item: remove li_open and li_close (2 tokens around the content).
        // The list_open stays. The li_open was at list_abs_pos + 1 (after list open).
        // Actually we need to remove the li_open (1 token) at list_abs_pos+1
        // and the li_close (1 token) after the li content.
        // But we also removed the list structure around the extracted content...
        // Wait, for first item unwrap:
        //   Before: <list_open> <li_open> [content] <li_close> [remaining items] <list_close>
        //   After:  [content] <list_open> [remaining items] <list_close>
        // So we removed list_open + li_open before content (2 tokens), and
        // removed li_close after content (1 token), and the list_open that was
        // at the start now appears after the content.
        // Net tokens removed = li_open + li_close + list_open_moved_after = hmm...
        //
        // Actually: the list_open moved. Let me think in terms of total size.
        // Old size of list node = 1(list_open) + sum(li_sizes) + 1(list_close)
        // New: extracted_content_size + (1 + remaining_items_size + 1) = extracted + remaining_list_size
        // Diff = (extracted + remaining_list) - list_node_size
        //      = extracted + (1 + (sum - li_size) + 1) - (1 + sum + 1)
        //      = extracted + 2 + sum - li_size - 2 - sum
        //      = extracted - li_size
        //      = li_content_size - (1 + li_content_size + 1)
        //      = -2
        // So 2 tokens removed total. They are the li_open and li_close.
        //
        // For position mapping: positions before list_abs_pos unchanged.
        // Content inside the extracted li shifts by -2 (lost list_open and li_open before it).
        let map = StepMap::from_delete(list_abs_pos, 2);
        Ok((new_doc, map))
    } else if list_item_idx == total_list_items - 1 {
        // Last item: similar to first but the extracted content comes after the list.
        // Before: <list_open> [preceding items] <li_open> [content] <li_close> <list_close>
        // After:  <list_open> [preceding items] <list_close> [content]
        // The list close token moves into the old li_open slot, so positions
        // inside the extracted content should stay stable. Only the old
        // li_close + list_close pair after the content disappears.
        let mut preceding_size: u32 = 0;
        for i in 0..list_item_idx {
            preceding_size += list_content.child(i).unwrap().node_size();
        }
        let li_open_pos = list_abs_pos + 1 + preceding_size;
        let map_start = StepMap::from_replace(li_open_pos, 1, 1);
        let close_pos = li_open_pos + 1 + li_content.size();
        let map_end = StepMap::from_delete(close_pos, 2);
        let map = map_start.compose(&map_end);
        Ok((new_doc, map))
    } else {
        // Middle item: split the list. Net change = 0 tokens (remove 2, add 2 for new list boundary).
        // But positions shift locally. Use empty map as approximation.
        // Actually more precisely:
        //   Before: ... <li_open> [content] <li_close> ...
        //   After:  ... <list_close> [content] <list_open> ...
        // The li_open/li_close are replaced by list_close/list_open — same token count.
        // Positions are unchanged.
        Ok((new_doc, StepMap::empty()))
    }
}

// ---------------------------------------------------------------------------
// InsertNode
// ---------------------------------------------------------------------------

fn apply_insert_node(
    doc: &Document,
    pos: u32,
    node: &Node,
    _schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc
        .resolve(pos)
        .map_err(|e| TransformError::OutOfBounds(e))?;
    let parent = resolved.parent(doc);

    // insert_node_in_children handles both block-level (between element
    // children) and inline-level (splitting text nodes) insertion uniformly.
    let parent_offset = resolved.parent_offset;
    let new_children = insert_node_in_children(parent, parent_offset, node);
    let new_parent = rebuild_element(parent, new_children);

    let new_root = replace_node_at_path(doc.root(), &resolved.node_path, &new_parent);
    let new_doc = Document::new(new_root);
    let map = StepMap::from_insert(pos, node.node_size());

    Ok((new_doc, map))
}

fn apply_indent_list_item(
    doc: &Document,
    pos: u32,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let context = resolve_list_item_context(doc, pos, schema)?;

    if context.list_item_idx == 0 {
        return Ok((doc.clone(), StepMap::empty()));
    }

    let list_content = context
        .list_node
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("list node has no content".to_string()))?;
    let previous_item = list_content
        .child(context.list_item_idx - 1)
        .ok_or_else(|| TransformError::OutOfBounds("previous list item not found".to_string()))?;
    let current_item = list_content
        .child(context.list_item_idx)
        .ok_or_else(|| TransformError::OutOfBounds("current list item not found".to_string()))?
        .clone();

    let previous_children = previous_item
        .content()
        .ok_or_else(|| {
            TransformError::InvalidTarget("previous list item has no content".to_string())
        })?
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let new_previous_children = append_list_item_to_nested_list(
        previous_children,
        context.list_node.node_type(),
        context.list_node.attrs(),
        current_item,
    );
    let new_previous_item = Node::element(
        previous_item.node_type().to_string(),
        previous_item.attrs().clone(),
        Fragment::from(new_previous_children),
    );

    let mut new_list_children = Vec::with_capacity(list_content.child_count() - 1);
    for i in 0..list_content.child_count() {
        if i == context.list_item_idx - 1 {
            new_list_children.push(new_previous_item.clone());
        } else if i == context.list_item_idx {
            continue;
        } else {
            new_list_children.push(list_content.child(i).unwrap().clone());
        }
    }

    let new_list = Node::element(
        context.list_node.node_type().to_string(),
        context.list_node.attrs().clone(),
        Fragment::from(new_list_children),
    );
    let new_root = replace_node_at_path(doc.root(), &context.list_path, &new_list);
    Ok((Document::new(new_root), StepMap::empty()))
}

fn apply_outdent_list_item(
    doc: &Document,
    pos: u32,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let context = resolve_list_item_context(doc, pos, schema)?;

    if context.list_path.is_empty() {
        return Ok((doc.clone(), StepMap::empty()));
    }

    let parent_list_item_path = &context.list_path[..context.list_path.len() - 1];
    let parent_list_item = match doc.node_at(parent_list_item_path) {
        Some(node) if node.node_type() == "listItem" => node,
        _ => return Ok((doc.clone(), StepMap::empty())),
    };

    if parent_list_item_path.is_empty() {
        return Ok((doc.clone(), StepMap::empty()));
    }

    let parent_list_path = &parent_list_item_path[..parent_list_item_path.len() - 1];
    let parent_list_node = doc
        .node_at(parent_list_path)
        .ok_or_else(|| TransformError::OutOfBounds("parent list path invalid".to_string()))?;

    let parent_list_content = parent_list_node
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("parent list has no content".to_string()))?;
    let parent_list_item_idx = *parent_list_item_path.last().ok_or_else(|| {
        TransformError::InvalidTarget("parent list item path missing index".to_string())
    })? as usize;

    let nested_list_content = context
        .list_node
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("nested list has no content".to_string()))?;
    let current_item = nested_list_content
        .child(context.list_item_idx)
        .ok_or_else(|| TransformError::OutOfBounds("nested list item not found".to_string()))?
        .clone();

    let before_nested_items = (0..context.list_item_idx)
        .map(|i| nested_list_content.child(i).unwrap().clone())
        .collect::<Vec<_>>();
    let after_nested_items = ((context.list_item_idx + 1)..nested_list_content.child_count())
        .map(|i| nested_list_content.child(i).unwrap().clone())
        .collect::<Vec<_>>();

    let mut moved_item_children = current_item
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("moved list item has no content".to_string()))?
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    if !after_nested_items.is_empty() {
        let trailing_nested_list = Node::element(
            context.list_node.node_type().to_string(),
            context.list_node.attrs().clone(),
            Fragment::from(after_nested_items),
        );
        moved_item_children =
            append_or_merge_nested_list_node(moved_item_children, trailing_nested_list);
    }
    let moved_item = Node::element(
        current_item.node_type().to_string(),
        current_item.attrs().clone(),
        Fragment::from(moved_item_children),
    );

    let parent_item_children = parent_list_item
        .content()
        .ok_or_else(|| {
            TransformError::InvalidTarget("parent list item has no content".to_string())
        })?
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let nested_list_child_idx = *context.list_path.last().ok_or_else(|| {
        TransformError::InvalidTarget("nested list path missing index".to_string())
    })? as usize;

    let mut new_parent_item_children = Vec::with_capacity(parent_item_children.len());
    for (idx, child) in parent_item_children.into_iter().enumerate() {
        if idx != nested_list_child_idx {
            new_parent_item_children.push(child);
            continue;
        }

        if !before_nested_items.is_empty() {
            new_parent_item_children.push(Node::element(
                context.list_node.node_type().to_string(),
                context.list_node.attrs().clone(),
                Fragment::from(before_nested_items.clone()),
            ));
        }
    }

    let new_parent_list_item = Node::element(
        parent_list_item.node_type().to_string(),
        parent_list_item.attrs().clone(),
        Fragment::from(new_parent_item_children),
    );

    let mut new_parent_list_children = Vec::with_capacity(parent_list_content.child_count() + 1);
    for i in 0..parent_list_content.child_count() {
        if i == parent_list_item_idx {
            new_parent_list_children.push(new_parent_list_item.clone());
            new_parent_list_children.push(moved_item.clone());
        } else {
            new_parent_list_children.push(parent_list_content.child(i).unwrap().clone());
        }
    }

    let new_parent_list = Node::element(
        parent_list_node.node_type().to_string(),
        parent_list_node.attrs().clone(),
        Fragment::from(new_parent_list_children),
    );
    let new_root = replace_node_at_path(doc.root(), parent_list_path, &new_parent_list);
    Ok((Document::new(new_root), StepMap::empty()))
}

struct ListItemContext<'a> {
    list_path: Vec<u16>,
    list_node: &'a Node,
    list_item_idx: usize,
}

fn resolve_list_item_context<'a>(
    doc: &'a Document,
    pos: u32,
    schema: &Schema,
) -> Result<ListItemContext<'a>, TransformError> {
    let resolved = doc
        .resolve(pos)
        .map_err(|e| TransformError::OutOfBounds(e))?;
    let path = &resolved.node_path;

    let mut current_node = doc.root();
    let mut list_item_depth = None;

    for (depth_idx, &child_idx) in path.iter().enumerate() {
        let content = current_node.content().ok_or_else(|| {
            TransformError::InvalidTarget(format!(
                "node '{}' has no content while resolving list item",
                current_node.node_type()
            ))
        })?;
        let child = content.child(child_idx as usize).ok_or_else(|| {
            TransformError::OutOfBounds(format!(
                "child {} missing while resolving list item",
                child_idx
            ))
        })?;

        if let Some(spec) = schema.node(child.node_type()) {
            if matches!(spec.role, NodeRole::ListItem) {
                list_item_depth = Some(depth_idx);
            }
        }

        current_node = child;
    }

    let li_depth = list_item_depth.ok_or_else(|| {
        TransformError::InvalidTarget("position is not inside a list item".to_string())
    })?;
    let list_path = path[..li_depth].to_vec();
    let list_node = doc
        .node_at(&list_path)
        .ok_or_else(|| TransformError::OutOfBounds("list node path invalid".to_string()))?;
    let list_spec = schema.node(list_node.node_type()).ok_or_else(|| {
        TransformError::InvalidTarget(format!(
            "list node '{}' not found in schema",
            list_node.node_type()
        ))
    })?;
    if !matches!(list_spec.role, NodeRole::List { .. }) {
        return Err(TransformError::InvalidTarget(format!(
            "parent of list item is '{}' (role {:?}), expected a list",
            list_node.node_type(),
            list_spec.role
        )));
    }

    Ok(ListItemContext {
        list_path,
        list_node,
        list_item_idx: path[li_depth] as usize,
    })
}

fn append_list_item_to_nested_list(
    children: Vec<Node>,
    list_type: &str,
    list_attrs: &HashMap<String, serde_json::Value>,
    item: Node,
) -> Vec<Node> {
    let nested_list = Node::element(
        list_type.to_string(),
        list_attrs.clone(),
        Fragment::from(vec![item]),
    );
    append_or_merge_nested_list_node(children, nested_list)
}

fn append_or_merge_nested_list_node(mut children: Vec<Node>, nested_list: Node) -> Vec<Node> {
    let nested_type = nested_list.node_type().to_string();
    if let Some(last_child) = children.last_mut() {
        if last_child.node_type() == nested_type {
            if let (Some(existing_content), Some(new_content)) =
                (last_child.content(), nested_list.content())
            {
                let mut merged_items = existing_content.iter().cloned().collect::<Vec<_>>();
                merged_items.extend(new_content.iter().cloned());
                *last_child = Node::element(
                    last_child.node_type().to_string(),
                    last_child.attrs().clone(),
                    Fragment::from(merged_items),
                );
                return children;
            }
        }
    }

    children.push(nested_list);
    children
}

/// Insert a node into a parent's children at the given parent-content offset.
///
/// Works for both block-level insertion (between element children) and
/// inline-level insertion (between text/void children within a text block).
fn insert_node_in_children(parent: &Node, offset: u32, insert_node: &Node) -> Vec<Node> {
    let content = parent.content().expect("parent should be an element node");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count() + 2);
    let mut remaining_offset = offset;

    // If the parent has no children, just insert the node.
    if content.child_count() == 0 {
        new_children.push(insert_node.clone());
        return new_children;
    }

    let mut inserted = false;

    for child in content.iter() {
        if inserted {
            new_children.push(child.clone());
            continue;
        }

        let child_size = child.node_size();

        if child.is_text() {
            if remaining_offset <= child_size {
                if remaining_offset == 0 {
                    // Insert before this text node.
                    new_children.push(insert_node.clone());
                    new_children.push(child.clone());
                    inserted = true;
                    continue;
                } else if remaining_offset == child_size {
                    // Insert after this text node — continue to next child or
                    // insert after all children.
                    new_children.push(child.clone());
                    remaining_offset -= child_size;
                    continue;
                } else {
                    // Split the text node at the offset, insert node between halves.
                    let (left, right) = split_text_node(child, remaining_offset);
                    if let Some(l) = left {
                        new_children.push(l);
                    }
                    new_children.push(insert_node.clone());
                    if let Some(r) = right {
                        new_children.push(r);
                    }
                    inserted = true;
                    continue;
                }
            }
            new_children.push(child.clone());
            remaining_offset -= child_size;
        } else if child.is_void() {
            if remaining_offset == 0 {
                new_children.push(insert_node.clone());
                new_children.push(child.clone());
                inserted = true;
                continue;
            }
            remaining_offset -= 1;
            new_children.push(child.clone());
        } else {
            // Element child — offset must be at a boundary (before or after this child).
            if remaining_offset == 0 {
                new_children.push(insert_node.clone());
                new_children.push(child.clone());
                inserted = true;
                continue;
            }
            remaining_offset -= child_size;
            new_children.push(child.clone());
        }
    }

    // If we haven't inserted yet, the offset is at the end.
    if !inserted {
        new_children.push(insert_node.clone());
    }

    merge_adjacent_text_nodes(new_children)
}

// ---------------------------------------------------------------------------
// ReplaceRange
// ---------------------------------------------------------------------------

fn apply_replace_range(
    doc: &Document,
    from: u32,
    to: u32,
    content: &Fragment,
    _schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    if from > to {
        return Err(TransformError::InvalidRange(format!(
            "replace range from ({from}) is greater than to ({to})"
        )));
    }

    let resolved_from = doc
        .resolve(from)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    if from == to && content.size() == 0 {
        // No-op: empty range and empty content.
        return Ok((doc.clone(), StepMap::empty()));
    }

    // If from != to, resolve `to` and check if same parent.
    if from != to {
        let resolved_to = doc
            .resolve(to)
            .map_err(|e| TransformError::OutOfBounds(e))?;

        if resolved_from.node_path != resolved_to.node_path {
            // Cross-parent replace: delete across parents first, then insert.
            return apply_cross_parent_replace(
                doc,
                from,
                to,
                content,
                &resolved_from,
                &resolved_to,
            );
        }
    }

    let parent = resolved_from.parent(doc);
    let from_offset = resolved_from.parent_offset;
    let deleted_len = to - from;
    let to_offset = from_offset + deleted_len;

    // Step 1: Delete the range [from_offset, to_offset) in the parent's children.
    let after_delete = if deleted_len > 0 {
        delete_in_children(parent, from_offset, to_offset)
    } else {
        parent
            .content()
            .expect("parent should be an element node")
            .iter()
            .cloned()
            .collect()
    };

    // Step 2: Insert the content nodes at from_offset in the resulting children.
    let after_insert = if content.size() > 0 {
        // Build a temporary parent with the after-delete children so we can
        // use insert_nodes_in_children to splice in the content.
        let temp_parent = rebuild_element(parent, after_delete);
        insert_nodes_in_children(&temp_parent, from_offset, content)
    } else {
        after_delete
    };

    let new_parent = rebuild_element(parent, after_insert);
    let new_root = replace_node_at_path(doc.root(), &resolved_from.node_path, &new_parent);
    let new_doc = Document::new(new_root);

    let inserted_size = content.size();
    let map = StepMap::from_replace(from, deleted_len, inserted_size);

    Ok((new_doc, map))
}

/// Insert multiple nodes (from a Fragment) into a parent's children at the
/// given parent-content offset.
fn insert_nodes_in_children(parent: &Node, offset: u32, fragment: &Fragment) -> Vec<Node> {
    let content = parent.content().expect("parent should be an element node");
    let insert_nodes: Vec<&Node> = fragment.iter().collect();
    let mut new_children: Vec<Node> =
        Vec::with_capacity(content.child_count() + insert_nodes.len() + 2);
    let mut remaining_offset = offset;

    // If the parent has no children, just insert all fragment nodes.
    if content.child_count() == 0 {
        for node in &insert_nodes {
            new_children.push((*node).clone());
        }
        return merge_adjacent_text_nodes(new_children);
    }

    let mut inserted = false;

    for child in content.iter() {
        if inserted {
            new_children.push(child.clone());
            continue;
        }

        let child_size = child.node_size();

        if child.is_text() {
            if remaining_offset <= child_size {
                if remaining_offset == 0 {
                    // Insert before this text node.
                    for node in &insert_nodes {
                        new_children.push((*node).clone());
                    }
                    new_children.push(child.clone());
                    inserted = true;
                    continue;
                } else if remaining_offset == child_size {
                    // At the end of this text node — continue.
                    new_children.push(child.clone());
                    remaining_offset -= child_size;
                    continue;
                } else {
                    // Split the text node and insert between halves.
                    let (left, right) = split_text_node(child, remaining_offset);
                    if let Some(l) = left {
                        new_children.push(l);
                    }
                    for node in &insert_nodes {
                        new_children.push((*node).clone());
                    }
                    if let Some(r) = right {
                        new_children.push(r);
                    }
                    inserted = true;
                    continue;
                }
            }
            remaining_offset -= child_size;
        } else if child.is_void() {
            if remaining_offset == 0 {
                for node in &insert_nodes {
                    new_children.push((*node).clone());
                }
                new_children.push(child.clone());
                inserted = true;
                continue;
            }
            remaining_offset -= 1;
            new_children.push(child.clone());
        } else {
            // Element child.
            if remaining_offset == 0 {
                for node in &insert_nodes {
                    new_children.push((*node).clone());
                }
                new_children.push(child.clone());
                inserted = true;
                continue;
            }
            remaining_offset -= child_size;
            new_children.push(child.clone());
        }
    }

    // If we haven't inserted yet, the offset is at the end.
    if !inserted {
        for node in &insert_nodes {
            new_children.push((*node).clone());
        }
    }

    merge_adjacent_text_nodes(new_children)
}

// ---------------------------------------------------------------------------
// Cross-parent deletion
// ---------------------------------------------------------------------------

use crate::model::ResolvedPos;

/// Delete content that spans two different parent blocks. The common case is
/// two sibling blocks under the same grandparent (e.g. selecting across two
/// paragraphs). This function:
/// 1. Keeps content before `from` in the first block
/// 2. Removes all blocks between the first and last
/// 3. Keeps content after `to` in the last block
/// 4. Merges the remaining content of first and last blocks into one block
fn apply_cross_parent_delete(
    doc: &Document,
    from: u32,
    to: u32,
    resolved_from: &ResolvedPos,
    resolved_to: &ResolvedPos,
) -> Result<(Document, StepMap), TransformError> {
    // Find the common ancestor. For sibling blocks under doc, both paths will
    // share a common prefix. We need the common prefix of node_path.
    let common_depth = common_prefix_len(&resolved_from.node_path, &resolved_to.node_path);

    // The common ancestor is the node reached by following path[..common_depth].
    let common_path = &resolved_from.node_path[..common_depth];
    let common_ancestor = doc
        .node_at(common_path)
        .ok_or_else(|| TransformError::OutOfBounds("common ancestor path invalid".to_string()))?;

    let common_content = common_ancestor.content().ok_or_else(|| {
        TransformError::InvalidTarget("common ancestor has no content".to_string())
    })?;

    // The first and last blocks at the common ancestor level.
    let first_child_idx = *resolved_from.node_path.get(common_depth).ok_or_else(|| {
        TransformError::InvalidRange(
            "cross-parent delete: from endpoint resolves to common ancestor boundary".to_string(),
        )
    })? as usize;
    let last_child_idx = *resolved_to.node_path.get(common_depth).ok_or_else(|| {
        TransformError::InvalidRange(
            "cross-parent delete: to endpoint resolves to common ancestor boundary".to_string(),
        )
    })? as usize;

    if first_child_idx >= last_child_idx {
        return Err(TransformError::InvalidRange(
            "cross-parent delete: first block index >= last block index".to_string(),
        ));
    }

    let first_block = common_content
        .child(first_child_idx)
        .ok_or_else(|| TransformError::OutOfBounds("first block not found".to_string()))?;
    let _last_block = common_content
        .child(last_child_idx)
        .ok_or_else(|| TransformError::OutOfBounds("last block not found".to_string()))?;

    // Compute the content offset within the first and last blocks.
    // The `from` position is inside the first block. We need to figure out
    // how deep we are in the first block's subtree and what content to keep.
    // For simplicity, handle the common case: both endpoints are directly
    // inside their respective text blocks (depth difference of 1 from common).
    let from_offset_in_first = resolved_from.parent_offset;
    let to_offset_in_last = resolved_to.parent_offset;

    // Get the first block's content before `from`, and last block's content
    // after `to`.
    let first_parent = resolved_from.parent(doc);
    let last_parent = resolved_to.parent(doc);

    // Keep left part of first block.
    let (left_children, _) = split_children_at(first_parent, from_offset_in_first);
    // Keep right part of last block.
    let (_, right_children) = split_children_at(last_parent, to_offset_in_last);

    // Merge the kept parts into one block (using the first block's type/attrs).
    let mut merged_children = left_children;
    merged_children.extend(right_children);
    let merged_children = merge_adjacent_text_nodes(merged_children);
    let merged_block = rebuild_element(first_block, merged_children);

    // Rebuild the common ancestor's children: keep children before first_child_idx,
    // insert the merged block, skip children between first and last (inclusive),
    // keep children after last_child_idx.
    let mut new_common_children: Vec<Node> =
        Vec::with_capacity(common_content.child_count() - (last_child_idx - first_child_idx));
    for (i, child) in common_content.iter().enumerate() {
        if i == first_child_idx {
            new_common_children.push(merged_block.clone());
        } else if i > first_child_idx && i <= last_child_idx {
            // Skip — these are being deleted.
        } else {
            new_common_children.push(child.clone());
        }
    }

    let new_common = rebuild_element(common_ancestor, new_common_children);
    let new_root = replace_node_at_path(doc.root(), common_path, &new_common);
    let new_doc = Document::new(new_root);
    let deleted_len = to - from;
    let map = StepMap::from_delete(from, deleted_len);

    Ok((new_doc, map))
}

/// Cross-parent replacement: delete across parent boundaries, then insert content.
fn apply_cross_parent_replace(
    doc: &Document,
    from: u32,
    to: u32,
    content: &Fragment,
    resolved_from: &ResolvedPos,
    resolved_to: &ResolvedPos,
) -> Result<(Document, StepMap), TransformError> {
    // First, perform the cross-parent delete.
    let (after_delete, delete_map) =
        apply_cross_parent_delete(doc, from, to, resolved_from, resolved_to)?;

    // Now insert the content at `from` in the post-delete document.
    if content.size() == 0 {
        return Ok((after_delete, delete_map));
    }

    // Resolve `from` in the post-delete doc and insert content there.
    let resolved_insert = after_delete
        .resolve(from)
        .map_err(|e| TransformError::OutOfBounds(e))?;

    let parent = resolved_insert.parent(&after_delete);
    let insert_offset = resolved_insert.parent_offset;

    let temp_parent = rebuild_element(parent, parent.content().unwrap().iter().cloned().collect());
    let after_insert = insert_nodes_in_children(&temp_parent, insert_offset, content);
    let new_parent = rebuild_element(parent, after_insert);

    let new_root =
        replace_node_at_path(after_delete.root(), &resolved_insert.node_path, &new_parent);
    let new_doc = Document::new(new_root);

    let deleted_len = to - from;
    let inserted_size = content.size();
    let map = StepMap::from_replace(from, deleted_len, inserted_size);

    Ok((new_doc, map))
}

/// Find the length of the common prefix of two paths.
fn common_prefix_len(a: &[u16], b: &[u16]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

// ---------------------------------------------------------------------------
// Tree rebuilding
// ---------------------------------------------------------------------------

/// Replace a node at the given path in the tree with two new nodes, returning
/// a new root. The node at `path` is removed and replaced by `first` and
/// `second` in that order.
fn replace_node_with_two(root: &Node, path: &[u16], first: &Node, second: &Node) -> Node {
    if path.is_empty() {
        panic!("replace_node_with_two called with empty path — cannot replace root with two nodes");
    }

    if path.len() == 1 {
        // We're at the direct parent of the node to replace.
        let content = root
            .content()
            .expect("non-leaf node in path must be an element");
        let replace_idx = path[0] as usize;
        let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count() + 1);

        for (i, child) in content.iter().enumerate() {
            if i == replace_idx {
                new_children.push(first.clone());
                new_children.push(second.clone());
            } else {
                new_children.push(child.clone());
            }
        }

        return rebuild_element(root, new_children);
    }

    // Recurse into the child indicated by path[0].
    let content = root
        .content()
        .expect("non-leaf node in path must be an element");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count() + 1);

    for (i, child) in content.iter().enumerate() {
        if i == path[0] as usize {
            new_children.push(replace_node_with_two(child, &path[1..], first, second));
        } else {
            new_children.push(child.clone());
        }
    }

    rebuild_element(root, new_children)
}

/// Replace a node at the given path with multiple nodes, returning a new root.
///
/// The node at `path` is removed and replaced by all nodes in `replacements`.
fn replace_node_with_many(root: &Node, path: &[u16], replacements: &[Node]) -> Node {
    if path.is_empty() {
        panic!("replace_node_with_many called with empty path — cannot replace root with multiple nodes");
    }

    if path.len() == 1 {
        let content = root
            .content()
            .expect("non-leaf node in path must be an element");
        let replace_idx = path[0] as usize;
        let mut new_children: Vec<Node> =
            Vec::with_capacity(content.child_count() + replacements.len() - 1);

        for (i, child) in content.iter().enumerate() {
            if i == replace_idx {
                new_children.extend(replacements.iter().cloned());
            } else {
                new_children.push(child.clone());
            }
        }

        return rebuild_element(root, new_children);
    }

    // Recurse into the child indicated by path[0].
    let content = root
        .content()
        .expect("non-leaf node in path must be an element");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count());

    for (i, child) in content.iter().enumerate() {
        if i == path[0] as usize {
            new_children.push(replace_node_with_many(child, &path[1..], replacements));
        } else {
            new_children.push(child.clone());
        }
    }

    rebuild_element(root, new_children)
}

/// Replace a node at the given path in the tree, returning a new root.
///
/// `path` is a sequence of child indices from the root. An empty path means
/// replace the root itself.
fn replace_node_at_path(root: &Node, path: &[u16], replacement: &Node) -> Node {
    if path.is_empty() {
        return replacement.clone();
    }

    let content = root
        .content()
        .expect("non-leaf node in path must be an element");
    let mut new_children: Vec<Node> = Vec::with_capacity(content.child_count());

    for (i, child) in content.iter().enumerate() {
        if i == path[0] as usize {
            new_children.push(replace_node_at_path(child, &path[1..], replacement));
        } else {
            new_children.push(child.clone());
        }
    }

    rebuild_element(root, new_children)
}

// ---------------------------------------------------------------------------
// Document validation
// ---------------------------------------------------------------------------

/// Validate that every node in the document satisfies its schema content rule.
///
/// This is called after all steps in a transaction have been applied.
pub(crate) fn validate_document(doc: &Document, schema: &Schema) -> Result<(), TransformError> {
    validate_node(doc.root(), schema)
}

fn validate_node(node: &Node, schema: &Schema) -> Result<(), TransformError> {
    if node.is_text() || node.is_void() {
        return Ok(());
    }

    let spec = match schema.node(node.node_type()) {
        Some(s) => s,
        None => {
            // Unknown node type — skip validation (lenient).
            return Ok(());
        }
    };

    let content = node.content().expect("element node should have content");

    // Validate each content rule part sequentially.
    let mut child_idx = 0;
    for part in &spec.content.parts {
        let mut count = 0u32;

        while child_idx < content.child_count() {
            let child = content.child(child_idx).unwrap();
            if child_matches_group(child, &part.group, schema) {
                count += 1;
                child_idx += 1;

                if let Some(max) = part.max {
                    if count >= max {
                        break;
                    }
                }
            } else {
                break;
            }
        }

        if count < part.min {
            return Err(TransformError::ContentViolation(format!(
                "node '{}' requires at least {} child(ren) matching '{}', found {}",
                node.node_type(),
                part.min,
                part.group,
                count
            )));
        }
    }

    // If there are remaining children not matched by any rule, that's an error.
    if child_idx < content.child_count() {
        let remaining = content.child(child_idx).unwrap();
        return Err(TransformError::ContentViolation(format!(
            "node '{}' has unexpected child '{}' at index {}",
            node.node_type(),
            remaining.node_type(),
            child_idx
        )));
    }

    // Recursively validate children.
    for i in 0..content.child_count() {
        validate_node(content.child(i).unwrap(), schema)?;
    }

    Ok(())
}

/// Check if a child node matches a content group name.
///
/// A child matches if:
/// - Its node_type equals the group name exactly, OR
/// - Its schema spec has `group == Some(group_name)`
fn child_matches_group(child: &Node, group: &str, schema: &Schema) -> bool {
    // Direct name match.
    if child.node_type() == group {
        return true;
    }

    // Group membership match.
    if let Some(child_spec) = schema.node(child.node_type()) {
        if child_spec.group.as_deref() == Some(group) {
            return true;
        }
    }

    false
}
