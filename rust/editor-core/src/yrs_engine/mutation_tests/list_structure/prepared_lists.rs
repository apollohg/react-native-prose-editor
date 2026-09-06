#[test]
fn indent_then_outdent_cancels_the_unchanged_prepared_move() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (_, _, _, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two),
            },
        ],
        schema,
    );
    assert_eq!(
        to_prosemirror_json(&compiled.preview, &tiptap_schema()),
        source
    );
    assert!(compiled.mutation_plan.actions.is_empty());
    assert_eq!(compiled.encoded_growth_bound, 0);
    assert_eq!(compiled.undo_units_bound, 0);
}

#[test]
fn indent_edit_then_outdent_reinserts_only_the_changed_prepared_item() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::InsertText {
                at: point_for_test(two),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two),
            },
        ],
        schema,
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren { .. },
            YrsMutationAction::InsertXmlChildren { .. }
        ]
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(
        actual["content"][0]["content"][1]["content"][0]["content"][0]["text"],
        "tXwo"
    );
}

#[test]
fn outdent_from_a_multi_item_prepared_nested_list_retains_its_prefix_blueprint() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] },
                { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "three" }] }] }
            ]
        }]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two") + 1;
    let three = rendered_scalar_offset(&source, &schema, "three") + 1;
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::IndentListItem {
                at: point_for_test(two),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(three),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(three),
            },
        ],
    );
    assert_eq!(actual, expected);
    let outer = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(outer.len(), 2);
    assert_eq!(
        outer[0]["content"][1]["content"][0]["content"][0]["content"][0]["text"],
        "two"
    );
    assert_eq!(outer[1]["content"][0]["content"][0]["text"], "three");
}

#[test]
fn outdent_rewrites_a_fully_prepared_outer_list_and_parent_batch() {
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
        ]
    });
    let schema = tiptap_schema();
    let two = rendered_scalar_offset(&source, &schema, "two");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::WrapInList {
                range: range_for_test(0, two + 3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            TypedOperation::IndentListItem {
                at: point_for_test(two + 1),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(two + 1),
            },
        ],
        schema,
    );
    assert_eq!(
        compiled
            .mutation_plan
            .actions
            .iter()
            .filter(|action| matches!(action, YrsMutationAction::InsertXmlChildren { .. }))
            .count(),
        1
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap(),
        expected
    );
}

#[test]
fn outdent_first_nested_item_under_a_prepared_parent_rewrites_the_owned_batch() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [{
                            "type": "listItem",
                            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }]
                        }]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let outer = u32::try_from(rendered[..rendered.find("outer").unwrap()].chars().count()).unwrap();
    let inner = u32::try_from(rendered[..rendered.find("inner").unwrap()].chars().count()).unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(inner + 1),
            },
        ],
        schema,
    );
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["content"][0]["content"][0]["text"], "ou");
    assert_eq!(items[1]["content"][0]["content"][0]["text"], "ter");
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "inner");
}

#[test]
fn outdent_first_of_multiple_nested_items_under_a_prepared_parent_keeps_the_tail() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let outer = rendered_scalar_offset(&source, &schema, "outer");
    let inner = rendered_scalar_offset(&source, &schema, "inner");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::OutdentListItem {
                at: point_for_test(inner + 1),
            },
        ],
    );
    assert_eq!(actual, expected);
    let items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[2]["content"][0]["content"][0]["text"], "inner");
    assert_eq!(
        items[2]["content"][1]["content"][0]["content"][0]["content"][0]["text"],
        "tail"
    );
}

#[test]
fn unwrap_first_of_multiple_nested_items_under_a_prepared_parent_keeps_the_tail() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                    {
                        "type": "bulletList",
                        "content": [
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] },
                            { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "tail" }] }] }
                        ]
                    }
                ]
            }]
        }]
    });
    let schema = tiptap_schema();
    let outer = rendered_scalar_offset(&source, &schema, "outer");
    let inner = rendered_scalar_offset(&source, &schema, "inner");
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![
            TypedOperation::SplitBlock {
                at: point_for_test(outer + 2),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(inner + 1),
            },
        ],
    );
    assert_eq!(actual, expected);
    let outer_items = actual["content"][0]["content"].as_array().unwrap();
    assert_eq!(outer_items.len(), 2);
    let split_outer = outer_items[1]["content"].as_array().unwrap();
    assert_eq!(split_outer[0]["content"][0]["text"], "ter");
    assert_eq!(split_outer[1]["content"][0]["text"], "inner");
    assert_eq!(
        split_outer[2]["content"][0]["content"][0]["content"][0]["text"],
        "tail"
    );
}
