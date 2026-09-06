#[test]
fn tight_history_metadata_budget_falls_back_to_full_candidate_derivation() {
    let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
        max_derived_output_bytes: 2 * (512 + "prosemirror".len() + 2),
        ..crate::yrs_engine::EditingLimits::default()
    });
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    engine.undo_with_result(108_005).unwrap().unwrap();

    assert_eq!(
        engine.document_json().unwrap(),
        serde_json::from_str::<serde_json::Value>(
            r#"{"type":"doc","content":[{"type":"paragraph"}]}"#,
        )
        .unwrap()
    );
    let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    assert!(full_passes.canonical_projections > 0);
    assert!(full_passes.canonical_serializations > 0);
    assert!(full_passes.canonical_hashes > 0);
}

#[test]
fn deep_wide_history_snapshot_budget_accounts_for_spilled_position_paths() {
    fn deep_wide_document() -> serde_json::Value {
        let mut content = (0..24)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": format!("row {index}")}]
                })
            })
            .collect::<Vec<_>>();
        for _ in 0..10 {
            content = vec![json!({"type": "blockquote", "content": content})];
        }
        json!({"type": "doc", "content": content})
    }

    fn insert(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    let document = deep_wide_document();
    let mut probe = transaction_engine();
    probe
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let compiled = probe
        .compile_typed_transaction(insert(&probe, 108_006))
        .unwrap();
    let after = compiled.preview_derivations.as_ref().unwrap();
    let before = probe.derived_state.as_ref().unwrap();
    let before_retained =
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &before.document,
                canonical_artifact: &before.canonical_artifact,
                position_map: &before.position_map,
                rendered_text: &before.rendered_text,
                render_blocks: &before.render_blocks,
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap();
    let after_retained =
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &compiled.preview,
                canonical_artifact: compiled.canonical_artifact.as_ref().unwrap(),
                position_map: &after.position_map,
                rendered_text: &after.rendered_text,
                render_blocks: &crate::render::incremental::CachedRenderBlocks::build(
                    &compiled.preview,
                    &probe.schema,
                    &probe.resource_limits,
                )
                .unwrap(),
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap();
    let exact_budget =
        super::history_metadata_bytes(before.stored_marks.as_deref(), &probe.fragment_name)
            .checked_add(super::history_metadata_bytes(None, &probe.fragment_name))
            .and_then(|bytes| bytes.checked_add(before_retained.get()))
            .and_then(|bytes| bytes.checked_add(after_retained.get()))
            .unwrap();

    let run = |limit, request_id| {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        engine.editing_limits.max_derived_output_bytes = limit;
        engine
            .apply_typed_transaction(insert(&engine, request_id))
            .unwrap();
        assert!(
            engine.can_undo(),
            "base history capture must remain admitted"
        );
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(request_id + 1).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };

    let exact_passes = run(exact_budget, 108_007);
    assert_eq!(
        exact_passes.canonical_projections, 0,
        "the exact retained bound should admit the optional snapshots"
    );

    let full_passes = run(exact_budget - 1, 108_009);
    assert!(
        full_passes.canonical_projections > 0,
        "one under the retained bound must omit only the optional snapshots"
    );
}

#[test]
fn history_snapshot_charge_tracks_spare_node_string_capacity() {
    const SPARE_CAPACITY: usize = 1024 * 1024;

    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        let mut node_type = String::with_capacity(SPARE_CAPACITY);
        node_type.push_str("hardBreak");
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::void(node_type, HashMap::new()),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    fn snapshot_charge(engine: &YrsDocumentEngine) -> usize {
        let state = engine.derived_state.as_ref().unwrap();
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &engine.schema_fingerprint,
                fragment_name: &engine.fragment_name,
                scope: engine.scope.as_ref(),
            },
        )
        .unwrap()
        .get()
    }

    let before_probe =
        fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    let before_charge = snapshot_charge(&before_probe);
    let before_metadata =
        super::history_metadata_bytes(before_probe.stored_marks(), &before_probe.fragment_name);
    let mut after_probe =
        fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    after_probe
        .apply_typed_transaction(transaction(&after_probe, 108_020))
        .unwrap();
    let after_charge = snapshot_charge(&after_probe);
    assert!(after_charge >= SPARE_CAPACITY);
    let exact = before_metadata
        .checked_add(super::history_metadata_bytes(
            after_probe.stored_marks(),
            &after_probe.fragment_name,
        ))
        .and_then(|bytes| bytes.checked_add(before_charge))
        .and_then(|bytes| bytes.checked_add(after_charge))
        .unwrap();

    for (limit, expect_fast, request_id) in [(exact, true, 108_021), (exact - 1, false, 108_023)] {
        let mut engine = fixture(limit);
        engine
            .apply_typed_transaction(transaction(&engine, request_id))
            .unwrap();
        assert!(engine.can_undo());
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(request_id + 1).unwrap().unwrap();
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_projections == 0, expect_fast);
    }
}

#[test]
fn stored_mark_metadata_accounts_spare_hash_capacity_at_exact_boundary() {
    const SPARE_ENTRIES: usize = 32 * 1024;

    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 108_030, 1, 1);
        let mut attrs = HashMap::with_capacity(SPARE_ENTRIES);
        attrs.insert("href".into(), json!("x"));
        engine
            .apply_command(
                108_031,
                TypedCommand::SetMark {
                    mark_type: "link".into(),
                    attrs,
                },
            )
            .unwrap()
            .unwrap();
        assert!(engine.stored_marks().is_some());
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: engine.stored_marks().unwrap().to_vec(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    let mut probe = fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    let before_metadata = super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name);
    probe
        .apply_typed_transaction(transaction(&probe, 108_032))
        .unwrap();
    let exact = before_metadata
        .checked_add(super::history_metadata_bytes(
            probe.stored_marks(),
            &probe.fragment_name,
        ))
        .unwrap();

    let mut accepted = fixture(exact);
    accepted
        .apply_typed_transaction(transaction(&accepted, 108_033))
        .unwrap();
    assert!(accepted.can_undo());

    let mut rejected = fixture(exact - 1);
    let before = atomic_audit(&rejected);
    let error = rejected
        .apply_typed_transaction(transaction(&rejected, 108_034))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(atomic_audit(&rejected), before);
}

#[test]
fn compatible_auto_capture_admits_exact_after_only_metadata_increment() {
    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn insert(engine: &YrsDocumentEngine, request_id: u64, offset: u32) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let default_limit = crate::yrs_engine::EditingLimits::default().max_derived_output_bytes;
    let mut probe = fixture(default_limit);
    probe
        .apply_typed_transaction(insert(&probe, 108_040, 1))
        .unwrap();
    let retained_before_second = probe.history.replay_metadata_bytes_for_test();
    let second_before_metadata = {
        let state = probe.derived_state.as_ref().unwrap();
        let retained = crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap()
        .get();
        super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name)
            .checked_add(retained)
            .unwrap()
    };
    probe
        .apply_typed_transaction(insert(&probe, 108_041, 2))
        .unwrap();
    let second_after_metadata = probe
        .history
        .replay_metadata_bytes_for_test()
        .checked_sub(retained_before_second)
        .unwrap();
    let exact = retained_before_second
        .checked_add(second_after_metadata)
        .unwrap();
    assert!(
        exact
            < retained_before_second
                .checked_add(second_before_metadata)
                .and_then(|bytes| bytes.checked_add(second_after_metadata))
                .unwrap()
    );

    let mut engine = fixture(exact);
    engine
        .apply_typed_transaction(insert(&engine, 108_042, 1))
        .unwrap();
    engine
        .apply_typed_transaction(insert(&engine, 108_043, 2))
        .unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "axxb");
    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    engine.undo_with_result(108_044).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "ab");
    assert!(!engine.can_undo(), "compatible edits must remain one group");
    assert_eq!(
        crate::yrs_engine::observability::take_full_pass_counts_for_test().canonical_projections,
        0,
        "the exact boundary keeps optional document snapshots enabled"
    );
}
