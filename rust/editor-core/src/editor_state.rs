//! Pure editor-state queries shared by the standalone and Yrs engines.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Mark, Node};
use crate::schema::content_rule::WorkBudget;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;

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

/// Conservative, allocation-bounded command inputs for read-only state
/// queries. Mutation planners remain authoritative; this reports false when a
/// complete applicability proof is unavailable.
pub(crate) fn command_applicability(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> HashMap<String, bool> {
    let pos = selection.from(document);
    let resolved = document.resolve(pos).ok();
    let mut path = vec![document.root()];
    if let Some(resolved) = &resolved {
        let mut node = document.root();
        for index in &resolved.node_path {
            let Some(child) = node.child(*index as usize) else {
                break;
            };
            path.push(child);
            node = child;
        }
    }
    let has_text_context = !matches!(selection, Selection::All)
        && path.iter().any(|node| {
            schema
                .node(node.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        });
    let list_item_depth = path.iter().rposition(|node| {
        schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
    });
    let list_item_index = list_item_depth
        .and_then(|depth| resolved.as_ref()?.node_path.get(depth.saturating_sub(1)))
        .copied()
        .unwrap_or(0);
    let nested_list_item = list_item_depth.is_some_and(|depth| {
        path[..depth].iter().any(|node| {
            schema
                .node(node.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
        })
    });

    let mut commands = HashMap::new();
    commands.insert("indentList".into(), list_item_index > 0);
    commands.insert("outdentList".into(), nested_list_item);
    commands.insert(
        "toggleBlockquote".into(),
        has_text_context && schema.node_by_html_tag("blockquote").is_some(),
    );
    for level in 1..=6 {
        commands.insert(
            format!("toggleHeading{level}"),
            has_text_context && schema.node_by_html_tag(&format!("h{level}")).is_some(),
        );
    }
    commands.insert(
        "toggleCodeBlock".into(),
        has_text_context && schema.node("codeBlock").is_some(),
    );
    let task = path.iter().any(|node| {
        node.attrs().contains_key("checked")
            && schema
                .node(node.node_type())
                .is_some_and(|spec| spec.attrs.contains_key("checked"))
    });
    commands.insert("toggleTaskItem".into(), task);
    commands.insert(
        "wrapBulletList".into(),
        has_text_context && schema.node("bulletList").is_some(),
    );
    commands.insert(
        "wrapOrderedList".into(),
        has_text_context && schema.node("orderedList").is_some(),
    );
    commands
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
