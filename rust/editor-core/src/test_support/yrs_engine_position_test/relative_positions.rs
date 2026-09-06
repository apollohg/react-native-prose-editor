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

#[test]
fn revisioned_offsets_preserve_affinity_at_rendered_inter_block_break() {
    let (yrs_doc, schema) = rich_position_fixture();
    let document = rich_position_document();
    let map = PositionMap::build(&document, &schema);
    let rendered_text = "A😀e\u{301}\nmentionZ\n\u{fffc}\n• nested";
    let txn = yrs_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();

    assert_eq!(map.block(0).unwrap().scalar_len, 13);
    assert_eq!(map.block(0).unwrap().rendered_break_after, 1);
    assert!(map.block(1).unwrap().is_void_block);
    assert_eq!(map.block(1).unwrap().doc_start, 9);
    assert_eq!(rendered_text.chars().count() as u32, map.total_scalars());
    assert_eq!(map.scalar_to_doc(13, &document), 8);
    assert_eq!(scalar_offset_to_utf16(rendered_text, 13), Some(14));

    for (kind, offset) in [
        (EditorOffsetKind::Scalar, 13),
        (EditorOffsetKind::Utf16, 14),
    ] {
        let point = |affinity| {
            revisioned_position_to_relative_point(
                &txn,
                &fragment,
                RevisionedPosition {
                    offset,
                    kind,
                    affinity,
                },
                rendered_text,
                &map,
                &document,
                &schema,
            )
            .unwrap()
        };
        let before = point(Affinity::Before);
        assert!(revisioned_position_to_relative_point(
            &txn,
            &fragment,
            RevisionedPosition {
                offset,
                kind,
                affinity: Affinity::After,
            },
            rendered_text,
            &map,
            &document,
            &schema,
        )
        .is_none());
        assert_eq!(
            relative_point_to_doc_pos(&txn, &fragment, &before, &schema),
            Some(8)
        );
        let before_sticky = serde_json::to_value(&before.sticky).unwrap();
        assert_eq!(
            before_sticky,
            serde_json::json!({
                "assoc": -1,
                "item": { "client": 4242, "clock": 10 }
            })
        );
    }
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
