fn marked_text_document(marks: Value) -> Value {
    json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "x", "marks": marks }],
        }],
    })
}

fn set_json_envelope(request_id: u64, document: Value) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": "0",
        "setJson": document,
        "history": "resetAndClear",
    })
    .to_string()
}

fn set_html_envelope(request_id: u64, html: &str) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": "0",
        "setHtml": html,
        "history": "resetAndClear",
    })
    .to_string()
}

/// The marks a document's single text node carries, in stored order.
fn imported_mark_types(id: &str) -> Vec<String> {
    document_json_of(id)["content"][0]["content"][0]["marks"]
        .as_array()
        .expect("the imported text node carries marks")
        .iter()
        .map(|mark| {
            mark["type"]
                .as_str()
                .expect("every mark carries a string type")
                .to_string()
        })
        .collect()
}

#[test]
fn imported_json_marks_are_canonicalized_rather_than_refused() {
    // A serialized ProseMirror document preserves whatever order its producer
    // applied its marks in. Sorting is exactly what canonicalization does to
    // every step's output, so an import must not be refused for arriving in
    // another order.
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    ok_json(&v2::editor_v2_apply_local_api(
        id.clone(),
        set_json_envelope(
            801,
            marked_text_document(json!([{ "type": "italic" }, { "type": "bold" }])),
        ),
    ));

    assert_eq!(
        imported_mark_types(&id),
        vec!["bold".to_string(), "italic".to_string()],
        "the stored document is canonical whatever order the import arrived in"
    );
    destroy_handle(&id);
}

#[test]
fn imported_json_marks_out_of_order_around_a_link_are_canonicalized() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    ok_json(&v2::editor_v2_apply_local_api(
        id.clone(),
        set_json_envelope(
            802,
            marked_text_document(json!([
                { "type": "link", "attrs": { "href": "https://example.com" } },
                { "type": "bold" },
            ])),
        ),
    ));

    assert_eq!(
        imported_mark_types(&id),
        vec!["bold".to_string(), "link".to_string()],
    );
    destroy_handle(&id);
}

#[test]
fn imported_html_marks_are_canonicalized_for_either_nesting_order() {
    // `<em><strong>x</strong></em>` and `<strong><em>x</em></strong>` are the
    // same document; nesting order is not the author's contract with us.
    for (request_id, html) in [
        (803, "<p><strong><em>x</em></strong></p>"),
        (804, "<p><em><strong>x</strong></em></p>"),
    ] {
        let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

        ok_json(&v2::editor_v2_apply_local_api(
            id.clone(),
            set_html_envelope(request_id, html),
        ));

        assert_eq!(
            imported_mark_types(&id),
            vec!["bold".to_string(), "italic".to_string()],
            "{html} must import to the same canonical document"
        );
        destroy_handle(&id);
    }
}

#[test]
fn imported_marks_still_refuse_what_canonicalization_cannot_repair() {
    // Sorting fixes order. It cannot make a duplicate same-type mark
    // representable as a Yjs text attribute, nor invent a schema entry for an
    // unknown mark, so both stay refused.
    for (request_id, marks, reason) in [
        (
            805,
            json!([{ "type": "bold" }, { "type": "bold" }]),
            "duplicate same-type marks",
        ),
        (806, json!([{ "type": "notAMark" }]), "unknown mark"),
    ] {
        let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

        let error = err_json(&v2::editor_v2_apply_local_api(
            id.clone(),
            set_json_envelope(request_id, marked_text_document(marks)),
        ));

        assert_eq!(error.domain, "document", "{reason} must stay refused");
        destroy_handle(&id);
    }
}

#[test]
fn atom_doc_selection_maps_nested_unicode_and_deletes_with_undo() {
    let mut config = terminal_custom_atom_config();
    config["schema"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "name": "blockquote", "content": "block+", "group": "block", "role": "block"
        }));
    let content = json!([
        { "type": "blockquote", "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "😀é" }] },
            { "type": "counterCard", "attrs": { "count": 7 } },
            { "type": "paragraph", "content": [{ "type": "text", "text": "after" }] }
        ] }
    ]);
    config["initialization"]["json"]["content"] = content.clone();
    let id = create_handle(config);
    let select = |request_id: u64, revision: u64, doc_pos: u32, edge: &str| {
        json!({
        "version": 1, "requestId": request_id.to_string(), "baseDocumentRevision": revision.to_string(),
        "selection": { "type": "atom", "docPos": doc_pos, "edge": edge }
    }).to_string()
    };
    for (edge, scalar) in [("before", 2), ("after", 5)] {
        ok_json(&v2::editor_v2_set_selection(
            id.clone(),
            select(1, revision_of(&id), 5, edge),
        ));
        let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
        assert_eq!(
            update["selection"]["anchorScalar"], scalar,
            "{edge}: {update}"
        );
        assert_eq!(update["selection"]["headScalar"], scalar);
    }
    for doc_pos in [2, 4, 6, 999] {
        let outcome = ok_json(&v2::editor_v2_set_selection(
            id.clone(),
            select(2, revision_of(&id), doc_pos, "node"),
        ));
        assert_eq!(outcome["type"], "notApplicable");
    }
    let stale_revision = revision_of(&id);
    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        select(3, revision_of(&id), 5, "node"),
    ));
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(update["selection"]["type"], "node");
    assert_eq!(update["selection"]["posScalar"], 3);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(4, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));
    assert_eq!(
        document_json_of(&id)["content"][0]["content"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let stale = err_json(&v2::editor_v2_set_selection(
        id.clone(),
        select(5, stale_revision, 999, "node"),
    ));
    assert_eq!(stale.code, "REVISION_MISMATCH");
    ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(6)));
    assert_eq!(document_json_of(&id)["content"], content);
    destroy_handle(&id);
}

#[test]
fn atom_doc_selection_accepts_terminal_caret_boundaries() {
    let id = create_handle(terminal_custom_atom_config());
    for edge in ["before", "after", "node"] {
        ok_json(&v2::editor_v2_set_selection(id.clone(), json!({
            "version": 1, "requestId": "1", "baseDocumentRevision": revision_of(&id).to_string(),
            "selection": { "type": "atom", "docPos": 0, "edge": edge }
        }).to_string()));
    }
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));
    assert!(!document_json_of(&id).to_string().contains("counterCard"));
    ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(3)));
    assert!(document_json_of(&id).to_string().contains("counterCard"));
    destroy_handle(&id);
}
