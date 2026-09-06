fn replacement_blocks(engine: &YrsDocumentEngine) -> Vec<(String, String)> {
    engine
        .document()
        .unwrap()
        .root()
        .content()
        .unwrap()
        .iter()
        .map(|node| (node.node_type().to_string(), node.text_content()))
        .collect()
}

#[test]
fn plain_multiline_replacement_is_one_unicode_cross_block_transaction_and_undo() {
    let mut engine = transaction_engine();
    engine.import_json(
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a😀b"}]},{"type":"paragraph","content":[{"type":"text","text":"tail"}]}]}"#,
        TransactionOrigin::DocumentImport,
    ).unwrap();
    let before = replacement_blocks(&engine);
    select_text(&mut engine, 901, 1, 5);
    let revision = engine.revision();
    let CommandPlan::Transaction(transaction) = engine
        .plan_command(
            902,
            TypedCommand::ReplaceSelectionText {
                text: "文\n字".into(),
            },
        )
        .unwrap()
    else {
        panic!("replacement must plan")
    };
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(
        replacement_blocks(&engine),
        vec![
            ("paragraph".into(), "a文".into()),
            ("paragraph".into(), "字ail".into())
        ]
    );
    assert_eq!(engine.revision(), revision + 1);
    engine.undo_with_result(903).unwrap().unwrap();
    assert_eq!(replacement_blocks(&engine), before);
    engine.redo_with_result(904).unwrap().unwrap();
    assert_eq!(
        replacement_blocks(&engine),
        vec![
            ("paragraph".into(), "a文".into()),
            ("paragraph".into(), "字ail".into())
        ]
    );
}

#[test]
fn plain_multiline_replacement_normalizes_crlf_and_preserves_empty_lines() {
    let mut engine = transaction_engine();
    engine.import_json(r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"left right"}]}]}"#, TransactionOrigin::DocumentImport).unwrap();
    select_text(&mut engine, 911, 5, 5);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command(
            912,
            TypedCommand::ReplaceSelectionText {
                text: "one\r\n\r\ntwo\r".into(),
            },
        )
        .unwrap()
    else {
        panic!("replacement must plan")
    };
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(
        replacement_blocks(&engine),
        vec![
            ("paragraph".into(), "left one".into()),
            ("paragraph".into(), "".into()),
            ("paragraph".into(), "two".into()),
            ("paragraph".into(), "right".into())
        ]
    );
}

#[test]
fn plain_multiline_replacement_in_code_keeps_literal_newlines_and_language() {
    let mut engine = transaction_engine();
    engine.import_json(r#"{"type":"doc","content":[{"type":"codeBlock","attrs":{"language":"swift"},"content":[{"type":"text","text":"a😀b"}]}]}"#, TransactionOrigin::DocumentImport).unwrap();
    select_text(&mut engine, 921, 1, 2);
    let CommandPlan::Transaction(transaction) = engine
        .plan_command(
            922,
            TypedCommand::ReplaceSelectionText {
                text: "文\n字".into(),
            },
        )
        .unwrap()
    else {
        panic!("replacement must plan")
    };
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(
        replacement_blocks(&engine),
        vec![("codeBlock".into(), "a文\n字b".into())]
    );
    assert_eq!(
        engine
            .document()
            .unwrap()
            .root()
            .content()
            .unwrap()
            .child(0)
            .unwrap()
            .attrs()
            .get("language"),
        Some(&serde_json::json!("swift"))
    );
}

#[test]
fn plain_multiline_replacement_rejects_operation_budget_before_mutation() {
    let mut engine = transaction_engine_with_editing_limits(crate::yrs_engine::EditingLimits {
        max_operations_per_transaction: 3,
        ..crate::yrs_engine::EditingLimits::default()
    });
    engine.import_json(r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"keep"}]}]}"#, TransactionOrigin::DocumentImport).unwrap();
    select_text(&mut engine, 931, 0, 4);
    let revision = engine.revision();
    let error = engine
        .plan_command(
            932,
            TypedCommand::ReplaceSelectionText {
                text: "a\nb\nc".into(),
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(engine.revision(), revision);
    assert_eq!(
        replacement_blocks(&engine),
        vec![("paragraph".into(), "keep".into())]
    );
}
