#[test]
fn split_sequences_materialize_only_the_compact_final_plan() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let (_, _, _, inserted) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
        tiptap_schema(),
    );
    assert!(!inserted
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
    let inserted_nodes = inserted
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { children, .. } = &inserted_nodes[0].node else {
        panic!("prepared right block expected")
    };
    let PreparedXmlNode::Text { runs } = &children[0].node else {
        panic!("prepared right text expected")
    };
    assert_eq!(prepared_text_for_test(runs), "BX");

    let (_, _, _, marked) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::AddMark {
                range: range_for_test(2, 3),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ],
        tiptap_schema(),
    );
    assert!(!marked
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::FormatText { .. })));
    let marked_nodes = marked
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { children, .. } = &marked_nodes[0].node else {
        panic!("prepared marked block expected")
    };
    let PreparedXmlNode::Text { runs } = &children[0].node else {
        panic!("prepared marked text expected")
    };
    assert_eq!(prepared_text_for_test(runs), "B");
    assert_eq!(runs[0].attrs.get("bold"), Some(&Any::Bool(true)));

    let heading = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": { "id": "left" },
            "content": [{ "type": "text", "text": "abcd" }]
        }]
    });
    let (_, _, _, attributed) = compile_operations_with_schema(
        &heading,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "h2".into(),
                attrs: HashMap::from([("id".into(), Value::String("right-old".into()))]),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(2),
                attrs: HashMap::from([("id".into(), Value::String("right-new".into()))]),
            },
        ],
        attribute_schema(),
    );
    assert!(!attributed.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
    let attributed_nodes = attributed
        .mutation_plan
        .actions
        .iter()
        .find_map(|action| match action {
            YrsMutationAction::InsertXmlChildren { nodes, .. } => Some(nodes),
            _ => None,
        })
        .unwrap();
    let PreparedXmlNode::Element { attrs, .. } = &attributed_nodes[0].node else {
        panic!("prepared attributed block expected")
    };
    assert!(attrs
        .iter()
        .any(|(key, value)| { key == "id" && value == &Any::String("right-new".into()) }));
}

#[test]
fn folded_split_undo_bound_uses_the_compact_final_plan_exactly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "A😀B" }]
        }]
    });
    let operations = || {
        vec![
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::SplitBlock {
                at: point_for_test(2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
        ]
    };
    let exact =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), u64::MAX)
            .unwrap()
            .undo_units_bound;
    assert!(exact > 0);
    let accepted =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), exact).unwrap();
    assert_eq!(accepted.undo_units_bound, exact);
    let rejected =
        compile_operations_with_undo_limit(&source, operations(), tiptap_schema(), exact - 1)
            .unwrap_err();
    assert_eq!(rejected.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(rejected.limit, Some(exact - 1));
    assert_eq!(rejected.actual, Some(exact));

    let plain = compile_operations_with_undo_limit(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "x".into(),
            marks: vec![],
        }],
        tiptap_schema(),
        1,
    )
    .unwrap();
    assert_eq!(plain.undo_units_bound, 1);

    let emoji = compile_operations_with_undo_limit(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "🙂".into(),
            marks: vec![],
        }],
        tiptap_schema(),
        2,
    )
    .unwrap();
    assert_eq!(emoji.undo_units_bound, 2);

    let (emoji_doc, _, _, emoji_compiled) = compile_operations_with_schema(
        &source,
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "🙂".into(),
            marks: vec![],
        }],
        tiptap_schema(),
    );
    let fragment = emoji_doc
        .transact()
        .get_xml_fragment("prosemirror")
        .unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&emoji_doc, &fragment);
    {
        let mut txn = emoji_doc.transact_mut();
        execute_mutation_plan(emoji_compiled.mutation_plan, &mut txn);
    }
    let inserted = undo.undo_stack()[0]
        .insertions()
        .iter()
        .flat_map(|(_, ranges)| ranges.into_iter())
        .map(|range| u64::from(range.end - range.start))
        .sum::<u64>();
    assert_eq!(inserted, 2);
    assert!(inserted <= emoji_compiled.undo_units_bound);
}
