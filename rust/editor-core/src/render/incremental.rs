use std::collections::BTreeSet;

use crate::model::Document;
use crate::render::empty_text_block_placeholder_string;
use crate::render::inline_atom_label;
use crate::render::inline_atom_mention_theme;
use crate::render::opaque_node_is_inline;
use crate::render::task_list_marker_metadata;
use crate::render::ListContext;
use crate::render::RenderElement;
use crate::render::RenderMark;
use crate::schema::{NodeRole, Schema};

fn render_marks(node: &crate::model::Node) -> Vec<RenderMark> {
    node.marks()
        .iter()
        .map(|mark| RenderMark {
            mark_type: mark.mark_type().to_string(),
            attrs: mark.attrs().clone(),
        })
        .collect()
}

/// Result of an incremental re-render: a block index and its regenerated elements.
pub type BlockPatch = (usize, Vec<RenderElement>);

#[derive(Debug, Clone, PartialEq)]
pub struct RenderBlocksPatch {
    pub start_index: usize,
    pub delete_count: usize,
    pub blocks: Vec<Vec<RenderElement>>,
}

/// Re-generate render elements for only the affected top-level blocks.
///
/// `affected_indices` are 0-based indices into the document root's children
/// (i.e. the top-level block nodes). Only those blocks' RenderElement
/// subsequences are regenerated.
///
/// Returns a vec of `(block_index, elements)` pairs, sorted by block index.
pub fn incremental(doc: &Document, schema: &Schema, affected_indices: &[usize]) -> Vec<BlockPatch> {
    let affected: BTreeSet<usize> = affected_indices.iter().copied().collect();
    let root = doc.root();
    let mut results = Vec::new();

    // Walk top-level children to compute positions, but only generate elements
    // for affected blocks.
    let mut pos: u32 = 0;
    for i in 0..root.child_count() {
        let child = root.child(i).expect("child index in bounds");

        if affected.contains(&i) {
            let mut elements = Vec::new();
            let mut block_pos = pos;
            generate_block(child, schema, &mut elements, &mut block_pos, 0, None, i);
            results.push((i, elements));
        }

        // Advance position past this child regardless
        pos += child.node_size();
    }

    results
}

pub fn render_blocks(doc: &Document, schema: &Schema) -> Vec<Vec<RenderElement>> {
    let root = doc.root();
    if root.child_count() == 0 {
        return Vec::new();
    }
    let indices = (0..root.child_count()).collect::<Vec<_>>();
    incremental(doc, schema, &indices)
        .into_iter()
        .map(|(_, elements)| elements)
        .collect()
}

pub fn flatten_render_blocks(blocks: &[Vec<RenderElement>]) -> Vec<RenderElement> {
    let mut elements = Vec::new();
    for block in blocks {
        elements.extend(block.iter().cloned());
    }
    elements
}

pub fn contiguous_render_blocks_patch(
    old_doc: &Document,
    new_doc: &Document,
    schema: &Schema,
) -> Option<RenderBlocksPatch> {
    let old_blocks = render_blocks(old_doc, schema);
    let new_blocks = render_blocks(new_doc, schema);

    let mut prefix = 0usize;
    while prefix < old_blocks.len()
        && prefix < new_blocks.len()
        && old_blocks[prefix] == new_blocks[prefix]
    {
        prefix += 1;
    }

    if prefix == old_blocks.len() && prefix == new_blocks.len() {
        return None;
    }

    let mut old_suffix = old_blocks.len();
    let mut new_suffix = new_blocks.len();
    while old_suffix > prefix
        && new_suffix > prefix
        && old_blocks[old_suffix - 1] == new_blocks[new_suffix - 1]
    {
        old_suffix -= 1;
        new_suffix -= 1;
    }

    let start_index = if prefix > 0 { prefix - 1 } else { 0 };
    let old_end = if old_suffix < old_blocks.len() {
        old_suffix + 1
    } else {
        old_suffix
    };
    let new_end = if new_suffix < new_blocks.len() {
        new_suffix + 1
    } else {
        new_suffix
    };

    Some(RenderBlocksPatch {
        start_index,
        delete_count: old_end.saturating_sub(start_index),
        blocks: new_blocks[start_index..new_end].to_vec(),
    })
}

/// Derive a contiguous render patch and prove it reconstructs the complete
/// new render. Compiler hints may widen the exact rendered diff, never narrow
/// it. `Err(full)` is the safe fallback whenever the proof cannot be made.
pub fn safe_contiguous_render_blocks_patch(
    old_doc: &Document,
    new_doc: &Document,
    schema: &Schema,
    affected_indices: &[usize],
) -> Result<Option<RenderBlocksPatch>, Vec<Vec<RenderElement>>> {
    if old_doc == new_doc {
        return Ok(None);
    }

    let old_blocks = render_blocks(old_doc, schema);
    let new_blocks = render_blocks(new_doc, schema);
    let widest_len = old_blocks.len().max(new_blocks.len());
    if affected_indices.iter().any(|index| *index >= widest_len) {
        return Err(new_blocks);
    }

    let mut prefix = 0usize;
    while prefix < old_blocks.len()
        && prefix < new_blocks.len()
        && old_blocks[prefix] == new_blocks[prefix]
    {
        prefix += 1;
    }
    let mut old_end = old_blocks.len();
    let mut new_end = new_blocks.len();
    while old_end > prefix && new_end > prefix && old_blocks[old_end - 1] == new_blocks[new_end - 1]
    {
        old_end -= 1;
        new_end -= 1;
    }
    if prefix == old_blocks.len() && prefix == new_blocks.len() {
        return Err(new_blocks);
    }

    let mut start = prefix;
    for index in affected_indices {
        start = start.min(*index);
        if *index < old_blocks.len() {
            old_end = old_end.max(index.saturating_add(1));
        }
        if *index < new_blocks.len() {
            new_end = new_end.max(index.saturating_add(1));
        }
    }
    let patch = RenderBlocksPatch {
        start_index: start,
        delete_count: old_end.saturating_sub(start),
        blocks: new_blocks[start..new_end].to_vec(),
    };
    let mut reconstructed = old_blocks;
    let Some(end) = patch.start_index.checked_add(patch.delete_count) else {
        return Err(new_blocks);
    };
    if end > reconstructed.len() {
        return Err(new_blocks);
    }
    reconstructed.splice(patch.start_index..end, patch.blocks.clone());
    if reconstructed == new_blocks {
        Ok(Some(patch))
    } else {
        Err(new_blocks)
    }
}

/// Generate render elements for a single top-level block and its descendants.
/// This mirrors the logic in `generate::walk_children` but for a single node.
fn generate_block(
    node: &crate::model::Node,
    schema: &Schema,
    elements: &mut Vec<RenderElement>,
    pos: &mut u32,
    depth: u16,
    list_info: Option<(String, bool, u32, u32)>,
    child_index: usize,
) {
    let spec = schema.node(node.node_type());
    let role = spec.map(|s| &s.role);

    match role {
        Some(NodeRole::Text) => {
            let text = node.text_str().unwrap_or("").to_string();
            let marks = render_marks(node);
            elements.push(RenderElement::TextRun { text, marks });
            *pos += node.node_size();
        }
        Some(NodeRole::HardBreak) => {
            elements.push(RenderElement::VoidInline {
                node_type: node.node_type().to_string(),
                doc_pos: *pos,
                attrs: node.attrs().clone(),
            });
            *pos += node.node_size();
        }
        Some(NodeRole::List { ordered }) => {
            let ordered = *ordered;
            let start_attr = node
                .attrs()
                .get("start")
                .and_then(|v| v.as_u64())
                .unwrap_or(1) as u32;
            let total = node.child_count() as u32;

            *pos += 1; // list open tag
            for j in 0..node.child_count() {
                let item = node.child(j).expect("child index in bounds");
                generate_block(
                    item,
                    schema,
                    elements,
                    pos,
                    depth,
                    Some((node.node_type().to_string(), ordered, start_attr, total)),
                    j,
                );
            }
            *pos += 1; // list close tag
        }
        Some(NodeRole::ListItem) => {
            let list_context = list_info.map(|(list_node_type, ordered, start, total)| {
                let (kind, checked) = task_list_marker_metadata(&list_node_type, node);
                ListContext {
                    ordered,
                    index: if ordered {
                        start + child_index as u32
                    } else {
                        child_index as u32 + 1
                    },
                    total,
                    start,
                    is_first: child_index == 0,
                    is_last: child_index == (total as usize - 1),
                    kind,
                    checked,
                }
            });
            elements.push(RenderElement::BlockStart {
                node_type: node.node_type().to_string(),
                depth,
                list_context,
            });
            *pos += 1;
            for j in 0..node.child_count() {
                let child = node.child(j).expect("child index in bounds");
                generate_block(child, schema, elements, pos, depth + 1, None, j);
            }
            *pos += 1;
            elements.push(RenderElement::BlockEnd);
        }
        Some(NodeRole::TextBlock) => {
            elements.push(RenderElement::BlockStart {
                node_type: node.node_type().to_string(),
                depth,
                list_context: None,
            });
            *pos += 1;
            if node.child_count() == 0 {
                elements.push(RenderElement::TextRun {
                    text: empty_text_block_placeholder_string(),
                    marks: vec![],
                });
            } else {
                for j in 0..node.child_count() {
                    let child = node.child(j).expect("child index in bounds");
                    generate_block(child, schema, elements, pos, depth + 1, None, j);
                }
            }
            *pos += 1;
            elements.push(RenderElement::BlockEnd);
        }
        Some(NodeRole::Block) if node.is_void() => {
            elements.push(RenderElement::VoidBlock {
                node_type: node.node_type().to_string(),
                doc_pos: *pos,
                attrs: node.attrs().clone(),
            });
            *pos += node.node_size();
        }
        Some(NodeRole::Block) => {
            elements.push(RenderElement::BlockStart {
                node_type: node.node_type().to_string(),
                depth,
                list_context: None,
            });
            *pos += 1;
            for j in 0..node.child_count() {
                let child = node.child(j).expect("child index in bounds");
                generate_block(child, schema, elements, pos, depth + 1, None, j);
            }
            *pos += 1;
            elements.push(RenderElement::BlockEnd);
        }
        Some(NodeRole::Inline) if node.is_void() => {
            elements.push(RenderElement::OpaqueInlineAtom {
                node_type: node.node_type().to_string(),
                label: inline_atom_label(node.node_type(), node.attrs()),
                doc_pos: *pos,
                mention_theme: inline_atom_mention_theme(node.node_type(), node.attrs()),
            });
            *pos += node.node_size();
        }
        Some(NodeRole::Inline) => {
            *pos += node.node_size();
        }
        Some(NodeRole::Doc) => {
            *pos += 1;
            for j in 0..node.child_count() {
                let child = node.child(j).expect("child index in bounds");
                generate_block(child, schema, elements, pos, depth, None, j);
            }
            *pos += 1;
        }
        None => {
            if node.is_void() {
                let is_inline = opaque_node_is_inline(node, schema);
                if is_inline {
                    elements.push(RenderElement::OpaqueInlineAtom {
                        node_type: node.node_type().to_string(),
                        label: inline_atom_label(node.node_type(), node.attrs()),
                        doc_pos: *pos,
                        mention_theme: inline_atom_mention_theme(node.node_type(), node.attrs()),
                    });
                } else {
                    elements.push(RenderElement::OpaqueBlockAtom {
                        node_type: node.node_type().to_string(),
                        label: inline_atom_label(node.node_type(), node.attrs()),
                        doc_pos: *pos,
                    });
                }
                *pos += node.node_size();
            } else if node.is_text() {
                let text = node.text_str().unwrap_or("").to_string();
                let marks = render_marks(node);
                elements.push(RenderElement::TextRun { text, marks });
                *pos += node.node_size();
            } else {
                *pos += node.node_size();
            }
        }
    }
}
