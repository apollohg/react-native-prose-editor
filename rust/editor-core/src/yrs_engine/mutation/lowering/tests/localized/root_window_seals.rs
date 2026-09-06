#[test]
fn localized_root_window_rejects_exact_stale_seal_matrix() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let foreign_document =
        from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let replacement_content = Fragment::from(vec![document
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
        replacement_content,
        crate::selection::Selection::cursor(0),
    );
    let doc = seeded_document(&source, &schema, &limits);
    {
        let mut txn = doc.transact_mut();
        txn.get_or_insert_xml_fragment("alternate");
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let alternate_fragment = txn.get_xml_fragment("alternate").unwrap();
    let foreign_doc = seeded_document(&source, &schema, &limits);
    let foreign_txn = foreign_doc.transact();
    let foreign_fragment = foreign_txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        726,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        4,
        8,
    )
    .unwrap();
    let mut resource_drift = limits.clone();
    resource_drift.max_input_bytes -= 1;
    let mut editing_drift = editing_limits.clone();
    editing_drift.max_operations_per_transaction -= 1;
    let mint = |semantic,
                txn,
                fragment,
                resource_limits,
                editing_limits,
                max_length,
                fingerprint,
                epoch,
                revision| {
        LocalizedRootWindowLocator::mint(
            726,
            semantic,
            semantic,
            &replacement,
            &seed,
            txn,
            fragment,
            resource_limits,
            editing_limits,
            max_length,
            fingerprint,
            epoch,
            revision,
        )
        .unwrap()
        .is_none()
    };

    for (case, rejected) in [
        (
            "semanticRoot",
            mint(
                &foreign_document,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                4,
                8,
            ),
        ),
        (
            "store",
            mint(
                &document,
                &foreign_txn,
                &foreign_fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                4,
                8,
            ),
        ),
        (
            "fragment",
            mint(
                &document,
                &txn,
                &alternate_fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                4,
                8,
            ),
        ),
        (
            "schemaFingerprint",
            mint(
                &document,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-b",
                4,
                8,
            ),
        ),
        (
            "epoch",
            mint(
                &document,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                5,
                8,
            ),
        ),
        (
            "revision",
            mint(
                &document,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                None,
                "schema-a",
                4,
                9,
            ),
        ),
        (
            "resourceLimits",
            mint(
                &document,
                &txn,
                &fragment,
                &resource_drift,
                &editing_limits,
                None,
                "schema-a",
                4,
                8,
            ),
        ),
        (
            "editingLimits",
            mint(
                &document,
                &txn,
                &fragment,
                &limits,
                &editing_drift,
                None,
                "schema-a",
                4,
                8,
            ),
        ),
        (
            "maxLength",
            mint(
                &document,
                &txn,
                &fragment,
                &limits,
                &editing_limits,
                Some(1),
                "schema-a",
                4,
                8,
            ),
        ),
    ] {
        assert!(rejected, "{case} must signal the eager fallback boundary");
    }

    for (case, mismatch_txn, mismatch_fragment) in [
        ("store", &foreign_txn, &foreign_fragment),
        ("fragment", &txn, &alternate_fragment),
    ] {
        let locator = LocalizedRootWindowLocator::mint(
            726,
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
            4,
            8,
        )
        .unwrap()
        .unwrap();
        assert!(
            LocalizedRootWindowCompiler::try_new(
                726,
                mismatch_txn,
                mismatch_fragment,
                &schema,
                100_000,
                100_000,
                0,
                locator,
            )
            .unwrap()
            .is_none(),
            "{case} try_new must signal the eager fallback boundary"
        );
    }
}

#[test]
fn localized_root_window_fails_closed_for_invalid_shapes_and_shallow_drift() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let replacement_content = Fragment::from(vec![document
        .root()
        .content()
        .unwrap()
        .child(0)
        .unwrap()
        .clone()]);
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        721,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        4,
        8,
    )
    .unwrap();
    for (case, parent_path, from_child, to_child) in [
        ("nonroot", vec![0], 0, 1),
        ("empty", vec![], 0, 0),
        ("outOfBounds", vec![], 0, 2),
    ] {
        let replacement = crate::yrs_engine::StructuralReplacement::new(
            parent_path,
            from_child,
            to_child,
            replacement_content.clone(),
            crate::selection::Selection::cursor(0),
        );
        assert!(
            LocalizedRootWindowLocator::mint(
                721,
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
                4,
                8,
            )
            .unwrap()
            .is_none(),
            "{case}"
        );
    }

    let assert_alignment_fallback = |case: &str, semantic: Document, wire: Value| {
        let wire_doc = seeded_document(&wire, &schema, &limits);
        let wire_txn = wire_doc.transact();
        let wire_fragment = wire_txn.get_xml_fragment("prosemirror").unwrap();
        let wire_seed = MutationLookupSeed::build(
            722,
            &wire_txn,
            &wire_fragment,
            &schema,
            &semantic,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            5,
            9,
        )
        .unwrap();
        let content = Fragment::from(vec![semantic
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
        let locator = LocalizedRootWindowLocator::mint(
            722,
            &semantic,
            &semantic,
            &replacement,
            &wire_seed,
            &wire_txn,
            &wire_fragment,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            5,
            9,
        )
        .unwrap()
        .expect("shape and exact seed context should mint before alignment");
        assert!(
            LocalizedRootWindowCompiler::try_new(
                722,
                &wire_txn,
                &wire_fragment,
                &schema,
                100_000,
                100_000,
                0,
                locator,
            )
            .unwrap()
            .is_none(),
            "{case}"
        );
    };
    let heading = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{ "type": "h1", "content": [{ "type": "text", "text": "a" }] }]
        }),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    assert_alignment_fallback(
        "normalizedType",
        heading.clone(),
        json!({
            "type": "doc",
            "content": [{ "type": "h2", "content": [{ "type": "text", "text": "a" }] }]
        }),
    );
    assert_alignment_fallback(
        "normalizedAttrs",
        heading,
        json!({
            "type": "doc",
            "content": [{
                "type": "h1",
                "attrs": { "id": "foreign" },
                "content": [{ "type": "text", "text": "a" }]
            }]
        }),
    );
    assert_alignment_fallback(
        "cardinality",
        from_prosemirror_json(
            &json!({
                "type": "doc",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "a" }] },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] }
                ]
            }),
            &schema,
            UnknownTypeMode::Preserve,
        )
        .unwrap(),
        source.clone(),
    );
    let hostile_void = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "horizontalRule".into(),
            HashMap::new(),
            Fragment::empty(),
        )]),
    ));
    assert_alignment_fallback(
        "void",
        hostile_void,
        json!({ "type": "doc", "content": [{ "type": "horizontalRule" }] }),
    );

    let replacement = crate::yrs_engine::StructuralReplacement::new(
        Vec::new(),
        0,
        1,
        replacement_content,
        crate::selection::Selection::cursor(0),
    );
    let mut stale_width_seed = seed.clone();
    let MutationLookupSeedState::Ready(payload) = &mut stale_width_seed.state else {
        panic!("freshly built lookup seed must be ready")
    };
    payload.path_parent_widths = Arc::new(HashMap::from([(
        AsRef::<Branch>::as_ref(&fragment).id(),
        2,
    )]));
    let locator = LocalizedRootWindowLocator::mint(
        721,
        &document,
        &document,
        &replacement,
        &stale_width_seed,
        &txn,
        &fragment,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        4,
        8,
    )
    .unwrap()
    .unwrap();
    assert!(
        LocalizedRootWindowCompiler::try_new(
            721, &txn, &fragment, &schema, 100_000, 100_000, 0, locator,
        )
        .unwrap()
        .is_none(),
        "rootWidth"
    );
}
