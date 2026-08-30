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

pub(crate) fn simulate_plan(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    plan: &SemanticCommandPlan,
    limits: &ResourceLimits,
) -> Result<SimulatedCommandPlan, ()> {
    #[cfg(test)]
    crate::yrs_engine::observability::record_planner_simulation();
    if plan.operations.len() > limits.max_document_nodes {
        return Err(());
    }
    let work = WorkBudget::new(
        limits
            .max_document_nodes
            .saturating_mul(plan.operations.len().saturating_add(1)),
    );
    let mut preview = document.clone();
    let mut mapped = selection.clone();
    for operation in &plan.operations {
        if !work.consume_n(limits.max_document_nodes) {
            return Err(());
        }
        let (next, step_map) =
            crate::transform::apply_step_canonical_marks(&preview, &operation.as_step(), schema)
                .map_err(|_| ())?;
        mapped = mapped.map(&step_map);
        preview = next;
    }
    Ok(SimulatedCommandPlan {
        document: preview,
        selection: plan.selection_after.clone().unwrap_or(mapped),
    })
}

pub(crate) fn structural_diff_bounded(
    before: &Document,
    after: &Document,
    limits: &ResourceLimits,
) -> Result<Option<StructuralDiff>, ()> {
    let budget = WorkBudget::new(limits.max_document_nodes.saturating_mul(4));
    structural_diff_nodes(
        before.root(),
        after.root(),
        &mut Vec::new(),
        Some((&budget, limits.max_document_depth)),
        0,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};

    fn atom_schema() -> Schema {
        Schema::from_json(&serde_json::json!({
            "nodes": [
                { "name": "doc", "content": "block+", "role": "doc" },
                { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" },
                {
                    "name": "counterCard",
                    "content": "",
                    "group": "block",
                    "role": "block",
                    "isVoid": true,
                    "attrs": {
                        "title": { "default": "" },
                        "count": { "default": 0 }
                    }
                }
            ],
            "marks": []
        }))
        .unwrap()
    }

    fn atom_document(schema: &Schema) -> Document {
        from_prosemirror_json(
            &serde_json::json!({
                "type": "doc",
                "content": [
                    { "type": "counterCard", "attrs": { "title": "a", "count": 1 } },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }
                ]
            }),
            schema,
            UnknownTypeMode::Error,
        )
        .unwrap()
    }

    #[test]
    fn plan_update_node_attrs_rewrites_declared_attrs() {
        let schema = atom_schema();
        let document = atom_document(&schema);
        let position_map = PositionMap::build(&document, &schema);
        let doc_pos = position_map.block(0).unwrap().doc_start;
        let selection = Selection::cursor(doc_pos);

        let plan = plan_update_node_attrs(
            &document,
            &position_map,
            &schema,
            &selection,
            doc_pos,
            HashMap::from([("title".into(), serde_json::json!("b"))]),
            &ResourceLimits::default(),
        )
        .unwrap();

        assert_eq!(
            plan.operations,
            vec![SemanticOperation::UpdateNodeAttrs {
                pos: doc_pos,
                attrs: HashMap::from([
                    ("title".into(), serde_json::json!("b")),
                    ("count".into(), serde_json::json!(1)),
                ]),
            }]
        );
        assert_eq!(plan.selection_after, None);
    }

    #[test]
    fn plan_update_node_attrs_rejects_undeclared_attr_without_escape_hatch() {
        let schema = atom_schema();
        let document = atom_document(&schema);
        let position_map = PositionMap::build(&document, &schema);
        let doc_pos = position_map.block(0).unwrap().doc_start;

        assert!(plan_update_node_attrs(
            &document,
            &position_map,
            &schema,
            &Selection::cursor(doc_pos),
            doc_pos,
            HashMap::from([("bogus".into(), serde_json::json!(1))]),
            &ResourceLimits::default(),
        )
        .is_none());
    }

    #[test]
    fn plan_update_node_attrs_rejects_non_void_target() {
        let schema = atom_schema();
        let document = atom_document(&schema);
        let position_map = PositionMap::build(&document, &schema);
        let doc_pos = position_map.block(1).unwrap().doc_start;

        assert!(plan_update_node_attrs(
            &document,
            &position_map,
            &schema,
            &Selection::cursor(doc_pos),
            doc_pos,
            HashMap::new(),
            &ResourceLimits::default(),
        )
        .is_none());
    }
}

pub(crate) fn prove_structural_diff(
    before: &Document,
    after: &Document,
    diff: &StructuralDiff,
    schema: &Schema,
    limits: &ResourceLimits,
) -> Result<bool, ()> {
    let (from, to) = structural_diff_range(before, diff, limits)?;
    let step = Step::ReplaceRange {
        from,
        to,
        content: diff.content.clone(),
    };
    let (candidate, _) =
        crate::transform::apply_step_canonical_marks(before, &step, schema).map_err(|_| ())?;
    let budget = WorkBudget::new(limits.max_document_nodes.saturating_mul(2));
    nodes_equal_bounded(
        candidate.root(),
        after.root(),
        Some((&budget, limits.max_document_depth)),
        0,
    )
}

fn structural_diff_range(
    document: &Document,
    diff: &StructuralDiff,
    limits: &ResourceLimits,
) -> Result<(u32, u32), ()> {
    if diff.parent_path.len() > limits.max_document_depth {
        return Err(());
    }
    let budget = WorkBudget::new(limits.max_document_nodes);
    let mut node = document.root();
    let mut start = 0u32;
    for child_index in &diff.parent_path {
        if !budget.consume() {
            return Err(());
        }
        let content = node.content().ok_or(())?;
        let index = usize::try_from(*child_index).map_err(|_| ())?;
        for sibling in content.iter().take(index) {
            if !budget.consume() {
                return Err(());
            }
            start = start.checked_add(sibling.node_size()).ok_or(())?;
        }
        start = start.checked_add(1).ok_or(())?;
        node = content.child(index).ok_or(())?;
    }
    let content = node.content().ok_or(())?;
    let from_child = usize::try_from(diff.from_child).map_err(|_| ())?;
    let to_child = usize::try_from(diff.to_child).map_err(|_| ())?;
    if from_child > to_child || to_child > content.child_count() {
        return Err(());
    }
    let mut from = start;
    for child in content.iter().take(from_child) {
        if !budget.consume() {
            return Err(());
        }
        from = from.checked_add(child.node_size()).ok_or(())?;
    }
    let mut to = from;
    for child in content.iter().skip(from_child).take(to_child - from_child) {
        if !budget.consume() {
            return Err(());
        }
        to = to.checked_add(child.node_size()).ok_or(())?;
    }
    Ok((from, to))
}

fn structural_diff_nodes(
    before: &Node,
    after: &Node,
    path: &mut Vec<u32>,
    bound: Option<(&WorkBudget, usize)>,
    depth: usize,
) -> Result<Option<StructuralDiff>, ()> {
    if let Some((budget, max_depth)) = bound {
        if depth > max_depth || !budget.consume() {
            return Err(());
        }
    }
    if before.node_type() != after.node_type() || before.attrs() != after.attrs() {
        return Ok(None);
    }
    let (Some(before_content), Some(after_content)) = (before.content(), after.content()) else {
        return Ok(None);
    };
    let mut prefix = 0usize;
    while prefix
        < before_content
            .child_count()
            .min(after_content.child_count())
    {
        if !nodes_equal_bounded(
            before_content.child(prefix).ok_or(())?,
            after_content.child(prefix).ok_or(())?,
            bound,
            depth.saturating_add(1),
        )? {
            break;
        }
        prefix += 1;
    }
    let suffix_limit = before_content
        .child_count()
        .saturating_sub(prefix)
        .min(after_content.child_count().saturating_sub(prefix));
    let mut suffix = 0usize;
    while suffix < suffix_limit {
        let left = before_content
            .child(before_content.child_count() - 1 - suffix)
            .ok_or(())?;
        let right = after_content
            .child(after_content.child_count() - 1 - suffix)
            .ok_or(())?;
        if !nodes_equal_bounded(left, right, bound, depth.saturating_add(1))? {
            break;
        }
        suffix += 1;
    }
    let before_end = before_content.child_count().checked_sub(suffix).ok_or(())?;
    let after_end = after_content.child_count().checked_sub(suffix).ok_or(())?;
    if prefix == before_end && prefix == after_end {
        return Ok(None);
    }
    if before_end == prefix + 1 && after_end == prefix + 1 {
        let left = before_content.child(prefix).ok_or(())?;
        let right = after_content.child(prefix).ok_or(())?;
        if left.is_element()
            && right.is_element()
            && left.node_type() == right.node_type()
            && left.attrs() == right.attrs()
        {
            path.push(u32::try_from(prefix).map_err(|_| ())?);
            if let Some(diff) =
                structural_diff_nodes(left, right, path, bound, depth.saturating_add(1))?
            {
                path.pop();
                return Ok(Some(diff));
            }
            path.pop();
        }
    }
    Ok(Some(StructuralDiff {
        parent_path: path.clone(),
        from_child: u32::try_from(prefix).map_err(|_| ())?,
        to_child: u32::try_from(before_end).map_err(|_| ())?,
        content: Fragment::from(
            after_content
                .iter()
                .skip(prefix)
                .take(after_end - prefix)
                .cloned()
                .collect::<Vec<_>>(),
        ),
    }))
}

fn nodes_equal_bounded(
    left: &Node,
    right: &Node,
    bound: Option<(&WorkBudget, usize)>,
    depth: usize,
) -> Result<bool, ()> {
    if let Some((budget, max_depth)) = bound {
        if depth > max_depth || !budget.consume() {
            return Err(());
        }
    }
    if left.node_type() != right.node_type()
        || left.attrs() != right.attrs()
        || left.marks() != right.marks()
        || left.text_str() != right.text_str()
    {
        return Ok(false);
    }
    match (left.content(), right.content()) {
        (None, None) => Ok(true),
        (Some(left), Some(right)) if left.child_count() == right.child_count() => {
            for index in 0..left.child_count() {
                if !nodes_equal_bounded(
                    left.child(index).ok_or(())?,
                    right.child(index).ok_or(())?,
                    bound,
                    depth.saturating_add(1),
                )? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
