#[test]
fn mutation_lookup_seed_rejects_semantic_root_and_exact_config_drift() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = crate::yrs_engine::EditingLimits::default();
    let max_length = Some(100);
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "same" }]
        }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let foreign = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        715,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        max_length,
        "schema-a",
        1,
        2,
    )
    .unwrap();

    assert!(seed.matches_context(&document, &limits, &editing_limits, max_length));
    assert!(!seed.matches_context(&foreign, &limits, &editing_limits, max_length));

    let mut resource_drift = limits.clone();
    resource_drift.max_input_bytes -= 1;
    assert!(!seed.matches_context(&document, &resource_drift, &editing_limits, max_length));

    let mut editing_drift = editing_limits.clone();
    editing_drift.max_operations_per_transaction -= 1;
    assert!(!seed.matches_context(&document, &limits, &editing_drift, max_length));
    assert!(!seed.matches_context(&document, &limits, &editing_limits, Some(99)));
}

#[test]
fn unavailable_lookup_seed_rejects_promotion_and_rebind_never_revives_it() {
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
    let doc = seeded_document(&source, &schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        725,
        &txn,
        &fragment,
        &schema,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        8,
        12,
    )
    .unwrap();
    assert!(seed.is_ready_for_test());
    let unavailable = seed
        .prepare_unavailable_transition(
            725,
            &txn,
            &fragment,
            &document,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            8,
            12,
            9,
            13,
        )
        .unwrap();
    assert!(unavailable.is_unavailable_for_test());
    assert!(!unavailable.matches(
        &txn,
        &fragment,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        9,
        13,
    ));

    let promotion = MutationLookupPromotion {
        request_id: 725,
        source: MutationLookupPromotionSource::ExistingInsert,
        materialization_work_updates: Vec::new(),
        next_pending_traversal_work: 0,
    };
    let error = unavailable
        .prepare_promotion(
            &txn,
            &fragment,
            &promotion,
            &document,
            &document,
            &limits,
            &editing_limits,
            None,
            "schema-a",
            9,
            13,
            10,
            14,
        )
        .unwrap_err();
    assert_eq!(error.request_id, 725);
    assert!(error.message.contains("unavailable"));

    let rebound = unavailable.rebind_authoritative_store(&txn, &fragment, "schema-a", 10, 14);
    assert!(rebound.is_unavailable_for_test());
    assert!(!rebound.matches(
        &txn,
        &fragment,
        &document,
        &limits,
        &editing_limits,
        None,
        "schema-a",
        10,
        14,
    ));
}

#[test]
fn seeded_localized_insert_treats_stale_schema_epoch_revision_and_store_as_cache_misses() {
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
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let seed = MutationLookupSeed::build(
        708,
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
    let attempt = |txn: &yrs::Transaction<'_>,
                   fragment: &XmlFragmentRef,
                   fingerprint: &str,
                   epoch,
                   revision| {
        LocalizedInsertCompiler::try_new(
            708,
            txn,
            fragment,
            &schema,
            100_000,
            100_000,
            0,
            LocalizedInsertLocator {
                document: &document,
                block_path: block.node_path.as_slice(),
                position: block.doc_start + 1,
            },
            &seed,
            &limits,
            &editing_limits,
            None,
            fingerprint,
            epoch,
            revision,
        )
    };
    for result in [
        attempt(&txn, &fragment, "schema-b", 3, 2),
        attempt(&txn, &fragment, "schema-a", 4, 2),
        attempt(&txn, &fragment, "schema-a", 3, 4),
    ] {
        assert!(result.unwrap().is_none());
    }

    let foreign = seeded_document(&source, &schema, &limits);
    let foreign_txn = foreign.transact();
    let foreign_fragment = foreign_txn.get_xml_fragment("prosemirror").unwrap();
    assert!(attempt(&foreign_txn, &foreign_fragment, "schema-a", 3, 2)
        .unwrap()
        .is_none());
}
