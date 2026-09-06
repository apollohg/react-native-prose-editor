#[test]
fn localized_format_add_remove_four_leaf_unicode_fragmented_parity() {
    use yrs::types::xml::XmlTextPrelim;

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a😀bc🦀def" }]
        }]
    });
    let doc = seeded_document(&source, &schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph must be an XML element")
        };
        paragraph.remove_range(&mut txn, 0, 1);
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new("a😀"));
        let second = paragraph.push_back(&mut txn, XmlTextPrelim::new("bc"));
        let third = paragraph.push_back(&mut txn, XmlTextPrelim::new("🦀d"));
        let fourth = paragraph.push_back(&mut txn, XmlTextPrelim::new("ef"));
        first.format(
            &mut txn,
            0,
            1,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
        second.format(
            &mut txn,
            0,
            1,
            Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
        );
        third.format(
            &mut txn,
            0,
            2,
            Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
        );
        fourth.format(
            &mut txn,
            0,
            1,
            Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
        );
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let semantic_json = codec.read_json(&fragment, &txn).unwrap();
    let document =
        from_prosemirror_json(&semantic_json, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let seed = MutationLookupSeed::build(
        716,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        12,
        18,
    )
    .unwrap();
    let from = block.doc_start;
    let to = block.doc_end;
    let boundaries = (from..=to).collect::<Vec<_>>();
    let bold = Mark::new("bold".into(), HashMap::new());

    for (case, attrs) in [
        ("add", mark_attr(&bold)),
        ("remove", removed_mark_attr("italic")),
    ] {
        let mut eager =
            MutationCompiler::new(716, &txn, &fragment, &schema, 100_000, 100_000, 23).unwrap();
        eager
            .format(0, from, to, &boundaries, attrs.clone())
            .unwrap();
        let eager = eager.finish(Some(0)).unwrap();

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
            12,
            18,
        )
        .expect("exact four-leaf format context must mint a locator");
        let localized = LocalizedFormatCompiler::try_new(
            716, &txn, &fragment, &schema, 100_000, 100_000, 23, locator, "schema-a", 12, 18,
        )
        .unwrap()
        .expect("four-leaf format must localize")
        .format(0, from, to, &boundaries, attrs)
        .unwrap()
        .0;

        preflight_mutation_plan(716, &eager, &txn).unwrap();
        preflight_mutation_plan(716, &localized, &txn).unwrap();
        assert_plans_equal(&eager, &localized);
        let branches = localized
            .actions
            .iter()
            .filter_map(|action| match action {
                YrsMutationAction::FormatText { signature, .. } => Some(signature.target.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        assert_eq!(branches.len(), 4, "{case}");
    }
}

#[test]
fn localized_format_four_leaf_unicode_exact_action_and_scan_limits_match_eager() {
    use yrs::types::xml::XmlTextPrelim;

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "a😀bc🦀def" }]
        }]
    });
    let doc = seeded_document(&source, &schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(paragraph) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph must be an XML element")
        };
        paragraph.remove_range(&mut txn, 0, 1);
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new("a😀"));
        let second = paragraph.push_back(&mut txn, XmlTextPrelim::new("bc"));
        let third = paragraph.push_back(&mut txn, XmlTextPrelim::new("🦀d"));
        let fourth = paragraph.push_back(&mut txn, XmlTextPrelim::new("ef"));
        for text in [&first, &third] {
            text.format(
                &mut txn,
                0,
                1,
                Attrs::from([(Arc::<str>::from("bold"), Any::Bool(true))]),
            );
        }
        for text in [&second, &fourth] {
            text.format(
                &mut txn,
                0,
                1,
                Attrs::from([(Arc::<str>::from("italic"), Any::Bool(true))]),
            );
        }
    }
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let semantic_json = codec.read_json(&fragment, &txn).unwrap();
    let document =
        from_prosemirror_json(&semantic_json, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let seed = MutationLookupSeed::build(
        719,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        14,
        20,
    )
    .unwrap();
    let from = block.doc_start;
    let to = block.doc_end;
    let boundaries = (from..=to).collect::<Vec<_>>();
    let attrs = mark_attr(&Mark::new("bold".into(), HashMap::new()));
    let compile_eager = |action_limit, scan_limit| {
        let mut compiler =
            MutationCompiler::new(719, &txn, &fragment, &schema, action_limit, scan_limit, 23)?;
        compiler.format(0, from, to, &boundaries, attrs.clone())?;
        compiler.finish(Some(0))
    };
    let compile_localized = |action_limit, scan_limit| {
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
            14,
            20,
        )
        .expect("exact four-leaf format context must mint a locator");
        LocalizedFormatCompiler::try_new(
            719,
            &txn,
            &fragment,
            &schema,
            action_limit,
            scan_limit,
            23,
            locator,
            "schema-a",
            14,
            20,
        )?
        .expect("four-leaf format must localize")
        .format(0, from, to, &boundaries, attrs.clone())
        .map(|(plan, _)| plan)
    };

    let eager = compile_eager(100_000, 100_000).unwrap();
    let exact_actions = eager.compilation_work_for_test();
    let exact_scan = eager.scan_work;
    let localized = compile_localized(exact_actions, exact_scan).unwrap();
    assert_plans_equal(&eager, &localized);
    for (action_limit, scan_limit) in [
        (exact_actions - 1, exact_scan),
        (exact_actions, exact_scan - 1),
    ] {
        assert_eq!(
            compile_eager(action_limit, scan_limit).unwrap_err(),
            compile_localized(action_limit, scan_limit).unwrap_err()
        );
    }
}

#[test]
fn localized_format_rejects_foreign_semantic_root_with_identical_selected_block() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "source" }] }
        ]
    });
    let foreign_source = json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "same" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "foreign" }] }
        ]
    });
    let source_document =
        from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let foreign_document =
        from_prosemirror_json(&foreign_source, &schema, UnknownTypeMode::Preserve).unwrap();
    let block = PositionMap::build(&source_document, &schema)
        .block(0)
        .unwrap()
        .clone();
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        714,
        &txn,
        &fragment,
        &schema,
        &source_document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        5,
        9,
    )
    .unwrap();

    let localized = LocalizedFormatLocator::mint(
        &foreign_document,
        block.node_path.as_slice(),
        block.doc_start,
        block.doc_end,
        &seed,
        &txn,
        &fragment,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        5,
        9,
    );

    assert!(localized.is_none());
}

#[test]
fn localized_format_seal_rejects_stale_storage_schema_epoch_and_revision() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
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
    let doc = seeded_document(&source, &schema, &limits);
    {
        let mut txn = doc.transact_mut();
        txn.get_or_insert_xml_fragment("alternate");
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let alternate_fragment = txn.get_xml_fragment("alternate").unwrap();
    let seed = MutationLookupSeed::build(
        717,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        4,
        9,
    )
    .unwrap();
    let mint = |txn: &yrs::Transaction<'_>,
                fragment: &XmlFragmentRef,
                fingerprint: &str,
                epoch,
                revision| {
        LocalizedFormatLocator::mint(
            &document,
            block.node_path.as_slice(),
            block.doc_start,
            block.doc_end,
            &seed,
            txn,
            fragment,
            &limits,
            &editing_limits,
            None,
            fingerprint,
            epoch,
            revision,
        )
    };
    let locator =
        mint(&txn, &fragment, "schema-a", 4, 9).expect("exact format context must mint a locator");
    assert!(LocalizedFormatCompiler::try_new(
        717, &txn, &fragment, &schema, 100_000, 100_000, 0, locator, "schema-a", 4, 9,
    )
    .unwrap()
    .is_some());

    for (case, candidate) in [
        (
            "differentFragment",
            mint(&txn, &alternate_fragment, "schema-a", 4, 9),
        ),
        ("schema", mint(&txn, &fragment, "schema-b", 4, 9)),
        ("epoch", mint(&txn, &fragment, "schema-a", 5, 9)),
        ("revision", mint(&txn, &fragment, "schema-a", 4, 10)),
    ] {
        assert!(candidate.is_none(), "{case}");
    }
    for (case, result) in [
        (
            "differentFragment",
            LocalizedFormatCompiler::try_new(
                717,
                &txn,
                &alternate_fragment,
                &schema,
                100_000,
                100_000,
                0,
                locator,
                "schema-a",
                4,
                9,
            ),
        ),
        (
            "schema",
            LocalizedFormatCompiler::try_new(
                717, &txn, &fragment, &schema, 100_000, 100_000, 0, locator, "schema-b", 4, 9,
            ),
        ),
        (
            "epoch",
            LocalizedFormatCompiler::try_new(
                717, &txn, &fragment, &schema, 100_000, 100_000, 0, locator, "schema-a", 5, 9,
            ),
        ),
        (
            "revision",
            LocalizedFormatCompiler::try_new(
                717, &txn, &fragment, &schema, 100_000, 100_000, 0, locator, "schema-a", 4, 10,
            ),
        ),
    ] {
        assert!(result.unwrap().is_none(), "{case}");
    }

    let foreign = seeded_document(&source, &schema, &limits);
    let foreign_txn = foreign.transact();
    let foreign_fragment = foreign_txn.get_xml_fragment("prosemirror").unwrap();
    assert!(mint(&foreign_txn, &foreign_fragment, "schema-a", 4, 9).is_none());
    assert!(LocalizedFormatCompiler::try_new(
        717,
        &foreign_txn,
        &foreign_fragment,
        &schema,
        100_000,
        100_000,
        0,
        locator,
        "schema-a",
        4,
        9,
    )
    .unwrap()
    .is_none());
}
