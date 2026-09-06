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

include!("ffi_v2_test/native_intents.rs");

include!("ffi_v2_test/void_boundaries.rs");

include!("ffi_v2_test/session_fixtures.rs");

include!("ffi_v2_test/editor_lifecycle.rs");

include!("ffi_v2_test/collaboration.rs");

include!("ffi_v2_test/snapshots.rs");

include!("ffi_v2_test/render_state.rs");

include!("ffi_v2_test/mark_import.rs");
