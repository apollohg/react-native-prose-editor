#[test]
fn revision_overflow_rejects_before_candidate_swap() {
    let mut engine =
        crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
    engine.revision = u64::MAX;
    engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
    let before_client = engine.client_id();
    let before_json = engine.document_json();
    let before_state = engine.encoded_state().unwrap();

    let error = engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
            crate::yrs_engine::TransactionOrigin::DocumentImport,
        )
        .unwrap_err();

    assert_eq!(error.code, "REVISION_OVERFLOW");
    assert_eq!(engine.revision(), u64::MAX);
    assert_eq!(engine.client_id(), before_client);
    assert_eq!(engine.document_json(), before_json);
    assert_eq!(engine.encoded_state().unwrap(), before_state);
}

#[test]
fn candidate_state_revision_and_epoch_overflow_reject_before_swap() {
    for field in ["stateRevision", "yrsStateEpoch"] {
        let mut engine = transaction_engine();
        if field == "stateRevision" {
            engine.state_revision = u64::MAX;
            engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        } else {
            engine.yrs_state_epoch = u64::MAX;
        }
        let before = atomic_audit(&engine);

        let error = engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"changed"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap_err();

        assert_eq!(error.code, "REVISION_OVERFLOW", "{field}");
        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
        assert_eq!(atomic_audit(&engine), before, "{field}");
    }
}

#[test]
fn identical_selection_is_no_op_even_when_state_revision_is_max() {
    let mut engine = transaction_engine();
    engine.state_revision = u64::MAX;
    if let Some(state) = &mut engine.derived_state {
        state.state_revision = u64::MAX;
    }
    let before = atomic_audit(&engine);
    let transaction = TypedTransaction {
        request_id: 90_001,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(crate::yrs_engine::SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: 0,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: 0,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::Before,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let commit = engine.apply_typed_transaction(transaction).unwrap();
    assert!(!commit.changed);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn snapshot_export_envelope_budget_has_exact_and_over_boundaries_without_mutation() {
    let mut engine =
        crate::yrs_engine::YrsDocumentEngine::new(crate::yrs_engine::YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
    let state = engine.encoded_state().unwrap();
    let metadata_bytes =
        "doc".len() + "lineage".len() + "prosemirror".len() + engine.schema_fingerprint().len();
    engine.resource_limits.max_input_bytes = metadata_bytes;
    engine.resource_limits.max_encoded_state_bytes = state.len();
    assert!(engine.export_snapshot().is_ok());

    let before_revision = engine.revision();
    let before_client = engine.client_id();
    let before_json = engine.document_json();
    engine.resource_limits.max_input_bytes = metadata_bytes - 1;
    let error = engine.export_snapshot().unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({"phase": "snapshotExport"}))
    );
    assert_eq!(engine.revision(), before_revision);
    assert_eq!(engine.client_id(), before_client);
    assert_eq!(engine.document_json(), before_json);
    assert_eq!(engine.encoded_state().unwrap(), state);
}

#[test]
fn typed_transaction_rejects_every_revision_or_epoch_overflow_before_mutation() {
    for field in ["documentRevision", "stateRevision", "yrsStateEpoch"] {
        let mut engine = transaction_engine();
        match field {
            "documentRevision" => {
                engine.revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().document_revision = u64::MAX;
            }
            "stateRevision" => {
                engine.state_revision = u64::MAX;
                engine.derived_state.as_mut().unwrap().state_revision = u64::MAX;
            }
            "yrsStateEpoch" => engine.yrs_state_epoch = u64::MAX,
            _ => unreachable!(),
        }
        let transaction = insert_transaction(&engine, 71);
        let before = atomic_audit(&engine);

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{field}");
        assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
        assert_eq!(atomic_audit(&engine), before, "{field}");
    }
}

#[test]
fn compiled_transaction_epoch_is_checked_before_yrs_metadata_revalidation() {
    for changed in [true, false] {
        let mut engine = transaction_engine();
        let transaction = if changed {
            insert_transaction(&engine, 72)
        } else {
            TypedTransaction {
                request_id: 72,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            }
        };
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        engine.yrs_state_epoch += 1;
        let before = atomic_audit(&engine);

        let error = engine
            .apply_compiled_transaction(compiled, false)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
        assert!(error.message.contains("stale"), "changed={changed}");
        assert_eq!(atomic_audit(&engine), before, "changed={changed}");
    }
}

#[test]
fn compiled_transaction_state_revision_is_checked_before_result_or_no_op_work() {
    for changed in [true, false] {
        let mut engine = transaction_engine();
        let transaction = if changed {
            insert_transaction(&engine, 72_001)
        } else {
            TypedTransaction {
                request_id: 72_001,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Skip,
            }
        };
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        let seed = engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .clone();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 72_002,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert!(Arc::ptr_eq(
            &seed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
        let before = atomic_audit(&engine);

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "changed={changed}");
        assert!(error.message.contains("stale"), "changed={changed}");
        assert_eq!(atomic_audit(&engine), before, "changed={changed}");
    }
}

#[test]
fn projected_encoded_ceiling_accepts_exact_and_rejects_one_under_without_new_clock() {
    let mut exact = transaction_engine();
    let exact_transaction = insert_transaction(&exact, 73);
    let exact_compiled = exact
        .compile_typed_transaction(exact_transaction.clone())
        .unwrap();
    let exact_limit = exact
        .encoded_state()
        .unwrap()
        .len()
        .checked_add(exact_compiled.encoded_growth_bound)
        .unwrap();
    exact.resource_limits.max_encoded_state_bytes = exact_limit;

    let commit = exact.apply_typed_transaction(exact_transaction).unwrap();

    assert!(commit.changed);
    assert!(exact.encoded_state().unwrap().len() <= exact_limit);

    let mut one_under = transaction_engine();
    let rejected_transaction = insert_transaction(&one_under, 74);
    let rejected_compiled = one_under
        .compile_typed_transaction(rejected_transaction.clone())
        .unwrap();
    let rejected_limit = one_under
        .encoded_state()
        .unwrap()
        .len()
        .checked_add(rejected_compiled.encoded_growth_bound)
        .unwrap()
        - 1;
    one_under.resource_limits.max_encoded_state_bytes = rejected_limit;
    let before = atomic_audit(&one_under);

    let error = one_under
        .apply_typed_transaction(rejected_transaction)
        .unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxEncodedStateBytes" }))
    );
    assert_eq!(error.limit, Some(rejected_limit as u64));
    assert_eq!(error.actual, Some((rejected_limit + 1) as u64));
    assert_eq!(atomic_audit(&one_under), before);
}

#[test]
fn canonical_cache_output_accepts_exact_rejects_one_under_and_reuses_empty_noop_cache() {
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x" }]
        }]
    });
    let exact_bytes = serde_json::to_vec(&expected).unwrap().len();

    let mut exact = transaction_engine();
    exact.editing_limits.max_derived_output_bytes = exact_bytes;
    let transaction = insert_transaction(&exact, 77);
    exact.apply_typed_transaction(transaction).unwrap();
    assert_eq!(exact.document_json(), Some(expected));

    let mut one_under = transaction_engine();
    one_under.editing_limits.max_derived_output_bytes = exact_bytes - 1;
    let transaction = insert_transaction(&one_under, 78);
    let before = atomic_audit(&one_under);
    let error = one_under.apply_typed_transaction(transaction).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some((exact_bytes - 1) as u64));
    assert_eq!(error.actual, Some(exact_bytes as u64));
    assert_eq!(atomic_audit(&one_under), before);

    let mut empty_noop = transaction_engine();
    empty_noop.editing_limits.max_derived_output_bytes = 1;
    let transaction = TypedTransaction {
        request_id: 79,
        base_document_revision: empty_noop.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };
    let before = atomic_audit(&empty_noop);
    let commit = empty_noop.apply_typed_transaction(transaction).unwrap();
    assert!(!commit.changed);
    assert_eq!(atomic_audit(&empty_noop), before);
}

#[test]
fn local_empty_initialization_enforces_the_exact_canonical_output_ceiling() {
    let schema = tiptap_schema();
    let document = schema.default_document().unwrap();
    let value = crate::serialize::to_prosemirror_json(&document, &schema);
    let exact = serde_json::to_vec(&value).unwrap().len();
    let config = |limit| YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        },
        max_length: None,
        scope: None,
    };

    assert_eq!(
        YrsDocumentEngine::new(config(exact))
            .unwrap()
            .document_json(),
        Some(value)
    );
    let error = YrsDocumentEngine::new(config(exact - 1)).err().unwrap();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
}

#[test]
fn json_and_html_import_enforce_output_before_any_live_state_change() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "x"}]
        }]
    });
    let exact = serde_json::to_vec(&expected).unwrap().len();
    for (is_html, input) in [
        (false, serde_json::to_string(&expected).unwrap()),
        (true, "<p>x</p>".to_string()),
    ] {
        let mut accepted = transaction_engine();
        accepted.editing_limits.max_derived_output_bytes = exact;
        reset_canonical_artifact_counts_for_test();
        let commit = if is_html {
            accepted.import_html(
                &input,
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
        } else {
            accepted.import_json(&input, TransactionOrigin::DocumentImport)
        }
        .unwrap();
        assert!(commit.changed);
        assert_eq!(accepted.document_json(), Some(expected.clone()));
        assert_eq!(
            take_canonical_artifact_counts_for_test(),
            (1, usize::from(is_html))
        );

        let mut rejected = transaction_engine();
        rejected.editing_limits.max_derived_output_bytes = exact - 1;
        rejected.revision = u64::MAX;
        rejected.state_revision = u64::MAX;
        rejected.yrs_state_epoch = u64::MAX;
        rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
        rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
        let before = atomic_audit(&rejected);
        let artifact_before = rejected
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        reset_canonical_artifact_counts_for_test();
        let error = if is_html {
            rejected.import_html(
                &input,
                &FromHtmlOptions::default(),
                TransactionOrigin::DocumentImport,
            )
        } else {
            rejected.import_json(&input, TransactionOrigin::DocumentImport)
        }
        .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED", "is_html={is_html}");
        assert_eq!(error.limit, Some(exact - 1));
        assert_eq!(error.actual, Some(exact));
        assert_eq!(
            error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" }))
        );
        assert_eq!(atomic_audit(&rejected), before);
        assert!(
            artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact)
        );
        assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
    }
}

#[test]
fn changed_snapshot_restore_enforces_output_before_revisions_history_or_swap() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let exact = serde_json::to_vec(&source.document_json().unwrap())
        .unwrap()
        .len();

    let mut accepted = transaction_engine();
    accepted.editing_limits.max_derived_output_bytes = exact;
    reset_canonical_artifact_counts_for_test();
    assert!(accepted.restore_snapshot(&snapshot).unwrap().changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
    accepted.editing_limits.max_derived_output_bytes = 1;
    reset_canonical_artifact_counts_for_test();
    assert!(!accepted.restore_snapshot(&snapshot).unwrap().changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));

    let mut rejected = transaction_engine();
    rejected.editing_limits.max_derived_output_bytes = exact - 1;
    rejected.revision = u64::MAX;
    rejected.state_revision = u64::MAX;
    rejected.yrs_state_epoch = u64::MAX;
    rejected.derived_state.as_mut().unwrap().document_revision = u64::MAX;
    rejected.derived_state.as_mut().unwrap().state_revision = u64::MAX;
    let before = atomic_audit(&rejected);
    let artifact_before = rejected
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    reset_canonical_artifact_counts_for_test();
    let error = rejected.restore_snapshot(&snapshot).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!(atomic_audit(&rejected), before);
    assert!(artifact_before.ptr_eq(&rejected.derived_state.as_ref().unwrap().canonical_artifact));
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 1));
}

#[test]
fn typed_commit_installs_local_client_origin_and_candidate_revisions() {
    let mut source = transaction_engine();
    let imported = source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(imported.changed);
    assert_eq!(
        (
            source.revision,
            source.state_revision,
            source.yrs_state_epoch
        ),
        (1, 1, 1)
    );
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    assert!(!target.durable_client_ids.contains(&local_client));
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (1, 1, 1)
    );

    let transaction = insert_transaction(&target, 75);
    let commit = target.apply_typed_transaction(transaction).unwrap();

    assert!(commit.changed);
    assert!(target.durable_client_ids.contains(&local_client));
    assert_eq!(
        target.last_committed_origin,
        Some(TransactionOrigin::LocalApi)
    );
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (2, 2, 2)
    );

    let unchanged = target.document_json().unwrap();
    let commit = target
        .import_json(
            &serde_json::to_string(&unchanged).unwrap(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(
        (
            target.revision,
            target.state_revision,
            target.yrs_state_epoch
        ),
        (2, 2, 2)
    );
}

#[test]
fn restored_deletion_only_commit_does_not_claim_an_unauthored_local_client() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    assert!(!target.durable_client_ids.contains(&local_client));
    let from = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 1, ..from };
    let transaction = TypedTransaction {
        request_id: 80,
        base_document_revision: target.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::DeleteRange {
            range: RevisionedRange { from, to },
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    let compiled = target
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    assert_eq!(compiled.authored_clock_units, 0);
    target.apply_typed_transaction(transaction).unwrap();

    assert_prepared_candidate_state_vector_exact(&target);
    assert!(!target.durable_client_ids.contains(&local_client));
    let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert!(durable_clients.get(&ClientID::new(local_client)) == 0);
}

#[test]
fn restored_format_only_commit_records_its_authored_local_clock() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"seed"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    let mut target = transaction_engine();
    target.restore_snapshot(&snapshot).unwrap();
    let local_client = target.client_id();
    let from = RevisionedPosition {
        offset: 0,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 1, ..from };
    let transaction = TypedTransaction {
        request_id: 81,
        base_document_revision: target.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![TypedOperation::AddMark {
            range: RevisionedRange { from, to },
            mark: Mark::new("bold".into(), HashMap::new()),
        }],
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    };

    let compiled = target
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    assert!(compiled.authored_clock_units > 0);
    target.apply_typed_transaction(transaction).unwrap();

    assert_prepared_candidate_state_vector_exact(&target);
    assert!(target.durable_client_ids.contains(&local_client));
    let durable_clients = Update::decode_v1(&target.encoded_state().unwrap())
        .unwrap()
        .state_vector();
    assert!(durable_clients.get(&ClientID::new(local_client)) > 0);
}
