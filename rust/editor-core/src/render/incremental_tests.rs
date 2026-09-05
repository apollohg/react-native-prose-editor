use std::collections::HashMap;
use std::sync::Arc;

use proptest::prelude::*;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::render::incremental::{
    render_blocks, try_render_blocks, CachedRenderBlocks, CachedRenderTransitionUpdate,
};
use crate::render::RenderElement;
use crate::{prosemirror_schema, tiptap_schema};

fn text(value: &str) -> Node {
    Node::text(value.to_string(), vec![])
}

fn paragraph(children: Vec<Node>) -> Node {
    Node::element(
        "paragraph".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn doc(children: Vec<Node>) -> Document {
    Document::new(Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(children),
    ))
}

fn replace_top_level(document: &Document, index: usize, replacement: Node) -> Document {
    let mut children = (0..document.root().child_count())
        .map(|child_index| document.root().child(child_index).unwrap().clone())
        .collect::<Vec<_>>();
    children[index] = replacement;
    doc(children)
}

fn inline_atom(label: &str) -> Node {
    Node::void(
        "__opaque_json".to_string(),
        HashMap::from([
            (
                "opaque_placement".to_string(),
                serde_json::Value::String("inline".to_string()),
            ),
            (
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            ),
        ]),
    )
}

fn opaque_block_atom(label: &str) -> Node {
    Node::void(
        "__opaque_json".to_string(),
        HashMap::from([
            (
                "opaque_placement".to_string(),
                serde_json::Value::String("block".to_string()),
            ),
            (
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            ),
        ]),
    )
}

fn hard_break() -> Node {
    Node::void("hardBreak".to_string(), HashMap::new())
}

fn horizontal_rule() -> Node {
    Node::void("horizontalRule".to_string(), HashMap::new())
}

fn bullet_list(children: Vec<Node>) -> Node {
    Node::element(
        "bulletList".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn ordered_list(start: u32, children: Vec<Node>) -> Node {
    ordered_list_with_start(Some(serde_json::Value::Number(start.into())), children)
}

fn ordered_list_with_start(start: Option<serde_json::Value>, children: Vec<Node>) -> Node {
    let mut attrs = HashMap::new();
    if let Some(start) = start {
        attrs.insert("start".to_string(), start);
    }
    Node::element("orderedList".to_string(), attrs, Fragment::from(children))
}

fn list_item(children: Vec<Node>) -> Node {
    Node::element(
        "listItem".to_string(),
        HashMap::new(),
        Fragment::from(children),
    )
}

fn assert_update_reconstructs(
    old_render: Vec<Vec<RenderElement>>,
    transition: &super::CachedRenderTransition,
    expected: &[Vec<RenderElement>],
) {
    let reconstructed = match &transition.update {
        CachedRenderTransitionUpdate::None => old_render,
        CachedRenderTransitionUpdate::Patch(patch) => {
            let mut blocks = old_render;
            let end = patch
                .start_index
                .checked_add(patch.delete_count)
                .expect("test patch range should not overflow");
            blocks.splice(patch.start_index..end, patch.blocks.clone());
            blocks
        }
        CachedRenderTransitionUpdate::Full(blocks) => blocks.clone(),
    };
    assert_eq!(reconstructed, expected);
    assert_eq!(transition.cache.materialize(), expected);
}

#[test]
fn legacy_safe_patch_counter_counts_only_its_old_and_new_full_render_passes() {
    let schema = tiptap_schema();
    let old_doc = doc(vec![paragraph(vec![text("old")])]);
    let new_doc = doc(vec![paragraph(vec![text("new")])]);
    super::reset_cached_render_counts_for_test();

    super::safe_contiguous_render_blocks_patch(&old_doc, &new_doc, &schema, &[0])
        .expect("valid hint should produce a safe patch");

    assert_eq!(super::take_cached_render_counts_for_test(), (0, 0, 0, 0, 2));
}

#[test]
fn cached_render_slow_invariant_detects_private_block_tampering() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("second")]),
    ]);
    let mut cache = CachedRenderBlocks::build(&document, &schema, &limits).unwrap();

    assert!(cache.verify_slow_invariant(&document, &schema));
    let sealed_schema = Arc::clone(&cache.schema_fingerprint);
    cache.schema_fingerprint = Arc::<str>::from("tampered-schema");
    assert!(!cache.verify_slow_invariant(&document, &schema));
    cache.schema_fingerprint = sealed_schema;
    let sealed_root = cache.document_root_seal.clone();
    let foreign = doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("second")]),
    ]);
    cache.document_root_seal = foreign.root().clone();
    assert!(!cache.verify_slow_invariant(&document, &schema));
    cache.document_root_seal = sealed_root;
    let removed_block = cache.blocks.pop().unwrap();
    assert!(!cache.verify_slow_invariant(&document, &schema));
    cache.blocks.push(removed_block);
    let sealed_node = Arc::clone(&cache.blocks[0].node);
    cache.blocks[0].node = Arc::new(paragraph(vec![text("tampered")]));
    assert!(!cache.verify_slow_invariant(&document, &schema));
    cache.blocks[0].node = sealed_node;
    let sealed_node_size = cache.blocks[0].node_size;
    cache.blocks[0].node_size = cache.blocks[0].node_size.saturating_add(1);
    assert!(!cache.verify_slow_invariant(&document, &schema));
    cache.blocks[0].node_size = sealed_node_size;
    cache.blocks[1].start_pos = cache.blocks[1].start_pos.saturating_add(1);
    assert!(!cache.verify_slow_invariant(&document, &schema));
}

#[test]
fn cached_render_identity_accepts_only_the_sealed_root_and_schema() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = doc(vec![paragraph(vec![text("same")])]);
    let shared = document.clone();
    let foreign = doc(vec![paragraph(vec![text("same")])]);
    let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
    let cache = CachedRenderBlocks::build(&document, &schema, &limits).unwrap();

    assert_eq!(document, foreign);
    assert!(!document.root().shares_storage_with(foreign.root()));
    assert!(cache.matches_identity(&shared, &schema_fingerprint));
    assert!(!cache.matches_identity(&foreign, &schema_fingerprint));
    assert!(!cache.matches_identity(&shared, "foreign-schema"));
}

#[test]
fn cached_render_build_transition_and_full_fallback_propagate_identity_seals() {
    let schema = tiptap_schema();
    let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
    let limits = ResourceLimits::default();
    let old_document = doc(vec![paragraph(vec![text("old")])]);
    let new_document = doc(vec![paragraph(vec![text("new")])]);
    let foreign_new = doc(vec![paragraph(vec![text("new")])]);
    let cache = CachedRenderBlocks::build(&old_document, &schema, &limits).unwrap();

    assert!(cache.matches_identity(&old_document, &schema_fingerprint));
    let transition = cache
        .transition(&old_document, &new_document, &schema, &[0], &limits)
        .unwrap();
    assert!(transition
        .cache
        .matches_identity(&new_document, &schema_fingerprint));
    assert!(!transition
        .cache
        .matches_identity(&foreign_new, &schema_fingerprint));

    let fallback = cache
        .transition(&old_document, &new_document, &schema, &[1], &limits)
        .unwrap();
    assert!(matches!(
        fallback.update,
        CachedRenderTransitionUpdate::Full(_)
    ));
    assert!(fallback
        .cache
        .matches_identity(&new_document, &schema_fingerprint));
    assert!(!fallback
        .cache
        .matches_identity(&foreign_new, &schema_fingerprint));

    let shared_old = old_document.clone();
    let unchanged = cache
        .transition(&old_document, &shared_old, &schema, &[], &limits)
        .unwrap();
    assert!(unchanged
        .cache
        .matches_identity(&shared_old, &schema_fingerprint));

    let deep_equal_old = doc(vec![paragraph(vec![text("old")])]);
    assert_eq!(old_document, deep_equal_old);
    assert!(!old_document
        .root()
        .shares_storage_with(deep_equal_old.root()));
    let resealed_unchanged = cache
        .transition(&old_document, &deep_equal_old, &schema, &[], &limits)
        .unwrap();
    assert!(resealed_unchanged
        .cache
        .matches_identity(&deep_equal_old, &schema_fingerprint));
    assert!(!resealed_unchanged
        .cache
        .matches_identity(&old_document, &schema_fingerprint));

    let new_schema = prosemirror_schema();
    let new_schema_fingerprint = crate::schema::schema_fingerprint(&new_schema);
    let schema_fallback = cache
        .transition(&old_document, &old_document, &new_schema, &[], &limits)
        .unwrap();
    assert!(schema_fallback
        .cache
        .matches_identity(&old_document, &new_schema_fingerprint));
    assert!(!schema_fallback
        .cache
        .matches_identity(&old_document, &schema_fingerprint));
}

#[test]
fn cached_render_build_and_every_transition_run_the_slow_debug_verifier() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let old_document = doc(vec![paragraph(vec![text("old")])]);
    let new_document = doc(vec![paragraph(vec![text("new")])]);

    super::reset_slow_invariant_checks_for_test();
    let cache = CachedRenderBlocks::build(&old_document, &schema, &limits).unwrap();
    assert_eq!(super::take_slow_invariant_checks_for_test(), 1);

    super::reset_slow_invariant_checks_for_test();
    cache
        .transition(&old_document, &new_document, &schema, &[0], &limits)
        .unwrap();
    assert_eq!(super::take_slow_invariant_checks_for_test(), 1);

    super::reset_slow_invariant_checks_for_test();
    let foreign_old = doc(vec![paragraph(vec![text("old")])]);
    cache
        .transition(&foreign_old, &new_document, &schema, &[0], &limits)
        .unwrap();
    assert_eq!(
        super::take_slow_invariant_checks_for_test(),
        2,
        "full fallback verifies both its rebuilt cache and transition result"
    );

    super::reset_slow_invariant_checks_for_test();
    cache
        .transition(&old_document, &old_document, &schema, &[], &limits)
        .unwrap();
    assert_eq!(super::take_slow_invariant_checks_for_test(), 1);
}

#[test]
fn localized_render_transition_matches_generic_for_supported_insert_shapes() {
    fn assert_parity(old: Document, new: Document, target: usize, inserted_scalars: u32) {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let cache = CachedRenderBlocks::build(&old, &schema, &limits).unwrap();
        let affected = target.saturating_sub(1)..old.root().child_count();
        let affected = affected.collect::<Vec<_>>();
        let specialized = cache
            .transition_localized_insert(&old, &new, &schema, target, inserted_scalars, &limits)
            .unwrap();
        let generic = cache.transition(&old, &new, &schema, &[], &limits).unwrap();
        assert_eq!(specialized.update, generic.update);
        assert_eq!(specialized.cache.materialize(), generic.cache.materialize());
        assert_eq!(specialized.rerendered_new_blocks, 1);
        if old.root().child_count() == 160 {
            let CachedRenderTransitionUpdate::Patch(patch) = &specialized.update else {
                panic!("wide localized insert must retain the generic patch contract");
            };
            assert_eq!(patch.start_index, target);
            assert_eq!(patch.delete_count, 1);
            assert_eq!(patch.blocks.len(), 1);
            let conservative =
                super::classify_cached_transition(&cache, &specialized.cache, &affected, true);
            assert_ne!(conservative, specialized.update);
            let CachedRenderTransitionUpdate::Patch(conservative) = conservative else {
                panic!("conservative range should widen the patch for this fixture");
            };
            assert!(conservative.delete_count > patch.delete_count);
            assert!(conservative.blocks.len() > patch.blocks.len());
        }
    }

    let three = doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("middle")]),
        paragraph(vec![text("last")]),
    ]);
    for (target, replacement) in [(0, "firstx"), (1, "middlex"), (2, "lastx")] {
        assert_parity(
            three.clone(),
            replace_top_level(&three, target, paragraph(vec![text(replacement)])),
            target,
            1,
        );
    }

    let bold = Mark::new("bold".to_string(), HashMap::new());
    let fragmented = doc(vec![paragraph(vec![
        Node::text("ab".to_string(), vec![bold.clone()]),
        Node::text("cd".to_string(), vec![]),
    ])]);
    assert_parity(
        fragmented.clone(),
        replace_top_level(
            &fragmented,
            0,
            paragraph(vec![
                Node::text("ab".to_string(), vec![bold]),
                Node::text("c🙂\\\"\n\u{1}d".to_string(), vec![]),
            ]),
        ),
        0,
        5,
    );

    let nested = doc(vec![bullet_list(vec![
        list_item(vec![paragraph(vec![text("one")])]),
        list_item(vec![paragraph(vec![text("two")])]),
    ])]);
    assert_parity(
        nested.clone(),
        replace_top_level(
            &nested,
            0,
            bullet_list(vec![
                list_item(vec![paragraph(vec![text("one")])]),
                list_item(vec![paragraph(vec![text("twox")])]),
            ]),
        ),
        0,
        1,
    );

    let positioned_suffix = doc(vec![
        paragraph(vec![text("edit")]),
        paragraph(vec![text("later"), hard_break(), inline_atom("mention")]),
        horizontal_rule(),
        opaque_block_atom("trailing"),
    ]);
    assert_parity(
        positioned_suffix.clone(),
        replace_top_level(
            &positioned_suffix,
            0,
            paragraph(vec![text("edit expanded")]),
        ),
        0,
        9,
    );

    let wide = doc((0..160)
        .map(|index| paragraph(vec![text(&format!("block {index}"))]))
        .collect());
    assert_parity(
        wide.clone(),
        replace_top_level(&wide, 80, paragraph(vec![text("block 80x")])),
        80,
        1,
    );
}

#[test]
fn localized_render_transition_accepts_exact_element_capacity_and_rejects_one_under() {
    let schema = tiptap_schema();
    let default_limits = ResourceLimits::default();
    let old = doc(vec![paragraph(vec![
        text("a"),
        hard_break(),
        inline_atom("mention"),
        text("b"),
        hard_break(),
        inline_atom("emoji"),
        text("c"),
        hard_break(),
        inline_atom("mention"),
    ])]);
    let new = replace_top_level(
        &old,
        0,
        paragraph(vec![
            text("ax"),
            hard_break(),
            inline_atom("mention"),
            text("b"),
            hard_break(),
            inline_atom("emoji"),
            text("c"),
            hard_break(),
            inline_atom("mention"),
        ]),
    );
    let cache = CachedRenderBlocks::build(&old, &schema, &default_limits).unwrap();
    let new_cache = CachedRenderBlocks::build(&new, &schema, &default_limits).unwrap();
    let old_materialized = cache.materialize();
    let new_materialized = new_cache.materialize();
    let required_elements = old_materialized
        .iter()
        .chain(new_materialized.iter())
        .map(Vec::len)
        .max()
        .unwrap();
    let exact_nodes = required_elements.div_ceil(3);
    assert!(
        exact_nodes > 1,
        "fixture must make one-under resource-bound"
    );
    let exact = ResourceLimits {
        max_document_nodes: exact_nodes,
        ..default_limits.clone()
    };
    let one_under = ResourceLimits {
        max_document_nodes: exact_nodes - 1,
        ..default_limits
    };

    assert!(cache
        .transition_localized_insert(&old, &new, &schema, 0, 1, &exact)
        .is_ok());
    assert!(matches!(
        cache.transition_localized_insert(&old, &new, &schema, 0, 1, &one_under),
        Err(super::CachedRenderError::ResourceLimitExceeded)
    ));
}

#[test]
fn localized_render_transition_rejects_unsealed_shape_and_delta_facts() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let old = doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("middle")]),
        paragraph(vec![text("last")]),
    ]);
    let new = replace_top_level(&old, 1, paragraph(vec![text("middlex")]));
    let cache = CachedRenderBlocks::build(&old, &schema, &limits).unwrap();

    assert!(matches!(
        cache.transition_localized_insert(&old, &new, &schema, 1, 2, &limits),
        Err(super::CachedRenderError::CacheInvariantViolation)
    ));
    assert!(matches!(
        cache.transition_localized_insert(&old, &new, &schema, 3, 1, &limits),
        Err(super::CachedRenderError::CacheInvariantViolation)
    ));

    let changed_cardinality = doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("middlex")]),
        paragraph(vec![text("last")]),
        paragraph(vec![text("extra")]),
    ]);
    assert!(matches!(
        cache.transition_localized_insert(&old, &changed_cardinality, &schema, 1, 1, &limits,),
        Err(super::CachedRenderError::CacheInvariantViolation)
    ));

    let foreign_unchanged_blocks = doc(vec![
        paragraph(vec![text("first")]),
        paragraph(vec![text("middlex")]),
        paragraph(vec![text("last")]),
    ]);
    assert!(matches!(
        cache.transition_localized_insert(&old, &foreign_unchanged_blocks, &schema, 1, 1, &limits,),
        Err(super::CachedRenderError::CacheInvariantViolation)
    ));
}

#[test]
fn cached_transition_rerenders_early_text_and_rebases_later_atoms() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let old_doc = doc(vec![
        paragraph(vec![text("one")]),
        paragraph(vec![text("middle")]),
        paragraph(vec![text("before "), inline_atom("mention")]),
    ]);
    let new_doc = doc(vec![
        paragraph(vec![text("one expanded")]),
        paragraph(vec![text("middle")]),
        paragraph(vec![text("before "), inline_atom("mention")]),
    ]);
    let old_render = render_blocks(&old_doc, &schema);
    let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits)
        .expect("old document should be cacheable");

    let transition = cache
        .transition(&old_doc, &new_doc, &schema, &[0], &limits)
        .expect("transition should be cacheable");
    let new_render = render_blocks(&new_doc, &schema);

    assert_eq!(cache.materialize(), old_render);
    assert_eq!(transition.cache.materialize(), new_render);
    assert_eq!(transition.rerendered_new_blocks, 1);
    let CachedRenderTransitionUpdate::Patch(patch) = transition.update else {
        panic!("expected an exact contiguous patch");
    };
    let mut reconstructed = old_render;
    reconstructed.splice(
        patch.start_index..patch.start_index + patch.delete_count,
        patch.blocks,
    );
    assert_eq!(reconstructed, new_render);
}

#[test]
fn cached_transition_rebases_every_position_bearing_render_variant() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let old_doc = doc(vec![
        paragraph(vec![text("a")]),
        paragraph(vec![text("later"), hard_break(), inline_atom("inline")]),
        horizontal_rule(),
        opaque_block_atom("block"),
    ]);
    let new_doc = doc(vec![
        paragraph(vec![text("a much longer prefix")]),
        paragraph(vec![text("later"), hard_break(), inline_atom("inline")]),
        horizontal_rule(),
        opaque_block_atom("block"),
    ]);
    let old_render = render_blocks(&old_doc, &schema);
    let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
    let transition = cache
        .transition(&old_doc, &new_doc, &schema, &[0], &limits)
        .unwrap();
    let expected = render_blocks(&new_doc, &schema);

    assert_eq!(transition.rerendered_new_blocks, 1);
    assert_update_reconstructs(old_render.clone(), &transition, &expected);

    let reverse = transition
        .cache
        .transition(&new_doc, &old_doc, &schema, &[0], &limits)
        .unwrap();
    assert_eq!(reverse.rerendered_new_blocks, 1);
    assert_update_reconstructs(expected, &reverse, &old_render);
}

#[test]
fn cached_transition_handles_mark_only_change() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let old_doc = doc(vec![paragraph(vec![text("marked")])]);
    let mark = Mark::new("bold".to_string(), HashMap::new());
    let new_doc = doc(vec![paragraph(vec![Node::text(
        "marked".to_string(),
        vec![mark],
    )])]);
    let old_render = render_blocks(&old_doc, &schema);
    let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
    let transition = cache
        .transition(&old_doc, &new_doc, &schema, &[0], &limits)
        .unwrap();
    let expected = render_blocks(&new_doc, &schema);

    assert_eq!(transition.rerendered_new_blocks, 1);
    assert!(matches!(
        transition.update,
        CachedRenderTransitionUpdate::Patch(_)
    ));
    assert_update_reconstructs(old_render, &transition, &expected);
}

#[test]
fn cached_transition_handles_top_level_insert_and_delete() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let initial = doc(vec![
        paragraph(vec![text("one")]),
        paragraph(vec![text("three")]),
    ]);
    let inserted = doc(vec![
        paragraph(vec![text("one")]),
        paragraph(vec![text("two")]),
        paragraph(vec![text("three")]),
    ]);

    let initial_render = render_blocks(&initial, &schema);
    let initial_cache = CachedRenderBlocks::build(&initial, &schema, &limits).unwrap();
    let insertion = initial_cache
        .transition(&initial, &inserted, &schema, &[1], &limits)
        .unwrap();
    let inserted_render = render_blocks(&inserted, &schema);
    assert_eq!(insertion.rerendered_new_blocks, 1);
    assert_update_reconstructs(initial_render, &insertion, &inserted_render);

    let deletion = insertion
        .cache
        .transition(&inserted, &initial, &schema, &[1], &limits)
        .unwrap();
    assert_eq!(deletion.rerendered_new_blocks, 0);
    assert_update_reconstructs(
        inserted_render,
        &deletion,
        &render_blocks(&initial, &schema),
    );
}

#[test]
fn cached_transition_handles_lists_and_rebases_later_atom() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let list = |first: &str| {
        bullet_list(vec![
            list_item(vec![paragraph(vec![text(first)])]),
            list_item(vec![paragraph(vec![text("second")])]),
        ])
    };
    let old_doc = doc(vec![list("first"), paragraph(vec![inline_atom("later")])]);
    let new_doc = doc(vec![
        list("first item expanded"),
        paragraph(vec![inline_atom("later")]),
    ]);
    let old_render = render_blocks(&old_doc, &schema);
    let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
    let transition = cache
        .transition(&old_doc, &new_doc, &schema, &[0], &limits)
        .unwrap();
    let expected = render_blocks(&new_doc, &schema);

    assert_eq!(transition.rerendered_new_blocks, 1);
    assert_update_reconstructs(old_render, &transition, &expected);
}

#[test]
fn cached_transition_classifies_net_zero_as_none_even_with_invalid_hint() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = doc(vec![paragraph(vec![text("same")])]);
    let cache = CachedRenderBlocks::build(&document, &schema, &limits).unwrap();
    let transition = cache
        .transition(&document, &document, &schema, &[usize::MAX], &limits)
        .unwrap();

    assert_eq!(transition.rerendered_new_blocks, 0);
    assert_eq!(transition.update, CachedRenderTransitionUpdate::None);
}

#[test]
fn cached_transition_falls_back_when_schema_fingerprint_changes() {
    let old_schema = tiptap_schema();
    let new_schema = prosemirror_schema();
    let limits = ResourceLimits::default();
    let document = doc(vec![paragraph(vec![text("same document")])]);
    let cache = CachedRenderBlocks::build(&document, &old_schema, &limits).unwrap();

    let transition = cache
        .transition(&document, &document, &new_schema, &[], &limits)
        .unwrap();

    assert_eq!(
        transition.update,
        CachedRenderTransitionUpdate::Full(render_blocks(&document, &new_schema))
    );
}

#[test]
fn cached_transition_uses_full_fallback_for_invalid_hint() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let old_doc = doc(vec![paragraph(vec![text("old")])]);
    let new_doc = doc(vec![paragraph(vec![text("new")])]);
    let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
    let transition = cache
        .transition(&old_doc, &new_doc, &schema, &[1], &limits)
        .unwrap();

    assert_eq!(
        transition.update,
        CachedRenderTransitionUpdate::Full(render_blocks(&new_doc, &schema))
    );
}

#[test]
fn cached_transition_uses_full_fallback_when_changed_document_renders_identically() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let ignored = |flag: bool| {
        Node::element(
            "unrecognisedContainer".to_string(),
            HashMap::from([("flag".to_string(), serde_json::Value::Bool(flag))]),
            Fragment::from(vec![]),
        )
    };
    let old_doc = doc(vec![ignored(false)]);
    let new_doc = doc(vec![ignored(true)]);
    let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
    let transition = cache
        .transition(&old_doc, &new_doc, &schema, &[0], &limits)
        .unwrap();

    assert_eq!(
        transition.update,
        CachedRenderTransitionUpdate::Full(render_blocks(&new_doc, &schema))
    );
}

#[test]
fn cached_render_build_obeys_document_node_limit() {
    let schema = tiptap_schema();
    let limits = ResourceLimits {
        max_document_nodes: 2,
        ..ResourceLimits::default()
    };
    let document = doc(vec![paragraph(vec![text("too many nodes")])]);

    assert!(matches!(
        CachedRenderBlocks::build(&document, &schema, &limits),
        Err(super::CachedRenderError::ResourceLimitExceeded)
    ));
}

#[test]
fn cached_render_build_uses_canonical_root_depth_one() {
    let schema = tiptap_schema();
    let exact_limits = ResourceLimits {
        max_document_depth: 3,
        ..ResourceLimits::default()
    };
    let over_limits = ResourceLimits {
        max_document_depth: 2,
        ..ResourceLimits::default()
    };
    let document = doc(vec![paragraph(vec![text("depth three")])]);

    assert!(CachedRenderBlocks::build(&document, &schema, &exact_limits).is_ok());
    assert!(matches!(
        CachedRenderBlocks::build(&document, &schema, &over_limits),
        Err(super::CachedRenderError::ResourceLimitExceeded)
    ));
}

#[test]
fn cached_render_build_rejects_root_width_over_remaining_node_budget() {
    let schema = tiptap_schema();
    let limits = ResourceLimits {
        max_document_nodes: 3,
        ..ResourceLimits::default()
    };
    let document = doc(vec![
        paragraph(vec![]),
        paragraph(vec![]),
        paragraph(vec![]),
    ]);

    assert!(matches!(
        CachedRenderBlocks::build(&document, &schema, &limits),
        Err(super::CachedRenderError::ResourceLimitExceeded)
    ));
}

#[test]
fn cached_render_build_rejects_ordered_list_number_overflow() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let document = doc(vec![ordered_list(
        u32::MAX,
        vec![
            list_item(vec![paragraph(vec![text("one")])]),
            list_item(vec![paragraph(vec![text("two")])]),
        ],
    )]);

    assert!(matches!(
        CachedRenderBlocks::build(&document, &schema, &limits),
        Err(super::CachedRenderError::PositionOverflow)
    ));
}

#[test]
fn incremental_ordered_list_indices_are_exact_or_structured_overflow() {
    let schema = tiptap_schema();
    let exact = doc(vec![ordered_list(
        u32::MAX,
        vec![list_item(vec![paragraph(vec![text("last")])])],
    )]);

    let exact_blocks = try_render_blocks(&exact, &schema).expect("u32::MAX must render");
    let RenderElement::BlockStart {
        list_context: Some(context),
        ..
    } = &exact_blocks[0][0]
    else {
        panic!("ordered-list item must carry a list context");
    };
    assert_eq!(context.index, u32::MAX);

    let overflow = doc(vec![ordered_list(
        u32::MAX,
        vec![
            list_item(vec![paragraph(vec![text("last")])]),
            list_item(vec![paragraph(vec![text("overflow")])]),
        ],
    )]);
    assert!(matches!(
        try_render_blocks(&overflow, &schema),
        Err(super::CachedRenderError::PositionOverflow)
    ));
}

#[test]
fn ordered_list_start_defaults_only_when_absent_and_rejects_present_malformed_values() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let missing = doc(vec![ordered_list_with_start(
        None,
        vec![list_item(vec![paragraph(vec![text("first")])])],
    )]);

    let blocks = try_render_blocks(&missing, &schema).expect("missing start defaults to one");
    let RenderElement::BlockStart {
        list_context: Some(context),
        ..
    } = &blocks[0][0]
    else {
        panic!("ordered-list item must carry a list context");
    };
    assert_eq!(context.index, 1);

    for start in [
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::Value::Null,
        serde_json::json!("1"),
        serde_json::json!(u64::from(u32::MAX) + 1),
    ] {
        let malformed = doc(vec![ordered_list_with_start(
            Some(start),
            vec![list_item(vec![paragraph(vec![text("bad")])])],
        )]);
        assert!(matches!(
            CachedRenderBlocks::build(&malformed, &schema, &limits),
            Err(super::CachedRenderError::PositionOverflow)
        ));
        assert!(matches!(
            try_render_blocks(&malformed, &schema),
            Err(super::CachedRenderError::PositionOverflow)
        ));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn cached_transition_always_reconstructs_and_matches_full_render(
        values in prop::collection::vec("[a-z]{0,12}", 1..8),
        replacement in "[a-z]{0,12}",
        raw_index in any::<usize>(),
    ) {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let index = raw_index % values.len();
        let old_doc = doc(values.iter().map(|value| paragraph(vec![text(value)])).collect());
        let mut new_values = values;
        new_values[index] = replacement;
        let new_doc = doc(
            new_values
                .iter()
                .map(|value| paragraph(vec![text(value)]))
                .collect(),
        );
        let old_render = render_blocks(&old_doc, &schema);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[index], &limits)
            .unwrap();
        let expected = render_blocks(&new_doc, &schema);

        assert_update_reconstructs(old_render, &transition, &expected);
    }
}
