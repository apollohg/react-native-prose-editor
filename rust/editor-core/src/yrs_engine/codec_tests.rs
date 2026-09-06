use super::{
    actual_marks_equal, any_matches_json, any_to_json_bounded, attrs_to_marks,
    insert_prepared_node, marks_to_attrs, prepare_xml_nodes,
    take_json_projection_materialization_count_for_test, YrsDocumentCodec,
};
use crate::boundary::ResourceLimits;
use crate::schema::presets::tiptap_schema;
use crate::schema::Schema;
use serde_json::{json, Value};
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{
    Xml, XmlElementPrelim, XmlFragment, XmlFragmentPrelim, XmlIn, XmlTextPrelim,
};
use yrs::types::Attrs;
use yrs::{Any, ArrayPrelim};
use yrs::{Doc, OffsetKind, Options, ReadTxn, Transact, WriteTxn};

fn utf16_doc() -> Doc {
    let options = Options {
        offset_kind: OffsetKind::Utf16,
        ..Default::default()
    };
    Doc::with_options(options)
}

fn empty_json(document_root_type: &str) -> Value {
    json!({
        "type": document_root_type,
        "content": [],
    })
}

fn round_trip(next: Value) -> Value {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap();
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    codec.read_json(&fragment, &txn).unwrap()
}

#[test]
fn custom_json_projection_round_trips_through_yrs() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "info-box", "content": "inline*", "group": "block callout",
                "role": "textBlock", "htmlTag": "aside-info",
                "attrs": { "level": { "default": 0 } },
                "json": { "type": "callout", "attrs": { "tone": "info" } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "callout",
            "attrs": { "tone": "info", "level": 7 },
            "content": [{ "type": "text", "text": "Projected" }]
        }]
    });
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap();
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), next);
    assert!(codec
        .matches_validated_json_with_lookup(&fragment, &txn, &next)
        .0
        .unwrap());

    let malformed = utf16_doc();
    {
        let mut txn = malformed.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let callout = fragment.push_back(&mut txn, XmlElementPrelim::empty("callout"));
        callout.push_back(&mut txn, XmlTextPrelim::new("Projected"));
    }
    let txn = malformed.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert!(!codec
        .matches_validated_json_with_lookup(&fragment, &txn, &next)
        .0
        .unwrap());
}

#[test]
fn ordinary_numeric_attrs_require_the_exact_json_number_representation() {
    assert!(!any_matches_json(&Any::Number(2.0), Some(&json!(2))));
}

#[test]
fn legacy_heading_resolution_precedes_a_native_heading_node() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "heading", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "h2", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let heading = fragment.push_back(&mut txn, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(2));
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        YrsDocumentCodec::new(&schema, &ResourceLimits::default())
            .read_json(&fragment, &txn)
            .unwrap(),
        json!({ "type": "doc", "content": [{ "type": "h2" }] })
    );
}

#[test]
fn unresolved_legacy_heading_does_not_fall_back_to_a_native_heading_node() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "heading", "content": "inline*", "group": "block", "role": "textBlock" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let heading = fragment.push_back(&mut txn, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(2));
    }
    let expected = json!({ "type": "doc", "content": [{ "type": "h2" }] });
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();

    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert!(codec
        .matches_validated_json_with_lookup(&fragment, &txn, &expected)
        .0
        .unwrap());
}

#[test]
fn projected_native_wire_attributes_must_match_their_canonical_values() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let heading = fragment.push_back(&mut txn, XmlElementPrelim::empty("h2"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(3));
    }
    let expected = json!({
        "type": "doc",
        "content": [{ "type": "heading", "attrs": { "level": 2 } }]
    });
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();

    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert!(!codec
        .matches_validated_json_with_lookup(&fragment, &txn, &expected)
        .0
        .unwrap());
}

#[test]
fn legacy_heading_alias_drops_synthetic_level_before_unrelated_projection() {
    let schema = Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            {
                "name": "h2", "content": "inline*", "group": "block", "role": "textBlock",
                "json": { "type": "callout", "attrs": { "tone": "info" } }
            },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let heading = fragment.push_back(&mut txn, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", Any::BigInt(2));
    }
    let expected = json!({
        "type": "doc",
        "content": [{ "type": "callout", "attrs": { "tone": "info" } }]
    });
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();

    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    assert!(codec
        .matches_validated_json_with_lookup(&fragment, &txn, &expected)
        .0
        .unwrap());
}

include!("codec_tests/json_matching.rs");

include!("codec_tests/roundtrip_limits.rs");
