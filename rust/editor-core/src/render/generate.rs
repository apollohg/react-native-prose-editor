use crate::model::{Document, Node};
use crate::render::{
    empty_text_block_placeholder_string, inline_atom_label, ListContext, RenderElement,
    RenderMark,
};
use crate::schema::{NodeRole, Schema};

fn render_marks(node: &Node) -> Vec<RenderMark> {
    node.marks()
        .iter()
        .map(|mark| RenderMark {
            mark_type: mark.mark_type().to_string(),
            attrs: mark.attrs().clone(),
        })
        .collect()
}

/// Generate a complete flat sequence of `RenderElement` values from a document.
///
/// Walks the document tree depth-first, emitting BlockStart/BlockEnd pairs
/// around block-level nodes, TextRun for text, and VoidInline/VoidBlock for
/// atomic nodes. List nodes are transparent containers that provide
/// `ListContext` to their list-item children.
pub fn generate(doc: &Document, schema: &Schema) -> Vec<RenderElement> {
    let mut elements = Vec::new();
    // Position starts at 0 (inside the doc root's open tag).
    // The root doc node itself is not emitted; we walk its children.
    let root = doc.root();
    let mut pos: u32 = 0; // position within doc content (after root open tag)
    walk_children(root, schema, &mut elements, &mut pos, 0, None);
    elements
}

/// Walk the children of `parent`, emitting render elements.
///
/// `depth` is the nesting depth for BlockStart (0 = top-level blocks).
/// `list_info` is set when `parent` is a list node, carrying (ordered, start, total_items).
fn walk_children(
    parent: &Node,
    schema: &Schema,
    elements: &mut Vec<RenderElement>,
    pos: &mut u32,
    depth: u8,
    list_info: Option<(bool, u32, u32)>,
) {
    for i in 0..parent.child_count() {
        let child = parent.child(i).expect("child index in bounds");
        let spec = schema.node(child.node_type());
        let role = spec.map(|s| &s.role);

        match role {
            Some(NodeRole::Text) => {
                // Text node: emit TextRun
                let text = child.text_str().unwrap_or("").to_string();
                let marks = render_marks(child);
                elements.push(RenderElement::TextRun { text, marks });
                *pos += child.node_size();
            }
            Some(NodeRole::HardBreak) => {
                // Known inline void
                elements.push(RenderElement::VoidInline {
                    node_type: child.node_type().to_string(),
                    doc_pos: *pos,
                });
                *pos += child.node_size(); // 1 for void
            }
            Some(NodeRole::List { ordered }) => {
                // List is a transparent container. Walk its children (listItems)
                // providing list context. The list node's open tag consumes 1 token.
                let ordered = *ordered;
                let start_attr = child
                    .attrs()
                    .get("start")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                let total = child.child_count() as u32;

                *pos += 1; // list open tag
                walk_children(
                    child,
                    schema,
                    elements,
                    pos,
                    depth,
                    Some((ordered, start_attr, total)),
                );
                *pos += 1; // list close tag
            }
            Some(NodeRole::ListItem) => {
                // ListItem: emit BlockStart with ListContext, walk children, emit BlockEnd
                let list_context = list_info.map(|(ordered, start, total)| {
                    let index_0based = i as u32;
                    let index = if ordered {
                        start + index_0based
                    } else {
                        index_0based + 1
                    };
                    ListContext {
                        ordered,
                        index,
                        total,
                        start,
                        is_first: i == 0,
                        is_last: i == (total as usize - 1),
                    }
                });
                elements.push(RenderElement::BlockStart {
                    node_type: child.node_type().to_string(),
                    depth,
                    list_context,
                });
                *pos += 1; // listItem open tag
                walk_children(child, schema, elements, pos, depth + 1, None);
                *pos += 1; // listItem close tag
                elements.push(RenderElement::BlockEnd);
            }
            Some(NodeRole::TextBlock) => {
                // Paragraph or similar text block: BlockStart, walk inline children, BlockEnd
                elements.push(RenderElement::BlockStart {
                    node_type: child.node_type().to_string(),
                    depth,
                    list_context: None,
                });
                *pos += 1; // open tag
                if child.child_count() == 0 {
                    elements.push(RenderElement::TextRun {
                        text: empty_text_block_placeholder_string(),
                        marks: vec![],
                    });
                } else {
                    walk_children(child, schema, elements, pos, depth + 1, None);
                }
                *pos += 1; // close tag
                elements.push(RenderElement::BlockEnd);
            }
            Some(NodeRole::Block) if child.is_void() => {
                // Void block (e.g. horizontalRule)
                elements.push(RenderElement::VoidBlock {
                    node_type: child.node_type().to_string(),
                    doc_pos: *pos,
                });
                *pos += child.node_size(); // 1 for void
            }
            Some(NodeRole::Block) => {
                // Non-void block: treat as generic block container
                elements.push(RenderElement::BlockStart {
                    node_type: child.node_type().to_string(),
                    depth,
                    list_context: None,
                });
                *pos += 1; // open tag
                walk_children(child, schema, elements, pos, depth + 1, None);
                *pos += 1; // close tag
                elements.push(RenderElement::BlockEnd);
            }
            Some(NodeRole::Inline) if child.is_void() => {
                // Unknown inline void: opaque inline atom
                elements.push(RenderElement::OpaqueInlineAtom {
                    node_type: child.node_type().to_string(),
                    label: inline_atom_label(child.node_type(), child.attrs()),
                    doc_pos: *pos,
                });
                *pos += child.node_size();
            }
            Some(NodeRole::Inline) => {
                // Non-void inline (shouldn't normally happen but handle gracefully)
                *pos += child.node_size();
            }
            Some(NodeRole::Doc) => {
                // Nested doc (unusual): just walk children
                *pos += 1;
                walk_children(child, schema, elements, pos, depth, None);
                *pos += 1;
            }
            None => {
                // Unknown node type: use heuristics based on node kind
                if child.is_void() {
                    // Determine inline vs block by group
                    let is_inline = spec
                        .and_then(|s| s.group.as_deref())
                        .map(|g| g == "inline")
                        .unwrap_or(false);
                    if is_inline {
                        elements.push(RenderElement::OpaqueInlineAtom {
                            node_type: child.node_type().to_string(),
                            label: inline_atom_label(child.node_type(), child.attrs()),
                            doc_pos: *pos,
                        });
                    } else {
                        elements.push(RenderElement::OpaqueBlockAtom {
                            node_type: child.node_type().to_string(),
                            label: inline_atom_label(child.node_type(), child.attrs()),
                            doc_pos: *pos,
                        });
                    }
                    *pos += child.node_size();
                } else if child.is_text() {
                    // Text node not in schema (unusual): emit TextRun anyway
                    let text = child.text_str().unwrap_or("").to_string();
                    let marks = render_marks(child);
                    elements.push(RenderElement::TextRun { text, marks });
                    *pos += child.node_size();
                } else {
                    // Unknown element: skip its tokens
                    *pos += child.node_size();
                }
            }
        }
    }
}
