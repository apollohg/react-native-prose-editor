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
