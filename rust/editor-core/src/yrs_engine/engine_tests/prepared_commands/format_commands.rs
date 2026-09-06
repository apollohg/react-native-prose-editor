#[test]
fn prepared_toggle_mark_uses_no_eager_whole_tree_collectors() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, reset_range_format_lowering_counts_for_test,
        take_localized_lookup_counts_for_test, take_range_format_lowering_counts_for_test,
    };

    let mut content = Vec::with_capacity(161);
    content.push(json!({
        "type": "h1",
        "content": [{ "type": "text", "text": "h".repeat(42) }]
    }));
    for index in 0..160 {
        let inline = if index == 0 {
            vec![
                json!({ "type": "text", "text": "p".repeat(55) }),
                json!({
                    "type": "text",
                    "text": "b".repeat(55),
                    "marks": [{ "type": "bold" }]
                }),
                json!({
                    "type": "text",
                    "text": "i".repeat(55),
                    "marks": [{ "type": "italic" }]
                }),
                json!({ "type": "text", "text": "t".repeat(55) }),
            ]
        } else {
            vec![json!({
                "type": "text",
                "text": format!("{index:04} {}", "x".repeat(215))
            })]
        };
        content.push(json!({ "type": "paragraph", "content": inline }));
    }
    let mut engine = transaction_engine();
    engine
        .import_json(
            &json!({ "type": "doc", "content": content }).to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    select_text(&mut engine, 70_030_000, 44, 52);
    hydrate_import_for_compile_test(&mut engine);

    let before_document = engine.document_json().unwrap();
    let before_selection = engine.resolved_selection().unwrap().clone();
    let mut expected_document = before_document.clone();
    let inline = expected_document["content"][1]["content"]
        .as_array_mut()
        .unwrap();
    inline.splice(
        0..1,
        [
            json!({ "type": "text", "text": "p" }),
            json!({
                "type": "text",
                "text": "p".repeat(8),
                "marks": [{ "type": "bold" }]
            }),
            json!({ "type": "text", "text": "p".repeat(46) }),
        ],
    );

    reset_localized_lookup_counts_for_test();
    reset_range_format_lowering_counts_for_test();
    let result = engine
        .apply_command(
            70_030_001,
            TypedCommand::ToggleMark {
                mark_type: "bold".into(),
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(engine.document_json().unwrap(), expected_document);
    assert_eq!(result.selection, before_selection);
    assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
    assert!(result.history_state.can_undo);
    assert!(!result.history_state.can_redo);
    assert!(engine.can_undo());
    assert!(!engine.can_redo());
    let range_format_counts = take_range_format_lowering_counts_for_test();
    let localized_lookup_counts = take_localized_lookup_counts_for_test();
    assert_eq!(localized_lookup_counts, (0, 0, 0));
    assert_eq!(range_format_counts, (0, 0, 1, 0));

    engine.undo(70_030_002).unwrap().unwrap();
    assert_eq!(engine.document_json().unwrap(), before_document);
    assert_eq!(engine.resolved_selection().unwrap(), &before_selection);
    assert!(!engine.can_undo());
    assert!(engine.can_redo());
}

#[test]
fn prepared_reverse_toggle_mark_matches_public_eager_transaction_result() {
    let document = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "a😀", "marks": [{ "type": "italic" }] },
                { "type": "text", "text": "bc" },
                { "type": "text", "text": "🦀d", "marks": [{ "type": "bold" }] },
                { "type": "text", "text": "ef" }
            ]
        }]
    });
    let populated = || {
        let mut engine = transaction_engine();
        engine
            .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        select_text(&mut engine, 70_030_100, 7, 1);
        engine
    };
    let command = TypedCommand::ToggleMark {
        mark_type: "bold".into(),
    };

    let mut prepared = populated();
    let prepared_result = prepared
        .apply_command(70_030_101, command.clone())
        .unwrap()
        .unwrap();

    let mut generic = populated();
    let CommandPlan::Transaction(transaction) = generic.plan_command(70_030_101, command).unwrap()
    else {
        panic!("reverse toggle-mark must produce a transaction")
    };
    let generic_result = generic
        .apply_typed_transaction_with_result(transaction)
        .unwrap();

    assert_eq!(prepared_result, generic_result);
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());
}

#[test]
fn toggle_mark_structural_ranges_reject_before_lowering_with_public_parity() {
    use crate::yrs_engine::mutation::{
        reset_range_format_lowering_counts_for_test, take_range_format_lowering_counts_for_test,
    };

    let cases = [
        (
            "crossBlock",
            json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
                ]
            }),
            0,
            5,
            (0, 0, 0, 0),
        ),
        (
            "inlineVoid",
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        { "type": "text", "text": "a" },
                        { "type": "hardBreak" },
                        { "type": "text", "text": "b" }
                    ]
                }]
            }),
            0,
            3,
            (1, 1, 0, 1),
        ),
    ];

    for (case, document, anchor, head, expected_counts) in cases {
        let populated = || {
            let mut engine = transaction_engine();
            engine
                .import_json(&document.to_string(), TransactionOrigin::DocumentImport)
                .unwrap();
            select_text(&mut engine, 70_030_200, anchor, head);
            engine
        };
        let command = TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        };

        let mut prepared = populated();
        let prepared_before = atomic_audit(&prepared);
        reset_range_format_lowering_counts_for_test();
        let prepared_error = prepared
            .apply_command(70_030_201, command.clone())
            .unwrap_err();
        assert_eq!(
            take_range_format_lowering_counts_for_test(),
            expected_counts,
            "{case}"
        );
        assert_eq!(atomic_audit(&prepared), prepared_before, "{case}");

        let mut generic = populated();
        let generic_before = atomic_audit(&generic);
        reset_range_format_lowering_counts_for_test();
        let plan = generic.plan_command(70_030_201, command);
        let generic_error = if case == "crossBlock" {
            let error = plan.unwrap_err();
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                (0, 0, 0, 0),
                "{case} public plan"
            );
            error
        } else {
            let CommandPlan::Transaction(transaction) = plan.unwrap() else {
                panic!("{case} must produce a public typed transaction")
            };
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                (0, 0, 0, 0),
                "{case} public plan"
            );
            reset_range_format_lowering_counts_for_test();
            let error = generic
                .apply_typed_transaction_with_result(transaction)
                .unwrap_err();
            assert_eq!(
                take_range_format_lowering_counts_for_test(),
                (1, 1, 0, 0),
                "{case} public apply"
            );
            error
        };
        assert_eq!(prepared_error, generic_error, "{case}");
        assert_eq!(atomic_audit(&generic), generic_before, "{case}");
    }
}

#[test]
fn prepared_toggle_mark_exact_limits_and_one_under_errors_match_public_eager() {
    use crate::yrs_engine::{OperationResult, TypedTransactionResult};

    fn fixture() -> YrsDocumentEngine {
        let mut engine = transaction_engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀bc🦀def"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        select_text(&mut engine, 70_030_300, 0, 8);
        engine
    }

    fn command() -> TypedCommand {
        TypedCommand::ToggleMark {
            mark_type: "bold".into(),
        }
    }

    fn public_eager_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        let CommandPlan::Transaction(transaction) = engine.plan_command(request_id, command())?
        else {
            panic!("range ToggleMark must produce a typed transaction")
        };
        engine.apply_typed_transaction_with_result(transaction)
    }

    fn prepared_apply(
        engine: &mut YrsDocumentEngine,
        request_id: u64,
    ) -> OperationResult<TypedTransactionResult> {
        Ok(engine
            .apply_command(request_id, command())?
            .expect("range ToggleMark must produce a transaction result"))
    }

    fn set_limit(engine: &mut YrsDocumentEngine, field: &str, value: u64) {
        match field {
            "maxUndoRetainedUnits" => {
                engine.editing_limits.max_undo_retained_units = value;
            }
            "maxInputBytes" => {
                engine.resource_limits.max_input_bytes = usize::try_from(value).unwrap();
            }
            "maxDerivedOutputBytes" => {
                engine.editing_limits.max_derived_output_bytes = usize::try_from(value).unwrap();
            }
            "maxEncodedStateBytes" => {
                engine.resource_limits.max_encoded_state_bytes = usize::try_from(value).unwrap();
            }
            _ => unreachable!(),
        }
    }

    fn exact_limit(field: &str) -> u64 {
        let mut limit = 0;
        loop {
            let mut probe = fixture();
            set_limit(&mut probe, field, limit);
            match public_eager_apply(&mut probe, 70_030_301) {
                Ok(_) => return limit,
                Err(error) => {
                    assert_eq!(error.details, Some(json!({ "field": field })), "{field}");
                    let actual = error.actual.expect("limit rejection must report actual");
                    assert!(actual > limit, "{field} probe must make progress");
                    limit = actual;
                }
            }
        }
    }

    let exact_limits = [
        ("maxUndoRetainedUnits", exact_limit("maxUndoRetainedUnits")),
        ("maxInputBytes", exact_limit("maxInputBytes")),
        (
            "maxDerivedOutputBytes",
            exact_limit("maxDerivedOutputBytes"),
        ),
    ];

    for (index, (field, exact)) in exact_limits.into_iter().enumerate() {
        let request_id = 70_030_310 + u64::try_from(index).unwrap();
        let mut prepared = fixture();
        set_limit(&mut prepared, field, exact);
        let prepared_result = prepared
            .apply_command(request_id, command())
            .unwrap()
            .unwrap();
        let mut generic = fixture();
        set_limit(&mut generic, field, exact);
        let generic_result = public_eager_apply(&mut generic, request_id).unwrap();
        assert_eq!(prepared_result, generic_result, "{field} exact");
        assert_eq!(
            prepared.document_json(),
            generic.document_json(),
            "{field} exact"
        );
        assert_eq!(
            prepared.document_html(),
            generic.document_html(),
            "{field} exact"
        );
        assert_eq!(
            prepared.resolved_selection(),
            generic.resolved_selection(),
            "{field} exact"
        );
        assert_eq!(
            prepared.stored_marks(),
            generic.stored_marks(),
            "{field} exact"
        );
        assert_eq!(prepared.can_undo(), generic.can_undo(), "{field} exact");
        assert_eq!(prepared.can_redo(), generic.can_redo(), "{field} exact");

        let limit = exact
            .checked_sub(1)
            .expect("ToggleMark limits must be nonzero");
        let mut rejected_prepared = fixture();
        set_limit(&mut rejected_prepared, field, limit);
        let prepared_before = atomic_audit(&rejected_prepared);
        let prepared_error = rejected_prepared
            .apply_command(request_id, command())
            .unwrap_err();
        assert_eq!(
            atomic_audit(&rejected_prepared),
            prepared_before,
            "{field} prepared"
        );

        let mut rejected_generic = fixture();
        set_limit(&mut rejected_generic, field, limit);
        let generic_before = atomic_audit(&rejected_generic);
        let generic_error = public_eager_apply(&mut rejected_generic, request_id).unwrap_err();
        assert_eq!(
            atomic_audit(&rejected_generic),
            generic_before,
            "{field} generic"
        );

        assert_eq!(prepared_error, generic_error, "{field}");
        assert_eq!(
            prepared_error.details,
            Some(json!({ "field": field })),
            "{field}"
        );
        assert_eq!(prepared_error.limit, Some(limit), "{field}");
        assert_eq!(prepared_error.actual, Some(exact), "{field}");
    }

    fn exercise_max_encoded_state_boundary(
        request_id: u64,
        apply: fn(&mut YrsDocumentEngine, u64) -> OperationResult<TypedTransactionResult>,
    ) -> (YrsDocumentEngine, TypedTransactionResult) {
        let field = "maxEncodedStateBytes";
        let mut engine = fixture();
        let before = atomic_audit(&engine);
        let current_encoded = u64::try_from(engine.encoded_state().unwrap().len()).unwrap();
        set_limit(&mut engine, field, current_encoded);
        let probe_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(atomic_audit(&engine), before, "{field} probe");
        assert_eq!(probe_error.details, Some(json!({ "field": field })));
        let exact = probe_error
            .actual
            .expect("encoded-state rejection must report the exact instance size");
        let one_under = exact
            .checked_sub(1)
            .expect("encoded state must consume at least one byte");

        set_limit(&mut engine, field, one_under);
        let one_under_error = apply(&mut engine, request_id).unwrap_err();
        assert_eq!(atomic_audit(&engine), before, "{field} one-under");
        assert_eq!(one_under_error.details, Some(json!({ "field": field })));
        assert_eq!(one_under_error.limit, Some(one_under));
        assert_eq!(one_under_error.actual, Some(exact));

        set_limit(&mut engine, field, exact);
        let result = apply(&mut engine, request_id).unwrap();
        assert!(engine.encoded_state().unwrap().len() <= usize::try_from(exact).unwrap());
        (engine, result)
    }

    let request_id = 70_030_320;
    let (prepared, prepared_result) =
        exercise_max_encoded_state_boundary(request_id, prepared_apply);
    let (generic, generic_result) =
        exercise_max_encoded_state_boundary(request_id, public_eager_apply);
    assert_eq!(
        prepared_result, generic_result,
        "maxEncodedStateBytes exact"
    );
    assert_eq!(prepared.document_json(), generic.document_json());
    assert_eq!(prepared.document_html(), generic.document_html());
    assert_eq!(prepared.resolved_selection(), generic.resolved_selection());
    assert_eq!(prepared.stored_marks(), generic.stored_marks());
    assert_eq!(prepared.can_undo(), generic.can_undo());
    assert_eq!(prepared.can_redo(), generic.can_redo());
}
