#[test]
fn prepared_active_state_cache_allocation_and_budget_misses_are_optional() {
    use crate::yrs_engine::derived_state::{
        force_active_state_cache_allocation_failure_for_test,
        force_active_state_cache_budget_for_test,
        force_active_state_public_materialization_failure_for_test,
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 710_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
    }

    for budget_failure in [false, true] {
        let mut engine = fixture();
        reset_active_state_cache_counts_for_test();
        if budget_failure {
            force_active_state_cache_budget_for_test(Some(0));
        } else {
            force_active_state_cache_allocation_failure_for_test(true);
        }
        let result = engine
            .apply_command(710_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        force_active_state_cache_budget_for_test(None);
        force_active_state_cache_allocation_failure_for_test(false);

        assert!(result.changed);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc"
        );
        assert!(engine.can_undo());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 0, 0, 0, 1)
        );
    }

    let mut measured = fixture();
    let measured_result = measured
        .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let retained = measured
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap()
        .retained_bytes_for_test();

    let mut exact = fixture();
    reset_active_state_cache_counts_for_test();
    force_active_state_cache_budget_for_test(Some(retained));
    let exact_result = exact
        .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_active_state_cache_budget_for_test(None);
    assert_eq!(exact_result, measured_result);
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (1, 0, 1, 1, 1, 1, 0, 1, 1)
    );

    let mut under = fixture();
    reset_active_state_cache_counts_for_test();
    force_active_state_cache_budget_for_test(Some(retained - 1));
    let under_result = under
        .apply_command(710_010, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_active_state_cache_budget_for_test(None);
    assert_eq!(under_result, measured_result);
    assert_eq!(under.document_json(), measured.document_json());
    assert!(under
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_none());
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (1, 0, 1, 1, 1, 0, 0, 0, 1)
    );

    let mut materialization = measured;
    let mut baseline = exact;
    reset_active_state_cache_counts_for_test();
    force_active_state_public_materialization_failure_for_test(true);
    let materialized_result =
        materialization.apply_command(710_011, TypedCommand::InsertText { text: "y".into() });
    force_active_state_public_materialization_failure_for_test(false);
    let materialized_result = materialized_result.unwrap().unwrap();
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (1, 0, 1, 1, 1, 0, 1, 0, 1)
    );
    let baseline_result = baseline
        .apply_command(710_011, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .unwrap();
    assert_eq!(materialized_result, baseline_result);
    assert_eq!(materialization.document_json(), baseline.document_json());
    assert_eq!(materialization.can_undo(), baseline.can_undo());
    assert_eq!(materialization.can_redo(), baseline.can_redo());
    assert!(materialization
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_none());
}

#[test]
fn prepared_active_state_transition_tamper_falls_back_with_exact_parity() {
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 711_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(711_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        engine
    }

    fn compiled_insert(engine: &YrsDocumentEngine, request_id: u64) -> CompiledTransaction {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: "y".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must prepare a transaction")
        };
        engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap()
    }

    for (index, claim) in [
        "documentRevision",
        "stateRevision",
        "epoch",
        "schema",
        "resource",
        "editing",
        "maxLength",
        "selection",
        "relativeSelection",
        "legacySelection",
        "storedMarks",
        "structural",
        "resultSelection",
        "preview",
        "render",
        "lookup",
        "validation",
        "cachedPayloadIdentity",
    ]
    .into_iter()
    .enumerate()
    {
        let mut tampered = fixture();
        let mut generic = fixture();
        let request_id = 711_100 + u64::try_from(index).unwrap();
        let mut tampered_compiled = compiled_insert(&tampered, request_id);
        tampered_compiled
            .prepared_active_state_transition
            .as_mut()
            .unwrap()
            .tamper_for_test(claim);
        let mut generic_compiled = compiled_insert(&generic, request_id);
        generic_compiled.prepared_active_state_transition = None;

        reset_active_state_cache_counts_for_test();
        let tampered_result = tampered
            .apply_compiled_transaction(tampered_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 0, 0, 1, 0, 1),
            "{claim}"
        );
        let generic_result = generic
            .apply_compiled_transaction(generic_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(tampered_result, generic_result, "{claim}");
        assert_eq!(tampered.document_json(), generic.document_json(), "{claim}");
        assert_eq!(tampered.can_undo(), generic.can_undo(), "{claim}");
        assert_eq!(tampered.can_redo(), generic.can_redo(), "{claim}");
        assert!(tampered
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
    }

    for (index, current_claim) in [
        "missingCurrentCertificate",
        "replacedCurrentCertificate",
        "replacedCurrentPayload",
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = fixture();
        let compiled = compiled_insert(&engine, 711_500 + u64::try_from(index).unwrap());
        let state = engine.derived_state.as_mut().unwrap();
        match current_claim {
            "missingCurrentCertificate" => state.remove_active_state_certificate_for_test(),
            "replacedCurrentCertificate" => {
                state.replace_active_state_certificate_identity_for_test()
            }
            "replacedCurrentPayload" => state.replace_active_state_payload_identity_for_test(),
            _ => unreachable!(),
        }
        reset_active_state_cache_counts_for_test();
        let result = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert!(result.changed, "{current_claim}");
        let expected_drops = usize::from(current_claim != "missingCurrentCertificate");
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 0, 0, expected_drops, 0, 1),
            "{current_claim}"
        );
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_none());
    }
}

#[test]
fn prepared_active_state_cache_survives_post_result_rejection_by_identity() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 712_000,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    engine
        .apply_command(712_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let before = atomic_audit(&engine);
    let cache_before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap();

    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            712_002,
            TypedCommand::InsertText { text: "y".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("insert command must prepare a transaction")
    };
    let compiled = engine
        .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
        .unwrap();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::FinalPreflight));
    let rejected = engine.apply_compiled_transaction(compiled, true);
    set_atomic_failpoint_for_test(None);
    assert!(rejected.is_err());
    assert_eq!(atomic_audit(&engine), before);
    let cache_after = engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap();
    assert!(Arc::ptr_eq(&cache_before, &cache_after));
}

#[test]
fn prepared_active_state_certificate_is_cleared_by_changed_state_boundaries() {
    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 713_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(713_001, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .is_some());
        engine
    }

    let assert_cleared = |engine: &YrsDocumentEngine, boundary: &str| {
        assert!(
            engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_none(),
            "{boundary}"
        );
    };

    let mut selection = fixture();
    let point = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    selection
        .apply_typed_transaction(TypedTransaction {
            request_id: 713_010,
            base_document_revision: selection.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_cleared(&selection, "selection");

    let mut direct = fixture();
    let caret = direct
        .derived_state
        .as_ref()
        .unwrap()
        .resolved_selection
        .clone();
    let crate::yrs_engine::ResolvedSelection::Text { anchor, .. } = caret else {
        panic!("fixture retains a text caret")
    };
    direct
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 713_011,
            base_document_revision: direct.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: anchor.document,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "y".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert_cleared(&direct, "direct LocalInput");

    let mut undone = fixture();
    undone.undo(713_012).unwrap();
    assert_cleared(&undone, "undo");
    undone.redo(713_013).unwrap();
    assert_cleared(&undone, "redo");

    let mut stored_mark = fixture();
    stored_mark
        .apply_command(
            713_014,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_cleared(&stored_mark, "stored mark");

    let mut deleted = fixture();
    deleted
        .apply_command(713_015, TypedCommand::DeleteBackward)
        .unwrap()
        .unwrap();
    assert_cleared(&deleted, "prepared delete");

    let mut structural = fixture();
    structural
        .apply_command(713_016, TypedCommand::ToggleHeading { level: 2 })
        .unwrap()
        .unwrap();
    assert_cleared(&structural, "prepared structural command");

    let mut no_result = fixture();
    let crate::yrs_engine::ResolvedSelection::Text { anchor, .. } = no_result
        .derived_state
        .as_ref()
        .unwrap()
        .resolved_selection
        .clone()
    else {
        panic!("fixture retains a text caret")
    };
    no_result
        .apply_typed_transaction(TypedTransaction {
            request_id: 713_017,
            base_document_revision: no_result.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: anchor.document,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "z".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert_cleared(&no_result, "no-result changed transaction");

    let mut imported = fixture();
    imported
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replacement"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_cleared(&imported, "import");

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut restored = fixture();
    restored.restore_snapshot(&snapshot).unwrap();
    assert_cleared(&restored, "snapshot restore");

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut remote = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "active-cache".into(),
            lineage_id: "invalidation".into(),
        }),
    })
    .unwrap();
    remote
        .apply_remote_update_v1(713_020, &source.encoded_state().unwrap())
        .unwrap();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    remote
        .apply_typed_transaction(TypedTransaction {
            request_id: 713_021,
            base_document_revision: remote.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    remote
        .apply_command(713_022, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert!(remote
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_some());
    source
        .apply_typed_transaction(insert_transaction(&source, 713_023))
        .unwrap();
    let remote_vector = remote.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&remote_vector);
    assert!(
        remote
            .apply_remote_update_v1(713_024, &delta)
            .unwrap()
            .changed
    );
    assert_cleared(&remote, "accepted remote update");
}
