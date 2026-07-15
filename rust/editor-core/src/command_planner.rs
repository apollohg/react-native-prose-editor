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
    plan_apply_list_type, plan_indent_list_item, plan_insert_node, plan_outdent_list_item,
    plan_resize_image, plan_toggle_task_item_checked, plan_unwrap_from_list, plan_wrap_in_list,
    prove_structural_diff, simulate_plan, structural_diff, structural_diff_bounded,
    ResizeImageRequest,
};
pub(crate) use text::{
    apply_operations, plan_delete_backward, plan_delete_scalar_range, plan_insert_text,
    plan_replace_selection_text, plan_split,
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
) -> Option<SemanticOperation> {
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
    if item_index == 0 {
        Some(SemanticOperation::UnwrapFromList { pos: doc_to })
    } else {
        Some(SemanticOperation::JoinBlocks {
            pos: node_delete_start(document, &resolved.node_path[..=depth])?,
        })
    }
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

fn empty_block_delete_range(
    document: &Document,
    map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    doc_from: u32,
    doc_to: u32,
) -> Option<(u32, u32)> {
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
    if index == 0 {
        return None;
    }
    let parent = document.node_at(&resolved.node_path[..resolved.node_path.len() - 1])?;
    let previous = parent.child(index as usize - 1)?;
    if !previous.is_element() && !previous.is_void() {
        return None;
    }
    let same_doc = doc_from == doc_to;
    let boundary = scalar_to == scalar_from.saturating_add(1)
        && doc_from < doc_to
        && doc_to == open.saturating_add(1)
        && map.doc_to_scalar(doc_from, document) == scalar_from;
    (same_doc || boundary).then_some((open, open.checked_add(block.node_size())?))
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
                return Some(SemanticCommandPlan::one(if nested {
                    SemanticOperation::OutdentListItem { pos: position }
                } else {
                    SemanticOperation::UnwrapFromList { pos: position }
                }));
            }
        }
    }
    plan_empty_blockquote_exit(document, schema, position)
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
