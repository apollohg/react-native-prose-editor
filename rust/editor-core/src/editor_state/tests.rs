use super::*;
use crate::model::Fragment;

fn paragraph(text: &str) -> Node {
    Node::element(
        "paragraph".into(),
        HashMap::new(),
        Fragment::from(vec![Node::text(text.into(), Vec::new())]),
    )
}

fn element(node_type: &str, children: Vec<Node>) -> Node {
    Node::element(node_type.into(), HashMap::new(), Fragment::from(children))
}

fn document(children: Vec<Node>) -> Document {
    Document::new(element("doc", children))
}

fn assert_structural_preflights_match_oracle(
    document: &Document,
    schema: &Schema,
    selection: &Selection,
    limits: &ResourceLimits,
) {
    let nodes = document_node_count(document.root());
    let block_range = selected_block_range(
        document,
        schema,
        selection.from(document),
        selection.to(document),
    );
    let root_range = root_wrap_range(document, schema, selection);
    assert_eq!(
        can_toggle_blockquote_local(
            document,
            schema,
            selection,
            limits,
            nodes,
            block_range.as_ref(),
        ),
        can_toggle_blockquote_transaction_oracle(document, schema, selection, limits),
        "blockquote mismatch for {selection:?}"
    );
    for list_type in ["bulletList", "orderedList"] {
        assert_eq!(
            can_apply_list_type_local(
                document,
                schema,
                selection,
                list_type,
                limits,
                nodes,
                block_range.as_ref(),
                root_range.as_ref(),
            ),
            can_apply_list_type_transaction_oracle(document, schema, selection, list_type, limits,),
            "{list_type} mismatch for {selection:?}"
        );
    }
}

#[test]
fn structural_command_local_proofs_match_transaction_oracle() {
    let schema = crate::tiptap_schema();
    let documents = vec![
        document(vec![paragraph("one"), paragraph("two"), paragraph("three")]),
        document(vec![element("blockquote", vec![paragraph("quote")])]),
        document(vec![element(
            "bulletList",
            vec![
                element("listItem", vec![paragraph("one")]),
                element("listItem", vec![paragraph("two")]),
                element("listItem", vec![paragraph("three")]),
            ],
        )]),
        document(vec![element(
            "orderedList",
            vec![element(
                "listItem",
                vec![
                    paragraph("outer"),
                    element(
                        "bulletList",
                        vec![element("listItem", vec![paragraph("inner")])],
                    ),
                ],
            )],
        )]),
    ];
    for document in &documents {
        let size = document.content_size();
        let positions = [0, 1, size / 2, size.saturating_sub(1), size];
        for position in positions {
            for selection in [
                Selection::cursor(position),
                Selection::text(1.min(size), position),
                Selection::text(position, 1.min(size)),
                Selection::node(position),
                Selection::All,
            ] {
                assert_structural_preflights_match_oracle(
                    document,
                    &schema,
                    &selection,
                    &ResourceLimits::default(),
                );
            }
        }
        let nodes = document_node_count(document.root());
        let depth = node_relative_depth(document.root());
        for (nodes, depth) in [
            (nodes, depth),
            (nodes.saturating_add(1), depth),
            (nodes.saturating_add(8), depth.saturating_add(2)),
        ] {
            let limits = ResourceLimits {
                max_document_nodes: nodes,
                max_document_depth: depth,
                ..ResourceLimits::default()
            };
            assert_structural_preflights_match_oracle(
                document,
                &schema,
                &Selection::cursor(1.min(size)),
                &limits,
            );
        }
    }
}

#[test]
fn default_schema_list_commands_are_available() {
    let schema = crate::schema::presets::default_schema();
    let document = document(vec![paragraph("one")]);
    let limits = ResourceLimits::default();
    let state = active_state_for_debug_invariant(
        &document,
        &schema,
        &Selection::cursor(1),
        None,
        &limits,
        document_node_count(document.root()),
    );

    assert_eq!(state.commands.get("wrapBulletList"), Some(&true));
    assert_eq!(state.commands.get("wrapOrderedList"), Some(&true));
}

#[test]
fn structural_command_local_proofs_match_custom_schema_and_invalid_positions() {
    use crate::schema::AttrSpec;

    let base = crate::tiptap_schema();
    let mut nodes = base.all_nodes().cloned().collect::<Vec<_>>();
    for spec in &mut nodes {
        if spec.html_tag.as_deref() == Some("blockquote") || spec.name == "bulletList" {
            spec.attrs.insert(
                "requiredProofAttr".into(),
                AttrSpec {
                    default: None,
                    has_default: false,
                    ..AttrSpec::default()
                },
            );
        }
    }
    let schema = Schema::new(nodes, base.all_marks().cloned().collect());
    let document = document(vec![paragraph("one"), paragraph("two")]);
    for selection in [
        Selection::cursor(1),
        Selection::text(1, document.content_size().saturating_sub(1)),
        Selection::text(document.content_size().saturating_sub(1), 1),
        Selection::cursor(document.content_size().saturating_add(1)),
        Selection::cursor(u32::MAX),
    ] {
        assert_structural_preflights_match_oracle(
            &document,
            &schema,
            &selection,
            &ResourceLimits::default(),
        );
    }
}

#[test]
fn structural_command_local_proofs_match_generated_block_ranges() {
    let schema = crate::tiptap_schema();
    for block_count in 1..=8 {
        let document = document(
            (0..block_count)
                .map(|index| paragraph(&format!("block-{index}")))
                .collect(),
        );
        let size = document.content_size();
        let positions = (0..=size)
            .filter(|position| position % 3 == 0 || *position == 1 || *position == size)
            .collect::<Vec<_>>();
        for &anchor in &positions {
            for &head in &positions {
                assert_structural_preflights_match_oracle(
                    &document,
                    &schema,
                    &Selection::text(anchor, head),
                    &ResourceLimits::default(),
                );
            }
        }
    }
}

#[test]
fn known_node_count_entry_point_matches_standalone_wrapper() {
    let schema = crate::tiptap_schema();
    let document = document(vec![paragraph("one"), paragraph("two")]);
    let selection = Selection::cursor(1);
    assert_eq!(
        command_applicability_with_known_node_count(
            &document,
            &schema,
            &selection,
            &ResourceLimits::default(),
            document_node_count(document.root()),
        ),
        command_applicability(&document, &schema, &selection, &ResourceLimits::default(),)
    );
}

#[test]
fn node_selection_preserves_collapsed_stored_marks() {
    let schema = crate::tiptap_schema();
    let bold = Mark::new("bold".into(), HashMap::new());
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::text("x".into(), vec![bold.clone()])]),
        )]),
    ));
    let state = active_state(
        &document,
        &schema,
        &Selection::node(1),
        Some(std::slice::from_ref(&bold)),
        HashMap::new(),
        &ResourceLimits::default(),
    );
    assert_eq!(state.marks.get("bold"), Some(&true));
}

#[test]
fn task_list_wrap_availability_matches_schema_and_limits() {
    for list_name in ["taskList", "task_list"] {
        let schema = Schema::from_json(&serde_json::json!({"nodes": [
            {"name":"doc","content":"block+","role":"doc"},
            {"name":"paragraph","content":"text*","group":"block","role":"textBlock"},
            {"name":list_name,"content":"taskItem+","group":"block","role":"list"},
            {"name":"taskItem","content":"paragraph block*","role":"listItem","attrs":{"checked":{"default":false}}},
            {"name":"text","role":"text"}
        ],"marks":[]})).unwrap();
        let document = document(vec![paragraph("one")]);
        for (schema, limits, expected) in [
            (&schema, ResourceLimits::default(), true),
            (
                &schema,
                ResourceLimits {
                    max_document_nodes: 3,
                    ..ResourceLimits::default()
                },
                false,
            ),
            (&crate::tiptap_schema(), ResourceLimits::default(), false),
        ] {
            let state = active_state_for_debug_invariant(
                &document,
                schema,
                &Selection::cursor(1),
                None,
                &limits,
                document_node_count(document.root()),
            );
            assert_eq!(state.commands.get("wrapTaskList"), Some(&expected));
        }
    }
}
