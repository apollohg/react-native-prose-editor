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

fn matches_round_trip(next: &Value) -> (bool, bool) {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), next)
            .unwrap();
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let (matches, lookup) = codec.matches_validated_json_with_lookup(&fragment, &txn, next);
    (matches.unwrap(), lookup.is_some())
}

fn match_raw(
    doc: &Doc,
    expected: &Value,
    limits: &ResourceLimits,
) -> (super::YrsEngineResult<bool>, bool) {
    let schema = tiptap_schema();
    let codec = YrsDocumentCodec::new(&schema, limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let (matched, lookup) = codec.matches_validated_json_with_lookup(&fragment, &txn, expected);
    (matched, lookup.is_some())
}

#[test]
fn validated_json_matcher_avoids_old_value_projection() {
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": { "nested": [true, null, { "emoji": "😀" }] },
            "content": [{
                "type": "text",
                "text": "A😀e\u{301}",
                "marks": [
                    { "type": "bold" },
                    { "type": "link", "attrs": { "href": "https://example.test/😀" } }
                ]
            }]
        }, {
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": { "type": "callout", "attrs": { "rank": 7 } }
            }
        }]
    });

    take_json_projection_materialization_count_for_test();
    assert_eq!(matches_round_trip(&input), (true, true));
    assert_eq!(take_json_projection_materialization_count_for_test(), 0);

    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror");
    assert!(fragment.is_none());
    drop(txn);
    let mut txn = doc.transact_mut();
    let fragment = txn.get_or_insert_xml_fragment("prosemirror");
    drop(txn);
    let txn = doc.transact();
    codec.read_json(&fragment, &txn).unwrap();
    assert_eq!(take_json_projection_materialization_count_for_test(), 1);
}

#[test]
fn validated_json_matcher_coalesces_text_across_diffs_nodes_and_fragments() {
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let heading = fragment.push_back(&mut txn, XmlElementPrelim::empty("heading"));
        heading.insert_attribute(&mut txn, "level", 2_i64);
        heading.push_back(&mut txn, XmlTextPrelim::new("A"));
        let nested = XmlFragmentPrelim::new::<_, XmlIn>([
            XmlIn::from(XmlTextPrelim::new("")),
            XmlIn::from(XmlTextPrelim::new("😀")),
        ]);
        heading.push_back(&mut txn, XmlIn::from(nested));
        let tail = heading.push_back(&mut txn, XmlTextPrelim::new(""));
        tail.insert_with_attributes(&mut txn, 0, "e\u{301}", Attrs::default());
    }
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "heading",
            "attrs": { "level": 2 },
            "content": [{ "type": "text", "text": "A😀e\u{301}" }]
        }]
    });
    assert_eq!(
        match_raw(&doc, &expected, &ResourceLimits::default()),
        (Ok(true), true)
    );

    for mismatched in [
        json!({ "type": "doc", "content": [{ "type": "heading", "attrs": { "level": 3 }, "content": [{ "type": "text", "text": "A😀e\u{301}" }] }] }),
        json!({ "type": "doc", "content": [{ "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "different" }] }] }),
        json!({ "type": "doc", "content": [{ "type": "heading", "attrs": { "level": 2 }, "content": [{ "type": "text", "text": "A😀e\u{301}", "marks": [] }] }] }),
    ] {
        assert_eq!(
            match_raw(&doc, &mismatched, &ResourceLimits::default()).0,
            Ok(false)
        );
    }

    let null_projected_marks = utf16_doc();
    {
        let mut txn = null_projected_marks.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        first.insert_with_attributes(
            &mut txn,
            0,
            "a",
            mark_attrs_value("custom", Any::Bool(true)),
        );
        let second = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        second.insert_with_attributes(
            &mut txn,
            0,
            "b",
            mark_attrs_value("custom", Any::Number(f64::NAN)),
        );
    }
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "ab",
                "marks": [{ "type": "custom" }]
            }]
        }]
    });
    assert_eq!(read_raw(&null_projected_marks), expected);
    assert_eq!(
        match_raw(&null_projected_marks, &expected, &ResourceLimits::default()),
        (Ok(true), true)
    );

    let cross_variant_marks = utf16_doc();
    {
        let mut txn = cross_variant_marks.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        first.insert_with_attributes(
            &mut txn,
            0,
            "a",
            mark_attrs_value("custom", Any::Buffer(vec![1, 2].into())),
        );
        let second = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        second.insert_with_attributes(
            &mut txn,
            0,
            "b",
            mark_attrs_value(
                "custom",
                Any::Array(vec![Any::BigInt(1), Any::BigInt(2)].into()),
            ),
        );
    }
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "ab",
                "marks": [{ "type": "custom", "attrs": [1, 2] }]
            }]
        }]
    });
    assert_eq!(read_raw(&cross_variant_marks), expected);
    assert_eq!(
        match_raw(&cross_variant_marks, &expected, &ResourceLimits::default()),
        (Ok(true), true)
    );

    let empty_attrs_then_absent = utf16_doc();
    {
        let mut txn = empty_attrs_then_absent.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let first = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        first.insert_with_attributes(&mut txn, 0, "a", Attrs::default());
        paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
    }
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "ab" }]
        }]
    });
    assert_eq!(read_raw(&empty_attrs_then_absent), expected);
    assert_eq!(
        match_raw(
            &empty_attrs_then_absent,
            &expected,
            &ResourceLimits::default()
        ),
        (Ok(true), true)
    );
}

#[test]
fn validated_json_matcher_preserves_later_error_precedence_after_mismatch() {
    let malformed = utf16_doc();
    {
        let mut txn = malformed.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("first mismatch"));
        let invalid = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        invalid.insert_embed_with_attributes(&mut txn, 0, Any::Bool(false), Attrs::default());
    }
    let wrong = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "wrong" }] }]
    });
    let error = match_raw(&malformed, &wrong, &ResourceLimits::default())
        .0
        .unwrap_err();
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_eq!(error.details.unwrap()["field"], "xmlTextRun");

    let fragmented = utf16_doc();
    {
        let mut txn = fragmented.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        for _ in 0..385 {
            paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
        }
    }
    let error = match_raw(
        &fragmented,
        &wrong,
        &ResourceLimits {
            max_document_nodes: 3,
            ..ResourceLimits::default()
        },
    )
    .0
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.details.unwrap()["dimension"], "rawTextRuns");

    let distinct_text_nodes = utf16_doc();
    {
        let mut txn = distinct_text_nodes.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
        let bold = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        bold.insert_with_attributes(&mut txn, 0, "b", mark_attrs("bold"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("c"));
    }
    let error = match_raw(
        &distinct_text_nodes,
        &wrong,
        &ResourceLimits {
            max_document_nodes: 4,
            ..ResourceLimits::default()
        },
    )
    .0
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(4));
    assert_eq!(error.actual, Some(5));

    let element_boundary = utf16_doc();
    {
        let mut txn = element_boundary.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        fragment.push_back(&mut txn, XmlTextPrelim::new("a"));
        fragment.push_back(&mut txn, XmlElementPrelim::empty("hardBreak"));
        fragment.push_back(&mut txn, XmlTextPrelim::new("b"));
    }
    let error = match_raw(
        &element_boundary,
        &json!({ "type": "wrong", "content": [] }),
        &ResourceLimits {
            max_document_nodes: 3,
            ..ResourceLimits::default()
        },
    )
    .0
    .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(3));
    assert_eq!(error.actual, Some(4));
}

#[test]
fn validated_json_matcher_treats_lookup_collection_as_opportunistic() {
    use crate::yrs_engine::mutation::{
        set_lookup_seed_hydration_failpoint_for_test, LookupSeedHydrationFailpoint,
    };

    let input = json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }]
    });
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &input)
            .unwrap();
    }
    set_lookup_seed_hydration_failpoint_for_test(Some(
        LookupSeedHydrationFailpoint::InitialReservation,
    ));
    let result = match_raw(&doc, &input, &limits);
    set_lookup_seed_hydration_failpoint_for_test(None);
    assert_eq!(result, (Ok(true), false));
}

#[test]
fn validated_json_matcher_projects_nonfinite_any_only_as_null() {
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.insert_attribute(&mut txn, "nan", f64::NAN);
        paragraph.insert_attribute(&mut txn, "positive", f64::INFINITY);
        paragraph.insert_attribute(&mut txn, "negative", f64::NEG_INFINITY);
    }
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "attrs": { "nan": null, "positive": null, "negative": null }
        }]
    });
    assert_eq!(read_raw(&doc), expected);
    assert_eq!(
        match_raw(&doc, &expected, &ResourceLimits::default()),
        (Ok(true), true)
    );

    for wrong in [json!(false), json!("null"), json!([]), json!({})] {
        let mut mismatched = expected.clone();
        mismatched["content"][0]["attrs"]["nan"] = wrong;
        assert_eq!(
            match_raw(&doc, &mismatched, &ResourceLimits::default()).0,
            Ok(false)
        );
    }
}

fn read_raw(doc: &Doc) -> Value {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    codec.read_json(&fragment, &txn).unwrap()
}

fn mark_attrs(mark: &str) -> Attrs {
    mark_attrs_value(mark, Any::Bool(true))
}

fn mark_attrs_value(mark: &str, value: Any) -> Attrs {
    let mut attrs = Attrs::default();
    attrs.insert(mark.into(), value);
    attrs
}

#[test]
fn read_json_coalesces_only_adjacent_equal_mark_text_storage() {
    let siblings = utf16_doc();
    {
        let mut txn = siblings.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
    }
    assert_eq!(
        read_raw(&siblings),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "ab" }]
            }]
        })
    );

    let diff_runs = utf16_doc();
    let text = {
        let mut txn = diff_runs.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        text.insert_with_attributes(&mut txn, 0, "ab", mark_attrs("bold"));
        text.insert_embed_with_attributes(&mut txn, 1, Any::Bool(false), Attrs::default());
        text
    };
    let txn = diff_runs.transact();
    assert_eq!(text.diff(&txn, YChange::identity).len(), 3);
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(json!({
            "phase": "candidateMaterialization",
            "field": "xmlTextRun"
        }))
    );
    drop(txn);

    let shared_embed = utf16_doc();
    {
        let mut txn = shared_embed.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let text = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        text.insert_embed_with_attributes(
            &mut txn,
            0,
            ArrayPrelim::from(vec![Any::String("shared".into())]),
            Attrs::default(),
        );
    }
    let txn = shared_embed.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "CODEC_INVARIANT_FAILED");
    assert_eq!(
        error.details,
        Some(json!({
            "phase": "candidateMaterialization",
            "field": "xmlTextRun"
        }))
    );

    let different_marks = utf16_doc();
    {
        let mut txn = different_marks.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        let bold = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        bold.insert_with_attributes(&mut txn, 0, "a", mark_attrs("bold"));
        let italic = paragraph.push_back(&mut txn, XmlTextPrelim::new(""));
        italic.insert_with_attributes(&mut txn, 0, "b", mark_attrs("italic"));
    }
    assert_eq!(
        read_raw(&different_marks)["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let element_boundary = utf16_doc();
    {
        let mut txn = element_boundary.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("a"));
        paragraph.push_back(&mut txn, XmlElementPrelim::empty("hardBreak"));
        paragraph.push_back(&mut txn, XmlTextPrelim::new("b"));
    }
    let element_json = read_raw(&element_boundary);
    assert_eq!(
        element_json["content"][0]["content"],
        json!([
            { "type": "text", "text": "a" },
            { "type": "hardBreak" },
            { "type": "text", "text": "b" }
        ])
    );

    let schema = tiptap_schema();
    let exact_limits = ResourceLimits {
        max_document_nodes: 3,
        ..ResourceLimits::default()
    };
    let txn = siblings.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert!(YrsDocumentCodec::new(&schema, &exact_limits)
        .read_json(&fragment, &txn)
        .is_ok());
    let rejected_limits = ResourceLimits {
        max_document_nodes: 2,
        ..ResourceLimits::default()
    };
    let error = YrsDocumentCodec::new(&schema, &rejected_limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));

    let raw_work_limits = ResourceLimits {
        max_document_nodes: 3,
        ..ResourceLimits::default()
    };
    let exact_fragmented = utf16_doc();
    {
        let mut txn = exact_fragmented.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        for _ in 0..384 {
            paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
        }
    }
    let txn = exact_fragmented.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let exact_fragmented_json = YrsDocumentCodec::new(&schema, &raw_work_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(
        exact_fragmented_json["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        exact_fragmented_json["content"][0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        384
    );

    let fragmented = utf16_doc();
    {
        let mut txn = fragmented.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
        for _ in 0..385 {
            paragraph.push_back(&mut txn, XmlTextPrelim::new("x"));
        }
    }
    let txn = fragmented.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = YrsDocumentCodec::new(&schema, &raw_work_limits)
        .read_json(&fragment, &txn)
        .unwrap_err();
    assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(384));
    assert_eq!(error.actual, Some(385));
    assert_eq!(
        error.details,
        Some(json!({
            "phase": "candidateMaterialization",
            "dimension": "rawTextRuns"
        }))
    );
}

#[test]
fn round_trips_heading_mark_attrs_emoji_and_combining_text() {
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "heading",
            "attrs": { "level": 2 },
            "content": [{
                "type": "text",
                "text": "A😀e\u{301}",
                "marks": [{
                    "type": "link",
                    "attrs": {
                        "href": "https://example.test",
                        "target": "_blank"
                    }
                }]
            }]
        }]
    });

    assert_eq!(round_trip(next.clone()), next);
}

#[test]
fn prepared_builder_matches_codec_for_heading_void_and_nested_any() {
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "h2",
            "attrs": {
                "data": { "nested": [true, null, "😀", { "value": 7 }] }
            },
            "content": [{ "type": "text", "text": "title" }]
        }, {
            "type": "hardBreak",
            "attrs": { "meta": ["void", 2] }
        }, {
            "type": "image",
            "attrs": {
                "src": "https://example.test/image.png",
                "metadata": { "widths": [320, 640] }
            }
        }]
    });
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let codec = YrsDocumentCodec::new(&schema, &limits);

    let imported = utf16_doc();
    {
        let mut txn = imported.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &input)
            .unwrap();
    }
    let expected = {
        let txn = imported.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        codec.read_json(&fragment, &txn).unwrap()
    };

    let prepared_doc = utf16_doc();
    {
        let mut txn = prepared_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let batch = prepare_xml_nodes(input["content"].as_array().unwrap(), &limits, 2).unwrap();
        for child in batch.nodes {
            insert_prepared_node(&fragment, &mut txn, child.index, child.node);
        }
    }
    let actual = {
        let txn = prepared_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        codec.read_json(&fragment, &txn).unwrap()
    };
    assert_eq!(actual, expected);

    let unsafe_number = json!({
        "type": "image",
        "attrs": { "unsafe": u64::MAX }
    });
    let error = prepare_xml_nodes(&[unsafe_number], &limits, 2).unwrap_err();
    assert_eq!(error.code, "DOCUMENT_INVALID");
}

#[test]
fn borrowed_mark_conversion_preserves_exact_sorted_attrs() {
    let marks = vec![
        json!({ "type": "italic" }),
        json!({
            "type": "link",
            "attrs": {
                "href": "https://example.test/😀",
                "title": "e\u{301} אב"
            }
        }),
        json!({ "type": "bold" }),
    ];

    let attrs = marks_to_attrs(Some(&marks));
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let mut budget = super::ConversionBudget::new(&limits);
    assert_eq!(
        attrs_to_marks(Some(&attrs), &schema, &mut budget).unwrap(),
        vec![
            json!({ "type": "bold" }),
            json!({ "type": "italic" }),
            json!({
                "type": "link",
                "attrs": {
                    "href": "https://example.test/😀",
                    "title": "e\u{301} אב"
                }
            }),
        ]
    );
    let empty = Attrs::default();
    assert!(actual_marks_equal(None, Some(&empty)));
    assert!(actual_marks_equal(Some(&empty), None));
}

#[test]
fn shared_codec_preserves_multimark_unicode_and_opaque_payload_exactly() {
    let input = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "A😀e\u{301} אב",
                "marks": [
                    { "type": "italic" },
                    { "type": "link", "attrs": { "href": "https://example.test/😀" } },
                    { "type": "bold" }
                ]
            }]
        }, {
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": {
                    "type": "callout",
                    "attrs": { "payload": ["😀", "e\u{301}", "אב", null] }
                }
            }
        }]
    });
    let expected = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "A😀e\u{301} אב",
                "marks": [
                    { "type": "bold" },
                    { "type": "italic" },
                    { "type": "link", "attrs": { "href": "https://example.test/😀" } }
                ]
            }]
        }, {
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": {
                    "type": "callout",
                    "attrs": { "payload": ["😀", "e\u{301}", "אב", null] }
                }
            }
        }]
    });

    assert_eq!(round_trip(input), expected);
}

#[test]
fn round_trips_list_attrs_and_inline_and_block_void_nodes() {
    let next = json!({
        "type": "doc",
        "content": [
            {
                "type": "orderedList",
                "attrs": { "start": 3 },
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [
                            { "type": "text", "text": "before" },
                            { "type": "hardBreak" },
                            { "type": "text", "text": "after" }
                        ]
                    }]
                }]
            },
            { "type": "horizontalRule" },
            {
                "type": "image",
                "attrs": {
                    "src": "https://example.test/image.png",
                    "alt": "example"
                }
            }
        ]
    });

    assert_eq!(round_trip(next.clone()), next);
}

#[test]
fn round_trips_opaque_json_nodes_without_changing_payloads() {
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "__opaque_json",
            "attrs": {
                "original_type": "callout",
                "opaque_placement": "block",
                "original_json": {
                    "type": "callout",
                    "attrs": {
                        "kind": "warning",
                        "metadata": [true, null, { "rank": 2 }]
                    },
                    "content": [
                        { "type": "text", "text": "preserve " },
                        { "type": "text", "text": "me" }
                    ]
                }
            }
        }]
    });

    assert_eq!(round_trip(next.clone()), next);
}

#[test]
fn read_and_write_share_exact_node_limit_accounting() {
    let schema = tiptap_schema();
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "three nodes" }]
        }]
    });
    let exact_limits = ResourceLimits {
        max_document_nodes: 3,
        ..Default::default()
    };
    let exact_codec = YrsDocumentCodec::new(&schema, &exact_limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        exact_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap();
    }
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(exact_codec.read_json(&fragment, &txn).unwrap(), next);
    }

    let mut rejected_limits = exact_limits.clone();
    rejected_limits.max_document_nodes = 2;
    let rejected_codec = YrsDocumentCodec::new(&schema, &rejected_limits);
    let write_doc = utf16_doc();
    let write_error = {
        let mut txn = write_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        rejected_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap_err()
    };
    let read_error = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        rejected_codec.read_json(&fragment, &txn).unwrap_err()
    };

    for error in [write_error, read_error] {
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
    }
}

#[test]
fn read_and_write_share_exact_depth_limit_accounting() {
    let schema = tiptap_schema();
    let next = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "depth three" }]
        }]
    });
    let exact_limits = ResourceLimits {
        max_document_depth: 3,
        ..Default::default()
    };
    let exact_codec = YrsDocumentCodec::new(&schema, &exact_limits);
    let doc = utf16_doc();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        exact_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap();
    }
    {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        assert_eq!(exact_codec.read_json(&fragment, &txn).unwrap(), next);
    }

    let mut rejected_limits = exact_limits.clone();
    rejected_limits.max_document_depth = 2;
    let rejected_codec = YrsDocumentCodec::new(&schema, &rejected_limits);
    let write_doc = utf16_doc();
    let write_error = {
        let mut txn = write_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        rejected_codec
            .apply_json(&fragment, &mut txn, &empty_json("doc"), &next)
            .unwrap_err()
    };
    let read_error = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        rejected_codec.read_json(&fragment, &txn).unwrap_err()
    };

    for error in [write_error, read_error] {
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(2));
        assert_eq!(error.actual, Some(3));
    }
}

#[test]
fn any_materialization_has_exact_depth_work_and_output_boundaries() {
    let nested = yrs::Any::Array(vec![yrs::Any::Array(vec![yrs::Any::Null].into())].into());
    let exact_depth = ResourceLimits {
        max_document_depth: 3,
        ..ResourceLimits::default()
    };
    assert_eq!(
        any_to_json_bounded(&nested, &exact_depth).unwrap(),
        serde_json::json!([[null]])
    );
    let depth_error = any_to_json_bounded(
        &nested,
        &ResourceLimits {
            max_document_depth: 2,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(depth_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(depth_error.limit, Some(2));
    assert_eq!(depth_error.actual, Some(3));

    let exact_output = ResourceLimits {
        max_input_bytes: 3,
        ..ResourceLimits::default()
    };
    assert_eq!(
        any_to_json_bounded(&yrs::Any::String("x".into()), &exact_output).unwrap(),
        serde_json::json!("x")
    );
    let output_error = any_to_json_bounded(
        &yrs::Any::String("x".into()),
        &ResourceLimits {
            max_input_bytes: 2,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(output_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(output_error.limit, Some(2));
    assert_eq!(output_error.actual, Some(3));

    let exact_items = yrs::Any::Array(vec![yrs::Any::Null; 127].into());
    assert!(any_to_json_bounded(
        &exact_items,
        &ResourceLimits {
            max_document_nodes: 1,
            ..ResourceLimits::default()
        }
    )
    .is_ok());
    let over_items = yrs::Any::Array(vec![yrs::Any::Null; 128].into());
    let work_error = any_to_json_bounded(
        &over_items,
        &ResourceLimits {
            max_document_nodes: 1,
            ..ResourceLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(work_error.code, "DOCUMENT_LIMIT_EXCEEDED");
    assert_eq!(work_error.limit, Some(128));
    assert_eq!(work_error.actual, Some(129));
}
