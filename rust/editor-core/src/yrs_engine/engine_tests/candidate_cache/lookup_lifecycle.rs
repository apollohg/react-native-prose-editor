#[test]
fn imported_candidate_cache_supplies_first_staged_lookup_without_live_rebuild() {
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
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    reset_localized_lookup_counts_for_test();

    engine
        .apply_command(70_107, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
}

#[test]
fn validated_import_materializes_ready_lookup_without_a_second_tree_scan() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_localized_lookup_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(
        take_localized_lookup_counts_for_test(),
        (0, 0, 0),
        "validated codec traversal must carry the exact ready lookup payload"
    );
    assert!(engine
        .prepared_candidate_cache
        .as_ref()
        .and_then(|cache| cache.staged_lookup_seed.as_ref())
        .is_some());
}

#[test]
fn validated_import_lookup_materialization_matches_the_ordinary_builder() {
    let inputs = [
        r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":2}}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain"},{"type":"text","text":" bold","marks":[{"type":"bold"}]},{"type":"text","text":" 🦀"}]}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"hardBreak"},{"type":"text","text":"middle"},{"type":"hardBreak"},{"type":"hardBreak"}]}]}"#,
        r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"nested"}]}]},{"type":"horizontal_rule"},{"type":"mystery_widget","attrs":{"payload":{"x":[1,true,"v"]}}}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"},{"type":"text","text":"b","marks":[{"type":"italic"}]},{"type":"text","text":"c"}]}]}"#,
    ];

    for input in inputs {
        let mut engine = transaction_engine();
        engine
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        let staged = engine
            .prepared_candidate_cache
            .as_ref()
            .and_then(|cache| cache.staged_lookup_seed.as_ref())
            .unwrap_or_else(|| panic!("validated import carries the fused ready seed: {input}"));
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        assert!(
            crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
                &txn,
                &fragment,
                &engine.schema,
            ),
            "{input}"
        );
        let ordinary = crate::yrs_engine::mutation::MutationLookupSeed::build(
            77_001,
            &txn,
            &fragment,
            &engine.schema,
            &state.document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap();
        assert!(staged.has_same_ready_payload_for_test(&ordinary), "{input}");
    }
}

#[test]
fn lookup_materialization_matches_legacy_for_nested_fragment_and_empty_text_storage() {
    let engine = transaction_engine();
    let doc = utf16_doc();
    let mut txn = doc.transact_mut();
    let fragment = txn.get_or_insert_xml_fragment("content");
    let nested = XmlFragmentPrelim::new::<_, XmlIn>([
        XmlIn::from(XmlTextPrelim::new("")),
        XmlIn::from(XmlTextPrelim::new("x")),
    ]);
    fragment.insert(&mut txn, 0, XmlIn::from(nested));
    drop(txn);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("content").unwrap();

    assert!(
        crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
            &txn,
            &fragment,
            &engine.schema,
        )
    );
}

#[test]
fn import_lookup_materialization_failpoints_are_opportunistic_and_fallback_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, set_lookup_seed_hydration_failpoint_for_test,
        take_localized_lookup_counts_for_test, LookupSeedHydrationFailpoint,
    };

    for failpoint in [
        LookupSeedHydrationFailpoint::InitialReservation,
        LookupSeedHydrationFailpoint::MapGrowth,
        LookupSeedHydrationFailpoint::MapPublication,
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let mut engine = transaction_engine();
        reset_localized_lookup_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(
            take_localized_lookup_counts_for_test().0,
            1,
            "{failpoint:?}"
        );

        reset_localized_lookup_counts_for_test();
        engine
            .apply_typed_transaction(insert_transaction(&engine, 77_100))
            .unwrap();
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc",
            "{failpoint:?}"
        );
        assert_prepared_candidate_state_vector_exact(&engine);
    }
}

#[test]
fn ordinary_lookup_collection_fails_fast_while_codec_projection_finishes() {
    use crate::yrs_engine::mutation::{
        reset_import_lookup_event_count_for_test, set_lookup_seed_hydration_failpoint_for_test,
        take_import_lookup_event_count_for_test, LookupSeedHydrationFailpoint,
    };

    let value = json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "second"}]}
        ]
    });
    let mut engine = transaction_engine();
    engine
        .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    reset_import_lookup_event_count_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(LookupSeedHydrationFailpoint::MapGrowth));
    let error = crate::yrs_engine::mutation::MutationLookupSeed::build(
        77_200,
        &txn,
        &fragment,
        &engine.schema,
        &state.document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(take_import_lookup_event_count_for_test(), 2);
    drop(txn);

    let document =
        from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
    let source = ValidatedImportDocument::new(
        document,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        Some(value.to_string().len()),
    )
    .unwrap();
    reset_import_lookup_event_count_for_test();
    let candidate = engine
        .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert_eq!(take_import_lookup_event_count_for_test(), 2);
    assert!(candidate
        .import_encoded_state_receipt
        .as_ref()
        .is_some_and(|receipt| receipt.lookup_materialization.is_none()));
    let EngineDocumentState::Ready { document, .. } = candidate.state else {
        panic!("validated candidate must be ready")
    };
    assert_eq!(document.root().content().unwrap().child_count(), 2);
}

#[test]
fn missing_text_fallback_rebuilds_once_then_next_insert_localizes() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_111, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("empty paragraph insert must apply");
    engine
        .apply_command(70_112, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .expect("existing text insert must apply");

    assert_eq!(take_localized_lookup_counts_for_test(), (1, 1, 1));
}

#[test]
fn selection_only_change_retains_document_scoped_lookup_seed() {
    let mut engine = transaction_engine();
    let before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .clone();
    let canonical_before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_113,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    let after = &engine.derived_state.as_ref().unwrap().mutation_lookup_seed;
    assert!(Arc::ptr_eq(&before, after));
    assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
}

#[test]
fn localized_root_invalidation_rebuilds_ready_once_then_localizes() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .apply_command(
            70_113_100,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);
    let unavailable = engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .clone();
    assert!(unavailable.is_unavailable_for_test());

    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_113_101,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_113_102, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (2, 0, 0));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_113_103, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn canonical_artifact_derives_once_per_changed_intermediate_and_never_for_cached_noops() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_canonical_artifact_counts_for_test();
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_114))
        .unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));

    let revision = engine.revision();
    reset_canonical_artifact_counts_for_test();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_115,
            base_document_revision: revision,
            origin: TransactionOrigin::LocalApi,
            operations: vec![
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "a".into(),
                    marks: vec![],
                },
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "b".into(),
                    marks: vec![],
                },
            ],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (2, 3));

    reset_canonical_artifact_counts_for_test();
    let commit = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_116,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    to: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));
}

#[test]
fn public_history_pop_installs_candidate_seed_without_next_edit_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_prepared_admission_counts_for_test();
    assert!(engine.undo(70_119).unwrap().is_none());
    assert!(engine.redo(70_120).unwrap().is_none());
    let empty = take_prepared_admission_counts_for_test();
    assert_eq!(empty.staged_seed_preparations, 0);
    assert_eq!(empty.installed_base_seed_publications, 0);
    reset_localized_lookup_counts_for_test();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 0, 0));

    engine
        .apply_command(70_121, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("history insert must apply");
    reset_localized_lookup_counts_for_test();
    reset_prepared_admission_counts_for_test();
    let before_undo = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(engine.undo(70_122).unwrap().is_some());
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    let undo_counts = take_prepared_admission_counts_for_test();
    assert_eq!(undo_counts.staged_seed_preparations, 1);
    assert_eq!(undo_counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &before_undo,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    reset_localized_lookup_counts_for_test();
    reset_prepared_admission_counts_for_test();
    assert!(engine.redo(70_123).unwrap().is_some());
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    let redo_counts = take_prepared_admission_counts_for_test();
    assert_eq!(redo_counts.staged_seed_preparations, 1);
    assert_eq!(redo_counts.installed_base_seed_publications, 0);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_124, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .expect("the first edit after history restoration must apply");
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_125, TypedCommand::InsertText { text: "z".into() })
        .unwrap()
        .expect("the second edit after history restoration must apply");
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    reset_localized_lookup_counts_for_test();
    engine.restore_snapshot(&snapshot).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
}

#[test]
fn accepted_remote_candidate_builds_lookup_seed_in_its_own_store() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let update = source.encoded_state().unwrap();
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    reset_localized_lookup_counts_for_test();

    let commit = target.apply_remote_update_v1(70_131, &update).unwrap();
    assert!(commit.changed);
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    reset_localized_lookup_counts_for_test();
    target
        .apply_command(70_132, TypedCommand::InsertText { text: "!".into() })
        .unwrap()
        .expect("remote existing text must accept a local insert");
    assert_prepared_candidate_state_vector_exact(&target);
    let live_vector = target.doc.transact().state_vector();
    assert!(live_vector.get(&ClientID::new(source.client_id())) > 0);
    assert!(live_vector.get(&ClientID::new(target.client_id())) > 0);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
}

#[test]
fn arbitrary_remote_candidate_rebuilds_revision_bound_render_cache_once() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]},{"type":"paragraph","content":[{"type":"text","text":"tail"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    target
        .apply_remote_update_v1(70_133, &source.encoded_state().unwrap())
        .unwrap();
    source
        .apply_typed_transaction(insert_transaction(&source, 70_134))
        .unwrap();
    let target_vector = target.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);

    crate::render::incremental::reset_cached_render_counts_for_test();
    let commit = target.apply_remote_update_v1(70_135, &delta).unwrap();
    assert!(commit.changed);
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (1, 0, 0, 0, 0)
    );
    let next = target.derived_state.as_ref().unwrap();
    assert_eq!(
        next.render_blocks.materialize(),
        crate::render::incremental::render_blocks(&next.document, &target.schema)
    );
    assert_eq!(next.document_revision, target.revision());
    assert_eq!(next.schema_fingerprint, target.schema_fingerprint);
}

#[test]
fn multi_operation_and_explicit_selection_inserts_use_sealed_eager_fallback() {
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
    let mut transaction = insert_transaction(&engine, 70_141);
    transaction.operations.push(TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "y".into(),
        marks: vec![],
    });
    reset_localized_lookup_counts_for_test();
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));

    let mut transaction = insert_transaction(&engine, 70_142);
    let point = RevisionedPosition {
        offset: 2,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point,
        head: point,
    });
    reset_localized_lookup_counts_for_test();
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
}
