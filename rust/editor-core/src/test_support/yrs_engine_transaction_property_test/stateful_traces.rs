fn trace_span(salt: u64, scalar_len: u32) -> (u32, u32) {
    debug_assert!(scalar_len > 0);
    let from = u32::try_from(salt % u64::from(scalar_len)).unwrap();
    let remaining = scalar_len - from;
    let max_width = remaining.min(3);
    let width = 1 + u32::try_from(salt.rotate_left(13) % u64::from(max_width)).unwrap();
    (from, from + width)
}

fn stateful_inline_scenario(
    spec: &ActionSpec,
    document: &Document,
    schema: &Schema,
    coverage: &RefCell<Coverage>,
) -> (TypedOperation, Vec<Step>) {
    let rendered = rendered_text(document, schema);
    let scalar_len = u32::try_from(rendered.chars().count()).unwrap();
    let kind = if scalar_len == 0 { 0 } else { spec.kind % 6 };
    let pm = |scalar| legacy_position(document, schema, scalar);
    match kind {
        0 => {
            let at = u32::try_from(spec.salt % u64::from(scalar_len + 1)).unwrap();
            let text = if spec.salt & 4 == 0 {
                "😀".to_string()
            } else {
                char::from(b'a' + u8::try_from(spec.salt % 26).unwrap()).to_string()
            };
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
        1 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            (
                TypedOperation::DeleteRange {
                    range: range(&rendered, from, to, spec.salt, coverage),
                },
                vec![Step::DeleteRange {
                    from: pm(from),
                    to: pm(to),
                }],
            )
        }
        2 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            let replacement = if spec.salt & 8 == 0 { "Ω" } else { "Q" };
            let content = Fragment::from(vec![Node::text(replacement.into(), vec![])]);
            (
                TypedOperation::ReplaceRange {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    content: content.clone(),
                },
                vec![Step::ReplaceRange {
                    from: pm(from),
                    to: pm(to),
                    content,
                }],
            )
        }
        3 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            let mark = Mark::new("bold".into(), HashMap::new());
            (
                TypedOperation::AddMark {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    mark: mark.clone(),
                },
                vec![Step::AddMark {
                    from: pm(from),
                    to: pm(to),
                    mark,
                }],
            )
        }
        4 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            (
                TypedOperation::RemoveMark {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    mark_type: "bold".into(),
                },
                vec![Step::RemoveMark {
                    from: pm(from),
                    to: pm(to),
                    mark_type: "bold".into(),
                }],
            )
        }
        5 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            let mark = Mark::new("bold".into(), HashMap::new());
            (
                TypedOperation::ReplaceMark {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    mark: mark.clone(),
                },
                vec![
                    Step::RemoveMark {
                        from: pm(from),
                        to: pm(to),
                        mark_type: "bold".into(),
                    },
                    Step::AddMark {
                        from: pm(from),
                        to: pm(to),
                        mark,
                    },
                ],
            )
        }
        _ => unreachable!(),
    }
}

fn run_stateful_trace(trace: &[ActionSpec], coverage: &RefCell<Coverage>) {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a😀bcdef" }] }]
    });
    let mut legacy = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    for (index, spec) in trace.iter().enumerate() {
        let legacy_before = legacy.clone();
        let engine_before = (
            yrs.encoded_state().unwrap(),
            yrs.revision(),
            yrs.state_revision(),
        );
        let local_state_before = (
            yrs.relative_selection().cloned(),
            yrs.resolved_selection().cloned(),
            yrs.stored_marks().map(<[Mark]>::to_vec),
        );
        let (operation, legacy_steps) = stateful_inline_scenario(spec, &legacy, &schema, coverage);
        let mut legacy_transaction = Transaction::new(Source::Api);
        for step in legacy_steps {
            legacy_transaction.add_step(step);
        }
        legacy = legacy_transaction
            .apply_with_limits(&legacy, &schema, &ResourceLimits::default())
            .unwrap_or_else(|error| {
                panic!("legacy trace step {index} ({spec:?}) failed: {error:?}")
            })
            .0;
        let expected_changed = legacy != legacy_before;
        let operation_debug = format!("{operation:?}");
        let commit = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                10_000 + u64::try_from(index).unwrap(),
                operation,
            ))
            .unwrap_or_else(|error| panic!("Yrs trace step {index} ({spec:?}) failed: {error:?}"));
        assert_installed_position_map_matches_full_build(
            &yrs,
            &schema,
            &format!("trace step {index} ({spec:?})"),
        );
        let local_state_after = (
            yrs.relative_selection().cloned(),
            yrs.resolved_selection().cloned(),
            yrs.stored_marks().map(<[Mark]>::to_vec),
        );
        let local_state_changed = local_state_after != local_state_before;
        let expected_commit_changed = expected_changed || local_state_changed;
        assert_eq!(
            commit.changed, expected_commit_changed,
            "trace step {index}: {spec:?}; operation={operation_debug}; before_revisions=({}, {}); after=({}, {}, {:?}, {:?})",
            engine_before.1,
            engine_before.2,
            yrs.revision(),
            yrs.state_revision(),
            yrs.stored_marks(),
            yrs.resolved_selection(),
        );
        assert_eq!(
            yrs.revision(),
            engine_before.1 + u64::from(expected_changed),
            "document revision at trace step {index}: {spec:?}"
        );
        assert_eq!(
            yrs.state_revision(),
            engine_before.2 + u64::from(expected_commit_changed),
            "state revision at trace step {index}: {spec:?}"
        );
        if !expected_commit_changed {
            assert_eq!(
                (
                    yrs.encoded_state().unwrap(),
                    yrs.revision(),
                    yrs.state_revision()
                ),
                engine_before,
                "no-op trace step {index}: {spec:?}"
            );
        } else if !expected_changed {
            assert_eq!(
                yrs.encoded_state().unwrap(),
                engine_before.0,
                "state-only trace step wrote Yrs content at {index}: {spec:?}"
            );
        }

        assert_eq!(
            yrs.document(),
            Some(&legacy),
            "trace step {index}: {spec:?}"
        );
        assert_eq!(
            yrs.document_json(),
            Some(to_prosemirror_json(&legacy, &schema)),
            "trace step {index}: {spec:?}"
        );
        assert_eq!(
            yrs.document_html(),
            Some(to_html(&legacy, &schema)),
            "trace step {index}: {spec:?}"
        );
        DocumentValidator::validate(&legacy, &schema, &ResourceLimits::default()).unwrap();
        assert_encoded_state_matches(&yrs, &legacy, &schema);
    }
}

fn run_custom_root_case(coverage: &RefCell<Coverage>) {
    coverage.borrow_mut().custom_root = true;
    let schema = custom_root_schema();
    let source = serde_json::json!({
        "type": "article",
        "content": [{ "type": "body", "content": [{ "type": "text", "text": "root😀" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = rendered_text(&document, &schema);
    let at = revisioned(&rendered, 2, 3, coverage);
    let pos = legacy_position(&document, &schema, 2);
    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::InsertText {
        pos,
        text: "!".into(),
        marks: vec![],
    });
    let expected = legacy.apply(&document, &schema).unwrap().0;
    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    yrs.apply_typed_transaction(transaction(
        &yrs,
        9_001,
        TypedOperation::InsertText {
            at,
            text: "!".into(),
            marks: vec![],
        },
    ))
    .unwrap();
    assert_installed_position_map_matches_full_build(&yrs, &schema, "custom root insert");
    assert_eq!(yrs.document(), Some(&expected));
    assert_eq!(
        yrs.document_json(),
        Some(to_prosemirror_json(&expected, &schema))
    );
    assert_eq!(yrs.document_html(), Some(to_html(&expected, &schema)));
    assert_encoded_state_matches(&yrs, &expected, &schema);
}

fn run_evolving_list_chain(salt: u64, coverage: &RefCell<Coverage>) {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
        ]
    });
    let mut legacy = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    for operation_index in 0..4 {
        let rendered = rendered_text(&legacy, &schema);
        let pm = |scalar| legacy_position(&legacy, &schema, scalar);
        let operation_salt = salt.rotate_left(operation_index * 7);
        let (operation, step) = match operation_index {
            0 => {
                let end = u32::try_from(rendered.chars().count()).unwrap();
                (
                    TypedOperation::WrapInList {
                        range: range(&rendered, 0, end, operation_salt, coverage),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    },
                    Step::WrapInList {
                        from: pm(0),
                        to: pm(end),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    },
                )
            }
            1 => {
                let at = scalar_index(&rendered, "two") + 1;
                (
                    TypedOperation::IndentListItem {
                        at: revisioned(&rendered, at, operation_salt, coverage),
                    },
                    Step::IndentListItem { pos: pm(at) },
                )
            }
            2 => {
                let at = scalar_index(&rendered, "two") + 1;
                (
                    TypedOperation::OutdentListItem {
                        at: revisioned(&rendered, at, operation_salt, coverage),
                    },
                    Step::OutdentListItem { pos: pm(at) },
                )
            }
            3 => {
                let at = scalar_index(&rendered, "one") + 1;
                (
                    TypedOperation::UnwrapFromList {
                        at: revisioned(&rendered, at, operation_salt, coverage),
                    },
                    Step::UnwrapFromList { pos: pm(at) },
                )
            }
            _ => unreachable!(),
        };

        let mut legacy_transaction = Transaction::new(Source::Api);
        legacy_transaction.add_step(step);
        legacy = legacy_transaction
            .apply_with_limits(&legacy, &schema, &ResourceLimits::default())
            .unwrap()
            .0;
        let commit = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                30_000 + u64::from(operation_index),
                operation,
            ))
            .unwrap();
        assert_installed_position_map_matches_full_build(
            &yrs,
            &schema,
            &format!("evolving list operation {operation_index}"),
        );
        assert!(commit.changed);
        assert_eq!(yrs.document(), Some(&legacy));
        assert_eq!(
            yrs.document_json(),
            Some(to_prosemirror_json(&legacy, &schema))
        );
        assert_eq!(yrs.document_html(), Some(to_html(&legacy, &schema)));
        DocumentValidator::validate(&legacy, &schema, &ResourceLimits::default()).unwrap();
        assert_encoded_state_matches(&yrs, &legacy, &schema);
    }
}
