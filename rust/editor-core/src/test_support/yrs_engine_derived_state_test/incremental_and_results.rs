#[test]
fn incremental_and_fallback_position_updates_match_full_builds() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
            ]
        }),
    );
    assert_incremental_matches_full(&engine);

    let insert = transaction(
        &engine,
        8,
        vec![TypedOperation::InsertText {
            at: point(2, EditorOffsetKind::Scalar, Affinity::After),
            text: "😀".into(),
            marks: vec![],
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(insert).unwrap();
    assert_incremental_matches_full(&engine);

    let add_mark = transaction(
        &engine,
        81,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            },
            mark: Mark::new("bold".into(), Default::default()),
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(add_mark).unwrap();
    assert_incremental_matches_full(&engine);

    let delete = transaction(
        &engine,
        82,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            },
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(delete).unwrap();
    assert_incremental_matches_full(&engine);

    let replace = transaction(
        &engine,
        83,
        vec![TypedOperation::ReplaceRange {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            },
            content: Fragment::from(vec![Node::text("X".into(), vec![])]),
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(replace).unwrap();
    assert_incremental_matches_full(&engine);

    let split = transaction(
        &engine,
        9,
        vec![TypedOperation::SplitBlock {
            at: point(2, EditorOffsetKind::Scalar, Affinity::After),
            node_type: "paragraph".into(),
            attrs: Default::default(),
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(split).unwrap();
    assert_incremental_matches_full(&engine);

    let insert_node = transaction(
        &engine,
        84,
        vec![TypedOperation::InsertNode {
            at: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            node: Node::void("hardBreak".into(), Default::default()),
        }],
        SelectionIntent::Preserve,
    );
    engine.apply_typed_transaction(insert_node).unwrap();
    assert_incremental_matches_full(&engine);
}

#[test]
fn operation_result_selection_is_total_at_text_end_and_for_ranges() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"A😀B"}]}]}),
    );
    let insert = transaction(
        &engine,
        20,
        vec![TypedOperation::InsertText {
            at: point(3, EditorOffsetKind::Scalar, Affinity::After),
            text: "!".into(),
            marks: vec![],
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(insert).unwrap();
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected cursor")
    };
    assert_eq!((anchor.scalar, anchor.utf16), (4, 5));
    assert_eq!(anchor, head);

    let mark = transaction(
        &engine,
        21,
        vec![TypedOperation::AddMark {
            range: RevisionedRange {
                from: point(0, EditorOffsetKind::Scalar, Affinity::After),
                to: point(4, EditorOffsetKind::Scalar, Affinity::After),
            },
            mark: Mark::new("bold".into(), Default::default()),
        }],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(mark).unwrap();
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected range")
    };
    assert_eq!((anchor.scalar, head.scalar), (0, 4));
    assert_incremental_matches_full(&engine);

    let encoded = engine.encoded_state().unwrap();
    let revision = engine.revision();
    let state_revision = engine.state_revision();
    let selection_only = transaction(
        &engine,
        211,
        vec![TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: point(1, EditorOffsetKind::Scalar, Affinity::Before),
                to: point(1, EditorOffsetKind::Scalar, Affinity::Before),
            },
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = engine.apply_typed_transaction(selection_only).unwrap();
    assert!(commit.changed);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.state_revision(), state_revision + 1);

    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": {
                    "src": "asset://same",
                    "alt": null,
                    "title": null,
                    "width": null,
                    "height": null
                }
            }]
        }),
    );
    let select_all = transaction(
        &engine,
        212,
        vec![],
        SelectionIntent::Set(SelectionInput::All),
    );
    engine.apply_typed_transaction(select_all).unwrap();
    let encoded = engine.encoded_state().unwrap();
    let revision = engine.revision();
    let state_revision = engine.state_revision();
    let update_node = transaction(
        &engine,
        213,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
            attrs: serde_json::from_value(serde_json::json!({
                "src": "asset://same",
                "alt": null,
                "title": null,
                "width": null,
                "height": null
            }))
            .unwrap(),
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = engine.apply_typed_transaction(update_node).unwrap();
    assert!(commit.changed);
    assert_eq!(engine.encoded_state().unwrap(), encoded);
    assert_eq!(engine.revision(), revision);
    assert_eq!(engine.state_revision(), state_revision + 1);
    assert!(matches!(
        engine.resolved_selection(),
        Some(ResolvedSelection::Node { .. })
    ));
}

#[test]
fn update_node_attrs_operation_result_selects_only_void_nodes() {
    let mut container = engine();
    import(
        &mut container,
        serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "orderedList",
                "attrs": { "start": 1 },
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "one" }]
                    }]
                }]
            }]
        }),
    );
    let select_all = transaction(
        &container,
        214,
        vec![],
        SelectionIntent::Set(SelectionInput::All),
    );
    container.apply_typed_transaction(select_all).unwrap();
    let unchanged_container = transaction(
        &container,
        215,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
            attrs: serde_json::from_value(serde_json::json!({ "start": 1 })).unwrap(),
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = container
        .apply_typed_transaction(unchanged_container)
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(
        container.resolved_selection(),
        Some(&ResolvedSelection::All)
    );

    let update_container = transaction(
        &container,
        217,
        vec![TypedOperation::UpdateNodeAttrs {
            at: point(0, EditorOffsetKind::Scalar, Affinity::Before),
            attrs: serde_json::from_value(serde_json::json!({ "start": 2 })).unwrap(),
        }],
        SelectionIntent::UseOperationResult,
    );
    let commit = container.apply_typed_transaction(update_container).unwrap();
    assert!(commit.changed);
    assert_eq!(
        container.resolved_selection(),
        Some(&ResolvedSelection::All)
    );
}

#[test]
fn nonvoid_attrs_operation_result_maps_selection_through_prior_edits() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({
            "type": "doc",
            "content": [
                {
                    "type": "bulletList",
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "one" }]
                        }]
                    }]
                },
                {
                    "type": "orderedList",
                    "attrs": { "start": 1 },
                    "content": [{
                        "type": "listItem",
                        "content": [{
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": "two" }]
                        }]
                    }]
                }
            ]
        }),
    );
    let total_scalars = engine.position_map().unwrap().total_scalars();
    let set_cursor = transaction(
        &engine,
        218,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(3, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(3, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    engine.apply_typed_transaction(set_cursor).unwrap();
    let crate::yrs_engine::RelativeSelection::Text {
        anchor: relative_before,
        head: relative_before_head,
    } = engine.relative_selection().unwrap().clone()
    else {
        panic!("expected initial relative text cursor")
    };
    assert_eq!(relative_before, relative_before_head);

    let edit_then_attrs = transaction(
        &engine,
        219,
        vec![
            TypedOperation::UnwrapFromList {
                at: point(3, EditorOffsetKind::Scalar, Affinity::Before),
            },
            TypedOperation::UpdateNodeAttrs {
                at: point(
                    total_scalars - 1,
                    EditorOffsetKind::Scalar,
                    Affinity::Before,
                ),
                attrs: serde_json::from_value(serde_json::json!({ "start": 2 })).unwrap(),
            },
        ],
        SelectionIntent::UseOperationResult,
    );
    engine.apply_typed_transaction(edit_then_attrs).unwrap();
    let crate::yrs_engine::RelativeSelection::Text {
        anchor: relative_after,
        head: relative_after_head,
    } = engine.relative_selection().unwrap().clone()
    else {
        panic!("expected rematerialized relative text cursor")
    };
    assert_eq!(relative_after, relative_after_head);
    assert_eq!(relative_after.affinity, Affinity::Before);
    assert_ne!(
        relative_after.sticky, relative_before.sticky,
        "unwrap must delete the selected branch and exercise the mapped fallback"
    );
    let ResolvedSelection::Text { anchor, head } = engine.resolved_selection().unwrap() else {
        panic!("expected mapped text cursor")
    };
    assert_eq!(anchor, head);
    assert_eq!((anchor.document, anchor.scalar, anchor.utf16), (2, 1, 1));
}

#[test]
fn envelope_errors_precede_unrepresentable_explicit_selection() {
    let mut engine = engine();
    import(
        &mut engine,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"x"}]}]}),
    );
    let before = engine.encoded_state().unwrap();
    let mut stale = transaction(
        &engine,
        22,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
            head: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
        }),
    );
    stale.base_document_revision += 1;
    assert_eq!(
        engine.apply_typed_transaction(stale).unwrap_err().code,
        "REVISION_MISMATCH"
    );
    assert_eq!(engine.encoded_state().unwrap(), before);

    let mut invalid_origin = transaction(
        &engine,
        23,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
            head: point(u32::MAX, EditorOffsetKind::Utf16, Affinity::After),
        }),
    );
    invalid_origin.origin = TransactionOrigin::RemoteSync;
    assert_eq!(
        engine
            .apply_typed_transaction(invalid_origin)
            .unwrap_err()
            .code,
        "TRANSACTION_INVALID"
    );
    assert_eq!(engine.encoded_state().unwrap(), before);
}

#[test]
fn awaiting_restore_and_import_lifecycle_install_or_preserve_derived_state() {
    let mut source = scoped_engine(InitializationMode::LocalEmpty);
    import(
        &mut source,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}),
    );
    let snapshot = source.export_snapshot().unwrap();

    let mut target = scoped_engine(InitializationMode::AwaitRemote);
    assert!(!target.is_ready());
    assert!(target.position_map().is_none());
    assert!(target.relative_selection().is_none());
    assert!(target.resolved_selection().is_none());

    let mut direct_import = scoped_engine(InitializationMode::AwaitRemote);
    import(
        &mut direct_import,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"direct"}]}]}),
    );
    assert!(direct_import.is_ready());
    assert!(direct_import.resolved_selection().is_some());

    target.restore_snapshot(&snapshot).unwrap();
    assert!(target.is_ready());
    assert!(target.resolved_selection().is_some());
    assert_incremental_matches_full(&target);
    let restore_revision = target.revision();
    let restore_state_revision = target.state_revision();

    let select = transaction(
        &target,
        30,
        vec![],
        SelectionIntent::Set(SelectionInput::Text {
            anchor: point(2, EditorOffsetKind::Scalar, Affinity::Before),
            head: point(2, EditorOffsetKind::Scalar, Affinity::Before),
        }),
    );
    target.apply_typed_transaction(select).unwrap();
    let selected = target.relative_selection().cloned();
    let selected_state_revision = target.state_revision();

    let unchanged_snapshot = target.restore_snapshot(&snapshot).unwrap();
    assert!(!unchanged_snapshot.changed);
    assert_eq!(target.relative_selection().cloned(), selected);
    assert_eq!(target.revision(), restore_revision);
    assert_eq!(target.state_revision(), selected_state_revision);

    let current_json = target.document_json().unwrap().to_string();
    let unchanged_import = target
        .import_json(&current_json, TransactionOrigin::DocumentImport)
        .unwrap();
    assert!(!unchanged_import.changed);
    assert_eq!(target.relative_selection().cloned(), selected);
    assert_eq!(target.state_revision(), selected_state_revision);

    import(
        &mut target,
        serde_json::json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement"}]}]}),
    );
    assert_eq!(target.revision(), restore_revision + 1);
    assert_eq!(target.state_revision(), selected_state_revision + 1);
    assert_ne!(target.relative_selection().cloned(), selected);
    assert!(target.state_revision() > restore_state_revision);
    assert_incremental_matches_full(&target);
}
