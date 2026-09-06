#[test]
fn snapshot_export_restore_round_trip_and_policy_errors() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(1001, revision_of(&id), " persisted"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");

    // Export: metadata JSON plus direct state bytes.
    let export = v2_snapshot::editor_v2_snapshot_export(id.clone());
    assert!(export.error.is_none(), "{:?}", export.error);
    let export = export.value.expect("export carries a snapshot");
    let metadata: Value = serde_json::from_str(&export.metadata_json).unwrap();
    assert_eq!(
        metadata,
        json!({
            "formatVersion": 1,
            "documentId": DOCUMENT_ID,
            "lineageId": LINEAGE_ID,
            "fragmentName": FRAGMENT_NAME,
            "schemaFingerprint": snapshot.schema_fingerprint,
        }),
        "{metadata:?}",
    );
    assert!(!export.encoded_state.is_empty(), "direct state bytes");

    // Restore into an AwaitRemote room of the same scope: promotes to
    // RoomReady with the persisted document; the second restore is a
    // structured no-op.
    let target = create_handle(room_config(None));
    let outcome = ok_json(&v2_snapshot::editor_v2_snapshot_restore(
        target.clone(),
        export.metadata_json.clone(),
        export.encoded_state.clone(),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    assert_eq!(state_of(&target)["documentState"], "RoomReady");
    assert_eq!(state_of(&target)["transportState"], "Disconnected");
    assert_eq!(document_json_of(&target), document_json_of(&id));
    let outcome = ok_json(&v2_snapshot::editor_v2_snapshot_restore(
        target.clone(),
        export.metadata_json.clone(),
        export.encoded_state.clone(),
    ));
    assert_eq!(outcome["changed"], false, "{outcome:?}");

    // A tampered manifest rejects in the snapshot domain with the audit
    // fully preserved.
    let state_before = state_of(&target);
    let document_before = document_json_of(&target);
    let mut tampered = metadata.clone();
    tampered["lineageId"] = json!("other-lineage");
    let error = err_json(&v2_snapshot::editor_v2_snapshot_restore(
        target.clone(),
        tampered.to_string(),
        export.encoded_state.clone(),
    ));
    assert_error(&error, "snapshot", "SNAPSHOT_LINEAGE_MISMATCH", None);
    assert_eq!(state_of(&target), state_before);
    assert_eq!(document_json_of(&target), document_before);

    // Garbage state bytes never reach decode-time mutation.
    let error = err_json(&v2_snapshot::editor_v2_snapshot_restore(
        target.clone(),
        export.metadata_json.clone(),
        vec![0xff, 0xff, 0xff],
    ));
    assert_error(&error, "snapshot", "COLLABORATION_DECODE_FAILED", None);
    assert_eq!(state_of(&target), state_before);
    assert_eq!(document_json_of(&target), document_before);

    // Malformed metadata JSON is a boundary error before any session work.
    let error = err_json(&v2_snapshot::editor_v2_snapshot_restore(
        target.clone(),
        "{not json".into(),
        export.encoded_state.clone(),
    ));
    assert_error(&error, "boundary", "CONFIG_INVALID", None);

    // The transport gate: a synchronized editor refuses restore.
    let connected = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let _generation = synchronize_v2(&connected, &RawPeer::from_snapshot(&snapshot));
    let error = err_json(&v2_snapshot::editor_v2_snapshot_restore(
        connected.clone(),
        export.metadata_json.clone(),
        export.encoded_state.clone(),
    ));
    assert_error(&error, "snapshot", "SNAPSHOT_RESTORE_CONNECTED", None);
    destroy_handle(&connected);

    // Export requires a room scope.
    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let result = v2_snapshot::editor_v2_snapshot_export(local.clone());
    assert!(result.value.is_none(), "{result:?}");
    assert_error(
        &result.error.expect("error"),
        "snapshot",
        "SNAPSHOT_SCOPE_MISMATCH",
        None,
    );
    destroy_handle(&local);
    destroy_handle(&target);
    destroy_handle(&id);
}

#[test]
fn full_drive_local_editing_to_synchronized_room() {
    // Local editor: input, undo, redo through the mutation entries.
    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let outcome = ok_json(&v2::editor_v2_apply_input(
        local.clone(),
        input_envelope(1101, revision_of(&local), "drive"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    assert_eq!(
        ok_json(&v2::editor_v2_undo(local.clone(), history_envelope(1102))),
        json!({ "changed": true }),
    );
    assert_eq!(
        ok_json(&v2::editor_v2_redo(local.clone(), history_envelope(1103))),
        json!({ "changed": true }),
    );
    destroy_handle(&local);

    // Room editor with a snapshot: the full generation flow against a raw
    // yjs server, ending document-ready with the peer's edit applied.
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);

    // The peer edits; its update frame rides the receive entry as bytes.
    server.push_text(" from server");
    let outcome = receive_v2(
        &id,
        &generation,
        sync_frame(SyncMessage::Update(
            server.diff_for(&snapshot.encoded_state),
        )),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    assert_eq!(outcome["remoteCommitApplied"], true, "{outcome:?}");

    let state = state_of(&id);
    assert_eq!(state["documentState"], "RoomReady", "{state:?}");
    assert_eq!(state["transportState"], "Synchronized", "{state:?}");
    assert!(
        document_json_of(&id).to_string().contains("from server"),
        "{:?}",
        document_json_of(&id),
    );
    let outcome = close_v2(&id, &generation, None, None, 0);
    assert_eq!(outcome["transportState"], "Disconnected", "{outcome:?}");
    assert_eq!(outcome["nextDeadlineMillis"], "500", "{outcome:?}");
    destroy_handle(&id);
}

// Oversize inputs and error-envelope nullability

#[test]
fn oversize_inputs_fail_with_structured_limit_errors() {
    // Create config beyond the bounded config input limit.
    let huge = "x".repeat(21 * 1024 * 1024);
    let result = v2::editor_v2_create(
        json!({
            "initialization": { "type": "localEmpty" },
            "policy": { "inputFilter": huge },
        })
        .to_string(),
        None,
    );
    let error = err_json(&result);
    assert_error(&error, "boundary", "INPUT_LIMIT_EXCEEDED", None);
    assert!(error.limit.is_some(), "{error:?}");
    assert!(
        error
            .actual
            .as_deref()
            .zip(error.limit.as_deref())
            .is_some_and(
                |(actual, limit)| actual.parse::<u64>().unwrap() > limit.parse::<u64>().unwrap()
            ),
        "{error:?}"
    );

    // Mutation envelope beyond the same bounded limit.
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let error = err_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(1201, revision_of(&id), &"x".repeat(21 * 1024 * 1024)),
    ));
    assert_error(&error, "boundary", "INPUT_LIMIT_EXCEEDED", None);
    destroy_handle(&id);

    // An inbound protocol frame beyond maxFrameBytes closes the generation
    // as incompatible through the receive outcome (never a panic).
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));
    let oversized_state = json!({ "pad": "y".repeat(11 * 1024 * 1024) });
    let clients = [(
        yrs::ClientID::new(42_424),
        yrs::sync::awareness::AwarenessUpdateEntry {
            clock: 1,
            json: oversized_state.to_string().into(),
        },
    )]
    .into_iter()
    .collect();
    let frame = Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1();
    let outcome = receive_v2(&id, &generation, frame, 0);
    assert_eq!(outcome["transportState"], "Incompatible", "{outcome:?}");
    assert_eq!(outcome["nextDeadlineMillis"], Value::Null, "{outcome:?}");
    assert_eq!(state_of(&id)["transportState"], "Incompatible");
    destroy_handle(&id);
}

#[test]
fn error_envelopes_pin_nullability_and_decimal_request_ids() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(1301, revision_of(&id), "x"),
    ));

    // Rich error: request id rides as a decimal string, details present, and
    // every other nullable field is absent.
    let error = err_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(u64::MAX, 0, "y"),
    ));
    assert_error(
        &error,
        "operation",
        "REVISION_MISMATCH",
        Some("18446744073709551615"),
    );
    assert_eq!(error.operation_index, None, "{error:?}");
    assert_eq!(error.limit, None, "{error:?}");
    assert_eq!(error.actual, None, "{error:?}");
    assert!(error.details_json.is_some(), "{error:?}");

    // Minimal error: every nullable field absent.
    let error = err_json(&v2::editor_v2_get_state("not-a-handle".into()));
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
    assert_eq!(error.operation_index, None, "{error:?}");
    assert_eq!(error.limit, None, "{error:?}");
    assert_eq!(error.actual, None, "{error:?}");
    assert_eq!(error.details_json, None, "{error:?}");
    destroy_handle(&id);
}

// ---------------------------------------------------------------------------/16C:
// v2 render/selection/position accessor
//
