use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Node};
use crate::position::PositionMap;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;
use crate::transform::Transaction;

use super::{SemanticCommandHistory, SemanticCommandPlan, SemanticOperation};

fn insertion_marks(
    document: &Document,
    schema: &Schema,
    position: u32,
    stored_marks: Option<&[crate::model::Mark]>,
    collapsed: bool,
) -> Vec<crate::model::Mark> {
    let marks = if collapsed {
        stored_marks
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| crate::editor_state::marks_at_position(document, position))
    } else {
        crate::editor_state::marks_at_position(document, position)
    };
    super::canonical_marks(&marks, schema)
}

pub(crate) fn plan_insert_text(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[crate::model::Mark]>,
    text: &str,
) -> Option<SemanticCommandPlan> {
    let Selection::Text { anchor, head } = selection else {
        return None;
    };
    if anchor != head || text.is_empty() {
        return None;
    }
    let marks = insertion_marks(document, schema, *anchor, stored_marks, true);
    if is_terminal_void_block_gap(document, schema, *anchor) {
        let text_len = u32::try_from(text.chars().count()).ok()?;
        let block = default_text_block_with_content(
            schema,
            Fragment::from(vec![Node::text(text.to_string(), marks)]),
        )?;
        return Some(SemanticCommandPlan {
            operations: vec![SemanticOperation::InsertNode {
                pos: *anchor,
                node: block,
            }],
            selection_after: Some(Selection::cursor(
                anchor.checked_add(1)?.checked_add(text_len)?,
            )),
            history: SemanticCommandHistory::InputBoundary,
        });
    }
    Some(SemanticCommandPlan::one(SemanticOperation::InsertText {
        pos: *anchor,
        text: text.to_string(),
        marks,
    }))
}

pub(crate) fn plan_replace_selection_text(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    stored_marks: Option<&[crate::model::Mark]>,
    text: &str,
) -> Option<SemanticCommandPlan> {
    let Selection::Text { anchor, head } = selection else {
        return None;
    };
    let from = (*anchor).min(*head);
    let to = (*anchor).max(*head);
    let mut operations = Vec::with_capacity(2);
    if from < to {
        operations.push(SemanticOperation::DeleteRange { from, to });
    }
    if !text.is_empty() {
        operations.push(SemanticOperation::InsertText {
            pos: from,
            text: text.to_string(),
            marks: insertion_marks(document, schema, from, stored_marks, from == to),
        });
    }
    (!operations.is_empty()).then_some(SemanticCommandPlan {
        operations,
        selection_after: None,
        history: SemanticCommandHistory::InputBoundary,
    })
}

fn default_text_block_with_content(schema: &Schema, content: Fragment) -> Option<Node> {
    let spec = schema.preferred_text_block()?;
    let attrs = spec
        .attrs
        .iter()
        .filter_map(|(name, attr)| attr.default.clone().map(|value| (name.clone(), value)))
        .collect();
    Some(Node::element(spec.name.clone(), attrs, content))
}

pub(super) fn default_text_block(schema: &Schema) -> Option<Node> {
    default_text_block_with_content(schema, Fragment::empty())
}

fn is_terminal_void_block_gap(document: &Document, schema: &Schema, position: u32) -> bool {
    let resolved = match document.resolve(position) {
        Ok(resolved) => resolved,
        Err(_) => return false,
    };
    let parent = resolved.parent(document);
    if resolved.parent_offset != parent.content_size() {
        return false;
    }
    let Some(content) = parent.content() else {
        return false;
    };
    let Some(last) = content.iter().last() else {
        return false;
    };
    if !schema
        .node(last.node_type())
        .is_some_and(|spec| spec.is_void && matches!(spec.role, NodeRole::Block))
    {
        return false;
    }
    let Some(text_block) = schema.preferred_text_block() else {
        return false;
    };
    let Some(parent_spec) = schema.node(parent.node_type()) else {
        return false;
    };
    let child_types = content
        .iter()
        .map(Node::node_type)
        .chain(std::iter::once(text_block.name.as_str()))
        .collect::<Vec<_>>();
    parent_spec.content.matches(&child_types, |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    })
}

pub(super) fn node_delete_start(document: &Document, path: &[u32]) -> Option<u32> {
    let mut node = document.root();
    let mut open = 0u32;
    for index in path.iter().copied() {
        let content = node.content()?;
        let mut child_open = open.checked_add(1)?;
        for sibling in content.iter().take(index as usize) {
            child_open = child_open.checked_add(sibling.node_size())?;
        }
        node = content.child(index as usize)?;
        open = child_open;
    }
    open.checked_sub(1)
}

pub(crate) fn plan_delete_backward(
    document: &Document,
    position_map: &PositionMap,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) -> Result<Option<SemanticCommandPlan>, ()> {
    let Selection::Text { anchor, head } = selection else {
        return Ok(None);
    };
    let scalar_anchor = position_map.doc_to_scalar(*anchor, document);
    let scalar_head = position_map.doc_to_scalar(*head, document);
    let from = scalar_anchor.min(scalar_head);
    let to = scalar_anchor.max(scalar_head);
    if from < to {
        return plan_delete_scalar_range_impl(document, position_map, schema, from, to, true);
    }
    let cursor = selection.from(document);
    if let Some(plan) = super::plan_empty_split_action(document, schema, cursor) {
        return Ok(Some(plan));
    }
    if let Some(plan) = super::plan_empty_non_default_text_block(document, schema, cursor, limits)?
    {
        return Ok(Some(plan));
    }
    // Must precede the `to == 0` bail: a quote at the very start of the
    // document leaves the caret there, and giving up would make the keystroke
    // do nothing at all with no way out of the quote.
    if let Some(plan) = super::plan_blockquote_lift_at_start(document, schema, cursor) {
        return Ok(Some(plan));
    }
    if to == 0 {
        return Ok(None);
    }
    plan_delete_scalar_range_impl(document, position_map, schema, to - 1, to, true)
}

pub(crate) fn plan_delete_scalar_range(
    document: &Document,
    position_map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
) -> Result<Option<SemanticCommandPlan>, ()> {
    plan_delete_scalar_range_impl(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        false,
    )
}

fn plan_delete_scalar_range_impl(
    document: &Document,
    position_map: &PositionMap,
    schema: &Schema,
    scalar_from: u32,
    scalar_to: u32,
    is_backward_delete: bool,
) -> Result<Option<SemanticCommandPlan>, ()> {
    let doc_from = position_map.scalar_to_doc(scalar_from, document);
    let doc_to = position_map.scalar_to_doc(scalar_to, document);
    if let Some(pos) = super::empty_list_unwrap_pos(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_from,
        doc_to,
    ) {
        return Ok(Some(SemanticCommandPlan::one(
            SemanticOperation::UnwrapFromList { pos },
        )));
    }
    if scalar_from < scalar_to
        && doc_from == doc_to
        && position_map.doc_to_scalar(doc_to, document) == scalar_to
    {
        if let Some(plan) = super::plan_empty_blockquote_exit(document, schema, doc_to) {
            return Ok(Some(plan));
        }
    }
    if let Some(plan) = super::marker_backspace_action(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_from,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if let Some(plan) = super::lift_trailing_empty_list_block(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_from,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if let Some(plan) = super::replace_void_and_empty_block(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if let Some(plan) = super::replace_only_void_block(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_from,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if let Some((from, to)) = super::empty_block_delete_range(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_from,
        doc_to,
    ) {
        return Ok(Some(SemanticCommandPlan::one(
            SemanticOperation::DeleteRange { from, to },
        )));
    }
    if let Some(plan) = super::move_into_previous_blockquote_action(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if let Some(plan) = super::delete_selection_through_previous_void_block_action(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_from,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if let Some(plan) = super::delete_previous_void_block_action(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    if is_backward_delete
        && super::previous_void_block_at_text_head(
            document,
            position_map,
            schema,
            scalar_from,
            scalar_to,
            doc_to,
        )
        .is_some()
    {
        return Ok(None);
    }
    // Must precede the DeleteRange fallback: at the head of a non-empty text
    // block that range would straddle the block boundary, which the engine
    // rejects as a cross-parent structural deletion.
    if let Some(plan) = super::join_with_previous_block_action(
        document,
        position_map,
        schema,
        scalar_from,
        scalar_to,
        doc_to,
    ) {
        return Ok(Some(plan));
    }
    Ok(Some(SemanticCommandPlan::one(
        SemanticOperation::DeleteRange {
            from: doc_from,
            to: doc_to,
        },
    )))
}

pub(crate) fn plan_split(
    document: &Document,
    position_map: &PositionMap,
    schema: &Schema,
    selection: &Selection,
    delete_selection: bool,
    limits: &ResourceLimits,
) -> Result<Option<SemanticCommandPlan>, ()> {
    let Selection::Text { anchor, head } = selection else {
        return Ok(None);
    };
    let scalar_anchor = position_map.doc_to_scalar(*anchor, document);
    let scalar_head = position_map.doc_to_scalar(*head, document);
    let scalar_from = scalar_anchor.min(scalar_head);
    let scalar_to = scalar_anchor.max(scalar_head);
    let doc_from = position_map.scalar_to_doc(scalar_from, document);
    let doc_to = position_map.scalar_to_doc(scalar_to, document);

    if doc_from == doc_to && is_terminal_void_block_gap(document, schema, doc_from) {
        let block = default_text_block(schema).ok_or(())?;
        return Ok(Some(SemanticCommandPlan {
            operations: vec![SemanticOperation::InsertNode {
                pos: doc_from,
                node: block,
            }],
            selection_after: Some(Selection::cursor(doc_from.checked_add(1).ok_or(())?)),
            history: SemanticCommandHistory::InputBoundary,
        }));
    }

    if doc_from == doc_to {
        if let Some(plan) = plan_code_split(document, schema, doc_from, limits)? {
            return Ok(Some(plan));
        }
    }

    let mut operations = Vec::new();
    let preview = if delete_selection && doc_from < doc_to {
        let operation = SemanticOperation::DeleteRange {
            from: doc_from,
            to: doc_to,
        };
        let preview = apply_operations(document, schema, std::slice::from_ref(&operation))?;
        operations.push(operation);
        preview
    } else {
        document.clone()
    };

    if let Some(mut action) = super::plan_empty_split_action(&preview, schema, doc_from) {
        operations.append(&mut action.operations);
        action.operations = operations;
        return Ok(Some(action));
    }
    let Some(split) = super::preferred_split_operation(&preview, schema, doc_from, limits)? else {
        return Ok(None);
    };
    operations.push(split);
    Ok(Some(SemanticCommandPlan {
        operations,
        selection_after: None,
        history: SemanticCommandHistory::InputBoundary,
    }))
}

fn plan_code_split(
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
    if Some(block.node_type()) != super::code_block_node_name(schema) {
        return Ok(None);
    }
    if resolved.parent_offset == block.content_size() && block.text_content().ends_with('\n') {
        let delete = SemanticOperation::DeleteRange {
            from: position.checked_sub(1).ok_or(())?,
            to: position,
        };
        let preview = apply_operations(document, schema, std::slice::from_ref(&delete))?;
        let Some(split) = super::preferred_split_operation(&preview, schema, position - 1, limits)?
        else {
            return Err(());
        };
        let candidate = apply_operations(&preview, schema, std::slice::from_ref(&split))?;
        let block_path = resolved.node_path.as_slice();
        let &block_index = block_path.last().ok_or(())?;
        let parent_path = &block_path[..block_path.len() - 1];
        let parent = candidate.node_at(parent_path).ok_or(())?;
        let index = usize::try_from(block_index).map_err(|_| ())?;
        let left = parent.child(index).ok_or(())?.clone();
        let right = parent
            .child(index.checked_add(1).ok_or(())?)
            .ok_or(())?
            .clone();
        let from = node_delete_start(document, block_path).ok_or(())?;
        let cursor = from
            .checked_add(left.node_size())
            .and_then(|value| value.checked_add(1))
            .ok_or(())?;
        return Ok(Some(SemanticCommandPlan {
            operations: vec![SemanticOperation::ReplaceRange {
                from,
                to: from.checked_add(block.node_size()).ok_or(())?,
                content: Fragment::from(vec![left, right]),
            }],
            selection_after: Some(Selection::cursor(cursor)),
            history: SemanticCommandHistory::InputBoundary,
        }));
    }
    Ok(Some(SemanticCommandPlan::one(
        SemanticOperation::InsertText {
            pos: position,
            text: "\n".into(),
            marks: Vec::new(),
        },
    )))
}

pub(crate) fn apply_operations(
    document: &Document,
    schema: &Schema,
    operations: &[SemanticOperation],
) -> Result<Document, ()> {
    let mut transaction = Transaction::new(crate::transform::Source::Input);
    for operation in operations {
        transaction.add_step(operation.as_step());
    }
    transaction
        .apply_steps_unchecked(document, schema)
        .map(|(document, _)| document)
        .map_err(|_| ())
}
