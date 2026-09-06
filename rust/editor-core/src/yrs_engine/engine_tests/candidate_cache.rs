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

include!("candidate_cache/import_receipts.rs");

include!("candidate_cache/cache_reuse.rs");

include!("candidate_cache/lookup_lifecycle.rs");
