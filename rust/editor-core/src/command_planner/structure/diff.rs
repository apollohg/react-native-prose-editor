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
