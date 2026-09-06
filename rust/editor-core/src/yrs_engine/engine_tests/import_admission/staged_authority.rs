#[test]
fn staged_authority_supplies_every_unavailable_seed_consumer_without_installed_reads() {
    use crate::yrs_engine::observability::{
        reset_full_pass_counts_for_test, reset_prepared_admission_counts_for_test,
        take_full_pass_counts_for_test, take_prepared_admission_counts_for_test,
    };

    reset_prepared_admission_counts_for_test();
    let (engine, deferred, mut context, transaction, expected_document) =
        deferred_finalization_fixture();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    engine.prepare_mutation_identity(&mut context).unwrap();
    reset_full_pass_counts_for_test();

    let prepared = engine
        .finalize_deferred_for_test(deferred, &context, &transaction, &expected_document)
        .unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    let cached = state.compilation_view();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let authority = context
        .authority(
            crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                request_id: transaction.request_id,
                installed: state,
                txn: &txn,
                fragment: &fragment,
                fragment_name: &engine.fragment_name,
                schema_fingerprint: &engine.schema_fingerprint,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                max_length: engine.max_length,
                document_revision: engine.revision,
                state_revision: engine.state_revision,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        )
        .unwrap();
    assert!(installed.is_unavailable_for_test());
    assert!(authority.lookup_seed().is_ready_for_test());
    assert!(!Arc::ptr_eq(&installed, authority.lookup_seed()));

    let format_from = crate::yrs_engine::position::editor_offset_to_doc_pos(
        0,
        EditorOffsetKind::Scalar,
        &state.rendered_text,
        &state.position_map,
        &state.document,
    )
    .unwrap();
    let format_to = crate::yrs_engine::position::editor_offset_to_doc_pos(
        2,
        EditorOffsetKind::Scalar,
        &state.rendered_text,
        &state.position_map,
        &state.document,
    )
    .unwrap();
    let format_block = state
        .position_map
        .find_block_for_doc_pos(format_from)
        .and_then(|index| state.position_map.block(index))
        .unwrap();
    let format_locator = crate::yrs_engine::mutation::LocalizedFormatLocator::mint(
        &state.document,
        &format_block.node_path,
        format_from,
        format_to,
        authority.lookup_seed().as_ref(),
        &txn,
        &fragment,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .expect("staged authority mints a current localized format locator");
    assert!(
        crate::yrs_engine::mutation::LocalizedFormatCompiler::try_new(
            transaction.request_id,
            &txn,
            &fragment,
            &engine.schema,
            usize::MAX,
            engine.resource_limits.max_input_bytes,
            0,
            format_locator,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap()
        .is_some()
    );

    let first_child = state
        .document
        .root()
        .content()
        .and_then(|content| content.child(0))
        .unwrap()
        .clone();
    let root_replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        0,
        1,
        crate::model::Fragment::from(vec![first_child]),
        Selection::cursor(0),
    );
    let root_locator = crate::yrs_engine::mutation::LocalizedRootWindowLocator::mint(
        transaction.request_id,
        &state.document,
        &state.document,
        &root_replacement,
        authority.lookup_seed().as_ref(),
        &txn,
        &fragment,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .unwrap()
    .expect("staged authority mints a current localized root-window locator");
    assert!(
        crate::yrs_engine::mutation::LocalizedRootWindowCompiler::try_new(
            transaction.request_id,
            &txn,
            &fragment,
            &engine.schema,
            usize::MAX,
            engine.resource_limits.max_input_bytes,
            0,
            root_locator,
        )
        .unwrap()
        .is_some()
    );

    let mut compiled =
        crate::yrs_engine::compiler::compile_prepared_transaction_with_yrs_and_stored_marks(
            crate::yrs_engine::compiler::CompilationContext {
                document: cached.document,
                selection: Some(cached.selection),
                schema: &engine.schema,
                resource_limits: &engine.resource_limits,
                editing_limits: &engine.editing_limits,
                document_revision: engine.revision,
                max_length: engine.max_length,
            },
            transaction.clone(),
            &txn,
            &fragment,
            crate::yrs_engine::compiler::StoredMarksCompilationContext {
                stored_marks: state.stored_marks.as_deref(),
                resolved_selection: &state.resolved_selection,
                relative_selection: &state.relative_selection,
            },
            crate::yrs_engine::compiler::PreparedSemanticContext {
                admission: &prepared,
                expected_preview: &expected_document,
                yrs_state_epoch: engine.yrs_state_epoch,
                state_revision: engine.state_revision,
                schema_fingerprint: &engine.schema_fingerprint,
            },
            crate::yrs_engine::compiler::EngineCompilationView {
                cached,
                authority: &authority,
                state_revision: engine.state_revision,
                schema_fingerprint: &engine.schema_fingerprint,
                yrs_state_epoch: engine.yrs_state_epoch,
            },
        )
        .unwrap();
    assert!(compiled.localized_semantic_used);
    assert!(compiled.localized_insert_admission.is_some());
    assert!(compiled.prepared_derived_evidence.is_some());
    assert!(compiled.mutation_lookup_transition.is_some());

    let admission = compiled.localized_insert_admission.as_ref().unwrap();
    let crate::yrs_engine::compiler::StoredMarksPlan::Set(stored_marks) =
        &compiled.stored_marks_plan
    else {
        panic!("localized compiler seals stored marks")
    };
    let active_transition = state
        .prepare_active_state_transition(
            transaction.request_id,
            &authority,
            admission,
            &compiled.preview,
            admission.operation_result_selection(),
            stored_marks.as_deref(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .unwrap();
    let structural = admission.active_state_structural_seal();
    assert!(state
        .validate_active_state_transition(
            &authority,
            &active_transition,
            &structural,
            &compiled.preview,
            admission.operation_result_selection(),
            stored_marks.as_deref(),
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            engine.yrs_state_epoch,
        )
        .is_some());

    let selection_seal =
        crate::yrs_engine::compiler::PreparedSelectionMutationSeal::capture(&compiled)
            .expect("localized insert captures its prepared selection seal");
    assert!(selection_seal.matches(&compiled, &authority));

    let evidence = compiled.prepared_derived_evidence.take().unwrap();
    let derivations = compiled.preview_derivations.as_ref().unwrap();
    let render_transition = evidence
        .prepare_localized_render_transition(
            state,
            &compiled.preview,
            derivations,
            &compiled.affected_top_level_blocks,
            &engine.schema,
            &engine.schema_fingerprint,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
        )
        .expect("localized render proof remains current")
        .unwrap();
    let next_document_revision = engine.revision.checked_add(1).unwrap();
    let next_state_revision = engine.state_revision.checked_add(1).unwrap();
    let next_yrs_state_epoch = engine.yrs_state_epoch.checked_add(1).unwrap();
    assert!(evidence
        .finalize(
            &authority,
            &compiled.preview,
            compiled.canonical_artifact.as_ref().unwrap(),
            derivations,
            &render_transition.cache,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
        )
        .is_some());

    let next_seed = engine
        .prepare_mutation_lookup_transition_with_authority(
            transaction.request_id,
            &authority,
            compiled.mutation_lookup_transition.as_ref().unwrap(),
            &txn,
            &fragment,
            &compiled.preview,
            compiled.canonical_artifact.as_ref().unwrap(),
            next_yrs_state_epoch,
            next_document_revision,
        )
        .unwrap();
    assert!(next_seed.is_ready_for_test());
    assert!(!Arc::ptr_eq(&installed, &next_seed));
    let installed_adapter =
        crate::yrs_engine::prepared_admission::InstalledDerivedStateAuthority::new(state);
    assert!(
        crate::yrs_engine::prepared_admission::DerivedStateAuthority::lookup_seed(
            &installed_adapter,
            transaction.request_id,
        )
        .is_err()
    );
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let passes = take_full_pass_counts_for_test();
    assert_eq!(passes.document_validations, 0);
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn staged_authority_rejects_installed_substitution_and_live_seal_drift_before_transition() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for case in [
        "request",
        "store",
        "fragment",
        "schema",
        "resource_limits",
        "editing_limits",
        "max_length",
        "document_revision",
        "state_revision",
        "epoch",
        "identity",
    ] {
        let mut engine = import_document_with_unavailable_lookup_seed();
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        reset_prepared_admission_counts_for_test();
        let mut context = engine.prepare_mutation_lookup_seed(65_250).unwrap();
        engine.prepare_mutation_identity(&mut context).unwrap();

        if case == "identity" {
            let state = engine.derived_state.as_mut().unwrap();
            state.canonical_artifact = state
                .canonical_artifact
                .schema_context()
                .derive(&state.document)
                .unwrap();
        }
        let before = atomic_audit(&engine);
        let state = engine.derived_state.as_ref().unwrap();
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let foreign = transaction_engine();
        let foreign_txn = foreign.doc.transact();
        let foreign_fragment = foreign_txn
            .get_xml_fragment(foreign.fragment_name.as_str())
            .unwrap();
        let mut drifted_resources = engine.resource_limits.clone();
        drifted_resources.max_input_bytes = drifted_resources
            .max_input_bytes
            .checked_sub(1)
            .expect("fixture resource limit is positive");
        let mut drifted_editing = engine.editing_limits.clone();
        drifted_editing.max_operations_per_transaction = drifted_editing
            .max_operations_per_transaction
            .checked_sub(1)
            .expect("fixture editing limit is positive");
        let drifted_max_length = match engine.max_length {
            Some(_) => None,
            None => Some(1),
        };
        let drifted_document_revision = engine
            .revision
            .checked_add(1)
            .expect("fixture document revision can advance");
        let drifted_state_revision = engine
            .state_revision
            .checked_add(1)
            .expect("fixture state revision can advance");
        let drifted_schema = format!("{}!", engine.schema_fingerprint);

        let error = match case {
            "request" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_251,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "store" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &foreign_txn,
                        fragment: &foreign_fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "fragment" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: "foreign-fragment",
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "schema" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &drifted_schema,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "resource_limits" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &drifted_resources,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "editing_limits" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &drifted_editing,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "max_length" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: drifted_max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "document_revision" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: drifted_document_revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "state_revision" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: drifted_state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            "epoch" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch.saturating_add(1),
                    },
                )
                .err(),
            "identity" => context
                .authority(
                    crate::yrs_engine::prepared_admission::LiveMutationAuthorityContext {
                        request_id: 65_250,
                        installed: state,
                        txn: &txn,
                        fragment: &fragment,
                        fragment_name: &engine.fragment_name,
                        schema_fingerprint: &engine.schema_fingerprint,
                        resource_limits: &engine.resource_limits,
                        editing_limits: &engine.editing_limits,
                        max_length: engine.max_length,
                        document_revision: engine.revision,
                        state_revision: engine.state_revision,
                        yrs_state_epoch: engine.yrs_state_epoch,
                    },
                )
                .err(),
            _ => unreachable!(),
        }
        .expect("drifted live context must not mint an authority");
        assert_eq!(error.code, "ENGINE_INVARIANT_FAILED", "{case}");
        drop(foreign_txn);
        drop(txn);
        assert_eq!(atomic_audit(&engine), before, "{case}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 1, "{case}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{case}");
    }
}

#[test]
fn generic_typed_compilation_uses_staged_authority_without_publishing_base_seed() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    let mut public_rich = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    let transaction = insert_transaction(&engine, 65_225);
    let (commit, result) = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap();
    assert!(result.is_none());
    let counts = take_prepared_admission_counts_for_test();
    let authority_counts = take_compiled_commit_authority_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert_eq!(authority_counts, (1, 1));
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_prepared_admission_counts_for_test();
    let public_commit = public
        .apply_typed_transaction(insert_transaction(&public, 65_225))
        .unwrap();
    let public_counts = take_prepared_admission_counts_for_test();
    assert_eq!(public_counts.staged_seed_preparations, 1);
    assert_eq!(public_counts.installed_base_seed_publications, 0);
    assert!(public
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    assert_eq!(commit, public_commit);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());

    reset_prepared_admission_counts_for_test();
    let rich_result = public_rich
        .apply_typed_transaction_with_result(insert_transaction(&public_rich, 65_225))
        .unwrap();
    assert!(rich_result.changed);
    let rich_counts = take_prepared_admission_counts_for_test();
    assert_eq!(rich_counts.staged_seed_preparations, 1);
    assert_eq!(rich_counts.installed_base_seed_publications, 0);
    assert!(public_rich
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn staged_generic_compiler_semantic_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let transaction = insert_transaction(&engine, 65_226);
    let error = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_atomic_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn staged_generic_lookup_transition_failure_is_prewrite_and_atomic() {
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = import_document_with_unavailable_lookup_seed();
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    let before = atomic_audit(&engine);
    reset_prepared_admission_counts_for_test();
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::LookupTransition,
    ));
    let transaction = insert_transaction(&engine, 65_227);
    let error = engine
        .apply_typed_transaction_with_staged_context(
            transaction,
            false,
            &mut OutboundUpdateSink::detached(),
        )
        .unwrap_err();
    set_compiled_commit_stage_failpoint_for_test(None);

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(atomic_audit(&engine), before);
    assert!(Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
}

#[test]
fn history_candidate_swap_prepares_ready_candidate_seed_without_compiled_transaction() {
    use crate::yrs_engine::compiler::{set_atomic_failpoint_for_test, AtomicFailpoint};
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };
    use crate::yrs_engine::TransactionCommit;

    let mut engine = import_document_with_unavailable_lookup_seed();
    let mut public = import_document_with_unavailable_lookup_seed();
    engine
        .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    public
        .apply_command(65_226, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    force_lookup_seed_unavailable(&mut engine);
    force_lookup_seed_unavailable(&mut public);
    let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    reset_prepared_admission_counts_for_test();
    set_atomic_failpoint_for_test(Some(AtomicFailpoint::SemanticCompilation));
    let result = engine.apply_history_pop(65_227, true, true, &mut OutboundUpdateSink::detached());
    let compiler_failpoint = crate::yrs_engine::compiler::check_atomic_failpoint(
        65_227,
        AtomicFailpoint::SemanticCompilation,
    );
    set_atomic_failpoint_for_test(None);
    let (commit, result) = result.unwrap().unwrap();
    let result = result.unwrap();
    let compiler_error = compiler_failpoint.unwrap_err();
    assert_eq!(compiler_error.code, "ENGINE_INVARIANT_FAILED");
    let counts = take_prepared_admission_counts_for_test();
    assert_eq!(counts.staged_seed_preparations, 1);
    assert_eq!(counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &installed,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    let state = engine.derived_state.as_ref().unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    assert!(state
        .mutation_lookup_seed
        .matches_canonical_artifact(&state.canonical_artifact));
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
    drop(txn);
    assert_eq!(
        commit,
        TransactionCommit {
            request_id: result.request_id,
            changed: result.changed,
            document_revision: result.document_revision,
            state_revision: result.state_revision,
            origin: result.origin,
        }
    );

    let public_result = public.undo_with_result(65_227).unwrap().unwrap();
    assert_eq!(result, public_result);
    assert_eq!(engine.document_json(), public.document_json());
    assert_eq!(engine.resolved_selection(), public.resolved_selection());
    assert_eq!(engine.stored_marks(), public.stored_marks());
    assert_eq!(engine.can_undo(), public.can_undo());
    assert_eq!(engine.can_redo(), public.can_redo());
    assert_eq!(
        engine.history.replay_audit_for_test(),
        public.history.replay_audit_for_test()
    );
    assert_eq!(
        engine.history.retained_units(65_227).unwrap(),
        public.history.retained_units(65_227).unwrap()
    );
}

#[test]
fn history_candidate_publication_failures_are_pre_swap_and_atomic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    for (request_id, failpoint, stage) in [
        (
            65_228,
            LookupSeedHydrationFailpoint::CandidateBindingPublication,
            "candidateBindingPublication",
        ),
        (
            65_229,
            LookupSeedHydrationFailpoint::CandidateSeedPublication,
            "candidateSeedPublication",
        ),
    ] {
        let mut engine = import_document_with_unavailable_lookup_seed();
        engine
            .apply_command(
                request_id - 1,
                TypedCommand::InsertText { text: "x".into() },
            )
            .unwrap()
            .unwrap();
        force_lookup_seed_unavailable(&mut engine);
        let installed = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
        let before = atomic_audit(&engine);
        reset_prepared_admission_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        let error = engine
            .apply_history_pop(request_id, true, true, &mut OutboundUpdateSink::detached())
            .unwrap_err();
        set_lookup_seed_hydration_failpoint_for_test(None);

        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED", "{stage}");
        assert_eq!(
            error.message.as_ref(),
            format!("mutation lookup seed allocation failed during {stage}"),
            "{stage}"
        );
        assert_eq!(
            error.details,
            Some(json!({ "field": "mutationLookupSeed" })),
            "{stage}"
        );
        assert_eq!(atomic_audit(&engine), before, "{stage}");
        assert!(Arc::ptr_eq(
            &installed,
            &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
        ));
        let counts = take_prepared_admission_counts_for_test();
        assert_eq!(counts.staged_seed_preparations, 0, "{stage}");
        assert_eq!(counts.installed_base_seed_publications, 0, "{stage}");
    }
}
