fn delete_previous_void_block_action(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    let (previous_path, previous) =
        previous_void_block_at_text_head(document, map, schema, scalar_from, scalar_to, doc_to)?;
    let previous_spec = schema.node(previous.node_type())?;
    if !is_backspace_deletable_void_block(previous_spec) {
        return None;
    }
    let from = node_delete_start(document, &previous_path)?;
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::DeleteRange {
            from,
            to: from.checked_add(previous.node_size())?,
        }],
        selection_after: Some(Selection::cursor(from.checked_add(1)?)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn delete_selection_through_previous_void_block_action(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    if scalar_to <= scalar_from.checked_add(1)? || map.doc_to_scalar(doc_to, document) != scalar_to
    {
        return None;
    }
    let resolved_to = document.resolve(doc_to).ok()?;
    let ending_block = resolved_to.parent(document);
    if !is_text_block(schema, ending_block) || resolved_to.parent_offset != 0 {
        return None;
    }
    let (&ending_index, parent_path) = resolved_to.node_path.split_last()?;
    if ending_index == 0 {
        return None;
    }
    let parent = document.node_at(parent_path)?;
    let previous = parent.child(usize::try_from(ending_index).ok()?.checked_sub(1)?)?;
    let previous_spec = schema.node(previous.node_type())?;
    if !previous.is_void() || !matches!(previous_spec.role, NodeRole::Block) {
        return None;
    }
    let resolved_from = document.resolve(doc_from).ok()?;
    let (&starting_index, starting_parent_path) = resolved_from.node_path.split_last()?;
    if starting_parent_path != parent_path
        || starting_index >= ending_index
        || !is_text_block(schema, resolved_from.parent(document))
    {
        return None;
    }
    let after = text::apply_operations(
        document,
        schema,
        &[SemanticOperation::DeleteRange {
            from: doc_from,
            to: doc_to,
        }],
    )
    .ok()?;
    let replacement = after
        .node_at(parent_path)?
        .child(usize::try_from(starting_index).ok()?)?
        .clone();
    let from = node_delete_start(document, &resolved_from.node_path)?;
    let mut ending_path = parent_path.to_vec();
    ending_path.push(ending_index);
    let to = node_delete_start(document, &ending_path)?.checked_add(ending_block.node_size())?;
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from,
            to,
            content: Fragment::from(vec![replacement]),
        }],
        selection_after: Some(Selection::cursor(doc_from)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn is_backspace_deletable_void_block(spec: &NodeSpec) -> bool {
    matches!(spec.role, NodeRole::Block) && spec.deletable_on_backspace.unwrap_or(true)
}

fn previous_void_block_at_text_head<'a>(
    document: &'a Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_to: u32,
) -> Option<(Vec<u32>, &'a Node)> {
    if scalar_to != scalar_from.checked_add(1)? || map.doc_to_scalar(doc_to, document) != scalar_to
    {
        return None;
    }
    let resolved = document.resolve(doc_to).ok()?;
    let block = resolved.parent(document);
    if !is_text_block(schema, block) || resolved.parent_offset != 0 {
        return None;
    }
    let (&index, parent_path) = resolved.node_path.split_last()?;
    if index == 0 {
        return None;
    }
    let previous = document
        .node_at(parent_path)?
        .child(usize::try_from(index).ok()?.checked_sub(1)?)?;
    let previous_spec = schema.node(previous.node_type())?;
    if !previous.is_void() || !matches!(previous_spec.role, NodeRole::Block) {
        return None;
    }
    let mut path = parent_path.to_vec();
    path.push(index - 1);
    Some((path, previous))
}

/// Backspace at the head of a blockquote's *first* line lifts that line out of
/// the quote, keeping its content.
///
/// [`plan_empty_blockquote_exit`] already does this for an empty quoted line,
/// where the lifted block can simply be a fresh default block. A line with text
/// needs the block itself carried out. Without this the keystroke had nowhere
/// sensible to go: a lone quote left the caret at offset 0, where
/// [`text::plan_delete_backward`] gives up and nothing happens at all, and a
/// quote under a paragraph fell through to the `DeleteRange` fallback, whose
/// endpoints straddle the quote boundary and land in different parents.
///
/// Only the first line lifts. A later line has a quoted sibling above it to
/// merge into, which [`join_with_previous_block_action`] already handles.
fn plan_blockquote_lift_at_start(
    document: &Document,
    schema: &Schema,
    position: u32,
) -> Option<SemanticCommandPlan> {
    let quote_type = schema.node_by_html_tag("blockquote")?.name.as_str();
    let resolved = document.resolve(position).ok()?;
    let block = resolved.parent(document);
    if !is_text_block(schema, block) || resolved.parent_offset != 0 {
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
    // The caret's block must be a direct child of the quote.
    if resolved.node_path.len() != depth + 2 {
        return None;
    }
    if usize::try_from(*resolved.node_path.get(depth + 1)?).ok()? != 0 {
        return None;
    }

    let quote_path = &resolved.node_path[..=depth];
    let quote = document.node_at(quote_path)?;
    let content = quote.content()?;
    let replace_from = node_delete_start(document, quote_path)?;

    let mut replacement = vec![content.child(0)?.clone()];
    let remaining = content.iter().skip(1).cloned().collect::<Vec<_>>();
    if !remaining.is_empty() {
        replacement.push(Node::element(
            quote.node_type().to_string(),
            quote.attrs().clone(),
            Fragment::from(remaining),
        ));
    }

    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from: replace_from,
            to: replace_from.checked_add(quote.node_size())?,
            content: Fragment::from(replacement),
        }],
        // Into the lifted block's content, where the caret already sat.
        selection_after: Some(Selection::cursor(replace_from.checked_add(1)?)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn empty_text_block_context(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_to: u32,
    doc_to: u32,
) -> Option<(crate::model::resolved_pos::ResolvedPos, u32)> {
    let check = |resolved: crate::model::resolved_pos::ResolvedPos, open: u32| {
        let block = resolved.parent(document);
        (schema
            .node(block.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
            && resolved.parent_offset == 0
            && block.content_size() == 0
            && map.doc_to_scalar(doc_to, document) == scalar_to)
            .then_some((resolved, open))
    };
    if doc_to < document.content_size() {
        if let Ok(candidate) = document.resolve(doc_to + 1) {
            if let Some(result) = check(candidate, doc_to) {
                return Some(result);
            }
        }
    }
    check(document.resolve(doc_to).ok()?, doc_to.checked_sub(1)?)
}

fn replace_void_and_empty_block(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    if scalar_from >= scalar_to {
        return None;
    }
    let (resolved, block_open) =
        empty_text_block_context(document, map, schema, scalar_to, doc_to)?;
    let block = resolved.parent(document);
    let &index = resolved.node_path.last()?;
    if index == 0 {
        return None;
    }
    let parent_path = &resolved.node_path[..resolved.node_path.len() - 1];
    let parent = document.node_at(parent_path)?;
    let previous = parent.child(index as usize - 1)?;
    if !(previous.is_void()
        && schema
            .node(previous.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::Block)))
    {
        return None;
    }
    let mut previous_path = parent_path.to_vec();
    previous_path.push(index - 1);
    let from = node_delete_start(document, &previous_path)?;
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from,
            to: block_open.checked_add(block.node_size())?,
            content: Fragment::from(vec![default_text_block(schema)?]),
        }],
        selection_after: Some(Selection::cursor(from.checked_add(1)?)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn replace_only_void_block(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
) -> Option<SemanticCommandPlan> {
    let root = document.root();
    let content = root.content()?;
    let block = content.child(0)?;
    let root_spec = schema.node(root.node_type())?;
    if content.child_count() != 1
        || doc_from != 0
        || doc_to != root.content_size()
        || map.doc_to_scalar(doc_from, document) != scalar_from
        || map.doc_to_scalar(doc_to, document) != scalar_to
        || !block.is_void()
        || !schema
            .node(block.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::Block))
    {
        return None;
    }
    if root_spec.content.matches::<&str, _>(&[], |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    }) {
        return None;
    }
    let replacement = default_text_block(schema)?;
    if !root_spec
        .content
        .matches(&[replacement.node_type()], |child, symbol| {
            schema.node_matches_symbol(child, symbol)
        })
    {
        return None;
    }
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from: 0,
            to: block.node_size(),
            content: Fragment::from(vec![replacement]),
        }],
        selection_after: Some(Selection::cursor(1)),
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn empty_block_delete_action(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
    is_collapsed_backward_delete: bool,
) -> Option<SemanticCommandPlan> {
    if scalar_from >= scalar_to {
        return None;
    }
    let context =
        empty_text_block_context(document, map, schema, scalar_to, doc_to).or_else(|| {
            (scalar_to == scalar_from.saturating_add(1) && doc_from < doc_to)
                .then(|| scalar_to.checked_add(1))
                .flatten()
                .and_then(|after| empty_text_block_context(document, map, schema, after, doc_to))
        })?;
    let (resolved, open) = context;
    let block = resolved.parent(document);
    let &index = resolved.node_path.last()?;
    let parent_path = &resolved.node_path[..resolved.node_path.len() - 1];
    let parent = document.node_at(parent_path)?;
    let same_doc = doc_from == doc_to;
    let boundary = scalar_to == scalar_from.saturating_add(1)
        && doc_from < doc_to
        && doc_to == open.saturating_add(1)
        && map.doc_to_scalar(doc_from, document) == scalar_from;
    if index == 0 {
        if !is_collapsed_backward_delete
            || scalar_to != scalar_from.saturating_add(1)
            || (!same_doc && !boundary)
        {
            return None;
        }
        let next = parent.child(1)?;
        let next_spec = schema.node(next.node_type())?;
        if !next.is_void() || !matches!(next_spec.role, NodeRole::Block) {
            return None;
        }
        return Some(SemanticCommandPlan {
            operations: vec![SemanticOperation::DeleteRange {
                from: open,
                to: open.checked_add(block.node_size())?,
            }],
            selection_after: Some(Selection::cursor(open)),
            history: SemanticCommandHistory::InputBoundary,
        });
    }
    let previous = parent.child(index as usize - 1)?;
    if !previous.is_element() && !previous.is_void() {
        return None;
    }
    if !same_doc && !boundary {
        return None;
    }
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::DeleteRange {
            from: open,
            to: open.checked_add(block.node_size())?,
        }],
        selection_after: if is_collapsed_backward_delete {
            Some(Selection::cursor(open.checked_sub(1)?))
        } else {
            None
        },
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn plan_empty_split_action(
    document: &Document,
    schema: &Schema,
    position: u32,
) -> Option<SemanticCommandPlan> {
    let resolved = document.resolve(position).ok()?;
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || resolved.parent_offset != 0
        || block.content_size() != 0
    {
        return plan_empty_blockquote_exit(document, schema, position);
    }
    if let Some(depth) = nearest_list_item_depth(document, schema, &resolved.node_path) {
        if *resolved.node_path.get(depth + 1)? == 0 {
            let item = document.node_at(&resolved.node_path[..=depth])?;
            if item.child_count() == 1 {
                let nested = depth >= 2
                    && document
                        .node_at(&resolved.node_path[..depth - 1])
                        .and_then(|node| schema.node(node.node_type()))
                        .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem));
                if nested {
                    let operation = SemanticOperation::OutdentListItem { pos: position };
                    let after =
                        apply_operations(document, schema, std::slice::from_ref(&operation))
                            .ok()?;
                    let selection_after =
                        outdented_list_item_position(document, &after, position, schema)?;
                    return Some(SemanticCommandPlan {
                        operations: vec![operation],
                        selection_after: Some(Selection::cursor(selection_after)),
                        history: SemanticCommandHistory::InputBoundary,
                    });
                }
                return Some(SemanticCommandPlan::one(
                    SemanticOperation::UnwrapFromList { pos: position },
                ));
            }
        }
    }
    plan_empty_blockquote_exit(document, schema, position)
}
