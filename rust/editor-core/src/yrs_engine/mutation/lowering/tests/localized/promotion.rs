#[test]
fn promoted_marked_fragmented_non_bmp_insert_preserves_second_insert_exact_work_and_errors() {
    use crate::transform::{apply_step_canonical_marks, Step};
    use crate::yrs_engine::mutation::execute_mutation_plan;

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let bold = Mark::new("bold".into(), HashMap::new());
    let italic = Mark::new("italic".into(), HashMap::new());
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [
                { "type": "text", "text": "a", "marks": [{ "type": "bold" }] },
                { "type": "text", "text": "😀", "marks": [{ "type": "italic" }] },
                { "type": "text", "text": "b", "marks": [{ "type": "bold" }] }
            ]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let first_position = block.doc_start;
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        709,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        3,
        2,
    )
    .unwrap();
    let (first_plan, promotion) = LocalizedInsertCompiler::try_new(
        709,
        &txn,
        &fragment,
        &schema,
        100_000,
        100_000,
        11,
        LocalizedInsertLocator {
            document: &document,
            block_path: block.node_path.as_slice(),
            position: first_position,
        },
        &seed,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        3,
        2,
    )
    .unwrap()
    .expect("fragmented marked text must localize")
    .compile_with_promotion(0, first_position, "🦀", std::slice::from_ref(&bold))
    .unwrap();
    preflight_mutation_plan(709, &first_plan, &txn).unwrap();
    drop(txn);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(first_plan, &mut txn);
    }
    let (after, _) = apply_step_canonical_marks(
        &document,
        &Step::InsertText {
            pos: first_position,
            text: "🦀".into(),
            marks: vec![bold],
        },
        &schema,
    )
    .unwrap();
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let promoted = seed
        .prepare_promotion(
            &txn,
            &fragment,
            &promotion,
            &document,
            &after,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            3,
            2,
            4,
            3,
        )
        .unwrap();
    let second_block = PositionMap::build(&after, &schema)
        .block(0)
        .unwrap()
        .clone();
    let second_position = first_position + 1;

    let compile_eager = |action_limit, scan_limit| {
        let mut compiler =
            MutationCompiler::new(710, &txn, &fragment, &schema, action_limit, scan_limit, 11)?;
        compiler.insert(0, second_position, "界", std::slice::from_ref(&italic))?;
        compiler.finish(Some(0))
    };
    let compile_localized = |action_limit, scan_limit| {
        LocalizedInsertCompiler::try_new(
            710,
            &txn,
            &fragment,
            &schema,
            action_limit,
            scan_limit,
            11,
            LocalizedInsertLocator {
                document: &after,
                block_path: second_block.node_path.as_slice(),
                position: second_position,
            },
            &promoted,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            4,
            3,
        )?
        .expect("promoted fragmented marked text must localize again")
        .compile(0, second_position, "界", std::slice::from_ref(&italic))
    };

    let eager = compile_eager(100_000, 100_000).unwrap();
    let exact_actions = eager.compilation_work_for_test();
    let exact_input = eager.scan_work;
    let localized = compile_localized(exact_actions, exact_input).unwrap();
    assert_insert_plans_equal(&eager, &localized);
    for (action_limit, scan_limit) in [
        (exact_actions - 1, exact_input),
        (exact_actions, exact_input - 1),
    ] {
        assert_eq!(
            compile_eager(action_limit, scan_limit).unwrap_err(),
            compile_localized(action_limit, scan_limit).unwrap_err()
        );
    }
}

#[test]
fn promoted_insert_materialization_work_admits_chained_localized_format() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let bold = Mark::new("bold".into(), HashMap::new());
    let italic = Mark::new("italic".into(), HashMap::new());
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abc" }]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let insert_position = block.doc_start + 1;
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        711,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        5,
        8,
    )
    .unwrap();
    let (insert_plan, promotion) = LocalizedInsertCompiler::try_new(
        711,
        &txn,
        &fragment,
        &schema,
        100_000,
        100_000,
        13,
        LocalizedInsertLocator {
            document: &document,
            block_path: block.node_path.as_slice(),
            position: insert_position,
        },
        &seed,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        5,
        8,
    )
    .unwrap()
    .expect("existing insert must localize")
    .compile_with_promotion(0, insert_position, "X", std::slice::from_ref(&bold))
    .unwrap();
    drop(txn);
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(insert_plan, &mut txn);
    }
    let (after, _) = apply_step_canonical_marks(
        &document,
        &Step::InsertText {
            pos: insert_position,
            text: "X".into(),
            marks: vec![bold],
        },
        &schema,
    )
    .unwrap();
    let after_block = PositionMap::build(&after, &schema)
        .block(0)
        .unwrap()
        .clone();
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let promoted = seed
        .prepare_promotion(
            &txn,
            &fragment,
            &promotion,
            &document,
            &after,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            5,
            8,
            6,
            9,
        )
        .unwrap();
    let from = after_block.doc_start;
    let to = after_block.doc_end;
    let boundaries = [from, insert_position, insert_position + 1, to];

    let mut eager =
        MutationCompiler::new(712, &txn, &fragment, &schema, 100_000, 100_000, 13).unwrap();
    eager
        .format(0, from, to, &boundaries, mark_attr(&italic))
        .unwrap();
    let eager = eager.finish(Some(0)).unwrap();
    let locator = LocalizedFormatLocator::mint(
        &after,
        after_block.node_path.as_slice(),
        from,
        to,
        &promoted,
        &txn,
        &fragment,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        6,
        9,
    )
    .expect("the promoted insert seed must mint an exact format locator");
    let localized = LocalizedFormatCompiler::try_new(
        712, &txn, &fragment, &schema, 100_000, 100_000, 13, locator, "schema-a", 6, 9,
    )
    .unwrap()
    .expect("the promoted insert seed must admit a chained localized format")
    .format(0, from, to, &boundaries, mark_attr(&italic))
    .unwrap()
    .0;

    assert_plans_equal(&eager, &localized);
}

#[test]
fn localized_format_promotion_derives_current_work_in_one_target_pass() {
    use yrs::types::xml::XmlTextPrelim;

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let text = "abcdefghijklmnop";
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": text }]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let doc = seeded_document(&source, &schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph must be an XML element")
        };
        paragraph.remove_range(&mut txn, 0, 1);
        for scalar in text.chars() {
            paragraph.push_back(&mut txn, XmlTextPrelim::new(scalar.to_string()));
        }
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        713,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        7,
        11,
    )
    .unwrap();
    let from = block.doc_start;
    let to = block.doc_end;
    let locator = LocalizedFormatLocator::mint(
        &document,
        block.node_path.as_slice(),
        from,
        to,
        &seed,
        &txn,
        &fragment,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        7,
        11,
    )
    .expect("exact multi-leaf context must mint a format locator");
    let localized = LocalizedFormatCompiler::try_new(
        713, &txn, &fragment, &schema, 100_000, 100_000, 0, locator, "schema-a", 7, 11,
    )
    .unwrap()
    .expect("multi-leaf textblock must localize");

    reset_localized_format_promotion_target_visits_for_test();
    let (plan, _) = localized
        .format(
            0,
            from,
            to,
            &[from, to],
            mark_attr(&Mark::new("bold".into(), HashMap::new())),
        )
        .unwrap();
    let visits = take_localized_format_promotion_target_visits_for_test();

    assert_eq!(plan.actions.len(), text.chars().count());
    assert_eq!(visits, plan.actions.len());
}
