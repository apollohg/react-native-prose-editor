#[test]
fn history_snapshot_and_forced_fallback_match_affinity_and_stored_marks() {
    use crate::yrs_engine::derived_state::force_history_document_snapshot_fallback_for_test;

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                        {"type": "horizontalRule"},
                        {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                    ]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let boundary = |affinity| RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_052,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: boundary(Affinity::Before),
                    head: boundary(Affinity::After),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(
                108_051,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert!(engine.stored_marks().is_some());
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_053,
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
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        engine
    }

    fn local_state(
        engine: &YrsDocumentEngine,
    ) -> (
        serde_json::Value,
        Option<ResolvedSelection>,
        Option<Vec<crate::model::Mark>>,
        bool,
        bool,
    ) {
        (
            engine.document_json().unwrap(),
            engine.resolved_selection().cloned(),
            engine.stored_marks().map(<[_]>::to_vec),
            engine.can_undo(),
            engine.can_redo(),
        )
    }

    fn text_affinities(engine: &YrsDocumentEngine) -> (Affinity, Affinity) {
        let Some(crate::yrs_engine::RelativeSelection::Text { anchor, head }) =
            engine.relative_selection()
        else {
            panic!("history restores the captured text selection");
        };
        (anchor.affinity, head.affinity)
    }

    let mut fast = fixture();
    let mut fallback = fixture();

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    fast.undo_with_result(108_054).unwrap().unwrap();
    let fast_undo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    let fallback_undo_passes = {
        let _fallback = force_history_document_snapshot_fallback_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fallback.undo_with_result(108_054).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };
    assert_eq!(local_state(&fast), local_state(&fallback));
    assert_eq!(text_affinities(&fast), text_affinities(&fallback));
    assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
    assert_eq!(fast_undo_passes.canonical_projections, 0);
    assert!(fallback_undo_passes.canonical_projections > 0);

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    fast.redo_with_result(108_055).unwrap().unwrap();
    let fast_redo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    let fallback_redo_passes = {
        let _fallback = force_history_document_snapshot_fallback_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fallback.redo_with_result(108_055).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };
    assert_eq!(local_state(&fast), local_state(&fallback));
    assert_eq!(text_affinities(&fast), text_affinities(&fallback));
    assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
    assert_eq!(fast_redo_passes.canonical_projections, 0);
    assert!(fallback_redo_passes.canonical_projections > 0);
}

#[test]
fn history_snapshot_context_drift_falls_back_without_changing_undo_result() {
    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_060,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
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
        engine
    }

    for context in ["resource", "editing", "maxLength", "scope"] {
        let mut engine = fixture();
        match context {
            "resource" => {
                engine.resource_limits.max_document_depth =
                    engine.resource_limits.max_document_depth.saturating_add(1)
            }
            "editing" => {
                engine.editing_limits.max_operations_per_transaction = engine
                    .editing_limits
                    .max_operations_per_transaction
                    .saturating_add(1)
            }
            "maxLength" => engine.max_length = Some(100),
            "scope" => engine
                .scope
                .as_mut()
                .expect("fixture is document scoped")
                .lineage_id
                .push_str("-changed"),
            _ => unreachable!(),
        }

        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(108_061).unwrap().unwrap();
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(engine.document().unwrap().root().text_content(), "ab");
        assert!(
            passes.canonical_projections > 0,
            "{context} drift must reject snapshot reuse and run the fallback"
        );
    }
}

#[test]
fn invalid_history_stored_marks_precede_snapshot_publication_and_preserve_atomicity() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_070,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
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
    engine
        .history
        .replace_next_undo_stored_marks_for_test(vec![Mark::new("unknown".into(), HashMap::new())]);
    let before = atomic_audit(&engine);

    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = engine.undo_with_result(108_071);
    set_lookup_seed_hydration_failpoint_for_test(None);

    let error = result.expect_err("invalid history metadata must precede snapshot publication");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.request_id, 108_071);
    assert_eq!(
        error.message.as_ref(),
        "history metadata contains invalid stored marks: unknown mark 'unknown'"
    );
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn every_history_snapshot_semantic_fallback_precedes_seed_publication() {
    use crate::yrs_engine::derived_state::{
        force_history_document_snapshot_fallback_for_test,
        force_history_snapshot_semantic_fallback_for_test, HistorySnapshotSemanticFallbackForTest,
    };
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_072,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
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
        engine
    }

    for stage in [
        HistorySnapshotSemanticFallbackForTest::RenderIdentity,
        HistorySnapshotSemanticFallbackForTest::RelativeSelection,
        HistorySnapshotSemanticFallbackForTest::ResolvedSelection,
        HistorySnapshotSemanticFallbackForTest::ResolvedMismatch,
    ] {
        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let mut expected = fixture();
            let expected_result = {
                let _fallback = force_history_document_snapshot_fallback_for_test();
                expected.undo_with_result(108_073).unwrap().unwrap()
            };
            let mut actual = fixture();
            let actual_result = {
                let _fallback = force_history_snapshot_semantic_fallback_for_test(stage);
                set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
                let result = actual.undo_with_result(108_073);
                set_lookup_seed_hydration_failpoint_for_test(None);
                result.unwrap().unwrap()
            };

            assert_eq!(actual_result, expected_result, "{stage:?}/{failpoint:?}");
            assert_eq!(
                actual.document_json(),
                expected.document_json(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.resolved_selection(),
                expected.resolved_selection(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.stored_marks(),
                expected.stored_marks(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.can_undo(),
                expected.can_undo(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.can_redo(),
                expected.can_redo(),
                "{stage:?}/{failpoint:?}"
            );
        }
    }
}

#[test]
fn history_restore_request_relabeling_precedes_forced_semantic_fallback_and_probes() {
    use crate::yrs_engine::derived_state::{
        force_history_snapshot_semantic_fallback_for_test,
        history_document_snapshot_retained_bytes, DerivedStateCache,
        HistoryDocumentSnapshotRetainedInput, HistorySnapshotSemanticFallbackForTest,
    };
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let engine = transaction_engine();
    let state = engine.derived_state.as_ref().unwrap();
    let retained = history_document_snapshot_retained_bytes(HistoryDocumentSnapshotRetainedInput {
        document: &state.document,
        canonical_artifact: &state.canonical_artifact,
        position_map: &state.position_map,
        rendered_text: &state.rendered_text,
        render_blocks: &state.render_blocks,
        schema_fingerprint: &state.schema_fingerprint,
        fragment_name: &engine.fragment_name,
        scope: engine.scope.as_ref(),
    })
    .unwrap();
    let snapshot = state.capture_history_document_snapshot(
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.fragment_name,
        engine.scope.as_ref(),
        retained,
    );
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    for failpoint in [
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let (_, admission) = snapshot
            .prepare_candidate_read(
                108_074,
                &txn,
                &fragment,
                &engine.schema,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                &engine.fragment_name,
                engine.scope.as_ref(),
                engine.yrs_state_epoch,
                engine.revision,
            )
            .unwrap()
            .into_parts();
        let _fallback = force_history_snapshot_semantic_fallback_for_test(
            HistorySnapshotSemanticFallbackForTest::RenderIdentity,
        );
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = DerivedStateCache::restore_history_document_snapshot(
            108_075,
            &snapshot,
            admission.expect("matching read admits the retained snapshot"),
            &txn,
            &fragment,
            &engine.schema,
            &state.relative_selection,
            &state.resolved_selection,
            state.stored_marks.clone(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.revision,
            engine.state_revision,
            engine.yrs_state_epoch,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("request relabeling must precede semantic fallback");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(error.request_id, 108_075, "{failpoint:?}");
    }
}

#[test]
fn history_specific_initialization_keeps_candidate_limit_rejection_atomic() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.max_length = Some(2);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    to: RevisionedPosition {
                        offset: 3,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                },
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    let before = atomic_audit(&engine);

    let error = engine.undo_with_result(108_005).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn second_history_pop_max_length_drift_rejects_before_live_pop() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.max_length = Some(1);
    for (request_id, from, to) in [(108_006, 1, 2), (108_007, 0, 1)] {
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: from,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: to,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                    },
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
    }

    engine
        .undo(108_008)
        .unwrap()
        .expect("first pop must restore the one-character document");
    assert_eq!(engine.document().unwrap().root().text_content(), "a");
    let before = atomic_audit(&engine);

    let error = engine.undo(108_009).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(error.details, Some(json!({ "field": "maxLength" })));
    assert_eq!(atomic_audit(&engine), before);
    let repeated = engine.undo(108_010).unwrap_err();
    assert_eq!(repeated.code, error.code);
    assert_eq!(repeated.limit, error.limit);
    assert_eq!(repeated.actual, error.actual);
    assert_eq!(repeated.details, error.details);
    assert_eq!(atomic_audit(&engine), before);
}
