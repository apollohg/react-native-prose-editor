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
