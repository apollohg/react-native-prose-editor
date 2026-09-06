#[test]
fn create_local_editor_exposes_full_state_surface_and_destroy_lifecycle() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    assert!(
        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()),
        "handle is a decimal string: {id:?}",
    );

    // get_state: exact shape and values on a fresh local editor.
    let state = state_of(&id);
    assert_eq!(
        state,
        json!({
            "documentState": "LocalReady",
            "transportState": "Detached",
            "renderState": "Ready",
            "documentRevision": "0",
            "documentOrigin": "import",
            "stateRevision": "0",
            "canUndo": false,
            "canRedo": false,
        }),
        "{state:?}",
    );

    // get_document_json is the bare document JSON; get_document_html wraps
    // the HTML string; get_content_snapshot carries both.
    let document_json = document_json_of(&id);
    assert_eq!(document_json["type"], "doc", "{document_json:?}");
    assert!(document_json["content"].is_array(), "{document_json:?}");
    let document_html = ok_json(&v2::editor_v2_get_document_html(id.clone()));
    assert!(document_html["html"].is_string(), "{document_html:?}");
    let snapshot = ok_json(&v2::editor_v2_get_content_snapshot(id.clone()));
    assert_eq!(snapshot["json"], document_json, "{snapshot:?}");
    assert_eq!(snapshot["html"], document_html["html"], "{snapshot:?}");

    // destroy: unit success the first time, structured lifecycle error on
    // the replay, and every later call refused without a request id.
    destroy_handle(&id);
    let error = err_unit(&v2::editor_v2_destroy(id.clone()));
    assert_error(&error, "lifecycle", "ENGINE_DESTROYED", None);
    let error = err_json(&v2::editor_v2_get_state(id.clone()));
    assert_error(&error, "lifecycle", "ENGINE_DESTROYED", None);
}

#[test]
fn create_with_initial_content_and_invalid_content_errors() {
    let id = create_handle(json!({
        "initialization": { "type": "localJson", "json": serde_json::from_str::<Value>(JSON_SEED).unwrap() },
    }));
    assert_eq!(
        document_json_of(&id),
        serde_json::from_str::<Value>(JSON_SEED).unwrap()
    );
    let html = ok_json(&v2::editor_v2_get_document_html(id.clone()));
    assert!(
        html["html"].as_str().unwrap().contains("ffi seed"),
        "{html:?}",
    );
    destroy_handle(&id);

    let id = create_handle(json!({
        "initialization": { "type": "localHtml", "html": SEED_HTML },
    }));
    assert!(
        document_json_of(&id).to_string().contains("html seed"),
        "{:?}",
        document_json_of(&id),
    );
    destroy_handle(&id);

    // A structurally invalid document rejects with the document domain.
    let result = v2::editor_v2_create(
        json!({ "initialization": { "type": "localJson", "json": { "type": "bogus" } } })
            .to_string(),
        None,
    );
    let error = err_json(&result);
    assert_error(&error, "document", "DOCUMENT_INVALID", None);

    // A malformed create envelope rejects before any registry work.
    let result = v2::editor_v2_create("{not json".into(), None);
    let error = err_json(&result);
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
    let result = v2::editor_v2_create(json!({ "bogus": true }).to_string(), None);
    let error = err_json(&result);
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
}

#[test]
fn create_room_with_snapshot_bytes_and_pairing_rules() {
    let snapshot = snapshot_source();

    // Snapshot metadata rides in the room config; the encoded state rides as
    // direct bytes in the separate parameter (never a JSON number array).
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let state = state_of(&id);
    assert_eq!(state["documentState"], "RoomReady", "{state:?}");
    assert_eq!(state["transportState"], "Disconnected", "{state:?}");
    assert_eq!(
        document_json_of(&id),
        serde_json::from_str::<Value>(JSON_SEED).unwrap()
    );
    destroy_handle(&id);

    // A snapshot-less room starts AwaitRemote: getters that need a document
    // refuse ENGINE_NOT_READY while state stays readable (loading render).
    let id = create_handle(room_config(None));
    let state = state_of(&id);
    assert_eq!(state["documentState"], "AwaitRemote", "{state:?}");
    assert_eq!(state["renderState"], "Loading", "{state:?}");
    let error = err_json(&v2::editor_v2_get_document_json(id.clone()));
    assert_error(&error, "operation", "ENGINE_NOT_READY", None);
    destroy_handle(&id);

    // Pairing rules: metadata without bytes, bytes without metadata, and
    // bytes on a non-room initialization all reject atomically.
    let result = v2::editor_v2_create(room_config(Some(&snapshot)).to_string(), None);
    let error = err_json(&result);
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
    let result = v2::editor_v2_create(
        room_config(None).to_string(),
        Some(snapshot.encoded_state.clone()),
    );
    let error = err_json(&result);
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
    let result = v2::editor_v2_create(
        json!({ "initialization": { "type": "localEmpty" } }).to_string(),
        Some(snapshot.encoded_state.clone()),
    );
    let error = err_json(&result);
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
}

#[test]
fn malformed_handles_fail_with_structured_boundary_errors() {
    for handle in ["not-a-handle", "", "-1", "18446744073709551616"] {
        let error = err_json(&v2::editor_v2_get_state(handle.to_string()));
        assert_error(&error, "boundary", "CONFIG_INVALID", None);
        let error = err_json(&v2::editor_v2_apply_input(
            handle.to_string(),
            input_envelope(401, 0, "x"),
        ));
        assert_error(&error, "boundary", "CONFIG_INVALID", None);
        let result =
            v2_collab::editor_v2_collaboration_lease_outbound(handle.to_string(), "1".into());
        assert!(result.value.is_none(), "{result:?}");
        assert_error(
            &result.error.expect("error"),
            "boundary",
            "CONFIG_INVALID",
            None,
        );
    }
}

#[test]
fn unknown_editor_id_fails_every_entry_with_a_lifecycle_error() {
    let unknown = "777777".to_string();
    let assert_lifecycle = |error: FfiError| {
        assert_error(&error, "lifecycle", "ENGINE_DESTROYED", None);
    };

    assert_lifecycle(err_json(&v2::editor_v2_get_state(unknown.clone())));
    assert_lifecycle(err_json(&v2::editor_v2_get_document_json(unknown.clone())));
    assert_lifecycle(err_json(&v2::editor_v2_get_document_html(unknown.clone())));
    assert_lifecycle(err_json(&v2::editor_v2_get_content_snapshot(
        unknown.clone(),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_replace_document(
        unknown.clone(),
        json!({
            "version": 1,
            "requestId": "501",
            "setJson": { "type": "doc" },
            "history": "resetAndClear",
        })
        .to_string(),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_apply_input(
        unknown.clone(),
        input_envelope(502, 0, "x"),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_apply_command(
        unknown.clone(),
        command_envelope(503, 0, json!({ "type": "insertText", "text": "x" })),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_apply_local_api(
        unknown.clone(),
        replace_envelope(504, 0, JSON_SEED, "resetAndClear"),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_set_selection(
        unknown.clone(),
        selection_envelope(505, 0, 0, 0),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_undo(
        unknown.clone(),
        history_envelope(506),
    )));
    assert_lifecycle(err_json(&v2::editor_v2_redo(
        unknown.clone(),
        history_envelope(507),
    )));
    assert_lifecycle(err_unit(&v2::editor_v2_destroy(unknown.clone())));
    assert_lifecycle(err_json(&v2_collab::editor_v2_collaboration_drive(
        unknown.clone(),
        "0".into(),
    )));
    assert_lifecycle(err_json(&v2_collab::editor_v2_collaboration_socket_open(
        unknown.clone(),
        "1".into(),
        "0".into(),
    )));
    assert_lifecycle(err_json(&v2_collab::editor_v2_collaboration_receive(
        unknown.clone(),
        "1".into(),
        vec![0],
        "0".into(),
    )));
    assert_lifecycle(err_json(&v2_collab::editor_v2_collaboration_socket_close(
        unknown.clone(),
        "1".into(),
        None,
        None,
        "0".into(),
    )));
    let result = v2_collab::editor_v2_collaboration_lease_outbound(unknown.clone(), "1".into());
    assert!(result.value.is_none(), "{result:?}");
    assert_lifecycle(result.error.expect("error"));
    let result = v2_collab::editor_v2_collaboration_set_awareness(unknown.clone(), "{}".into());
    assert!(result.value.is_none(), "{result:?}");
    assert_lifecycle(result.error.expect("error"));
    assert_lifecycle(err_json(&v2_collab::editor_v2_collaboration_peers(
        unknown.clone(),
    )));
    let result = v2_snapshot::editor_v2_snapshot_export(unknown.clone());
    assert!(result.value.is_none(), "{result:?}");
    assert_lifecycle(result.error.expect("error"));
    assert_lifecycle(err_json(&v2_snapshot::editor_v2_snapshot_restore(
        unknown.clone(),
        "{}".into(),
        vec![],
    )));
}

#[test]
fn destroy_during_in_flight_calls_refuses_without_partial_work() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // The barrier guarantees the worker completes at least one full call
    // cycle before the destroy begins, so the race is genuine.
    let first_cycle_done = std::sync::Arc::new(std::sync::Barrier::new(2));
    let worker = {
        let id = id.clone();
        let stop = stop.clone();
        let first_cycle_done = first_cycle_done.clone();
        std::thread::spawn(move || {
            let mut revisions: Vec<Result<u64, (String, String)>> = Vec::new();
            let mut first = true;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let state = v2::editor_v2_get_state(id.clone());
                match (state.value, state.error) {
                    (Some(value), None) => {
                        let parsed: Value = serde_json::from_str(&value).unwrap();
                        let revision = parsed["documentRevision"]
                            .as_str()
                            .unwrap()
                            .parse::<u64>()
                            .unwrap();
                        revisions.push(Ok(revision));
                        let input = v2::editor_v2_apply_input(
                            id.clone(),
                            input_envelope(551, revision, "race"),
                        );
                        match (input.value, input.error) {
                            (Some(_), None) => {}
                            (None, Some(error)) => {
                                assert_eq!(error.domain, "lifecycle", "{error:?}");
                            }
                            torn => panic!("torn result: {torn:?}"),
                        }
                    }
                    (None, Some(error)) => revisions.push(Err((error.domain, error.code))),
                    torn => panic!("torn result: {torn:?}"),
                }
                if first {
                    first = false;
                    first_cycle_done.wait();
                }
                std::thread::yield_now();
            }
            revisions
        })
    };
    // Destroy from this handle while the worker's calls are in flight.
    first_cycle_done.wait();
    destroy_handle(&id);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let revisions = worker.join().expect("worker must never panic");
    assert!(!revisions.is_empty(), "the worker observed calls");

    // Every in-flight call either completed cleanly or refused with a
    // lifecycle error — never a panic, never a torn result.
    let mut last = 0;
    for revision in revisions
        .iter()
        .filter_map(|entry| entry.as_ref().ok().copied())
    {
        assert!(revision >= last, "revisions never regress: {revisions:?}");
        last = revision;
    }
    for (domain, code) in revisions
        .iter()
        .filter_map(|entry| entry.as_ref().err().cloned())
    {
        assert_eq!(domain, "lifecycle", "{code:?}");
        assert!(
            code == "ENGINE_DESTROYING" || code == "ENGINE_DESTROYED",
            "{code:?}",
        );
    }

    // Post-destroy: every entry refuses; a fresh editor is unaffected.
    assert_error(
        &err_json(&v2::editor_v2_get_state(id.clone())),
        "lifecycle",
        "ENGINE_DESTROYED",
        None,
    );
    let fresh = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    assert!(ok_json(&v2::editor_v2_get_state(fresh.clone())).is_object());
    destroy_handle(&fresh);
}

#[test]
fn apply_input_command_selection_and_local_api_outcome_matrix() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let base = revision_of(&id);

    // Input commit: typed transaction outcome with revisions and history.
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(601, base, "hello"),
    ));
    assert_eq!(
        outcome,
        json!({
            "type": "transaction",
            "changed": true,
            "documentRevision": (base + 1).to_string(),
            "stateRevision": outcome["stateRevision"],
            "canUndo": true,
            "canRedo": false,
        }),
        "{outcome:?}",
    );
    assert!(
        document_json_of(&id).to_string().contains("hello"),
        "{:?}",
        document_json_of(&id),
    );

    // Stale base revision: exact operation error with decimal request id and
    // structured details; limit/actual stay absent.
    let error = err_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(602, base, "stale"),
    ));
    assert_error(&error, "operation", "REVISION_MISMATCH", Some("602"));
    assert_eq!(error.limit, None, "{error:?}");
    assert_eq!(error.actual, None, "{error:?}");
    assert_eq!(error.operation_index, None, "{error:?}");
    let details: Value = serde_json::from_str(error.details_json.as_deref().unwrap()).unwrap();
    assert_eq!(
        details,
        json!({
            "expectedRevision": base.to_string(),
            "actualRevision": (base + 1).to_string(),
        }),
        "{error:?}",
    );

    // Envelope admission: bad version, the removed origin field, and empty
    // input text all reject before any engine work.
    for envelope in [
        json!({ "version": 2, "requestId": "603", "baseDocumentRevision": revision_of(&id).to_string(), "text": "x" }),
        json!({ "version": 1, "requestId": "604", "baseDocumentRevision": revision_of(&id).to_string(), "text": "x", "origin": "remote" }),
        json!({ "version": 1, "requestId": "605", "baseDocumentRevision": revision_of(&id).to_string(), "text": "" }),
    ] {
        // The bounded request-id probe preserves a canonical ID even when a
        // later exact-envelope parse rejects the removed origin field.
        let expected_request_id = envelope["requestId"].as_str().map(str::to_owned);
        let error = err_json(&v2::editor_v2_apply_input(id.clone(), envelope.to_string()));
        assert_error(
            &error,
            "boundary",
            "CONFIG_INVALID",
            expected_request_id.as_deref(),
        );
    }

    // Command: applicable lowers to a transaction; structurally inapplicable
    // is a structured notApplicable outcome, not an error.
    let base = revision_of(&id);
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(606, base, json!({ "type": "insertText", "text": " world" })),
    ));
    assert_eq!(outcome["type"], "transaction", "{outcome:?}");
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(607, revision_of(&id), json!({ "type": "outdentListItem" })),
    ));
    assert_eq!(outcome, json!({ "type": "notApplicable" }), "{outcome:?}");

    // Selection: state-only transaction outcome; revision unchanged.
    let base = revision_of(&id);
    let outcome = ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(608, base, 1, 3),
    ));
    assert_eq!(outcome["type"], "transaction", "{outcome:?}");
    assert_eq!(outcome["documentRevision"], base.to_string(), "{outcome:?}");

    // Local-API whole-document replacement: replacement outcome shape.
    let outcome = ok_json(&v2::editor_v2_apply_local_api(
        id.clone(),
        replace_envelope(609, revision_of(&id), JSON_SEED, "undoableBoundary"),
    ));
    assert_eq!(outcome["type"], "replacement", "{outcome:?}");
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    assert_eq!(
        document_json_of(&id),
        serde_json::from_str::<Value>(JSON_SEED).unwrap()
    );
    destroy_handle(&id);
}

#[test]
fn replace_document_session_seam_and_policy_gate() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let outcome = ok_json(&v2::editor_v2_replace_document(
        id.clone(),
        json!({
            "version": 1,
            "requestId": "651",
            "setJson": serde_json::from_str::<Value>(JSON_SEED).unwrap(),
            "history": "resetAndClear",
        })
        .to_string(),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    assert_eq!(
        document_json_of(&id),
        serde_json::from_str::<Value>(JSON_SEED).unwrap()
    );
    assert_eq!(
        state_of(&id)["canUndo"],
        false,
        "resetAndClear clears history"
    );

    // Exactly one of setJson/setHtml is required.
    let error = err_json(&v2::editor_v2_replace_document(
        id.clone(),
        json!({ "version": 1, "requestId": "652", "history": "resetAndClear" }).to_string(),
    ));
    assert_error(&error, "boundary", "CONFIG_INVALID", Some("652"));
    destroy_handle(&id);

    // AwaitRemote refuses replacement with ENGINE_NOT_READY.
    let id = create_handle(room_config(None));
    let error = err_json(&v2::editor_v2_replace_document(
        id.clone(),
        json!({
            "version": 1,
            "requestId": "653",
            "setJson": serde_json::from_str::<Value>(JSON_SEED).unwrap(),
            "history": "resetAndClear",
        })
        .to_string(),
    ));
    assert_error(&error, "operation", "ENGINE_NOT_READY", Some("653"));
    destroy_handle(&id);
}

#[test]
fn undo_redo_success_and_read_only_rejection_with_atomic_audit() {
    // Writable editor: undo/redo walk history with exact outcome shapes.
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let before_edit = document_json_of(&id);
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(701, revision_of(&id), "undoable"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");

    let outcome = ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(702)));
    assert_eq!(outcome, json!({ "changed": true }), "{outcome:?}");
    assert_eq!(document_json_of(&id), before_edit, "undo reverts the edit");
    assert_eq!(state_of(&id)["canRedo"], true);

    let outcome = ok_json(&v2::editor_v2_redo(id.clone(), history_envelope(703)));
    assert_eq!(outcome, json!({ "changed": true }), "{outcome:?}");
    assert!(
        document_json_of(&id).to_string().contains("undoable"),
        "{:?}",
        document_json_of(&id),
    );

    // Undo on exhausted history is a structured false, not an error.
    assert_eq!(
        ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(704))),
        json!({ "changed": true }),
    );
    assert_eq!(
        ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(705))),
        json!({ "changed": false }),
    );
    destroy_handle(&id);

    // Read-only editor: input, command, undo, and redo all reject with the
    // structured policy refusal; selection, local-API, and getters pass.
    let id = create_handle(json!({
        "initialization": { "type": "localJson", "json": serde_json::from_str::<Value>(JSON_SEED).unwrap() },
        "policy": { "readOnly": true },
    }));
    let state_before = state_of(&id);
    let document_before = document_json_of(&id);

    for (label, result, request_id) in [
        (
            "input",
            v2::editor_v2_apply_input(id.clone(), input_envelope(711, revision_of(&id), "x")),
            "711",
        ),
        (
            "command",
            v2::editor_v2_apply_command(
                id.clone(),
                command_envelope(
                    712,
                    revision_of(&id),
                    json!({ "type": "insertText", "text": "x" }),
                ),
            ),
            "712",
        ),
        (
            "undo",
            v2::editor_v2_undo(id.clone(), history_envelope(713)),
            "713",
        ),
        (
            "redo",
            v2::editor_v2_redo(id.clone(), history_envelope(714)),
            "714",
        ),
    ] {
        let error = err_json(&result);
        assert_error(&error, "boundary", "MUTATION_REJECTED", Some(request_id));
        assert!(error.message.contains("read-only"), "{label}: {error:?}",);
    }

    // Full atomic audit after every rejection: nothing moved.
    assert_eq!(state_of(&id), state_before, "read-only audit: state");
    assert_eq!(
        document_json_of(&id),
        document_before,
        "read-only audit: json"
    );

    // Selection stays available under read-only; local-API keeps the legacy
    // Source::Api pass-through; getters are unaffected.
    let outcome = ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope(717, revision_of(&id), 0, 1),
    ));
    assert_eq!(outcome["type"], "transaction", "{outcome:?}");
    let outcome = ok_json(&v2::editor_v2_apply_local_api(
        id.clone(),
        replace_envelope(718, revision_of(&id), JSON_SEED, "undoableBoundary"),
    ));
    assert_eq!(outcome["type"], "replacement", "{outcome:?}");
    destroy_handle(&id);
}

#[test]
fn input_filter_preserves_exact_semantics_and_replays_compile_errors() {
    // Per-character semantics across many commits: each committed character
    // is kept only if it matches the cached pattern.
    let id = create_handle(json!({
        "initialization": { "type": "localEmpty" },
        "policy": { "inputFilter": "^[0-9]$" },
    }));
    for index in 0..40u64 {
        let text = format!("a{index}b");
        let outcome = ok_json(&v2::editor_v2_apply_input(
            id.clone(),
            input_envelope(801 + index, revision_of(&id), &text),
        ));
        assert_eq!(outcome["changed"], true, "{outcome:?}");
    }
    let expected: String = (0..40u64).map(|index| index.to_string()).collect();
    assert!(
        document_json_of(&id).to_string().contains(&expected),
        "every commit must filter to digits only: {:?}",
        document_json_of(&id),
    );
    destroy_handle(&id);

    // A fully filtered commit lowers to a real no-op transaction.
    let id = create_handle(json!({
        "initialization": { "type": "localEmpty" },
        "policy": { "inputFilter": "^[0-9]$" },
    }));
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(851, revision_of(&id), "abc"),
    ));
    assert_eq!(outcome["changed"], false, "{outcome:?}");
    destroy_handle(&id);

    // An invalid pattern replays the identical structured error on every
    // request (cached compile failure, never a panic).
    let id = create_handle(json!({
        "initialization": { "type": "localEmpty" },
        "policy": { "inputFilter": "[unclosed" },
    }));
    let first = err_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(861, revision_of(&id), "x"),
    ));
    assert_error(&first, "boundary", "CONFIG_INVALID", Some("861"));
    for request_id in 862..=863u64 {
        let error = err_json(&v2::editor_v2_apply_input(
            id.clone(),
            input_envelope(request_id, revision_of(&id), "x"),
        ));
        assert_error(
            &error,
            "boundary",
            "CONFIG_INVALID",
            Some(&request_id.to_string()),
        );
        assert_eq!(error.message, first.message, "identical replay");
    }
    destroy_handle(&id);
}
