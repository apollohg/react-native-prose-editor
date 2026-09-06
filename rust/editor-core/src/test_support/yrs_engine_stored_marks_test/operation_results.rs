#[test]
fn compatible_adjacent_delete_and_caret_split_preserve_but_away_structural_operations_clear() {
    let mut deletion = engine();
    import_marked_text(&mut deletion);
    apply(
        &mut deletion,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut deletion,
        3,
        vec![TypedOperation::DeleteRange { range: range(0, 1) }],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(mark_types(&deletion), Some(vec!["bold", "italic"]));

    let mut structural = engine();
    import_marked_text(&mut structural);
    apply(
        &mut structural,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut structural,
        3,
        vec![TypedOperation::SplitBlock {
            at: before_point(1),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        SelectionIntent::Preserve,
    );
    // A split *at* the caret is Return: the active formatting continues onto
    // the new block, so the stored set survives.
    assert_eq!(mark_types(&structural), Some(vec!["bold", "italic"]));

    let mut structural_away = engine();
    import_marked_text(&mut structural_away);
    apply(
        &mut structural_away,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut structural_away,
        3,
        vec![TypedOperation::SplitBlock {
            at: before_point(0),
            node_type: "paragraph".into(),
            attrs: HashMap::new(),
        }],
        SelectionIntent::Preserve,
    );
    // A split away from the caret is an ordinary structural edit and still
    // clears the stored set.
    assert_eq!(structural_away.stored_marks(), None);

    let mut structural_delete = engine();
    structural_delete
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "a", "marks": [{"type": "bold"}]},
                        {"type": "hardBreak"},
                        {"type": "text", "text": "b"}
                    ]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        &mut structural_delete,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: before_point(1),
            head: before_point(1),
        }),
    );
    apply(
        &mut structural_delete,
        2,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: before_point(1),
                to: before_point(1),
            },
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert!(structural_delete.stored_marks().is_some());
    apply(
        &mut structural_delete,
        3,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: before_point(1),
                to: before_point(2),
            },
        }],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(structural_delete.stored_marks(), None);
}

#[test]
fn later_away_operation_result_cannot_reuse_an_earlier_input_exemption() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));

    apply(
        &mut engine,
        3,
        vec![
            TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold"), mark("italic")],
            },
            TypedOperation::DeleteRange { range: range(0, 0) },
        ],
        SelectionIntent::UseOperationResult,
    );

    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn no_op_result_at_the_fully_mapped_caret_preserves_stored_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );

    apply(
        &mut engine,
        3,
        vec![
            TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold"), mark("italic")],
            },
            TypedOperation::DeleteRange { range: range(1, 1) },
        ],
        SelectionIntent::UseOperationResult,
    );

    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn caret_mark_result_after_compatible_input_keeps_the_updated_stored_set() {
    for (final_operation, expected) in [
        (
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("strike"),
            },
            vec!["bold", "italic", "strike"],
        ),
        (
            TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "italic".into(),
            },
            vec!["bold"],
        ),
        (
            TypedOperation::ReplaceMark {
                range: range(1, 1),
                mark: link("https://example.com/after-input"),
            },
            vec!["bold", "italic", "link"],
        ),
    ] {
        let mut engine = engine();
        import_marked_text(&mut engine);
        apply(
            &mut engine,
            2,
            vec![TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("italic"),
            }],
            SelectionIntent::Preserve,
        );

        apply(
            &mut engine,
            3,
            vec![
                TypedOperation::InsertText {
                    at: point(1),
                    text: "x".into(),
                    marks: vec![mark("bold"), mark("italic")],
                },
                final_operation,
            ],
            SelectionIntent::UseOperationResult,
        );

        assert_eq!(mark_types(&engine), Some(expected));
    }
}

#[test]
fn net_stored_mark_state_in_one_envelope_controls_revision() {
    let mut marked_engine = engine();
    import_marked_text(&mut marked_engine);
    apply(
        &mut marked_engine,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    let state_revision = marked_engine.state_revision();
    let unchanged = apply(
        &mut marked_engine,
        3,
        vec![
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
            TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert!(!unchanged.changed);
    assert_eq!(unchanged.state_revision, state_revision);
    assert_eq!(marked_engine.stored_marks(), Some([].as_slice()));

    let mut plain = engine();
    let changed = apply(
        &mut plain,
        1,
        vec![
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
            TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert!(changed.changed);
    assert_eq!(plain.stored_marks(), Some([].as_slice()));
}

#[test]
fn duplicate_and_unknown_mark_inputs_reject_with_operation_attribution() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().map(<[Mark]>::to_vec),
    );
    let duplicate = engine
        .apply_typed_transaction(transaction(
            &engine,
            2,
            vec![TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: vec![mark("bold"), mark("bold")],
            }],
            SelectionIntent::UseOperationResult,
        ))
        .unwrap_err();
    assert_eq!(duplicate.code, "OPERATION_INVALID");
    assert_eq!(duplicate.operation_index, Some(0));
    assert_eq!(
        before,
        (
            engine.encoded_state().unwrap(),
            engine.revision(),
            engine.state_revision(),
            engine.stored_marks().map(<[Mark]>::to_vec),
        )
    );

    let unknown = engine
        .apply_typed_transaction(transaction(
            &engine,
            3,
            vec![TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "mystery".into(),
            }],
            SelectionIntent::Preserve,
        ))
        .unwrap_err();
    assert_eq!(unknown.code, "OPERATION_INVALID");
    assert_eq!(unknown.operation_index, Some(0));
}

#[test]
fn inherited_identical_add_and_replace_preserve_none_without_revision() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    assert_eq!(engine.stored_marks(), None);
    let state_revision = engine.state_revision();

    for (request_id, operation) in [
        (
            2,
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
        ),
        (
            3,
            TypedOperation::ReplaceMark {
                range: range(1, 1),
                mark: mark("bold"),
            },
        ),
    ] {
        let commit = apply(
            &mut engine,
            request_id,
            vec![operation],
            SelectionIntent::Preserve,
        );
        assert!(!commit.changed);
        assert_eq!(commit.state_revision, state_revision);
        assert_eq!(engine.stored_marks(), None);
    }
}

#[test]
fn add_same_type_with_different_attrs_rejects_for_caret_and_range() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": "ab",
                        "marks": [{ "type": "link", "attrs": { "href": "https://old" } }]
                    }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    apply(
        &mut engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(1),
            head: point(1),
        }),
    );
    let before = (
        engine.encoded_state().unwrap(),
        engine.revision(),
        engine.state_revision(),
        engine.stored_marks().map(<[Mark]>::to_vec),
    );
    for (request_id, target) in [(2, range(1, 1)), (3, range(0, 2))] {
        let error = engine
            .apply_typed_transaction(transaction(
                &engine,
                request_id,
                vec![TypedOperation::AddMark {
                    range: target,
                    mark: link("https://new"),
                }],
                SelectionIntent::Preserve,
            ))
            .unwrap_err();
        assert_eq!(error.code, "OPERATION_INVALID");
        assert_eq!(error.operation_index, Some(0));
        assert_eq!(
            before,
            (
                engine.encoded_state().unwrap(),
                engine.revision(),
                engine.state_revision(),
                engine.stored_marks().map(<[Mark]>::to_vec),
            )
        );
    }
}

#[test]
fn sequential_preview_marks_feed_later_collapsed_mark_operation() {
    let mut engine = engine();
    import_plain_text(&mut engine, "ab");
    apply(
        &mut engine,
        2,
        vec![
            TypedOperation::AddMark {
                range: range(0, 2),
                mark: mark("bold"),
            },
            TypedOperation::AddMark {
                range: range(1, 1),
                mark: mark("italic"),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn use_result_away_and_noncollapsed_results_clear_existing_stored_marks() {
    for operation in [
        TypedOperation::AddMark {
            range: range(0, 0),
            mark: mark("italic"),
        },
        TypedOperation::AddMark {
            range: range(0, 2),
            mark: mark("bold"),
        },
        TypedOperation::DeleteRange { range: range(0, 0) },
    ] {
        let mut engine = engine();
        import_marked_text(&mut engine);
        apply(
            &mut engine,
            2,
            vec![TypedOperation::RemoveMark {
                range: range(1, 1),
                mark_type: "bold".into(),
            }],
            SelectionIntent::Preserve,
        );
        assert!(engine.stored_marks().is_some());
        apply(
            &mut engine,
            3,
            vec![operation],
            SelectionIntent::UseOperationResult,
        );
        assert_eq!(engine.stored_marks(), None);
    }
}

#[test]
fn opposite_affinity_after_prior_insert_does_not_mutate_stored_marks() {
    let mut engine = engine();
    apply(
        &mut engine,
        1,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), Some([].as_slice()));
    let caret_affinity = match engine.relative_selection().unwrap() {
        crate::yrs_engine::RelativeSelection::Text { head, .. } => head.affinity,
        _ => panic!("expected collapsed text selection"),
    };
    let opposite = match caret_affinity {
        Affinity::Before => Affinity::After,
        Affinity::After => Affinity::Before,
    };
    apply(
        &mut engine,
        2,
        vec![
            TypedOperation::InsertText {
                at: affinity_point(1, caret_affinity),
                text: "x".into(),
                marks: vec![],
            },
            TypedOperation::AddMark {
                range: RevisionedRange {
                    from: affinity_point(1, opposite),
                    to: affinity_point(1, opposite),
                },
                mark: mark("italic"),
            },
        ],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), Some([].as_slice()));
}

#[test]
fn no_op_structural_operation_plus_compatible_insert_preserves_stored_marks() {
    let mut engine = engine();
    engine
        .import_json(
            &serde_json::json!({
                "type": "doc",
                "content": [{
                    "type": "orderedList",
                    "attrs": { "start": 1 },
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{
                                "type": "text",
                                "text": "ab",
                                "marks": [{ "type": "bold" }]
                            }]
                        }]
                    }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let block = engine.position_map().unwrap().block(0).unwrap();
    let caret = block.scalar_start + block.scalar_prefix_len + 1;
    apply(
        &mut engine,
        1,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(caret),
            head: point(caret),
        }),
    );
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(caret, caret),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    let caret_affinity = match engine.relative_selection().unwrap() {
        crate::yrs_engine::RelativeSelection::Text { head, .. } => head.affinity,
        _ => panic!("expected collapsed text selection"),
    };
    apply(
        &mut engine,
        3,
        vec![
            TypedOperation::UpdateNodeAttrs {
                at: before_point(0),
                attrs: serde_json::from_value(serde_json::json!({ "start": 1 })).unwrap(),
            },
            TypedOperation::InsertText {
                at: affinity_point(caret, caret_affinity),
                text: "x".into(),
                marks: vec![mark("bold"), mark("italic")],
            },
        ],
        SelectionIntent::UseOperationResult,
    );
    assert_eq!(mark_types(&engine), Some(vec!["bold", "italic"]));
}

#[test]
fn matching_marks_inserted_away_from_caret_clear_stored_marks() {
    let mut engine = engine();
    import_marked_text(&mut engine);
    apply(
        &mut engine,
        2,
        vec![TypedOperation::AddMark {
            range: range(1, 1),
            mark: mark("italic"),
        }],
        SelectionIntent::Preserve,
    );
    apply(
        &mut engine,
        3,
        vec![TypedOperation::InsertText {
            at: point(0),
            text: "x".into(),
            marks: vec![mark("bold"), mark("italic")],
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn snapshot_restore_is_document_scoped_and_never_transfers_local_stored_marks() {
    let mut target = scoped_engine();
    import_marked_text(&mut target);
    let exact = target.export_snapshot().unwrap();
    apply(
        &mut target,
        2,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert_eq!(target.stored_marks(), Some([].as_slice()));
    let exact_commit = target.restore_snapshot(&exact).unwrap();
    assert!(!exact_commit.changed);
    assert_eq!(target.stored_marks(), Some([].as_slice()));

    let mut rejected = exact.clone();
    rejected.format_version += 1;
    assert!(target.restore_snapshot(&rejected).is_err());
    assert_eq!(target.stored_marks(), Some([].as_slice()));

    let mut donor = scoped_engine();
    import_plain_text(&mut donor, "changed");
    let changed = donor.export_snapshot().unwrap();
    let changed_commit = target.restore_snapshot(&changed).unwrap();
    assert!(changed_commit.changed);
    assert_eq!(target.stored_marks(), None);

    let mut receiver = scoped_engine();
    apply(
        &mut receiver,
        1,
        vec![TypedOperation::RemoveMark {
            range: range(1, 1),
            mark_type: "bold".into(),
        }],
        SelectionIntent::Preserve,
    );
    assert!(receiver.stored_marks().is_some());
    receiver.restore_snapshot(&exact).unwrap();
    assert_eq!(receiver.stored_marks(), None);
}
