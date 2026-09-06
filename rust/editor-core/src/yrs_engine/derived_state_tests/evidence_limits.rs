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
