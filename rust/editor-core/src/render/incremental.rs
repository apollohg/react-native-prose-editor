use std::collections::BTreeSet;

use crate::model::Document;
use crate::render::empty_text_block_placeholder_string;
use crate::render::inline_atom_label;
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
    let old_children = old_doc
        .root()
        .content()
        .map(|content| content.children())
        .unwrap_or(&[]);
    let new_children = new_doc
        .root()
        .content()
        .map(|content| content.children())
        .unwrap_or(&[]);

    let mut prefix = 0usize;
    while prefix < old_children.len()
        && prefix < new_children.len()
        && old_children[prefix] == new_children[prefix]
    {
        prefix += 1;
    }

    if prefix == old_children.len() && prefix == new_children.len() {
        return None;
    }

    let mut old_suffix = old_children.len();
    let mut new_suffix = new_children.len();
    while old_suffix > prefix
        && new_suffix > prefix
        && old_children[old_suffix - 1] == new_children[new_suffix - 1]
    {
        old_suffix -= 1;
        new_suffix -= 1;
    }

    let start_index = if prefix > 0 { prefix - 1 } else { 0 };
    let old_end = if old_suffix < old_children.len() {
        old_suffix + 1
    } else {
        old_suffix
    };
    let new_end = if new_suffix < new_children.len() {
        new_suffix + 1
    } else {
        new_suffix
    };

    let affected_indices = (start_index..new_end).collect::<Vec<_>>();
    let blocks = incremental(new_doc, schema, &affected_indices)
        .into_iter()
        .map(|(_, elements)| elements)
        .collect::<Vec<_>>();

    Some(RenderBlocksPatch {
        start_index,
        delete_count: old_end.saturating_sub(start_index),
        blocks,
    })
}

/// Generate render elements for a single top-level block and its descendants.
/// This mirrors the logic in `generate::walk_children` but for a single node.
fn generate_block(
    node: &crate::model::Node,
    schema: &Schema,
    elements: &mut Vec<RenderElement>,
    pos: &mut u32,
    depth: u8,
    list_info: Option<(bool, u32, u32)>,
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
                    Some((ordered, start_attr, total)),
                    j,
                );
            }
            *pos += 1; // list close tag
        }
        Some(NodeRole::ListItem) => {
            let list_context = list_info.map(|(ordered, start, total)| super::ListContext {
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
                let is_inline = spec
                    .and_then(|s| s.group.as_deref())
                    .map(|g| g == "inline")
                    .unwrap_or(false);
                if is_inline {
                    elements.push(RenderElement::OpaqueInlineAtom {
                        node_type: node.node_type().to_string(),
                        label: inline_atom_label(node.node_type(), node.attrs()),
                        doc_pos: *pos,
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
