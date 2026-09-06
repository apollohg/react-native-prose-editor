use super::*;

#[test]
fn localized_insert_preserves_semantic_validation_error_precedence_over_lowering_limits() {
    fn constrained_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.editing_limits.max_operations_per_transaction = 1;
        engine.resource_limits.max_document_depth = 1;
        engine.resource_limits.max_document_nodes = 1;
        engine
    }

    let localized = constrained_engine();
    let localized_error = localized
        .compile_typed_transaction(insert_transaction(&localized, 70_143))
        .unwrap_err();

    let eager = constrained_engine();
    let mut eager_transaction = insert_transaction(&eager, 70_143);
    eager_transaction.selection_intent = SelectionIntent::Set(SelectionInput::All);
    let eager_error = eager
        .compile_typed_transaction(eager_transaction)
        .unwrap_err();

    assert_eq!(localized_error, eager_error);
    assert_eq!(localized_error.code, "DOCUMENT_LIMIT_EXCEEDED");
}

#[test]
fn engine_compile_reuses_all_cached_base_semantic_inputs() {
    use crate::yrs_engine::compiler::{
        reset_base_compilation_build_counts_for_test, take_base_compilation_build_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 70_002);
    let TypedOperation::InsertText { at, .. } = &mut transaction.operations[0] else {
        unreachable!()
    };
    at.offset = 2;
    let point = RevisionedPosition {
        offset: 2,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point,
        head: point,
    });
    reset_base_compilation_build_counts_for_test();

    engine.compile_typed_transaction(transaction).unwrap();

    assert_eq!(take_base_compilation_build_counts_for_test(), (0, 0, 0));
}

#[test]
fn selection_only_revision_refreshes_the_cached_compilation_view() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_003,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(2),
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    assert_eq!(
        engine.derived_state.as_ref().unwrap().legacy_selection,
        crate::selection::Selection::text(2, 3)
    );
    engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
}

#[test]
fn changed_rich_command_derives_preview_map_and_render_at_most_once() {
    use crate::yrs_engine::derived_state::{
        reset_preview_derivation_counts_for_test, take_preview_derivation_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_preview_derivation_counts_for_test();

    engine
        .apply_command(70_007, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    let (position_maps, rendered_texts) = take_preview_derivation_counts_for_test();
    assert!(position_maps <= 1, "built {position_maps} preview maps");
    assert!(
        rendered_texts <= 1,
        "built {rendered_texts} preview renders"
    );
}

#[test]
fn existing_text_command_skips_every_proved_document_wide_compiler_pass() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let caret = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_008,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: caret,
                head: caret,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    hydrate_import_for_compile_test(&mut engine);
    reset_full_pass_counts_for_test();

    engine
        .apply_command(70_009, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(
        take_full_pass_counts_for_test(),
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
            canonical_identity_predicate_nodes_visited: 3,
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
        }
    );
}

#[test]
fn existing_text_admission_certificate_matches_legacy_compiler_and_commit() {
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
            request_id: 70_010,
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
    let transaction = TypedTransaction {
        request_id: 70_011,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: point,
            text: "🙂\\\"‍".into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let compiled = engine
        .compile_typed_transaction(transaction.clone())
        .unwrap();
    let proof = compiled
        .localized_insert_admission
        .as_ref()
        .expect("strict-inside existing text produces E1 admission evidence")
        .clone();
    let read_txn = engine.doc.transact();
    let fragment = read_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let current = engine.derived_state.as_ref().unwrap();
    let admission_document_position = crate::yrs_engine::position::editor_offset_to_doc_pos(
        point.offset,
        point.kind,
        &current.rendered_text,
        &current.position_map,
        &current.document,
    )
    .unwrap();
    let validated = proof
        .validate_current(
            current,
            &transaction,
            admission_document_position,
            &read_txn,
            &fragment,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .expect("every private admission claim revalidates");
    let mut same_metrics_different_text = transaction.clone();
    let [TypedOperation::InsertText { text, .. }] =
        same_metrics_different_text.operations.as_mut_slice()
    else {
        unreachable!()
    };
    *text = "🙃\\\"‍".into();
    assert!(proof
        .validate_current(
            current,
            &same_metrics_different_text,
            admission_document_position,
            &read_txn,
            &fragment,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .is_none());
    for (claim, tampered) in proof.tampered_claims_for_test() {
        assert!(
            tampered
                .validate_current(
                    current,
                    &transaction,
                    admission_document_position,
                    &read_txn,
                    &fragment,
                    &engine.resource_limits,
                    &engine.editing_limits,
                    engine.max_length,
                    engine.yrs_state_epoch,
                )
                .is_none(),
            "tampered private claim must fail closed: {claim}"
        );
    }
    drop(read_txn);
    let full_stats =
        DocumentValidator::validate(&compiled.preview, &engine.schema, &engine.resource_limits)
            .unwrap();
    assert_eq!(
        full_stats,
        engine
            .derived_state
            .as_ref()
            .unwrap()
            .validation_certificate
            .stats()
    );
    let artifact = compiled.canonical_artifact.as_ref().unwrap();
    assert_eq!(
        artifact.text_scalar_len(),
        validated.next_raw_text_scalars()
    );
    assert_eq!(
        artifact.text_utf8_bytes(),
        validated.next_raw_text_utf8_bytes()
    );
    assert_eq!(
        artifact.serialized_len(),
        validated.next_canonical_serialized_len()
    );
    assert_eq!(compiled.undo_units_bound, validated.history_undo_units());
    assert_eq!(
        compiled.replay_work_units_bound,
        validated.history_undo_units()
    );
    assert_eq!(
        compiled
            .preview_derivations
            .as_ref()
            .unwrap()
            .position_map
            .total_scalars(),
        validated.next_rendered_scalars()
    );
    let expected_fingerprint = artifact.sha256();
    let expected_operation_result = validated.operation_result().clone();
    let expected_stored_marks = validated.stored_marks().map(<[_]>::to_vec);
    let expected_rendered_scalars = validated.next_rendered_scalars();

    let result = engine
        .apply_compiled_transaction(compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(result.selection, expected_operation_result);
    assert_eq!(engine.stored_marks(), expected_stored_marks.as_deref());
    assert!(engine.can_undo());
    let next = engine.derived_state.as_ref().unwrap();
    assert_eq!(next.validation_certificate.stats(), full_stats);
    assert_eq!(
        next.validation_certificate.canonical_fingerprint(),
        expected_fingerprint
    );
    assert_eq!(next.position_map.total_scalars(), expected_rendered_scalars);
    assert_eq!(
        u32::try_from(next.rendered_text.chars().count()).unwrap(),
        expected_rendered_scalars
    );
}

#[test]
fn admission_evidence_does_zero_work_before_envelope_admission() {
    use crate::yrs_engine::derived_state::{
        reset_localized_insert_admission_work_for_test,
        take_localized_insert_admission_work_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let position = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let insert = |base_document_revision, origin, text: &str| TypedTransaction {
        request_id: 70_012,
        base_document_revision,
        origin,
        operations: vec![TypedOperation::InsertText {
            at: position,
            text: text.into(),
            marks: Vec::new(),
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    reset_localized_insert_admission_work_for_test();
    assert!(engine
        .compile_typed_transaction(insert(
            engine.revision().saturating_add(1),
            TransactionOrigin::LocalInput,
            "x",
        ))
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    reset_localized_insert_admission_work_for_test();
    assert!(engine
        .compile_typed_transaction(insert(
            engine.revision(),
            TransactionOrigin::RemoteSync,
            "x",
        ))
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    engine.editing_limits.max_operations_per_transaction = 1;
    let mut excess = insert(engine.revision(), TransactionOrigin::LocalInput, "x");
    excess.operations.push(excess.operations[0].clone());
    reset_localized_insert_admission_work_for_test();
    assert!(engine.compile_typed_transaction(excess).is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    engine.resource_limits.max_input_bytes = 1;
    reset_localized_insert_admission_work_for_test();
    assert!(engine
        .compile_typed_transaction(insert(
            engine.revision(),
            TransactionOrigin::LocalInput,
            "oversized",
        ))
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);
}

#[test]
fn localized_insert_admission_does_zero_work_before_cached_view_and_yrs_scan_admission() {
    use crate::yrs_engine::derived_state::{
        reset_localized_insert_admission_work_for_test,
        take_localized_insert_admission_work_for_test,
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

    let mut cached_view_rejection = fixture();
    let cached_transaction = transaction(&cached_view_rejection, 700_122);
    cached_view_rejection
        .derived_state
        .as_mut()
        .unwrap()
        .rendered_scalars += 1;
    reset_localized_insert_admission_work_for_test();
    assert!(cached_view_rejection
        .compile_typed_transaction(cached_transaction)
        .is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);

    let mut yrs_scan_rejection = fixture();
    let scan_transaction = transaction(&yrs_scan_rejection, 700_123);
    yrs_scan_rejection.resource_limits.max_input_bytes = 8;
    reset_localized_insert_admission_work_for_test();
    let error = yrs_scan_rejection
        .compile_typed_transaction(scan_transaction)
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details.as_ref().unwrap()["field"], "maxInputBytes");
    assert_eq!(take_localized_insert_admission_work_for_test(), 0);
}

#[test]
fn localized_insert_admission_runs_before_mutation_preflight() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::{
        reset_localized_insert_admission_work_for_test,
        take_localized_insert_admission_work_for_test,
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
        request_id: 700_121,
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

    reset_localized_insert_admission_work_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let result = engine.compile_typed_transaction(transaction);
    set_atomic_failpoint_for_test(None);

    assert!(result.is_err());
    assert_eq!(take_localized_insert_admission_work_for_test(), 1);
}

#[test]
fn admission_evidence_rejects_unsupported_selection_and_history_contracts() {
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
    let transaction = |selection_intent, history_policy| TypedTransaction {
        request_id: 70_013,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations: vec![TypedOperation::InsertText {
            at: point,
            text: "x".into(),
            marks: Vec::new(),
        }],
        selection_intent,
        history_policy,
    };

    assert!(engine
        .compile_typed_transaction(transaction(SelectionIntent::Preserve, HistoryPolicy::Auto,))
        .unwrap()
        .localized_insert_admission
        .is_none());
    assert!(engine
        .compile_typed_transaction(transaction(
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Skip,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());
}

#[test]
fn localized_insert_admission_eligibility_is_exact() {
    let fixture = |marked: bool| {
        let mut engine = transaction_engine();
        let json = if marked {
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abc"}]}]}"#
        } else {
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#
        };
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        engine
    };
    let point = |offset| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let transaction = |engine: &YrsDocumentEngine,
                       origin,
                       at,
                       text: &str,
                       marks,
                       selection_intent,
                       history_policy| TypedTransaction {
        request_id: 700_131,
        base_document_revision: engine.revision(),
        origin,
        operations: vec![TypedOperation::InsertText {
            at,
            text: text.into(),
            marks,
        }],
        selection_intent,
        history_policy,
    };

    let engine = fixture(false);
    for origin in [
        TransactionOrigin::LocalInput,
        TransactionOrigin::LocalCommand,
        TransactionOrigin::LocalApi,
    ] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                origin,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_some());
    }

    for boundary in [point(0), point(3)] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                TransactionOrigin::LocalInput,
                boundary,
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Auto,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }

    for history_policy in [HistoryPolicy::Boundary, HistoryPolicy::Skip] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                TransactionOrigin::LocalInput,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                history_policy,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }
    for origin in [TransactionOrigin::LocalCommand, TransactionOrigin::LocalApi] {
        assert!(engine
            .compile_typed_transaction(transaction(
                &engine,
                origin,
                point(1),
                "x",
                Vec::new(),
                SelectionIntent::UseOperationResult,
                HistoryPolicy::Boundary,
            ))
            .unwrap()
            .localized_insert_admission
            .is_none());
    }
    assert!(engine
        .compile_typed_transaction(transaction(
            &engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::Preserve,
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());
    assert!(engine
        .compile_typed_transaction(transaction(
            &engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::Set(SelectionInput::Text {
                anchor: point(1),
                head: point(1),
            }),
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());

    let mut multiple = transaction(
        &engine,
        TransactionOrigin::LocalInput,
        point(1),
        "x",
        Vec::new(),
        SelectionIntent::UseOperationResult,
        HistoryPolicy::Auto,
    );
    multiple.operations.push(multiple.operations[0].clone());
    assert!(engine
        .compile_typed_transaction(multiple)
        .unwrap()
        .localized_insert_admission
        .is_none());

    let marked_engine = fixture(true);
    let bold = vec![Mark::new("bold".into(), HashMap::new())];
    assert!(marked_engine
        .compile_typed_transaction(transaction(
            &marked_engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            bold,
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_some());
    assert!(marked_engine
        .compile_typed_transaction(transaction(
            &marked_engine,
            TransactionOrigin::LocalInput,
            point(1),
            "x",
            Vec::new(),
            SelectionIntent::UseOperationResult,
            HistoryPolicy::Auto,
        ))
        .unwrap()
        .localized_insert_admission
        .is_none());
}

#[test]
fn localized_insert_admission_preserves_generic_results_errors_and_full_pass_counts() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
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
    let transaction = |engine: &YrsDocumentEngine, request_id, marks| TypedTransaction {
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
            marks,
        }],
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    };

    let mut admitted = fixture();
    reset_full_pass_counts_for_test();
    let admitted_result = admitted
        .apply_typed_transaction_with_result(transaction(&admitted, 700_132, Vec::new()))
        .unwrap();
    let admitted_counts = take_full_pass_counts_for_test();

    let mut generic = fixture();
    generic.derived_state.as_mut().unwrap().localized_text_index = None;
    reset_full_pass_counts_for_test();
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction(&generic, 700_132, Vec::new()))
        .unwrap();
    let generic_counts = take_full_pass_counts_for_test();

    assert_eq!(admitted_result, generic_result);
    assert_eq!(admitted.document_json(), generic.document_json());
    assert_eq!(admitted_counts.ordinary_step_applications, 0);
    assert_eq!(generic_counts.ordinary_step_applications, 1);
    assert_eq!(admitted.can_undo(), generic.can_undo());
    assert_eq!(admitted.can_redo(), generic.can_redo());

    let admitted_undo = admitted.undo(700_141).unwrap();
    let generic_undo = generic.undo(700_141).unwrap();
    assert_eq!(admitted_undo, generic_undo);
    assert_eq!(admitted.document_json(), generic.document_json());
    assert_eq!(admitted.can_undo(), generic.can_undo());
    assert_eq!(admitted.can_redo(), generic.can_redo());

    let admitted_redo = admitted.redo(700_142).unwrap();
    let generic_redo = generic.redo(700_142).unwrap();
    assert_eq!(admitted_redo, generic_redo);
    assert_eq!(admitted.document_json(), generic.document_json());
    assert_eq!(admitted.can_undo(), generic.can_undo());
    assert_eq!(admitted.can_redo(), generic.can_redo());

    let invalid_mark = vec![Mark::new("unknown".into(), HashMap::new())];
    let mut admitted_error_engine = fixture();
    let mut generic_error_engine = fixture();
    generic_error_engine
        .derived_state
        .as_mut()
        .unwrap()
        .localized_text_index = None;
    let admitted_error = admitted_error_engine
        .apply_typed_transaction_with_result(transaction(
            &admitted_error_engine,
            700_133,
            invalid_mark.clone(),
        ))
        .unwrap_err();
    let generic_error = generic_error_engine
        .apply_typed_transaction_with_result(transaction(
            &generic_error_engine,
            700_133,
            invalid_mark,
        ))
        .unwrap_err();
    assert_eq!(admitted_error, generic_error);
    assert_eq!(
        admitted_error_engine.document_json(),
        generic_error_engine.document_json()
    );
}

include!("localized_compilation/insert_parity.rs");

include!("localized_compilation/render_evidence.rs");

include!("localized_compilation/optional_indexes.rs");

include!("localized_compilation/active_state_evidence.rs");

include!("localized_compilation/active_state_parity.rs");
