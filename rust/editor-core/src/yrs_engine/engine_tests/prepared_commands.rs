use super::*;

#[test]
fn prepared_insert_compilation_uses_localized_semantics_after_planner_step() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };
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
    let point = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 700_137,
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

    engine.ensure_mutation_lookup_seed(700_138).unwrap();
    engine
        .derived_state
        .as_mut()
        .unwrap()
        .materialize_mutation_identity();
    reset_canonical_artifact_counts_for_test();
    let preparation = std::cell::RefCell::new(None);
    let plan = engine
        .plan_command_internal(
            700_138,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap();
    let CommandPlan::Transaction(transaction) = plan else {
        panic!("insert command must produce a transaction");
    };
    let proof = preparation.into_inner().unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 0));

    reset_full_pass_counts_for_test();
    let compiled = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap();
    assert!(compiled.localized_insert_admission.is_some());
    assert_eq!(
        take_full_pass_counts_for_test().ordinary_step_applications,
        0
    );
}

#[test]
fn stage4b2_prepared_same_leaf_insert_avoids_postwrite_relative_selection_traversals() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

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
            request_id: 700_153,
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

    reset_relative_selection_traversal_counts_for_test();
    reset_prewrite_selection_proof_counts_for_test();
    let result = engine
        .apply_command(700_154, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(
        result.selection,
        engine.resolved_selection().unwrap().clone()
    );
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (0, 0));
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 0, 1)
    );
}

#[test]
fn stage4b2_prepared_selection_tamper_fails_closed_to_generic_parity() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    fn fixture(snapshot: &crate::yrs_engine::DocumentSnapshot) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine.restore_snapshot(snapshot).unwrap();
        let point = RevisionedPosition {
            offset: 1,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_155,
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

    fn prepared_insert(engine: &YrsDocumentEngine, request_id: u64) -> CompiledTransaction {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap()
    }

    let mut baseline = transaction_engine();
    baseline
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = baseline.export_snapshot().unwrap();

    let mut tampered = fixture(&snapshot);
    reset_prewrite_selection_proof_counts_for_test();
    let mut compiled = prepared_insert(&tampered, 700_156);
    compiled.prepared_selection_state = Some(
        compiled
            .prepared_selection_state
            .as_ref()
            .unwrap()
            .tampered_for_test()
            .swap_remove(0),
    );
    reset_relative_selection_traversal_counts_for_test();
    let tampered_result = tampered
        .apply_compiled_transaction(compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 1, 0)
    );

    let mut generic = fixture(&snapshot);
    let mut generic_compiled = prepared_insert(&generic, 700_156);
    generic_compiled.prepared_selection_state = None;
    generic_compiled.prepared_selection_mutation_seal = None;
    reset_relative_selection_traversal_counts_for_test();
    let generic_result = generic
        .apply_compiled_transaction(generic_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    assert_eq!(tampered_result, generic_result);
    assert_eq!(tampered.document_json(), generic.document_json());
    assert_eq!(tampered.relative_selection(), generic.relative_selection());
    assert_eq!(tampered.resolved_selection(), generic.resolved_selection());
    assert_eq!(tampered.can_undo(), generic.can_undo());

    let mut optimized = fixture(&snapshot);
    let optimized_result = optimized
        .apply_command(700_156, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(optimized_result, generic_result);
    assert_eq!(optimized.document_json(), generic.document_json());
    assert_eq!(optimized.relative_selection(), generic.relative_selection());
    assert_eq!(optimized.resolved_selection(), generic.resolved_selection());
    assert_eq!(optimized.can_undo(), generic.can_undo());

    assert_eq!(
        tampered.undo(700_157).unwrap(),
        generic.undo(700_157).unwrap()
    );
    optimized.undo(700_157).unwrap();
    assert_eq!(tampered.document_json(), generic.document_json());
    assert_eq!(optimized.document_json(), generic.document_json());
    assert_eq!(
        tampered.redo(700_158).unwrap(),
        generic.redo(700_158).unwrap()
    );
    optimized.redo(700_158).unwrap();
    assert_eq!(tampered.document_json(), generic.document_json());
    assert_eq!(optimized.document_json(), generic.document_json());

    for tamper_index in 0..3 {
        let mut engine = fixture(&snapshot);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&engine, 700_160 + tamper_index as u64);
        compiled.prepared_selection_state = Some(
            compiled
                .prepared_selection_state
                .as_ref()
                .unwrap()
                .tampered_for_test()
                .swap_remove(tamper_index),
        );
        reset_relative_selection_traversal_counts_for_test();
        engine.apply_compiled_transaction(compiled, true).unwrap();
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 1, 0)
        );
        assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
    }

    let mut engine = fixture(&snapshot);
    let before = atomic_audit(&engine);
    reset_prewrite_selection_proof_counts_for_test();
    let mut compiled = prepared_insert(&engine, 700_163);
    compiled.prepared_selection_mutation_seal = None;
    reset_relative_selection_traversal_counts_for_test();
    let error = engine
        .apply_compiled_transaction(compiled, true)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 0, 0)
    );
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (0, 0));

    for case in [
        "actionIndex",
        "actionLength",
        "admissionResult",
        "origin",
        "history",
        "selectionPlan",
        "epoch",
        "revision",
    ] {
        let mut engine = fixture(&snapshot);
        let before = atomic_audit(&engine);
        reset_prewrite_selection_proof_counts_for_test();
        let mut compiled = prepared_insert(&engine, 700_164);
        match case {
            "actionIndex" => {
                let [YrsMutationAction::InsertText { index_utf16, .. }] =
                    compiled.mutation_plan.actions.as_mut_slice()
                else {
                    unreachable!()
                };
                *index_utf16 = index_utf16.saturating_add(1);
            }
            "actionLength" => {
                let [YrsMutationAction::InsertText { len_utf16, .. }] =
                    compiled.mutation_plan.actions.as_mut_slice()
                else {
                    unreachable!()
                };
                *len_utf16 = len_utf16.saturating_add(1);
            }
            "admissionResult" => {
                let admission = compiled.localized_insert_admission.as_ref().unwrap();
                compiled.localized_insert_admission = Some(
                    admission
                        .tampered_claims_for_test()
                        .into_iter()
                        .find(|(claim, _)| *claim == "operationResult")
                        .unwrap()
                        .1,
                );
            }
            "origin" => compiled.origin = TransactionOrigin::LocalInput,
            "history" => compiled.history_policy = HistoryPolicy::Auto,
            "selectionPlan" => {
                compiled.selection_plan = SelectionPlan::Explicit(Selection::cursor(1));
            }
            "epoch" => compiled.yrs_state_epoch = compiled.yrs_state_epoch.saturating_add(1),
            "revision" => {
                compiled.base_state_revision = compiled.base_state_revision.saturating_add(1);
            }
            _ => unreachable!(),
        }
        let authority = crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
            engine.derived_state.as_ref().unwrap(),
        );
        assert!(
            !compiled
                .prepared_selection_mutation_seal
                .as_ref()
                .unwrap()
                .matches(&compiled, &authority),
            "{case}"
        );
        reset_relative_selection_traversal_counts_for_test();
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 0),
            "{case}"
        );
        assert_eq!(
            take_relative_selection_traversal_counts_for_test(),
            (0, 0),
            "{case}"
        );
    }

    let mut engine = fixture(&snapshot);
    let before = atomic_audit(&engine);
    reset_prewrite_selection_proof_counts_for_test();
    let mut compiled = prepared_insert(&engine, 700_165);
    let original_target = match compiled.mutation_plan.actions.as_slice() {
        [YrsMutationAction::InsertText { target, .. }] => {
            <XmlTextRef as AsRef<Branch>>::as_ref(target).id()
        }
        _ => unreachable!(),
    };
    let foreign = utf16_doc();
    {
        let update = Update::decode_v1(&snapshot.encoded_state).unwrap();
        foreign.transact_mut().apply_update(update).unwrap();
    }
    let foreign_text = {
        let txn = foreign.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            unreachable!()
        };
        let XmlOut::Text(text) = paragraph.get(&txn, 0).unwrap() else {
            unreachable!()
        };
        text
    };
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&foreign_text).id(),
        original_target
    );
    let [YrsMutationAction::InsertText { target, .. }] =
        compiled.mutation_plan.actions.as_mut_slice()
    else {
        unreachable!()
    };
    *target = foreign_text;
    {
        let authority = crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(
            engine.derived_state.as_ref().unwrap(),
        );
        assert!(!compiled
            .prepared_selection_mutation_seal
            .as_ref()
            .unwrap()
            .matches(&compiled, &authority));
    }
    let error = engine
        .apply_compiled_transaction(compiled, true)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (1, 1, 0, 0)
    );
}

#[test]
fn stage4b2_direct_local_insert_does_not_enter_prewrite_selection_proof_lifecycle() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    reset_prewrite_selection_proof_counts_for_test();
    reset_relative_selection_traversal_counts_for_test();
    engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 700_159,
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
    assert_eq!(
        take_prewrite_selection_proof_counts_for_test(),
        (0, 0, 0, 0)
    );
    assert_eq!(take_relative_selection_traversal_counts_for_test(), (1, 1));
}

#[test]
fn stage4b2_prepared_failpoints_never_install_selection_proof() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
    };

    for failpoint in [
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ] {
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
                request_id: 700_166,
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
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                700_167,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            unreachable!()
        };
        reset_prewrite_selection_proof_counts_for_test();
        let compiled = engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap();
        let before = atomic_audit(&engine);
        set_atomic_failpoint_for_test(Some(failpoint));
        let error = engine
            .apply_compiled_transaction(compiled, true)
            .unwrap_err();
        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert_eq!(
            take_prewrite_selection_proof_counts_for_test(),
            (1, 1, 0, 0),
            "{failpoint:?}"
        );
    }
}

#[test]
fn stage4b2_optimized_selection_matches_generic_matrix() {
    fn fixture(snapshot: &crate::yrs_engine::DocumentSnapshot, offset: u32) -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine.restore_snapshot(snapshot).unwrap();
        let point = RevisionedPosition {
            offset,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 700_170,
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

    fn prepared_insert(
        engine: &YrsDocumentEngine,
        request_id: u64,
        text: &str,
    ) -> CompiledTransaction {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: text.into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must produce a transaction")
        };
        let proof = preparation.into_inner().unwrap();
        engine
            .compile_prepared_typed_transaction(transaction, proof)
            .unwrap()
    }

    let cases = [
        (
            "non-bmp",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            1,
            "🙂",
        ),
        (
            "marked-fragmented",
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"ab"},{"type":"text","marks":[{"type":"italic"}],"text":"cd"}]}]}"#,
            3,
            "x",
        ),
        (
            "nested",
            r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}]}"#,
            1,
            "x",
        ),
    ];

    for (index, (case, json, offset, inserted)) in cases.into_iter().enumerate() {
        let request_id = 700_171 + index as u64;
        let mut baseline = transaction_engine();
        baseline
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
        let snapshot = baseline.export_snapshot().unwrap();
        let mut optimized = fixture(&snapshot, offset);
        let optimized_result = optimized
            .apply_command(
                request_id,
                TypedCommand::InsertText {
                    text: inserted.into(),
                },
            )
            .unwrap()
            .unwrap();

        let mut generic = fixture(&snapshot, offset);
        let mut compiled = prepared_insert(&generic, request_id, inserted);
        assert!(compiled.prepared_selection_state.is_some(), "{case}");
        compiled.prepared_selection_state = None;
        compiled.prepared_selection_mutation_seal = None;
        let generic_result = generic
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap();

        assert_eq!(optimized_result, generic_result, "{case}");
        assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
        assert_eq!(
            optimized.relative_selection(),
            generic.relative_selection(),
            "{case}"
        );
        assert_eq!(
            optimized.resolved_selection(),
            generic.resolved_selection(),
            "{case}"
        );
        assert_eq!(
            optimized.derived_state.as_ref().unwrap().legacy_selection,
            generic.derived_state.as_ref().unwrap().legacy_selection,
            "{case}"
        );
        assert_eq!(optimized.can_undo(), generic.can_undo(), "{case}");
        assert_eq!(
            optimized.undo(700_180).unwrap(),
            generic.undo(700_180).unwrap(),
            "{case}"
        );
        assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
        assert_eq!(
            optimized.redo(700_181).unwrap(),
            generic.redo(700_181).unwrap(),
            "{case}"
        );
        assert_eq!(optimized.document_json(), generic.document_json(), "{case}");
    }
}

#[test]
fn stage4b2_wide_deep_selection_traversal_counts_are_constant() {
    use crate::yrs_engine::derived_state::{
        reset_prewrite_selection_proof_counts_for_test,
        reset_relative_selection_traversal_counts_for_test,
        take_prewrite_selection_proof_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };

    let mut observed = Vec::new();
    for factor in [1usize, 2] {
        let mut nested = json!({
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abc" }]
        });
        for _ in 0..(factor * 3) {
            nested = json!({ "type": "blockquote", "content": [nested] });
        }
        let mut content = vec![nested];
        content.extend((1..factor * 32).map(|index| {
            json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": format!("{index:04} abc") }]
            })
        }));
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({ "type": "doc", "content": content }).to_string(),
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
                request_id: 700_190 + factor as u64,
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
        reset_prewrite_selection_proof_counts_for_test();
        reset_relative_selection_traversal_counts_for_test();
        engine
            .apply_command(
                700_192 + factor as u64,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        observed.push((
            take_prewrite_selection_proof_counts_for_test(),
            take_relative_selection_traversal_counts_for_test(),
        ));
    }
    assert_eq!(observed[0], observed[1]);
    assert_eq!(observed[0], ((1, 1, 0, 1), (0, 0)));
}

#[test]
fn prepared_command_preserves_semantic_output_error_before_yrs_scan_admission() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            &json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "x".repeat(4_096) }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.editing_limits.max_derived_output_bytes = 1;
    engine.resource_limits.max_input_bytes = 128;
    let command = TypedCommand::InsertText { text: "y".into() };

    let probe = engine.plan_command(70_005, command.clone()).unwrap_err();
    let exact = usize::try_from(probe.actual.unwrap()).unwrap();
    engine.editing_limits.max_derived_output_bytes = exact;
    assert!(engine.plan_command(70_005, command.clone()).is_ok());
    let before = atomic_audit(&engine);
    let scan_error = engine.apply_command(70_005, command.clone()).unwrap_err();
    assert_eq!(
        scan_error.details,
        Some(json!({ "field": "maxInputBytes" })),
        "{scan_error:?}",
    );
    assert_eq!(atomic_audit(&engine), before);

    engine.editing_limits.max_derived_output_bytes = exact - 1;
    let planned_error = engine.plan_command(70_005, command.clone()).unwrap_err();
    assert_eq!(planned_error.operation_index, Some(0));
    assert_eq!(planned_error.actual, Some(exact as u64));
    assert_eq!(
        planned_error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );

    let applied_error = engine.apply_command(70_005, command).unwrap_err();

    assert_eq!(applied_error, planned_error);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_command_preserves_semantic_undo_error_before_yrs_scan_admission() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            &json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "x".repeat(4_096) }]
                }]
            })
            .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.editing_limits.max_undo_retained_units = 0;
    engine.resource_limits.max_input_bytes = 128;
    let command = TypedCommand::InsertText { text: "y".into() };

    let probe = engine.plan_command(70_006, command.clone()).unwrap_err();
    let exact = probe.actual.unwrap();
    engine.editing_limits.max_undo_retained_units = exact;
    assert!(engine.plan_command(70_006, command.clone()).is_ok());
    let before = atomic_audit(&engine);
    let scan_error = engine.apply_command(70_006, command.clone()).unwrap_err();
    assert_eq!(
        scan_error.details,
        Some(json!({ "field": "maxInputBytes" })),
        "{scan_error:?}",
    );
    assert_eq!(atomic_audit(&engine), before);

    engine.editing_limits.max_undo_retained_units = exact - 1;
    let planned_error = engine.plan_command(70_006, command.clone()).unwrap_err();
    assert_eq!(planned_error.operation_index, Some(0));
    assert_eq!(planned_error.actual, Some(exact));
    assert_eq!(
        planned_error.details,
        Some(json!({ "field": "maxUndoRetainedUnits" }))
    );

    let applied_error = engine.apply_command(70_006, command).unwrap_err();

    assert_eq!(applied_error, planned_error);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_insert_applies_collapsed_stored_marks_in_one_compilation() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .apply_command(
            70_010,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        engine.stored_marks().unwrap(),
        &[Mark::new("bold".into(), HashMap::new())]
    );
    reset_semantic_compilation_count_for_test();

    engine
        .apply_command(70_011, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(
        engine.document_json().unwrap(),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{
                    "type": "text",
                    "text": "x",
                    "marks": [{ "type": "bold" }]
                }]
            }]
        })
    );
    assert_eq!(engine.stored_marks(), None);
}

#[test]
fn delete_empty_block_compiles_once_with_exact_selection() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]},{"type":"paragraph"}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let scalar = engine
        .position_map()
        .unwrap()
        .doc_to_scalar(4, engine.document().unwrap());
    let point = RevisionedPosition {
        offset: scalar,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_020,
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
    reset_semantic_compilation_count_for_test();

    let result = engine
        .apply_command(70_021, TypedCommand::DeleteBackward)
        .unwrap()
        .unwrap();

    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(
        engine.document_json().unwrap(),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "a" }]
            }]
        })
    );
    let crate::yrs_engine::ResolvedSelection::Text { anchor, head } = result.selection else {
        panic!("structural fallback must preserve a text selection");
    };
    assert_eq!((anchor.scalar, head.scalar), (1, 1));
    assert!(result.history_state.can_undo);
}

#[test]
fn ambiguous_wrap_in_list_keeps_the_public_proof_path() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let engine = transaction_engine();
    reset_semantic_compilation_count_for_test();
    reset_full_pass_counts_for_test();

    let plan = engine
        .plan_command(
            70_030,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap();

    assert!(matches!(plan, CommandPlan::Transaction(_)));
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(take_full_pass_counts_for_test().planner_simulations, 1);
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["type"],
        "paragraph"
    );
}

include!("prepared_commands/format_commands.rs");

include!("prepared_commands/wrap_commands.rs");

include!("prepared_commands/wrap_limits.rs");
