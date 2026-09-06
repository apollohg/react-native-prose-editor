#[test]
fn generated_structural_trees_bound_and_converge_for_256_fixed_seeds() {
    fn nested_doc(mut blocks: Vec<Value>, depth: usize) -> Value {
        for _ in 0..depth {
            blocks = vec![json!({ "type": "blockquote", "content": blocks })];
        }
        json!({ "type": "doc", "content": blocks })
    }

    let schema = tiptap_schema();
    for seed in 0usize..256 {
        let depth = (seed / 11) % 4;
        let (source, operations) = match seed % 11 {
            0 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }),
                    ],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::InsertNode {
                        at: point_for_test(2),
                        node: Node::void("hardBreak".into(), HashMap::new()),
                    }],
                )
            }
            1 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }),
                    ],
                    0,
                );
                (
                    source,
                    vec![TypedOperation::SplitBlock {
                        at: point_for_test(2),
                        node_type: "paragraph".into(),
                        attrs: HashMap::new(),
                    }],
                )
            }
            2 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] }),
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }),
                    ],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::JoinBlocks {
                        at: point_for_test(2),
                    }],
                )
            }
            3 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }),
                    ],
                    0,
                );
                (
                    source,
                    vec![TypedOperation::WrapInList {
                        range: range_for_test(0, 3),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    }],
                )
            }
            4 => {
                let source = nested_doc(
                    vec![json!({
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                        ]
                    })],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "two") + 1;
                (
                    source,
                    vec![TypedOperation::IndentListItem {
                        at: point_for_test(at),
                    }],
                )
            }
            5 => {
                let source = nested_doc(
                    vec![json!({
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [
                                { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                                { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] }] }
                            ]
                        }]
                    })],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "inner") + 1;
                (
                    source,
                    vec![TypedOperation::OutdentListItem {
                        at: point_for_test(at),
                    }],
                )
            }
            6 => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }] }),
                    ],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "one") + 1;
                (
                    source,
                    vec![TypedOperation::UnwrapFromList {
                        at: point_for_test(at),
                    }],
                )
            }
            7 => {
                let source = nested_doc(
                    vec![json!({ "type": "paragraph", "content": [{ "type": "hardBreak" }] })],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::DeleteRange {
                        range: range_for_test(0, 1),
                    }],
                )
            }
            8 => {
                let source = nested_doc(
                    vec![json!({ "type": "paragraph", "content": [{ "type": "hardBreak" }] })],
                    depth,
                );
                (
                    source,
                    vec![TypedOperation::ReplaceRange {
                        range: range_for_test(0, 1),
                        content: Fragment::from(vec![Node::text(format!("seed-{seed}"), vec![])]),
                    }],
                )
            }
            9 => {
                let source = json!({
                    "type": "doc",
                    "content": [{
                        "type": "image",
                        "attrs": { "src": format!("old-{seed}"), "alt": "old alt" }
                    }]
                });
                (
                    source,
                    vec![TypedOperation::UpdateNodeAttrs {
                        at: point_for_test(0),
                        attrs: HashMap::from([
                            ("src".into(), Value::String(format!("new-{seed}"))),
                            ("alt".into(), Value::String("new alt".into())),
                            ("title".into(), Value::Null),
                            ("width".into(), Value::Null),
                            ("height".into(), Value::Null),
                        ]),
                    }],
                )
            }
            _ => {
                let source = nested_doc(
                    vec![
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "A😀" }] }),
                        json!({ "type": "paragraph", "content": [{ "type": "text", "text": "B" }] }),
                    ],
                    depth,
                );
                let at = rendered_scalar_offset(&source, &schema, "B") - 1;
                (
                    source,
                    vec![TypedOperation::InsertNode {
                        at: point_for_test(at),
                        node: Node::void("horizontalRule".into(), HashMap::new()),
                    }],
                )
            }
        };
        let (actual, expected, _, update_len, estimate) = compile_and_execute(source, operations);
        assert_eq!(actual, expected, "fixed structural seed {seed}");
        assert!(update_len <= estimate, "fixed structural seed {seed}");
    }

    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "AB" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "sentinel" }] }
        ]
    });
    let (doc, schema, limits, mut compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertNode {
            at: point_for_test(1),
            node: Node::void("hardBreak".into(), HashMap::new()),
        }],
        tiptap_schema(),
    );
    let sentinel_id = {
        let txn = doc.transact();
        txn.get_xml_fragment("prosemirror")
            .unwrap()
            .get(&txn, 1)
            .unwrap()
            .id()
    };
    {
        let txn = doc.transact();
        let preflight =
            preflight_mutation_work_for_test(71, &compiled.mutation_plan, &txn).unwrap();
        let exact = compiled.mutation_plan.compilation_work_for_test() + preflight;
        compiled.mutation_plan.set_work_limit_for_test(exact);
        preflight_mutation_plan(71, &compiled.mutation_plan, &txn).unwrap();
        compiled.mutation_plan.set_work_limit_for_test(exact - 1);
        assert_eq!(
            preflight_mutation_plan(71, &compiled.mutation_plan, &txn)
                .unwrap_err()
                .code,
            "OPERATION_LIMIT_EXCEEDED"
        );
        compiled.mutation_plan.set_work_limit_for_test(exact);
    }
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(fragment.get(&txn, 1).unwrap().id(), sentinel_id);
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn estimated_update_growth_bounds_supported_action_mixes(
        generated in prop::collection::vec((0u8..6, "[a-z]{1,3}"), 1..8)
    ) {
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcdef" }]
            }]
        });
        let operations = generated
            .into_iter()
            .map(|(kind, text)| match kind {
                0 => TypedOperation::InsertText {
                    at: point_for_test(2),
                    text,
                    marks: vec![],
                },
                1 => TypedOperation::DeleteRange {
                    range: range_for_test(2, 3),
                },
                2 => TypedOperation::ReplaceRange {
                    range: range_for_test(2, 3),
                    content: Fragment::from(vec![Node::text(text, vec![])]),
                },
                3 => TypedOperation::AddMark {
                    range: range_for_test(1, 4),
                    mark: Mark::new("bold".into(), HashMap::new()),
                },
                4 => TypedOperation::RemoveMark {
                    range: range_for_test(1, 4),
                    mark_type: "bold".into(),
                },
                _ => TypedOperation::ReplaceMark {
                    range: range_for_test(1, 4),
                    mark: Mark::new(
                        "link".into(),
                        HashMap::from([("href".into(), Value::String(text))]),
                    ),
                },
            })
            .collect();
        compile_and_execute(source, operations);
    }
}
