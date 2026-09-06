fn apply_split_block(
    doc: &Document,
    pos: u32,
    new_node_type: &str,
    new_attrs: &HashMap<String, serde_json::Value>,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;

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

fn apply_join_blocks(doc: &Document, pos: u32) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;

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
