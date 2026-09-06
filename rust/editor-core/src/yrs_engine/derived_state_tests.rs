use serde_json::json;
use yrs::{Doc, OffsetKind, Options, Transact, WriteTxn};

use super::*;
use crate::boundary::ResourceLimits;
use crate::schema::presets::tiptap_schema;
use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
use crate::yrs_engine::codec::YrsDocumentCodec;

fn initialize_test_document(
    schema: &Schema,
    source: serde_json::Value,
) -> Option<DerivedStateCache> {
    let context = super::super::canonical::CanonicalSchemaContext::new(schema);
    let fingerprint = context.schema_fingerprint().to_owned();
    initialize_test_document_with_context(schema, source, &context, &fingerprint)
}

fn initialize_test_document_with_context(
    schema: &Schema,
    source: serde_json::Value,
    artifact_context: &super::super::canonical::CanonicalSchemaContext,
    cache_fingerprint: &str,
) -> Option<DerivedStateCache> {
    let document = from_prosemirror_json(&source, schema, UnknownTypeMode::Preserve).unwrap();
    let doc = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(
                &fragment,
                &mut txn,
                &json!({ "type": "doc", "content": [] }),
                &source,
            )
            .unwrap();
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let artifact = artifact_context.derive(&document).unwrap();
    DerivedStateCache::initialize(
        document,
        artifact,
        &txn,
        &fragment,
        schema,
        &limits,
        &crate::yrs_engine::EditingLimits::default(),
        None,
        cache_fingerprint,
        0,
        0,
        0,
    )
}

fn document_and_yrs_from_json(
    schema: &Schema,
    source: &serde_json::Value,
    resource_limits: &ResourceLimits,
) -> (Document, Doc) {
    let document = from_prosemirror_json(source, schema, UnknownTypeMode::Preserve).unwrap();
    let doc = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    let codec = YrsDocumentCodec::new(schema, resource_limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(
                &fragment,
                &mut txn,
                &json!({ "type": "doc", "content": [] }),
                source,
            )
            .unwrap();
    }
    (document, doc)
}

#[test]
fn precomputed_document_history_charge_matches_legacy_helper_and_checks_overflow() {
    let schema = tiptap_schema();
    let state = initialize_test_document(
        &schema,
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "retained"}]
            }]
        }),
    )
    .unwrap();
    let charge = state
        .canonical_artifact
        .history_snapshot_retained_charge()
        .unwrap();
    let legacy = history_document_snapshot_retained_bytes_with_canonical_charge(
        &state.document,
        charge.canonical_retained_bytes,
        &state.position_map,
        &state.rendered_text,
        &state.render_blocks,
        &state.schema_fingerprint,
        "prosemirror",
        None,
    );
    let precomputed = history_document_snapshot_retained_bytes_with_precomputed_document_charge(
        charge.source_document_retained_bytes,
        charge.canonical_retained_bytes,
        &state.position_map,
        &state.rendered_text,
        &state.render_blocks,
        &state.schema_fingerprint,
        "prosemirror",
        None,
    );

    assert_eq!(precomputed, legacy);
    assert_eq!(
        history_document_snapshot_retained_bytes_with_precomputed_document_charge(
            usize::MAX,
            charge.canonical_retained_bytes,
            &state.position_map,
            &state.rendered_text,
            &state.render_blocks,
            &state.schema_fingerprint,
            "prosemirror",
            None,
        ),
        None
    );
}

#[test]
fn history_retention_refuses_a_mismatched_canonical_source_root_atomically() {
    let schema = tiptap_schema();
    let source = json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x"}]}]
    });
    let left = initialize_test_document(&schema, source.clone()).unwrap();
    let right = initialize_test_document(&schema, source).unwrap();
    crate::model::reset_history_snapshot_retained_bytes_traversals_for_test();

    let retained = history_document_snapshot_retained_bytes(HistoryDocumentSnapshotRetainedInput {
        document: &right.document,
        canonical_artifact: &left.canonical_artifact,
        position_map: &right.position_map,
        rendered_text: &right.rendered_text,
        render_blocks: &right.render_blocks,
        schema_fingerprint: &right.schema_fingerprint,
        fragment_name: "prosemirror",
        scope: None,
    });

    assert_eq!(retained, None);
    assert_eq!(
        crate::model::take_history_snapshot_retained_bytes_traversals_for_test(),
        0,
        "root identity must be rejected before any retained-size traversal"
    );
}

fn compiled_test_derivations(
    document: &Document,
    canonical_artifact: &CanonicalArtifact,
    schema: &Schema,
) -> CompiledDocumentDerivations {
    let rendered_text = crate::render::rendered_text(document, schema);
    CompiledDocumentDerivations {
        identity_seal: Arc::new(()),
        position_map: crate::position::build::build_position_map(document, schema),
        rendered_scalars: u32::try_from(rendered_text.chars().count()).unwrap(),
        rendered_text,
        document_text_bytes: canonical_artifact.text_utf8_bytes(),
        document_node_count: crate::editor_state::document_node_count(document.root()),
    }
}

fn initialize_test_document_with_limits(
    schema: &Schema,
    source: &serde_json::Value,
    canonical_schema: &super::super::canonical::CanonicalSchemaContext,
    schema_fingerprint: &str,
    resource_limits: &ResourceLimits,
) -> (Doc, DerivedStateCache) {
    let (document, doc) = document_and_yrs_from_json(schema, source, resource_limits);
    let canonical_artifact = canonical_schema.derive(&document).unwrap();
    let state = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        DerivedStateCache::initialize(
            document,
            canonical_artifact,
            &txn,
            &fragment,
            schema,
            resource_limits,
            &crate::yrs_engine::EditingLimits::default(),
            None,
            schema_fingerprint,
            0,
            0,
            0,
        )
        .unwrap()
    };
    (doc, state)
}

fn replace_yrs_json(
    doc: &Doc,
    schema: &Schema,
    previous: &serde_json::Value,
    next: &serde_json::Value,
    resource_limits: &ResourceLimits,
) {
    let codec = YrsDocumentCodec::new(schema, resource_limits);
    let mut txn = doc.transact_mut();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    codec
        .apply_json(&fragment, &mut txn, previous, next)
        .unwrap();
}

#[test]
fn initialize_rejects_a_canonical_artifact_from_another_schema() {
    let schema = tiptap_schema();
    let other_schema = crate::schema::presets::prosemirror_schema();
    let other_context = super::super::canonical::CanonicalSchemaContext::new(&other_schema);
    let fingerprint = crate::schema::schema_fingerprint(&schema);
    let source = json!({
        "type": "doc",
        "content": [{"type": "paragraph"}]
    });

    assert!(
        initialize_test_document_with_context(&schema, source, &other_context, &fingerprint,)
            .is_none()
    );
}

#[test]
fn validated_document_evidence_rejects_every_seal_and_mixed_report_tamper() {
    let schema = tiptap_schema();
    let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
    let canonical_schema = super::super::canonical::CanonicalSchemaContext::new(&schema);
    let resource_limits = ResourceLimits::default();
    let editing_limits = crate::yrs_engine::EditingLimits::default();
    let document = from_prosemirror_json(
        &json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{"type": "text", "text": "sealed"}]
            }]
        }),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let artifact = canonical_schema.derive(&document).unwrap();
    let validation =
        DocumentValidator::validate_report(&document, &schema, &resource_limits).unwrap();
    let yrs_doc = Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    });
    {
        let codec = YrsDocumentCodec::new(&schema, &resource_limits);
        let mut write = yrs_doc.transact_mut();
        let fragment = write.get_or_insert_xml_fragment("article");
        codec
            .apply_json(
                &fragment,
                &mut write,
                &json!({ "type": "doc", "content": [] }),
                artifact.value(),
            )
            .unwrap();
    }
    let txn = yrs_doc.transact();
    let fragment = txn.get_xml_fragment("article").unwrap();
    let evidence = ValidatedDocumentEvidence::mint(
        &document,
        document.root(),
        &artifact,
        validation,
        &resource_limits,
        &editing_limits,
        Some(64),
        &schema_fingerprint,
        &canonical_schema,
        "article",
        &txn,
        &fragment,
        5,
        7,
        11,
        13,
    )
    .expect("matching validated evidence should mint");

    let admits = |candidate: &ValidatedDocumentEvidence| {
        candidate
            .admitted_validation_report(
                &document,
                &artifact,
                &resource_limits,
                &editing_limits,
                Some(64),
                &schema_fingerprint,
                &canonical_schema,
                "article",
                &txn,
                &fragment,
                5,
                7,
                11,
                13,
            )
            .is_some()
    };
    assert!(admits(&evidence));
    for (seal, tampered) in evidence.tampered_for_test(&schema) {
        assert!(!admits(&tampered), "tampered {seal} seal was admitted");
        crate::yrs_engine::observability::reset_full_pass_counts_for_test();
        let initialized = DerivedStateCache::initialize_validated_candidate(
            document.clone(),
            artifact.clone(),
            &txn,
            &fragment,
            &schema,
            &resource_limits,
            &editing_limits,
            Some(64),
            &schema_fingerprint,
            ValidatedCandidateContext {
                evidence: &tampered,
                canonical_schema: &canonical_schema,
                fragment_name: "article",
                engine_epoch: 5,
            },
            7,
            11,
            13,
        );
        assert!(
            initialized.is_some(),
            "tampered {seal} evidence did not complete bounded fallback initialization"
        );
        let passes = crate::yrs_engine::observability::take_full_pass_counts_for_test();
        assert_eq!(passes.document_validations, 1, "tampered {seal}");
        assert_eq!(
            passes.validation_certificate_constructions, 1,
            "tampered {seal}"
        );
    }
}

#[test]
fn initialize_seals_revision_bound_render_blocks() {
    let schema = tiptap_schema();
    let fingerprint = crate::schema::schema_fingerprint(&schema);
    let context = super::super::canonical::CanonicalSchemaContext::new(&schema);
    let source = json!({
        "type": "doc",
        "content": [
            {"type": "paragraph", "content": [{"type": "text", "text": "one"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "two"}]}
        ]
    });
    let state = initialize_test_document_with_context(&schema, source, &context, &fingerprint)
        .expect("valid derived state");

    assert_eq!(
        state.render_blocks.materialize(),
        crate::render::incremental::render_blocks(&state.document, &schema)
    );
    assert_eq!(state.document_revision, 0);
    assert_eq!(state.schema_fingerprint, fingerprint);
    assert_eq!(state.validation_certificate.stats().node_count, 5);
    assert_eq!(state.validation_certificate.stats().max_depth, 3);
    assert_eq!(state.validation_certificate.document_revision(), 0);
    assert_eq!(state.validation_certificate.state_revision(), 0);
    assert_eq!(state.validation_certificate.yrs_state_epoch(), 0);
    assert_eq!(
        state.validation_certificate.canonical_fingerprint(),
        state.canonical_artifact.sha256()
    );
}

include!("derived_state_tests/text_index.rs");

include!("derived_state_tests/evidence_limits.rs");
