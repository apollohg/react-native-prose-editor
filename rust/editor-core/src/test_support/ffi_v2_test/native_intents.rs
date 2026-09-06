#[test]
fn native_intent_ffi_is_strict_owner_scoped_and_idempotent() {
    let id = create_handle(json!({
        "initialization": {
            "type": "localJson",
            "json": serde_json::from_str::<Value>(JSON_SEED).unwrap(),
        },
    }));
    assert_eq!(state_of(&id)["documentOrigin"], "import");
    let render = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "4".into(),
        None,
        None,
    ));
    let epoch = render["positionEpoch"].as_str().unwrap().to_owned();
    let request = json!({
        "version": 1,
        "requestId": "1",
        "ownerId": "4",
        "positionEpoch": epoch,
        "intent": {
            "type": "insertText",
            "anchor": 2,
            "head": 2,
            "text": "X",
        },
    });

    let first = v2::editor_v2_apply_native_intent(id.clone(), request.to_string());
    let first_value = first.value.clone().expect("first intent succeeds");
    let duplicate = v2::editor_v2_apply_native_intent(id.clone(), request.to_string());
    assert_eq!(duplicate.value.as_deref(), Some(first_value.as_str()));
    assert_eq!(
        document_json_of(&id)["content"][0]["content"][0]["text"],
        "ffXi seed"
    );
    assert_eq!(state_of(&id)["documentOrigin"], "nativeView");
    let incremental_render = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "4".into(),
        None,
        None,
    ));
    assert_eq!(incremental_render["renderBlocks"], Value::Null);
    assert!(incremental_render["renderPatch"].is_object());
    assert_eq!(
        incremental_render["renderPatch"]["baseDocumentVersion"],
        render["documentVersion"]
    );

    let mut unknown = request.clone();
    unknown["requestId"] = json!("2");
    unknown["intent"]["unexpected"] = json!(true);
    assert_error(
        &err_json(&v2::editor_v2_apply_native_intent(
            id.clone(),
            unknown.to_string(),
        )),
        "boundary",
        "CONFIG_INVALID",
        Some("2"),
    );

    let mut foreign = request.clone();
    foreign["requestId"] = json!("2");
    foreign["ownerId"] = json!("5");
    assert_eq!(
        err_json(&v2::editor_v2_apply_native_intent(
            id.clone(),
            foreign.to_string(),
        ))
        .code,
        "POSITION_EPOCH_INVALID",
    );

    ok_unit(&v2::editor_v2_release_native_binding(
        id.clone(),
        "4".into(),
    ));
    let recovered_render = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "4".into(),
        None,
        None,
    ));
    assert!(recovered_render["renderBlocks"].is_array());
    assert_eq!(recovered_render["renderPatch"], Value::Null);
    assert_eq!(
        err_json(&v2::editor_v2_apply_native_intent(
            id.clone(),
            request.to_string()
        ))
        .code,
        "POSITION_EPOCH_INVALID",
    );
    destroy_handle(&id);
}

#[test]
fn external_full_render_pin_advances_the_native_patch_base() {
    let document = |first: &str, third: &str| {
        json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": first}]},
                {"type": "paragraph", "content": [{"type": "text", "text": "two"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": third}]},
            ],
        })
    };
    let id = create_handle(json!({
        "initialization": {"type": "localJson", "json": document("one", "three")},
    }));
    let initial = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "41".into(),
        None,
        None,
    ));
    assert_eq!(initial["renderBlocks"].as_array().unwrap().len(), 3);

    ok_json(&v2::editor_v2_replace_document(
        id.clone(),
        json!({
            "version": 1,
            "requestId": "1",
            "setJson": document("ONE", "three"),
            "history": "resetAndClear",
        })
        .to_string(),
    ));
    let external = ok_json(&v2_render::editor_v2_render_update(id.clone(), None, None));
    let external_revision = external["documentVersion"].as_str().unwrap();
    ok_json(&v2::editor_v2_pin_position_epoch(
        id.clone(),
        "41".into(),
        external_revision.into(),
    ));

    ok_json(&v2::editor_v2_replace_document(
        id.clone(),
        json!({
            "version": 1,
            "requestId": "2",
            "setJson": document("ONE", "THREE"),
            "history": "resetAndClear",
        })
        .to_string(),
    ));
    let incremental = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "41".into(),
        None,
        None,
    ));
    assert_eq!(incremental["renderBlocks"], Value::Null);
    assert_eq!(incremental["renderPatch"]["startIndex"], 2);
    assert_eq!(incremental["renderPatch"]["deleteCount"], 1);
    assert_eq!(
        incremental["renderPatch"]["renderBlocks"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    destroy_handle(&id);
}

#[test]
fn native_intent_ffi_expires_results_outside_the_replay_window() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let render = ok_json(&v2_render::editor_v2_render_native(
        id.clone(),
        "7".into(),
        None,
        None,
    ));
    let epoch = render["positionEpoch"].as_str().unwrap();
    for request_id in 1..=257_u64 {
        let result = v2::editor_v2_apply_native_intent(
            id.clone(),
            json!({
                "version": 1,
                "requestId": request_id.to_string(),
                "ownerId": "7",
                "positionEpoch": epoch,
                "intent": { "type": "setSelection", "anchor": 0, "head": 0 },
            })
            .to_string(),
        );
        assert!(
            result.error.is_none(),
            "request {request_id}: {:?}",
            result.error
        );
    }
    let expired = err_json(&v2::editor_v2_apply_native_intent(
        id.clone(),
        json!({
            "version": 1,
            "requestId": "1",
            "ownerId": "7",
            "positionEpoch": epoch,
            "intent": { "type": "setSelection", "anchor": 0, "head": 0 },
        })
        .to_string(),
    ));
    assert_error(&expired, "boundary", "EXPIRED_NATIVE_REQUEST", Some("1"));
    destroy_handle(&id);
}

#[test]
fn v2_u64_wire_fields_are_canonical_decimal_strings_and_inputs_reject_numeric_compatibility() {
    let id = create_handle(json!({ "initialization": { "type": "localEmpty" } }));

    let state = state_of(&id);
    assert_eq!(state["documentRevision"], json!("0"));
    assert_eq!(state["stateRevision"], json!("0"));

    let room_id = create_handle(room_config(None));
    let directive = ok_json(&v2_collab::editor_v2_collaboration_drive(
        room_id.clone(),
        "0".into(),
    ));
    assert_eq!(directive["generationToOpen"], json!("1"));
    destroy_handle(&room_id);

    let maximum = u64::MAX.to_string();
    let error = err_json(&v2::editor_v2_apply_input(
        id.clone(),
        format!(r#"{{"version":1,"requestId":"{maximum}","baseDocumentRevision":"0","text":""}}"#),
    ));
    assert_error(&error, "boundary", "CONFIG_INVALID", Some(&maximum));

    for rejected in ["+1", "01", " 1", "1 ", "1e3"] {
        let error = err_json(&v2::editor_v2_apply_input(
            id.clone(),
            format!(
                r#"{{"version":1,"requestId":"{rejected}","baseDocumentRevision":"0","text":"x"}}"#
            ),
        ));
        assert_error(&error, "boundary", "CONFIG_INVALID", None);
    }

    destroy_handle(&id);
}

#[test]
fn ffi_lease_ids_and_deadlines_are_canonical_decimal_strings() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );

    let initial = ok_json(&v2_collab::editor_v2_collaboration_drive(
        id.clone(),
        "0".into(),
    ));
    assert_frozen_directive(&initial);
    assert_eq!(initial["transportState"], "Connecting", "{initial:?}");
    assert_eq!(initial["generationToOpen"], "1", "{initial:?}");
    assert_eq!(initial["nextDeadlineMillis"], Value::Null, "{initial:?}");
    assert_eq!(initial["remoteCommitApplied"], false, "{initial:?}");
    assert_eq!(initial["peersChanged"], false, "{initial:?}");
    assert_eq!(initial["renewedLocal"], false, "{initial:?}");
    assert_eq!(initial["expiredPeers"], json!([]), "{initial:?}");

    let opened = ok_json(&v2_collab::editor_v2_collaboration_socket_open(
        id.clone(),
        "1".into(),
        "0".into(),
    ));
    assert_frozen_directive(&opened);
    assert_eq!(opened["transportState"], "Handshaking", "{opened:?}");
    assert_eq!(opened["generationToOpen"], Value::Null, "{opened:?}");

    let lease = ok_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.clone(),
        "1".into(),
    ));
    assert_eq!(lease.lease_id, "1");
    assert!(
        !lease.frame.is_empty(),
        "Sync Step 1 crosses the FFI as bytes"
    );
    ok_json(&v2_collab::editor_v2_collaboration_ack_outbound(
        id.clone(),
        "1".into(),
        lease.lease_id.clone(),
    ));
    assert_empty_lease_v2(&id, "1");

    let malformed_lease = err_json(&v2_collab::editor_v2_collaboration_ack_outbound(
        id.clone(),
        "1".into(),
        "01".into(),
    ));
    assert_error(&malformed_lease, "boundary", "CONFIG_INVALID", None);
    let malformed_generation = err_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.clone(),
        "01".into(),
    ));
    assert_error(&malformed_generation, "boundary", "CONFIG_INVALID", None);

    let closed = ok_json(&v2_collab::editor_v2_collaboration_socket_close(
        id.clone(),
        "1".into(),
        None,
        None,
        "0".into(),
    ));
    assert_frozen_directive(&closed);
    assert_eq!(closed["transportState"], "Disconnected", "{closed:?}");
    assert_eq!(closed["generationToOpen"], Value::Null, "{closed:?}");
    assert_eq!(closed["nextDeadlineMillis"], "500", "{closed:?}");
    destroy_handle(&id);
}
