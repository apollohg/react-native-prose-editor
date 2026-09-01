//! Golden tests for the complete UniFFI v2 staging surface.
//!
//! Every `editor_v2_*` entry point is exercised through the real proc-macro
//! exports: success shapes with exact nullability, every reachable error
//! code with domain/code/requestId exact, unknown-id lifecycle errors,
//! destroy races refused without partial work, oversize inputs as
//! structured limit errors, binary round-trips (protocol frames, outbound
//! frames, snapshot bytes), the full generation flow with stale refusals,
//! read-only policy (including the ledger-tracked undo/redo coverage), the
//! ledger-tracked input-filter regex caching semantics, and the full drive
//! from local editing to a synchronized room against a raw yrs peer.
//!
//! Binary values are direct bytes end to end; JSON never carries byte
//! arrays. Handles are decimal strings; request ids in errors are decimal
//! strings, omitted when the entry takes no request envelope.

use crate::boundary::ResourceLimits;
use crate::ffi_v2::collaboration as v2_collab;
use crate::ffi_v2::editor as v2;
use crate::ffi_v2::render as v2_render;
use crate::ffi_v2::snapshot as v2_snapshot;
use crate::ffi_v2::types::{FfiError, FfiJsonResult, FfiOutboundLeaseResult};
use crate::tiptap_schema;
use crate::yrs_engine::{
    DocumentScope, DocumentSnapshot, EditingLimits, InitializationMode, OperationError,
    TransactionOrigin, YrsDocumentEngine, YrsEngineConfig,
};
use serde_json::{json, Value};
use std::sync::OnceLock;
use yrs::sync::awareness::Awareness;
use yrs::sync::{Message, SyncMessage};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, GetString, ReadTxn, StateVector, Text, Transact, Update, XmlFragment, XmlOut};

const DOCUMENT_ID: &str = "ffi-v2-room";
const LINEAGE_ID: &str = "ffi-v2-lineage";
const FRAGMENT_NAME: &str = "prosemirror";
const JSON_SEED: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"ffi seed"}]}]}"#;
const SEED_HTML: &str = "<p>html seed</p>";

#[test]
fn omitted_schema_uses_prosemirror_node_names() {
    let schema = v2_render::resolve_create_schema(&None).unwrap();

    assert!(schema.node("bullet_list").is_some());
    assert!(schema.node("bulletList").is_none());
}

fn ok_json(result: &FfiJsonResult) -> Value {
    assert!(
        result.error.is_none(),
        "expected success, got {:?}",
        result.error
    );
    serde_json::from_str(result.value.as_ref().expect("success carries a value"))
        .expect("success value is JSON")
}

fn err_json(result: &FfiJsonResult) -> FfiError {
    assert!(
        result.value.is_none(),
        "expected error, got {:?}",
        result.value
    );
    result.error.clone().expect("error result carries an error")
}

fn err_unit(result: &crate::ffi_v2::types::FfiUnitResult) -> FfiError {
    assert!(
        result.value.is_none(),
        "expected error, got {:?}",
        result.value
    );
    result.error.clone().expect("error result carries an error")
}

fn ok_lease(result: &FfiOutboundLeaseResult) -> crate::ffi_v2::types::FfiOutboundLease {
    assert!(
        !result.empty,
        "a leased value cannot be marked empty: {result:?}"
    );
    assert!(
        result.error.is_none(),
        "expected lease value, got {result:?}"
    );
    result.value.clone().expect("lease result carries a value")
}

fn err_lease(result: &FfiOutboundLeaseResult) -> FfiError {
    assert!(
        result.value.is_none(),
        "expected lease error, got {result:?}"
    );
    assert!(!result.empty, "an error cannot be marked empty: {result:?}");
    result.error.clone().expect("lease result carries an error")
}

#[test]
fn nested_u64_error_details_are_canonical_decimal_strings() {
    let session_error = crate::session::SessionError::from_operation(
        OperationError::revision_mismatch(1, 9_007_199_254_740_993, u64::MAX),
        crate::session::OperationFailureClass::ExistingStableCode,
    );
    let error = FfiError::from(session_error);
    let details: Value = serde_json::from_str(
        error
            .details_json
            .as_deref()
            .expect("revision mismatch must preserve details"),
    )
    .expect("details JSON must be valid");

    assert_eq!(
        details,
        json!({
            "expectedRevision": "9007199254740993",
            "actualRevision": "18446744073709551615",
        }),
    );
}

fn assert_error(error: &FfiError, domain: &str, code: &str, request_id: Option<&str>) {
    assert_eq!(error.domain, domain, "{error:?}");
    assert_eq!(error.code, code, "{error:?}");
    assert_eq!(error.request_id.as_deref(), request_id, "{error:?}");
}

fn ok_unit(result: &crate::ffi_v2::types::FfiUnitResult) {
    assert!(result.error.is_none(), "{:?}", result.error);
    assert_eq!(result.value, Some(true));
}

fn create_handle(config: Value) -> String {
    create_handle_with_state(config, None)
}

fn tiptap_schema_json() -> Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA
        .get_or_init(|| {
            let fixtures: Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/schema-fingerprints.json"
            )))
            .unwrap();
            fixtures["fingerprints"]
                .as_array()
                .unwrap()
                .iter()
                .find(|fixture| fixture["name"] == "Tiptap-compatible camelCase schema")
                .unwrap()["schema"]
                .clone()
        })
        .clone()
}

fn create_handle_with_state(mut config: Value, snapshot_state: Option<Vec<u8>>) -> String {
    if let Some(config) = config.as_object_mut() {
        config.entry("schema").or_insert_with(tiptap_schema_json);
    }
    let result = v2::editor_v2_create(config.to_string(), snapshot_state);
    ok_json(&result)["editorId"]
        .as_str()
        .expect("create returns a decimal-string handle")
        .to_string()
}

fn destroy_handle(id: &str) {
    ok_unit(&v2::editor_v2_destroy(id.to_string()));
}

fn state_of(id: &str) -> Value {
    ok_json(&v2::editor_v2_get_state(id.to_string()))
}

fn revision_of(id: &str) -> u64 {
    state_of(id)["documentRevision"]
        .as_str()
        .expect("state carries a canonical decimal-string documentRevision")
        .parse()
        .expect("document revision decimal string parses as u64")
}

fn document_json_of(id: &str) -> Value {
    ok_json(&v2::editor_v2_get_document_json(id.to_string()))
}

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
fn terminal_custom_atom_gap_accepts_text_in_one_transaction() {
    let id = create_handle(terminal_custom_atom_config());

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(2, revision_of(&id), "x"),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "counterCard", "attrs": { "count": 7 } },
                { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] },
            ],
        })
    );
    assert_eq!(
        ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(3))),
        json!({ "changed": true })
    );
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_gap_accepts_return_in_one_transaction() {
    let id = create_handle(terminal_custom_atom_config());

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "splitBlock" })),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [
                { "type": "counterCard", "attrs": { "count": 7 } },
                { "type": "paragraph" },
            ],
        })
    );
    assert_eq!(
        ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(3))),
        json!({ "changed": true })
    );
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn nested_terminal_void_gap_accepts_text_inside_its_container() {
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

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [{
                "type": "blockquote",
                "content": [
                    { "type": "paragraph", "content": [{ "type": "text", "text": "caption" }] },
                    { "type": "image", "attrs": { "src": "https://example.com/a.png" } },
                    { "type": "paragraph", "content": [{ "type": "text", "text": "x" }] }
                ]
            }]
        })
    );
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_gap_backspace_deletes_atom_in_one_transaction() {
    let id = create_handle(terminal_custom_atom_config());

    ok_json(&v2::editor_v2_set_selection(
        id.clone(),
        selection_envelope_with_affinity(1, revision_of(&id), 1, 1, "before"),
    ));
    let outcome = ok_json(&v2::editor_v2_apply_command(
        id.clone(),
        command_envelope(2, revision_of(&id), json!({ "type": "deleteBackward" })),
    ));

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(
        document_json_of(&id),
        json!({
            "type": "doc",
            "content": [{ "type": "paragraph" }],
        })
    );
    assert_eq!(
        ok_json(&v2::editor_v2_undo(id.clone(), history_envelope(3))),
        json!({ "changed": true })
    );
    assert_eq!(
        document_json_of(&id),
        terminal_custom_atom_config()["initialization"]["json"]
    );
    destroy_handle(&id);
}

#[test]
fn terminal_custom_atom_backspace_leaves_optional_root_empty() {
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

    assert_eq!(outcome["type"], "transaction");
    assert_eq!(outcome["changed"], true);
    assert_eq!(document_json_of(&id), json!({ "type": "doc" }));
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

fn selection_envelope(request_id: u64, base_revision: u64, anchor: u32, head: u32) -> String {
    selection_envelope_with_affinity(request_id, base_revision, anchor, head, "after")
}

fn selection_envelope_with_affinity(
    request_id: u64,
    base_revision: u64,
    anchor: u32,
    head: u32,
    affinity: &str,
) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": base_revision.to_string(),
        "selection": {
            "type": "text",
            "anchor": { "offset": anchor, "kind": "scalar", "affinity": affinity },
            "head": { "offset": head, "kind": "scalar", "affinity": affinity },
        },
    })
    .to_string()
}

fn replace_envelope(request_id: u64, base_revision: u64, json_doc: &str, history: &str) -> String {
    json!({
        "version": 1,
        "requestId": request_id.to_string(),
        "baseDocumentRevision": base_revision.to_string(),
        "setJson": serde_json::from_str::<Value>(json_doc).unwrap(),
        "history": history,
    })
    .to_string()
}

fn history_envelope(request_id: u64) -> String {
    json!({ "version": 1, "requestId": request_id.to_string() }).to_string()
}

// Room/snapshot fixtures and the raw-peer idiom (mirrors the protocol suite)

fn snapshot_source() -> DocumentSnapshot {
    let mut source = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: FRAGMENT_NAME.into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: DOCUMENT_ID.into(),
            lineage_id: LINEAGE_ID.into(),
        }),
    })
    .unwrap();
    source
        .import_json(JSON_SEED, TransactionOrigin::DocumentImport)
        .unwrap();
    source.export_snapshot().unwrap()
}

fn snapshot_metadata_json(snapshot: &DocumentSnapshot) -> Value {
    json!({
        "formatVersion": snapshot.format_version,
        "documentId": snapshot.document_id,
        "lineageId": snapshot.lineage_id,
        "fragmentName": snapshot.fragment_name,
        "schemaFingerprint": snapshot.schema_fingerprint,
    })
}

fn room_config(snapshot: Option<&DocumentSnapshot>) -> Value {
    let mut initialization = json!({
        "type": "room",
        "documentId": DOCUMENT_ID,
        "lineageId": LINEAGE_ID,
    });
    if let Some(snapshot) = snapshot {
        initialization["snapshot"] = snapshot_metadata_json(snapshot);
    }
    json!({ "initialization": initialization })
}

struct RawPeer {
    doc: Doc,
}

impl RawPeer {
    fn from_snapshot(snapshot: &DocumentSnapshot) -> Self {
        let peer = Self { doc: Doc::new() };
        peer.apply(&snapshot.encoded_state);
        peer
    }

    fn apply(&self, update: &[u8]) {
        self.doc
            .transact_mut()
            .apply_update(Update::decode_v1(update).unwrap())
            .unwrap();
    }

    fn state_vector_bytes(&self) -> Vec<u8> {
        self.doc.transact().state_vector().encode_v1()
    }

    fn diff_for(&self, remote_state_vector: &[u8]) -> Vec<u8> {
        self.doc
            .transact()
            .encode_state_as_update_v1(&StateVector::decode_v1(remote_state_vector).unwrap())
    }

    fn fragment_string(&self) -> String {
        let txn = self.doc.transact();
        txn.get_xml_fragment(FRAGMENT_NAME)
            .expect("peer must hold the configured fragment")
            .get_string(&txn)
    }

    fn push_text(&self, text: &str) {
        let mut txn = self.doc.transact_mut();
        let fragment = txn
            .get_xml_fragment(FRAGMENT_NAME)
            .expect("peer must hold the configured fragment");
        let Some(XmlOut::Element(paragraph)) = fragment.get(&txn, 0) else {
            panic!("seed content must start with a paragraph element");
        };
        let Some(XmlOut::Text(content)) = paragraph.get(&txn, 0) else {
            panic!("seed paragraph must start with a text node");
        };
        content.push(&mut txn, text);
    }
}

fn sync_frame(message: SyncMessage) -> Vec<u8> {
    Message::Sync(message).encode_v1()
}

fn step1_frame(state_vector: &[u8]) -> Vec<u8> {
    sync_frame(SyncMessage::SyncStep1(
        StateVector::decode_v1(state_vector).unwrap(),
    ))
}

fn step2_frame(update: Vec<u8>) -> Vec<u8> {
    sync_frame(SyncMessage::SyncStep2(update))
}

/// The state vector inside a framed Sync Step 1 message.
fn step1_state_vector(step1: &[u8]) -> StateVector {
    match Message::decode_v1(step1).expect("step1 frame must decode") {
        Message::Sync(SyncMessage::SyncStep1(state_vector)) => state_vector,
        other => panic!("expected a Sync Step 1 frame, got {other:?}"),
    }
}

fn assert_frozen_directive(directive: &Value) {
    let object = directive
        .as_object()
        .expect("transport directives must be JSON objects");
    let fields = object.keys().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(
        fields,
        vec![
            "expiredPeers",
            "generationToOpen",
            "nextDeadlineMillis",
            "peersChanged",
            "remoteCommitApplied",
            "renewedLocal",
            "transportState",
        ],
        "directive field set is frozen: {directive:?}"
    );
    assert!(directive["transportState"].is_string(), "{directive:?}");
    assert!(
        directive["generationToOpen"].is_null() || directive["generationToOpen"].is_string(),
        "{directive:?}"
    );
    assert!(
        directive["nextDeadlineMillis"].is_null() || directive["nextDeadlineMillis"].is_string(),
        "{directive:?}"
    );
    assert!(
        directive["remoteCommitApplied"].is_boolean(),
        "{directive:?}"
    );
    assert!(directive["peersChanged"].is_boolean(), "{directive:?}");
    assert!(directive["renewedLocal"].is_boolean(), "{directive:?}");
    assert!(
        directive["expiredPeers"]
            .as_array()
            .is_some_and(|peers| peers.iter().all(Value::is_string)),
        "{directive:?}"
    );
}

fn drive_v2(id: &str, now_millis: u64) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_drive(
        id.to_string(),
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn open_v2(id: &str, generation: &str, now_millis: u64) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_socket_open(
        id.to_string(),
        generation.to_string(),
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn receive_v2(id: &str, generation: &str, message: Vec<u8>, now_millis: u64) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_receive(
        id.to_string(),
        generation.to_string(),
        message,
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn close_v2(
    id: &str,
    generation: &str,
    code: Option<u32>,
    reason: Option<String>,
    now_millis: u64,
) -> Value {
    let directive = ok_json(&v2_collab::editor_v2_collaboration_socket_close(
        id.to_string(),
        generation.to_string(),
        code,
        reason,
        now_millis.to_string(),
    ));
    assert_frozen_directive(&directive);
    directive
}

fn lease_v2(id: &str, generation: &str) -> crate::ffi_v2::types::FfiOutboundLease {
    ok_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.to_string(),
        generation.to_string(),
    ))
}

fn ack_v2(id: &str, generation: &str, lease_id: String) {
    ok_json(&v2_collab::editor_v2_collaboration_ack_outbound(
        id.to_string(),
        generation.to_string(),
        lease_id,
    ));
}

fn assert_empty_lease_v2(id: &str, generation: &str) {
    let result =
        v2_collab::editor_v2_collaboration_lease_outbound(id.to_string(), generation.to_string());
    assert!(result.value.is_none(), "{result:?}");
    assert!(result.empty, "{result:?}");
    assert!(result.error.is_none(), "{result:?}");
}

/// Drive a RoomReady editor to Synchronized through the v2 boundary: open,
/// answer the owed Step 1 with a raw peer's Step 2, and return the live
/// generation.
fn synchronize_v2(id: &str, server: &RawPeer) -> String {
    let directive = drive_v2(id, 0);
    let generation = directive["generationToOpen"]
        .as_str()
        .expect("initial drive returns a generation");
    let opened = open_v2(id, generation, 0);
    assert_eq!(opened["transportState"], "Handshaking", "{opened:?}");
    let step1 = lease_v2(id, generation);
    let step2 = server.diff_for(&step1_state_vector(&step1.frame).encode_v1());
    ack_v2(id, generation, step1.lease_id);
    let outcome = receive_v2(id, generation, step2_frame(step2), 0);
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    assert_eq!(state_of(id)["transportState"], "Synchronized");
    generation.to_string()
}

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

#[test]
fn create_room_attaches_the_collaboration_runtime() {
    // Room sessions own the runtime (bounded outbox) from creation so
    // offline edits queue from the first keystroke; local sessions do not.
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let pending = crate::native_bridge_test_support::outbox_pending(id.parse().unwrap())
        .expect("test seam must read the session");
    assert_eq!(pending, Some((0, 0)), "room sessions attach the runtime");
    destroy_handle(&id);

    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let pending = crate::native_bridge_test_support::outbox_pending(local.parse().unwrap())
        .expect("test seam must read the session");
    assert_eq!(pending, None, "local sessions own no outbox");
    destroy_handle(&local);
}

#[test]
fn collaboration_generation_flow_with_stale_and_disposition_refusals() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);

    // Local-only editors remain detached; drive never creates a generation.
    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let detached = drive_v2(&local, 0);
    assert_eq!(detached["transportState"], "Detached", "{detached:?}");
    assert_eq!(detached["generationToOpen"], Value::Null, "{detached:?}");
    assert_eq!(detached["nextDeadlineMillis"], Value::Null, "{detached:?}");
    destroy_handle(&local);

    // Drive issues generation 1. A subsequent drive is observational only;
    // socket open queues Sync Step 1 for the retained lease path.
    let generation = drive_v2(&id, 0)["generationToOpen"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(generation, "1", "first issued generation");
    let stale_generation = (generation.parse::<u64>().unwrap() + 100).to_string();
    let waiting = drive_v2(&id, 0);
    assert_eq!(waiting["transportState"], "Connecting", "{waiting:?}");
    assert_eq!(waiting["generationToOpen"], Value::Null, "{waiting:?}");

    let error = err_json(&v2_collab::editor_v2_collaboration_socket_open(
        id.clone(),
        stale_generation.clone(),
        "0".into(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);
    let opened = open_v2(&id, &generation, 0);
    assert_eq!(opened["transportState"], "Handshaking", "{opened:?}");
    let step1 = lease_v2(&id, &generation);
    let our_sv = step1_state_vector(&step1.frame);
    assert!(!step1.frame.is_empty(), "step 1 bytes ride through a lease");
    ack_v2(&id, &generation, step1.lease_id);

    // receive on a stale generation refuses before any decode work.
    let error = err_json(&v2_collab::editor_v2_collaboration_receive(
        id.clone(),
        stale_generation.clone(),
        step2_frame(server.diff_for(&our_sv.encode_v1())),
        "0".into(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);

    // The real Step 2 completes the handshake.
    let outcome = receive_v2(
        &id,
        &generation,
        step2_frame(server.diff_for(&our_sv.encode_v1())),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    assert_eq!(outcome["remoteCommitApplied"], false, "{outcome:?}");

    // The peer's Step 1 earns a retained Step 2 reply. ACK consumes exactly
    // that lease; a subsequent lease observes the explicit empty variant.
    let outcome = receive_v2(
        &id,
        &generation,
        step1_frame(&server.state_vector_bytes()),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("outbound frame must decode") {
        Message::Sync(SyncMessage::SyncStep2(update)) => server.apply(&update),
        other => panic!("expected a Sync Step 2 reply frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);
    assert_empty_lease_v2(&id, &generation);

    // Stale generation refuses the lease; the close retires the generation.
    let error = err_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.clone(),
        stale_generation.clone(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);
    let outcome = close_v2(&id, &generation, None, None, 0);
    assert_eq!(outcome["transportState"], "Disconnected", "{outcome:?}");
    assert_eq!(outcome["nextDeadlineMillis"], "500", "{outcome:?}");
    let error = err_lease(&v2_collab::editor_v2_collaboration_lease_outbound(
        id.clone(),
        generation.clone(),
    ));
    assert_error(&error, "transport", "TRANSPORT_STALE_GENERATION", None);

    // Reconnect issues the next monotonic generation; a policy-violation
    // close code parks the transport Incompatible and drive remains inert.
    let generation = drive_v2(&id, 500)["generationToOpen"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(generation, "2", "generations stay monotonic");
    let outcome = close_v2(
        &id,
        &generation,
        Some(1008),
        Some("policy violation".into()),
        500,
    );
    assert_eq!(outcome["transportState"], "Incompatible", "{outcome:?}");
    assert_eq!(outcome["nextDeadlineMillis"], Value::Null, "{outcome:?}");
    let inert = drive_v2(&id, 500);
    assert_eq!(inert["transportState"], "Incompatible", "{inert:?}");
    assert_eq!(inert["generationToOpen"], Value::Null, "{inert:?}");
    destroy_handle(&id);
}

#[test]
fn typed_awareness_intent_ffi_and_collaboration_binary_round_trip() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);

    // A local edit rides one retained outbound lease; the raw peer applies
    // it and ACK consumes that exact frame.
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(901, revision_of(&id), " outbound"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("update frame must decode") {
        Message::Sync(SyncMessage::Update(update)) => server.apply(&update),
        other => panic!("expected a document update frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);
    assert!(
        server.fragment_string().contains("ffi seed")
            && server.fragment_string().contains(" outbound"),
        "{:?}",
        server.fragment_string(),
    );

    // Awareness takes exactly a typed intent and Rust publishes the
    // application state/focus beside its engine-owned cursor.
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({
            "state": { "name": "ffi peer" },
            "focused": true,
            "selection": { "type": "text", "anchor": 4, "head": 6 },
        })
        .to_string(),
    ));
    let peers = ok_json(&v2_collab::editor_v2_collaboration_peers(id.clone()));
    let local = peers["peers"]
        .as_array()
        .expect("peers array")
        .iter()
        .find(|peer| peer["isLocal"] == true)
        .expect("a local peer");
    assert_eq!(local["state"]["name"], json!("ffi peer"), "{local:?}");
    assert!(local["state"].get("state").is_none(), "{local:?}");
    assert_eq!(local["state"]["focused"], true, "{local:?}");
    assert_eq!(
        local["cursor"],
        json!({ "anchor": 4, "head": 6 }),
        "{local:?}"
    );
    assert!(
        local["clientId"]
            .as_str()
            .expect("clientId is a decimal string")
            .parse::<u64>()
            .is_ok(),
        "{local:?}",
    );
    let lease = lease_v2(&id, &generation);
    let mut raw_awareness = Awareness::new(Doc::new());
    match Message::decode_v1(&lease.frame).expect("awareness frame must decode") {
        Message::Awareness(update) => raw_awareness.apply_update(update).unwrap(),
        other => panic!("expected an awareness frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);
    assert_empty_lease_v2(&id, &generation);

    // An explicit null selection removes the engine-owned cursor while
    // retaining the application state and focus flag. (Omitting the key
    // instead would retain the cursor — see the awareness suite.)
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "cursorless" }, "focused": false, "selection": Value::Null })
            .to_string(),
    ));
    let peers = ok_json(&v2_collab::editor_v2_collaboration_peers(id.clone()));
    let local = peers["peers"]
        .as_array()
        .expect("peers array")
        .iter()
        .find(|peer| peer["isLocal"] == true)
        .expect("a local peer");
    assert_eq!(local["state"]["name"], json!("cursorless"));
    assert!(local["state"].get("state").is_none(), "{local:?}");
    assert_eq!(local["state"]["focused"], false);
    assert_eq!(local["cursor"], Value::Null, "{local:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("cursorless awareness frame must decode") {
        Message::Awareness(update) => raw_awareness.apply_update(update).unwrap(),
        other => panic!("expected cursorless awareness frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);

    // "null" withdraws the desired state with a tombstone broadcast.
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        "null".into(),
    ));
    let peers = ok_json(&v2_collab::editor_v2_collaboration_peers(id.clone()));
    assert_eq!(peers["peers"], json!([]), "{peers:?}");
    let lease = lease_v2(&id, &generation);
    match Message::decode_v1(&lease.frame).expect("tombstone frame must decode") {
        Message::Awareness(update) => raw_awareness.apply_update(update).unwrap(),
        other => panic!("expected an awareness tombstone frame, got {other:?}"),
    }
    ack_v2(&id, &generation, lease.lease_id);

    // Malformed awareness state is a structured error, never a panic.
    let result = v2_collab::editor_v2_collaboration_set_awareness(id.clone(), "{not json".into());
    assert!(result.value.is_none(), "{result:?}");
    assert_error(
        &result.error.expect("error"),
        "boundary",
        "AWARENESS_STATE_INVALID",
        None,
    );

    // Sessions without an attached runtime refuse runtime-shaped calls.
    let local = create_handle(json!({ "initialization": { "type": "localEmpty" } }));
    let error = err_json(&v2_collab::editor_v2_collaboration_peers(local.clone()));
    assert_error(&error, "boundary", "CONFIG_INVALID", None);
    destroy_handle(&local);
    destroy_handle(&id);
}

#[test]
fn awareness_selection_patch_ffi_has_a_closed_result_and_input_shape() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "ffi peer" }, "focused": true }).to_string(),
    ));
    let lease = lease_v2(&id, &generation);
    ack_v2(&id, &generation, lease.lease_id);

    let outcome = ok_json(&v2_collab::editor_v2_collaboration_set_awareness_selection(
        id.clone(),
        json!({ "type": "text", "anchor": 4, "head": 6 }).to_string(),
    ));
    let keys: std::collections::BTreeSet<&str> = outcome
        .as_object()
        .expect("selection patch outcome is an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["outboundChanged"]
            .into_iter()
            .collect::<std::collections::BTreeSet<&str>>(),
        "selection patch result has exactly one key: {keys:?}",
    );
    assert_eq!(outcome, json!({ "outboundChanged": true }));
    let lease = lease_v2(&id, &generation);
    ack_v2(&id, &generation, lease.lease_id);

    for invalid in [
        "{not json".to_string(),
        json!({ "type": "text", "anchor": 4, "head": 6, "unknown": true }).to_string(),
    ] {
        let error = err_json(&v2_collab::editor_v2_collaboration_set_awareness_selection(
            id.clone(),
            invalid,
        ));
        assert_error(&error, "boundary", "AWARENESS_STATE_INVALID", None);
    }
    destroy_handle(&id);
}

#[test]
fn awareness_review_fix_raw_publication_is_test_only() {
    let session = include_str!("../session.rs");
    let runtime = include_str!("../collaboration_runtime/awareness.rs");
    let document_api = include_str!("../document_api.rs");

    assert!(
        !session.contains("pub(crate) fn set_desired_awareness("),
        "EditorSession must not expose a production raw awareness setter",
    );
    assert!(
        session.contains("#[cfg(test)]\n    pub(crate) fn set_desired_awareness_for_test("),
        "legacy raw-state fixtures require a cfg(test)-gated session seam",
    );
    assert!(
        !runtime.contains("pub(crate) fn set_desired_awareness("),
        "the runtime must not retain a generic raw publication method",
    );
    assert!(
        runtime.contains("#[cfg(test)]\n    pub(crate) fn set_desired_awareness_for_test("),
        "the runtime raw parser must be cfg(test)-gated",
    );
    assert!(
        !document_api.contains("pub fn set_desired_awareness("),
        "the document facade must not expose a generic raw awareness setter",
    );
    assert!(
        document_api.contains("pub fn set_desired_awareness_for_test("),
        "raw document-facade publication must be explicitly test-only",
    );
    assert!(
        document_api.contains("#[cfg(test)]\npub mod session_initialization_test_support"),
        "the document facade raw helper must remain inside test-only support",
    );
}

#[test]
fn ffi_drive_reports_local_renewal_as_peer_change() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));

    for malformed in ["+1", "01", " 1", "1 ", "1e3"] {
        let error = err_json(&v2_collab::editor_v2_collaboration_drive(
            id.clone(),
            malformed.into(),
        ));
        assert_error(&error, "boundary", "CONFIG_INVALID", None);
    }

    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "tick local" }, "focused": false }).to_string(),
    ));
    let before = drive_v2(&id, 14_999);
    assert_eq!(
        before,
        json!({
            "transportState": "Synchronized",
            "generationToOpen": null,
            "nextDeadlineMillis": "15000",
            "remoteCommitApplied": false,
            "renewedLocal": false,
            "expiredPeers": [],
            "peersChanged": false,
        }),
        "{before:?}"
    );

    let at = drive_v2(&id, 15_000);
    assert_eq!(
        at,
        json!({
            "transportState": "Synchronized",
            "generationToOpen": null,
            "nextDeadlineMillis": "30000",
            "remoteCommitApplied": false,
            "renewedLocal": true,
            "expiredPeers": [],
            "peersChanged": true,
        }),
        "{at:?}"
    );
    let lease = lease_v2(&id, &generation);
    assert!(
        !lease.frame.is_empty(),
        "renewal enqueues an outbound awareness frame"
    );
    ack_v2(&id, &generation, lease.lease_id);
    destroy_handle(&id);
}

#[test]
fn collaboration_drive_rejects_regressing_time_without_corrupting_peer_expiry() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));

    drive_v2(&id, 10_000);
    let error = err_json(&v2_collab::editor_v2_collaboration_drive(
        id.clone(),
        "9999".into(),
    ));
    assert_error(&error, "transport", "AWARENESS_TIME_REGRESSION", None);
    assert_eq!(
        serde_json::from_str::<Value>(
            error
                .details_json
                .as_deref()
                .expect("regressing time errors carry clock context"),
        )
        .expect("error details are JSON"),
        json!({ "nowMillis": "9999", "lastNowMillis": "10000" }),
    );

    // The remote update must retain the last accepted drive time (10s), not
    // the rejected 9_999ms input, so expiry remains scheduled for 40s.
    let clients = [(
        yrs::ClientID::new(9_001),
        yrs::sync::awareness::AwarenessUpdateEntry {
            clock: 1,
            json: json!({ "name": "monotonic peer" }).to_string().into(),
        },
    )]
    .into_iter()
    .collect();
    let receive = receive_v2(
        &id,
        &generation,
        Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1(),
        10_000,
    );
    assert_eq!(receive["transportState"], "Synchronized", "{receive:?}");

    let before = drive_v2(&id, 39_999);
    assert_eq!(before["expiredPeers"], json!([]), "{before:?}");
    assert_eq!(before["nextDeadlineMillis"], json!("40000"), "{before:?}");

    let at = drive_v2(&id, 40_000);
    assert_eq!(at["expiredPeers"], json!(["9001"]), "{at:?}");
    destroy_handle(&id);
}

#[test]
fn collaboration_drive_expires_remote_peers_with_decimal_ids() {
    // Yrs client IDs occupy the same 53-bit integer domain as Yjs numbers.
    // Use its maximum valid value so the FFI must preserve the exact decimal
    // spelling without constructing an out-of-domain ID that aliases in
    // release builds.
    const MAX_YRS_CLIENT_ID: u64 = 9_007_199_254_740_991;
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));
    let clients = [(
        yrs::ClientID::new(MAX_YRS_CLIENT_ID),
        yrs::sync::awareness::AwarenessUpdateEntry {
            clock: 1,
            json: json!({ "name": "expiring remote" }).to_string().into(),
        },
    )]
    .into_iter()
    .collect();
    let receive = receive_v2(
        &id,
        &generation,
        Message::Awareness(yrs::sync::awareness::AwarenessUpdate { clients }).encode_v1(),
        0,
    );
    assert_eq!(receive["transportState"], "Synchronized", "{receive:?}");

    let before = drive_v2(&id, 29_999);
    assert_eq!(before["expiredPeers"], json!([]), "{before:?}");
    assert_eq!(before["peersChanged"], false, "{before:?}");

    let at = drive_v2(&id, 30_000);
    assert_eq!(at["nextDeadlineMillis"], Value::Null, "{at:?}");
    assert_eq!(
        at["expiredPeers"],
        json!([MAX_YRS_CLIENT_ID.to_string()]),
        "{at:?}"
    );
    assert_eq!(at["peersChanged"], true, "{at:?}");
    assert_eq!(at["remoteCommitApplied"], false, "{at:?}");
    destroy_handle(&id);
}

#[test]
fn collaboration_task8_detach_and_reattach_are_idempotent_after_incompatible() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let first_generation = synchronize_v2(&id, &RawPeer::from_snapshot(&snapshot));
    let close = close_v2(
        &id,
        &first_generation,
        Some(1008),
        Some("policy violation".into()),
        0,
    );
    assert_eq!(close["transportState"], "Incompatible", "{close:?}");
    let inert = drive_v2(&id, 0);
    assert_eq!(inert["transportState"], "Incompatible", "{inert:?}");
    assert_eq!(inert["generationToOpen"], Value::Null, "{inert:?}");

    ok_unit(&v2_collab::editor_v2_collaboration_detach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Detached");
    ok_unit(&v2_collab::editor_v2_collaboration_detach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Detached");
    ok_unit(&v2_collab::editor_v2_collaboration_reattach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Disconnected");
    ok_unit(&v2_collab::editor_v2_collaboration_reattach(id.clone()));
    assert_eq!(state_of(&id)["transportState"], "Disconnected");

    let next = drive_v2(&id, 0);
    assert_eq!(next["generationToOpen"], "2", "{next:?}");
    destroy_handle(&id);
}

#[test]
fn leased_outbound_drains_protocol_replies_before_document_updates() {
    let snapshot = snapshot_source();
    let id = create_handle_with_state(
        room_config(Some(&snapshot)),
        Some(snapshot.encoded_state.clone()),
    );
    let server = RawPeer::from_snapshot(&snapshot);
    let generation = synchronize_v2(&id, &server);
    assert_eq!(
        crate::session_initialization_test_support::pending_protocol_replies(id.parse().unwrap())
            .unwrap(),
        Some((0, 0)),
        "a freshly synchronized session has no protocol residue",
    );
    assert_eq!(
        crate::native_bridge_test_support::outbox_pending(id.parse().unwrap()).unwrap(),
        Some((0, 0)),
    );

    // Fill BOTH queues on the one live session: an awareness broadcast and
    // a Step 2 reply are transport-scoped protocol frames; the local edit
    // is a pending document update.
    ok_unit(&v2_collab::editor_v2_collaboration_set_awareness(
        id.clone(),
        json!({ "state": { "name": "ordering peer" }, "focused": false }).to_string(),
    ));
    let outcome = receive_v2(
        &id,
        &generation,
        step1_frame(&server.state_vector_bytes()),
        0,
    );
    assert_eq!(outcome["transportState"], "Synchronized", "{outcome:?}");
    let outcome = ok_json(&v2::editor_v2_apply_input(
        id.clone(),
        input_envelope(1101, revision_of(&id), " ordered"),
    ));
    assert_eq!(outcome["changed"], true, "{outcome:?}");

    // Non-vacuity: both queues are provably non-empty at pickup time.
    let (protocol_count, protocol_bytes) =
        crate::session_initialization_test_support::pending_protocol_replies(id.parse().unwrap())
            .unwrap()
            .expect("room sessions own a protocol queue");
    assert_eq!(protocol_count, 2, "awareness broadcast + Step 2 reply");
    assert!(protocol_bytes > 0);
    let (document_count, document_bytes) =
        crate::native_bridge_test_support::outbox_pending(id.parse().unwrap())
            .unwrap()
            .expect("room sessions own a document outbox");
    assert_eq!(document_count, 1, "the local edit is pending");
    assert!(document_bytes > 0);

    // Lease one frame per call: every frame decodes as a standard
    // yrs::sync::Message, every successful lease is ACKed, and every
    // protocol frame precedes every document frame.
    let mut kinds = Vec::new();
    loop {
        let result =
            v2_collab::editor_v2_collaboration_lease_outbound(id.clone(), generation.clone());
        if result.empty {
            assert!(
                result.value.is_none() && result.error.is_none(),
                "{result:?}"
            );
            break;
        }
        let lease = ok_lease(&result);
        let frame = lease.frame;
        let kind = match Message::decode_v1(&frame).expect("outbound frame must decode") {
            Message::Sync(SyncMessage::SyncStep2(update)) => {
                server.apply(&update);
                "protocol"
            }
            Message::Awareness(_) => "protocol",
            Message::Sync(SyncMessage::Update(update)) => {
                server.apply(&update);
                "document"
            }
            other => panic!("unexpected outbound frame: {other:?}"),
        };
        kinds.push(kind);
        ack_v2(&id, &generation, lease.lease_id);
    }
    assert_eq!(
        kinds,
        ["protocol", "protocol", "document"],
        "protocol replies drain before document updates",
    );
    assert!(
        server.fragment_string().contains(" ordered"),
        "the document frame carries the local edit: {:?}",
        server.fragment_string(),
    );

    // Both queues drain to exactly (0, 0).
    assert_eq!(
        crate::session_initialization_test_support::pending_protocol_replies(id.parse().unwrap())
            .unwrap(),
        Some((0, 0)),
    );
    assert_eq!(
        crate::native_bridge_test_support::outbox_pending(id.parse().unwrap()).unwrap(),
        Some((0, 0)),
    );

    destroy_handle(&id);
}

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
