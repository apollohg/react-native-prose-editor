#[test]
fn prepared_candidate_seals_actual_clock_for_redundant_inherited_mark_insert() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let local_client = engine.doc.client_id();

    let first = engine
        .compile_typed_transaction(marked_insert_transaction(&engine, 70_109, "a"))
        .unwrap();
    assert_eq!(first.authored_clock_units, 3);
    let before_first = engine.doc.transact().state_vector().get(&local_client);
    engine.apply_compiled_transaction(first, true).unwrap();
    let after_first = engine.doc.transact().state_vector().get(&local_client);
    assert_eq!(after_first - before_first, 3);

    let second = engine
        .compile_typed_transaction(marked_insert_transaction(&engine, 70_110, "b"))
        .unwrap();
    assert_eq!(second.authored_clock_units, 3);
    let before_second = engine.doc.transact().state_vector().get(&local_client);
    engine.apply_compiled_transaction(second, true).unwrap();
    let after_second = engine.doc.transact().state_vector().get(&local_client);

    assert_eq!(after_second - before_second, 1);
    assert_prepared_candidate_state_vector_exact(&engine);
}

#[test]
fn prepared_candidate_bounds_inherited_format_suspension_at_text_boundaries() {
    struct Case {
        name: &'static str,
        source: &'static str,
        offset: u32,
        inserted: &'static str,
        marks: Vec<Mark>,
        expected_bound: u64,
    }

    let bold = || Mark::new("bold".into(), HashMap::new());
    let italic = || Mark::new("italic".into(), HashMap::new());
    let cases = [
        Case {
            name: "plain at start",
            source: "ab",
            offset: 0,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "plain inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "plain at end",
            source: "ab",
            offset: 2,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "same mark inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![bold()],
            expected_bound: 3,
        },
        Case {
            name: "different mark inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![italic()],
            expected_bound: 5,
        },
        Case {
            name: "plain unicode inside",
            source: "😀b",
            offset: 1,
            inserted: "🦀",
            marks: vec![],
            expected_bound: 4,
        },
    ];

    for (index, case) in cases.into_iter().enumerate() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{
                            "type": "text",
                            "text": case.source,
                            "marks": [{ "type": "bold" }]
                        }]
                    }]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let request_id = 70_120 + u64::try_from(index).unwrap();
        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: case.offset,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: case.inserted.into(),
                    marks: case.marks,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_eq!(
            compiled.authored_clock_units, case.expected_bound,
            "{}",
            case.name
        );
        let local_client = engine.doc.client_id();
        let before = engine.doc.transact().state_vector().get(&local_client);
        engine.apply_compiled_transaction(compiled, true).unwrap();
        let after = engine.doc.transact().state_vector().get(&local_client);
        assert!(
            u64::from(after - before) <= case.expected_bound,
            "{}",
            case.name
        );
        assert_prepared_candidate_state_vector_exact(&engine);
    }

    let mut boundary = transaction_engine();
    boundary
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"bold"}]},{"type":"text","text":"b","marks":[{"type":"italic"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let compiled = boundary
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_126,
            base_document_revision: boundary.revision(),
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
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    // The lowering selects one exact storage target at this semantic
    // boundary; only that target's touching bold run contributes.
    assert_eq!(compiled.authored_clock_units, 3);
    boundary.apply_compiled_transaction(compiled, true).unwrap();
    assert_prepared_candidate_state_vector_exact(&boundary);

    let mut delete_then_insert = transaction_engine();
    delete_then_insert
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab","marks":[{"type":"bold"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let compiled = delete_then_insert
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_127,
            base_document_revision: delete_then_insert.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![
                TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: 0,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::Before,
                        },
                    },
                },
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 0,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                },
            ],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(compiled.authored_clock_units, 3);
    delete_then_insert
        .apply_compiled_transaction(compiled, true)
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&delete_then_insert);
}

#[test]
fn prepared_candidate_cache_failure_is_private_atomic_and_falls_back_once() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before = atomic_audit(&engine);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));
    reset_encoded_state_reuse_counts_for_test();

    let error = engine
        .apply_typed_transaction(insert_transaction(&engine, 70_105))
        .expect_err("candidate encoding failpoint must reject before the live write");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert!(engine.prepared_candidate_cache.is_none());
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
    reset_prepared_candidate_cache_counts_for_test();
    reset_encoded_state_reuse_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_106))
        .unwrap();

    assert!(engine.prepared_candidate_cache.is_some());
    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

#[test]
fn prepared_candidate_cache_revalidates_stale_revision_seal_before_reuse() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .document_revision = engine.revision.saturating_add(1);
    reset_prepared_candidate_cache_counts_for_test();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_108))
        .unwrap();
    let cache_counts = take_prepared_candidate_cache_counts_for_test();
    let lookup_counts = take_localized_lookup_counts_for_test();
    let cached_encoded = super::encode_state_bounded(
        &engine.prepared_candidate_cache.as_ref().unwrap().doc,
        &engine.resource_limits,
    )
    .unwrap();

    assert_eq!(cache_counts, (0, 1));
    assert_eq!(lookup_counts, (1, 1, 1));
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert_eq!(cached_encoded, engine.encoded_state().unwrap());
}
