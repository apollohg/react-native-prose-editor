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

#[test]
fn tight_history_metadata_budget_falls_back_to_full_candidate_derivation() {
    let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
        max_derived_output_bytes: 2 * (512 + "prosemirror".len() + 2),
        ..crate::yrs_engine::EditingLimits::default()
    });
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
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

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    engine.undo_with_result(108_005).unwrap().unwrap();

    assert_eq!(
        engine.document_json().unwrap(),
        serde_json::from_str::<serde_json::Value>(
            r#"{"type":"doc","content":[{"type":"paragraph"}]}"#,
        )
        .unwrap()
    );
    let full_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    assert!(full_passes.canonical_projections > 0);
    assert!(full_passes.canonical_serializations > 0);
    assert!(full_passes.canonical_hashes > 0);
}

#[test]
fn deep_wide_history_snapshot_budget_accounts_for_spilled_position_paths() {
    fn deep_wide_document() -> serde_json::Value {
        let mut content = (0..24)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": format!("row {index}")}]
                })
            })
            .collect::<Vec<_>>();
        for _ in 0..10 {
            content = vec![json!({"type": "blockquote", "content": content})];
        }
        json!({"type": "doc", "content": content})
    }

    fn insert(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
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
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    let document = deep_wide_document();
    let mut probe = transaction_engine();
    probe
        .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let compiled = probe
        .compile_typed_transaction(insert(&probe, 108_006))
        .unwrap();
    let after = compiled.preview_derivations.as_ref().unwrap();
    let before = probe.derived_state.as_ref().unwrap();
    let before_retained =
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &before.document,
                canonical_artifact: &before.canonical_artifact,
                position_map: &before.position_map,
                rendered_text: &before.rendered_text,
                render_blocks: &before.render_blocks,
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap();
    let after_retained =
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &compiled.preview,
                canonical_artifact: compiled.canonical_artifact.as_ref().unwrap(),
                position_map: &after.position_map,
                rendered_text: &after.rendered_text,
                render_blocks: &crate::render::incremental::CachedRenderBlocks::build(
                    &compiled.preview,
                    &probe.schema,
                    &probe.resource_limits,
                )
                .unwrap(),
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap();
    let exact_budget =
        super::history_metadata_bytes(before.stored_marks.as_deref(), &probe.fragment_name)
            .checked_add(super::history_metadata_bytes(None, &probe.fragment_name))
            .and_then(|bytes| bytes.checked_add(before_retained.get()))
            .and_then(|bytes| bytes.checked_add(after_retained.get()))
            .unwrap();

    let run = |limit, request_id| {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        engine.editing_limits.max_derived_output_bytes = limit;
        engine
            .apply_typed_transaction(insert(&engine, request_id))
            .unwrap();
        assert!(
            engine.can_undo(),
            "base history capture must remain admitted"
        );
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(request_id + 1).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };

    let exact_passes = run(exact_budget, 108_007);
    assert_eq!(
        exact_passes.canonical_projections, 0,
        "the exact retained bound should admit the optional snapshots"
    );

    let full_passes = run(exact_budget - 1, 108_009);
    assert!(
        full_passes.canonical_projections > 0,
        "one under the retained bound must omit only the optional snapshots"
    );
}

#[test]
fn history_snapshot_charge_tracks_spare_node_string_capacity() {
    const SPARE_CAPACITY: usize = 1024 * 1024;

    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        let mut node_type = String::with_capacity(SPARE_CAPACITY);
        node_type.push_str("hardBreak");
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::void(node_type, HashMap::new()),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    fn snapshot_charge(engine: &YrsDocumentEngine) -> usize {
        let state = engine.derived_state.as_ref().unwrap();
        crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
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
        .unwrap()
        .get()
    }

    let before_probe =
        fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    let before_charge = snapshot_charge(&before_probe);
    let before_metadata =
        super::history_metadata_bytes(before_probe.stored_marks(), &before_probe.fragment_name);
    let mut after_probe =
        fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    after_probe
        .apply_typed_transaction(transaction(&after_probe, 108_020))
        .unwrap();
    let after_charge = snapshot_charge(&after_probe);
    assert!(after_charge >= SPARE_CAPACITY);
    let exact = before_metadata
        .checked_add(super::history_metadata_bytes(
            after_probe.stored_marks(),
            &after_probe.fragment_name,
        ))
        .and_then(|bytes| bytes.checked_add(before_charge))
        .and_then(|bytes| bytes.checked_add(after_charge))
        .unwrap();

    for (limit, expect_fast, request_id) in [(exact, true, 108_021), (exact - 1, false, 108_023)] {
        let mut engine = fixture(limit);
        engine
            .apply_typed_transaction(transaction(&engine, request_id))
            .unwrap();
        assert!(engine.can_undo());
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(request_id + 1).unwrap().unwrap();
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(passes.canonical_projections == 0, expect_fast);
    }
}

#[test]
fn stored_mark_metadata_accounts_spare_hash_capacity_at_exact_boundary() {
    const SPARE_ENTRIES: usize = 32 * 1024;

    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 108_030, 1, 1);
        let mut attrs = HashMap::with_capacity(SPARE_ENTRIES);
        attrs.insert("href".into(), json!("x"));
        engine
            .apply_command(
                108_031,
                TypedCommand::SetMark {
                    mark_type: "link".into(),
                    attrs,
                },
            )
            .unwrap()
            .unwrap();
        assert!(engine.stored_marks().is_some());
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
                marks: engine.stored_marks().unwrap().to_vec(),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        }
    }

    let mut probe = fixture(crate::yrs_engine::EditingLimits::default().max_derived_output_bytes);
    let before_metadata = super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name);
    probe
        .apply_typed_transaction(transaction(&probe, 108_032))
        .unwrap();
    let exact = before_metadata
        .checked_add(super::history_metadata_bytes(
            probe.stored_marks(),
            &probe.fragment_name,
        ))
        .unwrap();

    let mut accepted = fixture(exact);
    accepted
        .apply_typed_transaction(transaction(&accepted, 108_033))
        .unwrap();
    assert!(accepted.can_undo());

    let mut rejected = fixture(exact - 1);
    let before = atomic_audit(&rejected);
    let error = rejected
        .apply_typed_transaction(transaction(&rejected, 108_034))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(atomic_audit(&rejected), before);
}

#[test]
fn compatible_auto_capture_admits_exact_after_only_metadata_increment() {
    fn fixture(limit: usize) -> YrsDocumentEngine {
        let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
            max_derived_output_bytes: limit,
            ..crate::yrs_engine::EditingLimits::default()
        });
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn insert(engine: &YrsDocumentEngine, request_id: u64, offset: u32) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        }
    }

    let default_limit = crate::yrs_engine::EditingLimits::default().max_derived_output_bytes;
    let mut probe = fixture(default_limit);
    probe
        .apply_typed_transaction(insert(&probe, 108_040, 1))
        .unwrap();
    let retained_before_second = probe.history.replay_metadata_bytes_for_test();
    let second_before_metadata = {
        let state = probe.derived_state.as_ref().unwrap();
        let retained = crate::yrs_engine::derived_state::history_document_snapshot_retained_bytes(
            crate::yrs_engine::derived_state::HistoryDocumentSnapshotRetainedInput {
                document: &state.document,
                canonical_artifact: &state.canonical_artifact,
                position_map: &state.position_map,
                rendered_text: &state.rendered_text,
                render_blocks: &state.render_blocks,
                schema_fingerprint: &probe.schema_fingerprint,
                fragment_name: &probe.fragment_name,
                scope: probe.scope.as_ref(),
            },
        )
        .unwrap()
        .get();
        super::history_metadata_bytes(probe.stored_marks(), &probe.fragment_name)
            .checked_add(retained)
            .unwrap()
    };
    probe
        .apply_typed_transaction(insert(&probe, 108_041, 2))
        .unwrap();
    let second_after_metadata = probe
        .history
        .replay_metadata_bytes_for_test()
        .checked_sub(retained_before_second)
        .unwrap();
    let exact = retained_before_second
        .checked_add(second_after_metadata)
        .unwrap();
    assert!(
        exact
            < retained_before_second
                .checked_add(second_before_metadata)
                .and_then(|bytes| bytes.checked_add(second_after_metadata))
                .unwrap()
    );

    let mut engine = fixture(exact);
    engine
        .apply_typed_transaction(insert(&engine, 108_042, 1))
        .unwrap();
    engine
        .apply_typed_transaction(insert(&engine, 108_043, 2))
        .unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "axxb");
    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    engine.undo_with_result(108_044).unwrap().unwrap();
    assert_eq!(engine.document().unwrap().root().text_content(), "ab");
    assert!(!engine.can_undo(), "compatible edits must remain one group");
    assert_eq!(
        crate::yrs_engine::observability::take_full_pass_counts_for_test().canonical_projections,
        0,
        "the exact boundary keeps optional document snapshots enabled"
    );
}

#[test]
fn history_snapshot_and_forced_fallback_match_affinity_and_stored_marks() {
    use crate::yrs_engine::derived_state::force_history_document_snapshot_fallback_for_test;

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                        {"type": "horizontalRule"},
                        {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                    ]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let boundary = |affinity| RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity,
        };
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_052,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: vec![],
                selection_intent: SelectionIntent::Set(SelectionInput::Text {
                    anchor: boundary(Affinity::Before),
                    head: boundary(Affinity::After),
                }),
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        engine
            .apply_command(
                108_051,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .unwrap();
        assert!(engine.stored_marks().is_some());
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_053,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: engine.stored_marks().unwrap().to_vec(),
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
        engine
    }

    fn local_state(
        engine: &YrsDocumentEngine,
    ) -> (
        serde_json::Value,
        Option<ResolvedSelection>,
        Option<Vec<crate::model::Mark>>,
        bool,
        bool,
    ) {
        (
            engine.document_json().unwrap(),
            engine.resolved_selection().cloned(),
            engine.stored_marks().map(<[_]>::to_vec),
            engine.can_undo(),
            engine.can_redo(),
        )
    }

    fn text_affinities(engine: &YrsDocumentEngine) -> (Affinity, Affinity) {
        let Some(crate::yrs_engine::RelativeSelection::Text { anchor, head }) =
            engine.relative_selection()
        else {
            panic!("history restores the captured text selection");
        };
        (anchor.affinity, head.affinity)
    }

    let mut fast = fixture();
    let mut fallback = fixture();

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    fast.undo_with_result(108_054).unwrap().unwrap();
    let fast_undo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    let fallback_undo_passes = {
        let _fallback = force_history_document_snapshot_fallback_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fallback.undo_with_result(108_054).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };
    assert_eq!(local_state(&fast), local_state(&fallback));
    assert_eq!(text_affinities(&fast), text_affinities(&fallback));
    assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
    assert_eq!(fast_undo_passes.canonical_projections, 0);
    assert!(fallback_undo_passes.canonical_projections > 0);

    crate::yrs_engine::observability::reset_full_pass_counts_for_test();
    fast.redo_with_result(108_055).unwrap().unwrap();
    let fast_redo_passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
    let fallback_redo_passes = {
        let _fallback = force_history_document_snapshot_fallback_for_test();
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        fallback.redo_with_result(108_055).unwrap().unwrap();
        crate::yrs_engine::observability::take_full_pass_counts_for_test()
    };
    assert_eq!(local_state(&fast), local_state(&fallback));
    assert_eq!(text_affinities(&fast), text_affinities(&fallback));
    assert_eq!(text_affinities(&fast), (Affinity::Before, Affinity::After));
    assert_eq!(fast_redo_passes.canonical_projections, 0);
    assert!(fallback_redo_passes.canonical_projections > 0);
}

#[test]
fn history_snapshot_context_drift_falls_back_without_changing_undo_result() {
    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_060,
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
        engine
    }

    for context in ["resource", "editing", "maxLength", "scope"] {
        let mut engine = fixture();
        match context {
            "resource" => {
                engine.resource_limits.max_document_depth =
                    engine.resource_limits.max_document_depth.saturating_add(1)
            }
            "editing" => {
                engine.editing_limits.max_operations_per_transaction = engine
                    .editing_limits
                    .max_operations_per_transaction
                    .saturating_add(1)
            }
            "maxLength" => engine.max_length = Some(100),
            "scope" => engine
                .scope
                .as_mut()
                .expect("fixture is document scoped")
                .lineage_id
                .push_str("-changed"),
            _ => unreachable!(),
        }

        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        engine.undo_with_result(108_061).unwrap().unwrap();
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(engine.document().unwrap().root().text_content(), "ab");
        assert!(
            passes.canonical_projections > 0,
            "{context} drift must reject snapshot reuse and run the fallback"
        );
    }
}

#[test]
fn invalid_history_stored_marks_precede_snapshot_publication_and_preserve_atomicity() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_070,
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
    engine
        .history
        .replace_next_undo_stored_marks_for_test(vec![Mark::new("unknown".into(), HashMap::new())]);
    let before = atomic_audit(&engine);

    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::BindingPublication,
    ));
    let result = engine.undo_with_result(108_071);
    set_lookup_seed_hydration_failpoint_for_test(None);

    let error = result.expect_err("invalid history metadata must precede snapshot publication");
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.request_id, 108_071);
    assert_eq!(
        error.message.as_ref(),
        "history metadata contains invalid stored marks: unknown mark 'unknown'"
    );
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn every_history_snapshot_semantic_fallback_precedes_seed_publication() {
    use crate::yrs_engine::derived_state::{
        force_history_document_snapshot_fallback_for_test,
        force_history_snapshot_semantic_fallback_for_test, HistorySnapshotSemanticFallbackForTest,
    };
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: 108_072,
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
        engine
    }

    for stage in [
        HistorySnapshotSemanticFallbackForTest::RenderIdentity,
        HistorySnapshotSemanticFallbackForTest::RelativeSelection,
        HistorySnapshotSemanticFallbackForTest::ResolvedSelection,
        HistorySnapshotSemanticFallbackForTest::ResolvedMismatch,
    ] {
        for failpoint in [
            LookupSeedHydrationFailpoint::BindingPublication,
            LookupSeedHydrationFailpoint::SeedPublication,
        ] {
            let mut expected = fixture();
            let expected_result = {
                let _fallback = force_history_document_snapshot_fallback_for_test();
                expected.undo_with_result(108_073).unwrap().unwrap()
            };
            let mut actual = fixture();
            let actual_result = {
                let _fallback = force_history_snapshot_semantic_fallback_for_test(stage);
                set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
                let result = actual.undo_with_result(108_073);
                set_lookup_seed_hydration_failpoint_for_test(None);
                result.unwrap().unwrap()
            };

            assert_eq!(actual_result, expected_result, "{stage:?}/{failpoint:?}");
            assert_eq!(
                actual.document_json(),
                expected.document_json(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.resolved_selection(),
                expected.resolved_selection(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.stored_marks(),
                expected.stored_marks(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.can_undo(),
                expected.can_undo(),
                "{stage:?}/{failpoint:?}"
            );
            assert_eq!(
                actual.can_redo(),
                expected.can_redo(),
                "{stage:?}/{failpoint:?}"
            );
        }
    }
}

#[test]
fn history_restore_request_relabeling_precedes_forced_semantic_fallback_and_probes() {
    use crate::yrs_engine::derived_state::{
        force_history_snapshot_semantic_fallback_for_test,
        history_document_snapshot_retained_bytes, DerivedStateCache,
        HistoryDocumentSnapshotRetainedInput, HistorySnapshotSemanticFallbackForTest,
    };
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let engine = transaction_engine();
    let state = engine.derived_state.as_ref().unwrap();
    let retained = history_document_snapshot_retained_bytes(HistoryDocumentSnapshotRetainedInput {
        document: &state.document,
        canonical_artifact: &state.canonical_artifact,
        position_map: &state.position_map,
        rendered_text: &state.rendered_text,
        render_blocks: &state.render_blocks,
        schema_fingerprint: &state.schema_fingerprint,
        fragment_name: &engine.fragment_name,
        scope: engine.scope.as_ref(),
    })
    .unwrap();
    let snapshot = state.capture_history_document_snapshot(
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.fragment_name,
        engine.scope.as_ref(),
        retained,
    );
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();

    for failpoint in [
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let (_, admission) = snapshot
            .prepare_candidate_read(
                108_074,
                &txn,
                &fragment,
                &engine.schema,
                &engine.resource_limits,
                &engine.editing_limits,
                engine.max_length,
                &engine.schema_fingerprint,
                &engine.fragment_name,
                engine.scope.as_ref(),
                engine.yrs_state_epoch,
                engine.revision,
            )
            .unwrap()
            .into_parts();
        let _fallback = force_history_snapshot_semantic_fallback_for_test(
            HistorySnapshotSemanticFallbackForTest::RenderIdentity,
        );
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let result = DerivedStateCache::restore_history_document_snapshot(
            108_075,
            &snapshot,
            admission.expect("matching read admits the retained snapshot"),
            &txn,
            &fragment,
            &engine.schema,
            &state.relative_selection,
            &state.resolved_selection,
            state.stored_marks.clone(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.revision,
            engine.state_revision,
            engine.yrs_state_epoch,
        );
        set_lookup_seed_hydration_failpoint_for_test(None);

        let error = result.expect_err("request relabeling must precede semantic fallback");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(error.request_id, 108_075, "{failpoint:?}");
    }
}

#[test]
fn history_specific_initialization_keeps_candidate_limit_rejection_atomic() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.max_length = Some(2);
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 108_004,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: RevisionedPosition {
                        offset: 2,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    to: RevisionedPosition {
                        offset: 3,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                },
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();
    let before = atomic_audit(&engine);

    let error = engine.undo_with_result(108_005).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn second_history_pop_max_length_drift_rejects_before_live_pop() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.max_length = Some(1);
    for (request_id, from, to) in [(108_006, 1, 2), (108_007, 0, 1)] {
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: from,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: to,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                    },
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Boundary,
            })
            .unwrap();
    }

    engine
        .undo(108_008)
        .unwrap()
        .expect("first pop must restore the one-character document");
    assert_eq!(engine.document().unwrap().root().text_content(), "a");
    let before = atomic_audit(&engine);

    let error = engine.undo(108_009).unwrap_err();

    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(1));
    assert_eq!(error.actual, Some(2));
    assert_eq!(error.details, Some(json!({ "field": "maxLength" })));
    assert_eq!(atomic_audit(&engine), before);
    let repeated = engine.undo(108_010).unwrap_err();
    assert_eq!(repeated.code, error.code);
    assert_eq!(repeated.limit, error.limit);
    assert_eq!(repeated.actual, error.actual);
    assert_eq!(repeated.details, error.details);
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn cached_render_preparation_failure_is_atomic_before_durable_write() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before = atomic_audit(&engine);
    crate::render::incremental::set_cached_render_error_for_test(Some(
        crate::render::incremental::CachedRenderError::AllocationFailed,
    ));
    let error = engine
        .apply_typed_transaction(insert_transaction(&engine, 109))
        .unwrap_err();
    crate::render::incremental::set_cached_render_error_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
}

fn apply_render_update_for_test(
    mut old_blocks: Vec<Vec<crate::render::RenderElement>>,
    update: crate::yrs_engine::RenderUpdate,
) -> Vec<Vec<crate::render::RenderElement>> {
    match update {
        crate::yrs_engine::RenderUpdate::None => old_blocks,
        crate::yrs_engine::RenderUpdate::Full(blocks) => blocks,
        crate::yrs_engine::RenderUpdate::Patch(patch) => {
            old_blocks.splice(
                patch.start_index..patch.start_index + patch.delete_count,
                patch.blocks,
            );
            old_blocks
        }
    }
}

#[test]
fn direct_command_admission_error_is_not_replanned_as_structure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"target"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 104, 6, 0);
    engine.resource_limits.max_input_bytes = 0;
    let before = atomic_audit(&engine);

    let error = engine
        .plan_command(105, TypedCommand::ReplaceSelectionText { text: "x".into() })
        .unwrap_err();

    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(error.details, Some(json!({ "field": "maxInputBytes" })));
    assert_eq!(atomic_audit(&engine), before);
}

#[test]
fn every_recoverable_atomic_stage_failpoint_is_pre_open_and_read_only() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for failpoint in failpoints {
        let mut engine = transaction_engine();
        let transaction = insert_transaction(&engine, 76);
        let before = atomic_audit(&engine);
        let canonical_before = engine
            .derived_state
            .as_ref()
            .unwrap()
            .canonical_artifact
            .clone();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        set_atomic_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(
            error.details,
            Some(json!({ "failpoint": failpoint.field_name() })),
            "{failpoint:?}"
        );
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
        assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
    }
}

#[test]
fn compiled_history_failure_does_not_publish_candidate_active_state_lifecycle() {
    use crate::yrs_engine::derived_state::{
        reset_active_state_cache_counts_for_test, take_active_state_cache_counts_for_test,
    };

    for pending_install in [true, false] {
        let request_id = if pending_install { 760_010 } else { 760_020 };
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
                request_id,
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
            .apply_command(
                request_id + 1,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        let live_certificate = engine
            .derived_state
            .as_ref()
            .unwrap()
            .active_state_cache_for_test()
            .expect("fixture must retain a live active-state certificate");
        let preparation = std::cell::RefCell::new(None);
        let CommandPlan::Transaction(transaction) = engine
            .plan_command_internal(
                request_id + 2,
                TypedCommand::InsertText { text: "y".into() },
                Some(&preparation),
            )
            .unwrap()
        else {
            panic!("insert command must prepare a transaction")
        };
        let mut compiled = engine
            .compile_prepared_typed_transaction(transaction, preparation.into_inner().unwrap())
            .unwrap();
        assert!(compiled.prepared_active_state_transition.is_some());
        if !pending_install {
            compiled.prepared_active_state_transition = None;
        }
        let before = atomic_audit(&engine);
        reset_active_state_cache_counts_for_test();
        set_compiled_commit_stage_failpoint_for_test(Some(
            CompiledCommitPreparationStage::HistorySnapshotConstruction,
        ));

        let error = engine
            .apply_compiled_transaction(compiled, true)
            .expect_err("late snapshot construction must reject the prepared candidate");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{pending_install}");
        assert!(
            error.message.contains("historySnapshotConstruction"),
            "{pending_install}"
        );
        let counts = take_active_state_cache_counts_for_test();
        assert_eq!(counts.5, 0, "pending install={pending_install}");
        assert_eq!(counts.6, 0, "pending install={pending_install}");
        assert_eq!(atomic_audit(&engine), before, "{pending_install}");
        assert!(Arc::ptr_eq(
            &live_certificate,
            &engine
                .derived_state
                .as_ref()
                .unwrap()
                .active_state_cache_for_test()
                .unwrap(),
        ));
    }
}

#[test]
fn compiled_recorded_history_admission_preserves_live_replay_allocation_on_later_failure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.history.compact_replay_event_capacity_for_test();
    let before = atomic_audit(&engine);
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let mut transaction = insert_transaction(&engine, 760_030);
    transaction.history_policy = HistoryPolicy::Auto;
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_typed_transaction(transaction)
        .expect_err("candidate update encoding must fail after recorded admission");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
}

#[test]
fn compiled_excluded_history_admission_preserves_live_replay_allocation_on_later_failure() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine.history.compact_replay_event_capacity_for_test();
    let before = atomic_audit(&engine);
    let ledger_before = engine.history.replay_ledger_allocation_audit_for_test();
    let transaction = insert_transaction(&engine, 760_040);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_typed_transaction(transaction)
        .expect_err("candidate update encoding must fail after excluded admission");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        ledger_before
    );
}

#[test]
fn compiled_history_admission_error_precedes_candidate_preparation_failure() {
    use crate::yrs_engine::history::set_replay_update_allocation_failure_for_test;

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 760_020);
    transaction.history_policy = HistoryPolicy::Auto;
    let compiled = engine.compile_typed_transaction(transaction).unwrap();
    let before = atomic_audit(&engine);
    set_replay_update_allocation_failure_for_test(true);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));

    let error = engine
        .apply_compiled_transaction(compiled, true)
        .expect_err("history admission must win error precedence");

    set_replay_update_allocation_failure_for_test(false);
    set_compiled_commit_stage_failpoint_for_test(None);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(error.details, Some(json!({ "field": "historyReplay" })));
    assert_eq!(atomic_audit(&engine), before);

    let mut lookup_first = transaction_engine();
    lookup_first
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    lookup_first.ensure_mutation_lookup_seed(760_021).unwrap();
    let mut transaction = insert_transaction(&lookup_first, 760_022);
    transaction.history_policy = HistoryPolicy::Auto;
    let compiled = lookup_first.compile_typed_transaction(transaction).unwrap();
    let before = atomic_audit(&lookup_first);
    set_replay_update_allocation_failure_for_test(true);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::LookupTransition,
    ));

    let error = lookup_first
        .apply_compiled_transaction(compiled, true)
        .expect_err("baseline lookup failure must retain precedence over history admission");

    set_replay_update_allocation_failure_for_test(false);
    set_compiled_commit_stage_failpoint_for_test(None);
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert!(error.message.contains("lookupTransition"));
    assert_eq!(atomic_audit(&lookup_first), before);
}

#[test]
fn compiled_first_structural_mutation_supports_an_empty_configured_root() {
    let schema = crate::schema::Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block*", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "empty-root".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "empty-root-doc".into(),
            lineage_id: "empty-root-lineage".into(),
        }),
    })
    .unwrap();
    let initial_json = engine.document_json().unwrap();
    let initial_encoded = engine.encoded_state().unwrap();
    let initial_revision = engine.revision();
    let initial_state_revision = engine.state_revision();
    let initial_selection = engine.resolved_selection().cloned();
    let initial_history = engine.history.replay_audit_for_test();
    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 760_030,
            base_document_revision: initial_revision,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertNode {
                at: RevisionedPosition {
                    offset: 0,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                node: crate::model::Node::element(
                    "paragraph".into(),
                    HashMap::new(),
                    crate::model::Fragment::empty(),
                ),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap();

    let changed_json = engine.document_json().unwrap();
    assert_ne!(engine.encoded_state().unwrap(), initial_encoded);
    assert_eq!(changed_json["type"], "doc");
    assert_eq!(changed_json["content"][0]["type"], "paragraph");
    assert_eq!(engine.revision(), initial_revision + 1);
    assert_eq!(engine.state_revision(), initial_state_revision + 1);
    assert_eq!(result.document_revision, engine.revision());
    assert_eq!(result.state_revision, engine.state_revision());
    assert_eq!(engine.resolved_selection(), Some(&result.selection));
    assert_eq!(result.history_state.can_undo, engine.can_undo());
    assert_eq!(result.history_state.can_redo, engine.can_redo());
    assert!(engine.can_undo());
    assert_ne!(engine.history.replay_audit_for_test(), initial_history);

    let undo = engine
        .undo(760_031)
        .unwrap()
        .expect("insert must be undoable");
    assert!(undo.changed);
    assert_eq!(engine.document_json().unwrap(), initial_json);
    assert_eq!(engine.resolved_selection(), initial_selection.as_ref());
    assert!(!engine.can_undo());
    assert!(engine.can_redo());
}

#[test]
fn compiled_excluded_rebase_rolls_baseline_and_appends_the_event() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before_encoded = engine.encoded_state().unwrap();
    engine.history.force_rebase_before_next_event_for_test();
    let transaction = insert_transaction(&engine, 760_040);

    engine.apply_typed_transaction(transaction).unwrap();

    let (rebase, baseline, event_count, last_is_excluded) =
        engine.history.compiled_excluded_rebase_audit_for_test();
    assert!(!rebase);
    assert_eq!(baseline, before_encoded);
    assert_eq!(event_count, 1);
    assert!(last_is_excluded);
}

#[test]
fn compiled_commit_guard_rejects_every_preparation_stage_after_durable_open() {
    let stages = [
        CompiledCommitPreparationStage::AllocationProbe,
        CompiledCommitPreparationStage::OperationPreparation,
        CompiledCommitPreparationStage::DocumentValidation,
        CompiledCommitPreparationStage::LookupTransition,
        CompiledCommitPreparationStage::HistoryReservation,
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
        CompiledCommitPreparationStage::SelectionFinalization,
        CompiledCommitPreparationStage::DerivedStateBuild,
        CompiledCommitPreparationStage::HistorySnapshotConstruction,
    ];
    for stage in stages {
        set_compiled_commit_stage_failpoint_for_test(None);
        mark_compiled_commit_durable_write_for_test();
        let error = check_compiled_commit_preparation_stage_for_test(760_050, stage)
            .expect_err("every guarded preparation stage must reject after durable open");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
        assert!(error.message.contains("postwrite"), "{stage:?}");
    }
    set_compiled_commit_stage_failpoint_for_test(None);
}

#[test]
fn compiled_commit_prepares_all_recoverable_work_before_durable_write() {
    let stages = [
        CompiledCommitPreparationStage::AllocationProbe,
        CompiledCommitPreparationStage::OperationPreparation,
        CompiledCommitPreparationStage::DocumentValidation,
        CompiledCommitPreparationStage::LookupTransition,
        CompiledCommitPreparationStage::HistoryReservation,
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
        CompiledCommitPreparationStage::SelectionFinalization,
        CompiledCommitPreparationStage::DerivedStateBuild,
        CompiledCommitPreparationStage::HistorySnapshotConstruction,
    ];
    for (index, stage) in stages.into_iter().enumerate() {
        let mut engine = transaction_engine();
        let request_id = 760_100 + index as u64;
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine.ensure_mutation_lookup_seed(request_id).unwrap();
        let mut transaction = insert_transaction(&engine, request_id);
        transaction.history_policy = HistoryPolicy::Auto;
        let before = atomic_audit(&engine);
        let seed_before = engine
            .derived_state
            .as_ref()
            .expect("ready fixture has derived state")
            .mutation_lookup_seed
            .clone();
        set_compiled_commit_stage_failpoint_for_test(Some(stage));

        let error = engine
            .apply_typed_transaction(transaction)
            .expect_err("every recoverable compiled-commit stage must be injectable");

        set_compiled_commit_stage_failpoint_for_test(None);
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{stage:?}");
        assert_eq!(atomic_audit(&engine), before, "{stage:?}");
        assert!(Arc::ptr_eq(
            &seed_before,
            &engine
                .derived_state
                .as_ref()
                .expect("failed commit retains derived state")
                .mutation_lookup_seed,
        ));
    }
}

#[test]
fn localized_seed_promotion_is_not_installed_before_any_recoverable_failpoint() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let failpoints = [
        AtomicFailpoint::EnvelopeAdmission,
        AtomicFailpoint::SemanticCompilation,
        AtomicFailpoint::MutationPreflight,
        AtomicFailpoint::FinalPreflight,
        AtomicFailpoint::EncodedAdmission,
        AtomicFailpoint::CanonicalOutputAdmission,
        AtomicFailpoint::RevisionAdmission,
        AtomicFailpoint::DurableMetadataAdmission,
    ];
    for failpoint in failpoints {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let transaction = insert_transaction(&engine, 76_001);
        let before = atomic_audit(&engine);
        reset_localized_lookup_counts_for_test();
        set_atomic_failpoint_for_test(Some(failpoint));

        let error = engine.apply_typed_transaction(transaction).unwrap_err();

        set_atomic_failpoint_for_test(None);
        let (_, _, promotions) = take_localized_lookup_counts_for_test();
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{failpoint:?}");
        assert_eq!(promotions, 0, "{failpoint:?}");
        assert_eq!(atomic_audit(&engine), before, "{failpoint:?}");
    }
}

#[test]
fn empty_skip_selection_bypasses_mutation_preflight_but_not_admission_or_boundaries() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let selection_transaction =
        |engine: &YrsDocumentEngine, request_id, history_policy| TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy,
        };

    let mut skip = transaction_engine();
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let result = skip
        .apply_typed_transaction_with_result(selection_transaction(&skip, 760, HistoryPolicy::Skip))
        .expect("empty Skip selection must not enter mutation preflight");
    set_atomic_failpoint_for_test(None);
    assert!(result.changed);
    assert_eq!(skip.revision(), 0);
    assert_eq!(skip.state_revision(), 1);
    let skip_counts = take_prepared_admission_counts_for_test();
    assert_eq!(skip_counts.staged_seed_preparations, 0);
    assert_eq!(skip_counts.installed_base_seed_publications, 0);

    let mut boundary = transaction_engine();
    let before_boundary = atomic_audit(&boundary);
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::MutationPreflight));
    let boundary_error = boundary
        .apply_typed_transaction(selection_transaction(
            &boundary,
            761,
            HistoryPolicy::Boundary,
        ))
        .unwrap_err();
    set_atomic_failpoint_for_test(None);
    assert_eq!(
        boundary_error.details,
        Some(json!({ "failpoint": "mutationPreflight" }))
    );
    assert_eq!(atomic_audit(&boundary), before_boundary);

    let mut rejected = transaction_engine();
    let before_rejected = atomic_audit(&rejected);
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::EnvelopeAdmission));
    let admission_error = rejected
        .apply_typed_transaction(selection_transaction(&rejected, 762, HistoryPolicy::Skip))
        .unwrap_err();
    set_atomic_failpoint_for_test(None);
    assert_eq!(
        admission_error.details,
        Some(json!({ "failpoint": "envelopeAdmission" }))
    );
    assert_eq!(atomic_audit(&rejected), before_rejected);
}

#[test]
fn empty_generic_state_only_transactions_do_not_prepare_lookup_seed() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (offset, history_policy) in [
        HistoryPolicy::Skip,
        HistoryPolicy::Auto,
        HistoryPolicy::Boundary,
    ]
    .into_iter()
    .enumerate()
    {
        let request_id = 760_100 + u64::try_from(offset).unwrap();
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(
                request_id,
                TypedCommand::ToggleMark {
                    mark_type: "bold".into(),
                },
            )
            .unwrap()
            .expect("collapsed toggle must set stored marks");
        assert_eq!(
            engine
                .stored_marks()
                .unwrap()
                .iter()
                .map(Mark::mark_type)
                .collect::<Vec<_>>(),
            vec!["bold"]
        );
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before_document_revision = engine.revision();
        let before_state_revision = engine.state_revision();
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(
            LookupSeedHydrationFailpoint::InitialReservation,
        ));

        let result = engine
            .apply_typed_transaction_with_result(TypedTransaction {
                request_id: request_id + 10,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy,
            })
            .expect("state-only generic transaction must not consume hydration failure");

        set_lookup_seed_hydration_failpoint_for_test(None);
        let counts = take_prepared_admission_counts_for_test();
        assert!(result.changed, "{history_policy:?}");
        assert_eq!(
            result.selection,
            ResolvedSelection::All,
            "{history_policy:?}"
        );
        assert_eq!(
            engine.revision(),
            before_document_revision,
            "{history_policy:?}"
        );
        assert_eq!(
            engine.state_revision(),
            before_state_revision + 1,
            "{history_policy:?}"
        );
        assert!(engine.stored_marks().is_none(), "{history_policy:?}");
        assert_eq!(counts.staged_seed_preparations, 0, "{history_policy:?}");
        assert_eq!(
            counts.installed_base_seed_publications, 0,
            "{history_policy:?}"
        );
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        assert!(engine
            .derived_state
            .as_ref()
            .unwrap()
            .mutation_lookup_seed
            .is_unavailable_for_test());
    }
}

#[test]
fn empty_generic_boundary_preserves_recorded_grouping_semantics() {
    let apply_insert = |engine: &mut YrsDocumentEngine, request_id, text: &str| {
        let at = engine.position_map().unwrap().total_scalars();
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: at,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: text.into(),
                    marks: Vec::new(),
                }],
                selection_intent: SelectionIntent::Preserve,
                history_policy: HistoryPolicy::Auto,
            })
            .unwrap();
    };

    for (offset, state_only_policy) in [HistoryPolicy::Auto, HistoryPolicy::Boundary]
        .into_iter()
        .enumerate()
    {
        let request_id = 760_120 + u64::try_from(offset).unwrap() * 10;
        let mut engine = import_document_with_unavailable_lookup_seed();
        apply_insert(&mut engine, request_id, "x");
        force_lookup_seed_unavailable(&mut engine);
        engine
            .apply_typed_transaction(TypedTransaction {
                request_id: request_id + 1,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalApi,
                operations: Vec::new(),
                selection_intent: SelectionIntent::Set(SelectionInput::All),
                history_policy: state_only_policy,
            })
            .unwrap();
        apply_insert(&mut engine, request_id + 2, "y");
        assert_eq!(engine.document().unwrap().root().text_content(), "abcxy");

        engine
            .undo(request_id + 3)
            .unwrap()
            .expect("recorded insert must be undoable");
        let expected_after_first_pop = if state_only_policy == HistoryPolicy::Boundary {
            "abcx"
        } else {
            "abc"
        };
        assert_eq!(
            engine.document().unwrap().root().text_content(),
            expected_after_first_pop,
            "{state_only_policy:?}"
        );
        if state_only_policy == HistoryPolicy::Boundary {
            engine
                .undo(request_id + 4)
                .unwrap()
                .expect("Boundary must retain the earlier group");
            assert_eq!(engine.document().unwrap().root().text_content(), "abc");
        }
    }
}

#[test]
fn changed_state_boundary_revision_overflow_precedes_replay_allocation() {
    let mut engine = import_document_with_unavailable_lookup_seed();
    engine.history.compact_replay_event_capacity_for_test();
    engine.state_revision = u64::MAX;
    engine
        .derived_state
        .as_mut()
        .unwrap()
        .reseal_state_revision(u64::MAX);
    let before = atomic_audit(&engine);
    let replay_before = engine.history.replay_ledger_allocation_audit_for_test();

    let error = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 760_110,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: Vec::new(),
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Boundary,
        })
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{error:?}");
    assert_eq!(
        error.message.as_ref(),
        "stateRevision cannot be incremented"
    );
    assert_eq!(error.details, Some(json!({ "field": "stateRevision" })));
    assert_eq!(atomic_audit(&engine), before);
    assert_eq!(
        engine.history.replay_ledger_allocation_audit_for_test(),
        replay_before
    );
}

#[test]
fn generic_structural_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
    let source =
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
    let schema = tiptap_schema();
    let base_document = from_prosemirror_json(
        &serde_json::from_str(source).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let old_node_limit = crate::editor_state::document_node_count(base_document.root());
    let current_node_limit = old_node_limit + 1;
    let old_limits = ResourceLimits {
        max_document_nodes: old_node_limit,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: current_node_limit,
        ..old_limits.clone()
    };

    let mut drifted = transaction_engine_with_resource_limits_and_mode(
        old_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    let mut one_under = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    for engine in [&mut drifted, &mut preconfigured, &mut one_under] {
        engine
            .import_json(source, TransactionOrigin::DocumentImport)
            .unwrap();
    }
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        old_node_limit
    );
    assert!(derived_evidence_matches_runtime_limits(&drifted));
    drifted.resource_limits = current_limits.clone();
    assert!(!derived_evidence_matches_runtime_limits(&drifted));

    let drifted_commit = drifted
        .apply_typed_transaction(hard_break_insert_transaction(&drifted, 760_200))
        .expect("loosened runtime limit must admit the generic structural candidate");
    let preconfigured_commit = preconfigured
        .apply_typed_transaction(hard_break_insert_transaction(&preconfigured, 760_200))
        .unwrap();
    assert_eq!(drifted_commit, preconfigured_commit);
    assert_eq!(drifted_commit.document_revision, 2);
    assert_eq!(drifted_commit.state_revision, 2);
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        current_node_limit
    );
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    let drifted_followup = drifted
        .apply_typed_transaction(insert_transaction(&drifted, 760_201))
        .expect("current-limit evidence must be reusable by the following mutation");
    let preconfigured_followup = preconfigured
        .apply_typed_transaction(insert_transaction(&preconfigured, 760_201))
        .unwrap();
    assert_eq!(drifted_followup, preconfigured_followup);
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    one_under.resource_limits = old_limits;
    let before = atomic_audit(&one_under);
    let error = one_under
        .apply_typed_transaction(hard_break_insert_transaction(&one_under, 760_202))
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(u64::try_from(old_node_limit).unwrap()));
    assert_eq!(
        error.actual,
        Some(u64::try_from(current_node_limit).unwrap())
    );
    assert_eq!(atomic_audit(&one_under), before);
}

#[test]
fn remote_limit_drift_matches_preconfigured_current_and_reuses_evidence() {
    let source_json =
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#;
    let schema = tiptap_schema();
    let base_document = from_prosemirror_json(
        &serde_json::from_str(source_json).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let old_node_limit = crate::editor_state::document_node_count(base_document.root());
    let current_node_limit = old_node_limit + 1;
    let old_limits = ResourceLimits {
        max_document_nodes: old_node_limit,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: current_node_limit,
        ..old_limits.clone()
    };
    let mut source = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::LocalEmpty,
    );
    source
        .import_json(source_json, TransactionOrigin::DocumentImport)
        .unwrap();
    let base_update = source.encoded_state().unwrap();
    let mut drifted = transaction_engine_with_resource_limits_and_mode(
        old_limits,
        crate::yrs_engine::InitializationMode::AwaitRemote,
    );
    let mut preconfigured = transaction_engine_with_resource_limits_and_mode(
        current_limits.clone(),
        crate::yrs_engine::InitializationMode::AwaitRemote,
    );
    let drifted_base = drifted
        .apply_remote_update_v1(760_210, &base_update)
        .unwrap();
    let preconfigured_base = preconfigured
        .apply_remote_update_v1(760_210, &base_update)
        .unwrap();
    assert_eq!(drifted_base, preconfigured_base);
    assert!(derived_evidence_matches_runtime_limits(&drifted));
    drifted.resource_limits = current_limits;
    assert!(!derived_evidence_matches_runtime_limits(&drifted));

    let target_vector = drifted.doc.transact().state_vector();
    source
        .apply_typed_transaction(paragraph_insert_transaction(&source, 760_211))
        .unwrap();
    let structural_delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    let drifted_commit = drifted
        .apply_remote_update_v1(760_212, &structural_delta)
        .expect("loosened runtime limit must admit the changed remote candidate");
    let preconfigured_commit = preconfigured
        .apply_remote_update_v1(760_212, &structural_delta)
        .unwrap();
    assert_eq!(drifted_commit, preconfigured_commit);
    assert_eq!(
        drifted.derived_state.as_ref().unwrap().document_node_count,
        current_node_limit
    );
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);

    let target_vector = drifted.doc.transact().state_vector();
    source
        .apply_typed_transaction(insert_transaction(&source, 760_213))
        .unwrap();
    let followup_delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);
    let drifted_followup = drifted
        .apply_remote_update_v1(760_214, &followup_delta)
        .expect("remote current-limit evidence must be reusable");
    let preconfigured_followup = preconfigured
        .apply_remote_update_v1(760_214, &followup_delta)
        .unwrap();
    assert_eq!(drifted_followup, preconfigured_followup);
    assert_limit_drift_semantic_parity(&drifted, &preconfigured);
}

#[test]
fn empty_skip_collapsed_text_prepares_one_forward_point_without_reverse_traversal() {
    use crate::yrs_engine::derived_state::{
        reset_relative_selection_traversal_counts_for_test,
        take_relative_selection_traversal_counts_for_test,
    };
    use crate::yrs_engine::position::{
        reset_relative_position_traversal_counts_for_test,
        take_relative_position_traversal_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"prefix"}]},{"type":"paragraph","content":[{"type":"text","text":"a😀middle"}]},{"type":"paragraph","content":[{"type":"text","text":"suffix"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let point = RevisionedPosition {
        offset: 9,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::Before,
    };
    reset_relative_position_traversal_counts_for_test();
    reset_relative_selection_traversal_counts_for_test();

    let result = engine
        .apply_typed_transaction_with_result(TypedTransaction {
            request_id: 759,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();

    assert!(result.changed);
    assert_eq!(result.document_revision, 1);
    assert_eq!(result.state_revision, 2);
    assert!(matches!(
        result.selection,
        ResolvedSelection::Text { anchor, head }
            if anchor == head && anchor.scalar == point.offset
    ));
    assert_eq!(
        take_relative_position_traversal_counts_for_test(),
        (0, 1, 0),
        "collapsed exact inputs must share one admitted forward materialization"
    );
    assert_eq!(
        take_relative_selection_traversal_counts_for_test(),
        (0, 0),
        "prepared resolved points must not round-trip through Yrs"
    );
}

#[test]
fn empty_skip_prepared_collapsed_text_preserves_overflow_and_output_atomicity() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        let point = RevisionedPosition {
            offset: 3,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::Before,
        };
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::Text {
                anchor: point,
                head: point,
            }),
            history_policy: HistoryPolicy::Skip,
        }
    }

    let mut overflow = populated_engine();
    overflow.state_revision = u64::MAX;
    overflow.derived_state.as_mut().unwrap().state_revision = u64::MAX;
    let overflow_before = atomic_audit(&overflow);
    let overflow_transaction = transaction(&overflow, 759_001);

    let overflow_error = overflow
        .apply_typed_transaction_with_result(overflow_transaction)
        .unwrap_err();

    assert_eq!(overflow_error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(
        overflow_error.details,
        Some(json!({ "field": "stateRevision" }))
    );
    assert_eq!(atomic_audit(&overflow), overflow_before);

    let mut output_limited = populated_engine();
    output_limited.editing_limits.max_derived_output_bytes = 1;
    let output_before = atomic_audit(&output_limited);
    let output_transaction = transaction(&output_limited, 759_002);

    let output_error = output_limited
        .apply_typed_transaction_with_result(output_transaction)
        .unwrap_err();

    assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(
        output_error.details,
        Some(json!({ "field": "maxDerivedOutputBytes" }))
    );
    assert_eq!(atomic_audit(&output_limited), output_before);
}

#[test]
fn empty_skip_fast_path_matches_full_compiler_at_yrs_scan_work_boundary() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"scan boundary"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn scan_work(engine: &YrsDocumentEngine) -> usize {
        let document_text_bytes = engine.document().unwrap().root().text_content().len();
        let txn = engine.doc.transact();
        let crdt_clock_work = txn
            .state_vector()
            .iter()
            .map(|(_, clock)| usize::try_from(*clock).unwrap() + 1)
            .sum::<usize>();
        document_text_bytes * 2 + crdt_clock_work * 2
    }

    fn transaction(engine: &YrsDocumentEngine, request_id: u64) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        }
    }

    let required = scan_work(&populated_engine());

    let mut exact_fast = populated_engine();
    exact_fast.resource_limits.max_input_bytes = required;
    let exact_fast_result = exact_fast
        .apply_typed_transaction_with_result(transaction(&exact_fast, 763))
        .unwrap();
    let mut exact_slow = populated_engine();
    exact_slow.resource_limits.max_input_bytes = required;
    let exact_slow_transaction = transaction(&exact_slow, 763);
    let exact_slow_compiled = exact_slow
        .compile_typed_transaction(exact_slow_transaction)
        .unwrap();
    let exact_slow_result = exact_slow
        .apply_compiled_transaction(exact_slow_compiled, true)
        .unwrap()
        .1
        .unwrap();
    assert_eq!(exact_fast_result, exact_slow_result);
    assert_eq!(exact_fast.document_json(), exact_slow.document_json());
    assert_eq!(exact_fast.document_html(), exact_slow.document_html());
    assert_eq!(exact_fast.revision(), exact_slow.revision());
    assert_eq!(exact_fast.state_revision(), exact_slow.state_revision());
    assert_eq!(
        exact_fast.resolved_selection(),
        exact_slow.resolved_selection()
    );
    assert_eq!(exact_fast.stored_marks(), exact_slow.stored_marks());
    assert_eq!(exact_fast.can_undo(), exact_slow.can_undo());
    assert_eq!(exact_fast.can_redo(), exact_slow.can_redo());

    let mut one_under_slow = populated_engine();
    one_under_slow.resource_limits.max_input_bytes = required - 1;
    let before_slow = atomic_audit(&one_under_slow);
    let slow_error = one_under_slow
        .compile_typed_transaction(transaction(&one_under_slow, 764))
        .unwrap_err();
    assert_eq!(atomic_audit(&one_under_slow), before_slow);

    let mut one_under_fast = populated_engine();
    one_under_fast.resource_limits.max_input_bytes = required - 1;
    let before_fast = atomic_audit(&one_under_fast);
    let fast_error = one_under_fast
        .apply_typed_transaction_with_result(transaction(&one_under_fast, 764))
        .unwrap_err();
    assert_eq!(fast_error, slow_error);
    assert_eq!(atomic_audit(&one_under_fast), before_fast);

    let mut changed_document = populated_engine();
    changed_document
        .apply_command(765, TypedCommand::InsertText { text: "é".into() })
        .unwrap()
        .unwrap();
    let cached_text_bytes = changed_document
        .derived_state
        .as_ref()
        .unwrap()
        .document_text_bytes;
    assert_eq!(
        cached_text_bytes,
        changed_document
            .document()
            .unwrap()
            .root()
            .text_content()
            .len()
    );
    let changed_required = scan_work(&changed_document);
    changed_document.resource_limits.max_input_bytes = changed_required - 1;
    let before_changed = atomic_audit(&changed_document);
    let changed_error = changed_document
        .apply_typed_transaction_with_result(transaction(&changed_document, 766))
        .unwrap_err();
    assert_eq!(changed_error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        changed_error.limit,
        Some(u64::try_from(changed_required - 1).unwrap())
    );
    assert_eq!(
        changed_error.actual,
        Some(u64::try_from(changed_required).unwrap())
    );
    assert_eq!(atomic_audit(&changed_document), before_changed);

    let invalid_selection = |engine: &YrsDocumentEngine| TypedTransaction {
        request_id: 767,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations: vec![],
        selection_intent: SelectionIntent::Set(SelectionInput::Text {
            anchor: RevisionedPosition {
                offset: u32::MAX,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::Before,
            },
            head: RevisionedPosition {
                offset: u32::MAX,
                kind: EditorOffsetKind::Utf16,
                affinity: Affinity::After,
            },
        }),
        history_policy: HistoryPolicy::Skip,
    };
    let invalid_slow = populated_engine();
    let before_invalid_slow = atomic_audit(&invalid_slow);
    let invalid_slow_error = invalid_slow
        .compile_typed_transaction(invalid_selection(&invalid_slow))
        .unwrap_err();
    assert_eq!(atomic_audit(&invalid_slow), before_invalid_slow);
    let mut invalid_fast = populated_engine();
    let before_invalid_fast = atomic_audit(&invalid_fast);
    let invalid_fast_error = invalid_fast
        .apply_typed_transaction_with_result(invalid_selection(&invalid_fast))
        .unwrap_err();
    assert_eq!(invalid_fast_error, invalid_slow_error);
    assert_eq!(atomic_audit(&invalid_fast), before_invalid_fast);
}

#[test]
fn empty_skip_fast_path_matches_full_compiler_for_selection_forms_and_local_state() {
    fn populated_engine() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &json!({
                    "type": "doc",
                    "content": [
                        {"type": "paragraph", "content": [{"type": "text", "text": "a😀b"}]},
                        {"type": "horizontalRule"},
                        {"type": "paragraph", "content": [{"type": "text", "text": "tail"}]}
                    ]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        engine
    }

    fn transaction(
        engine: &YrsDocumentEngine,
        request_id: u64,
        selection_intent: SelectionIntent,
    ) -> TypedTransaction {
        TypedTransaction {
            request_id,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent,
            history_policy: HistoryPolicy::Skip,
        }
    }

    fn slow_result(
        engine: &mut YrsDocumentEngine,
        transaction: TypedTransaction,
    ) -> crate::yrs_engine::TypedTransactionResult {
        let compiled = engine.compile_typed_transaction(transaction).unwrap();
        engine
            .apply_compiled_transaction(compiled, true)
            .unwrap()
            .1
            .unwrap()
    }

    let scalar = |offset, affinity| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity,
    };
    let utf16 = |offset, affinity| RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Utf16,
        affinity,
    };
    let intents = [
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: utf16(3, Affinity::Before),
            head: utf16(3, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Node {
            at: scalar(4, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::All),
        SelectionIntent::Preserve,
        SelectionIntent::UseOperationResult,
    ];

    for (index, intent) in intents.into_iter().enumerate() {
        let mut fast = populated_engine();
        let mut slow = populated_engine();
        let fast_before = atomic_audit(&fast);
        let slow_before = atomic_audit(&slow);
        let fast_transaction = transaction(&fast, 770 + index as u64, intent.clone());
        let slow_transaction = transaction(&slow, 770 + index as u64, intent.clone());

        let fast_result = fast
            .apply_typed_transaction_with_result(fast_transaction)
            .unwrap();
        let slow_result = slow_result(&mut slow, slow_transaction);

        assert_eq!(fast_result, slow_result, "intent={intent:?}");
        assert_eq!(
            fast.document_json(),
            slow.document_json(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.document_html(),
            slow.document_html(),
            "intent={intent:?}"
        );
        assert_eq!(fast.revision(), slow.revision(), "intent={intent:?}");
        assert_eq!(
            fast.state_revision(),
            slow.state_revision(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.resolved_selection(),
            slow.resolved_selection(),
            "intent={intent:?}"
        );
        assert_eq!(
            fast.stored_marks(),
            slow.stored_marks(),
            "intent={intent:?}"
        );
        assert_eq!(fast.can_undo(), slow.can_undo(), "intent={intent:?}");
        assert_eq!(fast.can_redo(), slow.can_redo(), "intent={intent:?}");
        assert_eq!(fast.encoded_state().unwrap(), fast_before.encoded);
        assert_eq!(slow.encoded_state().unwrap(), slow_before.encoded);
        assert_eq!(fast.yrs_state_epoch, fast_before.yrs_state_epoch);
        assert_eq!(slow.yrs_state_epoch, slow_before.yrs_state_epoch);
        assert_eq!(
            fast.history.replay_audit_for_test(),
            fast_before.replay_audit
        );
        assert_eq!(
            slow.history.replay_audit_for_test(),
            slow_before.replay_audit
        );
    }

    let stored_mark_intents = [
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(1, Affinity::Before),
            head: scalar(1, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(1, Affinity::Before),
            head: scalar(1, Affinity::After),
        }),
        SelectionIntent::Set(SelectionInput::Text {
            anchor: scalar(2, Affinity::Before),
            head: scalar(2, Affinity::Before),
        }),
        SelectionIntent::Set(SelectionInput::Node {
            at: scalar(4, Affinity::Before),
        }),
    ];
    for (index, intent) in stored_mark_intents.into_iter().enumerate() {
        let mut fast = populated_engine();
        let mut slow = populated_engine();
        select_text(&mut fast, 780, 1, 1);
        select_text(&mut slow, 780, 1, 1);
        for engine in [&mut fast, &mut slow] {
            engine
                .apply_command(
                    781,
                    TypedCommand::ToggleMark {
                        mark_type: "bold".into(),
                    },
                )
                .unwrap()
                .unwrap();
            assert!(engine.stored_marks().is_some());
        }
        let fast_transaction = transaction(&fast, 782 + index as u64, intent.clone());
        let slow_transaction = transaction(&slow, 782 + index as u64, intent.clone());

        let fast_result = fast
            .apply_typed_transaction_with_result(fast_transaction)
            .unwrap();
        let slow_result = slow_result(&mut slow, slow_transaction);

        assert_eq!(fast_result, slow_result, "stored intent={intent:?}");
        assert_eq!(
            fast.resolved_selection(),
            slow.resolved_selection(),
            "stored intent={intent:?}"
        );
        assert_eq!(
            fast.stored_marks(),
            slow.stored_marks(),
            "stored intent={intent:?}"
        );
        if index <= 1 {
            assert!(fast.stored_marks().is_some());
        } else {
            assert!(fast.stored_marks().is_none());
        }
    }
}
