#[test]
fn prepared_active_state_cache_rejection_and_noop_preserve_arc_identity() {
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
            request_id: 714_000,
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
        .apply_command(714_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    let cache = engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .unwrap();
    let before = atomic_audit(&engine);

    let rejected = engine.apply_typed_transaction(TypedTransaction {
        request_id: 714_002,
        base_document_revision: engine.revision().saturating_add(1),
        origin: TransactionOrigin::LocalApi,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Auto,
    });
    assert!(rejected.is_err());
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &cache,
        &engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap()
    ));

    let no_op = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 714_003,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!no_op.changed);
    assert!(Arc::ptr_eq(
        &cache,
        &engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap()
    ));

    let boundary = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 714_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    assert!(!boundary.changed);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .active_state_cache_for_test()
        .is_none());
}

#[test]
fn prepared_active_state_warm_hit_matches_forced_generic_at_output_boundaries() {
    use crate::yrs_engine::derived_state::{
        force_active_state_cache_hit_fallback_for_test, reset_active_state_cache_counts_for_test,
        take_active_state_cache_counts_for_test,
    };

    fn fixture(
        json: &str,
        caret: u32,
        first: &str,
        max_derived_output_bytes: usize,
    ) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine.editing_limits.max_derived_output_bytes = max_derived_output_bytes;
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        let point = RevisionedPosition {
            offset: caret,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 715_000,
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
            .apply_command(715_001, TypedCommand::InsertText { text: first.into() })
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

    fn assert_internal_parity(left: &YrsDocumentEngine, right: &YrsDocumentEngine) {
        assert_eq!(left.document_json(), right.document_json());
        assert_eq!(left.can_undo(), right.can_undo());
        assert_eq!(left.can_redo(), right.can_redo());
        let left_state = left.derived_state.as_ref().unwrap();
        let right_state = right.derived_state.as_ref().unwrap();
        assert_eq!(
            left_state.validation_certificate,
            right_state.validation_certificate
        );
        assert_eq!(
            left_state.localized_text_index,
            right_state.localized_text_index
        );
        assert_eq!(
            left_state.render_blocks.materialize(),
            right_state.render_blocks.materialize()
        );
        assert_eq!(
            left_state.active_state_cache_for_test().unwrap().value(),
            right_state.active_state_cache_for_test().unwrap().value()
        );
        for engine in [left, right] {
            let txn = engine.doc.transact();
            let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
            let state = engine.derived_state.as_ref().unwrap();
            assert!(state.mutation_lookup_seed.matches(
                &txn,
                &fragment,
                &state.document,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                engine.yrs_state_epoch,
                engine.revision,
            ));
        }
    }

    for (shape, json, caret, first) in [
        (
            "plain",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            1,
            "x",
        ),
        (
            "marked",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]}"#,
            1,
            "x",
        ),
        (
            "nonBmp",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
            1,
            "🦀",
        ),
    ] {
        // Keep the result-output boundary above the independently enforced
        // deep retained-state budget so the warm certificate exists at
        // both the exact and one-under output limits.
        let second = if shape == "nonBmp" {
            "界".repeat(2_048)
        } else {
            "y".repeat(4_096)
        };
        let mut probe = fixture(json, caret, first, usize::MAX / 2);
        let exact = probe
            .apply_command(
                715_002,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            )
            .unwrap()
            .unwrap()
            .derived_output_bytes();

        let mut hit = fixture(json, caret, first, exact);
        let mut generic = fixture(json, caret, first, exact);
        reset_active_state_cache_counts_for_test();
        let hit_result = hit
            .apply_command(
                715_003,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 1, 0, 0, 1, 1, 0, 1, 0),
            "{shape} hit"
        );
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_hit_fallback_for_test(true);
        let generic_result = generic.apply_command(
            715_003,
            TypedCommand::InsertText {
                text: second.clone(),
            },
        );
        force_active_state_cache_hit_fallback_for_test(false);
        let generic_result = generic_result.unwrap().unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 1, 0, 1, 1),
            "{shape} generic"
        );
        assert_eq!(hit_result.derived_output_bytes(), exact, "{shape}");
        assert_eq!(hit_result, generic_result, "{shape}");
        assert_internal_parity(&hit, &generic);

        let mut rejected_hit = fixture(json, caret, first, exact - 1);
        let mut rejected_generic = fixture(json, caret, first, exact - 1);
        let hit_cache = rejected_hit
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();
        let generic_cache = rejected_generic
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .unwrap();
        let hit_before = atomic_audit(&rejected_hit);
        let generic_before = atomic_audit(&rejected_generic);
        reset_active_state_cache_counts_for_test();
        let hit_error = rejected_hit
            .apply_command(
                715_004,
                TypedCommand::InsertText {
                    text: second.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 1, 0, 0, 1, 0, 0, 1, 0),
            "{shape} rejected hit"
        );
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_hit_fallback_for_test(true);
        let generic_error = rejected_generic.apply_command(
            715_004,
            TypedCommand::InsertText {
                text: second.clone(),
            },
        );
        force_active_state_cache_hit_fallback_for_test(false);
        let generic_error = generic_error.unwrap_err();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 0, 0, 1, 1),
            "{shape} rejected generic"
        );
        assert_eq!(hit_error, generic_error, "{shape}");
        assert_eq!(
            hit_error.details,
            Some(json!({ "field": "maxDerivedOutputBytes" })),
            "{shape}"
        );
        assert_eq!(atomic_audit(&rejected_hit), hit_before, "{shape}");
        assert_eq!(atomic_audit(&rejected_generic), generic_before, "{shape}");
        assert!(Arc::ptr_eq(
            &hit_cache,
            &rejected_hit
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap()
        ));
        assert!(Arc::ptr_eq(
            &generic_cache,
            &rejected_generic
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap()
        ));
    }
}

#[test]
fn prepared_active_state_context_matrix_matches_forced_generic() {
    use crate::yrs_engine::derived_state::{
        force_active_state_cache_hit_fallback_for_test, reset_active_state_cache_counts_for_test,
        take_active_state_cache_counts_for_test,
    };

    fn fixture(
        shape: &str,
        json: &str,
        target_text: &str,
        intra_leaf_scalar: u32,
        explicit_stored_bold: bool,
    ) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        let byte_start = state.rendered_text.find(target_text).unwrap();
        let scalar_start =
            u32::try_from(state.rendered_text[..byte_start].chars().count()).unwrap();
        let rendered_position = scalar_start + intra_leaf_scalar;
        let selection_at = |engine: &YrsDocumentEngine, affinity| {
            let point = RevisionedPosition {
                offset: rendered_position,
                kind: EditorOffsetKind::Scalar,
                affinity,
            };
            TypedTransaction {
                request_id: 716_000,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: point,
                    head: point,
                }),
                history_policy: HistoryPolicy::Skip,
            }
        };
        if engine
            .apply_typed_transaction(selection_at(&engine, Affinity::After))
            .is_err()
        {
            engine
                .apply_typed_transaction(selection_at(&engine, Affinity::Before))
                .unwrap();
        }
        if explicit_stored_bold {
            for request_id in [716_001, 716_002] {
                engine
                    .apply_command(
                        request_id,
                        TypedCommand::ToggleMark {
                            mark_type: "bold".into(),
                        },
                    )
                    .unwrap()
                    .unwrap();
            }
            assert!(engine
                .stored_marks()
                .is_some_and(|marks| { marks.iter().any(|mark| mark.mark_type() == "bold") }));
        }
        engine
            .apply_command(716_003, TypedCommand::InsertText { text: "x".into() })
            .unwrap()
            .unwrap();
        assert!(
            engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .is_some(),
            "{shape}"
        );
        engine
    }

    let wide = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"middle"}]},{"type":"paragraph","content":[{"type":"text","text":"last"}]}]}"#;
    for (shape, json, target, explicit_stored_bold) in [
        (
            "nested-list-item",
            r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
            "abc",
            false,
        ),
        (
            "blockquote",
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}"#,
            "abc",
            false,
        ),
        ("first-top-level", wide, "first", false),
        ("middle-top-level", wide, "middle", false),
        ("last-top-level", wide, "last", false),
        (
            "explicit-stored-marks",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc","marks":[{"type":"bold"}]}]}]}"#,
            "abc",
            true,
        ),
    ] {
        let mut hit = fixture(shape, json, target, 1, explicit_stored_bold);
        let mut generic = fixture(shape, json, target, 1, explicit_stored_bold);
        reset_active_state_cache_counts_for_test();
        let hit_result = hit
            .apply_command(716_004, TypedCommand::InsertText { text: "y".into() })
            .unwrap()
            .unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 1, 0, 0, 1, 1, 0, 1, 0),
            "{shape} hit"
        );
        reset_active_state_cache_counts_for_test();
        force_active_state_cache_hit_fallback_for_test(true);
        let generic_result =
            generic.apply_command(716_004, TypedCommand::InsertText { text: "y".into() });
        force_active_state_cache_hit_fallback_for_test(false);
        let generic_result = generic_result.unwrap().unwrap();
        assert_eq!(
            take_active_state_cache_counts_for_test(),
            (1, 0, 1, 1, 1, 1, 0, 1, 1),
            "{shape} generic"
        );
        assert_eq!(hit_result, generic_result, "{shape}");
        assert_eq!(hit.document_json(), generic.document_json(), "{shape}");
        assert_eq!(hit.can_undo(), generic.can_undo(), "{shape}");
        assert_eq!(hit.can_redo(), generic.can_redo(), "{shape}");
        let hit_state = hit.derived_state.as_ref().unwrap();
        let generic_state = generic.derived_state.as_ref().unwrap();
        assert_eq!(
            hit_state.validation_certificate, generic_state.validation_certificate,
            "{shape}"
        );
        assert_eq!(
            hit_state.localized_text_index, generic_state.localized_text_index,
            "{shape}"
        );
        assert_eq!(
            hit_state.render_blocks.materialize(),
            generic_state.render_blocks.materialize(),
            "{shape}"
        );
        assert_eq!(
            hit_state.active_state_cache_for_test().unwrap().value(),
            generic_state.active_state_cache_for_test().unwrap().value(),
            "{shape}"
        );
    }
}
