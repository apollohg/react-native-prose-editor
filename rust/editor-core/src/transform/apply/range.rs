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

    let resolved_from = doc.resolve(from).map_err(TransformError::OutOfBounds)?;

    if from == to && content.size() == 0 {
        // No-op: empty range and empty content.
        return Ok((doc.clone(), StepMap::empty()));
    }

    // If from != to, resolve `to` and check if same parent.
    if from != to {
        let resolved_to = doc.resolve(to).map_err(TransformError::OutOfBounds)?;

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
            new_children.push(child.clone());
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
        .map_err(TransformError::OutOfBounds)?;

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
fn common_prefix_len(a: &[u32], b: &[u32]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

/// Replace a node at the given path in the tree with two new nodes, returning
/// a new root. The node at `path` is removed and replaced by `first` and
/// `second` in that order.
fn replace_node_with_two(root: &Node, path: &[u32], first: &Node, second: &Node) -> Node {
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
fn replace_node_with_many(root: &Node, path: &[u32], replacements: &[Node]) -> Node {
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
fn replace_node_at_path(root: &Node, path: &[u32], replacement: &Node) -> Node {
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
