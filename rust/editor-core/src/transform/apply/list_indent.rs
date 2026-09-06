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
        Some(node)
            if schema
                .node(node.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem)) =>
        {
            node
        }
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
    list_path: Vec<u32>,
    list_node: &'a Node,
    list_item_idx: usize,
}

fn resolve_list_item_context<'a>(
    doc: &'a Document,
    pos: u32,
    schema: &Schema,
) -> Result<ListItemContext<'a>, TransformError> {
    let resolved = doc.resolve(pos).map_err(TransformError::OutOfBounds)?;
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
