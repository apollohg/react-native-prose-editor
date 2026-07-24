//! Task 12: UniFFI v2 editor lifecycle, state, and mutation entry points
//! (production since the Task 16C cutover removed the staging gate).
//!
//! Every entry follows the frozen contract: decimal-string handle parsing,
//! registry lookup, `slot.with_alive` under-lock recheck, and a typed
//! result record carrying exactly one of value or error — never a panic,
//! never a stringly error. Internal seams still require a `u64` request id,
//! while FFI error correlation retains request-id presence separately so an
//! admitted external `0` remains distinguishable from no request envelope.
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

#[derive(Clone, Copy)]
struct JsonSpan {
    start: usize,
    end: usize,
}

impl JsonSpan {
    fn len(self) -> Result<usize, SessionError> {
        self.end
            .checked_sub(self.start)
            .ok_or_else(|| create_scan_invalid("invalid create JSON span"))
    }
}

#[derive(Default)]
struct CreateRootSpans {
    schema: Option<JsonSpan>,
    initialization: Option<JsonSpan>,
}

#[derive(Default)]
struct InitializationSpans {
    kind: Option<ScannedInitializationKind>,
    json: Option<JsonSpan>,
    html: Option<JsonSpan>,
}

#[derive(Clone, Copy)]
enum ScannedInitializationKind {
    LocalJson,
    LocalHtml,
    Other,
}

struct CreateJsonScanner<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> CreateJsonScanner<'a> {
    fn new(json: &'a str) -> Self {
        Self {
            bytes: json.as_bytes(),
            index: 0,
        }
    }

    fn at(json: &'a str, index: usize) -> Self {
        Self {
            bytes: json.as_bytes(),
            index,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn bump(&mut self) -> Result<u8, SessionError> {
        let byte = self
            .peek()
            .ok_or_else(|| create_scan_invalid("unexpected end of create JSON"))?;
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| create_scan_invalid("create JSON index overflow"))?;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) -> Result<(), SessionError> {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.bump()?;
        }
        Ok(())
    }

    fn consume(&mut self, expected: u8) -> Result<(), SessionError> {
        if self.bump()? != expected {
            return Err(create_scan_invalid("invalid create JSON syntax"));
        }
        Ok(())
    }

    fn scan_string(&mut self, expected: Option<&[u8]>) -> Result<(JsonSpan, bool), SessionError> {
        let start = self.index;
        self.consume(b'"')?;
        let mut expected_index = 0usize;
        let mut matches_expected = expected.is_some();
        loop {
            let byte = self.bump()?;
            match byte {
                b'"' => {
                    let matched = matches_expected
                        && expected.is_some_and(|value| expected_index == value.len());
                    return Ok((
                        JsonSpan {
                            start,
                            end: self.index,
                        },
                        matched,
                    ));
                }
                b'\\' => {
                    let escaped = self.bump()?;
                    let decoded = match escaped {
                        b'"' | b'\\' | b'/' => Some(escaped),
                        b'b' => Some(0x08),
                        b'f' => Some(0x0c),
                        b'n' => Some(b'\n'),
                        b'r' => Some(b'\r'),
                        b't' => Some(b'\t'),
                        b'u' => {
                            let mut code = 0u16;
                            for _ in 0..4 {
                                let digit = self.bump()?;
                                let value = match digit {
                                    b'0'..=b'9' => u16::from(digit - b'0'),
                                    b'a'..=b'f' => u16::from(digit - b'a') + 10,
                                    b'A'..=b'F' => u16::from(digit - b'A') + 10,
                                    _ => {
                                        return Err(create_scan_invalid(
                                            "invalid unicode escape in create JSON",
                                        ));
                                    }
                                };
                                code = code
                                    .checked_mul(16)
                                    .and_then(|current| current.checked_add(value))
                                    .ok_or_else(|| {
                                        create_scan_invalid("create JSON unicode escape overflow")
                                    })?;
                            }
                            u8::try_from(code).ok()
                        }
                        _ => return Err(create_scan_invalid("invalid escape in create JSON")),
                    };
                    if let Some(decoded) = decoded {
                        match_expected_byte(
                            expected,
                            &mut expected_index,
                            &mut matches_expected,
                            decoded,
                        )?;
                    } else {
                        matches_expected = false;
                    }
                }
                0x00..=0x1f => {
                    return Err(create_scan_invalid("unescaped control byte in create JSON"));
                }
                0x20..=0x7f => {
                    match_expected_byte(expected, &mut expected_index, &mut matches_expected, byte)?
                }
                _ => matches_expected = false,
            }
        }
    }

    fn scan_value(&mut self, depth: usize) -> Result<JsonSpan, SessionError> {
        enum Action {
            Value(usize),
            ObjectAfterValue(usize),
            ArrayAfterValue(usize),
        }

        let start = self.index;
        let mut actions = vec![Action::Value(depth)];
        while let Some(action) = actions.pop() {
            match action {
                Action::Value(depth) => {
                    if depth > CREATE_SCAN_MAX_DEPTH {
                        return Err(create_scan_invalid(
                            "create JSON nesting exceeds scanner limit",
                        ));
                    }
                    self.skip_whitespace()?;
                    match self.peek() {
                        Some(b'"') => {
                            self.scan_string(None)?;
                        }
                        Some(b'{') => {
                            self.bump()?;
                            self.skip_whitespace()?;
                            if self.peek() == Some(b'}') {
                                self.bump()?;
                                continue;
                            }
                            if self.peek() != Some(b'"') {
                                return Err(create_scan_invalid(
                                    "object key must be a string in create JSON",
                                ));
                            }
                            self.scan_string(None)?;
                            self.skip_whitespace()?;
                            self.consume(b':')?;
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ObjectAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        Some(b'[') => {
                            self.bump()?;
                            self.skip_whitespace()?;
                            if self.peek() == Some(b']') {
                                self.bump()?;
                                continue;
                            }
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ArrayAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        Some(b't') => self.scan_literal(b"true")?,
                        Some(b'f') => self.scan_literal(b"false")?,
                        Some(b'n') => self.scan_literal(b"null")?,
                        Some(b'-' | b'0'..=b'9') => self.scan_number()?,
                        _ => return Err(create_scan_invalid("invalid value in create JSON")),
                    }
                }
                Action::ObjectAfterValue(depth) => {
                    self.skip_whitespace()?;
                    match self.bump()? {
                        b'}' => {}
                        b',' => {
                            self.skip_whitespace()?;
                            if self.peek() != Some(b'"') {
                                return Err(create_scan_invalid(
                                    "object key must be a string in create JSON",
                                ));
                            }
                            self.scan_string(None)?;
                            self.skip_whitespace()?;
                            self.consume(b':')?;
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ObjectAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        _ => {
                            return Err(create_scan_invalid(
                                "invalid object delimiter in create JSON",
                            ))
                        }
                    }
                }
                Action::ArrayAfterValue(depth) => {
                    self.skip_whitespace()?;
                    match self.bump()? {
                        b']' => {}
                        b',' => {
                            let child_depth = depth
                                .checked_add(1)
                                .ok_or_else(|| create_scan_invalid("create JSON depth overflow"))?;
                            actions.push(Action::ArrayAfterValue(depth));
                            actions.push(Action::Value(child_depth));
                        }
                        _ => {
                            return Err(create_scan_invalid(
                                "invalid array delimiter in create JSON",
                            ))
                        }
                    }
                }
            }
        }
        Ok(JsonSpan {
            start,
            end: self.index,
        })
    }

    fn scan_literal(&mut self, literal: &[u8]) -> Result<(), SessionError> {
        let end = self
            .index
            .checked_add(literal.len())
            .ok_or_else(|| create_scan_invalid("create JSON literal index overflow"))?;
        if self.bytes.get(self.index..end) != Some(literal) {
            return Err(create_scan_invalid("invalid literal in create JSON"));
        }
        self.index = end;
        Ok(())
    }

    fn scan_number(&mut self) -> Result<(), SessionError> {
        if self.peek() == Some(b'-') {
            self.bump()?;
        }
        match self.peek() {
            Some(b'0') => {
                self.bump()?;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(create_scan_invalid("leading zero in create JSON number"));
                }
            }
            Some(b'1'..=b'9') => {
                self.bump()?;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.bump()?;
                }
            }
            _ => return Err(create_scan_invalid("invalid create JSON number")),
        }
        if self.peek() == Some(b'.') {
            self.bump()?;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(create_scan_invalid(
                    "invalid fraction in create JSON number",
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump()?;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.bump()?;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.bump()?;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(create_scan_invalid(
                    "invalid exponent in create JSON number",
                ));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.bump()?;
            }
        }
        Ok(())
    }

    fn span_equals_ascii(&self, span: JsonSpan, expected: &[u8]) -> Result<bool, SessionError> {
        let mut scanner = Self {
            bytes: self.bytes,
            index: span.start,
        };
        let (rescanned, matched) = scanner.scan_string(Some(expected))?;
        if rescanned.end != span.end {
            return Err(create_scan_invalid("invalid create JSON string span"));
        }
        Ok(matched)
    }
}

fn match_expected_byte(
    expected: Option<&[u8]>,
    expected_index: &mut usize,
    matches_expected: &mut bool,
    decoded: u8,
) -> Result<(), SessionError> {
    if !*matches_expected {
        return Ok(());
    }
    if expected.and_then(|value| value.get(*expected_index)) != Some(&decoded) {
        *matches_expected = false;
        return Ok(());
    }
    *expected_index = expected_index
        .checked_add(1)
        .ok_or_else(|| create_scan_invalid("create JSON string index overflow"))?;
    Ok(())
}

fn create_scan_invalid(message: impl Into<String>) -> SessionError {
    config_invalid(None, message)
}

fn scan_create_root(json: &str) -> Result<CreateRootSpans, SessionError> {
    let mut scanner = CreateJsonScanner::new(json);
    scanner.skip_whitespace()?;
    scanner.consume(b'{')?;
    scanner.skip_whitespace()?;
    let mut spans = CreateRootSpans::default();
    if scanner.peek() == Some(b'}') {
        scanner.bump()?;
    } else {
        loop {
            if scanner.peek() != Some(b'"') {
                return Err(create_scan_invalid("create root key must be a string"));
            }
            let (key, _) = scanner.scan_string(None)?;
            scanner.skip_whitespace()?;
            scanner.consume(b':')?;
            let value = scanner.scan_value(1)?;
            if scanner.span_equals_ascii(key, b"schema")? {
                if spans.schema.replace(value).is_some() {
                    return Err(create_scan_invalid("duplicate schema field in create JSON"));
                }
            } else if scanner.span_equals_ascii(key, b"initialization")?
                && spans.initialization.replace(value).is_some()
            {
                return Err(create_scan_invalid(
                    "duplicate initialization field in create JSON",
                ));
            }
            scanner.skip_whitespace()?;
            match scanner.bump()? {
                b'}' => break,
                b',' => scanner.skip_whitespace()?,
                _ => return Err(create_scan_invalid("invalid create root delimiter")),
            }
        }
    }
    scanner.skip_whitespace()?;
    if scanner.index != scanner.bytes.len() {
        return Err(create_scan_invalid("trailing bytes in create JSON"));
    }
    Ok(spans)
}

fn scan_initialization(
    json: &str,
    initialization: JsonSpan,
) -> Result<InitializationSpans, SessionError> {
    let mut scanner = CreateJsonScanner::at(json, initialization.start);
    if scanner.peek() != Some(b'{') {
        return Ok(InitializationSpans::default());
    }
    scanner.consume(b'{')?;
    scanner.skip_whitespace()?;
    let mut type_span = None;
    let mut spans = InitializationSpans::default();
    if scanner.peek() == Some(b'}') {
        scanner.bump()?;
    } else {
        loop {
            if scanner.peek() != Some(b'"') {
                return Err(create_scan_invalid("initialization key must be a string"));
            }
            let (key, _) = scanner.scan_string(None)?;
            scanner.skip_whitespace()?;
            scanner.consume(b':')?;
            let value = scanner.scan_value(2)?;
            if scanner.span_equals_ascii(key, b"type")? {
                if type_span.replace(value).is_some() {
                    return Err(create_scan_invalid("duplicate initialization type"));
                }
            } else if scanner.span_equals_ascii(key, b"json")? {
                if spans.json.replace(value).is_some() {
                    return Err(create_scan_invalid("duplicate initialization json"));
                }
            } else if scanner.span_equals_ascii(key, b"html")?
                && spans.html.replace(value).is_some()
            {
                return Err(create_scan_invalid("duplicate initialization html"));
            }
            scanner.skip_whitespace()?;
            match scanner.bump()? {
                b'}' => break,
                b',' => scanner.skip_whitespace()?,
                _ => return Err(create_scan_invalid("invalid initialization delimiter")),
            }
        }
    }
    if scanner.index != initialization.end {
        return Err(create_scan_invalid("invalid initialization span"));
    }
    spans.kind = match type_span {
        Some(value) if scanner.bytes.get(value.start) == Some(&b'"') => {
            if scanner.span_equals_ascii(value, b"localJson")? {
                Some(ScannedInitializationKind::LocalJson)
            } else if scanner.span_equals_ascii(value, b"localHtml")? {
                Some(ScannedInitializationKind::LocalHtml)
            } else {
                Some(ScannedInitializationKind::Other)
            }
        }
        Some(_) => Some(ScannedInitializationKind::Other),
        None => None,
    };
    Ok(spans)
}

fn admit_create_retained_envelope(json: &str) -> Result<(), SessionError> {
    let root = scan_create_root(json)?;
    let initialization = root
        .initialization
        .map(|span| scan_initialization(json, span))
        .transpose()?
        .unwrap_or_default();
    let selected_payload = match initialization.kind {
        Some(ScannedInitializationKind::LocalJson) => initialization.json,
        Some(ScannedInitializationKind::LocalHtml) => initialization.html,
        Some(ScannedInitializationKind::Other) | None => None,
    };
    let mut retained = json.len();
    for deferred in [root.schema, selected_payload].into_iter().flatten() {
        retained = retained
            .checked_sub(deferred.len()?)
            .ok_or_else(|| create_scan_invalid("create retained-envelope underflow"))?;
    }
    admit_create_envelope_bytes(retained)
}

// ---------------------------------------------------------------------------
// Shared session access (used by the collaboration and snapshot modules too)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Request envelopes
// ---------------------------------------------------------------------------

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
/// with the Task 7 bridge local-API envelope).
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

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

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
    let container_limit = crate::schema::MAX_SCHEMA_METADATA_DEPTH
        .checked_add(16)
        .ok_or_else(|| BoundaryError::new("SCHEMA_INVALID", "schema depth limit overflow"))?;
    let schema = parse_json_value_stack_safe(
        input.as_str(),
        container_limit,
        crate::schema::MAX_SCHEMA_METADATA_DEPTH,
        "SCHEMA_INVALID",
        "SCHEMA_INVALID",
    )?;
    crate::schema::Schema::from_json_with_limits(schema.as_value(), &config.resource_limits)
        .map_err(SessionError::from)
}

fn build_config(
    envelope: CreateEnvelope<'_>,
    initialization_probe: InitializationProbe,
    snapshot_state: Option<Vec<u8>>,
) -> Result<(EditorSessionConfig, bool), SessionError> {
    let CreateEnvelope {
        schema,
        fragment_name,
        initialization: initialization_json,
        policy,
        limits,
    } = envelope;
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
            None,
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
                        None,
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
            "documentRevision": decimal_u64(session.engine.revision()),
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

// ---------------------------------------------------------------------------
// Mutation entries
// ---------------------------------------------------------------------------

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
