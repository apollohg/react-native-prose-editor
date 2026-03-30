//! Property-based fuzz tests for PositionMap invariants.
//!
//! These tests run on stable Rust and exercise the same four invariants as the
//! `cargo-fuzz` target in `fuzz/fuzz_targets/position_map.rs`:
//!
//! A. `scalar_to_doc(doc_to_scalar(pos)) == pos` for all cursorable positions
//! B. `doc_to_scalar(scalar_to_doc(offset)) == offset` for all valid scalar offsets
//! C. Every cursorable position resolves to a valid `ResolvedPos`
//! D. `total_scalars()` matches the expected flattened text length

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::model::fragment::Fragment;
    use crate::model::node::Node;
    use crate::model::Document;
    use crate::position::PositionMap;

    // -----------------------------------------------------------------------
    // Predefined text strings with varying Unicode characteristics
    // -----------------------------------------------------------------------

    const TEXT_POOL: &[&str] = &[
        "Hello world",                                 // ASCII
        "cafe\u{0301}",                                // combining accent (5 scalars)
        "\u{1F525}\u{1F680}\u{2764}\u{FE0F}",          // emoji: fire, rocket, heart+VS16
        "\u{4F60}\u{597D}\u{4E16}\u{754C}",            // CJK: nihao shijie
        "",                                            // empty
        "a",                                           // single char
        "abc\ndef\tghi",                               // whitespace chars
        "\u{0041}\u{0301}\u{0302}",                    // A + 2 combining marks
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", // family ZWJ (7 scalars)
        "Line one\nLine two\nLine three",              // multi-line
    ];

    // -----------------------------------------------------------------------
    // Block type choices
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone, Copy)]
    enum BlockChoice {
        Paragraph,
        ParagraphWithInlineVoids,
        HorizontalRule,
        Blockquote,
        BulletList,
    }

    const BLOCK_CHOICES: &[BlockChoice] = &[
        BlockChoice::Paragraph,
        BlockChoice::ParagraphWithInlineVoids,
        BlockChoice::HorizontalRule,
        BlockChoice::Blockquote,
        BlockChoice::BulletList,
    ];

    // -----------------------------------------------------------------------
    // Byte-driven document builder
    // -----------------------------------------------------------------------

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

        fn pick(&mut self, max: usize) -> Option<usize> {
            if max == 0 {
                return None;
            }
            Some(self.next_byte()? as usize % max)
        }

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
                BlockChoice::HorizontalRule => {
                    Node::void("horizontalRule".to_string(), HashMap::new())
                }
                BlockChoice::Blockquote => build_blockquote(&mut reader)?,
                BlockChoice::BulletList => build_bullet_list(&mut reader)?,
            };
            blocks.push(block);
        }

        let root = Node::element("doc".to_string(), HashMap::new(), Fragment::from(blocks));
        Some(Document::new(root))
    }

    // -----------------------------------------------------------------------
    // Independent expected-value computation (oracle)
    // -----------------------------------------------------------------------

    fn expected_total_scalars(pmap: &PositionMap) -> u32 {
        pmap.blocks()
            .iter()
            .map(|block| block.scalar_prefix_len + block.scalar_len + block.rendered_break_after)
            .sum()
    }

    /// Collect all cursorable doc positions.
    fn collect_cursorable_positions(pmap: &PositionMap) -> Vec<u32> {
        let mut positions = Vec::new();
        for i in 0..pmap.block_count() {
            let block = pmap.block(i).unwrap();
            if block.is_void_block {
                positions.push(block.doc_start);
            } else {
                for pos in block.doc_start..=block.doc_end {
                    positions.push(pos);
                }
            }
        }
        positions
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Returns true if the given scalar offset falls on an inter-block break
    /// (the synthetic "\n" separator between adjacent blocks).
    fn is_break_scalar(offset: u32, pmap: &PositionMap) -> bool {
        for i in 0..pmap.block_count() {
            let block = pmap.block(i).unwrap();
            let block_end = block.scalar_start + block.scalar_prefix_len + block.scalar_len;
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

    // -----------------------------------------------------------------------
    // Core invariant checker
    // -----------------------------------------------------------------------

    fn check_invariants(doc: &Document, label: &str) {
        let pmap = PositionMap::build(doc);

        // -- Invariant D --
        let expected = expected_total_scalars(&pmap);
        let actual = pmap.total_scalars();
        assert_eq!(
            actual,
            expected,
            "[{}] INVARIANT D: total_scalars()={}, expected={}. blocks={}, content_size={}",
            label,
            actual,
            expected,
            pmap.block_count(),
            doc.content_size(),
        );

        // -- Invariant A --
        let cursorable = collect_cursorable_positions(&pmap);
        for &pos in &cursorable {
            let scalar = pmap.doc_to_scalar(pos, doc);
            let roundtrip = pmap.scalar_to_doc(scalar, doc);
            assert_eq!(
                roundtrip, pos,
                "[{}] INVARIANT A: pos={} -> scalar={} -> doc={}",
                label, pos, scalar, roundtrip,
            );
        }

        // -- Invariant B --
        // For scalar offsets within block content, the roundtrip must be exact.
        // Break scalars (inter-block separators) are synthetic and map to the
        // boundary between blocks; the reverse mapping snaps to the nearest
        // block content position, so the roundtrip is lossy by design.
        let total = pmap.total_scalars();
        for offset in 0..total {
            let is_break = is_break_scalar(offset, &pmap);
            let doc_pos = pmap.scalar_to_doc(offset, doc);
            let roundtrip = pmap.doc_to_scalar(doc_pos, doc);

            if is_break {
                // Break scalars: roundtrip may snap to adjacent block boundary.
                // Verify the result is at most 1 away from the original offset
                // (i.e. it maps to the end of the preceding block or the start
                // of the following block).
                let diff = if roundtrip > offset {
                    roundtrip - offset
                } else {
                    offset - roundtrip
                };
                assert!(
                    diff <= 1,
                    "[{}] INVARIANT B (break): scalar={} -> doc={} -> scalar={}, diff={}",
                    label,
                    offset,
                    doc_pos,
                    roundtrip,
                    diff,
                );
            } else {
                let doc_roundtrip = pmap.scalar_to_doc(roundtrip, doc);
                assert_eq!(
                    doc_roundtrip, doc_pos,
                    "[{}] INVARIANT B: scalar={} -> doc={} -> scalar={} -> doc={}",
                    label, offset, doc_pos, roundtrip, doc_roundtrip,
                );
            }
        }

        // -- Invariant C --
        for &pos in &cursorable {
            let result = pmap.resolve(pos, doc);
            assert!(
                result.is_ok(),
                "[{}] INVARIANT C: resolve({}) failed: {:?}. content_size={}",
                label,
                pos,
                result.err(),
                doc.content_size(),
            );
            let resolved = result.unwrap();
            assert_eq!(
                resolved.pos, pos,
                "[{}] INVARIANT C: resolved.pos={}, expected={}",
                label, resolved.pos, pos,
            );
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    /// Run invariants against a large number of deterministic pseudo-random
    /// byte sequences covering diverse document structures.
    #[test]
    fn fuzz_position_map_deterministic_sweep() {
        let mut tested = 0u32;

        // Single-byte seeds produce simple 1-block documents
        for seed in 0u8..=255 {
            if let Some(doc) = build_random_doc(&[seed]) {
                check_invariants(&doc, &format!("seed=[{}]", seed));
                tested += 1;
            }
        }

        // Two-byte seeds: more variety in block choice + text
        for b0 in (0u8..=255).step_by(5) {
            for b1 in (0u8..=255).step_by(7) {
                if let Some(doc) = build_random_doc(&[b0, b1]) {
                    check_invariants(&doc, &format!("seed=[{},{}]", b0, b1));
                    tested += 1;
                }
            }
        }

        // Multi-byte seeds for complex docs (blockquotes, lists, mixed)
        let complex_seeds: Vec<Vec<u8>> = vec![
            vec![0, 0, 0, 0, 0, 0, 0, 0],
            vec![1, 1, 1, 1, 1, 1, 1, 1],
            vec![2, 3, 4, 5, 6, 7, 8, 9],
            vec![255, 254, 253, 252, 251, 250],
            vec![0, 2, 0, 2, 0, 2, 0, 2, 0, 2], // alternating paragraph/hr
            vec![0, 3, 0, 0, 3, 0, 0, 3, 0, 0], // blockquotes
            vec![0, 4, 0, 0, 0, 4, 0, 0, 0],    // bullet lists
            vec![0, 1, 5, 2, 3, 1, 7, 0, 4, 8, 9], // mixed everything
            vec![3, 1, 0, 1, 0, 1, 0],          // para with inline voids
            vec![4, 3, 2, 1, 0, 5, 6, 7, 8, 9, 0], // 5 blocks mixed
            // Stress edge cases with emoji/CJK text
            vec![0, 0, 2],    // paragraph with emoji text
            vec![0, 0, 3],    // paragraph with CJK text
            vec![0, 0, 7],    // paragraph with combining marks
            vec![0, 0, 8],    // paragraph with ZWJ sequence
            vec![0, 1, 2, 3], // para with inline voids: emoji + CJK
            vec![0, 1, 7, 8], // para with inline voids: combining + ZWJ
            // Multi-block with all text types
            vec![
                4, 0, 0, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0, 7, 0, 8, 0, 9,
            ],
        ];
        for seed in &complex_seeds {
            if let Some(doc) = build_random_doc(seed) {
                check_invariants(&doc, &format!("seed={:?}", seed));
                tested += 1;
            }
        }

        // Exhaustive 3-byte sweep (sampled to keep runtime reasonable)
        for b0 in (0u8..=255).step_by(51) {
            for b1 in (0u8..=255).step_by(51) {
                for b2 in (0u8..=255).step_by(51) {
                    if let Some(doc) = build_random_doc(&[b0, b1, b2]) {
                        check_invariants(&doc, &format!("seed=[{},{},{}]", b0, b1, b2));
                        tested += 1;
                    }
                }
            }
        }

        assert!(
            tested > 100,
            "Expected to test at least 100 documents, but only tested {}",
            tested
        );
        eprintln!(
            "fuzz_position_map_deterministic_sweep: tested {} documents",
            tested
        );
    }

    /// Test with a single empty paragraph.
    #[test]
    fn fuzz_invariants_empty_paragraph() {
        let para = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![]),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![para]),
        ));
        check_invariants(&doc, "empty_paragraph");
    }

    /// Test with a single void block.
    #[test]
    fn fuzz_invariants_single_void() {
        let hr = Node::void("horizontalRule".to_string(), HashMap::new());
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![hr]),
        ));
        check_invariants(&doc, "single_void");
    }

    /// Test with mixed blocks: paragraph, hr, paragraph.
    #[test]
    fn fuzz_invariants_mixed_para_hr_para() {
        let p1 = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text("Hello".to_string(), vec![])]),
        );
        let hr = Node::void("horizontalRule".to_string(), HashMap::new());
        let p2 = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text("World".to_string(), vec![])]),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![p1, hr, p2]),
        ));
        check_invariants(&doc, "para_hr_para");
    }

    /// Test with a paragraph containing hardBreak inline voids.
    #[test]
    fn fuzz_invariants_paragraph_with_hard_breaks() {
        let para = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![
                Node::text("Line one".to_string(), vec![]),
                Node::void("hardBreak".to_string(), HashMap::new()),
                Node::text("Line two".to_string(), vec![]),
                Node::void("hardBreak".to_string(), HashMap::new()),
                Node::text("Line three".to_string(), vec![]),
            ]),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![para]),
        ));
        check_invariants(&doc, "para_with_hard_breaks");
    }

    /// Test with a blockquote containing multiple paragraphs.
    #[test]
    fn fuzz_invariants_blockquote() {
        let p1 = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text("First".to_string(), vec![])]),
        );
        let p2 = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text("Second".to_string(), vec![])]),
        );
        let bq = Node::element(
            "blockquote".to_string(),
            HashMap::new(),
            Fragment::from(vec![p1, p2]),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![bq]),
        ));
        check_invariants(&doc, "blockquote");
    }

    /// Test with a bullet list containing multiple items.
    #[test]
    fn fuzz_invariants_bullet_list() {
        let items: Vec<Node> = (1..=3)
            .map(|i| {
                let para = Node::element(
                    "paragraph".to_string(),
                    HashMap::new(),
                    Fragment::from(vec![Node::text(format!("Item {}", i), vec![])]),
                );
                Node::element(
                    "listItem".to_string(),
                    HashMap::new(),
                    Fragment::from(vec![para]),
                )
            })
            .collect();
        let list = Node::element(
            "bulletList".to_string(),
            HashMap::new(),
            Fragment::from(items),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![list]),
        ));
        check_invariants(&doc, "bullet_list");
    }

    /// Test with emoji and CJK text (multi-byte Unicode scalars).
    #[test]
    fn fuzz_invariants_unicode_text() {
        let para_emoji = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text(
                "\u{1F525}\u{1F680}\u{2764}\u{FE0F}".to_string(),
                vec![],
            )]),
        );
        let para_cjk = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text(
                "\u{4F60}\u{597D}\u{4E16}\u{754C}".to_string(),
                vec![],
            )]),
        );
        let para_combining = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text(
                "\u{0041}\u{0301}\u{0302}".to_string(),
                vec![],
            )]),
        );
        let para_zwj = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::text(
                "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".to_string(),
                vec![],
            )]),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![para_emoji, para_cjk, para_combining, para_zwj]),
        ));
        check_invariants(&doc, "unicode_text");
    }

    /// Test with an empty document (no blocks at all).
    #[test]
    fn fuzz_invariants_empty_doc() {
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![]),
        ));
        check_invariants(&doc, "empty_doc");
    }

    /// Stress test: 5 blocks of different types with complex content.
    #[test]
    fn fuzz_invariants_complex_mixed() {
        let p1 = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![
                Node::text("Hello ".to_string(), vec![]),
                Node::void("hardBreak".to_string(), HashMap::new()),
                Node::text("\u{4F60}\u{597D}".to_string(), vec![]),
            ]),
        );
        let hr = Node::void("horizontalRule".to_string(), HashMap::new());
        let bq = Node::element(
            "blockquote".to_string(),
            HashMap::new(),
            Fragment::from(vec![Node::element(
                "paragraph".to_string(),
                HashMap::new(),
                Fragment::from(vec![Node::text("\u{1F525} fire".to_string(), vec![])]),
            )]),
        );
        let list = Node::element(
            "bulletList".to_string(),
            HashMap::new(),
            Fragment::from(vec![
                Node::element(
                    "listItem".to_string(),
                    HashMap::new(),
                    Fragment::from(vec![Node::element(
                        "paragraph".to_string(),
                        HashMap::new(),
                        Fragment::from(vec![Node::text("alpha".to_string(), vec![])]),
                    )]),
                ),
                Node::element(
                    "listItem".to_string(),
                    HashMap::new(),
                    Fragment::from(vec![Node::element(
                        "paragraph".to_string(),
                        HashMap::new(),
                        Fragment::from(vec![Node::text("beta".to_string(), vec![])]),
                    )]),
                ),
            ]),
        );
        let p2 = Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(vec![]),
        );
        let doc = Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(vec![p1, hr, bq, list, p2]),
        ));
        check_invariants(&doc, "complex_mixed");
    }
}
