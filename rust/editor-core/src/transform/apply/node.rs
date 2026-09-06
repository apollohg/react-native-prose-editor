fn apply_insert_node(
    doc: &Document,
    pos: u32,
    node: &Node,
    _schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;
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

fn apply_update_node_attrs(
    doc: &Document,
    pos: u32,
    attrs: &HashMap<String, serde_json::Value>,
) -> Result<(Document, StepMap), TransformError> {
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;
    let parent = resolved.parent(doc);
    let content = parent
        .content()
        .ok_or_else(|| TransformError::InvalidTarget("parent node has no content".to_string()))?;

    let mut offset = 0;
    let mut target_index = None;
    for (index, child) in content.iter().enumerate() {
        let child_size = child.node_size();
        let matches = !child.is_text() && resolved.parent_offset == offset;
        if matches {
            target_index = Some(index);
            break;
        }
        offset += child_size;
    }

    let target_index = target_index.ok_or_else(|| {
        TransformError::InvalidTarget(format!("position {pos} does not resolve to a node"))
    })?;
    let target_child = content.child(target_index).ok_or_else(|| {
        TransformError::OutOfBounds(format!("node at index {target_index} not found"))
    })?;
    if target_child.is_text() {
        return Err(TransformError::InvalidTarget(
            "cannot update attrs on a text node".to_string(),
        ));
    }

    let replacement = if target_child.is_void() {
        Node::void(target_child.node_type().to_string(), attrs.clone())
    } else {
        Node::element(
            target_child.node_type().to_string(),
            attrs.clone(),
            target_child
                .content()
                .cloned()
                .unwrap_or_else(Fragment::empty),
        )
    };

    let mut new_children = Vec::with_capacity(content.child_count());
    for (index, child) in content.iter().enumerate() {
        if index == target_index {
            new_children.push(replacement.clone());
        } else {
            new_children.push(child.clone());
        }
    }

    let new_parent = rebuild_element(parent, new_children);
    let new_root = replace_node_at_path(doc.root(), &resolved.node_path, &new_parent);
    Ok((Document::new(new_root), StepMap::empty()))
}
