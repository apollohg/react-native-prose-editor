#[test]
fn changed_commit_survives_optional_index_allocation_failure_exactly() {
    use crate::yrs_engine::derived_state::force_localized_index_allocation_failure_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_revision = engine.revision();
    let before_state_revision = engine.state_revision();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    force_localized_index_allocation_failure_for_test(true);
    let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
        request_id: 700_121,
        base_document_revision: before_revision,
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: point,
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    });
    force_localized_index_allocation_failure_for_test(false);
    let result = applied.expect("optional index failure cannot abort commit");
    assert!(result.changed);
    assert_eq!(result.document_revision, before_revision + 1);
    assert_eq!(result.state_revision, before_state_revision + 1);
    assert!(result.changed);
    assert!(matches!(
        result.selection,
        crate::yrs_engine::ResolvedSelection::Text { ref anchor, ref head }
            if anchor.document == 3 && head.document == 3
    ));
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert!(engine.can_undo());
    let state = engine.derived_state.as_ref().unwrap();
    assert_eq!(state.document_revision, result.document_revision);
    assert_eq!(state.state_revision, result.state_revision);
    assert!(state.localized_text_index.is_none());

    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 700_122,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition { offset: 2, ..point },
                text: "y".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_none());
}

#[test]
fn changed_commit_survives_optional_index_budget_failure_exactly() {
    use crate::yrs_engine::derived_state::force_localized_index_budget_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_revision = engine.revision();
    force_localized_index_budget_for_test(Some(1));
    let result = engine.apply_typed_transaction_with_result(TypedTransaction {
        request_id: 700_123,
        base_document_revision: before_revision,
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset: 1,
                kind: EditorOffsetKind::Scalar,
                affinity: Affinity::After,
            },
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    });
    force_localized_index_budget_for_test(None);
    let result = result.expect("optional index budget cannot abort commit");
    assert!(result.changed);
    assert_eq!(result.document_revision, before_revision + 1);
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert!(engine.can_undo());
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .is_none());
}

#[test]
fn changed_commit_survives_each_optional_index_allocation_stage() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_allocation_stage_for_test, LocalizedIndexAllocationStage,
    };

    for (stage_index, stage) in [
        LocalizedIndexAllocationStage::InitialLeafCapacity,
        LocalizedIndexAllocationStage::TraversalPath,
        LocalizedIndexAllocationStage::LeafGrowth,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"},{"type":"hardBreak"},{"type":"text","text":"cd"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let before_document_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        force_localized_index_allocation_stage_for_test(Some(stage));
        let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_130 + u64::try_from(stage_index).unwrap(),
            base_document_revision: before_document_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        });
        force_localized_index_allocation_stage_for_test(None);

        let result = applied.expect("optional index failure cannot abort a live commit");
        assert!(result.changed, "stage {stage:?}");
        assert_eq!(result.document_revision, before_document_revision + 1);
        assert_eq!(result.state_revision, before_state_revision + 1);
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axb"
        );
        assert!(engine.can_undo());
        let state = engine.derived_state.as_ref().unwrap();
        assert_eq!(state.document_revision, result.document_revision);
        assert_eq!(state.state_revision, result.state_revision);
        assert!(state.localized_text_index.is_none(), "stage {stage:?}");

        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_140 + u64::try_from(stage_index).unwrap(),
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition { offset: 2, ..point },
                    text: "y".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert!(compiled.localized_insert_admission.is_none());
    }
}

#[test]
fn selection_only_optional_index_copy_failure_degrades_evidence_to_none() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_allocation_stage_for_test, LocalizedIndexAllocationStage,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_document_revision = engine.revision();
    let before_state_revision = engine.state_revision();
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    force_localized_index_allocation_stage_for_test(Some(
        LocalizedIndexAllocationStage::InitialLeafCapacity,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .clone_with_fallible_localized_index()
        .localized_text_index
        .is_none());
    let applied = engine.apply_typed_transaction_with_result(TypedTransaction {
        request_id: 700_150,
        base_document_revision: before_document_revision,
        origin: TransactionOrigin::LocalApi,
        operations: Vec::new(),
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: point,
            head: point,
        }),
        history_policy: HistoryPolicy::Auto,
    });
    force_localized_index_allocation_stage_for_test(None);

    let result = applied.expect("optional evidence copy failure cannot abort selection");
    assert!(result.changed);
    assert_eq!(result.document_revision, before_document_revision);
    assert_eq!(result.state_revision, before_state_revision + 1);
    let state = engine.derived_state.as_ref().unwrap();
    assert!(state.localized_text_index.is_none());
    assert_eq!(state.document_revision, before_document_revision);
    assert_eq!(state.state_revision, before_state_revision + 1);

    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 700_151,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_none());
}

#[test]
fn selection_only_revision_reseal_allows_following_strict_insert_admission() {
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
            request_id: 70_014,
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
    hydrate_import_for_compile_test(&mut engine);
    let state = engine.derived_state.as_ref().unwrap();
    assert_eq!(
        state.validation_certificate.state_revision(),
        engine.state_revision()
    );

    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_015,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_some());

    engine
        .apply_command(
            70_016,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    engine
        .apply_command(
            700_161,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    assert_eq!(
        state.validation_certificate.state_revision(),
        engine.state_revision()
    );
    let stored_marks = engine.stored_marks().unwrap_or_default().to_vec();
    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_017,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: point,
                text: "x".into(),
                marks: stored_marks,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    assert!(compiled.localized_insert_admission.is_some());
}

#[test]
fn benchmark_shaped_bursts_decompose_direct_result_and_command_full_passes() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, reset_localized_index_lifecycle_counts_for_test,
        take_active_state_cache_counts_for_test, take_localized_index_lifecycle_counts_for_test,
    };
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        let content = (0..160)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{
                        "type": "text",
                        "text": format!("{index:04} {}", "x".repeat(214))
                    }]
                })
            })
            .collect::<Vec<_>>();
        engine
            .import_json(
                &json!({"type": "doc", "content": content}).to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let point = RevisionedPosition {
            offset: 44,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 70_100,
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
        hydrate_import_for_compile_test(&mut engine);
        engine
    }

    fn direct(engine: &YrsDocumentEngine, index: usize) -> TypedTransaction {
        TypedTransaction {
            request_id: 70_200 + index as u64,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 44 + index as u32,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let mut direct_commit = fixture();
    let mut commit_counts = Vec::new();
    for index in 0..20 {
        reset_full_pass_counts_for_test();
        reset_localized_lookup_counts_for_test();
        reset_localized_index_lifecycle_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        let transaction = direct(&direct_commit, index);
        direct_commit.apply_typed_transaction(transaction).unwrap();
        commit_counts.push((
            take_full_pass_counts_for_test(),
            take_localized_lookup_counts_for_test(),
            take_localized_index_lifecycle_counts_for_test(),
            take_cached_render_counts_for_test(),
            take_localized_render_transition_counts_for_test(),
        ));
    }

    let mut direct_result = fixture();
    let mut result_counts = Vec::new();
    for index in 0..20 {
        reset_full_pass_counts_for_test();
        reset_localized_lookup_counts_for_test();
        reset_localized_index_lifecycle_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        let transaction = direct(&direct_result, index);
        direct_result
            .apply_typed_transaction_with_result(transaction)
            .unwrap();
        result_counts.push((
            take_full_pass_counts_for_test(),
            take_localized_lookup_counts_for_test(),
            take_localized_index_lifecycle_counts_for_test(),
            take_cached_render_counts_for_test(),
            take_localized_render_transition_counts_for_test(),
        ));
    }

    let mut command = fixture();
    let mut command_counts = Vec::new();
    reset_active_state_cache_counts_for_test();
    for index in 0..20 {
        reset_full_pass_counts_for_test();
        reset_localized_lookup_counts_for_test();
        reset_localized_index_lifecycle_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        command
            .apply_command(
                70_300 + index as u64,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        command_counts.push((
            take_full_pass_counts_for_test(),
            take_localized_lookup_counts_for_test(),
            take_localized_index_lifecycle_counts_for_test(),
            take_cached_render_counts_for_test(),
            take_localized_render_transition_counts_for_test(),
        ));
    }
    assert_eq!(
        take_active_state_cache_counts_for_test(),
        (20, 19, 1, 1, 20, 20, 0, 20, 1),
        "prepared command burst must build ActiveState once, then reuse it"
    );

    let expected_commit = (
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 0,
            document_validations: 0,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 0,
            canonical_projections: 1,
            canonical_serializations: 2,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 0,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 0,
            ordinary_step_applications: 0,
        },
        (0, 1, 1),
        (0, 1, 1, 0),
        (0, 1, 1, 0, 0),
        (1, 1, 0),
    );
    let expected_result = (
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 0,
            document_validations: 0,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 0,
            canonical_projections: 1,
            canonical_serializations: 2,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 0,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 1,
            ordinary_step_applications: 0,
        },
        (0, 1, 1),
        (0, 1, 1, 0),
        (0, 1, 1, 0, 0),
        (1, 1, 0),
    );
    let expected_command = (
        FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 1,
            document_validations: 1,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 321,
            canonical_projections: 1,
            canonical_serializations: 1,
            canonical_hashes: 1,
            affected_top_level_scans: 0,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 0,
            raw_document_text_scans: 1,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 1,
            ordinary_step_applications: 1,
        },
        (0, 1, 1),
        (0, 1, 1, 0),
        (0, 1, 1, 0, 0),
        (1, 1, 0),
    );
    for (index, actual) in commit_counts.iter().enumerate() {
        assert_eq!(*actual, expected_commit, "direct commit edit {index}");
    }
    for (index, actual) in result_counts.iter().enumerate() {
        assert_eq!(*actual, expected_result, "direct result edit {index}");
    }
    for (index, actual) in command_counts.iter().enumerate() {
        let mut expected = expected_command;
        expected.0.active_applicability_passes = usize::from(index == 0);
        assert_eq!(*actual, expected, "command edit {index}");
    }

    let mut promoted = fixture();
    let mut rebuilt = fixture();
    for index in 0..20 {
        rebuilt.derived_state.as_mut().unwrap().localized_text_index = None;
        let promoted_transaction = direct(&promoted, index);
        let rebuilt_transaction = direct(&rebuilt, index);
        let promoted_result = promoted
            .apply_typed_transaction_with_result(promoted_transaction)
            .unwrap();
        let rebuilt_result = rebuilt
            .apply_typed_transaction_with_result(rebuilt_transaction)
            .unwrap();
        assert_eq!(promoted_result, rebuilt_result, "sequential edit {index}");
        assert_eq!(promoted.document_json(), rebuilt.document_json());
        let promoted_state = promoted.derived_state.as_ref().unwrap();
        let rebuilt_state = rebuilt.derived_state.as_ref().unwrap();
        assert_eq!(
            promoted_state.validation_certificate, rebuilt_state.validation_certificate,
            "sequential edit {index}"
        );
        assert_eq!(
            promoted_state.localized_text_index, rebuilt_state.localized_text_index,
            "sequential edit {index}"
        );
    }
    assert_eq!(
        promoted.undo(700_153).unwrap(),
        rebuilt.undo(700_153).unwrap()
    );
    assert_eq!(promoted.document_json(), rebuilt.document_json());
    assert_eq!(
        promoted.redo(700_154).unwrap(),
        rebuilt.redo(700_154).unwrap()
    );
    assert_eq!(promoted.document_json(), rebuilt.document_json());
}
