use std::collections::HashMap;

use editor_core::model::{Document, Fragment, Node};
use editor_core::position::PositionMap;
use editor_core::schema::Schema;
use editor_core::selection::Selection;
use editor_core::tiptap_schema;
use editor_core::yrs_engine::{
    doc_pos_to_relative_point, relative_point_to_doc_pos, relative_selection_to_selection,
    revisioned_position_to_doc_pos, scalar_offset_to_utf16, utf16_offset_to_scalar, Affinity,
    EditorOffsetKind, RelativeSelection, RevisionedPosition,
};
use proptest::prelude::*;
use yrs::types::text::Text;
use yrs::types::xml::{
    XmlElementPrelim, XmlElementRef, XmlFragment, XmlFragmentRef, XmlTextPrelim, XmlTextRef,
};
use yrs::{Doc, OffsetKind, Options, ReadTxn, Transact, WriteTxn};

fn utf16_doc() -> Doc {
    Doc::with_options(Options {
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
    let doc = utf16_doc();
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
    let doc = utf16_doc();
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
fn revisioned_scalar_and_utf16_offsets_share_the_current_position_map() {
    let schema = tiptap_schema();
    let document = paragraph_document("A😀e\u{301}Z");
    let map = PositionMap::build(&document, &schema);
    let position = |offset, kind| RevisionedPosition {
        offset,
        kind,
        affinity: Affinity::After,
    };

    assert_eq!(
        revisioned_position_to_doc_pos(
            position(2, EditorOffsetKind::Scalar),
            "A😀e\u{301}Z",
            &map,
            &document,
        ),
        Some(3)
    );
    assert_eq!(
        revisioned_position_to_doc_pos(
            position(3, EditorOffsetKind::Utf16),
            "A😀e\u{301}Z",
            &map,
            &document,
        ),
        Some(3)
    );
    assert_eq!(
        revisioned_position_to_doc_pos(
            position(2, EditorOffsetKind::Utf16),
            "A😀e\u{301}Z",
            &map,
            &document,
        ),
        None
    );
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
        prop_assert_eq!(target_before.affinity, Affinity::Before);
        prop_assert_eq!(target_after.affinity, Affinity::After);

        let current = paragraph_document("αω");
        let map = PositionMap::build(&current, &schema);
        let relative_selection = RelativeSelection::Text {
            anchor: target_before,
            head: target_after,
        };
        let selection = relative_selection_to_selection(
            &txn,
            &fragment,
            &relative_selection,
            &schema,
            &current,
            &map,
        ).unwrap();
        prop_assert_eq!(selection, Selection::cursor(2));
    }
}
