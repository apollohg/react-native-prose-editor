use smallvec::SmallVec;

use crate::model::node::Node;
use crate::model::Document;
use crate::render;
use crate::schema::{NodeRole, Schema};

use super::{BlockMapping, PositionMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PositionBlockKind {
    Text,
    Void,
}

/// Classify nodes exactly as the PositionMap DFS does. Callers must stop
/// descending when this returns a block kind.
pub(crate) fn classify_position_block(node: &Node, schema: &Schema) -> Option<PositionBlockKind> {
    if node.is_void() {
        Some(PositionBlockKind::Void)
    } else if node.is_element()
        && schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
    {
        Some(PositionBlockKind::Text)
    } else {
        None
    }
}

/// Placeholder character used for void block-level nodes (e.g. horizontalRule)
/// in the rendered text. U+FFFC OBJECT REPLACEMENT CHARACTER.
pub const VOID_BLOCK_PLACEHOLDER: char = '\u{FFFC}';

/// Placeholder character used for inline void nodes (e.g. hardBreak) in the
/// rendered text.
pub const HARD_BREAK_PLACEHOLDER: char = '\n';

/// Number of rendered scalars for a block separator (newline between blocks).
pub const BLOCK_BREAK_SCALARS: u32 = 1;

/// Build a `PositionMap` by walking the document tree.
///
/// The walk identifies every "text block" (a leaf-level element that directly
/// contains inline content, e.g. `paragraph`) and every block-level void node
/// (e.g. `horizontalRule`). Each one becomes a `BlockMapping`.
///
/// `schema` is required so list/list-item detection matches the renderer,
/// which walks by `NodeRole::List` / `NodeRole::ListItem` rather than
/// hardcoded node names — a custom-named list (e.g. an app-defined
/// `todoTaskList`) must be recognized here exactly as it is by the renderer.
pub fn build_position_map(doc: &Document, schema: &Schema) -> PositionMap {
    let mut blocks: Vec<BlockMapping> = Vec::new();
    let mut scalar_cursor: u32 = 0;
    let path: SmallVec<[u32; 8]> = SmallVec::new();

    // The root node ("doc") is an element. We walk its direct and indirect
    // children looking for text blocks and block-level void nodes.
    walk_node(
        doc.root(),
        schema,
        &path,
        0, // doc_offset: content starts at position 0 inside the doc
        &mut blocks,
        &mut scalar_cursor,
        0,
    );

    // Fix up rendered_break_after: every block except the last gets a break
    // scalar. The last block (terminal) gets 0.
    let block_count = blocks.len();
    for (i, block) in blocks.iter_mut().enumerate() {
        if i + 1 < block_count {
            block.rendered_break_after = BLOCK_BREAK_SCALARS;
            // scalar_start of subsequent blocks is recalculated below.
        } else {
            block.rendered_break_after = 0;
        }
    }

    // Now fix scalar_start values to account for inter-block breaks.
    // We rebuild them: the first block starts at 0, each subsequent block
    // starts at prev.scalar_start + prev.scalar_len + prev.rendered_break_after.
    if !blocks.is_empty() {
        blocks[0].scalar_start = 0;
        for i in 1..blocks.len() {
            let prev_end = blocks[i - 1].scalar_start
                + blocks[i - 1].scalar_prefix_len
                + blocks[i - 1].scalar_len
                + blocks[i - 1].rendered_break_after;
            blocks[i].scalar_start = prev_end;
        }
    }

    PositionMap::from_blocks(blocks, schema)
}

pub(crate) fn rebuild_existing_block_mapping(
    node: &Node,
    old_block: &BlockMapping,
    schema: &Schema,
) -> Option<BlockMapping> {
    if old_block.is_void_block {
        if classify_position_block(node, schema) != Some(PositionBlockKind::Void) {
            return None;
        }

        return Some(BlockMapping {
            doc_start: old_block.doc_start,
            doc_end: old_block.doc_start,
            scalar_start: old_block.scalar_start,
            scalar_len: block_visible_scalar_len(node, schema),
            scalar_prefix_len: old_block.scalar_prefix_len,
            rendered_break_after: old_block.rendered_break_after,
            node_path: old_block.node_path.clone(),
            is_void_block: true,
        });
    }

    if classify_position_block(node, schema) != Some(PositionBlockKind::Text) {
        return None;
    }

    let content = node.content()?;
    Some(BlockMapping {
        doc_start: old_block.doc_start,
        doc_end: old_block.doc_start + content.size(),
        scalar_start: old_block.scalar_start,
        scalar_len: compute_inline_scalars(node, schema),
        scalar_prefix_len: old_block.scalar_prefix_len,
        rendered_break_after: old_block.rendered_break_after,
        node_path: old_block.node_path.clone(),
        is_void_block: false,
    })
}

/// Recursively walk a node to find text blocks and block-level void nodes.
///
/// `doc_offset` is the doc position at the start of `node`'s content
/// (i.e. just after the open tag for element nodes, or the position of a
/// void/text node).
fn walk_node(
    node: &Node,
    schema: &Schema,
    path: &SmallVec<[u32; 8]>,
    doc_offset: u32,
    blocks: &mut Vec<BlockMapping>,
    scalar_cursor: &mut u32,
    mut pending_prefix_len: u32,
) {
    if node.is_text() {
        // Text nodes are inline content — handled by their parent block.
        return;
    }

    if classify_position_block(node, schema) == Some(PositionBlockKind::Void) {
        // Block-level void node (e.g. horizontalRule).
        // Rendered as a placeholder or opaque label.
        blocks.push(BlockMapping {
            doc_start: doc_offset,
            doc_end: doc_offset, // void has no "content range", just a position
            scalar_start: *scalar_cursor, // will be recalculated
            scalar_len: block_visible_scalar_len(node, schema),
            scalar_prefix_len: std::mem::take(&mut pending_prefix_len),
            rendered_break_after: 0, // will be fixed up
            node_path: path.clone(),
            is_void_block: true,
        });
        *scalar_cursor +=
            blocks.last().unwrap().scalar_prefix_len + blocks.last().unwrap().scalar_len;
        return;
    }

    // Element node — check if it's a text block (contains only inline content)
    // or a container (contains other elements).
    let content = node.content().expect("element nodes have content");

    if classify_position_block(node, schema) == Some(PositionBlockKind::Text) {
        // This is a text block. Compute the scalar length from its inline content.
        let scalar_len = compute_inline_scalars(node, schema);

        blocks.push(BlockMapping {
            doc_start: doc_offset,
            doc_end: doc_offset + content.size(),
            scalar_start: *scalar_cursor, // will be recalculated
            scalar_len,
            scalar_prefix_len: std::mem::take(&mut pending_prefix_len),
            rendered_break_after: 0, // will be fixed up
            node_path: path.clone(),
            is_void_block: false,
        });
        *scalar_cursor += blocks.last().unwrap().scalar_prefix_len + scalar_len;
        return;
    }

    // Container node — recurse into children.
    let mut child_doc_offset = doc_offset;
    for (child_idx, child) in content.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(u32::try_from(child_idx).expect("validated document index fits u32"));
        let mut child_prefix_len = pending_prefix_len;
        pending_prefix_len = 0;

        if schema.is_list(node.node_type()) && schema.is_list_item(child.node_type()) {
            child_prefix_len += list_marker_len(schema, node, child_idx);
        }

        if child.is_element() {
            // Skip the open tag to get to the child's content start
            walk_node(
                child,
                schema,
                &child_path,
                child_doc_offset + 1, // +1 for open tag
                blocks,
                scalar_cursor,
                child_prefix_len,
            );
        } else if child.is_void() {
            // Block-level void (e.g. hr at doc level).
            // The doc position of the void node is child_doc_offset.
            walk_node(
                child,
                schema,
                &child_path,
                child_doc_offset,
                blocks,
                scalar_cursor,
                child_prefix_len,
            );
        } else {
            pending_prefix_len = child_prefix_len;
        }
        // Text nodes at this level would be unusual (text directly in doc)
        // but we skip them — they'd only appear in text blocks which we
        // handle above.

        child_doc_offset += child.node_size();
    }
}

/// Determine if a node is a "text block" — an element that contains only
/// inline content (text nodes and inline void nodes like hardBreak).
///
/// An element with no children (empty paragraph) is also a text block.
/// Count the number of rendered scalars in a text block's inline content.
///
/// - Text nodes contribute their Unicode scalar count.
/// - Inline void nodes (hardBreak) contribute 1 scalar each.
fn compute_inline_scalars(node: &Node, schema: &Schema) -> u32 {
    let content = match node.content() {
        Some(c) => c,
        None => return 0,
    };

    if content.child_count() == 0 {
        return 1;
    }

    let mut count: u32 = 0;
    for child in content.iter() {
        if child.is_text() {
            count += child.node_size(); // node_size for text = scalar count
        } else if child.is_void() {
            count += inline_visible_scalar_len(child, schema);
        }
        // Element children inside a text block shouldn't exist, but if they
        // do we skip them (defensive).
    }
    count
}

/// Number of rendered scalars a list item's marker prefix occupies.
///
/// Derives from `render::task_list_marker_metadata` — the same predicate the
/// renderer uses to decide whether an item gets a checkbox marker — so the
/// position map can never disagree with what was actually rendered.
fn list_marker_len(schema: &Schema, list_node: &Node, child_index: usize) -> u32 {
    if let Some(item) = list_node.child(child_index) {
        let (kind, checked) = render::task_list_marker_metadata(list_node.node_type(), item);
        if kind.as_deref() == Some("task") {
            return render::task_list_marker_string(checked.unwrap_or(false))
                .chars()
                .count() as u32;
        }
    }

    let ordered = schema.is_ordered_list(list_node.node_type());
    let start = list_node
        .attrs()
        .get("start")
        .and_then(|value| value.as_u64())
        .unwrap_or(1) as u32;
    let index = if ordered {
        start + child_index as u32
    } else {
        child_index as u32 + 1
    };
    render::list_marker_string(ordered, index).chars().count() as u32
}

fn inline_visible_scalar_len(node: &Node, schema: &Schema) -> u32 {
    let label = render::inline_atom_label(node.node_type(), node.attrs());
    render::inline_node_visible_scalar_len(
        node.node_type(),
        Some(label.as_str()),
        schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::HardBreak)),
    )
}

fn block_visible_scalar_len(node: &Node, schema: &Schema) -> u32 {
    if schema
        .node(node.node_type())
        .is_some_and(|spec| matches!(spec.role, NodeRole::Block))
    {
        return 1;
    }
    let label = render::inline_atom_label(node.node_type(), node.attrs());
    render::block_node_visible_scalar_len(
        node.node_type(),
        Some(label.as_str()),
        matches!(node.node_type(), "horizontalRule" | "horizontal_rule"),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::position::update::UpdateMode;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
    use crate::transform::StepMap;

    fn role_schema() -> Schema {
        Schema::from_json(&json!({
            "nodes": [
                { "name": "doc", "content": "block*", "role": "doc" },
                { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
                { "name": "image", "content": "", "group": "block", "role": "block", "isVoid": true },
                { "name": "customVoid", "content": "", "group": "block", "role": "block", "isVoid": true },
                { "name": "emptyBlock", "content": "", "group": "block", "role": "block" },
                { "name": "softBreak", "content": "", "group": "inline", "role": "hardBreak", "isVoid": true, "allowUndeclaredAttrs": true },
                { "name": "mention", "content": "", "group": "inline", "role": "inline", "isVoid": true, "allowUndeclaredAttrs": true },
                { "name": "text", "group": "inline", "role": "text" }
            ],
            "marks": []
        }))
        .unwrap()
    }

    fn assert_render_map_parity(value: serde_json::Value) -> (Document, PositionMap, String) {
        let schema = role_schema();
        let document = from_prosemirror_json(&value, &schema, UnknownTypeMode::Preserve).unwrap();
        let map = build_position_map(&document, &schema);
        let rendered = render::rendered_text(&document, &schema);
        assert_eq!(map.total_scalars(), rendered.chars().count() as u32);
        (document, map, rendered)
    }

    #[test]
    fn role_driven_void_blocks_match_rendering_alone_and_with_a_sibling() {
        let (image, image_map, rendered) = assert_render_map_parity(json!({
            "type": "doc",
            "content": [{ "type": "image" }]
        }));
        assert_eq!(rendered, VOID_BLOCK_PLACEHOLDER.to_string());
        assert_eq!(image_map.scalar_to_doc(0, &image), 0);

        let (document, map, rendered) = assert_render_map_parity(json!({
            "type": "doc",
            "content": [
                { "type": "customVoid" },
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }
            ]
        }));
        assert_eq!(rendered, format!("{}\nx", VOID_BLOCK_PLACEHOLDER));
        assert_eq!(map.scalar_to_doc(0, &document), 0);
        assert_eq!(map.scalar_to_doc(2, &document), 2);
    }

    #[test]
    fn empty_doc_and_generic_empty_block_do_not_become_text_blocks() {
        assert_render_map_parity(json!({ "type": "doc" }));
        let (_, map, rendered) = assert_render_map_parity(json!({
            "type": "doc",
            "content": [{ "type": "emptyBlock" }]
        }));
        assert!(rendered.is_empty());
        assert_eq!(map.block_count(), 0);
    }

    #[test]
    fn custom_hard_break_and_inline_atom_use_renderer_visible_lengths() {
        let (document, map, rendered) = assert_render_map_parity(json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    { "type": "softBreak", "attrs": { "label": "ignored-multiscalar-label" } },
                    { "type": "mention", "attrs": { "label": "Ada" } }
                ]
            }]
        }));
        assert_eq!(rendered, "\nAda");
        assert_eq!(
            (0..=4)
                .map(|scalar| map.scalar_to_doc(scalar, &document))
                .collect::<Vec<_>>(),
            vec![1, 2, 2, 2, 3]
        );
        assert_eq!(
            (0..=3)
                .map(|position| map.doc_to_scalar(position, &document))
                .collect::<Vec<_>>(),
            vec![0, 0, 1, 4]
        );
    }

    #[test]
    fn incremental_rebuild_uses_the_same_schema_role_lengths() {
        let schema = role_schema();
        let old_document = from_prosemirror_json(
            &json!({
                "type": "doc",
                "content": [{ "type": "paragraph", "content": [{ "type": "softBreak" }] }]
            }),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let new_document = from_prosemirror_json(
            &json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "mention", "attrs": { "label": "Ada" } }]
                }]
            }),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap();
        let mut map = build_position_map(&old_document, &schema);
        map.update(
            &StepMap::from_replace(1, 1, 1),
            &old_document,
            &new_document,
            UpdateMode::InlineTextOnly,
            &schema,
        );
        assert_eq!(
            map.total_scalars(),
            render::rendered_text(&new_document, &schema)
                .chars()
                .count() as u32
        );
    }
}
