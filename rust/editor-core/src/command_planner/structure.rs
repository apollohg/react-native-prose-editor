use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Node};
use crate::position::PositionMap;
use crate::schema::content_rule::WorkBudget;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;
use crate::transform::Step;

use super::{SemanticCommandHistory, SemanticCommandPlan, SemanticOperation};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StructuralDiff {
    pub parent_path: Vec<u32>,
    pub from_child: u32,
    pub to_child: u32,
    pub content: Fragment,
}

pub(crate) struct SimulatedCommandPlan {
    pub document: Document,
    pub selection: Selection,
}

pub(crate) struct AdmittedSemanticCommandPlan {
    pub plan: SemanticCommandPlan,
    pub simulated: SimulatedCommandPlan,
}

fn default_attrs(
    schema: &Schema,
    node_type: &str,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    compatible_declared_attrs(schema, node_type, &Default::default())
}

fn compatible_declared_attrs(
    schema: &Schema,
    node_type: &str,
    current: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    let node_spec = schema.node(node_type)?;
    let mut attrs = if node_spec.allow_undeclared_attrs {
        current.clone()
    } else {
        std::collections::HashMap::new()
    };
    for (name, attr_spec) in &node_spec.attrs {
        attrs.insert(
            name.clone(),
            current
                .get(name)
                .cloned()
                .or_else(|| attr_spec.default.clone())?,
        );
    }
    Some(attrs)
}

fn node_open_pos(document: &Document, path: &[u32]) -> Option<u32> {
    let mut node = document.root();
    let mut open = 0u32;
    for &index in path {
        let content = node.content()?;
        let mut child_open = open.checked_add(1)?;
        for sibling in content.iter().take(usize::try_from(index).ok()?) {
            child_open = child_open.checked_add(sibling.node_size())?;
        }
        node = content.child(usize::try_from(index).ok()?)?;
        open = child_open;
    }
    Some(open)
}

fn node_delete_start(document: &Document, path: &[u32]) -> Option<u32> {
    node_open_pos(document, path)?.checked_sub(1)
}

fn containing_role_path(
    document: &Document,
    schema: &Schema,
    position: u32,
    predicate: impl Fn(&NodeRole) -> bool,
) -> Option<Vec<u32>> {
    let resolved = document.resolve(position).ok()?;
    let mut node = document.root();
    let mut path = Vec::new();
    let mut nearest = None;
    for index in resolved.node_path {
        node = node.child(usize::try_from(index).ok()?)?;
        path.push(index);
        if schema
            .node(node.node_type())
            .is_some_and(|spec| predicate(&spec.role))
        {
            nearest = Some(path.clone());
        }
    }
    nearest
}

fn selected_block_range(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<(Vec<u32>, u32, u32, Vec<Node>)> {
    let block_path = |position: u32| {
        let resolved = document.resolve(position).ok()?;
        let mut node = document.root();
        let mut path = Vec::new();
        let mut block_path = None;
        for index in resolved.node_path {
            node = node.child(usize::try_from(index).ok()?)?;
            path.push(index);
            if schema.node(node.node_type()).is_some_and(|spec| {
                matches!(
                    spec.role,
                    NodeRole::TextBlock | NodeRole::Block | NodeRole::List { .. }
                )
            }) {
                block_path = Some(path.clone());
            }
        }
        block_path
    };
    let from = selection.from(document);
    let to = selection.to(document);
    let start = block_path(from)?;
    let end = block_path(if to > from { to - 1 } else { from })?;
    let start_parent = &start[..start.len().checked_sub(1)?];
    let end_parent = &end[..end.len().checked_sub(1)?];
    if start_parent != end_parent {
        return None;
    }
    let parent = if start_parent.is_empty() {
        document.root()
    } else {
        document.node_at(start_parent)?
    };
    let first = usize::try_from(*start.last()?).ok()?;
    let last = usize::try_from(*end.last()?).ok()?;
    if first > last {
        return None;
    }
    let nodes = (first..=last)
        .map(|index| parent.child(index).cloned())
        .collect::<Option<Vec<_>>>()?;
    let replace_from = node_delete_start(document, &start)?;
    let replace_to =
        node_delete_start(document, &end)?.checked_add(parent.child(last)?.node_size())?;
    Some((start_parent.to_vec(), replace_from, replace_to, nodes))
}

fn admitted_plan(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    plan: SemanticCommandPlan,
    limits: &ResourceLimits,
) -> Option<AdmittedSemanticCommandPlan> {
    let simulated = simulate_plan(document, schema, selection, &plan, limits).ok()?;
    (simulated.document != *document || simulated.selection != *selection)
        .then_some(AdmittedSemanticCommandPlan { plan, simulated })
}

fn list_attrs_for_type(
    schema: &Schema,
    list_type: &str,
    current: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    compatible_declared_attrs(schema, list_type, current)
}

pub(crate) fn plan_wrap_in_list(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    list_type: &str,
    item_type: &str,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    plan_wrap_in_list_admitted(document, schema, selection, list_type, item_type, limits)
        .map(|admitted| admitted.plan)
}

pub(crate) fn plan_wrap_in_list_admitted(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    list_type: &str,
    item_type: &str,
    limits: &ResourceLimits,
) -> Option<AdmittedSemanticCommandPlan> {
    let (parent_path, replace_from, replace_to, selected) =
        selected_block_range(document, schema, selection)?;
    let in_blockquote = (!parent_path.is_empty())
        .then(|| document.node_at(&parent_path))
        .flatten()
        .and_then(|node| schema.node(node.node_type()))
        .is_some_and(|spec| spec.html_tag.as_deref() == Some("blockquote"));
    let plan = if in_blockquote {
        let items = selected
            .into_iter()
            .map(|block| {
                Some(Node::element(
                    item_type.to_string(),
                    default_attrs(schema, item_type)?,
                    Fragment::from(vec![block]),
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        SemanticCommandPlan {
            operations: vec![SemanticOperation::ReplaceRange {
                from: replace_from,
                to: replace_to,
                content: Fragment::from(vec![Node::element(
                    list_type.to_string(),
                    list_attrs_for_type(schema, list_type, &Default::default())?,
                    Fragment::from(items),
                )]),
            }],
            selection_after: None,
            history: SemanticCommandHistory::InputBoundary,
        }
    } else {
        SemanticCommandPlan {
            operations: vec![SemanticOperation::WrapInList {
                from: selection.from(document),
                to: selection.to(document),
                list_type: list_type.to_string(),
                item_type: item_type.to_string(),
                attrs: list_attrs_for_type(schema, list_type, &Default::default())?,
                item_attrs: default_attrs(schema, item_type)?,
            }],
            selection_after: None,
            history: SemanticCommandHistory::InputBoundary,
        }
    };
    admitted_plan(document, schema, selection, plan, limits)
}

pub(crate) fn plan_apply_list_type(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    list_type: &str,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    schema
        .node(list_type)
        .filter(|spec| matches!(spec.role, NodeRole::List { .. }))?;
    let position = selection.from(document);
    if let Some(path) = containing_role_path(document, schema, position, |role| {
        matches!(role, NodeRole::List { .. })
    }) {
        let list = document.node_at(&path)?;
        if list.node_type() != list_type {
            let from = node_delete_start(document, &path)?;
            let item_type = schema.list_item_type_for(list_type)?;
            let items = list
                .content()?
                .iter()
                .map(|item| {
                    if item.node_type() == item_type {
                        return Some(item.clone());
                    }
                    schema
                        .node(item.node_type())
                        .filter(|spec| matches!(spec.role, NodeRole::ListItem))?;
                    Some(Node::element(
                        item_type.clone(),
                        compatible_declared_attrs(schema, &item_type, item.attrs())?,
                        item.content().cloned().unwrap_or_else(Fragment::empty),
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            let replacement = Node::element(
                list_type.to_string(),
                list_attrs_for_type(schema, list_type, list.attrs())?,
                Fragment::from(items),
            );
            return admitted_plan(
                document,
                schema,
                selection,
                SemanticCommandPlan {
                    operations: vec![SemanticOperation::ReplaceRange {
                        from,
                        to: from.checked_add(list.node_size())?,
                        content: Fragment::from(vec![replacement]),
                    }],
                    selection_after: None,
                    history: SemanticCommandHistory::FormatBoundary,
                },
                limits,
            )
            .map(|admitted| admitted.plan);
        }
    }
    let item_type = schema.list_item_type_for(list_type)?;
    plan_wrap_in_list(document, schema, selection, list_type, &item_type, limits)
}

fn plan_list_position_operation(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    operation: SemanticOperation,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    admitted_plan(
        document,
        schema,
        selection,
        SemanticCommandPlan {
            operations: vec![operation],
            selection_after: None,
            history: SemanticCommandHistory::InputBoundary,
        },
        limits,
    )
    .map(|admitted| admitted.plan)
}

pub(crate) fn plan_unwrap_from_list(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    plan_list_position_operation(
        document,
        schema,
        selection,
        SemanticOperation::UnwrapFromList {
            pos: selection.from(document),
        },
        limits,
    )
}

pub(crate) fn plan_indent_list_item(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    plan_list_position_operation(
        document,
        schema,
        selection,
        SemanticOperation::IndentListItem {
            pos: selection.from(document),
        },
        limits,
    )
}

pub(crate) fn plan_outdent_list_item(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    let granular = plan_list_position_operation(
        document,
        schema,
        selection,
        SemanticOperation::OutdentListItem {
            pos: selection.from(document),
        },
        limits,
    )?;
    let simulated = simulate_plan(document, schema, selection, &granular, limits).ok()?;
    if simulated.document == *document {
        return None;
    }
    let inverse_is_local = structural_diff_bounded(&simulated.document, document, limits)
        .ok()
        .flatten()
        .and_then(|inverse| {
            let (from, to) = structural_diff_range(&simulated.document, &inverse, limits).ok()?;
            crate::transform::apply_step_canonical_marks(
                &simulated.document,
                &Step::ReplaceRange {
                    from,
                    to,
                    content: inverse.content,
                },
                schema,
            )
            .ok()
            .map(|(restored, _)| restored == *document)
        })
        .unwrap_or(false);
    if inverse_is_local {
        return Some(granular);
    }
    let forward = structural_diff_bounded(document, &simulated.document, limits)
        .ok()
        .flatten()?;
    let (from, to) = structural_diff_range(document, &forward, limits).ok()?;
    Some(SemanticCommandPlan {
        operations: vec![SemanticOperation::ReplaceRange {
            from,
            to,
            content: forward.content,
        }],
        selection_after: Some(simulated.selection),
        history: SemanticCommandHistory::InputBoundary,
    })
}

pub(crate) fn plan_toggle_task_item_checked(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    let path = containing_role_path(document, schema, selection.from(document), |role| {
        matches!(role, NodeRole::ListItem)
    })?;
    let item = document.node_at(&path)?;
    schema.node(item.node_type())?.attrs.get("checked")?;
    let mut attrs = item.attrs().clone();
    let checked = attrs
        .get("checked")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    attrs.insert("checked".into(), serde_json::Value::Bool(!checked));
    plan_list_position_operation(
        document,
        schema,
        selection,
        SemanticOperation::UpdateNodeAttrs {
            pos: node_delete_start(document, &path)?,
            attrs,
        },
        limits,
    )
}

fn resolve_block_insert_pos(document: &Document, schema: &Schema, position: u32) -> u32 {
    let Ok(resolved) = document.resolve(position) else {
        return position;
    };
    let parent = resolved.parent(document);
    if !schema
        .node(parent.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
    {
        return position;
    }
    node_delete_start(document, &resolved.node_path)
        .and_then(|start| start.checked_add(parent.node_size()))
        .unwrap_or(position)
}

fn resolve_block_drop_pos(document: &Document, schema: &Schema, position: u32) -> u32 {
    let Ok(resolved) = document.resolve(position) else {
        return position;
    };
    let parent = resolved.parent(document);
    if !schema
        .node(parent.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
    {
        return position;
    }
    let Some(start) = node_delete_start(document, &resolved.node_path) else {
        return position;
    };
    if resolved.parent_offset.saturating_mul(2) < parent.content_size() {
        start
    } else {
        start.checked_add(parent.node_size()).unwrap_or(position)
    }
}

fn selected_fragment(document: &Document, from: u32, to: u32) -> Option<Fragment> {
    let resolved_from = document.resolve(from).ok()?;
    let resolved_to = document.resolve(to).ok()?;
    if resolved_from.node_path != resolved_to.node_path {
        return None;
    }
    let parent = resolved_from.parent(document);
    let content = parent.content()?;
    let from_offset = resolved_from.parent_offset;
    let to_offset = resolved_to.parent_offset;
    let mut offset = 0u32;
    let mut selected = Vec::new();

    for child in content.iter() {
        let child_end = offset.checked_add(child.node_size())?;
        let overlap_from = from_offset.max(offset);
        let overlap_to = to_offset.min(child_end);
        if overlap_from < overlap_to {
            if let Some(text) = child.text_str() {
                let start = usize::try_from(overlap_from.checked_sub(offset)?).ok()?;
                let len = usize::try_from(overlap_to.checked_sub(overlap_from)?).ok()?;
                let value = text.chars().skip(start).take(len).collect::<String>();
                selected.push(Node::text(value, child.marks().to_vec()));
            } else if overlap_from == offset && overlap_to == child_end {
                selected.push(child.clone());
            } else {
                return None;
            }
        }
        offset = child_end;
    }

    (!selected.is_empty()).then(|| Fragment::from(selected))
}

pub(crate) fn plan_move_selection(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    from: u32,
    to: u32,
    destination: u32,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    let (from, to) = (from.min(to), from.max(to));
    if from >= to || (from..=to).contains(&destination) {
        return None;
    }

    let fragment = selected_fragment(document, from, to)?;
    let contains_block = fragment.iter().any(|node| {
        schema.node(node.node_type()).is_some_and(|spec| {
            matches!(
                spec.role,
                NodeRole::Block | NodeRole::TextBlock | NodeRole::List { .. }
            )
        })
    });
    let destination = if contains_block {
        resolve_block_drop_pos(document, schema, destination)
    } else {
        destination
    };
    if (from..=to).contains(&destination) {
        return None;
    }
    let mapped_destination = if destination > to {
        destination.checked_sub(to.checked_sub(from)?)?
    } else {
        destination
    };
    let cursor = mapped_destination.checked_add(fragment.size())?;
    let plan = SemanticCommandPlan {
        operations: vec![
            SemanticOperation::ReplaceRange {
                from,
                to,
                content: Fragment::empty(),
            },
            SemanticOperation::ReplaceRange {
                from: mapped_destination,
                to: mapped_destination,
                content: fragment,
            },
        ],
        selection_after: Some(Selection::cursor(cursor)),
        history: SemanticCommandHistory::InputBoundary,
    };
    admitted_plan(document, schema, selection, plan, limits).map(|admitted| admitted.plan)
}

fn empty_text_block_range(
    document: &Document,
    schema: &Schema,
    position: u32,
) -> Option<(u32, u32)> {
    let resolved = document.resolve(position).ok()?;
    let block = resolved.parent(document);
    schema
        .node(block.node_type())
        .filter(|spec| matches!(spec.role, NodeRole::TextBlock))?;
    (block.content_size() == 0).then(|| {
        let from = node_delete_start(document, &resolved.node_path)?;
        Some((from, from.checked_add(block.node_size())?))
    })?
}

pub(crate) fn plan_insert_node(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    node_type: &str,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    let spec = schema.node(node_type)?;
    if !spec.is_void {
        return None;
    }
    let attrs = default_attrs(schema, node_type)?;
    let node = Node::void(node_type.to_string(), attrs);
    let from = selection.from(document);
    let to = selection.to(document);
    let plan = match spec.role {
        NodeRole::Inline | NodeRole::HardBreak => {
            let resolved = document.resolve(from).ok()?;
            schema
                .node(resolved.parent(document).node_type())
                .filter(|parent| matches!(parent.role, NodeRole::TextBlock))?;
            let operation = if from < to {
                SemanticOperation::ReplaceRange {
                    from,
                    to,
                    content: Fragment::from(vec![node]),
                }
            } else {
                SemanticOperation::InsertNode { pos: from, node }
            };
            SemanticCommandPlan {
                operations: vec![operation],
                selection_after: Some(Selection::cursor(from.checked_add(1)?)),
                history: SemanticCommandHistory::InputBoundary,
            }
        }
        NodeRole::Block => {
            let insert = resolve_block_insert_pos(document, schema, from);
            if matches!(node_type, "horizontalRule" | "horizontal_rule") {
                let (replace_from, replace_to) =
                    empty_text_block_range(document, schema, from).unwrap_or((insert, insert));
                let text_spec = schema.preferred_text_block()?;
                let text_block = Node::element(
                    text_spec.name.clone(),
                    default_attrs(schema, &text_spec.name)?,
                    Fragment::empty(),
                );
                SemanticCommandPlan {
                    operations: vec![SemanticOperation::ReplaceRange {
                        from: replace_from,
                        to: replace_to,
                        content: Fragment::from(vec![node, text_block]),
                    }],
                    selection_after: Some(Selection::cursor(replace_from.checked_add(2)?)),
                    history: SemanticCommandHistory::InputBoundary,
                }
            } else {
                SemanticCommandPlan {
                    operations: vec![SemanticOperation::InsertNode { pos: insert, node }],
                    selection_after: Some(Selection::cursor(insert.checked_add(1)?)),
                    history: SemanticCommandHistory::InputBoundary,
                }
            }
        }
        _ => return None,
    };
    admitted_plan(document, schema, selection, plan, limits).map(|admitted| admitted.plan)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ResizeImageRequest {
    pub doc_position: u32,
    pub width: u32,
    pub height: u32,
}

fn void_node_at<'a>(
    document: &'a Document,
    position_map: &PositionMap,
    doc_pos: u32,
) -> Option<(u32, &'a Node)> {
    let block = position_map.block(position_map.find_block_for_doc_pos(doc_pos)?)?;
    if !block.is_void_block {
        return None;
    }
    Some((block.doc_start, document.node_at(&block.node_path)?))
}

pub(crate) fn plan_update_node_attrs(
    document: &Document,
    position_map: &PositionMap,
    schema: &Schema,
    selection: &Selection,
    doc_pos: u32,
    new_attrs: std::collections::HashMap<String, serde_json::Value>,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    let (resolved_pos, node) = void_node_at(document, position_map, doc_pos)?;
    if resolved_pos != doc_pos {
        return None;
    }
    let spec = schema.node(node.node_type())?;
    if !spec.is_void {
        return None;
    }
    if !spec.allow_undeclared_attrs && new_attrs.keys().any(|key| !spec.attrs.contains_key(key)) {
        return None;
    }
    let mut attrs = node.attrs().clone();
    attrs.extend(new_attrs);
    let plan = SemanticCommandPlan {
        operations: vec![SemanticOperation::UpdateNodeAttrs {
            pos: doc_pos,
            attrs,
        }],
        selection_after: None,
        history: SemanticCommandHistory::InputBoundary,
    };
    admitted_plan(document, schema, selection, plan, limits).map(|admitted| admitted.plan)
}

pub(crate) fn plan_resize_image(
    document: &Document,
    position_map: &PositionMap,
    schema: &Schema,
    selection: &Selection,
    request: ResizeImageRequest,
    limits: &ResourceLimits,
) -> Option<SemanticCommandPlan> {
    let ResizeImageRequest {
        doc_position,
        width,
        height,
    } = request;
    if width == 0 || height == 0 {
        return None;
    }
    let (doc_start, node) = void_node_at(document, position_map, doc_position)?;
    if node.node_type() != "image" {
        return None;
    }
    let mut attrs = node.attrs().clone();
    let width = serde_json::Value::Number(width.into());
    let height = serde_json::Value::Number(height.into());
    if attrs.get("width") == Some(&width) && attrs.get("height") == Some(&height) {
        return None;
    }
    attrs.insert("width".into(), width);
    attrs.insert("height".into(), height);
    admitted_plan(
        document,
        schema,
        selection,
        SemanticCommandPlan {
            operations: vec![SemanticOperation::UpdateNodeAttrs {
                pos: doc_start,
                attrs,
            }],
            selection_after: Some(Selection::node(doc_start)),
            history: SemanticCommandHistory::InputBoundary,
        },
        limits,
    )
    .map(|admitted| admitted.plan)
}

include!("structure/diff.rs");
