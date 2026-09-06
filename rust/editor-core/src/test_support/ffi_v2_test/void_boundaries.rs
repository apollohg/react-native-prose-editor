fn input_envelope(request_id: u64, base_revision: u64, text: &str) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": base_revision.to_string(),
        "text": text,
    })
    .to_string()
}

fn command_envelope(request_id: u64, base_revision: u64, command: Value) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": base_revision.to_string(),
        "command": command,
    })
    .to_string()
}

fn terminal_custom_atom_config() -> Value {
    json!({
        "schema": {
            "nodes": [
                { "name": "doc", "content": "block+", "role": "doc" },
                { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" },
                {
                    "name": "counterCard",
                    "content": "",
                    "group": "block",
                    "role": "block",
                    "isVoid": true,
                    "attrs": { "count": { "default": 0 } },
                },
            ],
            "marks": [],
        },
        "initialization": {
            "type": "localJson",
            "json": {
                "type": "doc",
                "content": [{ "type": "counterCard", "attrs": { "count": 7 } }],
            },
        },
    })
}

#[test]
fn ranged_backspace_ending_after_custom_atom_deletes_the_full_selection() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph", "content": [{ "type": "text", "text": "prefix" }] },
        { "type": "counterCard", "attrs": { "count": 7 } },
        { "type": "paragraph", "content": [{ "type": "text", "text": "after" }] },
    ]);
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 3, 9, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));
    assert_eq!(outcome["type"], "transaction");
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "preafter" }] },
            ],
        })
    );
    destroy_handle(&id);
}

#[test]
fn ranged_backspace_ending_after_image_deletes_the_full_selection() {
    let id = create_handle(local_json_config(
        r#"{
            "type":"doc",
            "content":[
                {"type":"paragraph","content":[{"type":"text","text":"prefix"}]},
                {"type":"image","attrs":{"src":"https://example.com/a.png"}},
                {"type":"paragraph","content":[{"type":"text","text":"after"}]}
            ]
        }"#,
    ));

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 3, 9, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "preafter" }] },
            ],
        })
    );
    destroy_handle(&id);
}

#[test]
fn custom_atom_render_id_is_stable_when_text_before_it_changes() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph", "content": [{ "type": "text", "text": "a" }] },
        { "type": "counterCard", "attrs": { "count": 7 } },
        { "type": "paragraph", "content": [{ "type": "text", "text": "b" }] },
    ]);
    let id = create_handle(config);
    let atom_id = |update: &Value| {
        update["renderBlocks"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|block| block.as_array().into_iter().flatten())
            .find(|element| element["nodeType"] == "counterCard")
            .and_then(|element| element["atomId"].as_str())
            .map(str::to_owned)
    };
    let before = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    let before_id = atom_id(&before).expect("custom atom render must carry an identity");

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(1, revision_of(&id), 0, 0),
    ));
    ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(2, revision_of(&id), "x"),
    ));
    let after = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));

    assert_eq!(atom_id(&after).as_deref(), Some(before_id.as_str()));
    destroy_handle(&id);
}

#[test]
fn backspace_at_text_start_after_image_is_not_applicable() {
    let id = create_handle(local_json_config(
        r#"{
            "type":"doc",
            "content":[
                {"type":"image","attrs":{"src":"https://example.com/a.png"}},
                {"type":"paragraph","content":[{"type":"text","text":"caption"}]}
            ]
        }"#,
    ));
    let before = document_json_of(&id);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(document_json_of(&id), before);
    destroy_handle(&id);
}

#[test]
fn schema_policy_can_preserve_a_custom_void_block_on_backspace() {
    let mut config = terminal_custom_atom_config();
    config["schema"]["nodes"][3]["deletableOnBackspace"] = json!(false);
    config["initialization"]["json"]["content"] = json!([
        { "type": "counterCard", "attrs": { "count": 7 } },
        { "type": "paragraph", "content": [{ "type": "text", "text": "caption" }] },
    ]);
    let expected = config["initialization"]["json"].clone();
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 2, 2, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(document_json_of(&id), expected);
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_boundary_does_not_accept_text() {
    let id = create_handle(terminal_custom_atom_config());

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(2, revision_of(&id), "x"),
    ));

    assert_eq!(caret_scalar(&id), 0);
    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_boundary_does_not_accept_return() {
    let id = create_handle(terminal_custom_atom_config());

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "splitBlock" })),
    ));

    assert_eq!(caret_scalar(&id), 0);
    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn nested_terminal_void_boundary_does_not_accept_text() {
    let id = create_handle(local_json_config(
        r#"{
            "type":"doc",
            "content":[
                {"type":"blockquote","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"caption"}]},
                    {"type":"image","attrs":{"src":"https://example.com/a.png"}}
                ]}
            ]
        }"#,
    ));

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 9, 9, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(2, revision_of(&id), "x"),
    ));

    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [{
                "type": "blockquote",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "caption" }] },
                    { "type": "image", "attrs": { "src": "https://example.com/a.png" } }
                ]
            }]
        })
    );
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_boundary_backspace_is_not_applicable() {
    let id = create_handle(terminal_custom_atom_config());

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(caret_scalar(&id), 0);
    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn backspace_in_empty_paragraph_after_custom_atom_keeps_atom() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "counterCard", "attrs": { "count": 7 } },
        { "type": "paragraph" },
    ]);
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 3, 3, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    assert_eq!(caret_scalar(&id), 0);
    destroy_handle(&id);
}

#[test]
fn deleting_blank_paragraph_before_atom_moves_caret_to_previous_text() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
        { "type": "paragraph" },
        { "type": "counterCard", "attrs": { "count": 7 } },
    ]);
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 2, 2, "before"),
    ));
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
                { "type": "counterCard", "attrs": { "count": 7 } },
            ],
        })
    );
    assert_eq!(caret_scalar(&id), 1);

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(3, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph" },
                { "type": "counterCard", "attrs": { "count": 7 } },
            ],
        })
    );
    destroy_handle(&id);
}

#[test]
fn deleting_first_blank_paragraph_before_atom_keeps_caret_at_document_start() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph" },
        { "type": "counterCard", "attrs": { "count": 7 } },
    ]);
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    let atom_only = json!({
        "type": "doc",
        "content": [{ "type": "counterCard", "attrs": { "count": 7 } }],
    });
    assert_eq!(document_json_of(&id), atom_only);
    assert_eq!(caret_scalar(&id), 0);

    assert_eq!(
        ok_json(&v2::editor_v2_apply_command(
            id.clone(),
            command_envelope(3, revision_of(&id), json!({ "type": "deleteBackward" })),
        )),
        json!({ "type": "notApplicable" })
    );
    assert_eq!(document_json_of(&id), atom_only);
    destroy_handle(&id);
}

#[test]
fn forward_delete_before_first_blank_paragraph_does_not_remove_it() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph" },
        { "type": "counterCard", "attrs": { "count": 7 } },
    ]);
    let id = create_handle(config);
    let render = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "4".into(),
        None,
        None,
    ));
    let epoch = render["positionEpoch"].as_str().unwrap();

    let outcome = ok_json(&v2::editor_v2_apply_native_intent(
        id.clone(),
        json!({
            "version": 1,
            "requestId": "1",
            "ownerId": "4",
            "positionEpoch": epoch,
            "intent": {
                "type": "deleteForward",
                "anchor": 0,
                "head": 0,
            },
        })
        .to_string(),
    ));
    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], false);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph" },
                { "type": "counterCard", "attrs": { "count": 7 } },
            ],
        })
    );
    destroy_handle(&id);
}

#[test]
fn explicit_blank_paragraph_range_is_not_treated_as_boundary_backspace() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph" },
        { "type": "counterCard", "attrs": { "count": 7 } },
    ]);
    let id = create_handle(config);

    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            1,
            revision_of(&id),
            json!({
                "type": "deleteRange",
                "range": {
                    "from": { "offset": 0, "kind": "scalar" },
                    "to": { "offset": 1, "kind": "scalar" },
                },
            }),
        ),
    ));
    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], false);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph" },
                { "type": "counterCard", "attrs": { "count": 7 } },
            ],
        })
    );
    destroy_handle(&id);
}

#[test]
fn explicit_non_first_blank_paragraph_range_preserves_mapped_selection() {
    let mut config = terminal_custom_atom_config();
    config["initialization"]["json"]["content"] = json!([
        { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
        { "type": "paragraph" },
        { "type": "counterCard", "attrs": { "count": 7 } },
    ]);
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 0, 0, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            2,
            revision_of(&id),
            json!({
                "type": "deleteRange",
                "range": {
                    "from": { "offset": 1, "kind": "scalar" },
                    "to": { "offset": 2, "kind": "scalar" },
                },
            }),
        ),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(update["selection"]["anchorScalar"], 1);
    assert_eq!(update["selection"]["headScalar"], 2);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
                { "type": "counterCard", "attrs": { "count": 7 } },
            ],
        })
    );
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_boundary_backspace_preserves_optional_root_atom() {
    let mut config = terminal_custom_atom_config();
    config["schema"]["nodes"][0]["content"] = json!("block*");
    let id = create_handle(config);

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(outcome, json!({ "type": "notApplicable" }));
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn move_selection_command_reorders_text_in_one_transaction() {
    let id = create_handle(local_json_config(
        r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcd"}]}]}"#,
    ));

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(1, revision_of(&id), 0, 2),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            2,
            revision_of(&id),
            json!({
                "type": "moveSelection",
                "range": {
                    "from": { "offset": 0, "kind": "scalar" },
                    "to": { "offset": 2, "kind": "scalar" },
                },
                "at": { "offset": 4, "kind": "scalar" },
            }),
        ),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "cdab" }],
            }],
        })
    );
    assert_eq!(
        ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(3))),
        json!({ "changed": true })
    );
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcd" }],
            }],
        })
    );
    destroy_handle(&id);
}

#[test]
fn move_selection_command_preserves_custom_atom_attributes() {
    let id = create_handle(json!({
        "schema": {
            "nodes": [
                { "name": "doc", "content": "block+", "role": "doc" },
                { "name": "paragraph", "content": "text*", "group": "block", "role": "textBlock" },
                { "name": "text", "content": "", "role": "text" },
                {
                    "name": "counterCard",
                    "content": "",
                    "group": "block",
                    "role": "block",
                    "isVoid": true,
                    "attrs": {
                        "title": { "default": "" },
                        "count": { "default": 0 },
                    },
                },
            ],
            "marks": [],
        },
        "initialization": {
            "type": "localJson",
            "json": {
                "type": "doc",
                "content": [
                    { "type": "counterCard", "attrs": { "title": "Keep", "count": 7 } },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
                ],
            },
        },
    }));

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(1, revision_of(&id), 0, 1),
    ));
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            2,
            revision_of(&id),
            json!({
                "type": "moveSelection",
                "range": {
                    "from": { "offset": 0, "kind": "scalar" },
                    "to": { "offset": 1, "kind": "scalar" },
                },
                "at": { "offset": 3, "kind": "scalar" },
            }),
        ),
    ));

    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
                { "type": "counterCard", "attrs": { "title": "Keep", "count": 7 } },
            ],
        })
    );

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            3,
            revision_of(&id),
            json!({
                "type": "moveSelection",
                "range": {
                    "from": { "offset": 2, "kind": "scalar" },
                    "to": { "offset": 3, "kind": "scalar" },
                },
                "at": { "offset": 0, "kind": "scalar" },
            }),
        ),
    ));
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "counterCard", "attrs": { "title": "Keep", "count": 7 } },
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
            ],
        })
    );
    destroy_handle(&id);
}
