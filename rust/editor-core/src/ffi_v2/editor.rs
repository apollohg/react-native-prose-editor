//! UniFFI v2 editor lifecycle, state, and mutation entry points.
//!
//! Every entry follows the frozen contract: decimal-string handle parsing,
//! registry lookup, `slot.with_alive` under-lock recheck, and a typed
//! result record carrying exactly one of value or error — never a panic,
//! never a stringly error. Internal seams still require a `u64` request id,
//! while FFI error correlation retains request-id presence separately so an
//! admitted external `0` remains distinguishable from no request envelope.
//!
//! `create` reuses the session config format: room initialization carries
//! `documentId`/`lineageId` plus optional snapshot *metadata* in the JSON;
//! the snapshot's encoded state rides as direct bytes in the separate
//! `snapshot_state` parameter (binary values are never JSON number arrays).
//! Room-bound sessions own a collaboration runtime from creation, so
//! offline edits queue against the bounded outbox from the first keystroke.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use serde_json::value::RawValue;

use crate::boundary::{
    deserialize_non_null_option, parse_json_value_stack_safe, BoundaryError, BoundedInput,
    InputKind, ResourceLimitOverrides, ResourceLimits, HARD_MAX_DOCUMENT_DEPTH,
    HARD_MAX_INPUT_BYTES,
};
use crate::document_api::DocumentApiFacade;
use crate::native_transaction_bridge::{
    HistoryModeEnvelope, NativeBridgeOutcome, NativeTransactionBridge,
};
use crate::registry;
use crate::session::{
    CollaborationLimitOverrides, CollaborationLimits, EditorInitialization, EditorSession,
    EditorSessionConfig, ErrorDomain, InitialContent, SessionError,
};
use crate::viewer::FfiViewerSourceKind;
use crate::yrs_engine::{
    DocumentScope, EditingLimitOverrides, EditingLimits, EngineRenderState, ReplacementHistory,
};

use super::snapshot::SnapshotMetadataEnvelope;
use super::types::{
    decimal_u64, deserialize_canonical_u64, parse_canonical_u64, recover_request_id, FfiError,
    FfiJsonResult, FfiUnitResult,
};

/// Session operations require a concrete `u64` even when their FFI entry has
/// no request envelope. This value is internal-only: FFI error conversion
/// explicitly clears its correlation field for envelope-less entries.
pub(crate) const INTERNAL_UNCORRELATED_REQUEST_ID: u64 = 0;

/// One supported version for every v2 request envelope.
const V2_ENVELOPE_VERSION: u32 = 1;

/// Non-payload create metadata is intentionally tiny. Schema and local
/// document/HTML values are borrowed as `RawValue`s and excluded from this
/// retained-envelope budget until configured limits have resolved.
const CREATE_ENVELOPE_MAX_BYTES: usize = 64 * 1024;

/// A create can carry one schema plus one local document or HTML payload.
/// Schema/document JSON is bounded in its encoded form. HTML is bounded after
/// decoding and a valid JSON string can use at most six wire bytes per decoded
/// byte (`\u00XX`), plus its quotes. This finite pre-parse ceiling therefore
/// admits every payload allowed by the authoritative decoded limits without
/// permitting an unbounded syntax scan.
const fn create_wire_max_bytes() -> usize {
    let payload_bytes = match HARD_MAX_INPUT_BYTES.checked_mul(7) {
        Some(bytes) => bytes,
        None => panic!("create payload wire ceiling overflow"),
    };
    let with_envelope = match payload_bytes.checked_add(CREATE_ENVELOPE_MAX_BYTES) {
        Some(bytes) => bytes,
        None => panic!("create envelope wire ceiling overflow"),
    };
    match with_envelope.checked_add(2) {
        Some(bytes) => bytes,
        None => panic!("create string quote wire ceiling overflow"),
    }
}

const CREATE_WIRE_MAX_BYTES: usize = create_wire_max_bytes();

/// A document node can add both an object and its content array, and metadata
/// at the deepest node can independently consume the document-depth ceiling.
/// Fixed slack covers the create/initialization/mark wrappers.
const CREATE_SCAN_MAX_DEPTH: usize = match HARD_MAX_DOCUMENT_DEPTH.checked_mul(3) {
    Some(depth) => match depth.checked_add(16) {
        Some(depth) => depth,
        None => panic!("create scanner depth ceiling overflow"),
    },
    None => panic!("create scanner depth ceiling overflow"),
};

include!("editor/create_scanner.rs");

// Shared session access (used by the collaboration and snapshot modules too)

/// Decimal-string handles at the boundary; malformed handles are structured
/// boundary errors, never panics.
fn parse_editor_id(handle: &str) -> Result<u64, FfiError> {
    parse_canonical_u64(handle).ok_or_else(|| {
        FfiError::new(
            ErrorDomain::Boundary,
            "CONFIG_INVALID",
            format!("malformed editor handle: {handle:?}"),
        )
    })
}

fn unknown_editor_error() -> FfiError {
    FfiError::new(
        ErrorDomain::Lifecycle,
        "ENGINE_DESTROYED",
        "editor session is not registered",
    )
}

/// Convert a session error without changing its correlation. This keeps an
/// admitted external request id, including canonical `0`, in structured FFI
/// errors.
pub(crate) fn ffi_error(error: SessionError) -> FfiError {
    FfiError::from(error)
}

fn ffi_error_without_request(mut error: SessionError) -> FfiError {
    error.request_id = None;
    ffi_error(error)
}

/// Registry lookup -> `slot.with_alive` under-lock recheck -> typed error on
/// failure. These entries take no request envelope, so any internal request
/// id is intentionally omitted from their FFI errors.
pub(crate) fn with_editor<T>(
    handle: &str,
    operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
) -> Result<T, FfiError> {
    let id = parse_editor_id(handle)?;
    let slot = registry::get_session(id).ok_or_else(unknown_editor_error)?;
    crate::boundary::with_document_stack(|| {
        slot.with_alive(operation)
            .and_then(|value| value)
            .map_err(ffi_error_without_request)
    })
}

/// As [`with_editor`], but preserves correlation from a request-envelope
/// entry. Parse failures before request-id admission naturally remain absent;
/// failures after admission retain the canonical decimal id, including `0`.
fn with_editor_request_envelope<T>(
    handle: &str,
    operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
) -> Result<T, FfiError> {
    let id = parse_editor_id(handle)?;
    let slot = registry::get_session(id).ok_or_else(unknown_editor_error)?;
    crate::boundary::with_document_stack(|| {
        slot.with_alive(operation)
            .and_then(|value| value)
            .map_err(ffi_error)
    })
}

pub(crate) fn json_result(result: Result<String, FfiError>) -> FfiJsonResult {
    match result {
        Ok(value) => FfiJsonResult::ok(value),
        Err(error) => FfiJsonResult::err(error),
    }
}

pub(crate) fn unit_result(result: Result<(), FfiError>) -> FfiUnitResult {
    match result {
        Ok(()) => FfiUnitResult::ok(),
        Err(error) => FfiUnitResult::err(error),
    }
}

fn parse_request_envelope<'a, T: serde::Deserialize<'a>>(
    session: &EditorSession,
    json: &'a str,
) -> Result<T, SessionError> {
    let input = BoundedInput::new(json, InputKind::Config, session.engine.resource_limits())?;
    serde_json::from_str(input.as_str()).map_err(|error| {
        config_invalid(
            recover_request_id(input.as_str()),
            BoundaryError::parse("CONFIG_INVALID", error).message,
        )
    })
}

fn admit_version(version: u32, request_id: u64) -> Result<(), SessionError> {
    if version != V2_ENVELOPE_VERSION {
        return Err(config_invalid(
            Some(request_id),
            format!(
                "unsupported v2 envelope version {version}; supported version is {V2_ENVELOPE_VERSION}"
            ),
        ));
    }
    Ok(())
}

fn config_invalid(request_id: Option<u64>, message: impl Into<String>) -> SessionError {
    let mut error = SessionError::new(ErrorDomain::Boundary, "CONFIG_INVALID", message);
    error.request_id = request_id;
    error
}

/// Data-only mirror of the replacement history policy (shared wire shape
/// with the bridge local-API envelope).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplaceDocumentEnvelope<'a> {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
    #[serde(default, borrow, deserialize_with = "deserialize_non_null_raw_value")]
    set_json: Option<&'a RawValue>,
    set_html: Option<String>,
    history: HistoryModeEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryRequestEnvelope {
    version: u32,
    #[serde(deserialize_with = "deserialize_canonical_u64")]
    request_id: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateEnvelope<'a> {
    #[serde(default, borrow, deserialize_with = "deserialize_non_null_raw_value")]
    schema: Option<&'a RawValue>,
    #[serde(default, borrow, deserialize_with = "deserialize_non_null_raw_value")]
    fragment_name: Option<&'a RawValue>,
    #[serde(borrow)]
    initialization: &'a RawValue,
    #[serde(default, borrow, deserialize_with = "deserialize_non_null_raw_value")]
    policy: Option<&'a RawValue>,
    #[serde(default, borrow, deserialize_with = "deserialize_non_null_raw_value")]
    limits: Option<&'a RawValue>,
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyEnvelope {
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    max_length: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    read_only: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    input_filter: Option<String>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    allow_base64_images: Option<bool>,
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LimitsEnvelope {
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    resource: Option<ResourceLimitOverrides>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    editing: Option<EditingLimitOverrides>,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    collaboration: Option<CollaborationLimitOverrides>,
}

/// First-pass initialization shape. Payloads remain borrowed and unknown
/// fields remain visible to the retained-envelope byte calculation. A second
/// exact variant parse runs only after all configured limits have resolved.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializationProbe {
    #[serde(rename = "type")]
    kind: InitializationKind,
}

fn deserialize_non_null_raw_value<'de, D>(
    deserializer: D,
) -> Result<Option<&'de RawValue>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <&RawValue as serde::Deserialize>::deserialize(deserializer)?;
    if raw.get() == "null" {
        return Err(serde::de::Error::custom("null is not allowed"));
    }
    Ok(Some(raw))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalEmptyInitialization {
    #[serde(rename = "type")]
    _kind: InitializationKind,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalJsonInitialization<'a> {
    #[serde(rename = "type")]
    _kind: InitializationKind,
    #[serde(borrow, deserialize_with = "deserialize_required_non_null_raw_value")]
    json: &'a RawValue,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LocalHtmlInitialization<'a> {
    #[serde(rename = "type")]
    _kind: InitializationKind,
    #[serde(borrow)]
    html: &'a RawValue,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoomInitialization {
    #[serde(rename = "type")]
    _kind: InitializationKind,
    document_id: String,
    lineage_id: String,
    #[serde(default, deserialize_with = "deserialize_non_null_option")]
    snapshot: Option<SnapshotMetadataEnvelope>,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum InitializationKind {
    LocalEmpty,
    LocalJson,
    LocalHtml,
    Room,
}

fn deserialize_required_non_null_raw_value<'de, D>(
    deserializer: D,
) -> Result<&'de RawValue, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <&RawValue as serde::Deserialize>::deserialize(deserializer)?;
    if raw.get() == "null" {
        return Err(serde::de::Error::custom("null is not allowed"));
    }
    Ok(raw)
}

#[uniffi::export]
pub fn editor_v2_create(config_json: String, snapshot_state: Option<Vec<u8>>) -> FfiJsonResult {
    json_result(crate::boundary::with_document_stack(|| {
        create_impl(&config_json, snapshot_state)
    }))
}

fn create_impl(config_json: &str, snapshot_state: Option<Vec<u8>>) -> Result<String, FfiError> {
    admit_create_wire_bytes(config_json.len()).map_err(ffi_error)?;
    admit_create_retained_envelope(config_json).map_err(ffi_error)?;
    let envelope: CreateEnvelope<'_> = parse_create_json(config_json).map_err(ffi_error)?;
    let initialization_probe: InitializationProbe =
        parse_create_json(envelope.initialization.get()).map_err(ffi_error)?;
    let (config, room_bound) =
        build_config(envelope, initialization_probe, snapshot_state).map_err(ffi_error)?;
    let schema = match &config.initialization {
        EditorInitialization::Local {
            initial_content: InitialContent::Empty,
        } => resolve_local_empty_document(config_json)
            .map(|resolved| resolved.schema)
            .map_err(ffi_error)?,
        EditorInitialization::Local {
            initial_content: InitialContent::Json(source),
        } => resolve_local_document(config_json, FfiViewerSourceKind::Json, source)
            .map(|resolved| resolved.schema)
            .map_err(ffi_error)?,
        EditorInitialization::Local {
            initial_content: InitialContent::Html(source),
        } => resolve_local_document(config_json, FfiViewerSourceKind::Html, source)
            .map(|resolved| resolved.schema)
            .map_err(ffi_error)?,
        EditorInitialization::Room { .. } => {
            resolve_configured_create_schema(&config).map_err(ffi_error)?
        }
    };
    let id = DocumentApiFacade::create_with_schema(config, schema.clone()).map_err(ffi_error)?;
    if room_bound {
        // Room sessions own the collaboration runtime (bounded outbox,
        // awareness bookkeeping) from creation; attachment is idempotent
        // and infallible, and the id was just issued, so the slot exists.
        let slot = registry::get_session(id).ok_or_else(unknown_editor_error)?;
        slot.with_alive(|session| {
            session.attach_collaboration_runtime();
            Ok::<(), SessionError>(())
        })
        .and_then(|value| value)
        .map_err(ffi_error)?;
    }
    super::render::register_session_schema(id, schema);
    Ok(serde_json::json!({ "editorId": id.to_string() }).to_string())
}

include!("editor/initialization.rs");

#[uniffi::export]
pub fn editor_v2_destroy(editor_id: String) -> FfiUnitResult {
    unit_result((|| {
        let id = parse_editor_id(&editor_id)?;
        if registry::get_session(id).is_none() {
            return Err(unknown_editor_error());
        }
        registry::destroy_session(id);
        super::render::unregister_session_schema(id);
        Ok(())
    })())
}

#[uniffi::export]
pub fn editor_v2_get_state(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let render_state = match session.render_state() {
            EngineRenderState::Loading => "Loading",
            EngineRenderState::Ready => "Ready",
        };
        Ok(serde_json::json!({
            "documentState": session.document_state.as_str(),
            "transportState": session.transport_state().as_str(),
            "renderState": render_state,
            "documentRevision": decimal_u64(session.engine.revision()),
            "documentOrigin": session.engine.document_origin().as_str(),
            "stateRevision": decimal_u64(session.engine.state_revision()),
            "canUndo": session.engine.can_undo(),
            "canRedo": session.engine.can_redo(),
        })
        .to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_get_document_json(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| session.get_json_string()))
}

#[uniffi::export]
pub fn editor_v2_get_document_html(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        session
            .get_html()
            .map(|html| serde_json::json!({ "html": html }).to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_get_content_snapshot(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let html = session.get_html()?;
        let json = session.get_json_string()?;
        let html = serde_json::to_string(&html).expect("HTML strings always serialize");
        let mut output =
            String::with_capacity(html.len().saturating_add(json.len()).saturating_add(18));
        output.push_str("{\"html\":");
        output.push_str(&html);
        output.push_str(",\"json\":");
        output.push_str(&json);
        output.push('}');
        Ok(output)
    }))
}

#[uniffi::export]
pub fn editor_v2_replace_document(editor_id: String, request_json: String) -> FfiJsonResult {
    json_result(with_editor_request_envelope(&editor_id, |session| {
        let envelope: ReplaceDocumentEnvelope<'_> = parse_request_envelope(session, &request_json)?;
        admit_version(envelope.version, envelope.request_id)?;
        let request_id = envelope.request_id;
        let history = ReplacementHistory::from(envelope.history);
        let commit = match (envelope.set_json, envelope.set_html) {
            (Some(json), None) => session.replace_document_json(request_id, json.get(), history)?,
            (None, Some(html)) => session.replace_document_html(request_id, &html, history)?,
            _ => {
                return Err(config_invalid(
                    Some(request_id),
                    "replace requests carry exactly one of setJson or setHtml",
                ));
            }
        };
        Ok(serde_json::json!({
            "changed": commit.changed,
            "documentRevision": decimal_u64(commit.document_revision),
        })
        .to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_apply_input(editor_id: String, request_json: String) -> FfiJsonResult {
    bridge_entry(&editor_id, |bridge| bridge.submit_input(&request_json))
}

#[uniffi::export]
pub fn editor_v2_apply_command(editor_id: String, request_json: String) -> FfiJsonResult {
    bridge_entry(&editor_id, |bridge| bridge.submit_command(&request_json))
}

#[uniffi::export]
pub fn editor_v2_apply_local_api(editor_id: String, request_json: String) -> FfiJsonResult {
    bridge_entry(&editor_id, |bridge| bridge.submit_local_api(&request_json))
}

#[uniffi::export]
pub fn editor_v2_set_selection(editor_id: String, request_json: String) -> FfiJsonResult {
    bridge_entry(&editor_id, |bridge| bridge.submit_selection(&request_json))
}

#[uniffi::export]
pub fn editor_v2_pin_position_epoch(
    editor_id: String,
    owner_id: String,
    document_revision: String,
) -> FfiJsonResult {
    let owner_id = match parse_canonical_u64(&owner_id) {
        Some(value) => value,
        None => {
            return FfiJsonResult::err(FfiError::from(config_invalid(
                None,
                "ownerId must be a canonical decimal u64 string",
            )));
        }
    };
    let document_revision = match parse_canonical_u64(&document_revision) {
        Some(value) => value,
        None => {
            return FfiJsonResult::err(FfiError::from(config_invalid(
                None,
                "documentRevision must be a canonical decimal u64 string",
            )));
        }
    };
    json_result(with_editor(&editor_id, |session| {
        let epoch = session.pin_position_epoch(owner_id, document_revision)?;
        Ok(serde_json::json!({ "positionEpoch": epoch.to_string() }).to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_apply_native_intent(editor_id: String, request_json: String) -> FfiJsonResult {
    json_result(with_editor_request_envelope(&editor_id, |session| {
        NativeTransactionBridge::new(session).submit_native_intent(&request_json)
    }))
}

#[uniffi::export]
pub fn editor_v2_release_native_binding(editor_id: String, owner_id: String) -> FfiUnitResult {
    let owner_id = match parse_canonical_u64(&owner_id) {
        Some(value) => value,
        None => {
            return FfiUnitResult::err(FfiError::from(config_invalid(
                None,
                "ownerId must be a canonical decimal u64 string",
            )));
        }
    };
    unit_result(with_editor(&editor_id, |session| {
        session.release_native_binding(owner_id);
        Ok(())
    }))
}

fn bridge_entry(
    editor_id: &str,
    entry: impl FnOnce(&mut NativeTransactionBridge<'_>) -> Result<NativeBridgeOutcome, SessionError>,
) -> FfiJsonResult {
    json_result(with_editor_request_envelope(editor_id, |session| {
        let mut bridge = NativeTransactionBridge::new(session);
        entry(&mut bridge).map(|outcome| match outcome {
            NativeBridgeOutcome::Transaction(result) => serde_json::json!({
                "type": "transaction",
                "changed": result.changed,
                "documentRevision": decimal_u64(result.document_revision),
                "stateRevision": decimal_u64(result.state_revision),
                "canUndo": result.history_state.can_undo,
                "canRedo": result.history_state.can_redo,
            })
            .to_string(),
            NativeBridgeOutcome::NotApplicable => {
                serde_json::json!({ "type": "notApplicable" }).to_string()
            }
            NativeBridgeOutcome::Replacement(commit) => serde_json::json!({
                "type": "replacement",
                "changed": commit.changed,
                "documentRevision": decimal_u64(commit.document_revision),
            })
            .to_string(),
        })
    }))
}

#[uniffi::export]
pub fn editor_v2_undo(editor_id: String, request_json: String) -> FfiJsonResult {
    history_entry(&editor_id, &request_json, |bridge, request_id| {
        bridge.undo(request_id)
    })
}

#[uniffi::export]
pub fn editor_v2_redo(editor_id: String, request_json: String) -> FfiJsonResult {
    history_entry(&editor_id, &request_json, |bridge, request_id| {
        bridge.redo(request_id)
    })
}

fn history_entry(
    editor_id: &str,
    request_json: &str,
    which: impl FnOnce(&mut NativeTransactionBridge<'_>, u64) -> Result<bool, SessionError>,
) -> FfiJsonResult {
    json_result(with_editor_request_envelope(editor_id, |session| {
        let envelope: HistoryRequestEnvelope = parse_request_envelope(session, request_json)?;
        admit_version(envelope.version, envelope.request_id)?;
        let mut bridge = NativeTransactionBridge::new(session);
        let changed = which(&mut bridge, envelope.request_id)?;
        Ok(serde_json::json!({ "changed": changed }).to_string())
    }))
}
