fn arb_trace() -> impl Strategy<Value = Vec<ActionSpec>> {
    prop::collection::vec(
        (0_u8..14, any::<u64>()).prop_map(|(kind, salt)| ActionSpec { kind, salt }),
        1..=100,
    )
}

fn operation_error_class(error: &OperationError) -> ErrorClass {
    match error.code {
        "POSITION_INVALID" => ErrorClass::Position,
        "OPERATION_INVALID"
            if error
                .details
                .as_ref()
                .and_then(|value| value["field"].as_str())
                == Some("range") =>
        {
            ErrorClass::InvalidRange
        }
        "OPERATION_INVALID" | "DOCUMENT_INVALID" => ErrorClass::InvalidContent,
        other => panic!("unclassified operation error {other}: {error:?}"),
    }
}

fn transform_error_class(error: &TransformError) -> ErrorClass {
    match error {
        TransformError::OutOfBounds(_) | TransformError::InvalidTarget(_) => ErrorClass::Position,
        TransformError::InvalidRange(_) => ErrorClass::InvalidRange,
        TransformError::ContentViolation(_) | TransformError::NotImplemented(_) => {
            ErrorClass::InvalidContent
        }
    }
}

#[test]
fn deterministic_transaction_traces_match_the_legacy_oracle_after_every_operation() {
    let coverage = RefCell::new(Coverage::default());

    for kind in 0..14 {
        run_scenario(
            &ActionSpec {
                kind,
                salt: u64::from(kind),
            },
            &coverage,
        );
    }
    run_scenario(&ActionSpec { kind: 12, salt: 0 }, &coverage);
    run_scenario(&ActionSpec { kind: 12, salt: 4 }, &coverage);
    run_custom_root_case(&coverage);
    let fixed_hundred = (0..100)
        .map(|index| ActionSpec {
            kind: u8::try_from(index % 14).unwrap(),
            salt: index as u64,
        })
        .collect::<Vec<_>>();
    run_stateful_trace(&fixed_hundred, &coverage);
    coverage.borrow_mut().longest_trace = fixed_hundred.len();

    let config = Config {
        cases: 256,
        failure_persistence: None,
        max_shrink_iters: 4_096,
        ..Config::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[0x8a; 32]);
    let mut runner = TestRunner::new_with_rng(config, rng);
    runner
        .run(&arb_trace(), |trace| {
            {
                let mut coverage = coverage.borrow_mut();
                coverage.longest_trace = coverage.longest_trace.max(trace.len());
                let randomized = &trace[0];
                coverage.randomized_operations[usize::from(randomized.kind)] = true;
                if randomized.kind == 12 {
                    if randomized.salt & 4 == 0 {
                        coverage.randomized_void_node = true;
                    } else {
                        coverage.randomized_opaque_node = true;
                    }
                }
            }
            run_stateful_trace(&trace, &coverage);
            run_scenario(&trace[0], &coverage);
            run_evolving_list_chain(trace[0].salt, &coverage);
            Ok(())
        })
        .unwrap();

    let coverage = coverage.into_inner();
    assert!(coverage.operations.into_iter().all(|seen| seen));
    assert!(coverage.randomized_operations.into_iter().all(|seen| seen));
    assert!(coverage.scalar && coverage.utf16);
    assert!(coverage.before && coverage.after);
    assert!(coverage.custom_root && coverage.void_node && coverage.opaque_node);
    assert!(coverage.randomized_void_node && coverage.randomized_opaque_node);
    assert_eq!(coverage.longest_trace, 100);
}

#[test]
fn one_transaction_maps_same_base_utf16_position_by_before_and_after_affinity() {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }]
    });
    let base = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let base_pos = legacy_position(&base, &schema, 2);

    for (request_id, second_affinity, second_pos, expected_text) in [
        (20_001, Affinity::Before, base_pos, "A😀yxB"),
        (20_002, Affinity::After, base_pos + 1, "A😀xyB"),
    ] {
        let mut legacy_transaction = Transaction::new(Source::Api);
        legacy_transaction.add_step(Step::InsertText {
            pos: base_pos,
            text: "x".into(),
            marks: vec![],
        });
        legacy_transaction.add_step(Step::InsertText {
            pos: second_pos,
            text: "y".into(),
            marks: vec![],
        });
        let expected = legacy_transaction.apply(&base, &schema).unwrap().0;
        assert_eq!(expected.root().text_content(), expected_text);

        let point = |affinity| RevisionedPosition {
            offset: 3,
            kind: EditorOffsetKind::Utf16,
            affinity,
        };
        let mut yrs = engine(schema.clone());
        yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let commit = yrs
            .apply_typed_transaction(transaction_with_operations(
                &yrs,
                request_id,
                vec![
                    TypedOperation::InsertText {
                        at: point(Affinity::After),
                        text: "x".into(),
                        marks: vec![],
                    },
                    TypedOperation::InsertText {
                        at: point(second_affinity),
                        text: "y".into(),
                        marks: vec![],
                    },
                ],
            ))
            .unwrap();
        assert!(commit.changed);
        assert_eq!(yrs.document(), Some(&expected));
        assert_encoded_state_matches(&yrs, &expected, &schema);
    }
}

#[test]
fn no_op_and_rejection_classes_match_the_legacy_oracle_without_state_changes() {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a😀bcdef" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = rendered_text(&document, &schema);
    let coverage = RefCell::new(Coverage::default());

    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let before = (
        yrs.encoded_state().unwrap(),
        yrs.document_json(),
        yrs.document_html(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let no_op = yrs
        .apply_typed_transaction(transaction(
            &yrs,
            9_100,
            TypedOperation::DeleteRange {
                range: range(&rendered, 2, 2, 0, &coverage),
            },
        ))
        .unwrap();
    assert!(!no_op.changed);
    let mut legacy_no_op = Transaction::new(Source::Api);
    legacy_no_op.add_step(Step::DeleteRange {
        from: legacy_position(&document, &schema, 2),
        to: legacy_position(&document, &schema, 2),
    });
    assert_eq!(legacy_no_op.apply(&document, &schema).unwrap().0, document);
    assert_eq!(
        (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision()
        ),
        before
    );

    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::DeleteRange {
        from: legacy_position(&document, &schema, 4),
        to: legacy_position(&document, &schema, 1),
    });
    let transform_error = legacy.apply(&document, &schema).unwrap_err();
    for (index, (kind, affinity)) in [
        (EditorOffsetKind::Scalar, Affinity::Before),
        (EditorOffsetKind::Scalar, Affinity::After),
        (EditorOffsetKind::Utf16, Affinity::Before),
        (EditorOffsetKind::Utf16, Affinity::After),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = |scalar| match kind {
            EditorOffsetKind::Scalar => scalar,
            EditorOffsetKind::Utf16 => scalar_offset_to_utf16(&rendered, scalar).unwrap(),
        };
        let reverse = TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: RevisionedPosition {
                    offset: offset(4),
                    kind,
                    affinity,
                },
                to: RevisionedPosition {
                    offset: offset(1),
                    kind,
                    affinity,
                },
            },
        };
        let before = (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision(),
        );
        let operation_error = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                9_101 + u64::try_from(index).unwrap(),
                reverse,
            ))
            .unwrap_err();
        assert_eq!(
            operation_error_class(&operation_error),
            transform_error_class(&transform_error)
        );
        assert_eq!(
            (
                yrs.encoded_state().unwrap(),
                yrs.document_json(),
                yrs.document_html(),
                yrs.revision(),
                yrs.state_revision(),
            ),
            before
        );
    }

    let before = (
        yrs.encoded_state().unwrap(),
        yrs.document_json(),
        yrs.document_html(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let invalid_position = TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 99,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "x".into(),
        marks: vec![],
    };
    let operation_error = yrs
        .apply_typed_transaction(transaction(&yrs, 9_102, invalid_position))
        .unwrap_err();
    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::InsertText {
        pos: document.root().node_size() + 1,
        text: "x".into(),
        marks: vec![],
    });
    let transform_error = legacy.apply(&document, &schema).unwrap_err();
    assert_eq!(
        operation_error_class(&operation_error),
        transform_error_class(&transform_error)
    );
    assert_eq!(
        (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision()
        ),
        before
    );

    let before = (
        yrs.encoded_state().unwrap(),
        yrs.document_json(),
        yrs.document_html(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let invalid_mark = Mark::new("notDeclared".into(), HashMap::new());
    let invalid_content = TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "x".into(),
        marks: vec![invalid_mark.clone()],
    };
    let operation_error = yrs
        .apply_typed_transaction(transaction(&yrs, 9_200, invalid_content))
        .unwrap_err();
    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::InsertText {
        pos: legacy_position(&document, &schema, 2),
        text: "x".into(),
        marks: vec![invalid_mark],
    });
    let transform_error = legacy.apply(&document, &schema).unwrap_err();
    assert_eq!(
        operation_error_class(&operation_error),
        transform_error_class(&transform_error)
    );
    assert_eq!(
        operation_error_class(&operation_error),
        ErrorClass::InvalidContent
    );
    assert_eq!(
        (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision()
        ),
        before
    );
}

mod whole_root_replacement {
    use std::collections::HashMap;

    use crate::boundary::ResourceLimits;
    use crate::tiptap_schema;
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, ReplacementHistory, TransactionOrigin,
        YrsDocumentEngine, YrsEngineConfig,
    };
    use proptest::prelude::*;
    use yrs::updates::decoder::Decode;
    use yrs::{Doc, GetString, ReadTxn, Transact, Update};

    fn engine_with_document(json: &serde_json::Value) -> YrsDocumentEngine {
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
        engine
            .import_json(&json.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        engine
    }

    fn paragraphs(texts: &[String]) -> serde_json::Value {
        serde_json::json!({
            "type": "doc",
            "content": texts
                .iter()
                .map(|text| {
                    if text.is_empty() {
                        serde_json::json!({ "type": "paragraph" })
                    } else {
                        serde_json::json!({
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": text }],
                        })
                    }
                })
                .collect::<Vec<_>>(),
        })
    }

    fn state_vector_entries(update: &[u8]) -> HashMap<u64, u32> {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(update).unwrap())
                .unwrap();
        }
        let txn = doc.transact();
        let vector = txn.state_vector();
        vector
            .iter()
            .map(|(client, clock)| (client.get(), *clock))
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn root_replacement_is_same_store_import_equivalent_and_convergent(
            before in proptest::collection::vec("[a-z]{0,6}", 1..4usize),
            after in proptest::collection::vec("[a-z]{0,6}", 1..4usize),
            reset in proptest::bool::ANY,
        ) {
            let before_doc = paragraphs(&before);
            let after_doc = paragraphs(&after);
            let mut engine = engine_with_document(&before_doc);
            let reference = engine_with_document(&after_doc);
            let expected_changed = engine.document_json() != reference.document_json();

            let base_state = engine.encoded_state().unwrap();
            let writer = engine.client_id();
            let history = if reset {
                ReplacementHistory::ResetAndClear
            } else {
                ReplacementHistory::UndoableBoundary
            };

            let commit = engine
                .prepare_root_replacement_json(7, &after_doc.to_string(), history)
                .unwrap();

            // Replacement is canonically equivalent to a fresh import of the
            // same target document.
            prop_assert_eq!(commit.changed, expected_changed);
            prop_assert_eq!(engine.document_json(), reference.document_json());
            prop_assert_eq!(engine.client_id(), writer);

            // Same-store: the writer's clock strictly advances on change and
            // no other state-vector entry moves.
            let after_state = engine.encoded_state().unwrap();
            let before_entries = state_vector_entries(&base_state);
            let after_entries = state_vector_entries(&after_state);
            if expected_changed {
                prop_assert!(
                    after_entries.get(&writer).copied().unwrap_or(0)
                        > before_entries.get(&writer).copied().unwrap_or(0)
                );
            } else {
                prop_assert_eq!(&after_entries, &before_entries);
            }
            for (client, clock) in &before_entries {
                if *client != writer {
                    prop_assert_eq!(after_entries.get(client), Some(clock));
                }
            }

            // Standard incremental convergence: a peer holding the prior
            // state converges through the base->after Update-v1 alone.
            let peer = Doc::new();
            let peer_fragment = peer.get_or_insert_xml_fragment("prosemirror");
            {
                let mut txn = peer.transact_mut();
                txn.apply_update(Update::decode_v1(&base_state).unwrap()).unwrap();
            }
            let base_vector = peer.transact().state_vector();
            let replica = Doc::new();
            let replica_fragment = replica.get_or_insert_xml_fragment("prosemirror");
            {
                let mut txn = replica.transact_mut();
                txn.apply_update(Update::decode_v1(&after_state).unwrap()).unwrap();
            }
            let incremental = replica.transact().encode_state_as_update_v1(&base_vector);
            {
                let mut txn = peer.transact_mut();
                txn.apply_update(Update::decode_v1(&incremental).unwrap()).unwrap();
            }
            {
                let peer_txn = peer.transact();
                let replica_txn = replica.transact();
                prop_assert_eq!(peer_txn.state_vector(), replica_txn.state_vector());
                prop_assert_eq!(
                    peer_fragment.get_string(&peer_txn),
                    replica_fragment.get_string(&replica_txn)
                );
            }

            // Exact history policy per mode.
            if reset {
                prop_assert!(!engine.can_undo());
                prop_assert!(!engine.can_redo());
            } else {
                prop_assert_eq!(engine.can_undo(), expected_changed);
                if expected_changed {
                    prop_assert!(engine.undo(8).unwrap().is_some());
                    let restored = engine_with_document(&before_doc);
                    prop_assert_eq!(engine.document_json(), restored.document_json());
                }
            }
        }
    }
}

/// Property extension: every durable local path — typed input transaction,
/// command, undo, redo, replace (`UndoableBoundary`), and reset
/// (`ResetAndClear`) — reserves a conservative outbound bound before the
/// irreversible Yrs write and captures an incremental Update-v1 whose length
/// never exceeds that admitted bound, while a twin replica fed only by the
/// captured outbox entries converges exactly. Selection requests reserve and
/// enqueue nothing.
mod outbound_update_bounds {
    use crate::boundary::ResourceLimits;
    use crate::native_bridge_test_support::{self as bridge, BridgeTestOutcome, SessionOptions};
    use crate::tiptap_schema;
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
    };
    use proptest::prelude::*;

    fn paragraphs_json(texts: &[String]) -> String {
        serde_json::json!({
            "type": "doc",
            "content": texts
                .iter()
                .map(|text| {
                    if text.is_empty() {
                        serde_json::json!({ "type": "paragraph" })
                    } else {
                        serde_json::json!({
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": text }],
                        })
                    }
                })
                .collect::<Vec<_>>(),
        })
        .to_string()
    }

    fn twin_replica() -> YrsDocumentEngine {
        // Content-free AwaitRemote replica: converges exclusively from the
        // captured outbox entries.
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "bounds-twin".into(),
                lineage_id: "bounds-twin-lineage".into(),
            }),
        })
        .unwrap()
    }

    fn revision(id: u64) -> u64 {
        bridge::session_audit(id).unwrap().document_revision
    }

    fn text_fragment() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z]{1,6}",
            "[\u{1F600}-\u{1F604}]{1,3}",
            "[\u{e9}-\u{ef}]{1,4}",
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        #[test]
        fn captured_updates_stay_within_admitted_bounds_and_converge(
            seed_texts in proptest::collection::vec(text_fragment(), 1..3usize),
            typed in text_fragment(),
            typed_second in text_fragment(),
            replace_texts in proptest::collection::vec(text_fragment(), 1..3usize),
            reset_texts in proptest::collection::vec(text_fragment(), 1..3usize),
        ) {
            let id = bridge::create_session(SessionOptions {
                initial_json: Some(paragraphs_json(&seed_texts)),
                attach_runtime: true,
                ..SessionOptions::default()
            })
            .unwrap();

            let mut twin = twin_replica();
            let mut replay = 50_000u64;
            let initial = bridge::session_audit(id).unwrap().encoded_state.unwrap();
            twin.apply_remote_update_v1(replay, &initial).unwrap();

            let mut replay_one = |twin: &mut YrsDocumentEngine,
                                  label: &str|
             -> Result<(), TestCaseError> {
                let bound = bridge::last_reserved_upper_bound(id).unwrap();
                let bound = bound.unwrap_or_else(|| panic!("{label}: missing reservation"));
                let lease = bridge::lease_next_update(id)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{label}: missing outbox lease"));
                let lease_id = lease.lease_id;
                let update = lease.update_v1;
                prop_assert!(
                    update.len() <= bound,
                    "{} captured {} bytes above admitted bound {}",
                    label,
                    update.len(),
                    bound,
                );
                replay += 1;
                twin.apply_remote_update_v1(replay, &update).unwrap();
                prop_assert_eq!(
                    twin.document_json(),
                    bridge::session_audit(id).unwrap().document_json,
                    "{} twin replica must converge from the captured update",
                    label,
                );
                bridge::ack_leased_update(id, lease_id).unwrap();
                prop_assert!(
                    bridge::lease_next_update(id).unwrap().is_none(),
                    "{} must enqueue exactly one entry",
                    label,
                );
                Ok(())
            };

            // Typed local-input transaction.
            let outcome = bridge::submit_input(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "1",
                    "baseDocumentRevision": revision(id).to_string(),
                    "text": typed,
                })
                .to_string(),
            )
            .unwrap();
            let changed_transaction = matches!(
                outcome,
                BridgeTestOutcome::Transaction { changed: true, .. }
            );
            prop_assert!(changed_transaction);
            replay_one(&mut twin, "input")?;

            // Selection/state-only request: reserves nothing, enqueues nothing.
            bridge::submit_selection(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "2",
                    "baseDocumentRevision": revision(id).to_string(),
                    "selection": {
                        "type": "text",
                        "anchor": { "offset": 0, "kind": "scalar" },
                        "head": { "offset": 0, "kind": "scalar" },
                    },
                })
                .to_string(),
            )
            .unwrap();
            prop_assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));

            // Command.
            let outcome = bridge::submit_command(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "3",
                    "baseDocumentRevision": revision(id).to_string(),
                    "command": { "type": "toggleBlockquote" },
                })
                .to_string(),
            )
            .unwrap();
            let changed_transaction = matches!(
                outcome,
                BridgeTestOutcome::Transaction { changed: true, .. }
            );
            prop_assert!(changed_transaction);
            replay_one(&mut twin, "command")?;

            // Second input so undo has a mixed-history group to pop.
            bridge::submit_input(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "4",
                    "baseDocumentRevision": revision(id).to_string(),
                    "text": typed_second,
                })
                .to_string(),
            )
            .unwrap();
            replay_one(&mut twin, "input-second")?;

            // Undo and redo.
            prop_assert!(bridge::undo(id, 5).unwrap());
            replay_one(&mut twin, "undo")?;
            prop_assert!(bridge::redo(id, 6).unwrap());
            replay_one(&mut twin, "redo")?;

            // Whole-document replace: one undoable local-API boundary.
            bridge::submit_local_api(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "7",
                    "baseDocumentRevision": revision(id).to_string(),
                    "setJson": serde_json::from_str::<serde_json::Value>(
                        &paragraphs_json(&replace_texts),
                    )
                    .unwrap(),
                    "history": "undoableBoundary",
                })
                .to_string(),
            )
            .unwrap();
            if bridge::outbox_pending(id).unwrap() == Some((0, 0)) {
                // Identical replacement content is an unchanged commit and
                // must not enqueue an update.
                prop_assert_eq!(
                    twin.document_json(),
                    bridge::session_audit(id).unwrap().document_json,
                );
            } else {
                replay_one(&mut twin, "replace")?;
            }

            // Reset: non-undoable, clears history, still one bounded entry.
            bridge::submit_local_api(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "8",
                    "baseDocumentRevision": revision(id).to_string(),
                    "setJson": serde_json::from_str::<serde_json::Value>(
                        &paragraphs_json(&reset_texts),
                    )
                    .unwrap(),
                    "history": "resetAndClear",
                })
                .to_string(),
            )
            .unwrap();
            let audit = bridge::session_audit(id).unwrap();
            prop_assert!(!audit.can_undo);
            if bridge::outbox_pending(id).unwrap() == Some((0, 0)) {
                prop_assert_eq!(twin.document_json(), audit.document_json);
            } else {
                replay_one(&mut twin, "reset")?;
            }

            bridge::destroy_session(id);
        }
    }
}
