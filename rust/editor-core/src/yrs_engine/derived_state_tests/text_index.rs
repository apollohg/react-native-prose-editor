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
