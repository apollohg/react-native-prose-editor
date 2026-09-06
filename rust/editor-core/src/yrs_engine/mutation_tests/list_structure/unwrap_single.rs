#[test]
fn unwrap_only_list_item_replaces_the_list_with_its_blocks_directly() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let one = u32::try_from(rendered[..rendered.find("one").unwrap()].chars().count()).unwrap();
    let (actual, expected, _, _, _) = compile_and_execute(
        source,
        vec![TypedOperation::UnwrapFromList {
            at: point_for_test(one + 1),
        }],
    );
    assert_eq!(actual, expected);
    assert_eq!(actual["content"][0]["type"], "paragraph");
    assert_eq!(actual["content"][0]["content"][0]["text"], "one");
}

#[test]
fn unwrap_only_then_insert_text_folds_into_prepared_text() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(3),
            },
            TypedOperation::InsertText {
                at: point_for_test(3),
                text: "X".into(),
                marks: vec![],
            },
        ],
        tiptap_schema(),
    );
    assert!(!compiled
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
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
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn edit_then_unwrap_tombstones_deleted_text_actions() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertText {
                at: point_for_test(one + 1),
                text: "X".into(),
                marks: vec![],
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
        ],
        schema,
    );
    assert!(!compiled
        .mutation_plan
        .actions
        .iter()
        .any(|action| matches!(action, YrsMutationAction::InsertText { .. })));
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
    assert_eq!(actual["content"][0]["content"][0]["text"], "oXne");
}

#[test]
fn insert_node_then_unwrap_owns_one_canonical_prepared_batch() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "one" }]
                }]
            }]
        }]
    });
    let schema = tiptap_schema();
    let one = rendered_scalar_offset(&source, &schema, "one");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::InsertNode {
                at: point_for_test(one + 1),
                node: Node::void("hardBreak".into(), HashMap::new()),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(one + 1),
            },
        ],
        schema,
    );
    assert!(matches!(
        compiled.mutation_plan.actions.as_slice(),
        [
            YrsMutationAction::DeleteXmlChildren {
                child_index: 0,
                child_count: 1,
                ..
            },
            YrsMutationAction::InsertXmlChildren { nodes, .. }
        ] if nodes.len() == 1
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
    assert!(actual["content"][0]["content"]
        .as_array()
        .unwrap()
        .iter()
        .any(|node| node["type"] == "hardBreak"));
}

#[test]
fn unwrap_only_then_update_extracted_block_attrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "lead" }]
                    },
                    {
                        "type": "h2",
                        "attrs": { "id": "before" },
                        "content": [{ "type": "text", "text": "heading" }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, &schema);
    let lead = u32::try_from(rendered[..rendered.find("lead").unwrap()].chars().count()).unwrap();
    let heading = u32::try_from(
        rendered[..rendered.find("heading").unwrap()]
            .chars()
            .count(),
    )
    .unwrap();
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UnwrapFromList {
                at: point_for_test(lead + 1),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(heading),
                attrs: HashMap::from([("id".into(), Value::String("after".into()))]),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
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
    assert_eq!(actual["content"][1]["attrs"]["id"], "after");
}

#[test]
fn attrs_then_unwrap_tombstones_deleted_element_attrs() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "taskList",
            "content": [{
                "type": "taskItem",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "lead" }]
                    },
                    {
                        "type": "h2",
                        "attrs": { "id": "before" },
                        "content": [{ "type": "text", "text": "heading" }]
                    }
                ]
            }]
        }]
    });
    let schema = attribute_schema();
    let lead = rendered_scalar_offset(&source, &schema, "lead");
    let heading = rendered_scalar_offset(&source, &schema, "heading");
    let (doc, schema, limits, compiled) = compile_operations_with_schema(
        &source,
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: point_for_test(heading),
                attrs: HashMap::from([("id".into(), Value::String("after".into()))]),
            },
            TypedOperation::UnwrapFromList {
                at: point_for_test(lead + 1),
            },
        ],
        schema,
    );
    assert!(!compiled.mutation_plan.actions.iter().any(|action| {
        matches!(
            action,
            YrsMutationAction::SetXmlAttribute { .. }
                | YrsMutationAction::RemoveXmlAttribute { .. }
        )
    }));
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
    assert_eq!(actual["content"][1]["attrs"]["id"], "after");
}
