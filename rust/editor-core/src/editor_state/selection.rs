#[cfg(test)]
fn can_toggle_blockquote_transaction_oracle(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> bool {
    let Some(blockquote_type) = schema
        .node_by_html_tag("blockquote")
        .map(|spec| spec.name.as_str())
    else {
        return false;
    };
    let pos = selection.from(document);
    let mut transaction = Transaction::new(Source::Format);
    if let Some((start, quote)) =
        containing_node_at(document, schema, pos, |_, name| name == blockquote_type)
    {
        let Some(content) = quote.content() else {
            return false;
        };
        transaction.add_step(Step::ReplaceRange {
            from: start,
            to: start.saturating_add(quote.node_size()),
            content: Fragment::from(content.iter().cloned().collect::<Vec<_>>()),
        });
    } else {
        let Some(range) = selected_block_range(
            document,
            schema,
            selection.from(document),
            selection.to(document),
        ) else {
            return false;
        };
        let Some(quote_spec) = schema.node(blockquote_type) else {
            return false;
        };
        let selected = range
            .selected_blocks
            .iter()
            .map(Node::node_type)
            .collect::<Vec<_>>();
        if !quote_spec.content.matches(&selected, |child, symbol| {
            schema.node_matches_symbol(child, symbol)
        }) {
            return false;
        }
        transaction.add_step(Step::ReplaceRange {
            from: range.replace_from,
            to: range.replace_to,
            content: Fragment::from(vec![Node::element(
                blockquote_type.to_string(),
                HashMap::new(),
                Fragment::from(range.selected_blocks),
            )]),
        });
    }
    transaction
        .apply_with_limits(document, schema, limits)
        .is_ok()
}

#[cfg(test)]
fn can_apply_list_type_transaction_oracle(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    list_type: &str,
    limits: &ResourceLimits,
) -> bool {
    if schema.node(list_type).is_none() {
        return false;
    }
    let pos = selection.from(document);
    let mut transaction = Transaction::new(Source::Format);
    if let Some((start, list)) = containing_node_at(document, schema, pos, |role, _| {
        matches!(role, NodeRole::List { .. })
    }) {
        if list.node_type() == list_type {
            transaction.add_step(Step::UnwrapFromList { pos });
        } else {
            transaction.add_step(Step::ReplaceRange {
                from: start,
                to: start.saturating_add(list.node_size()),
                content: Fragment::from(vec![Node::element(
                    list_type.to_string(),
                    list_attrs_for_type(list_type, list.attrs()),
                    list.content().cloned().unwrap_or_else(Fragment::empty),
                )]),
            });
        }
    } else {
        let Some(item_type) = schema.list_item_type_for(list_type) else {
            return false;
        };
        let range = selected_block_range(
            document,
            schema,
            selection.from(document),
            selection.to(document),
        );
        let in_quote = range
            .as_ref()
            .and_then(|range| document.node_at(&range.parent_path))
            .and_then(|parent| schema.node(parent.node_type()))
            .is_some_and(|spec| spec.html_tag.as_deref() == Some("blockquote"));
        if let Some(range) = range.filter(|_| in_quote) {
            let items = range
                .selected_blocks
                .into_iter()
                .map(|block| {
                    Node::element(
                        item_type.clone(),
                        HashMap::new(),
                        Fragment::from(vec![block]),
                    )
                })
                .collect::<Vec<_>>();
            transaction.add_step(Step::ReplaceRange {
                from: range.replace_from,
                to: range.replace_to,
                content: Fragment::from(vec![Node::element(
                    list_type.to_string(),
                    HashMap::new(),
                    Fragment::from(items),
                )]),
            });
        } else {
            transaction.add_step(Step::WrapInList {
                from: selection.from(document),
                to: selection.to(document),
                list_type: list_type.to_string(),
                item_type,
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            });
        }
    }
    transaction
        .apply_with_limits(document, schema, limits)
        .is_ok()
}

fn can_toggle_task_item(
    document: &Document,
    schema: &Schema,
    pos: u32,
    limits: &ResourceLimits,
) -> bool {
    let Some(path) = nearest_list_item_path(document, schema, pos) else {
        return false;
    };
    let Some(node) = document.node_at(&path) else {
        return false;
    };
    let Some(spec) = schema.node(node.node_type()) else {
        return false;
    };
    if !spec.attrs.contains_key("checked") {
        return false;
    }
    let mut attrs = node.attrs().clone();
    let checked = attrs
        .get("checked")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    attrs.insert("checked".into(), serde_json::Value::Bool(!checked));
    let Some(node_pos) = node_delete_start_pos(document, &path) else {
        return false;
    };
    let mut transaction = Transaction::new(Source::Input);
    transaction.add_step(Step::UpdateNodeAttrs {
        pos: node_pos,
        attrs,
    });
    transaction
        .apply_with_limits(document, schema, limits)
        .is_ok()
}

pub(crate) fn selected_text_block_range(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<BlockSelectionRange> {
    let range = selected_block_range(
        document,
        schema,
        selection.from(document),
        selection.to(document),
    )?;
    (!range.selected_blocks.is_empty()
        && range.selected_blocks.iter().all(|block| {
            schema
                .node(block.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        }))
    .then_some(range)
}

pub(crate) fn can_replace_selected_text_blocks(
    document: &Document,
    schema: &Schema,
    range: &BlockSelectionRange,
    target_type: &str,
) -> bool {
    let Some(target_spec) = schema.node(target_type) else {
        return false;
    };
    if !matches!(target_spec.role, NodeRole::TextBlock) {
        return false;
    }
    let replacement = vec![target_type; range.selected_blocks.len()];
    parent_accepts_range_replacement(document, schema, range, &replacement)
}

pub(crate) fn selected_block_range(
    document: &Document,
    schema: &Schema,
    from: u32,
    to: u32,
) -> Option<BlockSelectionRange> {
    let start_path = block_path_for_pos(document, schema, from)?;
    let end_path = block_path_for_pos(document, schema, if to > from { to - 1 } else { from })?;
    let start_parent = &start_path[..start_path.len().saturating_sub(1)];
    let end_parent = &end_path[..end_path.len().saturating_sub(1)];
    if start_parent != end_parent {
        return None;
    }
    let parent_path = start_parent.to_vec();
    let parent = if parent_path.is_empty() {
        document.root()
    } else {
        document.node_at(&parent_path)?
    };
    let first = usize::try_from(*start_path.last()?).ok()?;
    let last = usize::try_from(*end_path.last()?).ok()?;
    if first > last {
        return None;
    }
    let selected_blocks = (first..=last)
        .map(|index| parent.child(index).cloned())
        .collect::<Option<Vec<_>>>()?;
    let replace_from = node_delete_start_pos(document, &start_path)?;
    let replace_to =
        node_delete_start_pos(document, &end_path)?.checked_add(parent.child(last)?.node_size())?;
    Some(BlockSelectionRange {
        parent_path,
        first_child_index: first,
        replace_from,
        replace_to,
        selected_blocks,
    })
}

fn block_path_for_pos(document: &Document, schema: &Schema, pos: u32) -> Option<Vec<u32>> {
    let resolved = document.resolve(pos).ok()?;
    let mut node = document.root();
    let mut path = Vec::new();
    let mut block_path = None;
    for index in resolved.node_path {
        let child = node.child(index as usize)?;
        path.push(index);
        let role = &schema.node(child.node_type())?.role;
        if matches!(
            role,
            NodeRole::TextBlock | NodeRole::Block | NodeRole::List { .. }
        ) {
            block_path = Some(path.clone());
        }
        node = child;
    }
    block_path
}

pub(crate) fn containing_node_at<'a>(
    document: &'a Document,
    schema: &Schema,
    pos: u32,
    matches_node: impl Fn(&NodeRole, &str) -> bool,
) -> Option<(u32, &'a Node)> {
    let resolved = document.resolve(pos).ok()?;
    let mut node = document.root();
    let mut content_start = 0u32;
    let mut nearest = None;
    for index in resolved.node_path {
        let content = node.content()?;
        let mut child_open = content_start;
        for sibling in content.iter().take(index as usize) {
            child_open = child_open.checked_add(sibling.node_size())?;
        }
        let child = content.child(index as usize)?;
        let spec = schema.node(child.node_type())?;
        if matches_node(&spec.role, child.node_type()) {
            nearest = Some((child_open, child));
        }
        if !child.is_element() {
            break;
        }
        node = child;
        content_start = child_open.checked_add(1)?;
    }
    nearest
}

fn list_item_context_at(document: &Document, schema: &Schema, pos: u32) -> Option<ListItemContext> {
    let resolved = document.resolve(pos).ok()?;
    let path = &resolved.node_path;
    let mut node = document.root();
    let mut list_item_depth = None;
    for (depth, index) in path.iter().copied().enumerate() {
        let child = node.child(index as usize)?;
        if schema
            .node(child.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
        {
            list_item_depth = Some(depth);
        }
        node = child;
    }
    let depth = list_item_depth?;
    let parent_is_list_item = depth > 0
        && document
            .node_at(&path[..depth - 1])
            .and_then(|node| schema.node(node.node_type()))
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem));
    Some(ListItemContext {
        list_item_index: usize::try_from(path[depth]).ok()?,
        parent_is_list_item,
    })
}

fn nearest_list_item_path(document: &Document, schema: &Schema, pos: u32) -> Option<Vec<u32>> {
    let resolved = document.resolve(pos).ok()?;
    let mut node = document.root();
    let mut path = Vec::new();
    let mut nearest = None;
    for index in resolved.node_path {
        node = node.child(index as usize)?;
        path.push(index);
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
        {
            nearest = Some(path.clone());
        }
    }
    nearest
}

fn node_delete_start_pos(document: &Document, path: &[u32]) -> Option<u32> {
    let mut current = document.root();
    let mut open_pos = 0u32;
    for index in path.iter().copied() {
        let content = current.content()?;
        let mut child_open = open_pos.checked_add(1)?;
        for sibling in content.iter().take(index as usize) {
            child_open = child_open.checked_add(sibling.node_size())?;
        }
        current = content.child(index as usize)?;
        open_pos = child_open;
    }
    open_pos.checked_sub(1)
}
