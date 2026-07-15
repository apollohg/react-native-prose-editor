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
    let pos = selection.from(document);
    let list_context = list_item_context_at(document, schema, pos);

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
        can_toggle_blockquote(document, schema, selection, limits),
    );
    for level in 1..=6 {
        commands.insert(
            format!("toggleHeading{level}"),
            can_toggle_heading(document, schema, selection, level),
        );
    }
    commands.insert(
        "toggleCodeBlock".into(),
        can_toggle_code_block(document, schema, selection),
    );
    commands.insert(
        "toggleTaskItem".into(),
        can_toggle_task_item(document, schema, pos, limits),
    );
    commands.insert(
        "wrapBulletList".into(),
        can_apply_list_type(document, schema, selection, "bulletList", limits),
    );
    commands.insert(
        "wrapOrderedList".into(),
        can_apply_list_type(document, schema, selection, "orderedList", limits),
    );
    commands
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
    selection: &Selection,
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
    let Some(range) = selected_text_block_range(document, schema, selection) else {
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
    can_replace_selected_text_blocks(document, schema, &range, replacement_type)
}

fn can_toggle_code_block(document: &Document, schema: &Schema, selection: &Selection) -> bool {
    let Some(code_block_type) = crate::command_planner::code_block_node_name(schema) else {
        return false;
    };
    let Some(paragraph_type) = paragraph_node_name(schema) else {
        return false;
    };
    let Some(range) = selected_text_block_range(document, schema, selection) else {
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
    can_replace_selected_text_blocks(document, schema, &range, replacement_type)
}

fn can_toggle_blockquote(
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

fn can_apply_list_type(
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
    let parent = if range.parent_path.is_empty() {
        document.root()
    } else {
        let Some(parent) = document.node_at(&range.parent_path) else {
            return false;
        };
        parent
    };
    let Some(parent_spec) = schema.node(parent.node_type()) else {
        return false;
    };
    let replaced_end = range
        .first_child_index
        .saturating_add(range.selected_blocks.len());
    let child_types = parent
        .content()
        .map(|content| {
            content
                .iter()
                .enumerate()
                .map(|(index, child)| {
                    if index >= range.first_child_index && index < replaced_end {
                        target_type
                    } else {
                        child.node_type()
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    parent_spec.content.matches(&child_types, |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    })
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
