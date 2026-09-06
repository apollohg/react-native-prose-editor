fn scenario(spec: &ActionSpec, coverage: &RefCell<Coverage>) -> Scenario {
    coverage.borrow_mut().operations[usize::from(spec.kind)] = true;
    let schema = tiptap_schema();
    let source = match spec.kind {
        4 => serde_json::from_str(BOLD).unwrap(),
        5 => serde_json::from_str(LINK).unwrap(),
        7 => serde_json::json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
            ]
        }),
        8 => serde_json::json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
        }),
        9 => serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }]
            }]
        }),
        10 => serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                ]
            }]
        }),
        11 => serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                        { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] }] }
                    ]
                }]
            }]
        }),
        13 => serde_json::json!({
            "type": "doc",
            "content": [{ "type": "image", "attrs": { "src": "old", "alt": null, "title": null, "width": null, "height": null } }]
        }),
        _ => serde_json::from_str(PLAIN).unwrap(),
    };
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = rendered_text(&document, &schema);
    let pm = |scalar| legacy_position(&document, &schema, scalar);
    let bold = Mark::new("bold".into(), HashMap::new());
    let (operation, legacy_steps) = match spec.kind {
        0 => {
            let at = 3;
            let text = char::from(b'a' + u8::try_from(spec.salt % 26).unwrap()).to_string();
            (
                TypedOperation::InsertText {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                    text: text.clone(),
                    marks: vec![],
                },
                vec![Step::InsertText {
                    pos: pm(at),
                    text,
                    marks: vec![],
                }],
            )
        }
        1 => (
            TypedOperation::DeleteRange {
                range: range(&rendered, 1, 4, spec.salt, coverage),
            },
            vec![Step::DeleteRange {
                from: pm(1),
                to: pm(4),
            }],
        ),
        2 => {
            let content = Fragment::from(vec![Node::text("XY".into(), vec![])]);
            (
                TypedOperation::ReplaceRange {
                    range: range(&rendered, 2, 4, spec.salt, coverage),
                    content: content.clone(),
                },
                vec![Step::ReplaceRange {
                    from: pm(2),
                    to: pm(4),
                    content,
                }],
            )
        }
        3 => (
            TypedOperation::AddMark {
                range: range(&rendered, 1, 5, spec.salt, coverage),
                mark: bold.clone(),
            },
            vec![Step::AddMark {
                from: pm(1),
                to: pm(5),
                mark: bold,
            }],
        ),
        4 => (
            TypedOperation::RemoveMark {
                range: range(&rendered, 1, 5, spec.salt, coverage),
                mark_type: "bold".into(),
            },
            vec![Step::RemoveMark {
                from: pm(1),
                to: pm(5),
                mark_type: "bold".into(),
            }],
        ),
        5 => {
            let mark = Mark::new(
                "link".into(),
                HashMap::from([(
                    "href".into(),
                    serde_json::json!(format!("new-{}", spec.salt % 7)),
                )]),
            );
            (
                TypedOperation::ReplaceMark {
                    range: range(&rendered, 1, 5, spec.salt, coverage),
                    mark: mark.clone(),
                },
                vec![
                    Step::RemoveMark {
                        from: pm(1),
                        to: pm(5),
                        mark_type: "link".into(),
                    },
                    Step::AddMark {
                        from: pm(1),
                        to: pm(5),
                        mark,
                    },
                ],
            )
        }
        6 => (
            TypedOperation::SplitBlock {
                at: revisioned(&rendered, 3, spec.salt, coverage),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            vec![Step::SplitBlock {
                pos: pm(3),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
        ),
        7 => (
            TypedOperation::JoinBlocks {
                at: revisioned(&rendered, 2, spec.salt, coverage),
            },
            vec![Step::JoinBlocks { pos: 4 }],
        ),
        8 => (
            TypedOperation::WrapInList {
                range: range(&rendered, 0, 3, spec.salt, coverage),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            vec![Step::WrapInList {
                from: pm(0),
                to: pm(3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
        ),
        9 => {
            let at = scalar_index(&rendered, "one") + 1;
            (
                TypedOperation::UnwrapFromList {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                },
                vec![Step::UnwrapFromList { pos: pm(at) }],
            )
        }
        10 => {
            let at = scalar_index(&rendered, "two") + 1;
            (
                TypedOperation::IndentListItem {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                },
                vec![Step::IndentListItem { pos: pm(at) }],
            )
        }
        11 => {
            let at = scalar_index(&rendered, "inner") + 1;
            (
                TypedOperation::OutdentListItem {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                },
                vec![Step::OutdentListItem { pos: pm(at) }],
            )
        }
        12 => {
            let node = if spec.salt & 4 == 0 {
                coverage.borrow_mut().void_node = true;
                Node::void("hardBreak".into(), HashMap::new())
            } else {
                coverage.borrow_mut().opaque_node = true;
                opaque_inline(spec.salt)
            };
            (
                TypedOperation::InsertNode {
                    at: revisioned(&rendered, 3, spec.salt, coverage),
                    node: node.clone(),
                },
                vec![Step::InsertNode { pos: pm(3), node }],
            )
        }
        13 => {
            let attrs = HashMap::from([
                (
                    "src".into(),
                    serde_json::json!(format!("new-{}", spec.salt % 11)),
                ),
                ("alt".into(), serde_json::json!("trace")),
                ("title".into(), serde_json::Value::Null),
                ("width".into(), serde_json::Value::Null),
                ("height".into(), serde_json::Value::Null),
            ]);
            (
                TypedOperation::UpdateNodeAttrs {
                    at: revisioned(&rendered, 0, spec.salt, coverage),
                    attrs: attrs.clone(),
                },
                vec![Step::UpdateNodeAttrs { pos: pm(0), attrs }],
            )
        }
        _ => unreachable!(),
    };
    Scenario {
        schema,
        source,
        operation,
        legacy_steps,
    }
}

fn run_scenario(spec: &ActionSpec, coverage: &RefCell<Coverage>) {
    let Scenario {
        schema,
        source,
        operation,
        legacy_steps,
    } = scenario(spec, coverage);
    let mut legacy = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let mut legacy_transaction = Transaction::new(Source::Api);
    for step in legacy_steps {
        legacy_transaction.add_step(step);
    }
    let (expected, _) = legacy_transaction
        .apply_with_limits(&legacy, &schema, &ResourceLimits::default())
        .unwrap_or_else(|error| panic!("legacy kind {} failed: {error:?}", spec.kind));
    legacy = expected;

    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let commit = yrs
        .apply_typed_transaction(transaction(&yrs, spec.salt, operation))
        .unwrap_or_else(|error| panic!("Yrs kind {} failed: {error:?}", spec.kind));
    assert_installed_position_map_matches_full_build(
        &yrs,
        &schema,
        &format!("operation kind {}", spec.kind),
    );
    assert_eq!(
        commit.changed,
        yrs.document().unwrap()
            != &from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap()
    );
    assert_eq!(yrs.document().unwrap(), &legacy);
    assert_eq!(
        yrs.document_json().unwrap(),
        to_prosemirror_json(&legacy, &schema)
    );
    assert_eq!(yrs.document_html().unwrap(), to_html(&legacy, &schema));
    DocumentValidator::validate(yrs.document().unwrap(), &schema, &ResourceLimits::default())
        .unwrap();
    DocumentValidator::validate(&legacy, &schema, &ResourceLimits::default()).unwrap();
    assert_encoded_state_matches(&yrs, &legacy, &schema);
}
