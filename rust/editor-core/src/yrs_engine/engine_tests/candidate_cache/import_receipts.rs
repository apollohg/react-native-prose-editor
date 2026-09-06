fn validated_json_import_candidate(engine: &YrsDocumentEngine) -> CandidateDocument {
    let value = serde_json::json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "abc"}]
        }]
    });
    let document =
        from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
    let source = ValidatedImportDocument::new(
        document,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        Some(serde_json::to_vec(&value).unwrap().len()),
    )
    .unwrap();
    engine
        .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
        .unwrap()
}

fn equal_clock_divergent_valid_update(
    engine: &YrsDocumentEngine,
    candidate: &CandidateDocument,
) -> Vec<u8> {
    let divergent = super::equivalent_private_candidate_doc(&candidate.doc);
    let empty_json = json!({
        "type": engine.schema.doc_node_type(),
        "content": [],
    });
    let divergent_json = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "xyz"}]
        }]
    });
    let codec = super::YrsDocumentCodec::new(&engine.schema, &engine.resource_limits);
    {
        let mut txn =
            divergent.transact_mut_with(TransactionOrigin::DocumentImport.as_yrs_origin());
        let fragment = txn.get_or_insert_xml_fragment(engine.fragment_name.as_str());
        codec
            .apply_json(&fragment, &mut txn, &empty_json, &divergent_json)
            .unwrap();
    }
    let candidate_txn = candidate.doc.transact();
    let divergent_txn = divergent.transact();
    assert_eq!(
        divergent_txn.state_vector(),
        candidate_txn.state_vector(),
        "the tamper must keep identical client clocks"
    );
    let candidate_encoded = candidate_txn.encode_state_as_update_v1(&StateVector::default());
    let divergent_encoded = divergent_txn.encode_state_as_update_v1(&StateVector::default());
    assert_ne!(
        divergent_encoded, candidate_encoded,
        "the tamper must carry different valid content"
    );
    divergent_encoded
}

#[test]
fn tampered_import_encoded_state_receipt_falls_back_to_one_cache_encode() {
    for case in [
        "bytes",
        "sha256",
        "stateVector",
        "fragment",
        "clientId",
        "guid",
        "offsetKind",
        "skipGc",
        "deleteSetEligibility",
        "lookupSourceDocument",
        "lookupCanonicalArtifact",
        "lookupResourceLimits",
        "lookupEditingLimits",
        "lookupMaxLength",
        "lookupSchemaToken",
        "lookupStoreToken",
    ] {
        let lookup_only_tamper = case.starts_with("lookup");
        let mut engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let installed = engine.derived_state.as_ref().unwrap();
        let foreign_document = installed.document.clone();
        let foreign_artifact = installed.canonical_artifact.clone();
        let receipt = candidate
            .import_encoded_state_receipt
            .as_mut()
            .expect("validated JSON candidates carry one private encoded-state receipt");
        match case {
            "bytes" => receipt.encoded_state = vec![0xff],
            "sha256" => receipt.encoded_state_sha256[0] ^= 1,
            "stateVector" => receipt.state_vector = StateVector::default(),
            "fragment" => receipt.fragment_id = BranchID::Root(Arc::from("foreign")),
            "clientId" => receipt.client_id = ClientID::new(receipt.client_id.get() ^ 1),
            "guid" => receipt.guid = Arc::from("foreign-guid"),
            "offsetKind" => receipt.offset_kind = OffsetKind::Bytes,
            "skipGc" => receipt.skip_gc = !receipt.skip_gc,
            "deleteSetEligibility" => receipt.delete_set_is_empty = !receipt.delete_set_is_empty,
            "lookupSourceDocument" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .source_document = foreign_document
            }
            "lookupCanonicalArtifact" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .canonical_artifact = foreign_artifact
            }
            "lookupResourceLimits" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .resource_limits
                    .max_document_nodes ^= 1
            }
            "lookupEditingLimits" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .editing_limits
                    .max_operations_per_transaction ^= 1
            }
            "lookupMaxLength" => {
                receipt.lookup_materialization.as_mut().unwrap().max_length = Some(1)
            }
            "lookupSchemaToken" => {
                receipt
                    .lookup_materialization
                    .as_mut()
                    .unwrap()
                    .schema_token ^= 1
            }
            "lookupStoreToken" => receipt.lookup_materialization.as_mut().unwrap().store_token ^= 1,
            _ => unreachable!(),
        }
        reset_import_state_encoding_counts_for_test();
        crate::yrs_engine::mutation::reset_localized_lookup_counts_for_test();

        engine
            .commit_candidate(candidate, TransactionOrigin::DocumentImport)
            .unwrap();

        assert_eq!(
            take_import_state_encoding_counts_for_test(),
            if lookup_only_tamper { (0, 0) } else { (0, 1) },
            "{case}"
        );
        assert_eq!(
            crate::yrs_engine::mutation::take_localized_lookup_counts_for_test().0,
            1,
            "{case}"
        );
        assert_prepared_candidate_state_vector_exact(&engine);
        assert_eq!(
            engine
                .prepared_candidate_cache
                .as_ref()
                .unwrap()
                .encoded_state_seal
                .as_ref()
                .unwrap()
                .encoded_state,
            super::encode_state_bounded(&engine.doc, &engine.resource_limits).unwrap(),
            "{case}"
        );
    }
}

#[test]
fn equal_clock_divergent_valid_receipt_bytes_fall_back_to_authoritative_state() {
    let mut engine = transaction_engine();
    let mut candidate = validated_json_import_candidate(&engine);
    let divergent_encoded = equal_clock_divergent_valid_update(&engine, &candidate);
    candidate
        .import_encoded_state_receipt
        .as_mut()
        .unwrap()
        .encoded_state = divergent_encoded.clone();
    reset_import_state_encoding_counts_for_test();

    engine
        .commit_candidate(candidate, TransactionOrigin::DocumentImport)
        .unwrap();

    assert_eq!(take_import_state_encoding_counts_for_test(), (0, 1));
    assert_prepared_candidate_state_vector_exact(&engine);
    let sealed = &engine
        .prepared_candidate_cache
        .as_ref()
        .unwrap()
        .encoded_state_seal
        .as_ref()
        .unwrap()
        .encoded_state;
    assert_eq!(
        sealed,
        &super::encode_state_bounded(&engine.doc, &engine.resource_limits).unwrap()
    );
    assert_ne!(sealed, &divergent_encoded);
}

#[test]
fn oversized_receipt_falls_back_before_standard_update_decode() {
    let engine = transaction_engine();
    let mut candidate = validated_json_import_candidate(&engine);
    let mut receipt = candidate.import_encoded_state_receipt.take().unwrap();
    let limit = receipt.encoded_state.len().checked_mul(2).unwrap();
    receipt.encoded_state = vec![0xff; limit + 1];
    receipt.encoded_state_sha256 = sha2::Sha256::digest(&receipt.encoded_state).into();
    reset_import_state_encoding_counts_for_test();
    reset_import_receipt_state_decodings_for_test();

    let cache = super::prepare_import_candidate_cache(
        &candidate.doc,
        &engine.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: limit,
            ..engine.resource_limits.clone()
        },
        Some(receipt),
        None,
        1,
        1,
    );

    assert!(cache.is_some());
    assert_eq!(take_import_state_encoding_counts_for_test(), (0, 1));
    assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
}

#[test]
fn import_receipt_obeys_exact_retained_and_two_x_candidate_boundaries() {
    let prepare_at = |boundary: &str| {
        let engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let receipt = candidate.import_encoded_state_receipt.take().unwrap();
        let len = receipt.encoded_state.len();
        let retained =
            super::retained_import_state_charge(len, receipt.encoded_state.capacity()).unwrap();
        let limit = match boundary {
            "retained" => retained,
            "oneUnderRetained" => retained - 1,
            "twoX" => len.checked_mul(2).unwrap(),
            _ => unreachable!(),
        };
        reset_import_state_encoding_counts_for_test();
        let cache = super::prepare_import_candidate_cache(
            &candidate.doc,
            &engine.fragment_name,
            &ResourceLimits {
                max_encoded_state_bytes: limit,
                ..engine.resource_limits.clone()
            },
            Some(receipt),
            None,
            1,
            1,
        );
        assert_eq!(take_import_state_encoding_counts_for_test(), (0, 0));
        cache
    };
    assert!(prepare_at("retained").unwrap().encoded_state_seal.is_some());
    assert!(prepare_at("oneUnderRetained")
        .unwrap()
        .encoded_state_seal
        .is_none());
    assert!(prepare_at("twoX").unwrap().encoded_state_seal.is_none());
}

#[test]
fn import_encoded_state_seal_obeys_exact_retained_charge_without_dropping_two_x_cache() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let encoded = super::encode_state_bounded(&source.doc, &source.resource_limits).unwrap();
    let encoded_len = encoded.len();
    let encoded_capacity = encoded.capacity();
    let exact_retained_charge =
        super::retained_import_state_charge(encoded_len, encoded_capacity).unwrap();

    let exact_cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: exact_retained_charge,
            ..source.resource_limits.clone()
        },
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("the exact retained charge retains the private candidate");
    let exact_seal = exact_cache.encoded_state_seal.as_ref().unwrap();
    assert_eq!(exact_seal.encoded_state.len(), encoded_len);
    assert_eq!(exact_seal.encoded_state.capacity(), encoded_capacity);

    let one_under_cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: exact_retained_charge - 1,
            ..source.resource_limits.clone()
        },
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("a document above one third but within the 2x ceiling retains its candidate");
    assert!(one_under_cache.encoded_state_seal.is_none());

    let exact_two_x_cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &ResourceLimits {
            max_encoded_state_bytes: encoded_len.checked_mul(2).unwrap(),
            ..source.resource_limits.clone()
        },
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("the existing exact 2x candidate admission remains unchanged");
    assert!(exact_two_x_cache.encoded_state_seal.is_none());
}

fn assert_next_insert_uses_full_current_state_encode(
    engine: &mut YrsDocumentEngine,
    request_id: u64,
) {
    reset_encoded_state_reuse_counts_for_test();
    engine
        .apply_typed_transaction(insert_transaction(engine, request_id))
        .unwrap();
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

fn imported_engine_with_sealed_state() -> YrsDocumentEngine {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(engine
        .prepared_candidate_cache
        .as_ref()
        .and_then(|cache| cache.encoded_state_seal.as_ref())
        .is_some());
    engine
}

#[test]
fn sealed_state_vector_drift_falls_back() {
    let mut engine = imported_engine_with_sealed_state();
    let compiled = engine
        .compile_typed_transaction(insert_transaction(&engine, 70_115))
        .unwrap();
    let live_doc = engine.doc.clone();
    let live_txn = live_doc.transact();
    let live_fragment = live_txn
        .get_xml_fragment(engine.fragment_name.as_str())
        .unwrap();
    let exact_state_vector = live_txn.state_vector();
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .state_vector = StateVector::default();
    let reused = engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .take_matching_encoded_state(
            &live_doc,
            &live_fragment,
            &compiled.mutation_plan,
            engine.revision,
            engine.yrs_state_epoch,
            engine.resource_limits.max_encoded_state_bytes,
        );
    assert!(reused.is_none());
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .state_vector = exact_state_vector;
    drop(live_txn);

    reset_encoded_state_reuse_counts_for_test();
    engine.apply_compiled_transaction(compiled, true).unwrap();
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

#[test]
fn import_with_nonempty_delete_set_retains_candidate_without_sealed_bytes() {
    let mut source = imported_engine_with_sealed_state();
    let from = RevisionedPosition {
        offset: 1,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    let to = RevisionedPosition { offset: 2, ..from };
    source
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_116,
            base_document_revision: source.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange { from, to },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!source.doc.transact().snapshot().delete_set.is_empty());

    let cache = super::prepare_import_candidate_cache(
        &source.doc,
        &source.fragment_name,
        &source.resource_limits,
        None,
        None,
        source.revision,
        source.yrs_state_epoch,
    )
    .expect("the existing 2x private candidate remains available");
    assert!(cache.encoded_state_seal.is_none());
}

#[test]
fn sealed_state_fragment_options_revision_and_epoch_drift_fall_back() {
    let mut stale_fragment = imported_engine_with_sealed_state();
    stale_fragment
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap()
        .fragment_id = BranchID::Root(Arc::from("other"));
    assert_next_insert_uses_full_current_state_encode(&mut stale_fragment, 70_118);

    let mut stale_options = imported_engine_with_sealed_state();
    let seal = stale_options
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap();
    seal.offset_kind = match seal.offset_kind {
        OffsetKind::Bytes => OffsetKind::Utf16,
        OffsetKind::Utf16 => OffsetKind::Bytes,
    };
    assert_next_insert_uses_full_current_state_encode(&mut stale_options, 70_119);

    let mut stale_revision = imported_engine_with_sealed_state();
    stale_revision
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap()
        .document_revision = stale_revision.revision.saturating_add(1);
    assert_next_insert_uses_full_current_state_encode(&mut stale_revision, 70_120);

    let mut stale_epoch = imported_engine_with_sealed_state();
    stale_epoch
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal
        .as_mut()
        .unwrap()
        .yrs_state_epoch = stale_epoch.yrs_state_epoch.saturating_add(1);
    assert_next_insert_uses_full_current_state_encode(&mut stale_epoch, 70_121);
}

#[test]
fn sealed_state_rechecks_current_limit_and_survives_selection_only_state_change() {
    let mut limit_drift = transaction_engine();
    let large_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "a".repeat(2_048)}]
        }]
    })
    .to_string();
    limit_drift
        .import_json(&large_source, TransactionOrigin::DocumentImport)
        .unwrap();
    let retained_len = limit_drift
        .prepared_candidate_cache
        .as_ref()
        .unwrap()
        .encoded_state_seal
        .as_ref()
        .unwrap()
        .encoded_state
        .len();
    limit_drift.resource_limits.max_encoded_state_bytes = retained_len.checked_mul(3).unwrap() - 1;
    assert_next_insert_uses_full_current_state_encode(&mut limit_drift, 70_122);

    let mut selection_only = imported_engine_with_sealed_state();
    let document_revision = selection_only.revision;
    let yrs_state_epoch = selection_only.yrs_state_epoch;
    select_text(&mut selection_only, 70_123, 1, 3);
    assert_eq!(selection_only.revision, document_revision);
    assert_eq!(selection_only.yrs_state_epoch, yrs_state_epoch);
    assert!(selection_only
        .prepared_candidate_cache
        .as_ref()
        .unwrap()
        .encoded_state_seal
        .is_some());
    reset_encoded_state_reuse_counts_for_test();
    selection_only
        .apply_typed_transaction(insert_transaction(&selection_only, 70_124))
        .unwrap();
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
}

#[test]
fn sealed_state_bytes_match_stock_oracle_with_history_undo_redo_parity() {
    let mut optimized = imported_engine_with_sealed_state();
    let mut stock = imported_engine_with_sealed_state();
    stock
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .encoded_state_seal = None;
    let stock_current =
        super::encode_state_bounded(&optimized.doc, &optimized.resource_limits).unwrap();
    assert_eq!(
        optimized
            .prepared_candidate_cache
            .as_ref()
            .unwrap()
            .encoded_state_seal
            .as_ref()
            .unwrap()
            .encoded_state
            .as_slice(),
        stock_current.as_slice()
    );

    optimized
        .apply_typed_transaction(insert_transaction(&optimized, 70_125))
        .unwrap();
    stock
        .apply_typed_transaction(insert_transaction(&stock, 70_125))
        .unwrap();
    assert_eq!(optimized.document_json(), stock.document_json());
    assert_eq!(optimized.can_undo(), stock.can_undo());
    assert_eq!(optimized.can_redo(), stock.can_redo());

    optimized.undo(70_126).unwrap();
    stock.undo(70_126).unwrap();
    assert_eq!(optimized.document_json(), stock.document_json());
    assert_eq!(optimized.can_redo(), stock.can_redo());

    optimized.redo(70_127).unwrap();
    stock.redo(70_127).unwrap();
    assert_eq!(optimized.document_json(), stock.document_json());
    assert_eq!(optimized.can_undo(), stock.can_undo());
}
