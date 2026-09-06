use super::*;

fn engine() -> YrsDocumentEngine {
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: None,
    })
    .unwrap();
    engine
        .import_json(TWO_PARAGRAPHS, TransactionOrigin::DocumentImport)
        .unwrap();
    engine
}

#[test]
fn one_character_insert_compiles_a_direct_mutation_action() {
    let engine = engine();
    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 51,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "!".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();

    assert_eq!(compiled.preview.root().text_content(), "a!lphaomega");
    assert_eq!(compiled.mutation_plan.actions.len(), 1);
    assert!(compiled.encoded_growth_bound > 0);
}

#[test]
fn every_text_and_mark_operation_executes_to_its_exact_preview() {
    let plain = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let bold = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Hello",
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let link = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Hello",
                "marks": [{ "type": "link", "attrs": { "href": "old" } }]
            }]
        }]
    });
    let cases = vec![
        (
            plain.clone(),
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "🙂".into(),
                marks: vec![Mark::new("bold".into(), HashMap::new())],
            },
        ),
        (
            plain.clone(),
            TypedOperation::DeleteRange {
                range: range_for_test(1, 4),
            },
        ),
        (
            plain.clone(),
            TypedOperation::ReplaceRange {
                range: range_for_test(1, 4),
                content: Fragment::from(vec![Node::text(
                    "XY".into(),
                    vec![Mark::new("italic".into(), HashMap::new())],
                )]),
            },
        ),
        (
            plain.clone(),
            TypedOperation::AddMark {
                range: range_for_test(1, 4),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ),
        (
            bold,
            TypedOperation::RemoveMark {
                range: range_for_test(1, 4),
                mark_type: "bold".into(),
            },
        ),
        (
            link,
            TypedOperation::ReplaceMark {
                range: range_for_test(1, 4),
                mark: Mark::new(
                    "link".into(),
                    HashMap::from([("href".into(), Value::String("new".into()))]),
                ),
            },
        ),
    ];

    for (source, operation) in cases {
        compile_and_execute(source, vec![operation]);
    }
}

#[test]
fn empty_textblocks_create_direct_text_targets_for_insert_and_collapsed_replace() {
    let empty = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let inserted = compile_and_execute(
        empty.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "hello".into(),
            marks: vec![Mark::new("bold".into(), HashMap::new())],
        }],
    );
    assert_eq!(inserted.0["content"][0]["content"][0]["text"], "hello");

    let replaced = compile_and_execute(
        empty,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(1, 1),
            content: Fragment::from(vec![Node::text("world".into(), vec![])]),
        }],
    );
    assert_eq!(replaced.0["content"][0]["content"][0]["text"], "world");
}

#[test]
fn created_text_target_executes_multi_piece_replacement_and_follow_up_edits() {
    let empty = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let pieces = Fragment::from(vec![
        Node::text("ab".into(), vec![Mark::new("bold".into(), HashMap::new())]),
        Node::text(
            "cd".into(),
            vec![Mark::new("italic".into(), HashMap::new())],
        ),
    ]);
    let actual = compile_and_execute(
        empty,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(1, 1),
            content: pieces,
        }],
    );
    assert_eq!(actual.0["content"][0]["content"][0]["text"], "ab");
    assert_eq!(actual.0["content"][0]["content"][1]["text"], "cd");

    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let (doc, schema, limits, _editing_limits, document) = diagnostic_doc(&source);
    let plan = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            83,
            &txn,
            &fragment,
            &schema,
            1_000,
            limits.max_input_bytes,
            0,
        )
        .unwrap();
        compiler
            .insert(0, 1, "abcd", &[])
            .and_then(|_| compiler.insert(1, 3, "X", &[]))
            .and_then(|_| compiler.delete(2, 2, 3, &[]))
            .and_then(|_| {
                compiler.format(
                    3,
                    1,
                    5,
                    &[1, 5],
                    super::mutation::mark_attr(&Mark::new("bold".into(), HashMap::new())),
                )
            })
            .unwrap();
        let plan = compiler.finish(Some(3)).unwrap();
        preflight_mutation_plan(83, &plan, &txn).unwrap();
        plan
    };
    assert!(matches!(
        plan.actions.first(),
        Some(YrsMutationAction::CreateText { follow_up, .. }) if follow_up.len() == 3
    ));
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    let actual = from_prosemirror_json(&actual, &schema, UnknownTypeMode::Preserve).unwrap();
    assert_eq!(actual.root().text_content(), "aXcd");
    assert_eq!(document.root().text_content(), "");
    let marks = actual
        .root()
        .content()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .content()
        .unwrap();
    assert!(marks
        .iter()
        .all(|node| node.marks().iter().any(|mark| mark.mark_type() == "bold")));
}

#[test]
fn atom_only_textblock_supports_both_gaps_and_adjacent_opaque_atoms() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let before = compile_and_execute(
        source.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(0),
            text: "before".into(),
            marks: vec![],
        }],
    );
    assert_eq!(before.0["content"][0]["content"][0]["text"], "before");
    assert_eq!(before.0["content"][0]["content"][1]["type"], "hardBreak");

    let after = compile_and_execute(
        source.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "after".into(),
            marks: vec![],
        }],
    );
    assert_eq!(after.0["content"][0]["content"][0]["type"], "hardBreak");
    assert_eq!(after.0["content"][0]["content"][1]["text"], "after");

    let both = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(0),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "R".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(both.0["content"][0]["content"][0]["text"], "L");
    assert_eq!(both.0["content"][0]["content"][1]["type"], "hardBreak");
    assert_eq!(both.0["content"][0]["content"][2]["text"], "R");

    let mention_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "mention",
                "attrs": { "id": "user-1", "label": "Alice" }
            }]
        }]
    });
    let (mention_doc, mention_schema, mention_limits, _, _) = diagnostic_doc(&mention_source);
    let mention_plan = {
        let txn = mention_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            87,
            &txn,
            &fragment,
            &mention_schema,
            1_000,
            mention_limits.max_input_bytes,
            0,
        )
        .unwrap();
        let after_mention = compiler
            .target_positions_for_test()
            .unwrap()
            .last()
            .unwrap()
            .0;
        compiler.insert(0, after_mention, "tail", &[]).unwrap();
        let plan = compiler.finish(Some(0)).unwrap();
        preflight_mutation_plan(87, &plan, &txn).unwrap();
        plan
    };
    {
        let mut txn = mention_doc.transact_mut();
        execute_mutation_plan(mention_plan, &mut txn);
    }
    let txn = mention_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mention = YrsDocumentCodec::new(&mention_schema, &mention_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(mention["content"][0]["content"][0]["type"], "mention");
    assert_eq!(mention["content"][0]["content"][1]["text"], "tail");
}

include!("direct_operations/structural_insert.rs");

include!("direct_operations/insert_positions.rs");

include!("direct_operations/opaque_validation.rs");

include!("direct_operations/opaque_limits.rs");

include!("direct_operations/attributes.rs");
