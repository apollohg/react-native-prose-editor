// The accessor derives, from the live v2 session alone, everything the
// (since-deleted) stateless legacy render probe provided to the staging
// adapters: full render blocks, toolbar active state, the mirrored scalar
// selection resolved to doc positions, the lenient doc<->scalar position
// mapping (including the u32::MAX extent query), and the document's scalar
// extent. deleted the legacy runtime, so the probe-parity fixture matrix went
// with it; these tests pin the accessor's own wire shape, its v2-native
// history/revision facts, and its structured errors.

fn local_json_config(document: &str) -> Value {
    json!({
        "schema": tiptap_schema_json(),
        "initialization": {
            "type": "localJson",
            "json": serde_json::from_str::<Value>(document).unwrap(),
        }
    })
}

const FIXTURE_MULTI_BLOCK: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ab"}]},{"type":"paragraph","content":[{"type":"text","text":"cd"}]}]}"#;
const ORDERED_LIST_START_MISSING: &str = r#"{"type":"doc","content":[{"type":"orderedList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"first"}]}]}]}]}"#;
const ORDERED_LIST_START_MAX: &str = r#"{"type":"doc","content":[{"type":"orderedList","attrs":{"start":4294967295},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"last"}]}]}]}]}"#;
const ORDERED_LIST_START_ABOVE_U32: &str = r#"{"type":"doc","content":[{"type":"orderedList","attrs":{"start":4294967296},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"overflow"}]}]}]}]}"#;
const ORDERED_LIST_INDEX_ABOVE_U32: &str = r#"{"type":"doc","content":[{"type":"orderedList","attrs":{"start":4294967295},"content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"last"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"overflow"}]}]}]}]}"#;

#[test]
fn render_update_ordered_list_u32_boundary_is_exact_or_rejected() {
    let id = create_handle(local_json_config(ORDERED_LIST_START_MISSING));
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(
        update["renderBlocks"][0][0]["listContext"]["index"],
        json!(1),
        "an absent ordered-list start must default to one"
    );
    destroy_handle(&id);

    let id = create_handle(local_json_config(ORDERED_LIST_START_MAX));
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(
        update["renderBlocks"][0][0]["listContext"]["index"],
        json!(u32::MAX),
        "the v2 render accessor must preserve u32::MAX exactly"
    );
    destroy_handle(&id);

    let malformed_starts = [
        json!(-1),
        json!(1.5),
        Value::Null,
        json!("1"),
        json!(u64::from(u32::MAX) + 1),
    ];
    let malformed_documents = malformed_starts.into_iter().map(|start| {
        json!({
            "type": "doc",
            "content": [{
                "type": "orderedList",
                "attrs": { "start": start },
                "content": [{
                    "type": "listItem",
                    "content": [{
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": "bad" }],
                    }],
                }],
            }],
        })
        .to_string()
    });

    for document in [
        ORDERED_LIST_START_ABOVE_U32.to_string(),
        ORDERED_LIST_INDEX_ABOVE_U32.to_string(),
    ]
    .into_iter()
    .chain(malformed_documents)
    {
        let error = err_json(&v2::editor_v2_create(
            local_json_config(&document).to_string(),
            None,
        ));
        assert_error(&error, "boundary", "CODEC_INVARIANT_FAILED", None);
    }
}

#[test]
fn render_update_is_one_complete_atomic_snapshot() {
    let id = create_handle(local_json_config(FIXTURE_MULTI_BLOCK));
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    let keys: std::collections::BTreeSet<&str> = update
        .as_object()
        .expect("render update is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "renderBlocks",
            "renderPatch",
            "selection",
            "activeState",
            "historyState",
            "documentVersion",
            "stateRevision",
            "scalarLength",
            "documentIsEmpty",
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<&str>>(),
        "the no-mirror update carries exactly the frozen accessor keys: {keys:?}",
    );

    // History and version are the v2 engine's own facts, consistent with
    // getState at every revision.
    let assert_history_matches_state = |id: &str| {
        let state = state_of(id);
        let update = ok_json(&v2_render::editor_v2_render_update(
            id.to_string(),
            None,
            None,
        ));
        assert_eq!(update["documentVersion"], state["documentRevision"]);
        assert_eq!(update["stateRevision"], state["stateRevision"]);
        assert_eq!(
            update["historyState"],
            json!({
                "canUndo": state["canUndo"].as_bool().unwrap(),
                "canRedo": state["canRedo"].as_bool().unwrap(),
            })
        );
    };
    assert_history_matches_state(&id);
    ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(41, revision_of(&id), "Z"),
    ));
    assert_history_matches_state(&id);
    ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(42)));
    assert_history_matches_state(&id);
    destroy_handle(&id);
}

#[test]
fn render_update_cannot_mix_fields_with_a_concurrent_mutation() {
    use std::sync::mpsc::{sync_channel, RecvTimeoutError};
    use std::time::Duration;

    let id = create_handle(local_json_config(FIXTURE_MULTI_BLOCK));
    let base_revision = revision_of(&id);
    let state_before = state_of(&id);
    let (entered_tx, entered_rx) = sync_channel(0);
    let (resume_tx, resume_rx) = sync_channel(0);
    let _hook = v2_render::install_render_snapshot_test_hook(
        id.parse().expect("editor handle is a canonical u64"),
        entered_tx,
        resume_rx,
    );

    let render_id = id.clone();
    let render_thread =
        std::thread::spawn(move || v2_render::editor_v2_render_update(render_id, None, None));
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("render snapshot reached the forced pause");

    let mutation_id = id.clone();
    let (mutation_tx, mutation_rx) = sync_channel(1);
    let mutation_thread = std::thread::spawn(move || {
        let result = v2::editor_v2_apply_input(mutation_id, input_envelope(71, base_revision, "Z"));
        mutation_tx.send(result).unwrap();
    });
    assert!(matches!(
        mutation_rx.recv_timeout(Duration::from_millis(50)),
        Err(RecvTimeoutError::Timeout)
    ));

    resume_tx.send(()).unwrap();
    let snapshot = ok_json(&render_thread.join().expect("render thread succeeds"));
    let mutation = mutation_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mutation completes after the snapshot releases the editor");
    ok_json(&mutation);
    mutation_thread.join().expect("mutation thread succeeds");

    assert_eq!(
        snapshot["documentVersion"],
        state_before["documentRevision"]
    );
    assert_eq!(snapshot["stateRevision"], state_before["stateRevision"]);
    assert_eq!(
        snapshot["historyState"],
        json!({ "canUndo": false, "canRedo": false })
    );
    assert_eq!(snapshot["selection"]["type"], json!("text"));
    assert_eq!(snapshot["selection"]["anchor"], json!(1));
    assert_eq!(snapshot["selection"]["head"], json!(1));
    assert!(snapshot["scalarLength"].as_u64().is_some());
    assert!(
        !snapshot["renderBlocks"].to_string().contains('Z'),
        "render content must come from the same pre-mutation state"
    );
    assert_eq!(revision_of(&id), base_revision + 1);
    destroy_handle(&id);
}

// Hard cutover: without a mirror, the snapshot carries and evaluates the
// authoritative engine selection. A supplied mirror explicitly replaces
// that selection for the snapshot.
const ACTIVE_STATE_DOC: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"plain "},{"type":"text","text":"bold","marks":[{"type":"bold"}]}]}]}"#;

#[test]
fn render_update_active_state_uses_authoritative_or_explicit_mirror_selection() {
    let id = create_handle(local_json_config(ACTIVE_STATE_DOC));

    // The authoritative initial cursor is at the document start.
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(update["activeState"]["marks"]["bold"], json!(false));

    // A scalar mirror inside the bold word (scalars 7..=10) activates it.
    let update = ok_json(&v2_render::editor_v2_render_update(
        id.clone(),
        Some(8),
        Some(8),
    ));
    assert_eq!(update["activeState"]["marks"]["bold"], json!(true));

    // The engine now tracks a selection inside the bold word. Without a
    // mirror, both selection and active state must use that exact state.
    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(61, revision, 8, 8),
    ));
    let expected_selection = ok_json(&v2_render::editor_v2_resolve_scalar_selection(
        id.clone(),
        8,
        8,
    ));
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(update["selection"], expected_selection);
    assert_eq!(update["activeState"]["marks"]["bold"], json!(true));

    destroy_handle(&id);
}

#[test]
fn render_update_active_state_no_mirror_uses_engine_stored_marks() {
    let id = create_handle(local_json_config(ACTIVE_STATE_DOC));

    // Collapse the engine selection into the plain region and toggle bold:
    // the engine records a stored mark for the next typed character.
    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(62, revision, 3, 3),
    ));
    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            63,
            revision,
            json!({ "type": "toggleMark", "markType": "bold" }),
        ),
    ));

    // The atomic snapshot evaluates the authoritative selection and its
    // stored marks together.
    let update = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(update["activeState"]["marks"]["bold"], json!(true));

    destroy_handle(&id);
}

#[test]
fn staging_render_accessor_errors_are_structured() {
    // Unknown session: lifecycle/ENGINE_DESTROYED on every accessor.
    let unknown = "424242".to_string();
    for result in [
        v2_render::editor_v2_render_update(unknown.clone(), None, None),
        v2_render::editor_v2_resolve_scalar_selection(unknown.clone(), 0, 0),
        v2_render::editor_v2_doc_to_scalar(unknown.clone(), 0),
        v2_render::editor_v2_scalar_to_doc(unknown.clone(), 0),
    ] {
        let error = err_json(&result);
        assert_error(&error, "lifecycle", "ENGINE_DESTROYED", None);
    }

    // Malformed handle: boundary/CONFIG_INVALID, no request id.
    let error = err_json(&v2_render::editor_v2_render_update(
        "not-a-handle".into(),
        None,
        None,
    ));
    assert_error(&error, "boundary", "CONFIG_INVALID", None);

    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    // A one-sided mirror is a boundary misuse, never a guessed selection.
    for (anchor, head) in [(Some(1u32), None), (None, Some(1u32))] {
        let error = err_json(&v2_render::editor_v2_render_update(
            id.clone(),
            anchor,
            head,
        ));
        assert_error(&error, "boundary", "CONFIG_INVALID", None);
    }

    // An AwaitRemote room owns no document yet: operation/ENGINE_NOT_READY.
    let room = create_handle(room_config(None));
    for result in [
        v2_render::editor_v2_render_update(room.clone(), None, None),
        v2_render::editor_v2_resolve_scalar_selection(room.clone(), 0, 0),
        v2_render::editor_v2_doc_to_scalar(room.clone(), 0),
        v2_render::editor_v2_scalar_to_doc(room.clone(), 0),
    ] {
        let error = err_json(&result);
        assert_error(&error, "operation", "ENGINE_NOT_READY", None);
    }
    destroy_handle(&room);

    // Destroyed session: lifecycle/ENGINE_DESTROYED.
    destroy_handle(&id);
    let error = err_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_error(&error, "lifecycle", "ENGINE_DESTROYED", None);
}

/// Reported active mark state, as the toolbar reads it.
///
/// `NativeToolbarState` on iOS is built from the render update's `activeState`
/// (see `activeState["marks"]` in `NativeEditorExpoView.swift`), so that is the
/// surface a toolbar button's lit/unlit state actually comes from.
fn active_mark(id: &str, mark_type: &str) -> Value {
    let update = ok_json(&v2_render::editor_v2_render_update(
        id.to_string(),
        None,
        None,
    ));
    update["activeState"]["marks"][mark_type].clone()
}

/// Toolbar button state must update the moment the button is pressed.
///
/// Pressing bold with a collapsed caret is a state-only transaction — it stores
/// the mark without touching the document — so if the reported active state
/// ignores stored marks the button stays unlit until the user types a character
/// and the document finally carries the mark. That is the "bold doesn't light
/// up until I type" behaviour.
#[test]
fn collapsed_mark_toggle_updates_reported_toolbar_state_before_the_next_character() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(1, 0, json!({ "type": "insertText", "text": "word" })),
    ));
    assert_eq!(
        active_mark(&id, "bold"),
        json!(false),
        "precondition: bold is off while typing plain text"
    );

    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            2,
            revision,
            json!({ "type": "toggleMark", "markType": "bold" }),
        ),
    ));

    assert_eq!(
        active_mark(&id, "bold"),
        json!(true),
        "the bold button must read as active immediately after it is pressed, \
         before any character is typed"
    );

    // And it must stay active once the next character actually arrives.
    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(3, revision, json!({ "type": "insertText", "text": "X" })),
    ));
    assert_eq!(
        active_mark(&id, "bold"),
        json!(true),
        "bold must remain active while typing inside the bold run"
    );

    destroy_handle(&id);
}

/// The mirror: switching a mark off with a collapsed caret must clear the
/// button immediately too, rather than waiting for the next keystroke.
#[test]
fn collapsed_mark_untoggle_clears_reported_toolbar_state_immediately() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(1, 0, json!({ "type": "toggleMark", "markType": "bold" })),
    ));
    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision, json!({ "type": "insertText", "text": "bold" })),
    ));
    assert_eq!(active_mark(&id, "bold"), json!(true));

    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            3,
            revision,
            json!({ "type": "toggleMark", "markType": "bold" }),
        ),
    ));
    assert_eq!(
        active_mark(&id, "bold"),
        json!(false),
        "switching bold off must unlight the button before the next character"
    );

    destroy_handle(&id);
}

/// The caret the host renders, as scalar offsets.
///
/// A collapsed caret serializes as a text selection whose anchor and head
/// coincide; the scalar pair is what the native view maps onto its own text
/// storage, so it is the offset a user sees the caret drawn at.
fn caret_scalar(id: &str) -> u64 {
    let update = ok_json(&v2_render::editor_v2_render_update(
        id.to_string(),
        None,
        None,
    ));
    let selection = &update["selection"];
    assert_eq!(selection["type"], json!("text"), "{selection:?}");
    let anchor = selection["anchorScalar"]
        .as_u64()
        .unwrap_or_else(|| panic!("selection carries a scalar anchor: {selection:?}"));
    let head = selection["headScalar"]
        .as_u64()
        .unwrap_or_else(|| panic!("selection carries a scalar head: {selection:?}"));
    assert_eq!(anchor, head, "the caret must stay collapsed: {selection:?}");
    anchor
}

/// Converting a line into a list item must leave the caret on the same
/// character it was on before.
///
/// Wrapping shifts every scalar offset in the line: the bullet list, list item,
/// and paragraph opening tokens sit in front of the text, so the same character
/// reports a higher offset afterwards. If the caret is carried over as a raw
/// number rather than re-resolved through the new structure, it lands short of
/// where the user left it — visibly jumping backwards into the text.
#[test]
fn converting_a_line_into_a_list_item_keeps_the_caret_on_the_same_character() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(1, 0, "one"),
    ));
    assert_eq!(
        caret_scalar(&id),
        3,
        "precondition: the caret sits after the third character of a bare line"
    );

    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            2,
            revision,
            json!({ "type": "applyListType", "listType": "bulletList" }),
        ),
    ));

    // "one" now begins two scalars in, behind the list and item openings, so the
    // end of the same text is offset 5 rather than 3.
    assert_eq!(
        caret_scalar(&id),
        5,
        "the caret must still sit at the end of the converted line, not at the \
         offset it held before the wrap"
    );

    destroy_handle(&id);
}

#[test]
fn default_schema_list_wrap_keeps_the_caret_on_the_same_character() {
    let created = ok_json(&v2::editor_v2_create(
        json!({ "initialization": { "type": "localEmpty" } }).to_string(),
        None,
    ));
    let id = created["editorId"].as_str().unwrap().to_string();

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(1, 0, json!({ "type": "insertText", "text": "one" })),
    ));
    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            2,
            revision,
            json!({
                "type": "wrapInList",
                "listType": "bullet_list",
                "itemType": "list_item"
            }),
        ),
    ));

    assert_eq!(caret_scalar(&id), 5);
    destroy_handle(&id);
}

/// The same check with the caret parked mid-word rather than at the end, so a
/// fix that merely pins the caret to the end of the line cannot pass.
#[test]
fn converting_a_line_into_a_list_item_keeps_a_mid_word_caret_in_place() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(1, 0, json!({ "type": "insertText", "text": "one" })),
    ));
    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(2, revision_of(&id), 1, 1),
    ));
    assert_eq!(caret_scalar(&id), 1, "precondition: caret between o and n");

    let revision = revision_of(&id);
    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(
            3,
            revision,
            json!({ "type": "applyListType", "listType": "bulletList" }),
        ),
    ));

    assert_eq!(
        caret_scalar(&id),
        3,
        "a caret one character into the line must still be one character in \
         after the wrap"
    );

    destroy_handle(&id);
}

/// Emptiness must be answerable from the core, not re-derived by the host.
///
/// The iOS placeholder is currently driven by scanning the rendered characters
/// in the text view's own storage (`RichTextEditorView.isRenderedContentEmpty`).
/// That scan structurally cannot see an empty list item: the bullet marker is
/// drawn from block structure rather than stored as text, so a document holding
/// one empty bullet looks character-for-character identical to an empty
/// document and the placeholder stays up over a visible bullet.
///
/// The render update is the payload the host already consumes, so it has to
/// carry a signal that separates the two.
#[test]
fn the_render_update_distinguishes_an_empty_document_from_an_empty_list_item() {
    let empty = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let empty_update = ok_json(&v2_render::editor_v2_render_update(
        empty.clone(),
        None,
        None,
    ));

    let listed = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    ok_json(&v2::editor_v2_apply_command(
        listed.clone(),
        command_envelope(
            1,
            0,
            json!({ "type": "applyListType", "listType": "bulletList" }),
        ),
    ));
    let listed_update = ok_json(&v2_render::editor_v2_render_update(
        listed.clone(),
        None,
        None,
    ));

    assert_eq!(
        empty_update["documentIsEmpty"],
        json!(true),
        "a fresh editor holds nothing the user authored"
    );
    assert_eq!(
        listed_update["documentIsEmpty"],
        json!(false),
        "one empty bullet is content: it renders no characters, so only the \
         core can tell the host this editor is no longer empty"
    );

    destroy_handle(&empty);
    destroy_handle(&listed);
}

/// A blank second line is content too.
///
/// Pressing Return in an empty editor leaves two blank lines. Not one character
/// exists in the document, so nothing downstream of the rendered text can tell
/// this apart from an untouched editor — only the core knows the user added a
/// line, and the placeholder has to get out of the way of it.
#[test]
fn a_blank_line_added_with_return_stops_the_document_being_empty() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    let before = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(
        before["documentIsEmpty"],
        json!(true),
        "precondition: a fresh editor is empty"
    );

    ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(1, 0, json!({ "type": "splitBlock" })),
    ));

    let after = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    assert_eq!(
        after["documentIsEmpty"],
        json!(false),
        "two blank lines are content, even though neither holds a character"
    );

    // The caret belongs on the new second line, not left behind on the first.
    // Both lines are blank, so the second line is the end of the document.
    assert_eq!(
        json!(caret_scalar(&id)),
        after["scalarLength"],
        "Return must leave the caret on the blank line it just created, which \
         with both lines blank is the end of the document"
    );

    destroy_handle(&id);
}
