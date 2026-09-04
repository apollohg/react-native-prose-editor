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

#[test]
fn localized_text_index_only_proves_strict_inside_same_marked_leaf() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let state = initialize_test_document(
        &schema,
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "a🙂\\\"b", "marks": [{"type": "bold"}]},
                    {"type": "hardBreak"},
                    {"type": "text", "text": "tail"}
                ]
            }]
        }),
    )
    .expect("valid indexed state");
    let leaf = state
        .localized_text_index
        .as_ref()
        .unwrap()
        .leaves()
        .first()
        .expect("first marked leaf");
    let at = leaf.doc_start() + 1;
    let marks = state.localized_text_index.as_ref().unwrap().leaves()[0]
        .resolve(&state.document, &state.position_map)
        .unwrap()
        .marks()
        .to_vec();

    let inserted = "‍x\\\"🙂";
    let proof = state
        .localized_insert_admission_for_test(at, inserted, &marks, &schema, &limits, Some(64), 0)
        .expect("strict-inside same-mark insertion is provable");
    assert_eq!(proof.leaf.block_index, 0);
    assert_eq!(proof.leaf.child_ordinal, 0);
    assert_eq!(proof.inserted_scalars, 5);
    assert_eq!(proof.inserted_utf8_bytes, inserted.len());
    assert_eq!(proof.inserted_utf16, 6);
    assert_eq!(proof.next_raw_text_scalars, 14);
    assert_eq!(proof.next_raw_text_utf8_bytes, 22);
    assert_eq!(proof.history_undo_units, 6);
    assert_eq!(proof.document_revision, state.document_revision);
    assert_eq!(proof.state_revision, state.state_revision);
    assert_eq!(proof.yrs_state_epoch, 0);
    assert_eq!(proof.selection, state.resolved_selection);
    assert_eq!(proof.relative_selection, state.relative_selection);
    assert_eq!(
        proof.stored_marks_sha256,
        state
            .stored_marks
            .as_deref()
            .map(|stored_marks| canonical_marks_sha256(stored_marks).unwrap())
    );
    assert_eq!(
        proof.canonical_fingerprint,
        state.canonical_artifact.sha256()
    );
    assert_eq!(
        proof.next_canonical_serialized_len,
        state.canonical_artifact.serialized_len() + proof.inserted_escaped_json_bytes
    );
    assert_eq!(
        proof.next_rendered_scalars,
        state.rendered_scalars + proof.inserted_scalars
    );
    assert!(Arc::ptr_eq(&proof.render_seal, &state.render_blocks));
    assert!(Arc::ptr_eq(&proof.lookup_seal, &state.mutation_lookup_seed));

    let (preview, step_map) = crate::transform::apply_step_canonical_marks(
        &state.document,
        &crate::transform::Step::InsertText {
            pos: at,
            text: inserted.into(),
            marks: marks.clone(),
        },
        &schema,
    )
    .unwrap();
    let full_stats = DocumentValidator::validate(&preview, &schema, &limits).unwrap();
    assert_eq!(full_stats, state.validation_certificate.stats());
    let full_artifact = state
        .canonical_artifact
        .schema_context()
        .derive(&preview)
        .unwrap();
    assert_eq!(full_artifact.text_scalar_len(), proof.next_raw_text_scalars);
    assert_eq!(
        full_artifact.text_utf8_bytes(),
        proof.next_raw_text_utf8_bytes
    );
    assert_eq!(
        full_artifact.serialized_len(),
        proof.next_canonical_serialized_len
    );
    let mut full_position_map = state.position_map.clone();
    full_position_map.update(
        &step_map,
        &state.document,
        &preview,
        UpdateMode::InlineTextOnly,
        &schema,
    );
    full_position_map.compact();
    assert_eq!(
        full_position_map.total_scalars(),
        proof.next_rendered_scalars
    );
    let full_rendered = crate::render::rendered_text(&preview, &schema);
    assert_eq!(
        u32::try_from(full_rendered.chars().count()).unwrap(),
        proof.next_rendered_scalars
    );
    let expected_selection = Selection::cursor(at + proof.inserted_scalars);
    let full_resolved = resolved_from_legacy_with_view(
        &preview,
        &expected_selection,
        &schema,
        &full_position_map,
        &full_rendered,
    )
    .unwrap();
    assert_eq!(full_resolved, proof.operation_result);
    assert!(state
        .localized_insert_admission_for_test(
            leaf.doc_start(),
            "x",
            &marks,
            &schema,
            &limits,
            Some(64),
            0,
        )
        .is_none());
    assert!(state
        .localized_insert_admission_for_test(at, "x", &marks, &schema, &limits, Some(9), 0)
        .is_none());
    let mut different_limits = limits.clone();
    different_limits.max_document_nodes -= 1;
    assert!(state
        .localized_insert_admission_for_test(
            at,
            "x",
            &marks,
            &schema,
            &different_limits,
            Some(64),
            0,
        )
        .is_none());
    assert!(state
        .localized_insert_admission_for_test(at, "x", &marks, &schema, &limits, Some(64), 1)
        .is_none());
    assert!(state
        .localized_insert_admission_for_test(
            leaf.doc_end(),
            "x",
            &marks,
            &schema,
            &limits,
            Some(64),
            0,
        )
        .is_none());
    assert!(state
        .localized_insert_admission_for_test(at, "x", &[], &schema, &limits, Some(64), 0,)
        .is_none());
    let mut stale_index = state;
    stale_index
        .localized_text_index
        .as_mut()
        .unwrap()
        .document_revision += 1;
    assert!(stale_index
        .localized_insert_admission_for_test(at, "x", &marks, &schema, &limits, Some(64), 0)
        .is_none());
}

#[test]
fn localized_text_index_maps_list_prefixes_without_admitting_void_boundaries() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let state = initialize_test_document(
        &schema,
        json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            {"type": "text", "text": "left"},
                            {"type": "hardBreak"},
                            {"type": "text", "text": "right"}
                        ]
                    }]
                }]
            }]
        }),
    )
    .expect("list fixture is valid");
    let index = state.localized_text_index.as_ref().unwrap();
    assert_eq!(index.leaves().len(), 2);
    let left = &index.leaves()[0];
    let right = &index.leaves()[1];
    assert!(left.scalar_start > 0, "list marker precedes text");
    assert_eq!(left.scalar_end - left.scalar_start, 4);
    assert_eq!(right.scalar_end - right.scalar_start, 5);
    assert_eq!(left.utf16_end - left.utf16_start, 4);
    assert_eq!(left.text_utf8_bytes, 4);
    assert!(state
        .localized_insert_admission_for_test(
            left.doc_start + 1,
            "🙂",
            &[],
            &schema,
            &limits,
            None,
            0,
        )
        .is_some());
    assert!(state
        .localized_insert_admission_for_test(left.doc_end, "x", &[], &schema, &limits, None, 0,)
        .is_none());
    assert!(right.doc_start > left.doc_end);
}

#[test]
fn localized_text_index_build_is_linear_and_lookup_is_logarithmic() {
    let schema = tiptap_schema();
    let content = (0..128)
        .map(|index| {
            json!({
                "type": "paragraph",
                "content": [{"type": "text", "text": format!("leaf-{index:03}")}]
            })
        })
        .collect::<Vec<_>>();
    reset_localized_index_metrics_for_test();
    let state = initialize_test_document(&schema, json!({"type": "doc", "content": content}))
        .expect("bounded index fixture");
    let (path_hops, build_visits, _, path_copy_elements, _) =
        take_localized_index_metrics_for_test();
    assert_eq!(path_hops, 256);
    assert_eq!(build_visits, 257);
    assert_eq!(path_copy_elements, 0);

    let index = state.localized_text_index.as_ref().unwrap();
    let last = index.leaves().last().unwrap();
    reset_localized_index_metrics_for_test();
    assert!(index.strict_inside(last.doc_start + 1).is_some());
    let (_, _, comparisons, _, _) = take_localized_index_metrics_for_test();
    assert!(
        comparisons <= 8,
        "128 sorted intervals need at most 8 probes"
    );
}

#[test]
fn localized_text_index_visits_nested_document_once_at_one_x_and_two_x_scale() {
    fn metrics(paragraphs: usize) -> (usize, usize) {
        let schema = tiptap_schema();
        let content = (0..paragraphs)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": format!("leaf-{index}")}]
                })
            })
            .collect::<Vec<_>>();
        reset_localized_index_metrics_for_test();
        let state = initialize_test_document(
            &schema,
            json!({
                "type": "doc",
                "content": [{"type": "blockquote", "content": content}]
            }),
        )
        .expect("nested fixture remains valid");
        assert_eq!(
            state.localized_text_index.as_ref().unwrap().leaves().len(),
            paragraphs
        );
        let (path_hops, node_visits, _, path_copy_elements, _) =
            take_localized_index_metrics_for_test();
        assert_eq!(path_copy_elements, 0);
        (path_hops, node_visits)
    }

    assert_eq!(metrics(64), (129, 130));
    assert_eq!(metrics(128), (257, 258));
}

#[test]
fn localized_text_index_deep_wide_block_discovery_has_zero_path_comparisons() {
    fn metrics(depth: usize, block_count: usize) -> (usize, usize, usize) {
        let schema = tiptap_schema();
        let paragraphs = (0..block_count)
            .map(|index| {
                json!({
                    "type": "paragraph",
                    "content": [{"type": "text", "text": format!("leaf-{index}")}]
                })
            })
            .collect::<Vec<_>>();
        let mut nested = json!({"type": "blockquote", "content": paragraphs});
        for _ in 1..depth {
            nested = json!({"type": "blockquote", "content": [nested]});
        }

        reset_localized_index_metrics_for_test();
        let state = initialize_test_document(&schema, json!({"type": "doc", "content": [nested]}))
            .expect("deep and wide fixture remains valid");
        assert_eq!(
            state.localized_text_index.as_ref().unwrap().leaves().len(),
            block_count
        );
        let (path_hops, node_visits, _, path_copy_elements, path_comparison_elements) =
            take_localized_index_metrics_for_test();
        assert_eq!(path_copy_elements, 0);
        (path_hops, node_visits, path_comparison_elements)
    }

    assert_eq!(metrics(24, 32), (24 + 64, 24 + 64 + 1, 0));
    assert_eq!(metrics(48, 64), (48 + 128, 48 + 128 + 1, 0));
}

#[test]
fn localized_text_index_deep_fragmented_leaf_scaling_has_no_path_copies() {
    fn metrics(depth: usize, leaf_count: usize) -> (usize, usize, usize, usize) {
        let schema = tiptap_schema();
        let leaves = (0..leaf_count)
            .map(|index| {
                json!({
                    "type": "text",
                    "text": "x",
                    "marks": [{
                        "type": "link",
                        "attrs": {"href": format!("https://example.test/{index}")}
                    }]
                })
            })
            .collect::<Vec<_>>();
        let mut nested = json!({"type": "paragraph", "content": leaves});
        for _ in 0..depth {
            nested = json!({"type": "blockquote", "content": [nested]});
        }

        reset_localized_index_metrics_for_test();
        let state = initialize_test_document(&schema, json!({"type": "doc", "content": [nested]}))
            .expect("deep fragmented fixture remains valid");
        let index = state.localized_text_index.as_ref().unwrap();
        assert_eq!(index.leaves().len(), leaf_count);
        assert_eq!(
            index.retained_bytes(),
            index
                .leaves
                .capacity()
                .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())
                .unwrap()
        );
        let (path_hops, node_visits, _, path_copy_elements, path_comparison_elements) =
            take_localized_index_metrics_for_test();
        assert_eq!(path_comparison_elements, 0);
        (
            path_hops,
            node_visits,
            path_copy_elements,
            index.retained_bytes(),
        )
    }

    let depth = 24;
    let one_x = metrics(depth, 32);
    let two_x = metrics(depth, 64);
    assert_eq!(one_x.0, depth + 32 + 1);
    assert_eq!(one_x.1, depth + 32 + 2);
    assert_eq!(one_x.2, 0);
    assert_eq!(two_x.0, depth + 64 + 1);
    assert_eq!(two_x.1, depth + 64 + 2);
    assert_eq!(two_x.2, 0);
    assert_eq!(
        one_x.3,
        32 * std::mem::size_of::<LocalizedTextLeafCertificate>()
    );
    assert_eq!(two_x.3, 2 * one_x.3);
}

#[test]
fn localized_text_index_retains_fixed_mark_digests_not_mark_payloads() {
    let schema = tiptap_schema();
    let content = (0..256)
        .map(|index| {
            json!({
                "type": "text",
                "text": "x",
                "marks": [{
                    "type": "link",
                    "attrs": {"href": format!("https://example.test/{index}/{}", "z".repeat(2048))}
                }]
            })
        })
        .collect::<Vec<_>>();
    let state = initialize_test_document(
        &schema,
        json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": content}]
        }),
    )
    .expect("large marked fixture remains valid");
    let index = state.localized_text_index.as_ref().unwrap();
    assert_eq!(index.leaves().len(), 256);
    assert!(index.retained_bytes() < 64 * 1024);
    for leaf in index.leaves() {
        let live = leaf.resolve(&state.document, &state.position_map).unwrap();
        assert_eq!(
            canonical_marks_sha256(live.marks()).unwrap(),
            leaf.marks_sha256
        );
    }
}

#[test]
fn localized_text_index_is_bounded_for_ascii_and_non_bmp_and_fails_closed_on_allocation() {
    let schema = tiptap_schema();
    for text in ["a".repeat(16_384), "🙂".repeat(4_096)] {
        let state = initialize_test_document(
            &schema,
            json!({
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": text}]}]
            }),
        )
        .expect("maximum-shaped text builds without per-scalar side arrays");
        assert!(
            state
                .localized_text_index
                .as_ref()
                .unwrap()
                .retained_bytes()
                <= ResourceLimits::default().max_input_bytes
        );
    }

    force_localized_index_allocation_failure_for_test(true);
    let failed = initialize_test_document(
        &schema,
        json!({
            "type": "doc",
            "content": [{"type": "paragraph", "content": [{"type": "text", "text": "abc"}]}]
        }),
    );
    force_localized_index_allocation_failure_for_test(false);
    let state = failed.expect("optional evidence failure must not reject derived state");
    assert!(state.localized_text_index.is_none());
}

#[test]
fn localized_text_index_fails_closed_at_each_fallible_allocation_stage() {
    let schema = tiptap_schema();
    for stage in [
        LocalizedIndexAllocationStage::InitialLeafCapacity,
        LocalizedIndexAllocationStage::TraversalPath,
        LocalizedIndexAllocationStage::LeafGrowth,
    ] {
        force_localized_index_allocation_stage_for_test(Some(stage));
        let state = initialize_test_document(
            &schema,
            json!({
                "type": "doc",
                "content": [{
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "ab"},
                        {"type": "hardBreak"},
                        {"type": "text", "text": "cd"}
                    ]
                }]
            }),
        )
        .expect("optional index allocation failure must not reject derived state");
        force_localized_index_allocation_stage_for_test(None);
        assert!(state.localized_text_index.is_none(), "stage {stage:?}");
    }
}

#[test]
fn initialize_rejects_supplied_fingerprint_that_does_not_identify_render_cache() {
    let schema = tiptap_schema();
    let other_context = super::super::canonical::CanonicalSchemaContext::new(
        &crate::schema::presets::prosemirror_schema(),
    );
    let other_fingerprint = other_context.schema_fingerprint().to_owned();
    let source = json!({
        "type": "doc",
        "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]
    });

    assert!(initialize_test_document_with_context(
        &schema,
        source,
        &other_context,
        &other_fingerprint,
    )
    .is_none());
}

#[test]
fn initialize_rejects_position_map_domain_larger_than_rendered_text() {
    let schema = tiptap_schema();
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abc" }]
        }]
    });
    FORCE_INITIALIZE_SCALAR_MISMATCH.with(|force| force.set(true));

    let initialized = initialize_test_document(&schema, source);

    FORCE_INITIALIZE_SCALAR_MISMATCH.with(|force| force.set(false));
    assert!(initialized.is_none());
}

#[test]
fn initialize_accepts_rendered_content_outside_the_position_map_domain() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "genericBlock",
                "content": "inline*",
                "group": "block",
                "role": "block"
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let initialized = initialize_test_document(
        &schema,
        json!({
            "type": "doc",
            "content": [{
                "type": "genericBlock",
                "content": [{ "type": "text", "text": "visible but not addressable" }]
            }]
        }),
    );

    assert!(initialized.is_some());
}

#[test]
fn generic_structural_evidence_uses_current_loosened_limits_and_reuses() {
    let schema = tiptap_schema();
    let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
    let canonical_schema = super::super::canonical::CanonicalSchemaContext::new(&schema);
    let base_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "base"}]
        }]
    });
    let candidate_source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{"type": "text", "text": "first"}]
            },
            {
                "type": "paragraph",
                "content": [{"type": "text", "text": "second"}]
            }
        ]
    });
    let base_document =
        from_prosemirror_json(&base_source, &schema, UnknownTypeMode::Preserve).unwrap();
    let candidate_document =
        from_prosemirror_json(&candidate_source, &schema, UnknownTypeMode::Preserve).unwrap();
    let base_node_count = crate::editor_state::document_node_count(base_document.root());
    let candidate_node_count = crate::editor_state::document_node_count(candidate_document.root());
    assert!(candidate_node_count > base_node_count);
    let old_limits = ResourceLimits {
        max_document_nodes: base_node_count,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: candidate_node_count,
        ..old_limits.clone()
    };
    let one_under = ResourceLimits {
        max_document_nodes: candidate_node_count - 1,
        ..current_limits.clone()
    };

    let (yrs_doc, base_state) = initialize_test_document_with_limits(
        &schema,
        &base_source,
        &canonical_schema,
        &schema_fingerprint,
        &old_limits,
    );
    let candidate_artifact = canonical_schema.derive(&candidate_document).unwrap();
    let candidate_derivations =
        compiled_test_derivations(&candidate_document, &candidate_artifact, &schema);
    let candidate_render_blocks = Arc::new(
        crate::render::incremental::CachedRenderBlocks::build(
            &candidate_document,
            &schema,
            &current_limits,
        )
        .unwrap(),
    );
    let authority =
        super::super::prepared_admission::InstalledDerivedStateAuthority::new(&base_state);
    let prior_certificate = base_state.validation_certificate.clone();
    assert!(base_state
        .prepare_generic_derived_evidence(
            7_000,
            &authority,
            &candidate_document,
            &candidate_artifact,
            &candidate_derivations,
            &schema,
            &one_under,
            &schema_fingerprint,
            1,
            1,
            1,
        )
        .is_none());
    assert_eq!(base_state.validation_certificate, prior_certificate);
    let evidence = base_state
        .prepare_generic_derived_evidence(
            7_001,
            &authority,
            &candidate_document,
            &candidate_artifact,
            &candidate_derivations,
            &schema,
            &current_limits,
            &schema_fingerprint,
            1,
            1,
            1,
        )
        .expect("candidate fitting current loosened limits must produce evidence");
    assert_eq!(
        evidence.validation_certificate.resource_limits,
        current_limits
    );
    assert_eq!(
        base_state.validation_certificate.resource_limits,
        old_limits
    );

    replace_yrs_json(
        &yrs_doc,
        &schema,
        &base_source,
        &candidate_source,
        &current_limits,
    );
    let fallback = base_state.legacy_selection();
    let next_state = {
        let txn = yrs_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        base_state
            .after_document_change(
                candidate_document,
                candidate_artifact,
                &txn,
                &fragment,
                &schema,
                &schema_fingerprint,
                &current_limits,
                &crate::yrs_engine::EditingLimits::default(),
                None,
                candidate_render_blocks,
                Some(candidate_derivations),
                &StepMap::empty(),
                UpdateMode::Rebuild,
                &[],
                None,
                Some(&fallback),
                false,
                None,
                None,
                Some(evidence),
                1,
                1,
                1,
            )
            .expect("current-limit evidence must install")
    };
    assert_eq!(
        next_state.validation_certificate.resource_limits,
        current_limits
    );
    assert_eq!(
        base_state.validation_certificate.resource_limits,
        old_limits
    );

    let reuse_source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{"type": "text", "text": "changed"}]
            },
            {
                "type": "paragraph",
                "content": [{"type": "text", "text": "again"}]
            }
        ]
    });
    let reuse_document =
        from_prosemirror_json(&reuse_source, &schema, UnknownTypeMode::Preserve).unwrap();
    let reuse_artifact = canonical_schema.derive(&reuse_document).unwrap();
    let reuse_derivations = compiled_test_derivations(&reuse_document, &reuse_artifact, &schema);
    let reuse_authority =
        super::super::prepared_admission::InstalledDerivedStateAuthority::new(&next_state);
    let reused = next_state
        .prepare_generic_derived_evidence(
            7_002,
            &reuse_authority,
            &reuse_document,
            &reuse_artifact,
            &reuse_derivations,
            &schema,
            &current_limits,
            &schema_fingerprint,
            2,
            2,
            2,
        )
        .expect("installed current-limit certificate must support the next evidence build");
    assert_eq!(
        reused.validation_certificate.resource_limits,
        current_limits
    );
}

#[test]
fn remote_style_fallback_uses_current_limits_and_rejects_one_under_atomically() {
    let schema = tiptap_schema();
    let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
    let canonical_schema = super::super::canonical::CanonicalSchemaContext::new(&schema);
    let base_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{"type": "text", "text": "base"}]
        }]
    });
    let candidate_source = json!({
        "type": "doc",
        "content": [
            {
                "type": "paragraph",
                "content": [{"type": "text", "text": "remote one"}]
            },
            {
                "type": "paragraph",
                "content": [{"type": "text", "text": "remote two"}]
            }
        ]
    });
    let base_document =
        from_prosemirror_json(&base_source, &schema, UnknownTypeMode::Preserve).unwrap();
    let candidate_document =
        from_prosemirror_json(&candidate_source, &schema, UnknownTypeMode::Preserve).unwrap();
    let base_node_count = crate::editor_state::document_node_count(base_document.root());
    let candidate_node_count = crate::editor_state::document_node_count(candidate_document.root());
    assert!(candidate_node_count > base_node_count);
    let old_limits = ResourceLimits {
        max_document_nodes: base_node_count,
        ..ResourceLimits::default()
    };
    let current_limits = ResourceLimits {
        max_document_nodes: candidate_node_count,
        ..old_limits.clone()
    };
    let one_under = ResourceLimits {
        max_document_nodes: candidate_node_count - 1,
        ..current_limits.clone()
    };

    let (yrs_doc, base_state) = initialize_test_document_with_limits(
        &schema,
        &base_source,
        &canonical_schema,
        &schema_fingerprint,
        &old_limits,
    );
    replace_yrs_json(
        &yrs_doc,
        &schema,
        &base_source,
        &candidate_source,
        &current_limits,
    );
    let candidate_artifact = canonical_schema.derive(&candidate_document).unwrap();
    let candidate_render_blocks = Arc::new(
        crate::render::incremental::CachedRenderBlocks::build(
            &candidate_document,
            &schema,
            &current_limits,
        )
        .unwrap(),
    );
    let fallback = base_state.legacy_selection();
    let prior_document = base_state.document.clone();
    let prior_certificate = base_state.validation_certificate.clone();
    let next_state = {
        let txn = yrs_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        base_state
            .after_document_change(
                candidate_document.clone(),
                candidate_artifact.clone(),
                &txn,
                &fragment,
                &schema,
                &schema_fingerprint,
                &current_limits,
                &crate::yrs_engine::EditingLimits::default(),
                None,
                Arc::clone(&candidate_render_blocks),
                Some(compiled_test_derivations(
                    &candidate_document,
                    &candidate_artifact,
                    &schema,
                )),
                &StepMap::empty(),
                UpdateMode::Rebuild,
                &[],
                None,
                Some(&fallback),
                false,
                None,
                None,
                None,
                1,
                1,
                1,
            )
            .expect("remote-style fallback must mint against current loosened limits")
    };
    assert_eq!(
        next_state.validation_certificate.resource_limits,
        current_limits
    );

    let rejected = {
        let txn = yrs_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        base_state.after_document_change(
            candidate_document.clone(),
            candidate_artifact.clone(),
            &txn,
            &fragment,
            &schema,
            &schema_fingerprint,
            &one_under,
            &crate::yrs_engine::EditingLimits::default(),
            None,
            candidate_render_blocks,
            Some(compiled_test_derivations(
                &candidate_document,
                &candidate_artifact,
                &schema,
            )),
            &StepMap::empty(),
            UpdateMode::Rebuild,
            &[],
            None,
            Some(&fallback),
            false,
            None,
            None,
            None,
            1,
            1,
            1,
        )
    };
    assert!(rejected.is_none());
    assert_eq!(base_state.validation_certificate, prior_certificate);
    assert_eq!(
        base_state.validation_certificate.resource_limits,
        old_limits
    );
    assert!(base_state
        .document
        .shares_root_storage_with(&prior_document));
}

#[test]
fn active_state_retained_meter_has_exact_deep_capacity_depth_and_cardinality_bounds() {
    let mut mark_attrs = std::collections::HashMap::new();
    mark_attrs.insert(
        "link".to_string(),
        json!({
            "href": "https://example.invalid/".repeat(32),
            "metadata": [{
                "labels": ["alpha".repeat(24), "beta".repeat(24)],
                "nested": { "value": "payload".repeat(48) }
            }]
        }),
    );
    let state = ActiveState {
        marks: [("bold".to_string(), true), ("link".to_string(), true)]
            .into_iter()
            .collect(),
        mark_attrs,
        nodes: [("paragraph".to_string(), true)].into_iter().collect(),
        commands: [("undo".to_string(), true), ("redo".to_string(), false)]
            .into_iter()
            .collect(),
        allowed_marks: vec!["bold".repeat(8), "link".repeat(8)],
        insertable_nodes: vec!["paragraph".repeat(8), "image".repeat(8)],
    };
    let defaults = ResourceLimits::default();
    let retained = active_state_retained_bytes(&state, &defaults).unwrap();
    assert!(retained > super::super::operation::active_state_bytes(&state));

    let mut exact_resources = defaults.clone();
    exact_resources.max_input_bytes = retained;
    let exact_editing = super::super::EditingLimits {
        max_derived_output_bytes: retained,
        ..super::super::EditingLimits::default()
    };
    let exact = CachedActiveState::try_new(state.clone(), &exact_resources, &exact_editing)
        .expect("exact retained-byte budget must admit");
    assert_eq!(exact.retained_bytes_for_test(), retained);
    assert_eq!(
        exact
            .clone_public(&exact_resources, &exact_editing)
            .unwrap(),
        state
    );

    let mut under_editing = exact_editing.clone();
    under_editing.max_derived_output_bytes = retained - 1;
    assert!(CachedActiveState::try_new(state.clone(), &exact_resources, &under_editing).is_err());
    assert!(exact
        .clone_public(&exact_resources, &under_editing)
        .is_none());

    let mut under_resources = defaults.clone();
    under_resources.max_input_bytes = retained - 1;
    assert!(CachedActiveState::try_new(state.clone(), &under_resources, &exact_editing).is_err());

    let mut shallow = defaults.clone();
    shallow.max_document_depth = 2;
    assert!(active_state_retained_bytes(&state, &shallow).is_none());
    let mut narrow = defaults;
    narrow.max_document_nodes = 4;
    assert!(active_state_retained_bytes(&state, &narrow).is_none());
}
