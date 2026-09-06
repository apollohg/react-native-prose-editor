#[derive(Clone)]
pub(crate) struct BlockSelectionRange {
    pub(crate) parent_path: Vec<u32>,
    pub(crate) first_child_index: usize,
    pub(crate) replace_from: u32,
    pub(crate) replace_to: u32,
    pub(crate) selected_blocks: Vec<Node>,
}

struct ListItemContext {
    list_item_index: usize,
    parent_is_list_item: bool,
}

pub(crate) fn plan_content_insertion(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    content: &Fragment,
) -> Option<CommandReplacement> {
    if content.size() == 0 {
        return None;
    }
    let from = selection.from(document);
    let to = selection.to(document);
    let is_block = content.iter().all(|node| {
        schema.node(node.node_type()).is_some_and(|spec| {
            matches!(
                spec.role,
                NodeRole::TextBlock | NodeRole::List { .. } | NodeRole::Block
            )
        })
    });
    if !is_block {
        return Some(CommandReplacement {
            from,
            to,
            content: content.clone(),
            selection_after: Selection::cursor(from.saturating_add(content.size())),
        });
    }
    let insert_at = block_insert_position(document, schema, from)?;
    let (replace_from, replace_to) =
        empty_text_block_range(document, schema, from).unwrap_or((insert_at, insert_at));
    let inserted_size = content.size();
    let mut nodes = content.iter().cloned().collect::<Vec<_>>();
    let ends_with_text_block = content
        .iter()
        .last()
        .and_then(|node| schema.node(node.node_type()))
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock));
    let selection_after = if ends_with_text_block {
        Selection::cursor(replace_from.saturating_add(inserted_size.saturating_sub(1)))
    } else {
        let paragraph = schema.preferred_text_block()?;
        let attrs = paragraph
            .attrs
            .iter()
            .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
            .collect();
        nodes.push(Node::element(
            paragraph.name.clone(),
            attrs,
            Fragment::empty(),
        ));
        Selection::cursor(replace_from.saturating_add(inserted_size).saturating_add(1))
    };
    Some(CommandReplacement {
        from: replace_from,
        to: replace_to,
        content: Fragment::from(nodes),
        selection_after,
    })
}

fn empty_text_block_range(
    document: &Document,
    schema: &Schema,
    position: u32,
) -> Option<(u32, u32)> {
    let resolved = document.resolve(position).ok()?;
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || block.content_size() != 0
    {
        return None;
    }
    let start = node_delete_start_pos(document, &resolved.node_path)?;
    Some((start, start.checked_add(block.node_size())?))
}

fn block_insert_position(document: &Document, schema: &Schema, position: u32) -> Option<u32> {
    let resolved = document.resolve(position).ok()?;
    let parent = resolved.parent(document);
    if !schema
        .node(parent.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
    {
        return Some(position);
    }
    let start = node_delete_start_pos(document, &resolved.node_path)?;
    start.checked_add(parent.node_size())
}

fn can_toggle_heading(
    document: &Document,
    schema: &Schema,
    range: Option<&BlockSelectionRange>,
    level: u8,
) -> bool {
    let Some(target_type) = schema
        .node_by_html_tag(&format!("h{level}"))
        .map(|spec| spec.name.as_str())
    else {
        return false;
    };
    let Some(paragraph_type) = paragraph_node_name(schema) else {
        return false;
    };
    let Some(range) = range.filter(|range| selected_blocks_are_text_blocks(schema, range)) else {
        return false;
    };
    let replacement_type = if range
        .selected_blocks
        .iter()
        .all(|block| block.node_type() == target_type)
    {
        paragraph_type
    } else {
        target_type
    };
    can_replace_selected_text_blocks(document, schema, range, replacement_type)
}

fn can_toggle_code_block(
    document: &Document,
    schema: &Schema,
    range: Option<&BlockSelectionRange>,
) -> bool {
    let Some(code_block_type) = crate::command_planner::code_block_node_name(schema) else {
        return false;
    };
    let Some(paragraph_type) = paragraph_node_name(schema) else {
        return false;
    };
    let Some(range) = range.filter(|range| selected_blocks_are_text_blocks(schema, range)) else {
        return false;
    };
    let replacement_type = if range
        .selected_blocks
        .iter()
        .all(|block| block.node_type() == code_block_type)
    {
        paragraph_type
    } else {
        code_block_type
    };
    can_replace_selected_text_blocks(document, schema, range, replacement_type)
}

fn can_toggle_blockquote_local(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
    document_node_count: usize,
    block_range: Option<&BlockSelectionRange>,
) -> bool {
    let Some(blockquote_type) = schema
        .node_by_html_tag("blockquote")
        .map(|spec| spec.name.as_str())
    else {
        return false;
    };
    let pos = selection.from(document);
    if let Some((path, quote)) =
        containing_node_path_at(document, schema, pos, |_, name| name == blockquote_type)
    {
        let Some(content) = quote.content() else {
            return false;
        };
        let replacement = content.iter().map(Node::node_type).collect::<Vec<_>>();
        return parent_accepts_path_replacement(document, schema, &path, &replacement);
    }

    let Some(range) = block_range else {
        return false;
    };
    let Some(quote_spec) = schema.node(blockquote_type) else {
        return false;
    };
    if !attrs_are_valid(schema, blockquote_type, &HashMap::new()) {
        return false;
    }
    let selected_types = range
        .selected_blocks
        .iter()
        .map(Node::node_type)
        .collect::<Vec<_>>();
    if !quote_spec
        .content
        .matches(&selected_types, |child, symbol| {
            schema.node_matches_symbol(child, symbol)
        })
        || !parent_accepts_range_replacement(document, schema, range, &[blockquote_type])
    {
        return false;
    }
    let deepest = range
        .selected_blocks
        .iter()
        .map(node_relative_depth)
        .max()
        .unwrap_or(0)
        .saturating_add(range.parent_path.len())
        .saturating_add(2);
    resource_growth_fits(document_node_count, limits, 1, deepest)
}

#[allow(clippy::too_many_arguments)]
fn can_apply_list_type_local(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    list_type: &str,
    limits: &ResourceLimits,
    document_node_count: usize,
    block_range: Option<&BlockSelectionRange>,
    root_wrap_range: Option<&BlockSelectionRange>,
) -> bool {
    if schema.node(list_type).is_none() {
        return false;
    }
    let pos = selection.from(document);
    if let Some((list_path, list)) = containing_node_path_at(document, schema, pos, |role, _| {
        matches!(role, NodeRole::List { .. })
    }) {
        if list.node_type() == list_type {
            return can_unwrap_list_item_local(document, schema, pos, &list_path, list);
        }
        let attrs = list_attrs_for_type(list_type, list.attrs());
        let Some(content) = list.content() else {
            return false;
        };
        let child_types = content.iter().map(Node::node_type).collect::<Vec<_>>();
        return attrs_are_valid(schema, list_type, &attrs)
            && schema.node(list_type).is_some_and(|spec| {
                spec.content.matches(&child_types, |child, symbol| {
                    schema.node_matches_symbol(child, symbol)
                })
            })
            && parent_accepts_path_replacement(document, schema, &list_path, &[list_type]);
    }

    let Some(item_type) = schema.list_item_type_for(list_type) else {
        return false;
    };
    let in_quote = block_range
        .and_then(|range| document.node_at(&range.parent_path))
        .and_then(|parent| schema.node(parent.node_type()))
        .is_some_and(|spec| spec.html_tag.as_deref() == Some("blockquote"));
    if let Some(range) = block_range.filter(|_| in_quote) {
        return can_wrap_range_in_list_local(
            document,
            schema,
            range,
            list_type,
            &item_type,
            limits,
            document_node_count,
        );
    }
    let Some(range) = root_wrap_range else {
        return false;
    };
    can_wrap_range_in_list_local(
        document,
        schema,
        range,
        list_type,
        &item_type,
        limits,
        document_node_count,
    )
}

fn can_wrap_range_in_list_local(
    document: &Document,
    schema: &Schema,
    range: &BlockSelectionRange,
    list_type: &str,
    item_type: &str,
    limits: &ResourceLimits,
    document_node_count: usize,
) -> bool {
    let selected_types = range
        .selected_blocks
        .iter()
        .map(Node::node_type)
        .collect::<Vec<_>>();
    can_build_list(schema, list_type, item_type, &selected_types)
        && parent_accepts_range_replacement(document, schema, range, &[list_type])
        && resource_growth_fits(
            document_node_count,
            limits,
            selected_types.len().saturating_add(1),
            range
                .selected_blocks
                .iter()
                .map(node_relative_depth)
                .max()
                .unwrap_or(0)
                .saturating_add(range.parent_path.len())
                .saturating_add(3),
        )
}

fn root_wrap_range(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<BlockSelectionRange> {
    let from = selection.from(document);
    let to = selection.to(document);
    if from > to {
        return None;
    }
    let content = document.root().content()?;
    let mut offset = 0u32;
    let mut first = None;
    let mut last = None;
    for (index, child) in content.iter().enumerate() {
        let end = offset.saturating_add(child.node_size());
        if end > from && offset < to {
            if schema
                .node(child.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::List { .. }))
            {
                return None;
            }
            first.get_or_insert(index);
            last = Some(index);
        }
        offset = end;
    }
    let (Some(first), Some(last)) = (first, last) else {
        return None;
    };
    let selected_blocks = content
        .iter()
        .skip(first)
        .take(last.saturating_sub(first).saturating_add(1))
        .cloned()
        .collect::<Vec<_>>();
    Some(BlockSelectionRange {
        parent_path: Vec::new(),
        first_child_index: first,
        replace_from: 0,
        replace_to: 0,
        selected_blocks,
    })
}

fn selected_blocks_are_text_blocks(schema: &Schema, range: &BlockSelectionRange) -> bool {
    !range.selected_blocks.is_empty()
        && range.selected_blocks.iter().all(|block| {
            schema
                .node(block.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        })
}

fn can_build_list(
    schema: &Schema,
    list_type: &str,
    item_type: &str,
    selected_types: &[&str],
) -> bool {
    let Some(list_spec) = schema.node(list_type) else {
        return false;
    };
    let Some(item_spec) = schema.node(item_type) else {
        return false;
    };
    matches!(list_spec.role, NodeRole::List { .. })
        && matches!(item_spec.role, NodeRole::ListItem)
        && attrs_are_valid(schema, list_type, &HashMap::new())
        && attrs_are_valid(schema, item_type, &HashMap::new())
        && list_spec
            .content
            .matches(&vec![item_type; selected_types.len()], |child, symbol| {
                schema.node_matches_symbol(child, symbol)
            })
        && selected_types.iter().all(|selected| {
            item_spec.content.matches(&[*selected], |child, symbol| {
                schema.node_matches_symbol(child, symbol)
            })
        })
}

fn can_unwrap_list_item_local(
    document: &Document,
    schema: &Schema,
    pos: u32,
    expected_list_path: &[u32],
    list: &Node,
) -> bool {
    let Ok(resolved) = document.resolve(pos) else {
        return false;
    };
    let mut node = document.root();
    let mut item_depth = None;
    for (depth, index) in resolved.node_path.iter().copied().enumerate() {
        let Some(child) = node.child(index as usize) else {
            return false;
        };
        if schema
            .node(child.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
        {
            item_depth = Some(depth);
        }
        node = child;
    }
    let Some(item_depth) = item_depth else {
        return false;
    };
    let list_path = &resolved.node_path[..item_depth];
    if list_path != expected_list_path {
        return false;
    }
    let item_index = resolved.node_path[item_depth] as usize;
    let Some(list_content) = list.content() else {
        return false;
    };
    let Some(item) = list_content.child(item_index) else {
        return false;
    };
    let Some(item_content) = item.content() else {
        return false;
    };
    let extracted = item_content.iter().map(Node::node_type).collect::<Vec<_>>();
    let total = list_content.child_count();
    let mut replacement = Vec::new();
    if total == 1 {
        replacement.extend(extracted);
    } else if item_index == 0 {
        replacement.extend(extracted);
        replacement.push(list.node_type());
        if !list_subset_is_valid(schema, list, 1, total) {
            return false;
        }
    } else if item_index + 1 == total {
        replacement.push(list.node_type());
        replacement.extend(extracted);
        if !list_subset_is_valid(schema, list, 0, item_index) {
            return false;
        }
    } else {
        replacement.push(list.node_type());
        replacement.extend(extracted);
        replacement.push(list.node_type());
        if !list_subset_is_valid(schema, list, 0, item_index)
            || !list_subset_is_valid(schema, list, item_index + 1, total)
        {
            return false;
        }
    }
    parent_accepts_path_replacement(document, schema, list_path, &replacement)
}

fn list_subset_is_valid(schema: &Schema, list: &Node, from: usize, to: usize) -> bool {
    let Some(spec) = schema.node(list.node_type()) else {
        return false;
    };
    let Some(content) = list.content() else {
        return false;
    };
    let types = content
        .iter()
        .skip(from)
        .take(to.saturating_sub(from))
        .map(Node::node_type)
        .collect::<Vec<_>>();
    spec.content.matches(&types, |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    })
}

fn attrs_are_valid(
    schema: &Schema,
    node_type: &str,
    attrs: &HashMap<String, serde_json::Value>,
) -> bool {
    schema.node(node_type).is_some_and(|spec| {
        spec.attrs
            .iter()
            .all(|(name, attr)| attr.has_default || attrs.contains_key(name))
            && (spec.allow_undeclared_attrs
                || attrs.keys().all(|name| spec.attrs.contains_key(name)))
    })
}

fn parent_accepts_path_replacement(
    document: &Document,
    schema: &Schema,
    path: &[u32],
    replacement: &[&str],
) -> bool {
    let Some((&child_index, parent_path)) = path.split_last() else {
        return false;
    };
    parent_accepts_replacement(
        document,
        schema,
        parent_path,
        child_index as usize,
        1,
        replacement,
    )
}

fn parent_accepts_range_replacement(
    document: &Document,
    schema: &Schema,
    range: &BlockSelectionRange,
    replacement: &[&str],
) -> bool {
    parent_accepts_replacement(
        document,
        schema,
        &range.parent_path,
        range.first_child_index,
        range.selected_blocks.len(),
        replacement,
    )
}

fn parent_accepts_replacement(
    document: &Document,
    schema: &Schema,
    parent_path: &[u32],
    first: usize,
    removed: usize,
    replacement: &[&str],
) -> bool {
    let parent = if parent_path.is_empty() {
        document.root()
    } else {
        let Some(parent) = document.node_at(parent_path) else {
            return false;
        };
        parent
    };
    let Some(parent_spec) = schema.node(parent.node_type()) else {
        return false;
    };
    let Some(content) = parent.content() else {
        return false;
    };
    if first > content.child_count() || first.saturating_add(removed) > content.child_count() {
        return false;
    }
    // Content expressions observe a child only through the symbols it
    // matches. Equal-length replacements with identical signatures preserve
    // an already-admitted parent sequence, so the shared selection range does
    // not need to rescan every immutable sibling for each command target.
    if removed == replacement.len()
        && content
            .iter()
            .skip(first)
            .take(removed)
            .zip(replacement.iter().copied())
            .all(|(current, candidate)| {
                parent_spec.content.symbols().all(|symbol| {
                    schema.node_matches_symbol(current.node_type(), symbol)
                        == schema.node_matches_symbol(candidate, symbol)
                })
            })
    {
        return true;
    }
    let mut child_types = Vec::with_capacity(
        content
            .child_count()
            .saturating_sub(removed)
            .saturating_add(replacement.len()),
    );
    child_types.extend(content.iter().take(first).map(Node::node_type));
    child_types.extend(replacement.iter().copied());
    child_types.extend(
        content
            .iter()
            .skip(first.saturating_add(removed))
            .map(Node::node_type),
    );
    parent_spec.content.matches(&child_types, |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    })
}

fn resource_growth_fits(
    document_node_count: usize,
    limits: &ResourceLimits,
    added_nodes: usize,
    deepest_new_node: usize,
) -> bool {
    document_node_count
        .checked_add(added_nodes)
        .is_some_and(|count| count <= limits.max_document_nodes)
        && deepest_new_node <= limits.max_document_depth
}
