#[allow(clippy::too_many_arguments)]
fn apply_wrap_in_list(
    doc: &Document,
    from: u32,
    to: u32,
    list_type: &str,
    item_type: &str,
    list_attrs: &HashMap<String, serde_json::Value>,
    item_attrs: &HashMap<String, serde_json::Value>,
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
            item_attrs.clone(),
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
    // A semantically empty block resolves both range ends to its content
    // position. Keep that position between the newly inserted list/item opens
    // and closes so a following base-revision edit can still address the empty
    // textblock instead of being pushed to the list boundary.
    let mapped_end = if from == to { to + 3 } else { to + 2 };
    let map_end = StepMap::from_insert(mapped_end, total_added - 2);
    let map = map_start.compose(&map_end);

    Ok((new_doc, map))
}

fn apply_unwrap_from_list(
    doc: &Document,
    pos: u32,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    // Resolve the position to find which list item we're in.
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;

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
