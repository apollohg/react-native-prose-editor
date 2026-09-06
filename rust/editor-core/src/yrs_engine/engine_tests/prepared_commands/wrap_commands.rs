#[test]
fn prepared_toggle_and_wrap_commands_each_simulate_and_compile_once() {
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let mut toggle = transaction_engine();
    toggle
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut toggle, 70_031, 0, 2);
    hydrate_import_for_compile_test(&mut toggle);
    reset_semantic_compilation_count_for_test();
    reset_full_pass_counts_for_test();
    toggle
        .apply_command(
            70_032,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    let toggle_passes = take_full_pass_counts_for_test();
    assert_eq!(toggle_passes.planner_simulations, 1);
    assert_eq!(toggle_passes.document_validations, 1);
    assert_eq!(toggle_passes.canonical_mark_tree_scans, 1);
    assert_eq!(toggle_passes.canonical_projections, 1);
    assert_eq!(toggle_passes.canonical_serializations, 2);
    assert_eq!(toggle_passes.canonical_hashes, 1);
    assert_eq!(toggle_passes.position_map_clones, 0);
    assert_eq!(toggle_passes.position_map_compactions, 0);
    assert_eq!(toggle_passes.rendered_text_derivations, 1);

    let mut wrap = transaction_engine();
    hydrate_import_for_compile_test(&mut wrap);
    reset_semantic_compilation_count_for_test();
    reset_full_pass_counts_for_test();
    wrap.apply_command(
        70_033,
        TypedCommand::WrapInList {
            list_type: "bulletList".into(),
            item_type: "listItem".into(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    let wrap_passes = take_full_pass_counts_for_test();
    assert_eq!(wrap_passes.planner_simulations, 1);
    assert_eq!(wrap_passes.document_validations, 1);
    assert_eq!(wrap_passes.canonical_mark_tree_scans, 1);
    assert_eq!(wrap_passes.canonical_projections, 1);
    assert_eq!(wrap_passes.canonical_serializations, 2);
    assert_eq!(wrap_passes.canonical_hashes, 1);
    assert_eq!(wrap_passes.position_map_clones, 0);
    assert_eq!(wrap_passes.position_map_compactions, 0);
    assert_eq!(wrap_passes.rendered_text_derivations, 1);
    assert_eq!(
        wrap.document_json().unwrap()["content"][0]["type"],
        "bulletList"
    );
}

#[test]
fn prepared_wrap_at_a_block_boundary_matches_its_simulated_selection() {
    let document = json!({
        "type": "doc",
        "content": [
            {
                "type": "h1",
                "content": [{ "type": "text", "text": "x".repeat(42) }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "y".repeat(220) }]
            }
        ]
    });
    let populated = || {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut engine, 70_033_001, 44, 44);
        engine
    };

    let mut prepared = populated();
    crate::yrs_engine::compiler::reset_semantic_compilation_count_for_test();
    let prepared_result = prepared
        .apply_command(
            70_033_002,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap();
    assert_eq!(
        crate::yrs_engine::compiler::take_semantic_compilation_count_for_test(),
        1
    );

    let mut generic = populated();
    let CommandPlan::Transaction(transaction) = generic
        .plan_command(
            70_033_002,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
    else {
        panic!("public block-boundary wrap must produce a transaction")
    };
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction)
        .unwrap();

    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared_result.unwrap().selection, generic_result.selection);
}

#[test]
fn prepared_article_wrap_uses_only_the_localized_root_window() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let mut content = Vec::with_capacity(161);
    content.push(json!({
        "type": "h1",
        "content": [{ "type": "text", "text": "h".repeat(42) }]
    }));
    for index in 0..160 {
        content.push(json!({
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": format!("{index:04} {}", "x".repeat(215))
            }]
        }));
    }
    let document = json!({ "type": "doc", "content": content });
    let mut engine = transaction_engine();
    engine
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    select_text(&mut engine, 70_033_100, 44, 44);
    hydrate_import_for_compile_test(&mut engine);

    let before_document = engine.document_json().unwrap();
    let before_selection = engine.resolved_selection().unwrap().clone();
    let before_revision = engine.revision();
    let mut expected_document = before_document.clone();
    let root_content = expected_document["content"].as_array_mut().unwrap();
    let paragraph = root_content.remove(1);
    root_content.insert(
        1,
        json!({
            "type": "bulletList",
            "content": [{
                "type": "listItem",
                "content": [paragraph]
            }]
        }),
    );

    reset_root_window_lowering_counts_for_test();
    let result = engine
        .apply_command(
            70_033_101,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
    let observed_counts = take_root_window_lowering_counts_for_test();

    assert_eq!(result.request_id, 70_033_101);
    assert_eq!(result.origin, TransactionOrigin::LocalCommand);
    assert!(result.changed);
    assert_eq!(result.document_revision, before_revision + 1);
    assert_eq!(engine.document_json().unwrap(), expected_document);
    assert!(matches!(
        result.selection,
        ResolvedSelection::Text { ref anchor, ref head }
            if (anchor.scalar, head.scalar) == (46, 46)
    ));
    assert_eq!(engine.resolved_selection().unwrap(), &result.selection);
    assert!(result.history_state.can_undo);
    assert!(!result.history_state.can_redo);
    assert!(engine.can_undo());
    assert!(!engine.can_redo());

    reset_root_window_lowering_counts_for_test();
    engine.undo(70_033_102).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before_document);
    assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
    assert!(!engine.can_undo());
    assert!(engine.can_redo());

    let redo = engine.redo_with_result(70_033_103).unwrap().unwrap();
    assert_eq!(redo.request_id, 70_033_103);
    assert_eq!(redo.origin, TransactionOrigin::UndoRedo);
    assert!(redo.changed);
    assert_eq!(redo.document_revision, before_revision + 3);
    assert_eq!(engine.document_json().unwrap(), expected_document);
    assert!(matches!(
        redo.selection,
        ResolvedSelection::Text { ref anchor, ref head }
            if (anchor.scalar, head.scalar) == (46, 46)
    ));
    assert_eq!(engine.resolved_selection().unwrap(), &redo.selection);
    assert!(redo.history_state.can_undo);
    assert!(!redo.history_state.can_redo);
    assert!(engine.can_undo());
    assert!(!engine.can_redo());

    assert_eq!(observed_counts, (0, 0, 1, 0, 0, 1));
}

#[test]
fn prepared_wrap_proof_binds_the_exact_transaction_and_candidate_identity() {
    let compile = |engine: &YrsDocumentEngine, request_id| {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::WrapInList {
                    list_type: "bulletList".into(),
                    item_type: "listItem".into(),
                },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("wrap command must produce a transaction")
        };
        (transaction, preparation.into_inner().unwrap())
    };

    let engine = transaction_engine();
    let before = atomic_audit(&engine);
    let (mut transaction, proof) = compile(&engine, 70_034);
    assert!(matches!(
        transaction.operations.as_slice(),
        [TypedOperation::ReplaceStructure(_)]
    ));
    transaction.selection_intent = SelectionIntent::Preserve;
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);

    let (transaction, mut proof) = compile(&engine, 70_035);
    proof.document = engine.document().unwrap().clone();
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);

    let (transaction, mut proof) = compile(&engine, 70_035_000);
    let base_artifact = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    proof
        .eager_semantic_admission_mut_for_test()
        .replace_candidate_artifact_for_test(base_artifact);
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_proof_rejects_resource_limit_context_drift() {
    let mut engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_001,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    engine.resource_limits.max_schema_nodes -= 1;
    let before = atomic_audit(&engine);

    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_insert_without_candidate_certificate_runs_live_preview_validation() {
    use crate::yrs_engine::compiler::force_localized_semantic_allocation_failure_for_test;
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, take_full_pass_counts_for_test,
    };

    let engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_010,
            TypedCommand::InsertText { text: "x".into() },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared insert must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();

    reset_full_pass_counts_for_test();
    force_localized_semantic_allocation_failure_for_test(true);
    let compiled = engine.compile_prepared_typed_transaction(transaction, proof);
    force_localized_semantic_allocation_failure_for_test(false);

    compiled.unwrap();
    let counts = take_full_pass_counts_for_test();
    assert!(counts.document_validations >= 1);
    assert!(counts.canonical_mark_tree_scans >= 1);
}

#[test]
fn prepared_insert_rejects_stale_root_and_foreign_canonical_context_artifacts() {
    let compile = |engine: &YrsDocumentEngine, request_id| {
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id,
                TypedCommand::InsertText { text: "x".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("prepared insert must produce a transaction")
        };
        (transaction, preparation.into_inner().unwrap())
    };

    let engine = transaction_engine();
    let separate = transaction_engine();
    let before = atomic_audit(&engine);

    let (transaction, mut proof) = compile(&engine, 70_035_011);
    let stale_root_artifact = engine
        .canonical_schema
        .derive(separate.document().unwrap())
        .unwrap();
    proof
        .eager_semantic_admission_mut_for_test()
        .replace_canonical_artifact_for_test(stale_root_artifact);
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);

    let (transaction, mut proof) = compile(&engine, 70_035_012);
    let foreign_context_artifact = separate.canonical_schema.derive(&proof.document).unwrap();
    proof
        .eager_semantic_admission_mut_for_test()
        .replace_canonical_artifact_for_test(foreign_context_artifact);
    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_candidate_rejects_foreign_same_total_position_layout() {
    let engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_012_001,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();

    let mut foreign = transaction_engine();
    foreign
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let foreign_map =
        crate::position::PositionMap::build(foreign.document().unwrap(), &engine.schema);
    let expected_map = crate::position::PositionMap::build(&proof.document, &engine.schema);
    assert_eq!(foreign_map.total_scalars(), expected_map.total_scalars());
    assert_ne!(
        foreign_map.block(0).unwrap().node_path,
        expected_map.block(0).unwrap().node_path
    );
    let foreign_seed = crate::yrs_engine::compiler::PreparedCandidateSeed::mint(
        transaction.request_id,
        foreign.document().unwrap(),
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
    )
    .unwrap();

    let error = crate::yrs_engine::compiler::PreparedSemanticAdmission::prepare_single_operation(
        transaction.request_id,
        engine.revision,
        engine.state_revision,
        engine.yrs_state_epoch,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &transaction,
        &proof.document,
        Some(foreign_seed),
        None,
        0,
        crate::yrs_engine::compiler::PreparedCommandContractKind::None,
    )
    .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
}

#[test]
fn prepared_wrap_rejects_max_length_context_drift_atomically() {
    let mut engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_013,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    engine.max_length = Some(0);
    let before = atomic_audit(&engine);

    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_rejects_editing_limit_context_drift_atomically() {
    let mut engine = transaction_engine();
    let preparation = std::cell::RefCell::new(None);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command_internal(
            70_035_014,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
            Some(&preparation),
        )
        .unwrap()
    else {
        panic!("prepared wrap must produce a transaction")
    };
    let proof = preparation.into_inner().unwrap();
    engine.editing_limits.max_undo_groups -= 1;
    let before = atomic_audit(&engine);

    let error = engine
        .compile_prepared_typed_transaction(transaction, proof)
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn prepared_wrap_hard_limit_rejection_is_atomic() {
    use crate::yrs_engine::mutation::{
        reset_root_window_lowering_counts_for_test, take_root_window_lowering_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine.resource_limits.max_input_bytes = 0;
    let before = atomic_audit(&engine);

    reset_root_window_lowering_counts_for_test();
    let error = engine
        .apply_command(
            70_036,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap_err();
    let counts = take_root_window_lowering_counts_for_test();

    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    assert_eq!((counts.2, counts.3), (0, 0));
    assert_eq!(atomic_audit(&engine), before);
}
