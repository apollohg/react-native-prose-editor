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

#[test]
fn every_localized_derived_evidence_tamper_falls_back_before_write() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::{
        reset_localized_index_lifecycle_counts_for_test,
        take_localized_index_lifecycle_counts_for_test, PreparedDerivedEvidence,
    };

    for case in PreparedDerivedEvidence::tamper_cases_for_test() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_150,
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
            })
            .unwrap();
        compiled
            .prepared_derived_evidence
            .as_mut()
            .unwrap()
            .tamper_for_test(case);
        let before = atomic_audit(&engine);
        reset_localized_index_lifecycle_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_atomic_failpoint_for_test(None);
        assert!(applied.is_err(), "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert_eq!(
            take_localized_index_lifecycle_counts_for_test(),
            (1, 0, 0, 0),
            "{case} must prepare generic evidence before the failpoint"
        );
    }
}

#[test]
fn every_localized_render_proof_tamper_falls_back_before_write() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::PreparedDerivedEvidence;
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let cases = PreparedDerivedEvidence::localized_render_tamper_cases_for_test()
        .iter()
        .copied()
        .chain(std::iter::once("affectedRange"));
    for case in cases {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_151,
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
            })
            .unwrap();
        if case == "affectedRange" {
            compiled.affected_top_level_blocks.clear();
        } else {
            compiled
                .prepared_derived_evidence
                .as_mut()
                .unwrap()
                .tamper_localized_render_for_test(case);
        }
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .expect_err("durable metadata failpoint must abort the fallback commit");
        set_atomic_failpoint_for_test(None);

        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        let passes = take_full_pass_counts_for_test();
        assert_eq!(passes.render_limit_tree_scans, 1, "{case}");
        assert_eq!(passes.render_identity_scans, 0, "{case}");
        assert_eq!(passes.render_top_level_start_scans, 1, "{case}");
        assert_eq!(
            take_cached_render_counts_for_test(),
            (0, 1, 1, 0, 0),
            "{case}"
        );
        assert_eq!(
            take_localized_render_transition_counts_for_test(),
            (1, 0, 1),
            "{case}"
        );
    }
}

#[test]
fn malformed_multiblock_localized_render_ranges_fall_back_exactly() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
    };
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    struct RangeAudit {
        error: crate::yrs_engine::OperationError,
        cached_counts: (usize, usize, usize, usize, usize),
        lifecycle_counts: (usize, usize, usize),
        full_pass_counts: FullPassCounts,
    }

    fn run(affected: Option<Vec<usize>>) -> RangeAudit {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"aaa"}]},{"type":"paragraph","content":[{"type":"text","text":"bbb"}]},{"type":"paragraph","content":[{"type":"text","text":"ccc"}]},{"type":"paragraph","content":[{"type":"text","text":"ddd"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        hydrate_import_for_compile_test(&mut engine);
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_154,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 9,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
        assert_eq!(compiled.affected_top_level_blocks, [1, 2, 3]);
        match affected {
            Some(affected) => compiled.affected_top_level_blocks = affected,
            None => compiled.localized_semantic_used = false,
        }
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_atomic_failpoint_for_test(Some(AtomicFailpoint::DurableMetadataAdmission));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_atomic_failpoint_for_test(None);
        let error = applied.expect_err("durable metadata failpoint must abort the commit");
        assert_eq!(atomic_audit(&engine), before);
        RangeAudit {
            error,
            cached_counts: take_cached_render_counts_for_test(),
            lifecycle_counts: take_localized_render_transition_counts_for_test(),
            full_pass_counts: take_full_pass_counts_for_test(),
        }
    }

    let generic = run(None);
    assert_eq!(generic.lifecycle_counts, (0, 0, 0));
    for (case, affected) in [
        ("empty", vec![]),
        ("tooNarrow", vec![1, 2]),
        ("wrongStart", vec![0, 1, 2]),
        ("duplicate", vec![1, 2, 2]),
        ("outOfOrder", vec![1, 3, 2]),
        ("outOfRange", vec![1, 2, 4]),
    ] {
        let malformed = run(Some(affected));
        assert_eq!(malformed.error, generic.error, "{case}");
        assert_eq!(malformed.cached_counts, generic.cached_counts, "{case}");
        assert_eq!(
            malformed.full_pass_counts, generic.full_pass_counts,
            "{case}"
        );
        assert_eq!(malformed.lifecycle_counts, (1, 0, 1), "{case}");
    }
}

#[test]
fn every_localized_render_stage_failure_falls_back_with_exact_parity() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test,
        reset_localized_render_failure_checkpoint_counts_for_test,
        reset_localized_render_transition_counts_for_test,
        set_localized_render_failure_stage_for_test, take_cached_render_counts_for_test,
        take_localized_render_failure_checkpoint_counts_for_test,
        take_localized_render_transition_counts_for_test, LocalizedRenderFailureStage,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
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

    let mut generic = fixture();
    let mut generic_compiled = generic
        .compile_typed_transaction(transaction(&generic, 700_152))
        .unwrap();
    generic_compiled
        .prepared_derived_evidence
        .as_mut()
        .unwrap()
        .tamper_localized_render_for_test("missing");
    let generic_result = generic
        .apply_compiled_transaction(generic_compiled, true)
        .unwrap()
        .1
        .unwrap();

    for stage in [
        LocalizedRenderFailureStage::Allocation,
        LocalizedRenderFailureStage::Resource,
        LocalizedRenderFailureStage::Position,
        LocalizedRenderFailureStage::Invariant,
    ] {
        let mut engine = fixture();
        let compiled = engine
            .compile_typed_transaction(transaction(&engine, 700_152))
            .unwrap();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        reset_localized_render_failure_checkpoint_counts_for_test();
        set_localized_render_failure_stage_for_test(Some(stage));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_localized_render_failure_stage_for_test(None);
        let result = applied.unwrap().1.unwrap();

        assert_eq!(result, generic_result, "{stage:?}");
        assert_eq!(engine.document_json(), generic.document_json(), "{stage:?}");
        let state = engine.derived_state.as_ref().unwrap();
        let generic_state = generic.derived_state.as_ref().unwrap();
        assert_eq!(
            state.validation_certificate, generic_state.validation_certificate,
            "{stage:?}"
        );
        assert_eq!(
            state.localized_text_index, generic_state.localized_text_index,
            "{stage:?}"
        );
        assert_eq!(
            state.render_blocks.materialize(),
            generic_state.render_blocks.materialize(),
            "{stage:?}"
        );
        assert_eq!(engine.can_undo(), generic.can_undo(), "{stage:?}");
        assert_eq!(engine.can_redo(), generic.can_redo(), "{stage:?}");
        assert_eq!(
            engine.history.retained_units(0).unwrap(),
            generic.history.retained_units(0).unwrap(),
            "{stage:?}"
        );
        assert_eq!(
            engine.history.replay_audit_for_test(),
            generic.history.replay_audit_for_test(),
            "{stage:?}"
        );
        assert_eq!(
            take_cached_render_counts_for_test(),
            (0, 1, 1, 0, 0),
            "{stage:?}"
        );
        assert_eq!(
            take_localized_render_transition_counts_for_test(),
            (1, 0, 1),
            "{stage:?}"
        );
        let expected_checkpoints = match stage {
            LocalizedRenderFailureStage::Allocation => (1, 0, 0, 0),
            LocalizedRenderFailureStage::Resource => (1, 1, 0, 0),
            LocalizedRenderFailureStage::Position => (1, 1, 1, 0),
            LocalizedRenderFailureStage::Invariant => (1, 1, 1, 1),
        };
        assert_eq!(
            take_localized_render_failure_checkpoint_counts_for_test(),
            expected_checkpoints,
            "{stage:?}"
        );
    }
}

#[test]
fn localized_render_failure_exposes_only_the_generic_transition_error() {
    use crate::render::incremental::{
        reset_cached_render_counts_for_test, reset_localized_render_transition_counts_for_test,
        set_cached_render_error_for_test, set_localized_render_failure_stage_for_test,
        take_cached_render_counts_for_test, take_localized_render_transition_counts_for_test,
        CachedRenderError, LocalizedRenderFailureStage,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test, FullPassCounts,
    };

    struct FailureAudit {
        error: crate::yrs_engine::OperationError,
        cached_counts: (usize, usize, usize, usize, usize),
        lifecycle_counts: (usize, usize, usize),
        full_pass_counts: FullPassCounts,
    }

    fn run(stage: Option<LocalizedRenderFailureStage>) -> FailureAudit {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let mut compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id: 700_153,
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
            })
            .unwrap();
        compiled.localized_semantic_used = stage.is_some();
        let before = atomic_audit(&engine);
        reset_full_pass_counts_for_test();
        reset_cached_render_counts_for_test();
        reset_localized_render_transition_counts_for_test();
        set_localized_render_failure_stage_for_test(stage);
        set_cached_render_error_for_test(Some(CachedRenderError::AllocationFailed));
        let applied = engine.apply_compiled_transaction(compiled, true);
        set_localized_render_failure_stage_for_test(None);
        set_cached_render_error_for_test(None);
        let error = applied.expect_err("forced generic transition failure must be returned");
        assert_eq!(atomic_audit(&engine), before);
        FailureAudit {
            error,
            cached_counts: take_cached_render_counts_for_test(),
            lifecycle_counts: take_localized_render_transition_counts_for_test(),
            full_pass_counts: take_full_pass_counts_for_test(),
        }
    }

    let generic = run(None);
    assert_eq!(generic.error.code, "ENGINE_INVARIANT_FAILED");
    assert!(generic.error.message.contains("AllocationFailed"));
    assert_eq!(generic.cached_counts, (0, 1, 0, 0, 0));
    assert_eq!(generic.lifecycle_counts, (0, 0, 0));
    assert_eq!(generic.full_pass_counts, FullPassCounts::default());
    for stage in [
        LocalizedRenderFailureStage::Allocation,
        LocalizedRenderFailureStage::Resource,
        LocalizedRenderFailureStage::Position,
        LocalizedRenderFailureStage::Invariant,
    ] {
        let localized = run(Some(stage));
        assert_eq!(localized.error, generic.error, "{stage:?}");
        assert_eq!(localized.cached_counts, generic.cached_counts, "{stage:?}");
        assert_eq!(localized.lifecycle_counts, (1, 0, 1), "{stage:?}");
        assert_eq!(
            localized.full_pass_counts, generic.full_pass_counts,
            "{stage:?}"
        );
    }
}

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
