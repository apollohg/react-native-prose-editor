use super::*;

fn unaffected_text_sticky(
    engine: &YrsDocumentEngine,
    text_child: u32,
    utf16_index: u32,
) -> (crate::yrs_engine::RelativePoint, BranchPtr, u32) {
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
        panic!("expected paragraph")
    };
    let XmlOut::Text(text) = paragraph.get(&txn, text_child).unwrap() else {
        panic!("expected text child")
    };
    let branch = BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&text));
    let sticky = StickyIndex::at(&txn, branch, utf16_index, Assoc::After).unwrap();
    let point = crate::yrs_engine::RelativePoint {
        sticky,
        affinity: Affinity::After,
    };
    let Some(offset) = point.sticky.get_offset(&txn) else {
        panic!("sticky must resolve")
    };
    let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
        &txn,
        &fragment,
        &point,
        &engine.schema,
    )
    .unwrap();
    let scalar = engine
        .position_map()
        .unwrap()
        .doc_to_scalar(doc_pos, engine.document().unwrap());
    (point, offset.branch, scalar)
}

fn assert_unaffected_sticky(
    engine: &YrsDocumentEngine,
    point: &crate::yrs_engine::RelativePoint,
    branch: BranchPtr,
    expected_scalar: u32,
) {
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let offset = point.sticky.get_offset(&txn).unwrap();
    assert_eq!(
        offset.branch, branch,
        "unaffected Yrs branch identity changed"
    );
    let doc_pos = crate::yrs_engine::position::relative_point_to_doc_pos(
        &txn,
        &fragment,
        point,
        &engine.schema,
    )
    .unwrap();
    assert_eq!(
        engine
            .position_map()
            .unwrap()
            .doc_to_scalar(doc_pos, engine.document().unwrap()),
        expected_scalar,
        "unaffected sticky point moved to the wrong rendered position"
    );
}

#[test]
fn granular_command_lowering_preserves_classification_locality_and_unaffected_sticky_identity() {
    let mut format = transaction_engine();
    format
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"link","attrs":{"href":"old"}}]},{"type":"text","text":"bc tail"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let (format_sticky, format_branch, format_scalar) = unaffected_text_sticky(&format, 1, 5);
    select_text(&mut format, 100, 2, 0);
    let CommandPlan::Transaction(format_transaction) = format
        .plan_command(
            101,
            TypedCommand::SetMark {
                mark_type: "link".into(),
                attrs: HashMap::from([("href".into(), json!("new"))]),
            },
        )
        .unwrap()
    else {
        panic!("range format must plan")
    };
    assert!(matches!(
        format_transaction.operations.as_slice(),
        [
            TypedOperation::RemoveMark { .. },
            TypedOperation::AddMark { .. }
        ]
    ));
    let compiled = format
        .compile_typed_transaction(format_transaction.clone())
        .unwrap();
    assert_eq!(
        compiled.history_class,
        crate::yrs_engine::compiler::HistoryClass::Format
    );
    assert_eq!(
        compiled.position_update_mode,
        crate::position::update::UpdateMode::MarksOnly
    );
    assert_eq!(compiled.affected_top_level_blocks, vec![0]);
    let format_result = format
        .apply_typed_transaction_with_result(format_transaction)
        .unwrap();
    let crate::yrs_engine::RenderUpdate::Patch(format_patch) = format_result.render_update else {
        panic!("range format must produce a local render patch")
    };
    assert_eq!(
        (
            format_patch.start_index,
            format_patch.delete_count,
            format_patch.blocks.len(),
        ),
        (0, 1, 1)
    );
    assert_unaffected_sticky(&format, &format_sticky, format_branch, format_scalar);
    assert!(format.can_undo());

    let mut replace = transaction_engine();
    replace
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"left target right"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let (replace_sticky, replace_branch, replace_scalar) = unaffected_text_sticky(&replace, 0, 13);
    select_text(&mut replace, 102, 11, 5);
    let CommandPlan::Transaction(replace_transaction) = replace
        .plan_command(
            103,
            TypedCommand::ReplaceSelectionText { text: "new".into() },
        )
        .unwrap()
    else {
        panic!("range replacement must plan")
    };
    assert!(matches!(
        replace_transaction.operations.as_slice(),
        [
            TypedOperation::DeleteRange { .. },
            TypedOperation::InsertText { .. }
        ]
    ));
    let compiled = replace
        .compile_typed_transaction(replace_transaction.clone())
        .unwrap();
    assert_eq!(
        compiled.history_class,
        crate::yrs_engine::compiler::HistoryClass::Structural
    );
    assert_eq!(
        compiled.position_update_mode,
        crate::position::update::UpdateMode::InlineTextOnly
    );
    assert_eq!(compiled.affected_top_level_blocks, vec![0]);
    let replace_result = replace
        .apply_typed_transaction_with_result(replace_transaction)
        .unwrap();
    let crate::yrs_engine::RenderUpdate::Patch(replace_patch) = replace_result.render_update else {
        panic!("range replacement must produce a local render patch")
    };
    assert_eq!(
        (
            replace_patch.start_index,
            replace_patch.delete_count,
            replace_patch.blocks.len(),
        ),
        (0, 1, 1)
    );
    assert_unaffected_sticky(
        &replace,
        &replace_sticky,
        replace_branch,
        replace_scalar - 3,
    );
    assert!(replace.can_undo());
}

#[test]
fn typed_edits_advance_cached_render_blocks_while_selection_only_retains_arc() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let initial = Arc::clone(&engine.derived_state.as_ref().unwrap().render_blocks);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 104,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &initial,
        &engine.derived_state.as_ref().unwrap().render_blocks
    ));

    let old_blocks = initial.materialize();
    crate::render::incremental::reset_cached_render_counts_for_test();
    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 105,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (0, 1, 1, 0, 0)
    );
    let next = engine.derived_state.as_ref().unwrap();
    assert!(!Arc::ptr_eq(&initial, &next.render_blocks));

    let reconstructed = match result.render_update {
        crate::yrs_engine::RenderUpdate::None => old_blocks,
        crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
        crate::yrs_engine::RenderUpdate::Patch(patch) => {
            let mut blocks = old_blocks;
            blocks.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks,
            );
            blocks
        }
    };
    assert_eq!(reconstructed, next.render_blocks.materialize());
    assert_eq!(
        next.render_blocks.materialize(),
        crate::render::incremental::render_blocks(&next.document, &engine.schema)
    );
}

#[test]
fn history_results_compare_sealed_render_caches_without_full_old_new_render() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]},{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_edit_cache = Arc::clone(
        &engine
            .derived_state
            .as_ref()
            .expect("import initializes derived state")
            .render_blocks,
    );
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 106,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();
    let after_edit_cache = Arc::clone(
        &engine
            .derived_state
            .as_ref()
            .expect("edit initializes derived state")
            .render_blocks,
    );

    let before_undo = engine
        .derived_state
        .as_ref()
        .unwrap()
        .render_blocks
        .materialize();
    crate::render::incremental::reset_cached_render_counts_for_test();
    let undo = engine.undo_with_result(107).unwrap().unwrap();
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (0, 0, 0, 0, 0)
    );
    let after_undo = engine.derived_state.as_ref().unwrap();
    assert!(Arc::ptr_eq(&before_edit_cache, &after_undo.render_blocks));
    let reconstructed = apply_render_update_for_test(before_undo, undo.render_update);
    assert_eq!(reconstructed, after_undo.render_blocks.materialize());

    let before_redo = after_undo.render_blocks.materialize();
    crate::render::incremental::reset_cached_render_counts_for_test();
    let redo = engine.redo_with_result(108).unwrap().unwrap();
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (0, 0, 0, 0, 0)
    );
    let after_redo = engine.derived_state.as_ref().unwrap();
    assert!(Arc::ptr_eq(&after_edit_cache, &after_redo.render_blocks));
    assert_eq!(
        apply_render_update_for_test(before_redo, redo.render_update),
        after_redo.render_blocks.materialize()
    );
}

#[test]
fn history_snapshot_seed_publication_errors_propagate_real_request_atomically() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (request_id, failpoint, expected_stage) in [
        (
            108_056,
            LookupSeedHydrationFailpoint::BindingPublication,
            "historyStoreSnapshotPublication",
        ),
        (
            108_057,
            LookupSeedHydrationFailpoint::SeedPublication,
            "historyUnavailableSeedPublication",
        ),
    ] {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: request_id - 1,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        assert!(engine.can_undo());
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_ready_for_test());
        let before = atomic_audit(&engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);

        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = engine.undo_with_result(request_id);
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("history snapshot publication failure must propagate");
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(error.request_id, request_id);
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {expected_stage}")
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" }))
        );
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0);
        assert_eq!(counts.installed_base_seed_publications, 0);
        assert_eq!(atomic_audit(&engine), before);
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
        ));
    }
}

#[test]
fn history_snapshot_equality_uses_document_snapshot_arc_identity() {
    let engine = transaction_engine();
    let state = engine.derived_state.as_ref().unwrap();
    let retained = crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
        crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
            document: &state.document,
            canonical_artifact: &state.canonical_artifact,
            position_map: &state.position_map,
            rendered_text: &state.rendered_text,
            render_blocks: &state.render_blocks,
            schema_fingerprint: &engine.schema_fingerprint,
            fragment_name: &engine.fragment_name,
            scope: engine.scope.as_ref(),
        },
    )
    .unwrap();
    let document_snapshot = state.capture_history_document_snapshot(
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.fragment_name,
        engine.scope.as_ref(),
        retained,
    );
    let snapshot = crate::yrs_engine::history::HistorySnapshot {
        relative_selection: state.relative_selection.clone(),
        resolved_selection: state.resolved_selection.clone(),
        stored_marks: state.stored_marks.clone(),
        text_length: state.canonical_artifact.text_scalar_len(),
        canonical_fingerprint: state.canonical_artifact.sha256(),
        derived_output_bytes: state.canonical_artifact.serialized_len(),
        metadata_bytes: retained.get(),
        document_snapshot: Some(document_snapshot),
    };
    let shared = snapshot.clone();
    assert_eq!(snapshot, shared);

    let mut equivalent_but_distinct = snapshot.clone();
    let document_snapshot = snapshot
        .document_snapshot
        .as_ref()
        .expect("default article history retains its document snapshot");
    equivalent_but_distinct.document_snapshot = Some(Arc::new((**document_snapshot).clone()));
    assert_ne!(snapshot, equivalent_but_distinct);
}

#[test]
fn history_restoration_resolves_only_the_popped_selection_without_a_default_roundtrip() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_json = engine.document_json().unwrap();
    let before_selection = engine.resolved_selection().cloned().unwrap();
    let before_marks = engine.stored_marks().map(<[_]>::to_vec);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_001,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 2,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    let after_json = engine.document_json().unwrap();
    let after_selection = engine.resolved_selection().cloned().unwrap();
    let after_marks = engine.stored_marks().map(<[_]>::to_vec);

    for (request_id, undoing) in [(108_002, true), (108_003, false)] {
        crate::yrs_engine::derived_state::reset_relative_selection_traversal_counts_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();

        if undoing {
            engine.undo_with_result(request_id).unwrap().unwrap();
        } else {
            engine.redo_with_result(request_id).unwrap().unwrap();
        }

        let (expected_json, expected_selection, expected_marks) = if undoing {
            (&before_json, &before_selection, &before_marks)
        } else {
            (&after_json, &after_selection, &after_marks)
        };
        assert_eq!(engine.document_json().as_ref(), Some(expected_json));
        assert_eq!(engine.resolved_selection(), Some(expected_selection));
        assert_eq!(
            engine.stored_marks().map(<[_]>::to_vec).as_ref(),
            expected_marks.as_ref()
        );

        assert_eq!(
            crate::yrs_engine::derived_state::take_relative_selection_traversal_counts_for_test(),
            (1, 1),
            "history restoration should materialize only the exact popped selection"
        );
        let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        // The document-scoped history snapshot is admitted by exact
        // candidate JSON equality, so no canonical projection,
        // serialization, or hash pass is repeated during the pop.
        assert_eq!(full_passes.canonical_projections, 0);
        assert_eq!(full_passes.canonical_serializations, 0);
        assert_eq!(full_passes.canonical_hashes, 0);
    }
}

include!("history_and_commit/snapshot_metadata.rs");

include!("history_and_commit/snapshot_fallback.rs");

include!("history_and_commit/commit_atomicity.rs");

include!("history_and_commit/state_only.rs");
