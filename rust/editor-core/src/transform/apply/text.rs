fn apply_insert_text(
    doc: &Document,
    pos: u32,
    insert_text: &str,
    marks: &[Mark],
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;
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

    let resolved_from = doc.resolve(from).map_err(TransformError::OutOfBounds)?;
    let resolved_to = doc.resolve(to).map_err(TransformError::OutOfBounds)?;

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

fn apply_add_mark(
    doc: &Document,
    from: u32,
    to: u32,
    mark: &Mark,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    if from >= to {
        // No-op: empty range.
        return Ok((doc.clone(), StepMap::empty()));
    }

    let resolved_from = doc.resolve(from).map_err(TransformError::OutOfBounds)?;
    let resolved_to = doc.resolve(to).map_err(TransformError::OutOfBounds)?;

    if resolved_from.node_path != resolved_to.node_path {
        return Err(TransformError::InvalidRange(
            "mark range spans different parent nodes".to_string(),
        ));
    }

    let parent = resolved_from.parent(doc);
    let from_offset = resolved_from.parent_offset;
    let to_offset = resolved_to.parent_offset;

    let new_children = add_mark_in_children(parent, from_offset, to_offset, mark, schema);
    let new_parent = rebuild_element(parent, new_children);

    let new_root = replace_node_at_path(doc.root(), &resolved_from.node_path, &new_parent);
    let new_doc = Document::new(new_root);

    // Mark operations don't change positions.
    Ok((new_doc, StepMap::empty()))
}

/// Add a mark to all text within `[from_offset, to_offset)` in a parent's children.
fn add_mark_in_children(
    parent: &Node,
    from_offset: u32,
    to_offset: u32,
    mark: &Mark,
    schema: &Schema,
) -> Vec<Node> {
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
                let mut new_marks = add_mark_to_set(child.marks(), mark);
                new_marks.sort_by(|left, right| {
                    schema
                        .mark_rank(left.mark_type())
                        .unwrap_or(usize::MAX)
                        .cmp(&schema.mark_rank(right.mark_type()).unwrap_or(usize::MAX))
                        .then_with(|| left.mark_type().cmp(right.mark_type()))
                });
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

fn apply_remove_mark(
    doc: &Document,
    from: u32,
    to: u32,
    mark_type: &str,
) -> Result<(Document, StepMap), TransformError> {
    if from >= to {
        return Ok((doc.clone(), StepMap::empty()));
    }

    let resolved_from = doc.resolve(from).map_err(TransformError::OutOfBounds)?;
    let resolved_to = doc.resolve(to).map_err(TransformError::OutOfBounds)?;

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
