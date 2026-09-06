#[test]
fn operation_count_and_aggregate_input_enforce_exact_boundaries() {
    let one_operation = EditingLimits::resolve(EditingLimitOverrides {
        max_operations_per_transaction: Some(1),
        ..EditingLimitOverrides::default()
    })
    .unwrap();
    let engine = engine_with(PLAIN, ResourceLimits::default(), one_operation, None);
    engine
        .compile_typed_transaction(transaction(
            &engine,
            vec![TypedOperation::InsertText {
                at: point(5),
                text: "x".into(),
                marks: vec![],
            }],
        ))
        .unwrap();
    let error = engine
        .compile_typed_transaction(transaction(
            &engine,
            vec![
                TypedOperation::InsertText {
                    at: point(5),
                    text: "x".into(),
                    marks: vec![],
                },
                TypedOperation::InsertText {
                    at: point(5),
                    text: "y".into(),
                    marks: vec![],
                },
            ],
        ))
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, None);
    assert_eq!(error.actual, Some(2));

    let engine = engine_with(
        PLAIN,
        ResourceLimits::default(),
        EditingLimits::default(),
        None,
    );
    let schema = tiptap_schema();
    let resource_limits = ResourceLimits {
        max_input_bytes: 3,
        ..ResourceLimits::default()
    };
    let context = super::CompilationContext {
        document: engine.document().unwrap(),
        selection: None,
        schema: &schema,
        resource_limits: &resource_limits,
        editing_limits: engine.editing_limits(),
        document_revision: engine.revision(),
        max_length: None,
    };
    super::compile_transaction(
        context,
        transaction(
            &engine,
            vec![TypedOperation::InsertText {
                at: point(5),
                text: "abc".into(),
                marks: vec![],
            }],
        ),
    )
    .unwrap();
    let error = super::compile_transaction(
        context,
        transaction(
            &engine,
            vec![
                TypedOperation::InsertText {
                    at: point(5),
                    text: "ab".into(),
                    marks: vec![],
                },
                TypedOperation::InsertText {
                    at: point(5),
                    text: "cd".into(),
                    marks: vec![],
                },
            ],
        ),
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(1));
    assert_eq!(error.limit, Some(3));
    assert_eq!(error.actual, Some(4));
}

#[test]
fn aggregate_output_and_undo_work_accept_exact_and_reject_one_over() {
    let engine = engine(PLAIN);
    let first = legacy(
        &engine,
        vec![Step::InsertText {
            pos: 6,
            text: "x".into(),
            marks: vec![],
        }],
    );
    let second = {
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::InsertText {
            pos: 7,
            text: "y".into(),
            marks: vec![],
        });
        tx.apply(&first, &tiptap_schema()).unwrap().0
    };
    let output_bytes = |document: &crate::model::Document| {
        serde_json::to_vec(&crate::serialize::to_prosemirror_json(
            document,
            &tiptap_schema(),
        ))
        .unwrap()
        .len()
    };
    let exact_output = output_bytes(&first) + output_bytes(&second);
    let operations = vec![
        TypedOperation::InsertText {
            at: point(5),
            text: "x".into(),
            marks: vec![],
        },
        TypedOperation::InsertText {
            at: point(5),
            text: "y".into(),
            marks: vec![],
        },
    ];
    for (limit, accepted) in [(exact_output, true), (exact_output - 1, false)] {
        let limits = EditingLimits {
            max_derived_output_bytes: limit,
            ..EditingLimits::default()
        };
        let schema = tiptap_schema();
        let context = super::CompilationContext {
            document: engine.document().unwrap(),
            selection: None,
            schema: &schema,
            resource_limits: engine.resource_limits(),
            editing_limits: &limits,
            document_revision: engine.revision(),
            max_length: None,
        };
        let result = super::compile_transaction(context, transaction(&engine, operations.clone()));
        if accepted {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
            assert_eq!(error.operation_index, Some(1));
            assert_eq!(error.limit, Some((exact_output - 1) as u64));
            assert_eq!(error.actual, Some(exact_output as u64));
        }
    }

    for (text, accepted) in [("x", true), ("xy", false)] {
        let limits = EditingLimits {
            max_undo_retained_units: 1,
            ..EditingLimits::default()
        };
        let schema = tiptap_schema();
        let context = super::CompilationContext {
            document: engine.document().unwrap(),
            selection: None,
            schema: &schema,
            resource_limits: engine.resource_limits(),
            editing_limits: &limits,
            document_revision: engine.revision(),
            max_length: None,
        };
        let result = super::compile_transaction(
            context,
            transaction(
                &engine,
                vec![TypedOperation::InsertText {
                    at: point(5),
                    text: text.into(),
                    marks: vec![],
                }],
            ),
        );
        if accepted {
            assert_eq!(result.unwrap().undo_units_bound, 1);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.operation_index, Some(0));
            assert_eq!(error.limit, Some(1));
            assert_eq!(error.actual, Some(2));
        }
    }

    let limits = EditingLimits {
        max_undo_retained_units: 1,
        ..EditingLimits::default()
    };
    let schema = tiptap_schema();
    let context = super::CompilationContext {
        document: engine.document().unwrap(),
        selection: None,
        schema: &schema,
        resource_limits: engine.resource_limits(),
        editing_limits: &limits,
        document_revision: engine.revision(),
        max_length: None,
    };
    let mut skipped = transaction(
        &engine,
        vec![TypedOperation::InsertText {
            at: point(5),
            text: "xy".into(),
            marks: vec![],
        }],
    );
    skipped.history_policy = HistoryPolicy::Skip;
    let compiled = super::compile_transaction(context, skipped).unwrap();
    assert_eq!(compiled.undo_units_bound, 0);
    assert_eq!(compiled.history_class, HistoryClass::Skip);

    let limited_engine = engine_with(PLAIN, ResourceLimits::default(), limits.clone(), None);
    let mut skipped = transaction(
        &limited_engine,
        vec![TypedOperation::InsertText {
            at: point(5),
            text: "xy".into(),
            marks: vec![],
        }],
    );
    skipped.history_policy = HistoryPolicy::Skip;
    let compiled = limited_engine.compile_typed_transaction(skipped).unwrap();
    assert_eq!(compiled.undo_units_bound, 0);
    assert_eq!(compiled.replay_work_units_bound, 2);

    for (range, accepted) in [(range(1, 2), true), (range(1, 3), false)] {
        let context = super::CompilationContext {
            document: engine.document().unwrap(),
            selection: None,
            schema: &schema,
            resource_limits: engine.resource_limits(),
            editing_limits: &limits,
            document_revision: engine.revision(),
            max_length: None,
        };
        let result = super::compile_transaction(
            context,
            transaction(
                &engine,
                vec![TypedOperation::AddMark {
                    range,
                    mark: Mark::new("bold".into(), HashMap::new()),
                }],
            ),
        );
        if accepted {
            assert_eq!(result.unwrap().undo_units_bound, 1);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.operation_index, Some(0));
            assert_eq!(error.limit, Some(1));
            assert_eq!(error.actual, Some(2));
        }
    }
}

#[test]
fn utf16_affinity_canonical_order_and_affected_blocks_are_deterministic() {
    let emoji = engine(
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"A😀B"}]}]}"#,
    );
    let compiled = emoji
        .compile_typed_transaction(transaction(
            &emoji,
            vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 3,
                    kind: EditorOffsetKind::Utf16,
                    affinity: Affinity::After,
                },
                text: "!".into(),
                marks: vec![],
            }],
        ))
        .unwrap();
    assert_eq!(compiled.preview.root().text_content(), "A😀!B");
    let error = emoji
        .compile_typed_transaction(transaction(
            &emoji,
            vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
                    kind: EditorOffsetKind::Utf16,
                    affinity: Affinity::After,
                },
                text: "!".into(),
                marks: vec![],
            }],
        ))
        .unwrap_err();
    assert_eq!(error.code, "POSITION_INVALID");
    assert_eq!(error.operation_index, Some(0));

    let list = engine(
        r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"A😀"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"B"}]}]}]}]}"#,
    );
    let schema = tiptap_schema();
    let list_rendered = crate::render::rendered_text(list.document().unwrap(), &schema);
    assert_eq!(list_rendered, "• A😀\n• B");
    assert_eq!(
        list_rendered.chars().count() as u32,
        crate::position::PositionMap::build(list.document().unwrap(), &schema).total_scalars()
    );

    let plain_engine = engine(PLAIN);
    let before = RevisionedPosition {
        affinity: Affinity::Before,
        ..point(5)
    };
    let compiled = plain_engine
        .compile_typed_transaction(transaction(
            &plain_engine,
            vec![
                TypedOperation::InsertText {
                    at: point(5),
                    text: "A".into(),
                    marks: vec![],
                },
                TypedOperation::InsertText {
                    at: before,
                    text: "B".into(),
                    marks: vec![],
                },
            ],
        ))
        .unwrap();
    assert_eq!(compiled.preview.root().text_content(), "HelloBA");

    let italic = engine(
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"italic"}],"text":"Hello"}]}]}"#,
    );
    let compiled = italic
        .compile_typed_transaction(transaction(
            &italic,
            vec![TypedOperation::AddMark {
                range: range(1, 4),
                mark: Mark::new("bold".into(), HashMap::new()),
            }],
        ))
        .unwrap();
    let marked = compiled
        .preview
        .root()
        .child(0)
        .unwrap()
        .content()
        .unwrap()
        .iter()
        .find(|node| node.marks().len() == 2)
        .unwrap();
    assert_eq!(
        marked
            .marks()
            .iter()
            .map(|mark| mark.mark_type())
            .collect::<Vec<_>>(),
        vec!["bold", "italic"]
    );

    let two_blocks = engine(
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"One"}]},{"type":"paragraph","content":[{"type":"text","text":"Two"}]}]}"#,
    );
    let compiled = two_blocks
        .compile_typed_transaction(transaction(
            &two_blocks,
            vec![TypedOperation::InsertText {
                at: point(5),
                text: "!".into(),
                marks: vec![],
            }],
        ))
        .unwrap();
    assert_eq!(compiled.affected_top_level_blocks, vec![0, 1]);
}

#[test]
fn rendered_text_helper_matches_position_map_for_visible_atoms_and_separators() {
    let schema = render_parity_schema();
    let attrs = |label: &str| HashMap::from([("label".to_string(), serde_json::json!(label))]);
    let document = crate::model::Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![
            Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![
                    Node::text("A".into(), vec![]),
                    Node::void("hardBreak".into(), HashMap::new()),
                    Node::void(
                        "mention".into(),
                        HashMap::from([
                            ("label".into(), serde_json::json!("Jay")),
                            ("mentionSuggestionChar".into(), serde_json::json!("@")),
                        ]),
                    ),
                    Node::void("chip".into(), attrs("C")),
                ]),
            ),
            Node::void("horizontalRule".into(), HashMap::new()),
            Node::void("widget".into(), attrs("W")),
            Node::element("paragraph".into(), HashMap::new(), Fragment::empty()),
        ]),
    ));
    let rendered = crate::render::rendered_text(&document, &schema);
    assert_eq!(rendered, "A\n@Jay[C]\n\u{fffc}\n[W]\n\u{200b}");
    assert_eq!(
        rendered.chars().count() as u32,
        crate::position::PositionMap::build(&document, &schema).total_scalars()
    );
}

#[test]
fn selection_errors_are_transaction_level_and_non_local_origins_are_rejected() {
    let engine = engine(PLAIN);
    let mut selection_only = transaction(&engine, vec![]);
    selection_only.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point(99),
        head: point(0),
    });
    let error = engine
        .compile_typed_transaction(selection_only)
        .unwrap_err();
    assert_eq!(error.code, "POSITION_INVALID");
    assert_eq!(error.operation_index, None);
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "selection.anchor" }))
    );

    for origin in [
        TransactionOrigin::UndoRedo,
        TransactionOrigin::RemoteSync,
        TransactionOrigin::SnapshotRestore,
        TransactionOrigin::DocumentImport,
    ] {
        let mut tx = transaction(&engine, vec![]);
        tx.origin = origin;
        let error = engine.compile_typed_transaction(tx).unwrap_err();
        assert_eq!(error.code, "TRANSACTION_INVALID", "{origin:?}");
        assert_eq!(error.operation_index, None, "{origin:?}");
        assert_eq!(
            error.details,
            Some(serde_json::json!({ "field": "origin" })),
            "{origin:?}"
        );
    }
}

#[test]
fn no_op_marks_do_not_consume_undo_or_rewrite_unrelated_text_nodes() {
    let bold_engine = engine(BOLD);
    let limits = EditingLimits {
        max_undo_retained_units: 1,
        ..EditingLimits::default()
    };
    let schema = tiptap_schema();
    let operations = [
        TypedOperation::AddMark {
            range: range(1, 4),
            mark: Mark::new("bold".into(), HashMap::new()),
        },
        TypedOperation::RemoveMark {
            range: range(1, 4),
            mark_type: "italic".into(),
        },
        TypedOperation::ReplaceMark {
            range: range(1, 4),
            mark: Mark::new("bold".into(), HashMap::new()),
        },
    ];
    for operation in operations {
        let context = super::CompilationContext {
            document: bold_engine.document().unwrap(),
            selection: None,
            schema: &schema,
            resource_limits: bold_engine.resource_limits(),
            editing_limits: &limits,
            document_revision: bold_engine.revision(),
            max_length: None,
        };
        let compiled =
            super::compile_transaction(context, transaction(&bold_engine, vec![operation]))
                .unwrap();
        assert_eq!(compiled.preview, *bold_engine.document().unwrap());
        assert_eq!(compiled.undo_units_bound, 0);
        assert_eq!(compiled.history_class, HistoryClass::Skip);
    }

    let document = crate::model::Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![
                Node::text("A".into(), vec![]),
                Node::text("B".into(), vec![]),
            ]),
        )]),
    ));
    let document = crate::transform::canonicalize_yrs_document(&document, &schema);
    let resource_limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let context = super::CompilationContext {
        document: &document,
        selection: None,
        schema: &schema,
        resource_limits: &resource_limits,
        editing_limits: &editing_limits,
        document_revision: 0,
        max_length: None,
    };
    let compiled = super::compile_transaction(
        context,
        TypedTransaction {
            request_id: 8,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations: vec![TypedOperation::AddMark {
                range: range(0, 0),
                mark: Mark::new("bold".into(), HashMap::new()),
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Auto,
        },
    )
    .unwrap();
    assert_eq!(compiled.preview, document);
    assert_eq!(compiled.preview.root().child(0).unwrap().child_count(), 1);
}

#[test]
fn final_semantic_no_ops_discard_provisional_undo_work() {
    let engine = engine(PLAIN);
    let limits = EditingLimits {
        max_undo_retained_units: 1,
        ..EditingLimits::default()
    };
    let schema = tiptap_schema();
    let current = Selection::cursor(6);
    let context = super::CompilationContext {
        document: engine.document().unwrap(),
        selection: Some(&current),
        schema: &schema,
        resource_limits: engine.resource_limits(),
        editing_limits: &limits,
        document_revision: engine.revision(),
        max_length: None,
    };

    let mut identical_replace = transaction(
        &engine,
        vec![TypedOperation::ReplaceRange {
            range: range(0, 5),
            content: Fragment::from(vec![Node::text("Hello".into(), vec![])]),
        }],
    );
    identical_replace.selection_intent = SelectionIntent::Preserve;
    let compiled = super::compile_transaction(context, identical_replace).unwrap();
    assert_eq!(compiled.preview, *engine.document().unwrap());
    assert_eq!(compiled.history_class, HistoryClass::Skip);
    assert_eq!(compiled.undo_units_bound, 0);
    assert_eq!(compiled.selection_plan, SelectionPlan::Preserve);

    let mut cancelling = transaction(
        &engine,
        vec![
            TypedOperation::DeleteRange { range: range(4, 5) },
            TypedOperation::InsertText {
                at: point(4),
                text: "o".into(),
                marks: vec![],
            },
        ],
    );
    cancelling.selection_intent = SelectionIntent::Preserve;
    let compiled = super::compile_transaction(context, cancelling).unwrap();
    assert_eq!(compiled.preview, *engine.document().unwrap());
    assert_eq!(compiled.history_class, HistoryClass::Skip);
    assert_eq!(compiled.undo_units_bound, 0);
    assert_eq!(compiled.selection_plan, SelectionPlan::Preserve);

    let changed = transaction(
        &engine,
        vec![
            TypedOperation::DeleteRange { range: range(4, 5) },
            TypedOperation::InsertText {
                at: point(4),
                text: "x".into(),
                marks: vec![],
            },
        ],
    );
    let error = super::compile_transaction(context, changed).unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(1));
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
}

#[test]
fn explicit_zero_max_length_rejects_growth_without_mutating_engine() {
    let engine = engine_with(
        PLAIN,
        ResourceLimits::default(),
        EditingLimits::default(),
        Some(0),
    );
    let before = audit(&engine);
    let error = engine
        .compile_typed_transaction(transaction(
            &engine,
            vec![TypedOperation::InsertText {
                at: point(0),
                text: "x".into(),
                marks: vec![],
            }],
        ))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "maxLength" }))
    );
    assert_eq!(audit(&engine), before);
}

fn structural_replacement(parent_path: Vec<u32>) -> crate::yrs_engine::StructuralReplacement {
    crate::yrs_engine::StructuralReplacement::new(
        parent_path,
        0,
        0,
        Fragment::empty(),
        Selection::cursor(0),
    )
}

// Deterministic structural-target ceilings are operation limits, not
// allocation-class resource exhaustion.
#[test]
fn structural_target_depth_excess_is_an_operation_limit_not_resource_exhaustion() {
    let engine = engine(PLAIN);
    let document = engine.document().unwrap().clone();
    let limits = ResourceLimits {
        max_document_depth: 1,
        ..ResourceLimits::default()
    };
    let error = super::resolve_structural_window(
        7,
        0,
        &document,
        &structural_replacement(vec![0, 0]),
        &limits,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "maxDocumentDepth" }))
    );
}

#[test]
fn structural_target_traversal_work_excess_is_an_operation_limit_not_resource_exhaustion() {
    let engine = engine(PLAIN);
    let document = engine.document().unwrap().clone();
    let limits = ResourceLimits {
        max_document_nodes: 1,
        ..ResourceLimits::default()
    };
    let error = super::resolve_structural_window(
        7,
        0,
        &document,
        &structural_replacement(vec![0, 0]),
        &limits,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "maxDocumentNodes" }))
    );
}

#[test]
fn deep_structural_replacement_maps_to_operation_limit_at_compile_and_session_boundary() {
    let engine = engine(PLAIN);
    let error = engine
        .compile_typed_transaction(transaction(
            &engine,
            vec![TypedOperation::ReplaceStructure(structural_replacement(
                vec![0; 257],
            ))],
        ))
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.operation_index, Some(0));
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "maxDocumentDepth" }))
    );
    let session_error = crate::session::SessionError::from_operation(
        error,
        crate::session::OperationFailureClass::ExistingStableCode,
    );
    assert_eq!(session_error.domain, crate::session::ErrorDomain::Operation);
    assert_eq!(session_error.code, "OPERATION_LIMIT_EXCEEDED");
}
