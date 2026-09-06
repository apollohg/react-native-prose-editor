//! Pure, backend-neutral command planning shared by both editor engines.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::schema::content_rule::WorkBudget;
use crate::schema::{NodeRole, NodeSpec, Schema};
use crate::selection::Selection;
use crate::transform::Step;

mod format;
mod structure;
mod text;
pub(crate) use format::{
    code_block_node_name, plan_set_mark, plan_toggle_blockquote, plan_toggle_code_block,
    plan_toggle_heading, plan_toggle_mark, plan_unset_mark, CommandReplacement, MarkCommandPlan,
};
pub(crate) use structure::{
    plan_apply_list_type, plan_indent_list_item, plan_insert_node, plan_move_selection,
    plan_outdent_list_item, plan_resize_image, plan_toggle_task_item_checked,
    plan_unwrap_from_list, plan_update_node_attrs, plan_wrap_in_list_admitted,
    prove_structural_diff, simulate_plan, structural_diff_bounded, AdmittedSemanticCommandPlan,
    ResizeImageRequest, SimulatedCommandPlan,
};
pub(crate) use text::{
    apply_operations, plan_delete_backward, plan_delete_scalar_range, plan_insert_text,
    plan_replace_selection_text, plan_split, TextReplacementPlanError,
};
use text::{default_text_block, node_delete_start};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SemanticOperation {
    InsertText {
        pos: u32,
        text: String,
        marks: Vec<Mark>,
    },
    DeleteRange {
        from: u32,
        to: u32,
    },
    AddMark {
        from: u32,
        to: u32,
        mark: Mark,
    },
    RemoveMark {
        from: u32,
        to: u32,
        mark_type: String,
    },
    ReplaceMark {
        from: u32,
        to: u32,
        mark: Mark,
    },
    ReplaceRange {
        from: u32,
        to: u32,
        content: Fragment,
    },
    SplitBlock {
        pos: u32,
        node_type: String,
        attrs: HashMap<String, serde_json::Value>,
    },
    JoinBlocks {
        pos: u32,
    },
    UnwrapFromList {
        pos: u32,
    },
    OutdentListItem {
        pos: u32,
    },
    WrapInList {
        from: u32,
        to: u32,
        list_type: String,
        item_type: String,
        attrs: HashMap<String, serde_json::Value>,
        item_attrs: HashMap<String, serde_json::Value>,
    },
    IndentListItem {
        pos: u32,
    },
    InsertNode {
        pos: u32,
        node: Node,
    },
    UpdateNodeAttrs {
        pos: u32,
        attrs: HashMap<String, serde_json::Value>,
    },
}

impl SemanticOperation {
    pub(crate) fn as_step(&self) -> Step {
        match self {
            Self::InsertText { pos, text, marks } => Step::InsertText {
                pos: *pos,
                text: text.clone(),
                marks: marks.clone(),
            },
            Self::DeleteRange { from, to } => Step::DeleteRange {
                from: *from,
                to: *to,
            },
            Self::AddMark { from, to, mark } => Step::AddMark {
                from: *from,
                to: *to,
                mark: mark.clone(),
            },
            Self::RemoveMark {
                from,
                to,
                mark_type,
            } => Step::RemoveMark {
                from: *from,
                to: *to,
                mark_type: mark_type.clone(),
            },
            // ReplaceMark is a collapsed stored-mark transition. The legacy
            // adapter commits `stored_marks_after` directly, so this fallback
            // step is never applied to a document range.
            Self::ReplaceMark { from, to, mark } => Step::AddMark {
                from: *from,
                to: *to,
                mark: mark.clone(),
            },
            Self::ReplaceRange { from, to, content } => Step::ReplaceRange {
                from: *from,
                to: *to,
                content: content.clone(),
            },
            Self::SplitBlock {
                pos,
                node_type,
                attrs,
            } => Step::SplitBlock {
                pos: *pos,
                node_type: node_type.clone(),
                attrs: attrs.clone(),
            },
            Self::JoinBlocks { pos } => Step::JoinBlocks { pos: *pos },
            Self::UnwrapFromList { pos } => Step::UnwrapFromList { pos: *pos },
            Self::OutdentListItem { pos } => Step::OutdentListItem { pos: *pos },
            Self::WrapInList {
                from,
                to,
                list_type,
                item_type,
                attrs,
                item_attrs,
            } => Step::WrapInList {
                from: *from,
                to: *to,
                list_type: list_type.clone(),
                item_type: item_type.clone(),
                attrs: attrs.clone(),
                item_attrs: item_attrs.clone(),
            },
            Self::IndentListItem { pos } => Step::IndentListItem { pos: *pos },
            Self::InsertNode { pos, node } => Step::InsertNode {
                pos: *pos,
                node: node.clone(),
            },
            Self::UpdateNodeAttrs { pos, attrs } => Step::UpdateNodeAttrs {
                pos: *pos,
                attrs: attrs.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SemanticCommandHistory {
    InputBoundary,
    FormatBoundary,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SemanticCommandPlan {
    pub operations: Vec<SemanticOperation>,
    pub selection_after: Option<Selection>,
    pub history: SemanticCommandHistory,
}

impl SemanticCommandPlan {
    fn one(operation: SemanticOperation) -> Self {
        Self {
            operations: vec![operation],
            selection_after: None,
            history: SemanticCommandHistory::InputBoundary,
        }
    }
}

pub(crate) fn canonical_marks(marks: &[Mark], schema: &Schema) -> Vec<Mark> {
    let mut marks = marks.to_vec();
    marks.sort_by(|left, right| {
        schema
            .mark_rank(left.mark_type())
            .unwrap_or(usize::MAX)
            .cmp(&schema.mark_rank(right.mark_type()).unwrap_or(usize::MAX))
            .then_with(|| left.mark_type().cmp(right.mark_type()))
    });
    marks
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarkRequestError {
    UnknownMark,
    RequiredAttribute(String),
    UndeclaredAttribute(String),
}

pub(crate) fn validate_mark_request(
    schema: &Schema,
    mark_type: &str,
    attrs: &HashMap<String, serde_json::Value>,
) -> Result<(), MarkRequestError> {
    validate_mark_type(schema, mark_type)?;
    let spec = schema
        .mark(mark_type)
        .expect("validated mark type must exist");
    if let Some(name) = spec.attrs.iter().find_map(|(name, attr)| {
        (!attr.has_default && !attrs.contains_key(name)).then(|| name.clone())
    }) {
        return Err(MarkRequestError::RequiredAttribute(name));
    }
    if !spec.allow_undeclared_attrs {
        if let Some(name) = attrs.keys().find(|name| !spec.attrs.contains_key(*name)) {
            return Err(MarkRequestError::UndeclaredAttribute(name.clone()));
        }
    }
    Ok(())
}

pub(crate) fn validate_mark_type(schema: &Schema, mark_type: &str) -> Result<(), MarkRequestError> {
    schema
        .mark(mark_type)
        .map(|_| ())
        .ok_or(MarkRequestError::UnknownMark)
}

include!("command_planner/backspace.rs");

include!("command_planner/empty_blocks.rs");

pub(crate) fn outdented_list_item_position(
    before: &Document,
    after: &Document,
    position: u32,
    schema: &Schema,
) -> Option<u32> {
    if before == after {
        return None;
    }
    let resolved = before.resolve(position).ok()?;
    let mut node = before.root();
    let mut item_depth = None;
    for (depth, index) in resolved.node_path.iter().copied().enumerate() {
        node = node.child(usize::try_from(index).ok()?)?;
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
        {
            item_depth = Some(depth);
        }
    }

    let item_depth = item_depth?;
    let nested_list_path = &resolved.node_path[..item_depth];
    let parent_item_path = nested_list_path.get(..nested_list_path.len().checked_sub(1)?)?;
    let parent_list_path = parent_item_path.get(..parent_item_path.len().checked_sub(1)?)?;
    let parent_item_index = *parent_item_path.last()?;

    let mut destination_path = parent_list_path.to_vec();
    destination_path.push(parent_item_index.checked_add(1)?);
    destination_path.extend_from_slice(resolved.node_path.get(item_depth.checked_add(1)?..)?);
    let destination_content_start = node_start_at_path(after, &destination_path)?.checked_add(1)?;
    destination_content_start.checked_add(resolved.parent_offset)
}

fn node_start_at_path(document: &Document, path: &[u32]) -> Option<u32> {
    let mut node = document.root();
    let mut content_start = 0u32;
    let mut node_start = 0u32;
    for index in path.iter().copied() {
        let content = node.content()?;
        let index = usize::try_from(index).ok()?;
        node_start = content_start;
        for sibling in content.iter().take(index) {
            node_start = node_start.checked_add(sibling.node_size())?;
        }
        node = content.child(index)?;
        content_start = node_start.checked_add(1)?;
    }
    Some(node_start)
}

fn preferred_split_operation(
    document: &Document,
    schema: &Schema,
    position: u32,
    limits: &ResourceLimits,
) -> Result<Option<SemanticOperation>, ()> {
    let resolved = match document.resolve(position) {
        Ok(resolved) => resolved,
        Err(_) => return Ok(None),
    };
    let Some(&block_index) = resolved.node_path.last() else {
        return Ok(None);
    };
    let parent_path = &resolved.node_path[..resolved.node_path.len() - 1];
    let parent = if parent_path.is_empty() {
        document.root()
    } else {
        document.node_at(parent_path).ok_or(())?
    };
    let parent_spec = schema.node(parent.node_type()).ok_or(())?;
    let content = parent.content().ok_or(())?;
    let index = usize::try_from(block_index).map_err(|_| ())?;
    let prefix = content
        .iter()
        .take(index + 1)
        .map(Node::node_type)
        .collect::<Vec<_>>();
    let suffix = content
        .iter()
        .skip(index + 1)
        .map(Node::node_type)
        .collect::<Vec<_>>();
    let work_limit = limits.max_document_nodes.saturating_mul(128);
    let budget = WorkBudget::new(work_limit);
    for name in preferred_text_blocks_for_parent(schema, parent_spec, &prefix, &suffix, &budget)? {
        let Some(spec) = schema.node(&name) else {
            continue;
        };
        if !spec.attrs.values().all(|attr| attr.has_default) {
            continue;
        }
        let attrs = spec
            .attrs
            .iter()
            .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
            .collect::<HashMap<_, _>>();
        let operation = SemanticOperation::SplitBlock {
            pos: position,
            node_type: name,
            attrs,
        };
        let Ok(candidate) = apply_operations(document, schema, std::slice::from_ref(&operation))
        else {
            continue;
        };
        if crate::transform::DocumentValidator::validate(&candidate, schema, limits).is_ok() {
            return Ok(Some(operation));
        }
    }
    Ok(None)
}

fn preferred_text_blocks_for_parent(
    schema: &Schema,
    parent_spec: &NodeSpec,
    prefix: &[&str],
    suffix: &[&str],
    budget: &WorkBudget,
) -> Result<Vec<String>, ()> {
    let groups = parent_spec.content.accepting_symbols_after_with_budget(
        prefix,
        |child, symbol| schema.node_matches_symbol(child, symbol),
        budget,
    )?;
    let paragraph = schema
        .node_by_html_tag("p")
        .or_else(|| schema.node("paragraph"))
        .map(|spec| spec.name.as_str());
    let mut candidates = Vec::new();
    for spec in schema.all_nodes() {
        if !budget.consume() {
            return Err(());
        }
        if !matches!(spec.role, NodeRole::TextBlock)
            || !groups
                .iter()
                .any(|group| schema.node_matches_symbol(&spec.name, group))
        {
            continue;
        }
        let types = prefix
            .iter()
            .copied()
            .chain(std::iter::once(spec.name.as_str()))
            .chain(suffix.iter().copied())
            .collect::<Vec<_>>();
        if parent_spec.content.matches_with_budget(
            &types,
            |child, symbol| schema.node_matches_symbol(child, symbol),
            budget,
        )? {
            candidates.push(spec.name.clone());
        }
    }
    candidates.sort_by(|left, right| {
        (Some(left.as_str()) != paragraph)
            .cmp(&(Some(right.as_str()) != paragraph))
            .then_with(|| left.cmp(right))
    });
    candidates.dedup();
    Ok(candidates)
}

fn plan_empty_non_default_text_block(
    document: &Document,
    schema: &Schema,
    position: u32,
    limits: &ResourceLimits,
) -> Result<Option<SemanticCommandPlan>, ()> {
    let resolved = match document.resolve(position) {
        Ok(resolved) => resolved,
        Err(_) => return Ok(None),
    };
    let block = resolved.parent(document);
    if !schema
        .node(block.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || resolved.parent_offset != 0
        || block.content_size() != 0
    {
        return Ok(None);
    }
    let Some(&index) = resolved.node_path.last() else {
        return Ok(None);
    };
    let parent_path = &resolved.node_path[..resolved.node_path.len() - 1];
    let parent = if parent_path.is_empty() {
        document.root()
    } else {
        document.node_at(parent_path).ok_or(())?
    };
    let parent_spec = schema.node(parent.node_type()).ok_or(())?;
    let content = parent.content().ok_or(())?;
    let index = usize::try_from(index).map_err(|_| ())?;
    let prefix = content
        .iter()
        .take(index)
        .map(Node::node_type)
        .collect::<Vec<_>>();
    let suffix = content
        .iter()
        .skip(index + 1)
        .map(Node::node_type)
        .collect::<Vec<_>>();
    let budget = WorkBudget::new(limits.max_document_nodes.saturating_mul(128));
    let candidates =
        preferred_text_blocks_for_parent(schema, parent_spec, &prefix, &suffix, &budget)?;
    if candidates
        .first()
        .is_some_and(|name| name == block.node_type())
    {
        return Ok(None);
    }
    let from = node_delete_start(document, &resolved.node_path).ok_or(())?;
    for name in candidates
        .into_iter()
        .filter(|name| name != block.node_type())
    {
        let Some(spec) = schema.node(&name) else {
            continue;
        };
        if !spec.attrs.values().all(|attr| attr.has_default) {
            continue;
        }
        let attrs = spec
            .attrs
            .iter()
            .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
            .collect();
        let operation = SemanticOperation::ReplaceRange {
            from,
            to: from.checked_add(block.node_size()).ok_or(())?,
            content: Fragment::from(vec![Node::element(name, attrs, Fragment::empty())]),
        };
        if apply_operations(document, schema, std::slice::from_ref(&operation)).is_ok() {
            return Ok(Some(SemanticCommandPlan {
                operations: vec![operation],
                selection_after: Some(Selection::cursor(from + 1)),
                history: SemanticCommandHistory::InputBoundary,
            }));
        }
    }
    Ok(None)
}
