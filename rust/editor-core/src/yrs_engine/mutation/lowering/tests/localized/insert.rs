#[test]
fn localized_existing_textblock_insert_matches_eager_action_signature_and_work() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abc" }]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let position_map = PositionMap::build(&document, &schema);
    let block = position_map.block(0).unwrap();
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();

    let mut eager =
        MutationCompiler::new(701, &txn, &fragment, &schema, 100_000, 100_000, 7).unwrap();
    let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
        701,
        &txn,
        &fragment,
        &schema,
        100_000,
        100_000,
        7,
        LocalizedInsertLocator {
            document: &document,
            block_path: block.node_path.as_slice(),
            position: 2,
        },
    )
    .unwrap();
    assert_eq!(mode, MutationCompilerBuild::Localized);

    eager.insert(0, 2, "X", &[]).unwrap();
    localized.insert(0, 2, "X", &[]).unwrap();
    let eager = eager.finish(Some(0)).unwrap();
    let localized = localized.finish(Some(0)).unwrap();

    assert_eq!(eager.actions.len(), localized.actions.len());
    assert_eq!(
        eager
            .actions
            .iter()
            .map(action_signature)
            .collect::<Vec<_>>(),
        localized
            .actions
            .iter()
            .map(action_signature)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        eager.compilation_work_for_test(),
        localized.compilation_work_for_test()
    );
    assert_eq!(eager.scan_work, localized.scan_work);
}

#[test]
fn localized_ascii_and_non_bmp_start_middle_and_end_match_eager_utf16_indices() {
    let schema = tiptap_schema();
    for (text, cases) in [
        ("abc", vec![(0, 0), (1, 1), (3, 3)]),
        ("a😀b", vec![(0, 0), (1, 1), (2, 3), (3, 4)]),
    ] {
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": text }]
            }]
        });
        for (block_offset, expected_utf16) in cases {
            let (_doc, eager, localized, mode) =
                compile_pair_at_block_offset(&source, &schema, 0, block_offset, "Ω");
            assert_eq!(mode, MutationCompilerBuild::Localized);
            assert_insert_plans_equal(&eager, &localized);
            assert_eq!(
                action_signature(&localized.actions[0]).index_utf16,
                expected_utf16
            );
        }
    }
}

#[test]
fn localized_fragmented_xml_text_targets_match_eager_signature_and_work() {
    let schema = tiptap_schema();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "ab", "marks": [{ "type": "bold" }] },
                { "type": "text", "text": "😀c" },
                { "type": "text", "text": "de", "marks": [{ "type": "italic" }] }
            ]
        }]
    });
    let (_doc, eager, localized, mode) = compile_pair_at_block_offset(&source, &schema, 0, 3, "Z");
    assert_eq!(mode, MutationCompilerBuild::Localized);
    assert_plans_equal(&eager, &localized);
}

#[test]
fn localized_fragmented_mark_runs_in_one_xml_text_match_eager() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab😀cde" }]
        }]
    });
    let doc = seeded_document(&source, &schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph must be an XML element")
        };
        let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
            panic!("paragraph must contain XML text")
        };
        text.format(
            &mut txn,
            0,
            2,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
        text.format(
            &mut txn,
            4,
            3,
            Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let semantic_json = codec.read_json(&fragment, &txn).unwrap();
    let document =
        from_prosemirror_json(&semantic_json, &schema, UnknownTypeMode::Preserve).unwrap();
    let position_map = PositionMap::build(&document, &schema);
    let block = position_map.block(0).unwrap();
    let position = block.doc_start + 3;
    let mut eager =
        MutationCompiler::new(704, &txn, &fragment, &schema, 100_000, 100_000, 0).unwrap();
    let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
        704,
        &txn,
        &fragment,
        &schema,
        100_000,
        100_000,
        0,
        LocalizedInsertLocator {
            document: &document,
            block_path: block.node_path.as_slice(),
            position,
        },
    )
    .unwrap();
    assert_eq!(mode, MutationCompilerBuild::Localized);
    eager.insert(0, position, "Z", &[]).unwrap();
    localized.insert(0, position, "Z", &[]).unwrap();
    let eager = eager.finish(Some(0)).unwrap();
    let localized = localized.finish(Some(0)).unwrap();
    preflight_mutation_plan(704, &eager, &txn).unwrap();
    preflight_mutation_plan(704, &localized, &txn).unwrap();
    assert_insert_plans_equal(&eager, &localized);
    assert!(action_signature(&localized.actions[0]).signature.runs.len() >= 3);
}

#[test]
fn localized_nested_custom_list_textblock_matches_eager() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "taskList+", "role": "doc" },
            { "name": "taskList", "content": "taskItem+", "role": "list" },
            { "name": "taskItem", "content": "body", "role": "listItem" },
            { "name": "body", "content": "text*", "role": "textBlock" },
            { "name": "text", "content": "", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [{
                    "type": "body",
                    "content": [{ "type": "text", "text": "nested" }]
                }]
            }]
        }]
    });
    let (_doc, eager, localized, mode) = compile_pair_at_block_offset(&source, &schema, 0, 3, "!");
    assert_eq!(mode, MutationCompilerBuild::Localized);
    assert_insert_plans_equal(&eager, &localized);
}

#[test]
fn localized_empty_inline_void_and_cross_block_inputs_choose_eager_before_lowering() {
    let schema = tiptap_schema();
    let cases = [
        (
            json!({
                "type": "doc",
                "content": [{ "type": "paragraph" }]
            }),
            0,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "a" },
                        { "type": "hardBreak" },
                        { "type": "text", "text": "b" }
                    ]
                }]
            }),
            0,
            0,
        ),
        (
            json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "a" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
                ]
            }),
            0,
            3,
        ),
        (
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "ab", "marks": [{ "type": "bold" }] },
                        { "type": "text", "text": "cd" }
                    ]
                }]
            }),
            0,
            2,
        ),
    ];
    for (source, block_index, extra_position) in cases {
        let limits = ResourceLimits::default();
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
        let position_map = PositionMap::build(&document, &schema);
        let doc = seeded_document(&source, &schema, &limits);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let block = position_map.block(block_index).unwrap();
        let position = block.doc_start + extra_position;
        let (_compiler, mode) = MutationCompiler::new_localized_insert_or_eager(
            703,
            &txn,
            &fragment,
            &schema,
            100_000,
            100_000,
            0,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
        )
        .unwrap();
        assert_eq!(mode, MutationCompilerBuild::EagerFallback);
    }
}

#[test]
fn localized_and_eager_hit_the_same_logical_work_limit() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }
        ]
    });
    let (_baseline_doc, baseline, localized_baseline, mode) =
        compile_pair_at_block_offset(&source, &schema, 1, 1, "X");
    assert_eq!(mode, MutationCompilerBuild::Localized);
    assert_insert_plans_equal(&baseline, &localized_baseline);
    let limit = baseline.compilation_work_for_test().saturating_sub(1);

    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let position_map = PositionMap::build(&document, &schema);
    let block = position_map.block(1).unwrap();
    let position = block.doc_start + 1;
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mut eager =
        MutationCompiler::new(705, &txn, &fragment, &schema, limit, 100_000, 11).unwrap();
    let (mut localized, mode) = MutationCompiler::new_localized_insert_or_eager(
        705,
        &txn,
        &fragment,
        &schema,
        limit,
        100_000,
        11,
        LocalizedInsertLocator {
            document: &document,
            block_path: block.node_path.as_slice(),
            position,
        },
    )
    .unwrap();
    assert_eq!(mode, MutationCompilerBuild::Localized);
    let eager_error = eager.insert(0, position, "X", &[]).unwrap_err();
    let localized_error = localized.insert(0, position, "X", &[]).unwrap_err();
    assert_eq!(eager_error, localized_error);
}

#[test]
fn localized_plan_retains_eager_stale_and_foreign_document_guards() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "guarded" }]
        }]
    });
    let (doc, _eager, localized, mode) = compile_pair_at_block_offset(&source, &schema, 0, 2, "!");
    assert_eq!(mode, MutationCompilerBuild::Localized);
    let foreign = seeded_document(&source, &schema, &limits);
    let foreign_txn = foreign.transact();
    let foreign_error = preflight_mutation_plan(702, &localized, &foreign_txn).unwrap_err();
    assert_eq!(foreign_error.code, "ENGINE_INVARIANT_FAILED");

    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph must be an XML element")
        };
        let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
            panic!("paragraph must contain XML text")
        };
        text.insert(&mut txn, 0, "stale");
    }
    let stale_txn = doc.transact();
    let stale_error = preflight_mutation_plan(702, &localized, &stale_txn).unwrap_err();
    assert_eq!(stale_error.code, "ENGINE_INVARIANT_FAILED");
}

#[test]
fn seeded_localized_insert_is_restricted_and_matches_eager_without_eager_rebuild() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a😀b" }]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let position_map = PositionMap::build(&document, &schema);
    let block = position_map.block(0).unwrap();
    let position = block.doc_start + 2;
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        706,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        9,
        4,
    )
    .unwrap();
    let localized = LocalizedInsertCompiler::try_new(
        706,
        &txn,
        &fragment,
        &schema,
        100_000,
        100_000,
        11,
        LocalizedInsertLocator {
            document: &document,
            block_path: block.node_path.as_slice(),
            position,
        },
        &seed,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        9,
        4,
    )
    .unwrap()
    .expect("existing text insert must localize");
    let localized = localized.compile(0, position, "Ω", &[]).unwrap();

    let mut eager =
        MutationCompiler::new(706, &txn, &fragment, &schema, 100_000, 100_000, 11).unwrap();
    eager.insert(0, position, "Ω", &[]).unwrap();
    let eager = eager.finish(Some(0)).unwrap();
    assert_insert_plans_equal(&eager, &localized);
}

#[test]
fn seeded_marked_non_bmp_insert_preserves_exact_action_and_input_ceilings() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a😀b" }]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let position = block.doc_start + 2;
    let marks = vec![Mark::new("bold".into(), HashMap::new())];
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        707,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        3,
        2,
    )
    .unwrap();

    let compile_eager = |action_limit, scan_limit| {
        let mut compiler =
            MutationCompiler::new(707, &txn, &fragment, &schema, action_limit, scan_limit, 11)?;
        compiler.insert(0, position, "🦀", &marks)?;
        compiler.finish(Some(0))
    };
    let compile_localized = |action_limit, scan_limit| {
        LocalizedInsertCompiler::try_new(
            707,
            &txn,
            &fragment,
            &schema,
            action_limit,
            scan_limit,
            11,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position,
            },
            &seed,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            3,
            2,
        )?
        .expect("marked existing-text insert must localize")
        .compile(0, position, "🦀", &marks)
    };

    let baseline = compile_eager(100_000, 100_000).unwrap();
    let exact_actions = baseline.compilation_work_for_test();
    let exact_input = baseline.scan_work;
    let localized = compile_localized(exact_actions, exact_input).unwrap();
    assert_insert_plans_equal(&baseline, &localized);
    assert!(!action_signature(&localized.actions[0]).attrs.is_empty());

    for (action_limit, scan_limit) in [
        (exact_actions.saturating_sub(1), exact_input),
        (exact_actions, exact_input.saturating_sub(1)),
    ] {
        assert_eq!(
            compile_eager(action_limit, scan_limit).unwrap_err(),
            compile_localized(action_limit, scan_limit).unwrap_err()
        );
    }
}
