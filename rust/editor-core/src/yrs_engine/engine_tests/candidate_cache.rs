use super::*;

#[test]
fn candidate_state_vector_seal_accepts_redundant_inherited_mark_clock_below_bound() {
    let local = ClientID::new(7);
    let remote = ClientID::new(8);
    let base = StateVector::from_iter([(local, 5), (remote, 13)]);
    let actual = StateVector::from_iter([(local, 6), (remote, 13)]);

    assert_eq!(
        seal_candidate_state_vector(1, &base, actual.clone(), local, 3).unwrap(),
        actual
    );
}

#[test]
fn candidate_state_vector_seal_accepts_zero_local_clock_delta() {
    let local = ClientID::new(7);
    let remote = ClientID::new(8);
    let base = StateVector::from_iter([(local, 5), (remote, 13)]);

    assert_eq!(
        seal_candidate_state_vector(1, &base, base.clone(), local, 0).unwrap(),
        base
    );
}

#[test]
fn candidate_state_vector_seal_rejects_authored_clock_bound_excess() {
    let local = ClientID::new(7);
    let base = StateVector::from_iter([(local, 5)]);
    let actual = StateVector::from_iter([(local, 9)]);

    let error = seal_candidate_state_vector(1, &base, actual, local, 3)
        .expect_err("candidate local authorship above the admitted bound must reject");

    assert!(error
        .message
        .contains("exceeded its admitted authored clock bound"));
}

#[test]
fn candidate_state_vector_seal_rejects_local_clock_regression() {
    let local = ClientID::new(7);
    let base = StateVector::from_iter([(local, 5)]);
    let actual = StateVector::from_iter([(local, 4)]);

    let error = seal_candidate_state_vector(1, &base, actual, local, 3)
        .expect_err("candidate local clock regression must reject");

    assert!(error.message.contains("regressed its local authored clock"));
}

#[test]
fn candidate_state_vector_seal_rejects_nonlocal_clock_drift() {
    let local = ClientID::new(7);
    let remote = ClientID::new(8);
    let injected = ClientID::new(9);
    let base = StateVector::from_iter([(local, 5), (remote, 13)]);
    let actual = StateVector::from_iter([(local, 6), (remote, 14), (injected, 1)]);

    let error = seal_candidate_state_vector(1, &base, actual, local, 3)
        .expect_err("candidate nonlocal clock drift must reject");

    assert!(error.message.contains("changed a nonlocal authored clock"));
}

#[test]
fn apply_command_runs_one_semantic_compilation() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };
    use crate::yrs_engine::compiler::{
        reset_semantic_compilation_count_for_test, take_semantic_compilation_count_for_test,
    };

    let mut engine = transaction_engine();
    reset_semantic_compilation_count_for_test();
    reset_canonical_artifact_counts_for_test();

    let result = engine
        .apply_command(70_001, TypedCommand::InsertText { text: "x".into() })
        .unwrap();

    assert!(result.is_some());
    assert_eq!(take_semantic_compilation_count_for_test(), 1);
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));
}

#[test]
fn existing_text_insert_burst_hits_localized_lookup_and_promotes_without_full_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_101))
        .unwrap();
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_102))
        .unwrap();

    assert_eq!(take_localized_lookup_counts_for_test(), (0, 2, 2));
}

#[test]
fn prepared_candidate_cache_reuses_one_exact_store_across_successful_insert_burst() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let imported_cache = engine
        .prepared_candidate_cache_store_token_for_test()
        .expect("successful bounded import prepares a candidate cache");
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    reset_prepared_candidate_cache_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_103))
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);
    assert_eq!(
        engine.prepared_candidate_cache_store_token_for_test(),
        Some(imported_cache),
        "the exact prepared candidate becomes the next sealed cache"
    );
    let cached_encoded = super::encode_state_bounded(
        &engine.prepared_candidate_cache.as_ref().unwrap().doc,
        &engine.resource_limits,
    )
    .unwrap();
    assert_eq!(cached_encoded, engine.encoded_state().unwrap());
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_104))
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);

    assert_eq!(
        engine.prepared_candidate_cache_store_token_for_test(),
        Some(imported_cache)
    );
    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (2, 0));
}

#[test]
fn imported_candidate_sealed_state_replaces_only_the_first_commit_full_encode() {
    let mut engine = transaction_engine();
    reset_encoded_state_reuse_counts_for_test();
    reset_import_state_encoding_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 0));
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_113))
        .unwrap();
    assert_eq!(
        take_encoded_state_reuse_counts_for_test(),
        (0, 0, 1),
        "the import's exact one-shot bytes must replace the first commit-time full encode"
    );

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_114))
        .unwrap();
    assert_eq!(
        take_encoded_state_reuse_counts_for_test(),
        (0, 1, 0),
        "successful mutation caches must not retain the stale import bytes"
    );
}

#[test]
fn validated_html_import_carries_its_first_bounded_encode_into_the_cache() {
    let mut engine = transaction_engine();
    reset_import_state_encoding_counts_for_test();

    engine
        .import_html(
            "<p>abc</p>",
            &FromHtmlOptions::default(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
    assert_prepared_candidate_state_vector_exact(&engine);
}

#[test]
fn import_cache_eligibility_requires_a_localized_mutation_target() {
    let empty_textblock_engine = transaction_engine();
    let empty_textblock_value = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let empty_textblock_document = from_prosemirror_json(
        &empty_textblock_value,
        &empty_textblock_engine.schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let empty_textblock_source = ValidatedImportDocument::new(
        empty_textblock_document,
        &empty_textblock_engine.schema,
        &empty_textblock_engine.canonical_schema,
        &empty_textblock_engine.resource_limits,
        Some(empty_textblock_value.to_string().len()),
    )
    .unwrap();
    let empty_textblock_candidate = empty_textblock_engine
        .build_candidate_from_document(empty_textblock_source, TransactionOrigin::DocumentImport)
        .unwrap();
    assert!(
        empty_textblock_candidate.import_acceleration_eligible,
        "the collector's trailing empty-textblock gap is a localized target"
    );
    assert!(empty_textblock_candidate
        .import_encoded_state_receipt
        .is_some());

    let mut one_text_target = transaction_engine();
    reset_import_receipt_sha256_counts_for_test();
    one_text_target
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(
        one_text_target.prepared_candidate_cache.is_some(),
        "one localized text target remains eligible"
    );
    assert_eq!(take_import_receipt_sha256_counts_for_test(), (1, 1));

    let mut known_void = transaction_engine();
    reset_import_state_encoding_counts_for_test();
    reset_import_receipt_state_decodings_for_test();
    reset_import_receipt_sha256_counts_for_test();
    known_void
        .import_json(
            r#"{"type":"doc","content":[{"type":"image","attrs":{"src":"asset://one"}}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(
        take_import_state_encoding_counts_for_test(),
        (1, 0),
        "candidate admission still performs its one mandatory bounded encode"
    );
    assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
    assert_eq!(take_import_receipt_sha256_counts_for_test(), (0, 0));
    assert!(
        known_void.prepared_candidate_cache.is_none(),
        "a textless void-only document has no localized target to accelerate"
    );
    assert_eq!(
        known_void.document_json().unwrap(),
        json!({
            "type": "doc",
            "content": [{
                "type": "image",
                "attrs": { "src": "asset://one" }
            }]
        })
    );

    for (name, value) in [
        (
            "mixedTextOpaque",
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "addressable" }]
                    },
                    {
                        "type": "customOpaqueBlock",
                        "attrs": { "payload": "retained" }
                    }
                ]
            }),
        ),
        (
            "article",
            json!({
                "type": "doc",
                "content": [
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Title" }]
                    },
                    {
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "Body" }]
                    }
                ]
            }),
        ),
    ] {
        let mut engine = transaction_engine();
        engine
            .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        assert!(
            engine.prepared_candidate_cache.is_some(),
            "{name} must retain import acceleration"
        );
        assert_eq!(engine.document_json().unwrap(), value, "{name}");
    }
}

#[test]
fn deferred_import_still_obeys_exact_candidate_encoded_state_ceiling() {
    fn validated_opaque_source(
        engine: &YrsDocumentEngine,
        value: &serde_json::Value,
    ) -> ValidatedImportDocument {
        let document =
            from_prosemirror_json(value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
        ValidatedImportDocument::new(
            document,
            &engine.schema,
            &engine.canonical_schema,
            &engine.resource_limits,
            Some(value.to_string().len()),
        )
        .unwrap()
    }

    let value = json!({
        "type": "doc",
        "content": [{
            "type": "benchmarkOpaqueBlock",
            "attrs": { "payload": "opaque" }
        }]
    });
    let probe = transaction_engine();
    let candidate = probe
        .build_candidate_from_document(
            validated_opaque_source(&probe, &value),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(!candidate.import_acceleration_eligible);
    assert!(candidate.import_encoded_state_receipt.is_none());
    let encoded_len = super::encode_state_bounded(&candidate.doc, &probe.resource_limits)
        .unwrap()
        .len();
    let exact_doc = super::equivalent_private_candidate_doc(&candidate.doc);
    let one_under_doc = super::equivalent_private_candidate_doc(&candidate.doc);

    let mut exact = transaction_engine();
    exact.resource_limits = ResourceLimits {
        max_encoded_state_bytes: encoded_len,
        ..exact.resource_limits.clone()
    };
    reset_import_state_encoding_counts_for_test();
    let exact_candidate = exact
        .build_candidate_from_document_in_doc(
            validated_opaque_source(&exact, &value),
            TransactionOrigin::DocumentImport,
            exact_doc,
        )
        .expect("the exact authoritative candidate byte ceiling must admit");
    assert!(!exact_candidate.import_acceleration_eligible);
    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));

    let mut one_under = transaction_engine();
    one_under.resource_limits = ResourceLimits {
        max_encoded_state_bytes: encoded_len - 1,
        ..one_under.resource_limits.clone()
    };
    reset_import_state_encoding_counts_for_test();
    let error = match one_under.build_candidate_from_document_in_doc(
        validated_opaque_source(&one_under, &value),
        TransactionOrigin::DocumentImport,
        one_under_doc,
    ) {
        Ok(_) => panic!("one under the authoritative candidate bytes must reject"),
        Err(error) => error,
    };
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(encoded_len - 1));
    assert_eq!(error.actual, Some(encoded_len));
    assert_eq!(take_import_state_encoding_counts_for_test(), (1, 0));
}

#[test]
fn opaque_only_import_defers_replica_then_first_structural_mutation_bootstraps() {
    let opaque = json!({
        "type": "doc",
        "content": [{
            "type": "benchmarkOpaqueBlock",
            "attrs": { "payload": "x".repeat(32 * 1024) }
        }]
    });
    let mut engine = transaction_engine();
    reset_import_state_encoding_counts_for_test();
    reset_import_receipt_state_decodings_for_test();
    reset_import_receipt_sha256_counts_for_test();

    engine
        .import_json(&opaque.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    assert_eq!(
        take_import_state_encoding_counts_for_test(),
        (1, 0),
        "candidate admission still performs its one mandatory bounded encode"
    );
    assert_eq!(take_import_receipt_state_decodings_for_test(), 0);
    assert_eq!(take_import_receipt_sha256_counts_for_test(), (0, 0));
    assert!(engine.prepared_candidate_cache.is_none());
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    assert_eq!(engine.document_json().unwrap(), opaque);

    reset_prepared_candidate_cache_counts_for_test();
    reset_encoded_state_reuse_counts_for_test();
    engine
        .apply_typed_transaction(paragraph_insert_transaction(&engine, 70_115))
        .unwrap();

    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
    assert!(engine.prepared_candidate_cache.is_some());
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    let json = engine.document_json().unwrap();
    assert_eq!(json["content"][0], opaque["content"][0]);
    assert_eq!(json["content"][1]["type"], "paragraph");
}

#[test]
fn validated_import_commit_does_not_recompute_schema_fingerprint() {
    use crate::schema::{
        reset_schema_fingerprint_count_for_test, take_schema_fingerprint_count_for_test,
    };

    let mut engine = transaction_engine();
    let candidate = validated_json_import_candidate(&engine);
    reset_schema_fingerprint_count_for_test();

    engine
        .commit_candidate(candidate, TransactionOrigin::DocumentImport)
        .unwrap();

    let total_fingerprints = take_schema_fingerprint_count_for_test();
    assert_eq!(
        total_fingerprints, 1,
        "the test-only render-cache slow invariant remains the sole fingerprint call"
    );
    assert_eq!(
        total_fingerprints.saturating_sub(1),
        0,
        "the exact immutable schema and canonical-artifact seals make commit-time hashing redundant"
    );
}

#[test]
fn import_lookup_schema_seal_drift_falls_back_exactly_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    for case in [
        "schemaToken",
        "currentSchemaFingerprint",
        "equalDistinctSchemaPointer",
    ] {
        let engine = transaction_engine();
        let mut candidate = validated_json_import_candidate(&engine);
        let (source_document, canonical_artifact) = match &candidate.state {
            EngineDocumentState::Ready {
                document,
                canonical_artifact,
            } => (document.clone(), canonical_artifact.clone()),
            EngineDocumentState::AwaitingRemote => {
                panic!("validated import candidate must be ready")
            }
        };
        let mut receipt = candidate
            .import_encoded_state_receipt
            .take()
            .expect("validated import candidate carries its lookup receipt");
        if case == "schemaToken" {
            receipt
                .lookup_materialization
                .as_mut()
                .unwrap()
                .schema_token ^= 1;
        }
        let equal_schema = engine.schema.clone();
        let schema = if case == "equalDistinctSchemaPointer" {
            &equal_schema
        } else {
            &engine.schema
        };
        let drifted_schema_fingerprint = format!("{}-drifted", engine.schema_fingerprint);
        let schema_fingerprint = if case == "currentSchemaFingerprint" {
            drifted_schema_fingerprint.as_str()
        } else {
            engine.schema_fingerprint.as_str()
        };

        reset_localized_lookup_counts_for_test();
        let fused = receipt.take_matching_lookup_materialization(
            &candidate.doc,
            &engine.fragment_name,
            &source_document,
            &canonical_artifact,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            schema,
            schema_fingerprint,
            1,
            1,
        );
        assert!(fused.is_none(), "{case}");

        let txn = candidate.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        crate::yrs_engine::mutation::MutationLookupSeed::build(
            0,
            &txn,
            &fragment,
            schema,
            &source_document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            schema_fingerprint,
            1,
            1,
        )
        .unwrap();
        assert_eq!(take_localized_lookup_counts_for_test().0, 1, "{case}");
    }
}

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

#[test]
fn prepared_candidate_seals_actual_clock_for_redundant_inherited_mark_insert() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let local_client = engine.doc.client_id();

    let first = engine
        .compile_typed_transaction(marked_insert_transaction(&engine, 70_109, "a"))
        .unwrap();
    assert_eq!(first.authored_clock_units, 3);
    let before_first = engine.doc.transact().state_vector().get(&local_client);
    engine.apply_compiled_transaction(first, true).unwrap();
    let after_first = engine.doc.transact().state_vector().get(&local_client);
    assert_eq!(after_first - before_first, 3);

    let second = engine
        .compile_typed_transaction(marked_insert_transaction(&engine, 70_110, "b"))
        .unwrap();
    assert_eq!(second.authored_clock_units, 3);
    let before_second = engine.doc.transact().state_vector().get(&local_client);
    engine.apply_compiled_transaction(second, true).unwrap();
    let after_second = engine.doc.transact().state_vector().get(&local_client);

    assert_eq!(after_second - before_second, 1);
    assert_prepared_candidate_state_vector_exact(&engine);
}

#[test]
fn prepared_candidate_bounds_inherited_format_suspension_at_text_boundaries() {
    struct Case {
        name: &'static str,
        source: &'static str,
        offset: u32,
        inserted: &'static str,
        marks: Vec<Mark>,
        expected_bound: u64,
    }

    let bold = || Mark::new("bold".into(), HashMap::new());
    let italic = || Mark::new("italic".into(), HashMap::new());
    let cases = [
        Case {
            name: "plain at start",
            source: "ab",
            offset: 0,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "plain inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "plain at end",
            source: "ab",
            offset: 2,
            inserted: "x",
            marks: vec![],
            expected_bound: 3,
        },
        Case {
            name: "same mark inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![bold()],
            expected_bound: 3,
        },
        Case {
            name: "different mark inside",
            source: "ab",
            offset: 1,
            inserted: "x",
            marks: vec![italic()],
            expected_bound: 5,
        },
        Case {
            name: "plain unicode inside",
            source: "😀b",
            offset: 1,
            inserted: "🦀",
            marks: vec![],
            expected_bound: 4,
        },
    ];

    for (index, case) in cases.into_iter().enumerate() {
        let mut engine = transaction_engine();
        engine
            .import_json(
                &serde_json::json!({
                    "type": "doc",
                    "content": [{
                        "type": "paragraph",
                        "content": [{
                            "type": "text",
                            "text": case.source,
                            "marks": [{ "type": "bold" }]
                        }]
                    }]
                })
                .to_string(),
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        let request_id = 70_120 + u64::try_from(index).unwrap();
        let compiled = engine
            .compile_typed_transaction(TypedTransaction {
                request_id,
                base_document_revision: engine.revision(),
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: case.offset,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: case.inserted.into(),
                    marks: case.marks,
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Skip,
            })
            .unwrap();
        assert_eq!(
            compiled.authored_clock_units, case.expected_bound,
            "{}",
            case.name
        );
        let local_client = engine.doc.client_id();
        let before = engine.doc.transact().state_vector().get(&local_client);
        engine.apply_compiled_transaction(compiled, true).unwrap();
        let after = engine.doc.transact().state_vector().get(&local_client);
        assert!(
            u64::from(after - before) <= case.expected_bound,
            "{}",
            case.name
        );
        assert_prepared_candidate_state_vector_exact(&engine);
    }

    let mut boundary = transaction_engine();
    boundary
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a","marks":[{"type":"bold"}]},{"type":"text","text":"b","marks":[{"type":"italic"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let compiled = boundary
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_126,
            base_document_revision: boundary.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "x".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    // The lowering selects one exact storage target at this semantic
    // boundary; only that target's touching bold run contributes.
    assert_eq!(compiled.authored_clock_units, 3);
    boundary.apply_compiled_transaction(compiled, true).unwrap();
    assert_prepared_candidate_state_vector_exact(&boundary);

    let mut delete_then_insert = transaction_engine();
    delete_then_insert
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab","marks":[{"type":"bold"}]}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let compiled = delete_then_insert
        .compile_typed_transaction(TypedTransaction {
            request_id: 70_127,
            base_document_revision: delete_then_insert.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![
                TypedOperation::DeleteRange {
                    range: RevisionedRange {
                        from: RevisionedPosition {
                            offset: 0,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::After,
                        },
                        to: RevisionedPosition {
                            offset: 2,
                            kind: EditorOffsetKind::Scalar,
                            affinity: Affinity::Before,
                        },
                    },
                },
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 0,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "x".into(),
                    marks: vec![],
                },
            ],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(compiled.authored_clock_units, 3);
    delete_then_insert
        .apply_compiled_transaction(compiled, true)
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&delete_then_insert);
}

#[test]
fn prepared_candidate_cache_failure_is_private_atomic_and_falls_back_once() {
    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let before = atomic_audit(&engine);
    set_compiled_commit_stage_failpoint_for_test(Some(
        CompiledCommitPreparationStage::HistoryUpdateEncoding,
    ));
    reset_encoded_state_reuse_counts_for_test();

    let error = engine
        .apply_typed_transaction(insert_transaction(&engine, 70_105))
        .expect_err("candidate encoding failpoint must reject before the live write");

    set_compiled_commit_stage_failpoint_for_test(None);
    assert!(error.message.contains("historyUpdateEncoding"));
    assert_eq!(atomic_audit(&engine), before);
    assert!(engine.prepared_candidate_cache.is_none());
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 0, 1));
    reset_prepared_candidate_cache_counts_for_test();
    reset_encoded_state_reuse_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_106))
        .unwrap();

    assert!(engine.prepared_candidate_cache.is_some());
    assert_eq!(take_prepared_candidate_cache_counts_for_test(), (0, 1));
    assert_eq!(take_encoded_state_reuse_counts_for_test(), (0, 1, 0));
}

#[test]
fn prepared_candidate_cache_revalidates_stale_revision_seal_before_reuse() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    engine
        .prepared_candidate_cache
        .as_mut()
        .unwrap()
        .document_revision = engine.revision.saturating_add(1);
    reset_prepared_candidate_cache_counts_for_test();
    reset_localized_lookup_counts_for_test();

    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_108))
        .unwrap();
    let cache_counts = take_prepared_candidate_cache_counts_for_test();
    let lookup_counts = take_localized_lookup_counts_for_test();
    let cached_encoded = super::encode_state_bounded(
        &engine.prepared_candidate_cache.as_ref().unwrap().doc,
        &engine.resource_limits,
    )
    .unwrap();

    assert_eq!(cache_counts, (0, 1));
    assert_eq!(lookup_counts, (1, 1, 1));
    assert_eq!(
        engine.document_json().unwrap()["content"][0]["content"][0]["text"],
        "axbc"
    );
    assert_eq!(cached_encoded, engine.encoded_state().unwrap());
}

#[test]
fn imported_candidate_cache_supplies_first_staged_lookup_without_live_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());
    reset_localized_lookup_counts_for_test();

    engine
        .apply_command(70_107, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();

    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
}

#[test]
fn validated_import_materializes_ready_lookup_without_a_second_tree_scan() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_localized_lookup_counts_for_test();

    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    assert_eq!(
        take_localized_lookup_counts_for_test(),
        (0, 0, 0),
        "validated codec traversal must carry the exact ready lookup payload"
    );
    assert!(engine
        .prepared_candidate_cache
        .as_ref()
        .and_then(|cache| cache.staged_lookup_seed.as_ref())
        .is_some());
}

#[test]
fn validated_import_lookup_materialization_matches_the_ordinary_builder() {
    let inputs = [
        r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":2}}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain"},{"type":"text","text":" bold","marks":[{"type":"bold"}]},{"type":"text","text":" 🦀"}]}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"hardBreak"},{"type":"text","text":"middle"},{"type":"hardBreak"},{"type":"hardBreak"}]}]}"#,
        r#"{"type":"doc","content":[{"type":"blockquote","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"nested"}]}]},{"type":"horizontal_rule"},{"type":"mystery_widget","attrs":{"payload":{"x":[1,true,"v"]}}}]}"#,
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"a"},{"type":"text","text":"b","marks":[{"type":"italic"}]},{"type":"text","text":"c"}]}]}"#,
    ];

    for input in inputs {
        let mut engine = transaction_engine();
        engine
            .import_json(input, TransactionOrigin::DocumentImport)
            .unwrap();
        let staged = engine
            .prepared_candidate_cache
            .as_ref()
            .and_then(|cache| cache.staged_lookup_seed.as_ref())
            .unwrap_or_else(|| panic!("validated import carries the fused ready seed: {input}"));
        let txn = engine.doc.transact();
        let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
        let state = engine.derived_state.as_ref().unwrap();
        assert!(
            crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
                &txn,
                &fragment,
                &engine.schema,
            ),
            "{input}"
        );
        let ordinary = crate::yrs_engine::mutation::MutationLookupSeed::build(
            77_001,
            &txn,
            &fragment,
            &engine.schema,
            &state.document,
            &engine.resource_limits,
            &engine.editing_limits,
            engine.max_length,
            &engine.schema_fingerprint,
            engine.yrs_state_epoch,
            engine.revision,
        )
        .unwrap();
        assert!(staged.has_same_ready_payload_for_test(&ordinary), "{input}");
    }
}

#[test]
fn lookup_materialization_matches_legacy_for_nested_fragment_and_empty_text_storage() {
    let engine = transaction_engine();
    let doc = utf16_doc();
    let mut txn = doc.transact_mut();
    let fragment = txn.get_or_insert_xml_fragment("content");
    let nested = XmlFragmentPrelim::new::<_, XmlIn>([
        XmlIn::from(XmlTextPrelim::new("")),
        XmlIn::from(XmlTextPrelim::new("x")),
    ]);
    fragment.insert(&mut txn, 0, XmlIn::from(nested));
    drop(txn);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("content").unwrap();

    assert!(
        crate::yrs_engine::mutation::lookup_payload_legacy_parity_for_test(
            &txn,
            &fragment,
            &engine.schema,
        )
    );
}

#[test]
fn import_lookup_materialization_failpoints_are_opportunistic_and_fallback_once() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, set_lookup_seed_hydration_failpoint_for_test,
        take_localized_lookup_counts_for_test, LookupSeedHydrationFailpoint,
    };

    for failpoint in [
        LookupSeedHydrationFailpoint::InitialReservation,
        LookupSeedHydrationFailpoint::MapGrowth,
        LookupSeedHydrationFailpoint::MapPublication,
        LookupSeedHydrationFailpoint::BindingPublication,
        LookupSeedHydrationFailpoint::SeedPublication,
    ] {
        let mut engine = transaction_engine();
        reset_localized_lookup_counts_for_test();
        set_lookup_seed_hydration_failpoint_for_test(Some(failpoint));
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
                TransactionOrigin::DocumentImport,
            )
            .unwrap();
        set_lookup_seed_hydration_failpoint_for_test(None);
        assert_eq!(
            take_localized_lookup_counts_for_test().0,
            1,
            "{failpoint:?}"
        );

        reset_localized_lookup_counts_for_test();
        engine
            .apply_typed_transaction(insert_transaction(&engine, 77_100))
            .unwrap();
        assert_eq!(
            engine.document_json().unwrap()["content"][0]["content"][0]["text"],
            "axbc",
            "{failpoint:?}"
        );
        assert_prepared_candidate_state_vector_exact(&engine);
    }
}

#[test]
fn ordinary_lookup_collection_fails_fast_while_codec_projection_finishes() {
    use crate::yrs_engine::mutation::{
        reset_import_lookup_event_count_for_test, set_lookup_seed_hydration_failpoint_for_test,
        take_import_lookup_event_count_for_test, LookupSeedHydrationFailpoint,
    };

    let value = json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "first"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "second"}]}
        ]
    });
    let mut engine = transaction_engine();
    engine
        .import_json(&value.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let txn = engine.doc.transact();
    let fragment = txn.get_xml_fragment(engine.fragment_name.as_str()).unwrap();
    let state = engine.derived_state.as_ref().unwrap();
    reset_import_lookup_event_count_for_test();
    set_lookup_seed_hydration_failpoint_for_test(Some(LookupSeedHydrationFailpoint::MapGrowth));
    let error = crate::yrs_engine::mutation::MutationLookupSeed::build(
        77_200,
        &txn,
        &fragment,
        &engine.schema,
        &state.document,
        &engine.resource_limits,
        &engine.editing_limits,
        engine.max_length,
        &engine.schema_fingerprint,
        engine.yrs_state_epoch,
        engine.revision,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(take_import_lookup_event_count_for_test(), 2);
    drop(txn);

    let document =
        from_prosemirror_json(&value, &engine.schema, UnknownTypeMode::Preserve).unwrap();
    let source = ValidatedImportDocument::new(
        document,
        &engine.schema,
        &engine.canonical_schema,
        &engine.resource_limits,
        Some(value.to_string().len()),
    )
    .unwrap();
    reset_import_lookup_event_count_for_test();
    let candidate = engine
        .build_candidate_from_document(source, TransactionOrigin::DocumentImport)
        .unwrap();
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert_eq!(take_import_lookup_event_count_for_test(), 2);
    assert!(candidate
        .import_encoded_state_receipt
        .as_ref()
        .is_some_and(|receipt| receipt.lookup_materialization.is_none()));
    let EngineDocumentState::Ready { document, .. } = candidate.state else {
        panic!("validated candidate must be ready")
    };
    assert_eq!(document.root().content().unwrap().child_count(), 2);
}

#[test]
fn missing_text_fallback_rebuilds_once_then_next_insert_localizes() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_111, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("empty paragraph insert must apply");
    engine
        .apply_command(70_112, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .expect("existing text insert must apply");

    assert_eq!(take_localized_lookup_counts_for_test(), (1, 1, 1));
}

#[test]
fn selection_only_change_retains_document_scoped_lookup_seed() {
    let mut engine = transaction_engine();
    let before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .clone();
    let canonical_before = engine
        .derived_state
        .as_ref()
        .unwrap()
        .canonical_artifact
        .clone();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_113,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Set(SelectionInput::All),
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    let after = &engine.derived_state.as_ref().unwrap().mutation_lookup_seed;
    assert!(Arc::ptr_eq(&before, after));
    assert!(canonical_before.ptr_eq(&engine.derived_state.as_ref().unwrap().canonical_artifact));
}

#[test]
fn localized_root_invalidation_rebuilds_ready_once_then_localizes() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .apply_command(
            70_113_100,
            TypedCommand::WrapInList {
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
            },
        )
        .unwrap()
        .unwrap();
    assert_prepared_candidate_state_vector_exact(&engine);
    let unavailable = engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .clone();
    assert!(unavailable.is_unavailable_for_test());

    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_113_101,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(Arc::ptr_eq(
        &unavailable,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_unavailable_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_113_102, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (2, 0, 0));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_113_103, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
}

#[test]
fn canonical_artifact_derives_once_per_changed_intermediate_and_never_for_cached_noops() {
    use crate::yrs_engine::canonical::{
        reset_canonical_artifact_counts_for_test, take_canonical_artifact_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_canonical_artifact_counts_for_test();
    engine
        .apply_typed_transaction(insert_transaction(&engine, 70_114))
        .unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (1, 2));

    let revision = engine.revision();
    reset_canonical_artifact_counts_for_test();
    engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_115,
            base_document_revision: revision,
            origin: TransactionOrigin::LocalApi,
            operations: vec![
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "a".into(),
                    marks: vec![],
                },
                TypedOperation::InsertText {
                    at: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    text: "b".into(),
                    marks: vec![],
                },
            ],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert_eq!(take_canonical_artifact_counts_for_test(), (2, 3));

    reset_canonical_artifact_counts_for_test();
    let commit = engine
        .apply_typed_transaction(TypedTransaction {
            request_id: 70_116,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalApi,
            operations: vec![TypedOperation::DeleteRange {
                range: RevisionedRange {
                    from: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                    to: RevisionedPosition {
                        offset: 1,
                        kind: EditorOffsetKind::Scalar,
                        affinity: Affinity::After,
                    },
                },
            }],
            selection_intent: SelectionIntent::Preserve,
            history_policy: HistoryPolicy::Skip,
        })
        .unwrap();
    assert!(!commit.changed);
    assert_eq!(take_canonical_artifact_counts_for_test(), (0, 0));
}

#[test]
fn public_history_pop_installs_candidate_seed_without_next_edit_rebuild() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };
    use crate::yrs_engine::observability::{
        reset_prepared_admission_counts_for_test, take_prepared_admission_counts_for_test,
    };

    let mut engine = transaction_engine();
    reset_prepared_admission_counts_for_test();
    assert!(engine.undo(70_119).unwrap().is_none());
    assert!(engine.redo(70_120).unwrap().is_none());
    let empty = take_prepared_admission_counts_for_test();
    assert_eq!(empty.staged_seed_preparations, 0);
    assert_eq!(empty.installed_base_seed_publications, 0);
    reset_localized_lookup_counts_for_test();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 0, 0));

    engine
        .apply_command(70_121, TypedCommand::InsertText { text: "x".into() })
        .unwrap()
        .expect("history insert must apply");
    reset_localized_lookup_counts_for_test();
    reset_prepared_admission_counts_for_test();
    let before_undo = Arc::clone(&engine.derived_state.as_ref().unwrap().mutation_lookup_seed);
    assert!(engine.undo(70_122).unwrap().is_some());
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    let undo_counts = take_prepared_admission_counts_for_test();
    assert_eq!(undo_counts.staged_seed_preparations, 1);
    assert_eq!(undo_counts.installed_base_seed_publications, 0);
    assert!(!Arc::ptr_eq(
        &before_undo,
        &engine.derived_state.as_ref().unwrap().mutation_lookup_seed,
    ));
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());
    reset_localized_lookup_counts_for_test();
    reset_prepared_admission_counts_for_test();
    assert!(engine.redo(70_123).unwrap().is_some());
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    let redo_counts = take_prepared_admission_counts_for_test();
    assert_eq!(redo_counts.staged_seed_preparations, 1);
    assert_eq!(redo_counts.installed_base_seed_publications, 0);
    assert!(engine
        .derived_state
        .as_ref()
        .unwrap()
        .mutation_lookup_seed
        .is_ready_for_test());

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_124, TypedCommand::InsertText { text: "y".into() })
        .unwrap()
        .expect("the first edit after history restoration must apply");
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

    reset_localized_lookup_counts_for_test();
    engine
        .apply_command(70_125, TypedCommand::InsertText { text: "z".into() })
        .unwrap()
        .expect("the second edit after history restoration must apply");
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"snapshot"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();
    reset_localized_lookup_counts_for_test();
    engine.restore_snapshot(&snapshot).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
}

#[test]
fn accepted_remote_candidate_builds_lookup_seed_in_its_own_store() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let update = source.encoded_state().unwrap();
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    reset_localized_lookup_counts_for_test();

    let commit = target.apply_remote_update_v1(70_131, &update).unwrap();
    assert!(commit.changed);
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
    reset_localized_lookup_counts_for_test();
    target
        .apply_command(70_132, TypedCommand::InsertText { text: "!".into() })
        .unwrap()
        .expect("remote existing text must accept a local insert");
    assert_prepared_candidate_state_vector_exact(&target);
    let live_vector = target.doc.transact().state_vector();
    assert!(live_vector.get(&ClientID::new(source.client_id())) > 0);
    assert!(live_vector.get(&ClientID::new(target.client_id())) > 0);
    assert_eq!(take_localized_lookup_counts_for_test(), (0, 1, 1));
}

#[test]
fn arbitrary_remote_candidate_rebuilds_revision_bound_render_cache_once() {
    let mut source = transaction_engine();
    source
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"remote"}]},{"type":"paragraph","content":[{"type":"text","text":"tail"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut target = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: crate::yrs_engine::InitializationMode::AwaitRemote,
        resource_limits: ResourceLimits::default(),
        editing_limits: crate::yrs_engine::EditingLimits::default(),
        max_length: None,
        scope: Some(crate::yrs_engine::DocumentScope {
            document_id: "doc".into(),
            lineage_id: "lineage".into(),
        }),
    })
    .unwrap();
    target
        .apply_remote_update_v1(70_133, &source.encoded_state().unwrap())
        .unwrap();
    source
        .apply_typed_transaction(insert_transaction(&source, 70_134))
        .unwrap();
    let target_vector = target.doc.transact().state_vector();
    let delta = source
        .doc
        .transact()
        .encode_state_as_update_v1(&target_vector);

    crate::render::incremental::reset_cached_render_counts_for_test();
    let commit = target.apply_remote_update_v1(70_135, &delta).unwrap();
    assert!(commit.changed);
    assert_eq!(
        crate::render::incremental::take_cached_render_counts_for_test(),
        (1, 0, 0, 0, 0)
    );
    let next = target.derived_state.as_ref().unwrap();
    assert_eq!(
        next.render_blocks.materialize(),
        crate::render::incremental::render_blocks(&next.document, &target.schema)
    );
    assert_eq!(next.document_revision, target.revision());
    assert_eq!(next.schema_fingerprint, target.schema_fingerprint);
}

#[test]
fn multi_operation_and_explicit_selection_inserts_use_sealed_eager_fallback() {
    use crate::yrs_engine::mutation::{
        reset_localized_lookup_counts_for_test, take_localized_lookup_counts_for_test,
    };

    let mut engine = transaction_engine();
    engine
        .import_json(
            r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abc"}]}]}"#,
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let mut transaction = insert_transaction(&engine, 70_141);
    transaction.operations.push(TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "y".into(),
        marks: vec![],
    });
    reset_localized_lookup_counts_for_test();
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));

    let mut transaction = insert_transaction(&engine, 70_142);
    let point = RevisionedPosition {
        offset: 2,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    };
    transaction.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point,
        head: point,
    });
    reset_localized_lookup_counts_for_test();
    engine.apply_typed_transaction(transaction).unwrap();
    assert_eq!(take_localized_lookup_counts_for_test(), (1, 0, 0));
}
