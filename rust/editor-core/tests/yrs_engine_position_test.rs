use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Node};
use editor_core::position::PositionMap;
use editor_core::schema::Schema;
use editor_core::selection::Selection;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    doc_pos_to_relative_point, relative_point_to_doc_pos, relative_selection_to_selection,
    revisioned_position_to_relative_point, scalar_offset_to_utf16, utf16_offset_to_scalar,
    Affinity, EditorOffsetKind, RelativeSelection, RevisionedPosition,
};
use proptest::prelude::*;
use serde::Deserialize;
use yrs::types::text::Text;
use yrs::types::xml::{
    XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlTextPrelim, XmlTextRef,
};
use yrs::{ClientID, Doc, OffsetKind, Options, ReadTxn, Transact, WriteTxn};

fn utf16_doc() -> Doc {
    utf16_doc_with_client_id(4444)
}

fn utf16_doc_with_client_id(client_id: u64) -> Doc {
    Doc::with_options(Options {
        client_id: ClientID::new(client_id),
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    })
}

fn push_element<P: XmlFragment>(
    parent: &P,
    txn: &mut yrs::TransactionMut<'_>,
    tag: &str,
) -> XmlElementRef {
    parent.push_back(txn, XmlElementPrelim::empty(tag))
}

fn push_text<P: XmlFragment>(
    parent: &P,
    txn: &mut yrs::TransactionMut<'_>,
    value: &str,
) -> XmlTextRef {
    parent.push_back(txn, XmlTextPrelim::new(value))
}

fn rich_position_fixture() -> (Doc, Schema) {
    let doc = utf16_doc_with_client_id(4242);
    let mut txn = doc.transact_mut();
    let fragment = txn.get_or_insert_xml_fragment("prosemirror");

    let paragraph = push_element(&fragment, &mut txn, "paragraph");
    push_text(&paragraph, &mut txn, "A😀e\u{301}");
    push_element(&paragraph, &mut txn, "hardBreak");
    push_element(&paragraph, &mut txn, "mention");
    push_text(&paragraph, &mut txn, "Z");

    push_element(&fragment, &mut txn, "horizontalRule");

    let list = push_element(&fragment, &mut txn, "bulletList");
    let item = push_element(&list, &mut txn, "listItem");
    let nested_paragraph = push_element(&item, &mut txn, "paragraph");
    push_text(&nested_paragraph, &mut txn, "nested");

    drop(txn);
    (doc, tiptap_schema())
}

fn custom_root_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "article", "content": "body+", "role": "doc" },
            {
                "name": "body",
                "content": "inline*",
                "group": "body",
                "role": "textBlock"
            },
            {
                "name": "calloutDivider",
                "content": "",
                "group": "body",
                "role": "block",
                "isVoid": true
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .expect("custom-root schema should be valid")
}

fn custom_root_fixture() -> (Doc, Schema) {
    let doc = utf16_doc_with_client_id(4343);
    let mut txn = doc.transact_mut();
    let fragment = txn.get_or_insert_xml_fragment("article-content");
    let first = push_element(&fragment, &mut txn, "body");
    push_text(&first, &mut txn, "root😀");
    push_element(&fragment, &mut txn, "calloutDivider");
    let second = push_element(&fragment, &mut txn, "body");
    push_text(&second, &mut txn, "tail");
    drop(txn);
    (doc, custom_root_schema())
}

#[derive(Debug, Deserialize)]
struct GoldenPair {
    before: Option<serde_json::Value>,
    after: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct GoldenFixture {
    fragment_name: String,
    content_size: u32,
    pairs: Vec<GoldenPair>,
}

#[derive(Debug, Deserialize)]
struct LegacyGoldens {
    rich: GoldenFixture,
    custom_root: GoldenFixture,
}

fn assert_fixture_matches_legacy_golden(doc: &Doc, schema: &Schema, golden: &GoldenFixture) {
    assert_eq!(golden.pairs.len(), golden.content_size as usize + 1);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment(golden.fragment_name.as_str()).unwrap();
    for (doc_pos, pair) in golden.pairs.iter().enumerate() {
        for (affinity, expected) in [
            (Affinity::Before, &pair.before),
            (Affinity::After, &pair.after),
        ] {
            let actual =
                doc_pos_to_relative_point(&txn, &fragment, doc_pos as u32, affinity, schema);
            let actual_serialized = actual
                .as_ref()
                .map(|point| serde_json::to_value(&point.sticky).unwrap());
            assert_eq!(
                actual_serialized.as_ref(),
                expected.as_ref(),
                "legacy sticky mismatch at {}:{doc_pos} {affinity:?}",
                golden.fragment_name
            );
            let actual_resolved = actual
                .as_ref()
                .and_then(|point| relative_point_to_doc_pos(&txn, &fragment, point, schema));
            let expected_resolved = expected.as_ref().map(|_| doc_pos as u32);
            assert_eq!(
                actual_resolved, expected_resolved,
                "legacy resolution mismatch at {}:{doc_pos} {affinity:?}",
                golden.fragment_name
            );
        }
    }
}

#[test]
fn shared_position_codec_matches_frozen_pre_extraction_sticky_goldens() {
    // Captured from the pre-extraction collaboration codec at commit 27b89e6.
    let goldens: LegacyGoldens =
        serde_json::from_str(include_str!("fixtures/yrs-position-legacy-golden.json"))
            .expect("legacy position golden fixture should be valid");
    let (rich_doc, rich_schema) = rich_position_fixture();
    assert_fixture_matches_legacy_golden(&rich_doc, &rich_schema, &goldens.rich);
    let (custom_doc, custom_schema) = custom_root_fixture();
    assert_fixture_matches_legacy_golden(&custom_doc, &custom_schema, &goldens.custom_root);
}

fn assert_all_positions_round_trip(
    doc: &Doc,
    fragment_name: &str,
    schema: &Schema,
    content_size: u32,
    after_unavailable: &[u32],
) {
    let txn = doc.transact();
    let fragment = txn
        .get_xml_fragment(fragment_name)
        .expect("fixture fragment should exist");
    let mut actual_after_unavailable = Vec::new();

    for doc_pos in 0..=content_size {
        for affinity in [Affinity::Before, Affinity::After] {
            let relative = doc_pos_to_relative_point(&txn, &fragment, doc_pos, affinity, schema);
            if affinity == Affinity::After && relative.is_none() {
                actual_after_unavailable.push(doc_pos);
                continue;
            }
            let relative = relative
                .unwrap_or_else(|| panic!("position {doc_pos} with {affinity:?} should map"));
            assert_eq!(relative.affinity, affinity);
            assert_eq!(
                relative_point_to_doc_pos(&txn, &fragment, &relative, schema),
                Some(doc_pos),
                "position {doc_pos} with {affinity:?} should round-trip"
            );
        }
    }
    assert_eq!(actual_after_unavailable, after_unavailable);
}

#[test]
fn shared_position_codec_round_trips_unicode_void_nodes_lists_and_paragraph_boundaries() {
    let (doc, schema) = rich_position_fixture();

    // paragraph(A + emoji + combining sequence + hard break + mention + Z) = 9,
    // horizontal rule = 1, nested bullet list = 12.
    assert_all_positions_round_trip(&doc, "prosemirror", &schema, 22, &[5, 7, 9, 20, 21, 22]);
}

#[test]
fn shared_position_codec_preserves_forward_and_backward_range_endpoints() {
    let (doc, schema) = rich_position_fixture();
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();

    for (anchor, head) in [(2, 18), (18, 2)] {
        for affinity in [Affinity::Before, Affinity::After] {
            let selection = RelativeSelection::Text {
                anchor: doc_pos_to_relative_point(&txn, &fragment, anchor, affinity, &schema)
                    .unwrap(),
                head: doc_pos_to_relative_point(&txn, &fragment, head, affinity, &schema).unwrap(),
            };
            let RelativeSelection::Text {
                anchor: relative_anchor,
                head: relative_head,
            } = selection
            else {
                unreachable!()
            };
            assert_eq!(
                relative_point_to_doc_pos(&txn, &fragment, &relative_anchor, &schema),
                Some(anchor)
            );
            assert_eq!(
                relative_point_to_doc_pos(&txn, &fragment, &relative_head, &schema),
                Some(head)
            );
        }
    }
}

#[test]
fn shared_position_codec_uses_schema_for_custom_roots_and_void_nodes() {
    let (doc, schema) = custom_root_fixture();

    // body("root😀") = 7, custom void = 1, body("tail") = 6.
    assert_all_positions_round_trip(&doc, "article-content", &schema, 14, &[6, 13, 14]);
}

#[test]
fn scalar_utf16_helpers_reject_surrogate_interiors_and_keep_combining_boundaries() {
    let value = "A😀e\u{301}Z";
    let expected = [(0, 0), (1, 1), (2, 3), (3, 4), (4, 5), (5, 6)];
    for (scalar, utf16) in expected {
        assert_eq!(scalar_offset_to_utf16(value, scalar), Some(utf16));
        assert_eq!(utf16_offset_to_scalar(value, utf16), Some(scalar));
    }
    assert_eq!(utf16_offset_to_scalar(value, 2), None);
    assert_eq!(scalar_offset_to_utf16(value, 6), None);
    assert_eq!(utf16_offset_to_scalar(value, 7), None);
}

#[test]
fn revisioned_scalar_and_utf16_offsets_create_affinity_aware_relative_points() {
    let (yrs_doc, schema) = rich_position_fixture();
    let document = paragraph_document("A😀e\u{301}Z");
    let map = PositionMap::build(&document, &schema);
    let txn = yrs_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let point = |offset, kind, affinity| {
        revisioned_position_to_relative_point(
            &txn,
            &fragment,
            RevisionedPosition {
                offset,
                kind,
                affinity,
            },
            "A😀e\u{301}Z",
            &map,
            &document,
            &schema,
        )
    };

    for (kind, offset) in [(EditorOffsetKind::Scalar, 2), (EditorOffsetKind::Utf16, 3)] {
        let before = point(offset, kind, Affinity::Before).unwrap();
        let after = point(offset, kind, Affinity::After).unwrap();
        assert_eq!(before.sticky.assoc, yrs::Assoc::Before);
        assert_eq!(after.sticky.assoc, yrs::Assoc::After);
        assert_ne!(
            serde_json::to_value(&before.sticky).unwrap(),
            serde_json::to_value(&after.sticky).unwrap()
        );
        assert_eq!(
            relative_point_to_doc_pos(&txn, &fragment, &before, &schema),
            Some(3)
        );
        assert_eq!(
            relative_point_to_doc_pos(&txn, &fragment, &after, &schema),
            Some(3)
        );
    }

    assert!(point(2, EditorOffsetKind::Utf16, Affinity::Before).is_none());
    assert!(point(99, EditorOffsetKind::Scalar, Affinity::After).is_none());
}

fn paragraph_document(text: &str) -> Document {
    Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![Node::element(
            "paragraph".into(),
            HashMap::new(),
            Fragment::from(vec![Node::text(text.into(), Vec::new())]),
        )]),
    ))
}

#[test]
fn minimized_deleted_boundary_case_observes_before_and_after_on_reinsertion() {
    let schema = tiptap_schema();
    let doc = utf16_doc_with_client_id(4646);
    let text = {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = push_element(&fragment, &mut txn, "paragraph");
        push_text(&paragraph, &mut txn, "αaaω")
    };
    let (before, after) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        (
            doc_pos_to_relative_point(&txn, &fragment, 2, Affinity::Before, &schema).unwrap(),
            doc_pos_to_relative_point(&txn, &fragment, 4, Affinity::After, &schema).unwrap(),
        )
    };
    {
        let mut txn = doc.transact_mut();
        text.remove_range(&mut txn, 1, 2);
    }
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(
            relative_point_to_doc_pos(&txn, &fragment, &before, &schema),
            Some(2)
        );
        assert_eq!(
            relative_point_to_doc_pos(&txn, &fragment, &after, &schema),
            Some(2)
        );
    }
    {
        let mut txn = doc.transact_mut();
        text.insert(&mut txn, 1, "Q");
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        relative_point_to_doc_pos(&txn, &fragment, &before, &schema),
        Some(2)
    );
    assert_eq!(
        relative_point_to_doc_pos(&txn, &fragment, &after, &schema),
        Some(3)
    );
    let current = paragraph_document("αQω");
    let map = PositionMap::build(&current, &schema);
    assert_eq!(
        relative_selection_to_selection(
            &txn,
            &fragment,
            &RelativeSelection::Text {
                anchor: before,
                head: after,
            },
            &schema,
            &current,
            &map,
        ),
        Some(Selection::text(2, 3))
    );
}

#[test]
fn relative_node_and_all_selections_normalize_directly() {
    let schema = tiptap_schema();
    let yrs_doc = utf16_doc_with_client_id(4747);
    {
        let mut txn = yrs_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let first = push_element(&fragment, &mut txn, "paragraph");
        push_text(&first, &mut txn, "a");
        push_element(&fragment, &mut txn, "horizontalRule");
        let second = push_element(&fragment, &mut txn, "paragraph");
        push_text(&second, &mut txn, "b");
    }
    let document = Document::new(Node::element(
        "doc".into(),
        HashMap::new(),
        Fragment::from(vec![
            Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::text("a".into(), Vec::new())]),
            ),
            Node::void("horizontalRule".into(), HashMap::new()),
            Node::element(
                "paragraph".into(),
                HashMap::new(),
                Fragment::from(vec![Node::text("b".into(), Vec::new())]),
            ),
        ]),
    ));
    let map = PositionMap::build(&document, &schema);
    let txn = yrs_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let node_point =
        doc_pos_to_relative_point(&txn, &fragment, 3, Affinity::Before, &schema).unwrap();

    assert_eq!(
        relative_selection_to_selection(
            &txn,
            &fragment,
            &RelativeSelection::Node { point: node_point },
            &schema,
            &document,
            &map,
        ),
        Some(Selection::node(3))
    );
    assert_eq!(
        relative_selection_to_selection(
            &txn,
            &fragment,
            &RelativeSelection::All,
            &schema,
            &document,
            &map,
        ),
        Some(Selection::all())
    );
}

proptest! {
    #[test]
    fn deleted_targets_resolve_deterministically_with_affinity_then_normalize(
        deleted_tokens in prop::collection::vec(
            prop_oneof![Just("a"), Just("😀"), Just("e\u{301}"), Just("中")],
            2..=8,
        )
    ) {
        let deleted = deleted_tokens.concat();
        let deleted_scalars = deleted.chars().count() as u32;
        let value = format!("α{deleted}ω");
        let doc = utf16_doc();
        let text = {
            let mut txn = doc.transact_mut();
            let fragment = txn.get_or_insert_xml_fragment("prosemirror");
            let paragraph = push_element(&fragment, &mut txn, "paragraph");
            push_text(&paragraph, &mut txn, &value)
        };
        let schema = tiptap_schema();
        let (left, target_before, target_after, right) = {
            let txn = doc.transact();
            let fragment = txn.get_xml_fragment("prosemirror").unwrap();
            let target = 2 + deleted_scalars / 2;
            (
                doc_pos_to_relative_point(&txn, &fragment, 2, Affinity::Before, &schema).unwrap(),
                doc_pos_to_relative_point(
                    &txn,
                    &fragment,
                    target,
                    Affinity::Before,
                    &schema,
                ).unwrap(),
                doc_pos_to_relative_point(
                    &txn,
                    &fragment,
                    target,
                    Affinity::After,
                    &schema,
                ).unwrap(),
                doc_pos_to_relative_point(
                    &txn,
                    &fragment,
                    2 + deleted_scalars,
                    Affinity::After,
                    &schema,
                ).unwrap(),
            )
        };

        {
            let mut txn = doc.transact_mut();
            text.remove_range(
                &mut txn,
                "α".encode_utf16().count() as u32,
                deleted.encode_utf16().count() as u32,
            );
        }

        {
            let txn = doc.transact();
            let fragment: XmlFragmentRef = txn.get_xml_fragment("prosemirror").unwrap();
            let resolve = |point| relative_point_to_doc_pos(&txn, &fragment, point, &schema).unwrap();
            let resolved = [
                resolve(&left),
                resolve(&target_before),
                resolve(&target_after),
                resolve(&right),
            ];
            prop_assert_eq!(resolved, [2, 2, 2, 2]);
        }
        prop_assert_eq!(target_before.affinity, Affinity::Before);
        prop_assert_eq!(target_after.affinity, Affinity::After);

        {
            let mut txn = doc.transact_mut();
            text.insert(&mut txn, "α".encode_utf16().count() as u32, "Q");
        }

        let txn = doc.transact();
        let fragment: XmlFragmentRef = txn.get_xml_fragment("prosemirror").unwrap();
        let before_after_insert = relative_point_to_doc_pos(
            &txn,
            &fragment,
            &left,
            &schema,
        ).unwrap();
        let after_after_insert = relative_point_to_doc_pos(
            &txn,
            &fragment,
            &right,
            &schema,
        ).unwrap();
        prop_assert_eq!([before_after_insert, after_after_insert], [2, 3]);
        let deleted_targets_after_insert = [
            relative_point_to_doc_pos(&txn, &fragment, &target_before, &schema).unwrap(),
            relative_point_to_doc_pos(&txn, &fragment, &target_after, &schema).unwrap(),
        ];
        prop_assert_eq!(deleted_targets_after_insert, [2, 2]);

        let current = paragraph_document("αQω");
        let map = PositionMap::build(&current, &schema);
        let relative_selection = RelativeSelection::Text {
            anchor: left,
            head: right,
        };
        let selection = relative_selection_to_selection(
            &txn,
            &fragment,
            &relative_selection,
            &schema,
            &current,
            &map,
        ).unwrap();
        prop_assert_eq!(selection, Selection::text(2, 3));
    }
}
