#[test]
fn localized_insert_compile_only_skips_every_proved_full_pass() {
    use crate::model::node::{
        reset_deep_node_payload_clones_for_test, take_deep_node_payload_clones_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    let fixture = || {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    };
    let transaction = |engine: &YrsDocumentEngine, request_id| TypedTransaction {
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
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let eligible = fixture();
    reset_full_pass_counts_for_test();
    let compiled = eligible
        .compile_typed_transaction(transaction(&eligible, 700_134))
        .unwrap();
    assert_eq!(compiled.affected_top_level_blocks, vec![0]);
    assert_eq!(
        take_full_pass_counts_for_test(),
        FullPassCounts {
            canonical_projections: 1,
            canonical_serializations: 2,
            canonical_hashes: 1,
            position_map_clones: 1,
            position_map_compactions: 1,
            render_identity_scans: 0,
            ..FullPassCounts::default()
        }
    );

    let mut generic = fixture();
    generic.derived_state.as_mut().unwrap().localized_text_index = None;
    reset_full_pass_counts_for_test();
    generic
        .compile_typed_transaction(transaction(&generic, 700_135))
        .unwrap();
    assert_eq!(
        take_full_pass_counts_for_test(),
        FullPassCounts {
            document_validations: 2,
            canonical_mark_tree_scans: 1,
            canonical_mark_validation_attempts: 1,
            canonical_mark_validation_completions: 1,
            canonical_mark_nodes_visited: 3,
            canonical_identity_predicate_nodes_visited: 3,
            canonical_projections: 1,
            canonical_serializations: 1,
            canonical_hashes: 0,
            affected_top_level_scans: 1,
            position_map_clones: 1,
            position_map_compactions: 1,
            rendered_text_derivations: 1,
            raw_document_text_scans: 2,
            document_node_count_scans: 1,
            render_identity_scans: 0,
            ordinary_step_applications: 1,
            ..FullPassCounts::default()
        }
    );

    let mut wide = transaction_engine();
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
    wide.import_json(
        &json!({"type": "doc", "content": content}).to_string(),
        TransactionOrigin::DocumentImport,
    )
    .unwrap();
    let rendered = &wide.derived_state.as_ref().unwrap().rendered_text;
    let needle = "0159 ";
    let needle_byte = rendered.find(needle).unwrap();
    let offset = u32::try_from(rendered[..needle_byte].chars().count() + needle.len()).unwrap();
    reset_deep_node_payload_clones_for_test();
    wide.compile_typed_transaction(TypedTransaction {
        request_id: 700_143,
        base_document_revision: wide.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: RevisionedPosition {
                offset,
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
    assert_eq!(
        take_deep_node_payload_clones_for_test(),
        0,
        "localized reconstruction must copy only immutable node handles"
    );
}

#[test]
fn localized_insert_semantic_preview_matches_forced_generic_matrix() {
    fn assert_compiled_parity(
        localized: &crate::yrs_engine::compiler::CompiledTransaction,
        generic: &crate::yrs_engine::compiler::CompiledTransaction,
    ) {
        assert_eq!(localized.preview, generic.preview);
        let localized_artifact = localized.canonical_artifact.as_ref().unwrap();
        let generic_artifact = generic.canonical_artifact.as_ref().unwrap();
        assert_eq!(localized_artifact.value(), generic_artifact.value());
        assert_eq!(localized_artifact.sha256(), generic_artifact.sha256());
        assert_eq!(
            localized_artifact.serialized_len(),
            generic_artifact.serialized_len()
        );
        assert_eq!(
            localized_artifact.text_scalar_len(),
            generic_artifact.text_scalar_len()
        );
        assert_eq!(
            localized_artifact.text_utf8_bytes(),
            generic_artifact.text_utf8_bytes()
        );
        assert!(localized_artifact.matches_document(&localized.preview));
        assert!(generic_artifact.matches_document(&generic.preview));
        assert_eq!(
            localized.composed_map.ranges(),
            generic.composed_map.ranges()
        );
        assert_eq!(localized.selection_plan, generic.selection_plan);
        assert_eq!(
            localized.relative_selection_plan,
            generic.relative_selection_plan
        );
        assert_eq!(localized.stored_marks_plan, generic.stored_marks_plan);
        assert_eq!(localized.history_class, generic.history_class);
        assert_eq!(localized.undo_units_bound, generic.undo_units_bound);
        assert_eq!(
            localized.replay_work_units_bound,
            generic.replay_work_units_bound
        );
        assert_eq!(localized.encoded_growth_bound, generic.encoded_growth_bound);
        assert_eq!(localized.authored_clock_units, generic.authored_clock_units);
        assert_eq!(
            localized.affected_top_level_blocks,
            generic.affected_top_level_blocks
        );
        assert_eq!(localized.position_update_mode, generic.position_update_mode);
        assert_eq!(
            format!("{:?}", localized.mutation_plan.actions),
            format!("{:?}", generic.mutation_plan.actions)
        );
        assert_eq!(
            localized.mutation_plan.compilation_work_for_test(),
            generic.mutation_plan.compilation_work_for_test()
        );
        assert_eq!(
            localized.mutation_plan.expected_preflight_work_for_test(),
            generic.mutation_plan.expected_preflight_work_for_test()
        );
        assert_eq!(
            localized.mutation_plan.scan_work,
            generic.mutation_plan.scan_work
        );
        let localized_derived = localized.preview_derivations.as_ref().unwrap();
        let generic_derived = generic.preview_derivations.as_ref().unwrap();
        assert_eq!(
            localized_derived.rendered_text,
            generic_derived.rendered_text
        );
        assert_eq!(
            localized_derived.rendered_scalars,
            generic_derived.rendered_scalars
        );
        assert_eq!(
            localized_derived.document_text_bytes,
            generic_derived.document_text_bytes
        );
        assert_eq!(
            localized_derived.document_node_count,
            generic_derived.document_node_count
        );
        assert_eq!(
            format!("{:?}", localized_derived.position_map),
            format!("{:?}", generic_derived.position_map)
        );
    }

    let cases = [
        (
            "ascii",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            "abc",
            1usize,
            "x",
            Vec::new(),
            vec![0],
        ),
        (
            "non-bmp-escaped-control",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            "abc",
            1,
            "🙂\\\"\n\u{1}",
            Vec::new(),
            vec![0],
        ),
        (
            "canonical-mark",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abc"}]}]}"#,
            "abc",
            1,
            "x",
            vec![Mark::new("bold".into(), HashMap::new())],
            vec![0],
        ),
        (
            "fragmented-mark-leaves",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"ab"},{"type":"text","marks":[{"type":"italic"}],"text":"cd"}]}]}"#,
            "cd",
            1,
            "🙂",
            vec![Mark::new("italic".into(), HashMap::new())],
            vec![0],
        ),
        (
            "deep-nesting",
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
            "abc",
            1,
            "x",
            Vec::new(),
            vec![0],
        ),
        (
            "list-prefix",
            r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}]}"#,
            "abc",
            1,
            "x",
            Vec::new(),
            vec![0],
        ),
        (
            "third-top-level",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]},{"type":"paragraph","content":[{"type":"text","text":"second"}]},{"type":"paragraph","content":[{"type":"text","text":"third"}]}]}"#,
            "third",
            1,
            "x",
            Vec::new(),
            vec![1, 2],
        ),
    ];

    for (case, json, needle, inside, inserted, marks, expected_affected) in cases {
        let mut engine = transaction_engine();
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let rendered = &engine.derived_state.as_ref().unwrap().rendered_text;
        let needle_byte = rendered.find(needle).unwrap();
        let offset = u32::try_from(rendered[..needle_byte].chars().count() + inside).unwrap();
        let transaction = TypedTransaction {
            request_id: 700_136,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: inserted.into(),
                marks,
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        };

        let localized = engine
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        assert!(localized.localized_insert_admission.is_some(), "{case}");
        assert_eq!(
            localized.affected_top_level_blocks, expected_affected,
            "{case}"
        );
        let saved_index = engine
            .derived_state
            .as_mut()
            .unwrap()
            .localized_text_index
            .take();
        let generic = engine
            .compile_typed_transaction(transaction.clone())
            .unwrap();
        engine.derived_state.as_mut().unwrap().localized_text_index = saved_index;
        assert_compiled_parity(&localized, &generic);

        let localized_result = engine
            .apply_compiled_transaction(localized, true)
            .unwrap()
            .1
            .unwrap();
        let mut generic_engine = transaction_engine();
        generic_engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        generic_engine
            .derived_state
            .as_mut()
            .unwrap()
            .localized_text_index = None;
        let generic_compiled = generic_engine
            .compile_typed_transaction(transaction)
            .unwrap();
        let generic_result = generic_engine
            .apply_compiled_transaction(generic_compiled, true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(localized_result, generic_result, "{case}");
        assert_eq!(
            engine.document_json(),
            generic_engine.document_json(),
            "{case}"
        );
        let localized_state = engine.derived_state.as_ref().unwrap();
        let generic_state = generic_engine.derived_state.as_ref().unwrap();
        assert_eq!(
            localized_state.validation_certificate, generic_state.validation_certificate,
            "{case}"
        );
        assert_eq!(
            localized_state.localized_text_index, generic_state.localized_text_index,
            "{case}"
        );
        assert_eq!(
            localized_state.canonical_artifact.value(),
            generic_state.canonical_artifact.value(),
            "{case}"
        );
        assert_eq!(
            localized_state.canonical_artifact.sha256(),
            generic_state.canonical_artifact.sha256(),
            "{case}"
        );
        assert_eq!(
            localized_state.rendered_text, generic_state.rendered_text,
            "{case}"
        );
        assert_eq!(engine.can_undo(), generic_engine.can_undo(), "{case}");
        assert_eq!(engine.can_redo(), generic_engine.can_redo(), "{case}");
        assert_eq!(
            engine.undo(700_151).unwrap(),
            generic_engine.undo(700_151).unwrap(),
            "{case}"
        );
        assert_eq!(
            engine.document_json(),
            generic_engine.document_json(),
            "{case}"
        );
        assert_eq!(
            engine.redo(700_152).unwrap(),
            generic_engine.redo(700_152).unwrap(),
            "{case}"
        );
        assert_eq!(
            engine.document_json(),
            generic_engine.document_json(),
            "{case}"
        );
    }

    use crate::yrs_engine::compiler::force_localized_semantic_allocation_failure_for_test;
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    let transaction = TypedTransaction {
        request_id: 700_139,
        base_document_revision: engine.revision(),
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
    };
    reset_full_pass_counts_for_test();
    force_localized_semantic_allocation_failure_for_test(true);
    let fallback = engine.compile_typed_transaction(transaction.clone());
    force_localized_semantic_allocation_failure_for_test(false);
    let fallback = fallback.unwrap();
    assert!(fallback.localized_insert_admission.is_some());
    assert_eq!(
        take_full_pass_counts_for_test().ordinary_step_applications,
        1
    );
    let saved_index = engine
        .derived_state
        .as_mut()
        .unwrap()
        .localized_text_index
        .take();
    let generic = engine.compile_typed_transaction(transaction).unwrap();
    engine.derived_state.as_mut().unwrap().localized_text_index = saved_index;
    assert_compiled_parity(&fallback, &generic);
}

#[test]
fn localized_insert_exact_limits_and_one_under_errors_match_generic() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };
    use crate::yrs_engine::EditingLimits;

    fn fixture(max_length: Option<u32>, editing_limits: EditingLimits) -> YrsDocumentEngine {
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits,
            max_length,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "doc".into(),
                lineage_id: "lineage".into(),
            }),
        })
        .unwrap();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    }

    fn transaction(engine: &YrsDocumentEngine) -> TypedTransaction {
        TypedTransaction {
            request_id: 700_140,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "xy".into(),
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    fn assert_error_pair(
        localized: YrsDocumentEngine,
        mut generic: YrsDocumentEngine,
        field: &str,
    ) {
        generic.derived_state.as_mut().unwrap().localized_text_index = None;
        reset_full_pass_counts_for_test();
        let localized_error = localized
            .compile_typed_transaction(transaction(&localized))
            .unwrap_err();
        assert_eq!(
            take_full_pass_counts_for_test().ordinary_step_applications,
            1,
            "{field} must silently fall back to generic compilation"
        );
        let generic_error = generic
            .compile_typed_transaction(transaction(&generic))
            .unwrap_err();
        assert_eq!(localized_error, generic_error);
        assert_eq!(localized_error.details.as_ref().unwrap()["field"], field);
    }

    let probe_engine = fixture(None, EditingLimits::default());
    let probe = probe_engine
        .compile_typed_transaction(transaction(&probe_engine))
        .unwrap();
    let exact_output = probe.canonical_artifact.as_ref().unwrap().serialized_len();
    let exact_undo = probe.undo_units_bound;

    let exact_length = fixture(Some(5), EditingLimits::default());
    assert!(exact_length
        .compile_typed_transaction(transaction(&exact_length))
        .unwrap()
        .localized_insert_admission
        .is_some());
    let rejected_length = fixture(Some(4), EditingLimits::default());
    let generic_length = fixture(Some(4), EditingLimits::default());
    assert_error_pair(rejected_length, generic_length, "maxLength");

    let exact_output_limits = EditingLimits {
        max_derived_output_bytes: exact_output,
        ..EditingLimits::default()
    };
    let exact_output_engine = fixture(None, exact_output_limits);
    assert!(exact_output_engine
        .compile_typed_transaction(transaction(&exact_output_engine))
        .unwrap()
        .localized_insert_admission
        .is_some());
    let rejected_output_limits = EditingLimits {
        max_derived_output_bytes: exact_output - 1,
        ..EditingLimits::default()
    };
    let rejected_output = fixture(None, rejected_output_limits.clone());
    let generic_output = fixture(None, rejected_output_limits);
    assert_error_pair(rejected_output, generic_output, "maxDerivedOutputBytes");

    let exact_undo_limits = EditingLimits {
        max_undo_retained_units: exact_undo,
        ..EditingLimits::default()
    };
    let exact_undo_engine = fixture(None, exact_undo_limits);
    assert!(exact_undo_engine
        .compile_typed_transaction(transaction(&exact_undo_engine))
        .unwrap()
        .localized_insert_admission
        .is_some());
    let rejected_undo_limits = EditingLimits {
        max_undo_retained_units: exact_undo - 1,
        ..EditingLimits::default()
    };
    let rejected_undo = fixture(None, rejected_undo_limits.clone());
    let generic_undo = fixture(None, rejected_undo_limits);
    assert_error_pair(rejected_undo, generic_undo, "maxUndoRetainedUnits");
}

#[test]
fn localized_index_promotion_allocation_failures_drop_only_optional_index() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_allocation_stage_for_test, force_localized_index_budget_for_test,
        reset_localized_index_lifecycle_counts_for_test,
        take_localized_index_lifecycle_counts_for_test, LocalizedIndexAllocationStage,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
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
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let mut baseline = fixture();
    let baseline_result = baseline
        .apply_typed_transaction_with_result(transaction(&baseline, 700_144))
        .unwrap();
    let baseline_json = baseline.document_json();

    for (index, stage) in [
        LocalizedIndexAllocationStage::PromotionClone,
        LocalizedIndexAllocationStage::PromotionGrowth,
        LocalizedIndexAllocationStage::PromotionUpdate,
    ]
    .into_iter()
    .enumerate()
    {
        let mut engine = fixture();
        reset_localized_index_lifecycle_counts_for_test();
        force_localized_index_allocation_stage_for_test(Some(stage));
        let compiled = engine.compile_typed_transaction(transaction(
            &engine,
            700_145 + u64::try_from(index).unwrap(),
        ));
        force_localized_index_allocation_stage_for_test(None);
        let result = engine
            .apply_compiled_transaction(compiled.unwrap(), true)
            .unwrap()
            .1
            .unwrap();
        assert_eq!(result.changed, baseline_result.changed, "{stage:?}");
        assert_eq!(result.selection, baseline_result.selection, "{stage:?}");
        assert_eq!(engine.document_json(), baseline_json, "{stage:?}");
        assert!(engine.can_undo(), "{stage:?}");
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .localized_text_index
            .is_none());
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (0, 1, 0, 1),
            "{stage:?}"
        );
    }

    let mut engine = fixture();
    reset_localized_index_lifecycle_counts_for_test();
    force_localized_index_budget_for_test(Some(0));
    let compiled = engine.compile_typed_transaction(transaction(&engine, 700_149));
    force_localized_index_budget_for_test(None);
    engine
        .apply_compiled_transaction(compiled.unwrap(), true)
        .unwrap();
    assert_eq!(engine.document_json(), baseline_json);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .is_none());
    assert_eq!(
        take_localized_index_lifecycle_counts_for_test(),
        (0, 1, 0, 1)
    );
}

#[test]
fn localized_index_promotion_obeys_exact_transient_budget_boundary() {
    use crate::yrs_engine::derived_state::{
        force_localized_index_budget_for_test, reset_localized_index_lifecycle_counts_for_test,
        take_localized_index_lifecycle_counts_for_test,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
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
                marks: Vec::new(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    fn history_audit(engine: &YrsDocumentEngine) -> (bool, bool, u64, (usize, usize, bool)) {
        (
            engine.can_undo(),
            engine.can_redo(),
            engine.history.retained_units(0).unwrap(),
            engine.history.replay_audit_for_test(),
        )
    }

    let mut exact = fixture();
    let exact_budget = exact
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .as_ref()
        .unwrap()
        .promotion_transient_budget_for_test()
        .unwrap();
    reset_localized_index_lifecycle_counts_for_test();
    force_localized_index_budget_for_test(Some(exact_budget));
    let exact_compiled = exact
        .compile_typed_transaction(transaction(&exact, 700_162))
        .unwrap();
    force_localized_index_budget_for_test(None);
    let exact_result = exact
        .apply_compiled_transaction(exact_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(
        take_localized_index_lifecycle_counts_for_test(),
        (0, 1, 1, 0)
    );

    let mut generic = fixture();
    generic.derived_state.as_mut().unwrap().localized_text_index = None;
    let generic_transaction = transaction(&generic, 700_162);
    let generic_result = generic
        .apply_typed_transaction_with_result(generic_transaction)
        .unwrap();
    assert_eq!(exact_result, generic_result);
    assert_eq!(exact.document_json(), generic.document_json());
    assert_eq!(history_audit(&exact), history_audit(&generic));
    assert_eq!(
        exact.derived_state.as_ref().unwrap().localized_text_index,
        generic.derived_state.as_ref().unwrap().localized_text_index
    );

    let mut one_under = fixture();
    reset_localized_index_lifecycle_counts_for_test();
    force_localized_index_budget_for_test(Some(exact_budget - 1));
    let one_under_compiled = one_under
        .compile_typed_transaction(transaction(&one_under, 700_162))
        .unwrap();
    force_localized_index_budget_for_test(None);
    let one_under_result = one_under
        .apply_compiled_transaction(one_under_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(one_under_result, generic_result);
    assert_eq!(one_under.document_json(), generic.document_json());
    assert_eq!(history_audit(&one_under), history_audit(&generic));
    assert!(one_under
        .derived_state
        .as_ref()
        .unwrap()
        .localized_text_index
        .is_none());
    assert_eq!(
        take_localized_index_lifecycle_counts_for_test(),
        (0, 1, 0, 1)
    );
}
