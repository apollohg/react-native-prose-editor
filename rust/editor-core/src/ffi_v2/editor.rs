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

use serde_json::{value::RawValue, Value};

use crate::boundary::{
    deserialize_non_null_option, BoundaryError, BoundedInput, InputKind, ResourceLimitOverrides,
    ResourceLimits, HARD_MAX_INPUT_BYTES,
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
use crate::yrs_engine::{
    DocumentScope, EditingLimitOverrides, EditingLimits, EngineRenderState, ReplacementHistory,
};

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

#[cfg(test)]
std::thread_local! {
    static CREATE_METADATA_MATERIALIZATION_COUNT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_create_metadata_materialization_count_for_test() {
    CREATE_METADATA_MATERIALIZATION_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_create_metadata_materialization_count_for_test() -> usize {
    CREATE_METADATA_MATERIALIZATION_COUNT.replace(0)
}

#[cfg(test)]
fn note_create_metadata_materialization_for_test() {
    CREATE_METADATA_MATERIALIZATION_COUNT.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn note_create_metadata_materialization_for_test() {}

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
struct InitializationProbe<'a> {
    #[serde(rename = "type")]
    kind: InitializationKind,
    #[serde(default, borrow)]
    json: Option<&'a RawValue>,
    #[serde(default, borrow)]
    html: Option<&'a RawValue>,
}

impl InitializationProbe<'_> {
    fn deferred_payload_bytes(&self) -> usize {
        match self.kind {
            InitializationKind::LocalJson => self.json.map_or(0, |json| json.get().len()),
            InitializationKind::LocalHtml => self.html.map_or(0, |html| html.get().len()),
            InitializationKind::LocalEmpty | InitializationKind::Room => 0,
        }
    }
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

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

#[uniffi::export]
pub fn editor_v2_create(config_json: String, snapshot_state: Option<Vec<u8>>) -> FfiJsonResult {
    json_result(create_impl(&config_json, snapshot_state))
}

fn create_impl(config_json: &str, snapshot_state: Option<Vec<u8>>) -> Result<String, FfiError> {
    admit_create_wire_bytes(config_json.len()).map_err(ffi_error)?;
    let envelope: CreateEnvelope<'_> = parse_create_json(config_json).map_err(ffi_error)?;
    let initialization_probe: InitializationProbe<'_> =
        parse_create_json(envelope.initialization.get()).map_err(ffi_error)?;
    let deferred_payload_bytes = envelope
        .schema
        .map_or(0, |schema| schema.get().len())
        .saturating_add(initialization_probe.deferred_payload_bytes());
    admit_create_envelope_bytes(config_json.len().saturating_sub(deferred_payload_bytes))
        .map_err(ffi_error)?;
    let (config, room_bound) =
        build_config(envelope, initialization_probe, snapshot_state).map_err(ffi_error)?;
    let schema = resolve_configured_create_schema(&config).map_err(ffi_error)?;
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

fn parse_create_json<'de, T>(json: &'de str) -> Result<T, SessionError>
where
    T: serde::Deserialize<'de>,
{
    serde_json::from_str(json)
        .map_err(|error| SessionError::from(BoundaryError::parse("CONFIG_INVALID", error)))
}

fn admit_create_wire_bytes(actual: usize) -> Result<(), SessionError> {
    if actual > CREATE_WIRE_MAX_BYTES {
        return Err(
            BoundaryError::limit("INPUT_LIMIT_EXCEEDED", CREATE_WIRE_MAX_BYTES, actual).into(),
        );
    }
    Ok(())
}

fn admit_create_envelope_bytes(actual: usize) -> Result<(), SessionError> {
    if actual > CREATE_ENVELOPE_MAX_BYTES {
        return Err(BoundaryError::limit(
            "INPUT_LIMIT_EXCEEDED",
            CREATE_ENVELOPE_MAX_BYTES,
            actual,
        )
        .into());
    }
    Ok(())
}

fn resolve_configured_create_schema(
    config: &EditorSessionConfig,
) -> Result<crate::schema::Schema, SessionError> {
    let Some(schema_json) = config.schema_json.as_deref() else {
        return super::render::resolve_create_schema(&None);
    };
    let input = BoundedInput::new(schema_json, InputKind::Config, &config.resource_limits)?;
    let schema: Value = serde_json::from_str(input.as_str())
        .map_err(|error| BoundaryError::parse("SCHEMA_INVALID", error))?;
    crate::schema::Schema::from_json_with_limits(&schema, &config.resource_limits)
        .map_err(SessionError::from)
}

fn build_config(
    envelope: CreateEnvelope<'_>,
    initialization_probe: InitializationProbe<'_>,
    snapshot_state: Option<Vec<u8>>,
) -> Result<(EditorSessionConfig, bool), SessionError> {
    let CreateEnvelope {
        schema,
        fragment_name,
        initialization: initialization_json,
        policy,
        limits,
    } = envelope;
    note_create_metadata_materialization_for_test();
    let LimitsEnvelope {
        resource,
        editing,
        collaboration,
    } = limits
        .map(|limits| parse_create_json(limits.get()))
        .transpose()?
        .unwrap_or_default();
    let resource_limits = ResourceLimits::resolve(resource.unwrap_or_default())?;
    let editing_limits = EditingLimits::resolve(editing.unwrap_or_default())?;
    let collaboration_limits = CollaborationLimits::resolve(collaboration.unwrap_or_default())?;
    let PolicyEnvelope {
        max_length,
        read_only,
        input_filter,
        allow_base64_images,
    } = policy
        .map(|policy| parse_create_json(policy.get()))
        .transpose()?
        .unwrap_or_default();
    let fragment_name = fragment_name
        .map(|fragment_name| parse_create_json(fragment_name.get()))
        .transpose()?;
    if !matches!(initialization_probe.kind, InitializationKind::Room) && snapshot_state.is_some() {
        return Err(config_invalid(
            ABSENT_REQUEST_ID,
            "snapshot state bytes require a room initialization with snapshot metadata",
        ));
    }
    let schema_json = schema
        .map(|schema| materialize_raw_payload(schema, InputKind::Config, &resource_limits))
        .transpose()?;
    let (initialization, room_bound) = match initialization_probe.kind {
        InitializationKind::LocalEmpty => {
            let _: LocalEmptyInitialization = parse_create_json(initialization_json.get())?;
            (
                EditorInitialization::Local {
                    initial_content: InitialContent::Empty,
                },
                false,
            )
        }
        InitializationKind::LocalJson => {
            let initialization: LocalJsonInitialization<'_> =
                parse_create_json(initialization_json.get())?;
            let json = materialize_raw_payload(
                initialization.json,
                InputKind::DocumentJson,
                &resource_limits,
            )?;
            (
                EditorInitialization::Local {
                    initial_content: InitialContent::Json(json),
                },
                false,
            )
        }
        InitializationKind::LocalHtml => {
            let initialization: LocalHtmlInitialization<'_> =
                parse_create_json(initialization_json.get())?;
            let html = materialize_html(initialization.html, &resource_limits)?;
            (
                EditorInitialization::Local {
                    initial_content: InitialContent::Html(html),
                },
                false,
            )
        }
        InitializationKind::Room => {
            let initialization: RoomInitialization = parse_create_json(initialization_json.get())?;
            let snapshot = match (initialization.snapshot, snapshot_state) {
                (Some(metadata), Some(encoded_state)) => {
                    admit_snapshot_state(&encoded_state, &resource_limits)?;
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
                        document_id: initialization.document_id,
                        lineage_id: initialization.lineage_id,
                    },
                    snapshot,
                },
                true,
            )
        }
    };
    Ok((
        EditorSessionConfig {
            schema_json,
            fragment_name: fragment_name.unwrap_or_else(|| "prosemirror".into()),
            initialization,
            resource_limits,
            editing_limits,
            collaboration_limits,
            max_length,
            read_only: read_only.unwrap_or(false),
            input_filter,
            allow_base64_images: allow_base64_images.unwrap_or(false),
        },
        room_bound,
    ))
}

fn materialize_raw_payload(
    raw: &RawValue,
    kind: InputKind,
    limits: &ResourceLimits,
) -> Result<String, SessionError> {
    let input = BoundedInput::new(raw.get(), kind, limits)?;
    Ok(input.as_str().to_owned())
}

fn materialize_html(raw: &RawValue, limits: &ResourceLimits) -> Result<String, SessionError> {
    let json = raw.get();
    if let Some(unescaped) = json
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .filter(|value| !value.as_bytes().contains(&b'\\'))
    {
        let input = BoundedInput::new(unescaped, InputKind::Html, limits)?;
        return Ok(input.as_str().to_owned());
    }
    let html: String = parse_create_json(json)?;
    BoundedInput::new(&html, InputKind::Html, limits)?;
    Ok(html)
}

fn admit_snapshot_state(encoded_state: &[u8], limits: &ResourceLimits) -> Result<(), SessionError> {
    if encoded_state.len() > limits.max_encoded_state_bytes {
        return Err(BoundaryError::limit(
            "INPUT_LIMIT_EXCEEDED",
            limits.max_encoded_state_bytes,
            encoded_state.len(),
        )
        .into());
    }
    Ok(())
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
