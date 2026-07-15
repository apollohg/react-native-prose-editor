use std::collections::HashMap;

use crate::model::{Document, Fragment, Node};
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;

/// A semantic document-range replacement produced by a shared command planner.
/// It deliberately contains no standalone-backend transform step.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommandReplacement {
    pub from: u32,
    pub to: u32,
    pub content: Fragment,
    pub selection_after: Selection,
}

pub(crate) fn code_block_node_name(schema: &Schema) -> Option<&str> {
    schema
        .node_by_html_tag("pre")
        .filter(|spec| matches!(spec.role, NodeRole::TextBlock))
        .map(|spec| spec.name.as_str())
}

pub(crate) fn plan_toggle_heading(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    level: u8,
) -> Option<CommandReplacement> {
    let target_type = schema.node_by_html_tag(&format!("h{level}"))?.name.as_str();
    let paragraph_type = crate::editor_state::paragraph_node_name(schema)?;
    let range = crate::editor_state::selected_text_block_range(document, schema, selection)?;
    let replacement_type = if range
        .selected_blocks
        .iter()
        .all(|block| block.node_type() == target_type)
    {
        paragraph_type
    } else {
        target_type
    };
    replacement_for_text_blocks(document, schema, selection, range, replacement_type)
}

pub(crate) fn plan_toggle_code_block(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<CommandReplacement> {
    let code_block_type = code_block_node_name(schema)?;
    let paragraph_type = crate::editor_state::paragraph_node_name(schema)?;
    let range = crate::editor_state::selected_text_block_range(document, schema, selection)?;
    let replacement_type = if range
        .selected_blocks
        .iter()
        .all(|block| block.node_type() == code_block_type)
    {
        paragraph_type
    } else {
        code_block_type
    };
    replacement_for_text_blocks(document, schema, selection, range, replacement_type)
}

pub(crate) fn plan_toggle_blockquote(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
) -> Option<CommandReplacement> {
    let blockquote_type = schema.node_by_html_tag("blockquote")?.name.as_str();
    let pos = selection.from(document);
    if let Some((start, quote)) =
        crate::editor_state::containing_node_at(document, schema, pos, |_, name| {
            name == blockquote_type
        })
    {
        let content = quote.content()?;
        return Some(CommandReplacement {
            from: start,
            to: start.checked_add(quote.node_size())?,
            content: Fragment::from(content.iter().cloned().collect()),
            selection_after: shift_selection(selection, -1)?,
        });
    }
    let range = crate::editor_state::selected_block_range(
        document,
        schema,
        selection.from(document),
        selection.to(document),
    )?;
    let quote_spec = schema.node(blockquote_type)?;
    let selected = range
        .selected_blocks
        .iter()
        .map(Node::node_type)
        .collect::<Vec<_>>();
    if !quote_spec.content.matches(&selected, |child, symbol| {
        schema.node_matches_symbol(child, symbol)
    }) {
        return None;
    }
    Some(CommandReplacement {
        from: range.replace_from,
        to: range.replace_to,
        content: Fragment::from(vec![Node::element(
            blockquote_type.to_string(),
            HashMap::new(),
            Fragment::from(range.selected_blocks),
        )]),
        selection_after: shift_selection(selection, 1)?,
    })
}

fn replacement_for_text_blocks(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    range: crate::editor_state::BlockSelectionRange,
    target_type: &str,
) -> Option<CommandReplacement> {
    if !crate::editor_state::can_replace_selected_text_blocks(document, schema, &range, target_type)
    {
        return None;
    }
    let content = range
        .selected_blocks
        .iter()
        .map(|block| {
            Some(Node::element(
                target_type.to_string(),
                HashMap::new(),
                block.content().cloned().unwrap_or_else(Fragment::empty),
            ))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(CommandReplacement {
        from: range.replace_from,
        to: range.replace_to,
        content: Fragment::from(content),
        selection_after: selection.clone(),
    })
}

fn shift_selection(selection: &Selection, delta: i32) -> Option<Selection> {
    let shift = |position: u32| {
        if delta >= 0 {
            position.checked_add(delta as u32)
        } else {
            position.checked_sub(delta.unsigned_abs())
        }
    };
    match selection {
        Selection::Text { anchor, head } => Some(Selection::text(shift(*anchor)?, shift(*head)?)),
        Selection::Node { pos } => Some(Selection::node(shift(*pos)?)),
        Selection::All => Some(Selection::All),
    }
}
