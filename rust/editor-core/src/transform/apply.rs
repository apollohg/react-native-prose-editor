//! Step application logic.
//!
//! Each step produces a new `Document` (immutable tree transformation) and a
//! `StepMap` recording how positions shifted.
use std::collections::HashMap;
use std::collections::HashSet;

#[cfg(test)]
thread_local! {
    static MARK_SET_HASH_ALLOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_mark_set_hash_allocation() {
    MARK_SET_HASH_ALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
}

#[cfg(test)]
fn reset_mark_set_hash_allocations_for_test() {
    MARK_SET_HASH_ALLOCATIONS.with(|count| count.set(0));
}

#[cfg(test)]
fn take_mark_set_hash_allocations_for_test() -> usize {
    MARK_SET_HASH_ALLOCATIONS.with(|count| count.replace(0))
}

use crate::boundary::{
    BoundaryError, BoundaryResult, JsonMeterDimension, JsonMeterError, JsonValueMeter,
    ResourceLimits,
};
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::content_rule::WorkBudget;
use crate::schema::{NodeRole, Schema};

use super::mapping::StepMap;
use super::steps::{
    add_mark_to_set, merge_adjacent_text_nodes, rebuild_element, remove_mark_from_set,
    split_text_node,
};
use super::{Step, TransformError};

/// Apply a single step to a document, producing a new document and step map.
///
/// This does NOT validate the resulting document against the schema — that is
/// done once after all steps in a transaction have been applied.
pub fn apply_step(
    doc: &Document,
    step: &Step,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    match step {
        Step::InsertText { pos, text, marks } => apply_insert_text(doc, *pos, text, marks, schema),
        Step::DeleteRange { from, to } => apply_delete_range(doc, *from, *to),
        Step::AddMark { from, to, mark } => apply_add_mark(doc, *from, *to, mark, schema),
        Step::RemoveMark {
            from,
            to,
            mark_type,
        } => apply_remove_mark(doc, *from, *to, mark_type),

        Step::SplitBlock {
            pos,
            node_type,
            attrs,
        } => apply_split_block(doc, *pos, node_type, attrs, schema),
        Step::JoinBlocks { pos } => apply_join_blocks(doc, *pos),
        Step::WrapInList {
            from,
            to,
            list_type,
            item_type,
            attrs,
            item_attrs,
        } => apply_wrap_in_list(
            doc, *from, *to, list_type, item_type, attrs, item_attrs, schema,
        ),
        Step::UnwrapFromList { pos } => apply_unwrap_from_list(doc, *pos, schema),

        Step::IndentListItem { pos } => apply_indent_list_item(doc, *pos, schema),

        Step::OutdentListItem { pos } => apply_outdent_list_item(doc, *pos, schema),
        Step::InsertNode { pos, node } => apply_insert_node(doc, *pos, node, schema),
        Step::UpdateNodeAttrs { pos, attrs } => apply_update_node_attrs(doc, *pos, attrs),
        Step::ReplaceRange { from, to, content } => {
            apply_replace_range(doc, *from, *to, content, schema)
        }
    }
}

/// Apply a semantic step and canonicalize mark order without changing the
/// legacy `apply_step` behavior used by the production standalone editor.
pub(crate) fn apply_step_canonical_marks(
    doc: &Document,
    step: &Step,
    schema: &Schema,
) -> Result<(Document, StepMap), TransformError> {
    #[cfg(test)]
    crate::yrs_engine::observability::record_ordinary_step_application();
    let (document, step_map) = apply_step(doc, step, schema)?;
    Ok((canonicalize_yrs_document(&document, schema), step_map))
}

pub(crate) fn canonicalize_yrs_document(document: &Document, schema: &Schema) -> Document {
    fn compare_marks(left: &Mark, right: &Mark, schema: &Schema) -> std::cmp::Ordering {
        schema
            .mark_rank(left.mark_type())
            .unwrap_or(usize::MAX)
            .cmp(&schema.mark_rank(right.mark_type()).unwrap_or(usize::MAX))
            .then_with(|| left.mark_type().cmp(right.mark_type()))
    }

    fn is_canonical_node(root: &Node, schema: &Schema) -> bool {
        let mut pending = vec![root];
        while let Some(node) = pending.pop() {
            #[cfg(test)]
            crate::yrs_engine::observability::record_canonical_identity_predicate_node_visited();
            if node
                .marks()
                .windows(2)
                .any(|marks| compare_marks(&marks[0], &marks[1], schema).is_gt())
            {
                return false;
            }
            let Some(content) = node.content() else {
                continue;
            };
            let mut previous = None;
            for child in content.iter() {
                if child.text_str().is_some_and(str::is_empty)
                    || previous.is_some_and(|previous: &Node| {
                        previous.is_text()
                            && child.is_text()
                            && super::steps::marks_eq(previous.marks(), child.marks())
                    })
                {
                    return false;
                }
                previous = Some(child);
            }
            pending.extend(content.iter());
        }
        true
    }

    fn canonicalize_node(root: &Node, schema: &Schema) -> Node {
        enum Frame<'a> {
            Visit(&'a Node),
            Build(&'a Node, usize),
        }

        let mut frames = vec![Frame::Visit(root)];
        let mut built = Vec::new();
        while let Some(frame) = frames.pop() {
            match frame {
                Frame::Visit(node) => {
                    if let Some(text) = node.text_str() {
                        let mut marks = node.marks().to_vec();
                        marks.sort_by(|left, right| compare_marks(left, right, schema));
                        built.push(Node::text(text.to_string(), marks));
                    } else if let Some(content) = node.content() {
                        frames.push(Frame::Build(node, content.child_count()));
                        frames.extend(content.iter().rev().map(Frame::Visit));
                    } else {
                        built.push(node.clone());
                    }
                }
                Frame::Build(node, child_count) => {
                    let first = built
                        .len()
                        .checked_sub(child_count)
                        .expect("canonicalization frame stack is balanced");
                    let children = merge_adjacent_text_nodes(built.split_off(first));
                    built.push(rebuild_element(node, children));
                }
            }
        }
        built.pop().expect("canonicalization produces one root")
    }

    if is_canonical_node(document.root(), schema) {
        return document.clone();
    }
    Document::new(canonicalize_node(document.root(), schema))
}

/// Canonicality proof produced as a by-product of canonical mark validation.
///
/// The proof is intentionally non-cloneable and bound to the exact immutable
/// root storage that was traversed.
#[derive(Debug)]
pub(crate) struct CanonicalMarksEvidence<'schema> {
    source_root: Node,
    source_schema: &'schema Schema,
    is_canonical: bool,
}

pub(crate) fn canonicalize_yrs_document_with_evidence(
    document: &Document,
    schema: &Schema,
    evidence: CanonicalMarksEvidence<'_>,
) -> Document {
    if evidence.is_canonical
        && document.root().shares_storage_with(&evidence.source_root)
        && std::ptr::eq(schema, evidence.source_schema)
    {
        document.clone()
    } else {
        canonicalize_yrs_document(document, schema)
    }
}

include!("apply/text.rs");

include!("apply/block.rs");

include!("apply/list_wrap.rs");

include!("apply/node.rs");

include!("apply/list_indent.rs");

include!("apply/range.rs");

include!("apply/validation.rs");
