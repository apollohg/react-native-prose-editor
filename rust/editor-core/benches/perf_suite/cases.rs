fn main() {
    let options = parse_options();
    let profile = options.mode.profile();
    let article_doc_cell = OnceLock::new();
    let article_doc = || {
        article_doc_cell
            .get_or_init(|| build_article_document(profile.article_blocks, profile.paragraph_chars))
    };
    let article_doc_2x_cell = OnceLock::new();
    let article_doc_2x = || {
        article_doc_2x_cell.get_or_init(|| {
            build_article_document(profile.article_blocks * 2, profile.paragraph_chars)
        })
    };
    let article_json_cell = OnceLock::new();
    let article_json = || {
        article_json_cell.get_or_init(|| {
            serde_json::to_string(article_doc())
                .expect("article benchmark fixture should serialize")
        })
    };
    let article_json_2x_cell = OnceLock::new();
    let article_json_2x = || {
        article_json_2x_cell.get_or_init(|| {
            serde_json::to_string(article_doc_2x())
                .expect("2x article benchmark fixture should serialize")
        })
    };
    let editing_insert_expected_cell = OnceLock::new();
    let editing_insert_expected = || {
        editing_insert_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_insert_document(article_doc(), "!", 1),
            )
        })
    };
    let editing_original_expected_cell = OnceLock::new();
    let editing_original_expected = || {
        editing_original_expected_cell
            .get_or_init(|| build_editing_case_expectation(article_doc(), article_doc().clone()))
    };
    let editing_burst_expected_cell = OnceLock::new();
    let editing_burst_expected = || {
        editing_burst_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_insert_document(article_doc(), "x", EDITING_TYPING_BURST),
            )
        })
    };
    let editing_bold_expected_cell = OnceLock::new();
    let editing_bold_expected = || {
        editing_bold_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_bold_document(
                    article_doc(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR + 8,
                ),
            )
        })
    };
    let editing_list_expected_cell = OnceLock::new();
    let editing_list_expected = || {
        editing_list_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc(),
                build_pure_list_document(article_doc(), EDITING_CURSOR_SCALAR),
            )
        })
    };
    let editing_undo_expected_cell = OnceLock::new();
    let editing_undo_expected = || {
        editing_undo_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                &build_pure_insert_document(article_doc(), "!", 1),
                article_doc().clone(),
            )
        })
    };
    let editing_insert_2x_expected_cell = OnceLock::new();
    let editing_insert_2x_expected = || {
        editing_insert_2x_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc_2x(),
                build_pure_insert_document(article_doc_2x(), "!", 1),
            )
        })
    };
    let editing_original_2x_expected_cell = OnceLock::new();
    let editing_original_2x_expected = || {
        editing_original_2x_expected_cell.get_or_init(|| {
            build_editing_case_expectation(article_doc_2x(), article_doc_2x().clone())
        })
    };
    let editing_list_2x_expected_cell = OnceLock::new();
    let editing_list_2x_expected = || {
        editing_list_2x_expected_cell.get_or_init(|| {
            build_editing_case_expectation(
                article_doc_2x(),
                build_pure_list_document(article_doc_2x(), EDITING_CURSOR_SCALAR),
            )
        })
    };
    let opaque_json_cell = OnceLock::new();
    let opaque_json = || {
        opaque_json_cell.get_or_init(|| {
            serde_json::to_string(&build_opaque_document(profile.opaque_payload_bytes))
                .expect("opaque benchmark fixture should serialize")
        })
    };
    let opaque_json_2x_cell = OnceLock::new();
    let opaque_json_2x = || {
        opaque_json_2x_cell.get_or_init(|| {
            serde_json::to_string(&build_opaque_document(profile.opaque_payload_bytes * 2))
                .expect("2x opaque benchmark fixture should serialize")
        })
    };
    let candidate_document_cell = OnceLock::new();
    let candidate_document = || {
        candidate_document_cell.get_or_init(|| {
            yrs_engine_with_document(article_json())
                .document()
                .expect("populated Yrs benchmark engine should be ready")
                .clone()
        })
    };

    let mut results = Vec::new();

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.edit.insert_char.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_insert_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                        .expect("Yrs insert-character benchmark should succeed")
                        .expect("Yrs insert-character command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 1,
                    EDITING_CURSOR_SCALAR + 1,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.edit.typing_burst.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            EDITING_TYPING_BURST,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_burst_expected(),
                )
            },
            |(engine, _)| {
                let mut final_output = None;
                for index in 0..EDITING_TYPING_BURST {
                    let output = engine
                        .apply_command(
                            index as u64 + 2,
                            TypedCommand::InsertText { text: "x".into() },
                        )
                        .expect("Yrs typing-burst benchmark should succeed")
                        .expect("Yrs typing-burst command should apply");
                    if index + 1 == EDITING_TYPING_BURST {
                        final_output = Some(output);
                    } else {
                        black_box(output);
                    }
                }
                black_box(final_output.expect("typing burst must produce a final output"))
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    EDITING_TYPING_BURST as u64 + 1,
                    TransactionOrigin::LocalCommand,
                    true,
                    1 + EDITING_TYPING_BURST as u64,
                    2 + EDITING_TYPING_BURST as u64,
                    EDITING_CURSOR_SCALAR + EDITING_TYPING_BURST as u32,
                    EDITING_CURSOR_SCALAR + EDITING_TYPING_BURST as u32,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.state.selection_light.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let (engine, position) = yrs_selection_fixture(article_json());
                (engine, position, editing_original_expected())
            },
            |(engine, position, _)| {
                black_box(
                    engine
                        .apply_typed_transaction_with_result(yrs_selection_transaction(
                            1,
                            engine.revision(),
                            *position,
                            *position,
                        ))
                        .expect("Yrs light-selection benchmark should succeed"),
                )
            },
            |(engine, position, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    1,
                    TransactionOrigin::LocalApi,
                    true,
                    1,
                    2,
                    *position,
                    *position,
                    false,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.command.toggle_mark.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR + 8,
                    ),
                    editing_bold_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(
                            2,
                            TypedCommand::ToggleMark {
                                mark_type: "bold".into(),
                            },
                        )
                        .expect("Yrs toggle-mark benchmark should succeed")
                        .expect("Yrs toggle-mark command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR + 8,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.command.wrap_list.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_list_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(
                            2,
                            TypedCommand::WrapInList {
                                list_type: "bulletList".into(),
                                item_type: "listItem".into(),
                            },
                        )
                        .expect("Yrs wrap-list benchmark should succeed")
                        .expect("Yrs wrap-list command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 2,
                    EDITING_CURSOR_SCALAR + 2,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.history.undo.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut engine = yrs_editing_fixture(
                    article_json(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                );
                engine
                    .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                    .expect("Yrs undo fixture edit should succeed")
                    .expect("Yrs undo fixture edit should apply");
                (engine, editing_undo_expected())
            },
            |(engine, _)| {
                black_box(
                    engine
                        .undo_with_result(3)
                        .expect("Yrs undo benchmark should succeed")
                        .expect("Yrs undo benchmark should have history"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    3,
                    TransactionOrigin::UndoRedo,
                    true,
                    3,
                    4,
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                    false,
                    true,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.history.redo.article.1x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut engine = yrs_editing_fixture(
                    article_json(),
                    EDITING_CURSOR_SCALAR,
                    EDITING_CURSOR_SCALAR,
                );
                engine
                    .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                    .expect("Yrs redo fixture edit should succeed")
                    .expect("Yrs redo fixture edit should apply");
                engine
                    .undo(3)
                    .expect("Yrs redo fixture undo should succeed")
                    .expect("Yrs redo fixture should create redo history");
                (engine, editing_insert_expected())
            },
            |(engine, _)| {
                black_box(
                    engine
                        .redo_with_result(4)
                        .expect("Yrs redo benchmark should succeed")
                        .expect("Yrs redo benchmark should have history"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    4,
                    TransactionOrigin::UndoRedo,
                    true,
                    4,
                    5,
                    EDITING_CURSOR_SCALAR + 1,
                    EDITING_CURSOR_SCALAR + 1,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.edit.insert_char.article.2x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json_2x(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_insert_2x_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(2, TypedCommand::InsertText { text: "!".into() })
                        .expect("2x Yrs insert-character benchmark should succeed")
                        .expect("2x Yrs insert-character command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 1,
                    EDITING_CURSOR_SCALAR + 1,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.state.selection_light.article.2x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let (engine, position) = yrs_selection_fixture(article_json_2x());
                (engine, position, editing_original_2x_expected())
            },
            |(engine, position, _)| {
                black_box(
                    engine
                        .apply_typed_transaction_with_result(yrs_selection_transaction(
                            1,
                            engine.revision(),
                            *position,
                            *position,
                        ))
                        .expect("2x Yrs light-selection benchmark should succeed"),
                )
            },
            |(engine, position, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    1,
                    TransactionOrigin::LocalApi,
                    true,
                    1,
                    2,
                    *position,
                    *position,
                    false,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        verified_bench_case(
            "yrs.command.wrap_list.article.2x",
            "yrs-editing",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    yrs_editing_fixture(
                        article_json_2x(),
                        EDITING_CURSOR_SCALAR,
                        EDITING_CURSOR_SCALAR,
                    ),
                    editing_list_2x_expected(),
                )
            },
            |(engine, _)| {
                black_box(
                    engine
                        .apply_command(
                            2,
                            TypedCommand::WrapInList {
                                list_type: "bulletList".into(),
                                item_type: "listItem".into(),
                            },
                        )
                        .expect("2x Yrs wrap-list benchmark should succeed")
                        .expect("2x Yrs wrap-list command should apply"),
                )
            },
            |(engine, expectation), output| {
                assert_yrs_editing_output(
                    engine,
                    expectation,
                    output,
                    2,
                    TransactionOrigin::LocalCommand,
                    true,
                    2,
                    3,
                    EDITING_CURSOR_SCALAR + 2,
                    EDITING_CURSOR_SCALAR + 2,
                    true,
                    false,
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), article_json().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("Yrs JSON import benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_export.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || yrs_engine_with_document(article_json()),
            |engine| black_box(engine.document_json_string()),
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.article.2x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), article_json_2x().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("2x Yrs JSON import benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_export.article.2x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || yrs_engine_with_document(article_json_2x()),
            |engine| black_box(engine.document_json_string()),
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.candidate_validation.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    candidate_document().clone(),
                    tiptap_schema(),
                    ResourceLimits::default(),
                )
            },
            |(document, schema, limits)| {
                black_box(
                    DocumentValidator::validate(document, schema, limits)
                        .expect("Yrs candidate validation benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.encoded_state.article.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || yrs_engine_with_document(article_json()),
            |engine| {
                black_box(
                    engine
                        .encoded_state()
                        .expect("Yrs encoded-state benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.opaque_large.1x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), opaque_json().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("large opaque Yrs import benchmark should succeed"),
                );
            },
        ),
    );

    push_case!(
        &mut results,
        &options,
        bench_case(
            "yrs.json_import.opaque_large.2x",
            "yrs-foundation",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || (empty_yrs_engine(), opaque_json_2x().clone()),
            |(engine, document)| {
                black_box(
                    engine
                        .import_json(document, TransactionOrigin::DocumentImport)
                        .expect("2x large opaque Yrs import benchmark should succeed"),
                );
            },
        ),
    );

    if results.is_empty() {
        eprintln!("no benchmarks matched the provided filter");
        std::process::exit(1);
    }

    if options.json_output {
        print_json_summary(options.mode, profile, &results);
    } else {
        print_table(options.mode, profile, &results);
    }
}
