//! Task 12: UniFFI v2 editor lifecycle, state, and mutation entry points
//! (production since the Task 16C cutover removed the staging gate).
//!
//! Every entry follows the frozen contract: decimal-string handle parsing,
//! registry lookup, `slot.with_alive` under-lock recheck, and a typed
//! result record carrying exactly one of value or error — never a panic,
//! never a stringly error. Internal seams stamp `u64` request ids; entries
//! that take no request envelope use [`ABSENT_REQUEST_ID`], which the
//! boundary strips back to absent per the Task 1 nullability rules.
//!
//! `create` reuses the Task 4 session config format: room initialization
//! carries `documentId`/`lineageId` plus optional snapshot *metadata* in
//! the JSON; the snapshot's encoded state rides as direct bytes in the
//! separate `snapshot_state` parameter (binary values are never JSON
//! number arrays). Room-bound sessions own a collaboration runtime from
//! creation, so offline edits queue against the bounded outbox from the
//! first keystroke.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use serde_json::Value;

use crate::boundary::{BoundaryError, BoundedInput, InputKind, ResourceLimits};
use crate::document_api::DocumentApiFacade;
use crate::native_transaction_bridge::{
    HistoryModeEnvelope, NativeBridgeOutcome, NativeTransactionBridge,
};
use crate::registry;
use crate::session::{
    EditorInitialization, EditorSession, EditorSessionConfig, ErrorDomain, InitialContent,
    SessionError,
};
use crate::yrs_engine::{DocumentScope, EngineRenderState, ReplacementHistory};

use super::snapshot::SnapshotMetadataEnvelope;
use super::types::{FfiError, FfiJsonResult, FfiUnitResult};

/// The request-id sentinel for entries that take no request envelope: the
/// internal seams stamp it, and the boundary strips it back to absent so
/// `requestId` is omitted per the frozen nullability rules. Envelopes that
/// carry an explicit request id keep it (a caller-supplied `0` is
/// indistinguishable from absent by design and documented as reserved).
pub(crate) const ABSENT_REQUEST_ID: u64 = 0;

/// One supported version for every v2 request envelope.
const V2_ENVELOPE_VERSION: u64 = 1;

// ---------------------------------------------------------------------------
// Shared session access (used by the collaboration and snapshot modules too)
// ---------------------------------------------------------------------------

/// Decimal-string handles at the boundary; malformed handles are structured
/// boundary errors, never panics.
fn parse_editor_id(handle: &str) -> Result<u64, FfiError> {
    handle.parse::<u64>().map_err(|_| {
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

/// Convert a session error, stripping the absent-request sentinel so
/// envelope-less entries omit `requestId` per the nullability rules.
pub(crate) fn ffi_error(error: SessionError) -> FfiError {
    let mut error = FfiError::from(error);
    if error.request_id.as_deref() == Some("0") {
        error.request_id = None;
    }
    error
}

/// Registry lookup -> `slot.with_alive` under-lock recheck -> typed error on
/// failure. The absent-request-id sentinel is stripped on the way out.
pub(crate) fn with_editor<T>(
    handle: &str,
    operation: impl FnOnce(&mut EditorSession) -> Result<T, SessionError>,
) -> Result<T, FfiError> {
    let id = parse_editor_id(handle)?;
    let slot = registry::get_session(id).ok_or_else(unknown_editor_error)?;
    slot.with_alive(operation)
        .and_then(|value| value)
        .map_err(ffi_error)
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

/// Serialize a session error for the receive outcome's structured close
/// cause, honoring the same nullability rules as the boundary envelope.
pub(crate) fn session_error_json(error: &SessionError) -> Value {
    let error = FfiError::from(error.clone());
    let mut value = serde_json::json!({
        "domain": error.domain,
        "code": error.code,
        "message": error.message,
    });
    let object = value.as_object_mut().expect("error base is an object");
    if let Some(request_id) = error.request_id {
        if request_id != ABSENT_REQUEST_ID.to_string() {
            object.insert("requestId".into(), Value::String(request_id));
        }
    }
    if let Some(operation_index) = error.operation_index {
        object.insert("operationIndex".into(), operation_index.into());
    }
    if let Some(limit) = error.limit {
        object.insert("limit".into(), limit.into());
    }
    if let Some(actual) = error.actual {
        object.insert("actual".into(), actual.into());
    }
    if let Some(details_json) = error.details_json {
        if let Ok(details) = serde_json::from_str::<Value>(&details_json) {
            object.insert("details".into(), details);
        }
    }
    value
}

// ---------------------------------------------------------------------------
// Request envelopes
// ---------------------------------------------------------------------------

fn parse_request_envelope<T: serde::de::DeserializeOwned>(
    session: &EditorSession,
    json: &str,
) -> Result<T, SessionError> {
    let input = BoundedInput::new(json, InputKind::Config, session.engine.resource_limits())?;
    serde_json::from_str(input.as_str())
        .map_err(|error| SessionError::from(BoundaryError::parse("CONFIG_INVALID", error)))
}

fn admit_version(version: u64, request_id: u64) -> Result<(), SessionError> {
    if version != V2_ENVELOPE_VERSION {
        return Err(config_invalid(
            request_id,
            format!(
                "unsupported v2 envelope version {version}; supported version is {V2_ENVELOPE_VERSION}"
            ),
        ));
    }
    Ok(())
}

fn config_invalid(request_id: u64, message: impl Into<String>) -> SessionError {
    let mut error = SessionError::new(ErrorDomain::Boundary, "CONFIG_INVALID", message);
    error.request_id = Some(request_id);
    error
}

/// Data-only mirror of the replacement history policy (shared wire shape
/// with the Task 7 bridge local-API envelope).
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplaceDocumentEnvelope {
    version: u64,
    request_id: u64,
    set_json: Option<Value>,
    set_html: Option<String>,
    history: HistoryModeEnvelope,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HistoryRequestEnvelope {
    version: u64,
    request_id: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateEnvelope {
    schema: Option<Value>,
    fragment_name: Option<String>,
    initialization: InitializationEnvelope,
    max_length: Option<u32>,
    read_only: Option<bool>,
    input_filter: Option<String>,
    allow_base64_images: Option<bool>,
}

/// `deny_unknown_fields` cannot be enforced inside an internally tagged
/// enum (same serde limitation the Task 7 bridge documents); no variant
/// carries a privileged field. Variant names are camelCase via the enum;
/// struct-variant fields need the per-variant attribute (serde applies
/// enum-level `rename_all` to variant names only).
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum InitializationEnvelope {
    LocalEmpty,
    LocalJson {
        json: Value,
    },
    LocalHtml {
        html: String,
    },
    #[serde(rename_all = "camelCase")]
    Room {
        document_id: String,
        lineage_id: String,
        snapshot: Option<SnapshotMetadataEnvelope>,
    },
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[uniffi::export]
pub fn editor_v2_create(config_json: String, snapshot_state: Option<Vec<u8>>) -> FfiJsonResult {
    json_result(create_impl(&config_json, snapshot_state))
}

fn create_impl(config_json: &str, snapshot_state: Option<Vec<u8>>) -> Result<String, FfiError> {
    let input = BoundedInput::new(config_json, InputKind::Config, &ResourceLimits::default())
        .map_err(SessionError::from)
        .map_err(ffi_error)?;
    let envelope: CreateEnvelope = serde_json::from_str(input.as_str())
        .map_err(|error| SessionError::from(BoundaryError::parse("CONFIG_INVALID", error)))
        .map_err(ffi_error)?;
    // Task 16B: resolve the schema before construction so the render
    // accessor can be registered with it below; an invalid schema fails here
    // with the identical error the session admission would produce.
    let schema = super::render::resolve_create_schema(&envelope.schema).map_err(ffi_error)?;
    let (config, room_bound) = build_config(envelope, snapshot_state).map_err(ffi_error)?;
    let id = DocumentApiFacade::create(config).map_err(ffi_error)?;
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

fn build_config(
    envelope: CreateEnvelope,
    snapshot_state: Option<Vec<u8>>,
) -> Result<(EditorSessionConfig, bool), SessionError> {
    let CreateEnvelope {
        schema,
        fragment_name,
        initialization,
        max_length,
        read_only,
        input_filter,
        allow_base64_images,
    } = envelope;
    if !matches!(initialization, InitializationEnvelope::Room { .. }) && snapshot_state.is_some() {
        return Err(config_invalid(
            ABSENT_REQUEST_ID,
            "snapshot state bytes require a room initialization with snapshot metadata",
        ));
    }
    let (initialization, room_bound) = match initialization {
        InitializationEnvelope::LocalEmpty => (
            EditorInitialization::Local {
                initial_content: InitialContent::Empty,
            },
            false,
        ),
        InitializationEnvelope::LocalJson { json } => (
            EditorInitialization::Local {
                initial_content: InitialContent::Json(json.to_string()),
            },
            false,
        ),
        InitializationEnvelope::LocalHtml { html } => (
            EditorInitialization::Local {
                initial_content: InitialContent::Html(html),
            },
            false,
        ),
        InitializationEnvelope::Room {
            document_id,
            lineage_id,
            snapshot,
        } => {
            let snapshot = match (snapshot, snapshot_state) {
                (Some(metadata), Some(encoded_state)) => {
                    Some(metadata.into_snapshot(encoded_state))
                }
                (None, None) => None,
                _ => {
                    return Err(config_invalid(
                        ABSENT_REQUEST_ID,
                        "room snapshot metadata and snapshot state bytes must arrive together",
                    ));
                }
            };
            (
                EditorInitialization::Room {
                    scope: DocumentScope {
                        document_id,
                        lineage_id,
                    },
                    snapshot,
                },
                true,
            )
        }
    };
    Ok((
        EditorSessionConfig {
            schema_json: schema.map(|schema| schema.to_string()),
            fragment_name: fragment_name.unwrap_or_else(|| "prosemirror".into()),
            initialization,
            resource_limits: ResourceLimits::default(),
            editing_limits: crate::yrs_engine::EditingLimits::default(),
            collaboration_limits: crate::session::CollaborationLimits::default(),
            max_length,
            read_only: read_only.unwrap_or(false),
            input_filter,
            allow_base64_images: allow_base64_images.unwrap_or(false),
        },
        room_bound,
    ))
}

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

// ---------------------------------------------------------------------------
// State getters
// ---------------------------------------------------------------------------

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
            "documentRevision": session.engine.revision(),
            "stateRevision": session.engine.state_revision(),
            "canUndo": session.engine.can_undo(),
            "canRedo": session.engine.can_redo(),
        })
        .to_string())
    }))
}

#[uniffi::export]
pub fn editor_v2_get_document_json(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        session.get_json().map(|document| document.to_string())
    }))
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
        Ok(serde_json::json!({
            "html": session.get_html()?,
            "json": session.get_json()?,
        })
        .to_string())
    }))
}

// ---------------------------------------------------------------------------
// Mutation entries
// ---------------------------------------------------------------------------

#[uniffi::export]
pub fn editor_v2_replace_document(editor_id: String, request_json: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let envelope: ReplaceDocumentEnvelope = parse_request_envelope(session, &request_json)?;
        admit_version(envelope.version, envelope.request_id)?;
        let request_id = envelope.request_id;
        let history = ReplacementHistory::from(envelope.history);
        let commit = match (envelope.set_json, envelope.set_html) {
            (Some(json), None) => {
                session.replace_document_json(request_id, &json.to_string(), history)?
            }
            (None, Some(html)) => session.replace_document_html(request_id, &html, history)?,
            _ => {
                return Err(config_invalid(
                    request_id,
                    "replace requests carry exactly one of setJson or setHtml",
                ));
            }
        };
        Ok(serde_json::json!({
            "changed": commit.changed,
            "documentRevision": commit.document_revision,
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

fn bridge_entry(
    editor_id: &str,
    entry: impl FnOnce(&mut NativeTransactionBridge<'_>) -> Result<NativeBridgeOutcome, SessionError>,
) -> FfiJsonResult {
    json_result(with_editor(editor_id, |session| {
        let mut bridge = NativeTransactionBridge::new(session);
        entry(&mut bridge).map(|outcome| match outcome {
            NativeBridgeOutcome::Transaction(result) => serde_json::json!({
                "type": "transaction",
                "changed": result.changed,
                "documentRevision": result.document_revision,
                "stateRevision": result.state_revision,
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
                "documentRevision": commit.document_revision,
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
    json_result(with_editor(editor_id, |session| {
        let envelope: HistoryRequestEnvelope = parse_request_envelope(session, request_json)?;
        admit_version(envelope.version, envelope.request_id)?;
        let mut bridge = NativeTransactionBridge::new(session);
        let changed = which(&mut bridge, envelope.request_id)?;
        Ok(serde_json::json!({ "changed": changed }).to_string())
    }))
}
