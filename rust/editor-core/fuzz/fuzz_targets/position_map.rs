//! Fuzz target for PositionMap invariants.
//!
//! # Requirements
//! - `cargo-fuzz` (install with `cargo install cargo-fuzz`)
//! - Nightly Rust toolchain (for libfuzzer instrumentation)
//!
//! # Running
//! ```sh
//! cd rust/editor-core
//! cargo +nightly fuzz run position_map -- -max_total_time=30
//! ```
//!
//! # Fallback (stable Rust)
//! The same invariant checks are available as standard `#[test]` property
//! tests in `rust/editor-core/src/position/fuzz_tests.rs` which run on
//! stable Rust without requiring cargo-fuzz or nightly.

#![no_main]

use libfuzzer_sys::fuzz_target;

use std::collections::HashMap;

use editor_core::model::fragment::Fragment;
use editor_core::model::node::Node;
use editor_core::model::Document;
use editor_core::position::PositionMap;

// ---------------------------------------------------------------------------
// Predefined text strings with varying Unicode characteristics
// ---------------------------------------------------------------------------

const TEXT_POOL: &[&str] = &[
    "Hello world",                         // ASCII
    "cafe\u{0301}",                        // combining accent (5 scalars)
    "\u{1F525}\u{1F680}\u{2764}\u{FE0F}", // emoji: fire, rocket, heart+VS16
    "\u{4F60}\u{597D}\u{4E16}\u{754C}",   // CJK: nihao shijie
    "",                                     // empty
    "a",                                    // single char
    "abc\ndef\tghi",                        // whitespace chars
    "\u{0041}\u{0301}\u{0302}",            // A + 2 combining marks
    "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", // family ZWJ sequence (7 scalars)
    "Line one\nLine two\nLine three",      // multi-line
];

// ---------------------------------------------------------------------------
// Block type choices
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum BlockChoice {
    /// A paragraph (text block), possibly empty.
    Paragraph,
    /// A paragraph containing inline void nodes (hardBreak) mixed with text.
    ParagraphWithInlineVoids,
    /// A block-level void node (horizontalRule).
    HorizontalRule,
    /// A blockquote containing one or more paragraphs.
    Blockquote,
    /// A bullet list with list items.
    BulletList,
}

const BLOCK_CHOICES: &[BlockChoice] = &[
    BlockChoice::Paragraph,
    BlockChoice::ParagraphWithInlineVoids,
    BlockChoice::HorizontalRule,
    BlockChoice::Blockquote,
    BlockChoice::BulletList,
];

// ---------------------------------------------------------------------------
// Byte-driven document builder
// ---------------------------------------------------------------------------

/// Consumes bytes from `data` to deterministically build a document.
/// Returns `None` if there aren't enough bytes.
struct ByteReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn next_byte(&mut self) -> Option<u8> {
        if self.pos < self.data.len() {
            let b = self.data[self.pos];
            self.pos += 1;
            Some(b)
        } else {
            None
        }
    }

    /// Pick an index in `0..max` (returns None if no bytes left or max == 0).
    fn pick(&mut self, max: usize) -> Option<usize> {
        if max == 0 {
            return None;
        }
        Some(self.next_byte()? as usize % max)
    }

    /// Pick a value in `lo..=hi`.
    fn pick_range(&mut self, lo: usize, hi: usize) -> Option<usize> {
        if hi < lo {
            return None;
        }
        Some(lo + self.next_byte()? as usize % (hi - lo + 1))
    }
}

fn build_paragraph(reader: &mut ByteReader) -> Option<Node> {
    let text_idx = reader.pick(TEXT_POOL.len())?;
    let text = TEXT_POOL[text_idx];
    let children = if text.is_empty() {
        vec![]
    } else {
        vec![Node::text(text.to_string(), vec![])]
    };
    Some(Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    ))
}

fn build_paragraph_with_inline_voids(reader: &mut ByteReader) -> Option<Node> {
    // Build: text + hardBreak + text (optionally)
    let text1_idx = reader.pick(TEXT_POOL.len())?;
    let text2_idx = reader.pick(TEXT_POOL.len())?;
    let text1 = TEXT_POOL[text1_idx];
    let text2 = TEXT_POOL[text2_idx];

    let mut children: Vec<Node> = Vec::new();
    if !text1.is_empty() {
        children.push(Node::text(text1.to_string(), vec![]));
    }
    children.push(Node::void("hardBreak".to_string(), HashMap::new()));
    if !text2.is_empty() {
        children.push(Node::text(text2.to_string(), vec![]));
    }
    Some(Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    ))
}

fn build_horizontal_rule() -> Node {
    Node::void("horizontalRule".to_string(), HashMap::new())
}

fn build_blockquote(reader: &mut ByteReader) -> Option<Node> {
    let para_count = reader.pick_range(1, 3)?;
    let mut paras = Vec::new();
    for _ in 0..para_count {
        paras.push(build_paragraph(reader)?);
    }
    Some(Node::element(
        "blockquote".to_string(),
        HashMap::new(),
        Fragment::from(paras),
    ))
}

fn build_list_item(reader: &mut ByteReader) -> Option<Node> {
    let para = build_paragraph(reader)?;
    Some(Node::element(
        "listItem".to_string(),
        HashMap::new(),
        Fragment::from(vec![para]),
    ))
}

fn build_bullet_list(reader: &mut ByteReader) -> Option<Node> {
    let item_count = reader.pick_range(1, 4)?;
    let mut items = Vec::new();
    for _ in 0..item_count {
        items.push(build_list_item(reader)?);
    }
    Some(Node::element(
        "bulletList".to_string(),
        HashMap::new(),
        Fragment::from(items),
    ))
}

fn build_random_doc(data: &[u8]) -> Option<Document> {
    let mut reader = ByteReader::new(data);

    let block_count = reader.pick_range(1, 5)?;
    let mut blocks = Vec::new();

    for _ in 0..block_count {
        let choice_idx = reader.pick(BLOCK_CHOICES.len())?;
        let block = match BLOCK_CHOICES[choice_idx] {
            BlockChoice::Paragraph => build_paragraph(&mut reader)?,
            BlockChoice::ParagraphWithInlineVoids => {
                build_paragraph_with_inline_voids(&mut reader)?
            }
            BlockChoice::HorizontalRule => build_horizontal_rule(),
            BlockChoice::Blockquote => build_blockquote(&mut reader)?,
            BlockChoice::BulletList => build_bullet_list(&mut reader)?,
        };
        blocks.push(block);
    }

    let root = Node::element("doc".to_string(), HashMap::new(), Fragment::from(blocks));
    Some(Document::new(root))
}

// ---------------------------------------------------------------------------
// Invariant checks
// ---------------------------------------------------------------------------

/// Compute the expected total scalar count by walking the document tree.
///
/// This mirrors what the rendered text view would show:
/// - Each text block contributes its text scalars (Unicode scalar count)
///   plus inline void placeholders (1 each)
/// - Each block-level void contributes 1 scalar (placeholder)
/// - Adjacent blocks are separated by 1 break scalar (except the last)
fn expected_total_scalars(doc: &Document) -> u32 {
    let mut block_scalar_counts: Vec<u32> = Vec::new();
    collect_block_scalars(doc.root(), &mut block_scalar_counts);

    if block_scalar_counts.is_empty() {
        return 0;
    }

    let content_scalars: u32 = block_scalar_counts.iter().sum();
    let break_scalars = (block_scalar_counts.len() as u32).saturating_sub(1);
    content_scalars + break_scalars
}

/// Recursively collect scalar counts for each renderable block.
fn collect_block_scalars(node: &Node, counts: &mut Vec<u32>) {
    if node.is_text() {
        return;
    }
    if node.is_void() {
        // Block-level void: 1 placeholder scalar
        counts.push(1);
        return;
    }

    // Element node
    let content = match node.content() {
        Some(c) => c,
        None => return,
    };

    if is_text_block(node) {
        // Sum inline content: text scalars + 1 per inline void
        let mut scalars = 0u32;
        for child in content.iter() {
            if child.is_text() {
                scalars += child.node_size();
            } else if child.is_void() {
                scalars += 1;
            }
        }
        counts.push(scalars);
        return;
    }

    // Container: recurse
    for child in content.iter() {
        collect_block_scalars(child, counts);
    }
}

/// Check if a node is a text block (all children are inline).
fn is_text_block(node: &Node) -> bool {
    let content = match node.content() {
        Some(c) => c,
        None => return false,
    };
    if content.child_count() == 0 {
        return true;
    }
    content.iter().all(|child| child.is_text() || child.is_void())
}

/// Returns true if the given scalar offset falls on an inter-block break
/// (the synthetic separator between adjacent blocks).
fn is_break_scalar(offset: u32, pmap: &PositionMap) -> bool {
    for i in 0..pmap.block_count() {
        let block = pmap.block(i).unwrap();
        let block_end = block.scalar_start + block.scalar_len;
        if block.rendered_break_after > 0 {
            let break_start = block_end;
            let break_end = block_end + block.rendered_break_after;
            if offset >= break_start && offset < break_end {
                return true;
            }
        }
    }
    false
}

/// Collect all cursorable doc positions (positions inside text block content
/// ranges, including start and end boundaries).
fn collect_cursorable_positions(pmap: &PositionMap) -> Vec<u32> {
    let mut positions = Vec::new();
    for i in 0..pmap.block_count() {
        let block = pmap.block(i).unwrap();
        if block.doc_start == block.doc_end {
            // Void block: the single position is cursorable
            positions.push(block.doc_start);
        } else {
            // Text block: every position from doc_start..=doc_end is cursorable
            for pos in block.doc_start..=block.doc_end {
                positions.push(pos);
            }
        }
    }
    positions
}

fn check_invariants(doc: &Document) {
    let pmap = PositionMap::build(doc);

    // -- Invariant D: total_scalars matches expected flattened text length --
    let expected = expected_total_scalars(doc);
    let actual = pmap.total_scalars();
    assert_eq!(
        actual, expected,
        "INVARIANT D FAILED: total_scalars() = {}, expected {}.\n\
         Block count: {}\nDoc content_size: {}",
        actual,
        expected,
        pmap.block_count(),
        doc.content_size(),
    );

    // -- Invariant A: scalar_to_doc(doc_to_scalar(pos)) == pos --
    // For all cursorable positions.
    let cursorable = collect_cursorable_positions(&pmap);
    for &pos in &cursorable {
        let scalar = pmap.doc_to_scalar(pos, doc);
        let roundtrip = pmap.scalar_to_doc(scalar, doc);
        assert_eq!(
            roundtrip, pos,
            "INVARIANT A FAILED: pos={}, doc_to_scalar=>{}, scalar_to_doc=>{}.\n\
             Expected roundtrip to return original pos.",
            pos, scalar, roundtrip,
        );
    }

    // -- Invariant B: doc_to_scalar(scalar_to_doc(offset)) == offset --
    // For scalar offsets within block content ranges. Break scalars
    // (inter-block separators) are synthetic and their roundtrip is lossy
    // by design -- they map to a structural doc position that snaps to
    // the nearest block boundary.
    let total = pmap.total_scalars();
    for offset in 0..total {
        let on_break = is_break_scalar(offset, &pmap);
        let doc_pos = pmap.scalar_to_doc(offset, doc);
        let roundtrip = pmap.doc_to_scalar(doc_pos, doc);

        if on_break {
            // Break scalars: roundtrip may snap to adjacent block boundary.
            let diff = if roundtrip > offset {
                roundtrip - offset
            } else {
                offset - roundtrip
            };
            assert!(
                diff <= 1,
                "INVARIANT B (break) FAILED: scalar={}, scalar_to_doc=>{}, doc_to_scalar=>{}, diff={}",
                offset, doc_pos, roundtrip, diff,
            );
        } else {
            assert_eq!(
                roundtrip, offset,
                "INVARIANT B FAILED: scalar={}, scalar_to_doc=>{}, doc_to_scalar=>{}",
                offset, doc_pos, roundtrip,
            );
        }
    }

    // -- Invariant C: every cursorable position resolves to a valid ResolvedPos --
    for &pos in &cursorable {
        let result = pmap.resolve(pos, doc);
        assert!(
            result.is_ok(),
            "INVARIANT C FAILED: resolve({}) returned error: {:?}.\n\
             Doc content_size: {}",
            pos,
            result.err(),
            doc.content_size(),
        );
        let resolved = result.unwrap();
        assert_eq!(
            resolved.pos, pos,
            "INVARIANT C FAILED: resolved.pos={} but expected {}",
            resolved.pos, pos,
        );
    }
}

// ---------------------------------------------------------------------------
// Fuzz target entry point
// ---------------------------------------------------------------------------

fuzz_target!(|data: &[u8]| {
    if let Some(doc) = build_random_doc(data) {
        check_invariants(&doc);
    }
});
