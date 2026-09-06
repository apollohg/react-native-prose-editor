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
            if schema.node("bullet_list").is_some() {
                "bullet_list"
            } else {
                "bulletList"
            },
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
            if schema.node("ordered_list").is_some() {
                "ordered_list"
            } else {
                "orderedList"
            },
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

include!("editor_state/structure.rs");

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
    document_is_empty_after_omitting(document, schema, |_| false)
}

/// Whether the document remains empty after a caller omits selected direct
/// children from the root and the preferred text block.
///
/// The omission is intentionally structural: it never derives emptiness from
/// rendered text, so authored zero-width text and empty non-text containers
/// remain content. Callers use this for presentation-only visibility rules
/// without mutating the admitted document.
pub(crate) fn document_is_empty_after_omitting(
    document: &Document,
    schema: &Schema,
    mut is_omitted: impl FnMut(&Node) -> bool,
) -> bool {
    let Some(content) = document.root().content() else {
        return true;
    };
    let mut blocks = content.iter().filter(|node| !is_omitted(node));
    let Some(block) = blocks.next() else {
        return true;
    };
    if blocks.next().is_some() {
        return false;
    }
    schema
        .preferred_text_block()
        .is_some_and(|spec| spec.name == block.node_type())
        && block
            .content()
            .is_none_or(|content| content.iter().all(|node| is_omitted(node)))
}

pub(crate) fn trailing_empty_text_block_count_after_omitting(
    document: &Document,
    schema: &Schema,
    mut is_omitted: impl FnMut(&Node) -> bool,
) -> usize {
    let Some(content) = document.root().content() else {
        return 0;
    };
    let Some(preferred_text_block) = schema.preferred_text_block() else {
        return 0;
    };
    let visible_blocks = content
        .iter()
        .filter(|node| !is_omitted(node))
        .collect::<Vec<_>>();

    visible_blocks
        .into_iter()
        .rev()
        .take_while(|block| {
            block.node_type() == preferred_text_block.name
                && block
                    .content()
                    .is_none_or(|content| content.iter().all(|node| is_omitted(node)))
        })
        .count()
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

include!("editor_state/selection.rs");

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
#[path = "editor_state/tests.rs"]
mod tests;
