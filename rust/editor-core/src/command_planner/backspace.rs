fn empty_list_unwrap_pos(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
) -> Option<u32> {
    if scalar_from >= scalar_to
        || doc_from != doc_to
        || map.doc_to_scalar(doc_to, document) != scalar_to
    {
        return None;
    }
    let resolved = document.resolve(doc_to).ok()?;
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || resolved.parent_offset != 0
        || block.content_size() != 0
    {
        return None;
    }
    let depth = nearest_list_item_depth(document, schema, &resolved.node_path)?;
    (*resolved.node_path.get(depth + 1)? == 0).then_some(doc_to)
}

fn marker_backspace_action(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    if scalar_from >= scalar_to
        || doc_from != doc_to
        || map.doc_to_scalar(doc_to, document) != scalar_to
    {
        return None;
    }
    let resolved = document.resolve(doc_to).ok()?;
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || resolved.parent_offset != 0
        || block.content_size() == 0
    {
        return None;
    }
    let depth = nearest_list_item_depth(document, schema, &resolved.node_path)?;
    if *resolved.node_path.get(depth + 1)? != 0 {
        return None;
    }
    let item_index = usize::try_from(resolved.node_path[depth]).ok()?;
    if item_index > 0 {
        // A later bullet merges into the one above it. Joining the two items
        // alone would leave their paragraphs side by side inside a single
        // bullet, so the paragraphs are joined too.
        //
        // Plan operations apply in sequence without remapping, so the second
        // position is stated in post-join coordinates: joining two blocks
        // removes exactly two tokens — the previous item's close and this
        // item's open — and the paragraph boundary sits after both.
        let item_boundary = node_delete_start(document, &resolved.node_path[..=depth])?;
        let paragraph_boundary =
            node_delete_start(document, &resolved.node_path)?.checked_sub(2)?;
        let joined_text_boundary = paragraph_boundary.checked_sub(1)?;
        return Some(SemanticCommandPlan {
            operations: vec![
                SemanticOperation::JoinBlocks { pos: item_boundary },
                SemanticOperation::JoinBlocks {
                    pos: paragraph_boundary,
                },
            ],
            selection_after: Some(Selection::cursor(joined_text_boundary)),
            history: SemanticCommandHistory::InputBoundary,
        });
    }
    // First bullet of a nested list: step it out one level, the mirror of Tab.
    // Unwrapping instead would drop it into the parent bullet as a second
    // paragraph, losing the bullet entirely.
    if nearest_list_item_depth(document, schema, &resolved.node_path[..depth]).is_some() {
        return Some(SemanticCommandPlan::one(
            SemanticOperation::OutdentListItem { pos: doc_to },
        ));
    }
    // First bullet of a top-level list: leave the list altogether.
    Some(SemanticCommandPlan::one(
        SemanticOperation::UnwrapFromList { pos: doc_to },
    ))
}

fn move_into_previous_blockquote_action(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    if scalar_from >= scalar_to || map.doc_to_scalar(doc_to, document) != scalar_to {
        return None;
    }
    let resolved = document.resolve(doc_to).ok()?;
    let block = resolved.parent(document);
    if !is_text_block(schema, block) || resolved.parent_offset != 0 || block.content_size() == 0 {
        return None;
    }

    let (&index, parent_path) = resolved.node_path.split_last()?;
    if index == 0 {
        return None;
    }
    let parent = document.node_at(parent_path)?;
    let parent_content = parent.content()?;
    let previous_index = usize::try_from(index).ok()?.checked_sub(1)?;
    let previous = parent_content.child(previous_index)?;
    let quote_type = schema.node_by_html_tag("blockquote")?.name.as_str();
    if previous.node_type() != quote_type {
        return None;
    }

    let quote_content = previous.content()?;
    let quote_child_types = quote_content
        .iter()
        .map(Node::node_type)
        .chain(std::iter::once(block.node_type()))
        .collect::<Vec<_>>();
    let quote_spec = schema.node(quote_type)?;
    if !quote_spec
        .content
        .matches(&quote_child_types, |child, symbol| {
            schema.node_matches_symbol(child, symbol)
        })
    {
        return None;
    }
    let remaining_parent_types = parent_content
        .iter()
        .enumerate()
        .filter(|(child_index, _)| *child_index != previous_index + 1)
        .map(|(_, child)| child.node_type())
        .collect::<Vec<_>>();
    let parent_spec = schema.node(parent.node_type())?;
    if !parent_spec
        .content
        .matches(&remaining_parent_types, |child, symbol| {
            schema.node_matches_symbol(child, symbol)
        })
    {
        return None;
    }

    let mut children = quote_content.iter().cloned().collect::<Vec<_>>();
    children.push(block.clone());
    let replacement = Node::element(
        previous.node_type().to_string(),
        previous.attrs().clone(),
        Fragment::from(children),
    );
    let mut previous_path = parent_path.to_vec();
    previous_path.push(index - 1);
    let from = node_delete_start(document, &previous_path)?;
    let block_start = node_delete_start_pos(document, &resolved.node_path)?;
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from,
            to: block_start.checked_add(block.node_size())?,
            content: Fragment::from(vec![replacement]),
        }],
        selection_after: Some(Selection::cursor(block_start)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

/// Backspace at the very start of a text block that still has content joins it
/// onto the block before it — what every comparable editor does.
///
/// [`marker_backspace_action`] already implements this for list items, where
/// the surrounding item structure makes the target unambiguous. Every other
/// text block (paragraph, heading, a line inside a blockquote) fell through to
/// the `DeleteRange` fallback at the end of [`text::plan_delete_scalar_range`],
/// which asks the engine to delete a range whose start sits on a block boundary
/// rather than inside tracked text. The engine correctly refuses that as a
/// cross-parent structural deletion, so the keystroke failed outright instead
/// of merging the two blocks.
///
/// Only a join against a previous *text block* sibling is planned here. Void
/// siblings are handled by `delete_previous_void_block_action`.
fn join_with_previous_block_action(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    if scalar_from >= scalar_to || map.doc_to_scalar(doc_to, document) != scalar_to {
        return None;
    }
    let resolved = document.resolve(doc_to).ok()?;
    let block = resolved.parent(document);
    if !is_text_block(schema, block) || resolved.parent_offset != 0 || block.content_size() == 0 {
        return None;
    }

    // The caret is at the head of this block; joining needs a preceding sibling
    // within the same parent.
    let (&index, ancestors) = resolved.node_path.split_last()?;
    if index == 0 {
        return None;
    }
    let mut parent = document.root();
    for &ancestor in ancestors {
        parent = parent.child(ancestor as usize)?;
    }
    let previous = parent.content()?.child(usize::try_from(index).ok()? - 1)?;
    if !is_text_block(schema, previous) {
        return None;
    }

    let block_boundary = node_delete_start_pos(document, &resolved.node_path)?;
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::JoinBlocks {
            pos: block_boundary,
        }],
        selection_after: Some(Selection::cursor(block_boundary.checked_sub(1)?)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn is_text_block(schema: &Schema, node: &crate::model::Node) -> bool {
    schema
        .node(node.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
}

/// Position of the boundary immediately before the node at `path` — the
/// `JoinBlocks` anchor.
fn node_delete_start_pos(document: &Document, path: &[u32]) -> Option<u32> {
    let mut node = document.root();
    let mut open = 0u32;
    for index in path.iter().copied() {
        let content = node.content()?;
        let mut child_open = open.checked_add(1)?;
        for sibling in content.iter().take(index as usize) {
            child_open = child_open.checked_add(sibling.node_size())?;
        }
        node = content.child(index as usize)?;
        open = child_open;
    }
    open.checked_sub(1)
}

fn nearest_list_item_depth(document: &Document, schema: &Schema, path: &[u32]) -> Option<usize> {
    let mut node = document.root();
    let mut found = None;
    for (depth, index) in path.iter().copied().enumerate() {
        node = node.child(index as usize)?;
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
        {
            found = Some(depth);
        }
    }
    found
}

fn lift_trailing_empty_list_block(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    if scalar_from >= scalar_to
        || doc_from != doc_to
        || map.doc_to_scalar(doc_to, document) != scalar_to
    {
        return None;
    }
    let resolved = document.resolve(doc_to).ok()?;
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || resolved.parent_offset != 0
        || block.content_size() != 0
    {
        return None;
    }
    let depth = nearest_list_item_depth(document, schema, &resolved.node_path)?;
    let block_index = usize::try_from(*resolved.node_path.get(depth + 1)?).ok()?;
    if block_index == 0 {
        return None;
    }
    let list_path = resolved.node_path[..depth].to_vec();
    let list = document.node_at(&list_path)?;
    if !schema
        .node(list.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::List { .. }))
    {
        return None;
    }
    let list_content = list.content()?;
    let item_index = usize::try_from(resolved.node_path[depth]).ok()?;
    let item = list_content.child(item_index)?;
    let item_content = item.content()?;
    if block_index + 1 != item_content.child_count() {
        return None;
    }
    let prefix = item_content
        .iter()
        .take(block_index)
        .cloned()
        .collect::<Vec<_>>();
    if prefix.is_empty() {
        return None;
    }
    let lifted = item_content.child(block_index)?.clone();
    let mut before_items = list_content
        .iter()
        .take(item_index)
        .cloned()
        .collect::<Vec<_>>();
    before_items.push(Node::element(
        item.node_type().to_string(),
        item.attrs().clone(),
        Fragment::from(prefix),
    ));
    let after_items = list_content
        .iter()
        .skip(item_index + 1)
        .cloned()
        .collect::<Vec<_>>();
    let list_start = node_delete_start(document, &list_path)?;
    let mut replacement = Vec::new();
    let selection = if !before_items.is_empty() {
        let before = Node::element(
            list.node_type().to_string(),
            list.attrs().clone(),
            Fragment::from(before_items),
        );
        let cursor = list_start.checked_add(before.node_size())?.checked_add(1)?;
        replacement.push(before);
        cursor
    } else {
        list_start.checked_add(1)?
    };
    replacement.push(lifted);
    if !after_items.is_empty() {
        replacement.push(Node::element(
            list.node_type().to_string(),
            list.attrs().clone(),
            Fragment::from(after_items),
        ));
    }
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from: list_start,
            to: list_start.checked_add(list.node_size())?,
            content: Fragment::from(replacement),
        }],
        selection_after: Some(Selection::cursor(selection)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn plan_empty_blockquote_exit(
    document: &Document,
    schema: &Schema,
    position: u32,
) -> Option<SemanticCommandPlan> {
    let quote_type = schema.node_by_html_tag("blockquote")?.name.as_str();
    let resolved = document.resolve(position).ok()?;
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || resolved.parent_offset != 0
        || block.content_size() != 0
    {
        return None;
    }
    let mut node = document.root();
    let mut quote_depth = None;
    for (depth, index) in resolved.node_path.iter().copied().enumerate() {
        node = node.child(index as usize)?;
        if node.node_type() == quote_type {
            quote_depth = Some(depth);
        }
    }
    let depth = quote_depth?;
    if resolved.node_path.len() != depth + 2 {
        return None;
    }
    let quote_path = &resolved.node_path[..=depth];
    let quote = document.node_at(quote_path)?;
    let content = quote.content()?;
    let block_index = usize::try_from(*resolved.node_path.get(depth + 1)?).ok()?;
    let replace_from = node_delete_start(document, quote_path)?;
    let mut replacement = Vec::new();
    let mut cursor = replace_from;
    let before = content
        .iter()
        .take(block_index)
        .cloned()
        .collect::<Vec<_>>();
    if !before.is_empty() {
        let before_quote = Node::element(
            quote.node_type().to_string(),
            quote.attrs().clone(),
            Fragment::from(before),
        );
        cursor = cursor.checked_add(before_quote.node_size())?;
        replacement.push(before_quote);
    }
    replacement.push(default_text_block(schema)?);
    let after = content
        .iter()
        .skip(block_index + 1)
        .cloned()
        .collect::<Vec<_>>();
    if !after.is_empty() {
        replacement.push(Node::element(
            quote.node_type().to_string(),
            quote.attrs().clone(),
            Fragment::from(after),
        ));
    }
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from: replace_from,
            to: replace_from.checked_add(quote.node_size())?,
            content: Fragment::from(replacement),
        }],
        selection_after: Some(Selection::cursor(cursor.checked_add(1)?)),
        history: SemanticCommandHistory::InputBoundary,
    })
}
