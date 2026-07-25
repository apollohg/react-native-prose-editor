//! Pure editor-state queries shared by the standalone and Yrs engines.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::command_planner::CommandReplacement;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::content_rule::WorkBudget;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;
use crate::transform::{Source, Step, Transaction};

/// Which marks and node types are active at the current selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveState {
    pub marks: HashMap<String, bool>,
    pub mark_attrs: HashMap<String, serde_json::Value>,
    pub nodes: HashMap<String, bool>,
    pub commands: HashMap<String, bool>,
    pub allowed_marks: Vec<String>,
    pub insertable_nodes: Vec<String>,
}

/// Whether undo/redo are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
}

/// Assemble active state from document semantics and preflighted command
/// applicability. Command planners stay with their owning engine while all
/// document/selection queries are shared here.
pub(crate) fn active_state(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
    commands: HashMap<String, bool>,
    limits: &ResourceLimits,
) -> ActiveState {
    crate::yrs_engine::record_active_state_full_assembly();
    active_state_impl(document, schema, selection, stored_marks, commands, limits)
}

fn active_state_impl(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
    commands: HashMap<String, bool>,
    limits: &ResourceLimits,
) -> ActiveState {
    let pos = selection.from(document);
    let marks_at = effective_marks_for_selection(document, selection, stored_marks);
    let nodes_at = nodes_at_position(document, pos);

    let mut marks = HashMap::new();
    let mut mark_attrs = HashMap::new();
    for mark_spec in schema.all_marks() {
        let active_mark = marks_at
            .iter()
            .find(|mark| mark.mark_type() == mark_spec.name);
        marks.insert(mark_spec.name.clone(), active_mark.is_some());
        if let Some(mark) = active_mark.filter(|mark| !mark.attrs().is_empty()) {
            mark_attrs.insert(
                mark_spec.name.clone(),
                serde_json::Value::Object(mark.attrs().clone().into_iter().collect()),
            );
        }
    }

    let active_list_type =
        containing_list_node_at(document, schema, pos).map(|node| node.node_type().to_string());
    let mut nodes = HashMap::new();
    for node_name in nodes_at {
        if schema.is_list(&node_name) {
            if active_list_type.as_deref() == Some(node_name.as_str()) {
                nodes.insert(node_name, true);
            }
        } else {
            nodes.insert(node_name, true);
        }
    }

    let (allowed_marks, insertable_nodes) = match selection {
        Selection::All => (Vec::new(), Vec::new()),
        Selection::Node { .. } => (
            Vec::new(),
            insertable_nodes(document, schema, pos, limits).unwrap_or_default(),
        ),
        Selection::Text { .. } => {
            let active_names = marks_at
                .iter()
                .map(|mark| mark.mark_type())
                .collect::<Vec<_>>();
            let allowed = document
                .resolve(pos)
                .ok()
                .and_then(|resolved| schema.node(resolved.parent(document).node_type()))
                .map(|spec| schema.allowed_marks_at(spec, &active_names))
                .unwrap_or_default();
            (
                allowed,
                insertable_nodes(document, schema, pos, limits).unwrap_or_default(),
            )
        }
    };

    ActiveState {
        marks,
        mark_attrs,
        nodes,
        commands,
        allowed_marks,
        insertable_nodes,
    }
}

/// Exact, allocation-bounded command preflights shared by both engines.
/// Transaction-backed checks run only against derived documents and report
/// false whenever a complete applicability proof is unavailable.
pub(crate) fn command_applicability(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> HashMap<String, bool> {
    command_applicability_with_known_node_count(
        document,
        schema,
        selection,
        limits,
        document_node_count(document.root()),
    )
}

pub(crate) fn command_applicability_with_known_node_count(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
    document_node_count: usize,
) -> HashMap<String, bool> {
    #[cfg(test)]
    crate::yrs_engine::observability::record_active_applicability_pass();
    command_applicability_with_known_node_count_impl(
        document,
        schema,
        selection,
        limits,
        document_node_count,
    )
}

fn command_applicability_with_known_node_count_impl(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
    document_node_count: usize,
) -> HashMap<String, bool> {
    let pos = selection.from(document);
    let list_context = list_item_context_at(document, schema, pos);
    let block_range = selected_block_range(
        document,
        schema,
        selection.from(document),
        selection.to(document),
    );
    let root_wrap_range = root_wrap_range(document, schema, selection);
    let mut commands = HashMap::new();
    commands.insert(
        "indentList".into(),
        list_context
            .as_ref()
            .is_some_and(|context| context.list_item_index > 0),
    );
    commands.insert(
        "outdentList".into(),
        list_context
            .as_ref()
            .is_some_and(|context| context.parent_is_list_item),
    );
    commands.insert(
        "toggleBlockquote".into(),
        can_toggle_blockquote_local(
            document,
            schema,
            selection,
            limits,
            document_node_count,
            block_range.as_ref(),
        ),
    );
    for level in 1..=6 {
        commands.insert(
            format!("toggleHeading{level}"),
            can_toggle_heading(document, schema, block_range.as_ref(), level),
        );
    }
    commands.insert(
        "toggleCodeBlock".into(),
        can_toggle_code_block(document, schema, block_range.as_ref()),
    );
    commands.insert(
        "toggleTaskItem".into(),
        can_toggle_task_item(document, schema, pos, limits),
    );
    commands.insert(
        "wrapBulletList".into(),
        can_apply_list_type_local(
            document,
            schema,
            selection,
            "bulletList",
            limits,
            document_node_count,
            block_range.as_ref(),
            root_wrap_range.as_ref(),
        ),
    );
    commands.insert(
        "wrapOrderedList".into(),
        can_apply_list_type_local(
            document,
            schema,
            selection,
            "orderedList",
            limits,
            document_node_count,
            block_range.as_ref(),
            root_wrap_range.as_ref(),
        ),
    );
    commands
}

#[cfg(test)]
pub(crate) fn active_state_for_debug_invariant(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
    limits: &ResourceLimits,
    document_node_count: usize,
) -> ActiveState {
    let commands = command_applicability_with_known_node_count_impl(
        document,
        schema,
        selection,
        limits,
        document_node_count,
    );
    active_state_impl(document, schema, selection, stored_marks, commands, limits)
}

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

/// Whether the document holds nothing the user has authored.
///
/// True only for the single empty default text block a fresh editor starts
/// with. Anything else — text, a second block, or a block that carries
/// structure such as a list item, a blockquote, or a heading — is content, even
/// when it renders no characters at all.
///
/// This is the authority for a host's empty-state affordances (the placeholder
/// above all). Hosts must not re-derive it from rendered text: an empty bullet
/// contributes no characters, so any character-based test reports it as empty
/// and hides a structure the user can plainly see.
pub(crate) fn document_is_empty(document: &Document, schema: &Schema) -> bool {
    let Some(content) = document.root().content() else {
        return true;
    };
    let mut blocks = content.iter();
    let Some(block) = blocks.next() else {
        return true;
    };
    if blocks.next().is_some() {
        return false;
    }
    if block.content_size() != 0 {
        return false;
    }
    schema
        .preferred_text_block()
        .is_some_and(|spec| spec.name == block.node_type())
}

pub(crate) fn document_node_count(node: &Node) -> usize {
    node.content().map_or(1, |content| {
        content
            .iter()
            .map(document_node_count)
            .fold(1usize, usize::saturating_add)
    })
}

fn node_relative_depth(node: &Node) -> usize {
    node.content().map_or(1, |content| {
        content
            .iter()
            .map(node_relative_depth)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    })
}

fn containing_node_path_at<'a>(
    document: &'a Document,
    schema: &Schema,
    pos: u32,
    matches_node: impl Fn(&NodeRole, &str) -> bool,
) -> Option<(Vec<u32>, &'a Node)> {
    let resolved = document.resolve(pos).ok()?;
    let mut node = document.root();
    let mut path = Vec::new();
    let mut nearest = None;
    for index in resolved.node_path {
        node = node.child(index as usize)?;
        path.push(index);
        let spec = schema.node(node.node_type())?;
        if matches_node(&spec.role, node.node_type()) {
            nearest = Some((path.clone(), node));
        }
    }
    nearest
}

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

pub(crate) fn paragraph_node_name(schema: &Schema) -> Option<&str> {
    schema
        .node_by_html_tag("p")
        .or_else(|| schema.node("paragraph"))
        .map(|spec| spec.name.as_str())
}

fn list_attrs_for_type(
    list_type: &str,
    current_attrs: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    if matches!(list_type, "orderedList" | "ordered_list") {
        current_attrs
            .get("start")
            .map(|start| HashMap::from([("start".into(), start.clone())]))
            .unwrap_or_default()
    } else {
        HashMap::new()
    }
}

pub(crate) fn marks_at_position(document: &Document, position: u32) -> Vec<Mark> {
    let Ok(resolved) = document.resolve(position) else {
        return Vec::new();
    };
    let Some(content) = resolved.parent(document).content() else {
        return Vec::new();
    };
    let mut offset = 0u32;
    for child in content.iter() {
        let child_size = child.node_size();
        if child.is_text()
            && offset <= resolved.parent_offset
            && resolved.parent_offset <= offset.saturating_add(child_size)
        {
            return child.marks().to_vec();
        }
        offset = offset.saturating_add(child_size);
    }
    if resolved.parent_offset == offset {
        return content
            .iter()
            .rev()
            .find(|child| child.is_text())
            .map_or_else(Vec::new, |child| child.marks().to_vec());
    }
    Vec::new()
}

pub(crate) fn range_has_mark(document: &Document, from: u32, to: u32, mark_name: &str) -> bool {
    let Ok(resolved) = document.resolve(from) else {
        return false;
    };
    let Some(content) = resolved.parent(document).content() else {
        return false;
    };
    let from_offset = resolved.parent_offset;
    let to_offset = from_offset.saturating_add(to.saturating_sub(from));
    let mut offset = 0u32;
    let mut found = false;
    for child in content.iter() {
        let end = offset.saturating_add(child.node_size());
        if child.is_text() && end > from_offset && offset < to_offset {
            found = true;
            if !child
                .marks()
                .iter()
                .any(|mark| mark.mark_type() == mark_name)
            {
                return false;
            }
        }
        offset = end;
    }
    found
}

pub(crate) fn mark_range_at_position(
    document: &Document,
    position: u32,
    mark_name: &str,
) -> Option<(u32, u32)> {
    let resolved = document.resolve(position).ok()?;
    let content = resolved.parent(document).content()?;
    let parent_content_start = content_start_for_path(document, &resolved.node_path)?;
    let mut offset = 0u32;
    let mut target = None;
    for (index, child) in content.iter().enumerate() {
        let end = offset.checked_add(child.node_size())?;
        if child.is_text()
            && child
                .marks()
                .iter()
                .any(|mark| mark.mark_type() == mark_name)
            && offset <= resolved.parent_offset
            && resolved.parent_offset <= end
        {
            target = Some((index, offset, end));
            break;
        }
        offset = end;
    }
    let (index, mut start, mut end) = target?;
    let target_mark = content
        .child(index)?
        .marks()
        .iter()
        .find(|mark| mark.mark_type() == mark_name)?;
    let mut left = index;
    while left > 0 {
        let sibling = content.child(left - 1)?;
        if !sibling.is_text() || !sibling.marks().iter().any(|mark| mark == target_mark) {
            break;
        }
        start = start.checked_sub(sibling.node_size())?;
        left -= 1;
    }
    let mut right = index;
    while right + 1 < content.child_count() {
        let sibling = content.child(right + 1)?;
        if !sibling.is_text() || !sibling.marks().iter().any(|mark| mark == target_mark) {
            break;
        }
        end = end.checked_add(sibling.node_size())?;
        right += 1;
    }
    Some((
        parent_content_start.checked_add(start)?,
        parent_content_start.checked_add(end)?,
    ))
}

fn content_start_for_path(document: &Document, path: &[u32]) -> Option<u32> {
    let mut start = 0u32;
    let mut node = document.root();
    for index in path.iter().copied() {
        let content = node.content()?;
        for sibling in content.iter().take(index as usize) {
            start = start.checked_add(sibling.node_size())?;
        }
        start = start.checked_add(1)?;
        node = content.child(index as usize)?;
    }
    Some(start)
}

pub(crate) fn nodes_at_position(document: &Document, position: u32) -> Vec<String> {
    let Ok(resolved) = document.resolve(position) else {
        return Vec::new();
    };
    let mut result = vec![document.root().node_type().to_string()];
    let mut node = document.root();
    for index in resolved.node_path {
        let Some(child) = node.child(index as usize) else {
            break;
        };
        result.push(child.node_type().to_string());
        node = child;
    }
    result
}

fn effective_marks_for_selection(
    document: &Document,
    selection: &Selection,
    stored_marks: Option<&[Mark]>,
) -> Vec<Mark> {
    let (anchor, head) = match selection {
        Selection::Text { anchor, head } => (anchor, head),
        Selection::Node { pos } => (pos, pos),
        Selection::All => return Vec::new(),
    };
    if anchor == head {
        return stored_marks
            .map(<[Mark]>::to_vec)
            .unwrap_or_else(|| marks_at_position(document, *anchor));
    }
    let from = (*anchor).min(*head);
    let to = (*anchor).max(*head);
    let mut overlapping = Vec::new();
    collect_text_marks(document.root(), 0, from, to, &mut overlapping, true);
    let Some(mut common) = overlapping.first().cloned() else {
        return marks_at_position(document, from);
    };
    for marks in overlapping.iter().skip(1) {
        common.retain(|mark| marks.contains(mark));
    }
    common
}

fn collect_text_marks(
    node: &Node,
    start: u32,
    from: u32,
    to: u32,
    out: &mut Vec<Vec<Mark>>,
    is_root: bool,
) {
    if from >= to {
        return;
    }
    if node.is_text() {
        if start < to && start.saturating_add(node.node_size()) > from {
            out.push(node.marks().to_vec());
        }
        return;
    }
    let Some(content) = node.content() else {
        return;
    };
    let mut child_start = if is_root {
        start
    } else {
        start.saturating_add(1)
    };
    for child in content.iter() {
        let child_end = child_start.saturating_add(child.node_size());
        if child_end > from && child_start < to {
            collect_text_marks(child, child_start, from, to, out, false);
        }
        child_start = child_end;
    }
}

fn insertable_nodes(
    document: &Document,
    schema: &Schema,
    position: u32,
    limits: &ResourceLimits,
) -> Result<Vec<String>, ()> {
    let resolved = document.resolve(position).map_err(|_| ())?;
    let (parent, prefix, suffix) =
        block_parent_and_siblings(document, schema, &resolved).ok_or(())?;
    let spec = schema.node(parent.node_type()).ok_or(())?;
    let work = limits.max_document_nodes.saturating_mul(128);
    let budget = WorkBudget::new(work);
    let insertable = schema.insertable_nodes_at_with_budget(spec, &prefix, &suffix, &budget)?;
    let mut filtered = insertable
        .into_iter()
        .filter(|node_type| {
            schema
                .node(node_type)
                .is_some_and(|spec| matches!(spec.role, NodeRole::Block) && spec.is_void)
        })
        .collect::<Vec<_>>();
    if matches!(parent.node_type(), "listItem" | "list_item") {
        filtered.retain(|node_type| {
            !matches!(node_type.as_str(), "horizontalRule" | "horizontal_rule")
        });
    }
    let inline_parent = resolved.parent(document);
    if schema
        .node(inline_parent.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
    {
        for node_type in schema
            .all_nodes()
            .filter(|spec| {
                matches!(spec.role, NodeRole::HardBreak | NodeRole::Inline) && spec.is_void
            })
            .map(|spec| spec.name.clone())
        {
            if !filtered.contains(&node_type) {
                filtered.push(node_type);
            }
        }
    }
    Ok(filtered)
}

fn block_parent_and_siblings<'a>(
    document: &'a Document,
    schema: &Schema,
    resolved: &crate::model::resolved_pos::ResolvedPos,
) -> Option<(&'a Node, Vec<&'a str>, Vec<&'a str>)> {
    let mut path_nodes = vec![document.root()];
    let mut current = document.root();
    for index in &resolved.node_path {
        current = current.child(*index as usize)?;
        path_nodes.push(current);
    }
    for depth in (0..path_nodes.len()).rev() {
        let node = path_nodes[depth];
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        {
            continue;
        }
        let insertion_index = if let Some(index) = resolved.node_path.get(depth) {
            usize::try_from(*index).ok()?.saturating_add(1)
        } else {
            let mut consumed = 0u32;
            node.content()?
                .iter()
                .take_while(|child| {
                    let before =
                        consumed.saturating_add(child.node_size()) <= resolved.parent_offset;
                    if before {
                        consumed = consumed.saturating_add(child.node_size());
                    }
                    before
                })
                .count()
        };
        let content = node.content()?;
        let prefix = content
            .iter()
            .take(insertion_index)
            .map(Node::node_type)
            .collect();
        let suffix = content
            .iter()
            .skip(insertion_index)
            .map(Node::node_type)
            .collect();
        return Some((node, prefix, suffix));
    }
    None
}

fn containing_list_node_at<'a>(
    document: &'a Document,
    schema: &Schema,
    position: u32,
) -> Option<&'a Node> {
    let resolved = document.resolve(position).ok()?;
    let mut node = document.root();
    let mut nearest = None;
    for index in resolved.node_path {
        node = node.child(index as usize)?;
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::List { .. }))
        {
            nearest = Some(node);
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Fragment;

    fn paragraph(text: &str) -> Node {
        Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::text(text.into(), Vec::new())]),
        )
    }

    fn element(node_type: &str, children: Vec<Node>) -> Node {
        Node::element(node_type.into(), HashMap::new(), Fragment::from(children))
    }

    fn document(children: Vec<Node>) -> Document {
        Document::new(element("doc", children))
    }

    fn assert_structural_preflights_match_oracle(
        document: &Document,
        schema: &Schema,
        selection: &Selection,
        limits: &ResourceLimits,
    ) {
        let nodes = document_node_count(document.root());
        let block_range = selected_block_range(
            document,
            schema,
            selection.from(document),
            selection.to(document),
        );
        let root_range = root_wrap_range(document, schema, selection);
        assert_eq!(
            can_toggle_blockquote_local(
                document,
                schema,
                selection,
                limits,
                nodes,
                block_range.as_ref(),
            ),
            can_toggle_blockquote_transaction_oracle(document, schema, selection, limits),
            "blockquote mismatch for {selection:?}"
        );
        for list_type in ["bulletList", "orderedList"] {
            assert_eq!(
                can_apply_list_type_local(
                    document,
                    schema,
                    selection,
                    list_type,
                    limits,
                    nodes,
                    block_range.as_ref(),
                    root_range.as_ref(),
                ),
                can_apply_list_type_transaction_oracle(
                    document, schema, selection, list_type, limits,
                ),
                "{list_type} mismatch for {selection:?}"
            );
        }
    }

    #[test]
    fn structural_command_local_proofs_match_transaction_oracle() {
        let schema = crate::tiptap_schema();
        let documents = vec![
            document(vec![paragraph("one"), paragraph("two"), paragraph("three")]),
            document(vec![element("blockquote", vec![paragraph("quote")])]),
            document(vec![element(
                "bulletList",
                vec![
                    element("listItem", vec![paragraph("one")]),
                    element("listItem", vec![paragraph("two")]),
                    element("listItem", vec![paragraph("three")]),
                ],
            )]),
            document(vec![element(
                "orderedList",
                vec![element(
                    "listItem",
                    vec![
                        paragraph("outer"),
                        element(
                            "bulletList",
                            vec![element("listItem", vec![paragraph("inner")])],
                        ),
                    ],
                )],
            )]),
        ];
        for document in &documents {
            let size = document.content_size();
            let positions = [0, 1, size / 2, size.saturating_sub(1), size];
            for position in positions {
                for selection in [
                    Selection::cursor(position),
                    Selection::text(1.min(size), position),
                    Selection::text(position, 1.min(size)),
                    Selection::node(position),
                    Selection::All,
                ] {
                    assert_structural_preflights_match_oracle(
                        document,
                        &schema,
                        &selection,
                        &ResourceLimits::default(),
                    );
                }
            }
            let nodes = document_node_count(document.root());
            let depth = node_relative_depth(document.root());
            for (nodes, depth) in [
                (nodes, depth),
                (nodes.saturating_add(1), depth),
                (nodes.saturating_add(8), depth.saturating_add(2)),
            ] {
                let limits = ResourceLimits {
                    max_document_nodes: nodes,
                    max_document_depth: depth,
                    ..ResourceLimits::default()
                };
                assert_structural_preflights_match_oracle(
                    document,
                    &schema,
                    &Selection::cursor(1.min(size)),
                    &limits,
                );
            }
        }
    }

    #[test]
    fn structural_command_local_proofs_match_custom_schema_and_invalid_positions() {
        use crate::schema::AttrSpec;

        let base = crate::tiptap_schema();
        let mut nodes = base.all_nodes().cloned().collect::<Vec<_>>();
        for spec in &mut nodes {
            if spec.html_tag.as_deref() == Some("blockquote") || spec.name == "bulletList" {
                spec.attrs.insert(
                    "requiredProofAttr".into(),
                    AttrSpec {
                        default: None,
                        has_default: false,
                    },
                );
            }
        }
        let schema = Schema::new(nodes, base.all_marks().cloned().collect());
        let document = document(vec![paragraph("one"), paragraph("two")]);
        for selection in [
            Selection::cursor(1),
            Selection::text(1, document.content_size().saturating_sub(1)),
            Selection::text(document.content_size().saturating_sub(1), 1),
            Selection::cursor(document.content_size().saturating_add(1)),
            Selection::cursor(u32::MAX),
        ] {
            assert_structural_preflights_match_oracle(
                &document,
                &schema,
                &selection,
                &ResourceLimits::default(),
            );
        }
    }

    #[test]
    fn structural_command_local_proofs_match_generated_block_ranges() {
        let schema = crate::tiptap_schema();
        for block_count in 1..=8 {
            let document = document(
                (0..block_count)
                    .map(|index| paragraph(&format!("block-{index}")))
                    .collect(),
            );
            let size = document.content_size();
            let positions = (0..=size)
                .filter(|position| position % 3 == 0 || *position == 1 || *position == size)
                .collect::<Vec<_>>();
            for &anchor in &positions {
                for &head in &positions {
                    assert_structural_preflights_match_oracle(
                        &document,
                        &schema,
                        &Selection::text(anchor, head),
                        &ResourceLimits::default(),
                    );
                }
            }
        }
    }

    #[test]
    fn known_node_count_entry_point_matches_standalone_wrapper() {
        let schema = crate::tiptap_schema();
        let document = document(vec![paragraph("one"), paragraph("two")]);
        let selection = Selection::cursor(1);
        assert_eq!(
            command_applicability_with_known_node_count(
                &document,
                &schema,
                &selection,
                &ResourceLimits::default(),
                document_node_count(document.root()),
            ),
            command_applicability(&document, &schema, &selection, &ResourceLimits::default(),)
        );
    }

    #[test]
    fn node_selection_preserves_collapsed_stored_marks() {
        let schema = crate::tiptap_schema();
        let bold = Mark::new("bold".into(), HashMap::new());
        let document = Document::new(Node::element(
            "doc".into(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::text("x".into(), vec![bold.clone()])]),
            )]),
        ));
        let state = active_state(
            &document,
            &schema,
            &Selection::node(1),
            Some(std::slice::from_ref(&bold)),
            HashMap::new(),
            &ResourceLimits::default(),
        );
        assert_eq!(state.marks.get("bold"), Some(&true));
    }
}
