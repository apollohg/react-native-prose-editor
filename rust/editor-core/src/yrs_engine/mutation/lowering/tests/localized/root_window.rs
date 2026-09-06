#[test]
fn localized_root_window_matches_eager_structural_plan_and_work() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }
        ]
    });
    let expected = json!({
        "type": "doc",
        "content": [
            { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
                }]
            }
        ]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let preview = from_prosemirror_json(&expected, &schema, UnknownTypeMode::Preserve).unwrap();
    let replacement_content = Fragment::from(vec![preview
        .root()
        .content()
        .unwrap()
        .child(1)
        .unwrap()
        .clone()]);
    let replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        1,
        2,
        replacement_content.clone(),
        crate::selection::Selection::cursor(0),
    );
    let from = document
        .root()
        .content()
        .unwrap()
        .child(0)
        .unwrap()
        .node_size();
    let to = from
        + document
            .root()
            .content()
            .unwrap()
            .child(1)
            .unwrap()
            .node_size();
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        720,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        3,
        7,
    )
    .unwrap();
    fn charge_eager_boundaries(node: &Node, eager: &mut MutationCompiler) -> OperationResult<()> {
        eager.charge_boundary_node(0)?;
        if let Some(text) = node.text_str() {
            eager.charge_boundary_text(0, text.len())?;
        }
        if let Some(content) = node.content() {
            for child in content.iter() {
                charge_eager_boundaries(child, eager)?;
            }
        }
        Ok(())
    }
    fn charge_localized_boundaries(
        node: &Node,
        localized: &mut LocalizedRootWindowCompiler,
    ) -> OperationResult<()> {
        localized.charge_boundary_node(0)?;
        if let Some(text) = node.text_str() {
            localized.charge_boundary_text(0, text.len())?;
        }
        if let Some(content) = node.content() {
            for child in content.iter() {
                charge_localized_boundaries(child, localized)?;
            }
        }
        Ok(())
    }
    let compile_eager = |action_limit, scan_limit| {
        let mut compiler =
            MutationCompiler::new(720, &txn, &fragment, &schema, action_limit, scan_limit, 19)?;
        charge_eager_boundaries(document.root(), &mut compiler)?;
        compiler.replace_structural_range(
            0,
            MutationDocumentContext {
                before: &document,
                after: &preview,
                schema: &schema,
                limits: &limits,
            },
            ReplacementInput {
                from,
                to,
                boundaries: &[],
                content: &replacement_content,
            },
        )?;
        compiler.finish(Some(0))
    };
    let compile_localized = |action_limit, scan_limit| {
        let locator = LocalizedRootWindowLocator::mint(
            720,
            &document,
            &preview,
            &replacement,
            &seed,
            &txn,
            &fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            3,
            7,
        )?
        .expect("exact prepared root window must mint");
        let mut compiler = LocalizedRootWindowCompiler::try_new(
            720,
            &txn,
            &fragment,
            &schema,
            action_limit,
            scan_limit,
            19,
            locator,
        )?
        .expect("aligned root must localize");
        charge_localized_boundaries(document.root(), &mut compiler)?;
        compiler.replace_structural_range(
            0,
            MutationDocumentContext {
                before: &document,
                after: &preview,
                schema: &schema,
                limits: &limits,
            },
            ReplacementInput {
                from,
                to,
                boundaries: &[],
                content: &replacement_content,
            },
        )
    };

    let baseline = compile_eager(100_000, 100_000).unwrap();
    let exact_compilation = baseline.compilation_work_for_test();
    let exact_preflight = baseline.expected_preflight_work_for_test();
    let exact_actions = exact_compilation
        .checked_add(exact_preflight)
        .expect("root-window action work must fit usize");
    let exact_scan = baseline.scan_work;
    assert!(exact_actions > 0);
    assert!(exact_scan > 0);
    let eager = compile_eager(exact_actions, exact_scan).unwrap();
    let localized = compile_localized(exact_actions, exact_scan).unwrap();
    preflight_mutation_plan(720, &eager, &txn).unwrap();
    preflight_mutation_plan(720, &localized, &txn).unwrap();

    assert_eq!(eager.actions.len(), 2);
    assert_eq!(localized.actions.len(), 2);
    for (eager, localized) in eager.actions.iter().zip(&localized.actions) {
        match (eager, localized) {
            (
                YrsMutationAction::DeleteXmlChildren {
                    child_index: ei,
                    child_count: ec,
                    signature: es,
                    operation_index: eo,
                    ..
                },
                YrsMutationAction::DeleteXmlChildren {
                    child_index: li,
                    child_count: lc,
                    signature: ls,
                    operation_index: lo,
                    ..
                },
            ) => assert_eq!((ei, ec, es, eo), (li, lc, ls, lo)),
            (
                YrsMutationAction::InsertXmlChildren {
                    child_index: ei,
                    nodes: en,
                    signature: es,
                    operation_index: eo,
                    ..
                },
                YrsMutationAction::InsertXmlChildren {
                    child_index: li,
                    nodes: ln,
                    signature: ls,
                    operation_index: lo,
                    ..
                },
            ) => {
                assert_eq!((ei, es, eo), (li, ls, lo));
                assert_eq!(format!("{en:?}"), format!("{ln:?}"));
            }
            _ => panic!("eager/localized structural action kinds differ"),
        }
    }
    assert_eq!(
        eager.compilation_work_for_test(),
        localized.compilation_work_for_test()
    );
    assert_eq!(
        eager.expected_preflight_work_for_test(),
        localized.expected_preflight_work_for_test()
    );
    assert_eq!(eager.scan_work, localized.scan_work);
    assert_eq!(eager.compilation_work_for_test(), exact_compilation);
    assert_eq!(eager.expected_preflight_work_for_test(), exact_preflight);
    assert_eq!(
        eager.compilation_work_for_test() + eager.expected_preflight_work_for_test(),
        exact_actions
    );
    assert_eq!(eager.scan_work, exact_scan);

    let action_limit = exact_actions - 1;
    let eager = compile_eager(action_limit, exact_scan).unwrap();
    let localized = compile_localized(action_limit, exact_scan).unwrap();
    assert_eq!(
        preflight_mutation_plan(720, &eager, &txn).unwrap_err(),
        preflight_mutation_plan(720, &localized, &txn).unwrap_err()
    );

    assert_eq!(
        compile_eager(exact_actions, exact_scan - 1).unwrap_err(),
        compile_localized(exact_actions, exact_scan - 1).unwrap_err()
    );
}

#[test]
fn localized_root_window_rejects_wrong_replacement_content_with_attribution() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }
        ]
    });
    let expected = json!({
        "type": "doc",
        "content": [
            { "type": "h1", "content": [{ "type": "text", "text": "title" }] },
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "abc" }] }]
                }]
            }
        ]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let preview = from_prosemirror_json(&expected, &schema, UnknownTypeMode::Preserve).unwrap();
    let expected_content = Fragment::from(vec![preview
        .root()
        .content()
        .unwrap()
        .child(1)
        .unwrap()
        .clone()]);
    let wrong_content = Fragment::from(vec![document
        .root()
        .content()
        .unwrap()
        .child(0)
        .unwrap()
        .clone()]);
    let replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        1,
        2,
        expected_content,
        crate::selection::Selection::cursor(0),
    );
    let from = document
        .root()
        .content()
        .unwrap()
        .child(0)
        .unwrap()
        .node_size();
    let to = from
        + document
            .root()
            .content()
            .unwrap()
            .child(1)
            .unwrap()
            .node_size();
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        723,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        6,
        10,
    )
    .unwrap();
    let locator = LocalizedRootWindowLocator::mint(
        723,
        &document,
        &preview,
        &replacement,
        &seed,
        &txn,
        &fragment,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        6,
        10,
    )
    .unwrap()
    .unwrap();
    let compiler = LocalizedRootWindowCompiler::try_new(
        723, &txn, &fragment, &schema, 100_000, 100_000, 0, locator,
    )
    .unwrap()
    .unwrap();

    let error = compiler
        .replace_structural_range(
            4,
            MutationDocumentContext {
                before: &document,
                after: &preview,
                schema: &schema,
                limits: &limits,
            },
            ReplacementInput {
                from,
                to,
                boundaries: &[],
                content: &wrong_content,
            },
        )
        .unwrap_err();

    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.request_id, 723);
    assert_eq!(error.operation_index, Some(4));
    assert!(error.message.contains("content"));
}

#[test]
fn localized_root_window_streams_normalized_attrs_without_map_materialization() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{ "type": "h1", "content": [{ "type": "text", "text": "title" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let content = Fragment::from(vec![document
        .root()
        .content()
        .unwrap()
        .child(0)
        .unwrap()
        .clone()]);
    let replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        0,
        1,
        content,
        crate::selection::Selection::cursor(0),
    );
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        724,
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
    let locator = LocalizedRootWindowLocator::mint(
        724,
        &document,
        &document,
        &replacement,
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
    .unwrap()
    .unwrap();

    reset_localized_root_attr_map_builds_for_test();
    assert!(LocalizedRootWindowCompiler::try_new(
        724, &txn, &fragment, &schema, 100_000, 100_000, 0, locator,
    )
    .unwrap()
    .is_some());
    assert_eq!(take_localized_root_attr_map_builds_for_test(), 0);
}
