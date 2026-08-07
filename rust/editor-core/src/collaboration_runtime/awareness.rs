//! Runtime awareness ownership: desired local state, peer projections, and
//! deterministic renewal/expiry clocks.
//!
//! The runtime owns the *lifecycle* of awareness — the desired local state
//! JSON (validated and size-bounded at set time), per-peer activity
//! deadlines, and the public peer projections — while ALL wire and clock
//! state stays sealed inside the engine-owned
//! [`crate::yrs_engine::AwarenessCodec`]. This module never constructs a
//! `yrs::sync::Awareness`, never duplicates the codec's clocks, and never
//! reads a wall clock: time enters exclusively as the `now_millis` parameter
//! of [`CollaborationRuntime::tick`], which the native host reaches through
//! Rust's returned collaboration-drive deadline.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::ffi_v2::types::{decimal_u64, AWARENESS_CLOCK_EXHAUSTED};
use crate::session::{CollaborationLimits, ErrorDomain, SessionError, TransportState};
use crate::yrs_engine::{AwarenessLimits, YrsDocumentEngine, YrsEngineError};

use super::outbox::OutboxReservationError;
use super::protocol::{
    frame_awareness_message, TRANSPORT_REPLY_LIMIT_EXCEEDED, TRANSPORT_RESOURCE_EXHAUSTED,
};
use super::CollaborationRuntime;

/// Local desired awareness is re-published with a fresh clock every renewal
/// interval while synchronized (design owners' value).
pub const AWARENESS_RENEWAL_INTERVAL_MILLIS: u64 = 15_000;
/// Remote peers with no observed activity for this long are expired
/// (standard y-protocols removal semantics; design owners' value).
pub const AWARENESS_EXPIRY_MILLIS: u64 = 30_000;

/// Refusal code for a desired awareness state that is not valid JSON.
pub const AWARENESS_STATE_INVALID: &str = "AWARENESS_STATE_INVALID";
/// Refusal code for a tick whose deterministic time regresses.
pub const AWARENESS_TIME_REGRESSION: &str = "AWARENESS_TIME_REGRESSION";

/// Wire action reported by awareness-shaped refusals.
const AWARENESS_ACTION: &str = "awareness";

/// The only caller-controlled shape accepted by the production awareness
/// ABI. Its published value is assembled by Rust, so `cursor` can only ever
/// be the sticky engine representation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalAwarenessIntent {
    state: Value,
    focused: bool,
    #[serde(default)]
    selection: LocalAwarenessSelectionWire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum LocalAwarenessSelection {
    Text { anchor: u32, head: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AwarenessSelectionOutcome {
    pub(crate) outbound_changed: bool,
}

/// How one published intent treats the local cursor. Rust owns the sticky
/// index, so a caller that is only changing focus or application state must
/// be able to say "leave it alone" without restating a document position it
/// may no longer be able to resolve.
///
/// - key omitted -> [`Self::Retain`]: keep the sticky cursor already
///   published, whatever the document has done since;
/// - `"selection": null` -> [`Self::Clear`]: publish no cursor at all;
/// - a text selection -> [`Self::Present`]: materialize these positions
///   against the current document, refusing if they do not resolve.
#[derive(Debug, Default)]
enum LocalAwarenessSelectionWire {
    #[default]
    Retain,
    Clear,
    Present(LocalAwarenessSelection),
}

impl<'de> Deserialize<'de> for LocalAwarenessSelectionWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match Option::<LocalAwarenessSelection>::deserialize(deserializer)? {
            None => Ok(Self::Clear),
            Some(selection) => Ok(Self::Present(selection)),
        }
    }
}

fn awareness_state_invalid(request_id: u64, message: impl Into<String>) -> SessionError {
    let mut error = SessionError::new(ErrorDomain::Boundary, AWARENESS_STATE_INVALID, message);
    error.request_id = Some(request_id);
    error
}

fn awareness_peer_bytes_limit_error(request_id: u64, limit: usize, actual: usize) -> SessionError {
    engine_error_with_request(
        YrsEngineError::limit("INPUT_LIMIT_EXCEEDED", limit, actual)
            .with_details(serde_json::json!({ "field": "maxAwarenessPeerBytes" })),
        request_id,
    )
}

fn contains_reserved_cursor(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_reserved_cursor),
        Value::Object(entries) => entries
            .iter()
            .any(|(key, value)| key == "cursor" || contains_reserved_cursor(value)),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

/// Runtime-owned awareness bookkeeping: exactly the three things the design
/// assigns to this layer — desired-state JSON, deadlines, and the inputs of
/// the public projections. Clocks, tombstones, and wire state live in the
/// engine-owned codec.
pub(crate) struct AwarenessRuntimeState {
    /// The validated desired local presence; survives disconnect, generation
    /// close, `Incompatible`, and detach/reattach by construction.
    desired_state: Option<Value>,
    /// The most recent `tick(now_millis)` observation; receives stamp peer
    /// activity with this value, so no other clock source exists.
    now_millis: u64,
    /// When the desired state was last handed to the outbox for broadcast.
    last_local_publish_millis: Option<u64>,
    /// One already-clocked local tombstone waiting for a retryable outbox
    /// reservation. Its framed bytes remain immutable across retries.
    pending_withdrawal: Option<PendingWithdrawal>,
    /// Remote client -> last observed activity (`now_millis` of the tick
    /// preceding the touching update). Sorted so expiry order is stable.
    peer_activity: BTreeMap<u64, u64>,
}

struct PendingWithdrawal {
    frame: Vec<u8>,
    retry_not_before_millis: Option<u64>,
}

impl PendingWithdrawal {
    fn new(frame: Vec<u8>, now_millis: u64) -> Self {
        Self {
            frame,
            retry_not_before_millis: now_millis.checked_add(AWARENESS_RENEWAL_INTERVAL_MILLIS),
        }
    }

    fn defer_retry(&mut self, now_millis: u64) {
        self.retry_not_before_millis = now_millis.checked_add(AWARENESS_RENEWAL_INTERVAL_MILLIS);
    }

    fn is_due(&self, now_millis: u64) -> bool {
        self.retry_not_before_millis
            .is_some_and(|deadline| now_millis >= deadline)
    }
}

impl AwarenessRuntimeState {
    pub(crate) fn new() -> Self {
        Self {
            desired_state: None,
            now_millis: 0,
            last_local_publish_millis: None,
            pending_withdrawal: None,
            peer_activity: BTreeMap::new(),
        }
    }

    /// Task 11 restore bookkeeping reset: peer activity deadlines die with
    /// the prior store's peers (the codec's store-swap rebind dropped the
    /// states themselves), and the renewal clock restarts — the fresh-clock
    /// local re-publish inside the restore never reached the outbox, so no
    /// broadcast has happened for the new store. The desired state itself is
    /// retained by design.
    pub(crate) fn reset_for_restore(&mut self) {
        self.peer_activity.clear();
        self.last_local_publish_millis = None;
    }

    /// The requested next tick: the earlier of the local renewal deadline
    /// (while synchronized with a desired state) and the earliest remote
    /// expiry deadline. Candidates beyond the representable clock range are
    /// unschedulable and omitted. `None` means no clock work is pending.
    fn next_deadline_millis(&self, transport_state: TransportState) -> Option<u64> {
        let local_publish =
            if transport_state == TransportState::Synchronized && self.desired_state.is_some() {
                self.last_local_publish_millis.map_or_else(
                    || Some(self.now_millis),
                    |published| published.checked_add(AWARENESS_RENEWAL_INTERVAL_MILLIS),
                )
            } else if transport_state == TransportState::Synchronized {
                self.pending_withdrawal
                    .as_ref()
                    .and_then(|pending| pending.retry_not_before_millis)
            } else {
                None
            };
        let expiry = self
            .peer_activity
            .values()
            .filter_map(|seen| seen.checked_add(AWARENESS_EXPIRY_MILLIS))
            .min();
        match (local_publish, expiry) {
            (Some(local_publish), Some(expiry)) => Some(local_publish.min(expiry)),
            (deadline @ Some(_), None) | (None, deadline @ Some(_)) => deadline,
            (None, None) => None,
        }
    }
}

/// The session collaborators one awareness operation composes; built by the
/// session's field-disjoint split borrow, mirroring the `ReceiveContext`
/// idiom.
pub(crate) struct AwarenessContext<'a> {
    pub(crate) engine: &'a mut YrsDocumentEngine,
    pub(crate) transport_state: TransportState,
    pub(crate) limits: &'a CollaborationLimits,
}

/// Public projection of one live awareness entry: identity, protocol clock,
/// validated JSON state, and the resolved cursor. Tombstoned clients are
/// never projected; peers whose cursor is invalid or unresolvable degrade to
/// a cursor-less entry instead of erroring.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AwarenessPeerProjection {
    pub(crate) client_id: u64,
    pub(crate) clock: u32,
    pub(crate) is_local: bool,
    pub(crate) state: Value,
    pub(crate) cursor: Option<AwarenessCursorProjection>,
}

/// A peer cursor resolved to ProseMirror document positions through the
/// engine's read-only sticky-position surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AwarenessCursorProjection {
    pub(crate) anchor: u32,
    pub(crate) head: u32,
}

/// What one accepted [`CollaborationRuntime::tick`] did, and when the next
/// tick is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TickOutcome {
    /// The desired local state was re-published with a fresh clock.
    pub(crate) renewed_local: bool,
    /// Whether this tick queued an outbound awareness update.
    pub(crate) outbound_changed: bool,
    /// Remote clients expired by this tick, in ascending client order.
    pub(crate) expired_peers: Vec<u64>,
    /// Whether this tick changed a local or remote peer projection.
    pub(crate) peers_changed: bool,
    /// The next renewal/expiry deadline for the native host to schedule.
    pub(crate) next_deadline_millis: Option<u64>,
}

/// The engine codec ceilings mirrored from the session's validated
/// collaboration limits (same field names by design).
pub(crate) fn awareness_limits(limits: &CollaborationLimits) -> AwarenessLimits {
    AwarenessLimits {
        max_awareness_peers: limits.max_awareness_peers,
        max_awareness_peer_bytes: limits.max_awareness_peer_bytes,
        max_awareness_bytes: limits.max_awareness_bytes,
    }
}

/// Resolve the sticky cursor carried by a peer state (`state.cursor.anchor`
/// / `state.cursor.head`, the sticky-position wire form the legacy
/// collaboration surface established). Any missing, malformed, or
/// unresolvable point degrades the whole cursor to `None`.
fn peer_cursor_projection(
    engine: &YrsDocumentEngine,
    state: &Value,
) -> Option<AwarenessCursorProjection> {
    let cursor = state.as_object()?.get("cursor")?.as_object()?;
    let anchor = engine.resolve_awareness_sticky_doc_pos(cursor.get("anchor")?)?;
    let head = engine.resolve_awareness_sticky_doc_pos(cursor.get("head")?)?;
    Some(AwarenessCursorProjection { anchor, head })
}

fn engine_error_with_request(error: YrsEngineError, request_id: u64) -> SessionError {
    let mut error = if error.code == AWARENESS_CLOCK_EXHAUSTED {
        SessionError::transport(error)
    } else {
        SessionError::from(error)
    };
    error.request_id = Some(request_id);
    error
}

fn time_regression_error(request_id: u64, now_millis: u64, last_now_millis: u64) -> SessionError {
    let mut error = SessionError::new(
        ErrorDomain::Transport,
        AWARENESS_TIME_REGRESSION,
        "awareness tick nowMillis must not decrease",
    );
    error.request_id = Some(request_id);
    error.details = Some(serde_json::json!({
        "nowMillis": decimal_u64(now_millis),
        "lastNowMillis": decimal_u64(last_now_millis),
    }));
    error
}

/// Outbox refusals for awareness broadcasts never close a generation — the
/// desired state is already retained and the deterministic renewal clock
/// re-attempts the broadcast — so they surface as plain retryable transport
/// errors, split per the saturation ruling.
fn broadcast_reservation_error(error: OutboxReservationError, request_id: u64) -> SessionError {
    let mut session_error = match error {
        OutboxReservationError::Saturated {
            field,
            limit,
            actual,
        } => {
            let mut session_error = SessionError::new(
                ErrorDomain::Transport,
                TRANSPORT_REPLY_LIMIT_EXCEEDED,
                format!("{field} exceeded while enqueueing an awareness broadcast"),
            );
            session_error.limit = Some(limit as u64);
            session_error.actual = Some(actual as u64);
            session_error.details = Some(serde_json::json!({
                "action": AWARENESS_ACTION,
                "field": field,
                "limit": limit,
                "actual": actual,
            }));
            session_error
        }
        OutboxReservationError::Allocation => SessionError::new(
            ErrorDomain::Transport,
            TRANSPORT_RESOURCE_EXHAUSTED,
            "awareness broadcast capacity could not be reserved",
        ),
    };
    session_error.request_id = Some(request_id);
    session_error
}

impl CollaborationRuntime {
    /// Validates the monotonic clock before a stateful transport callback
    /// mutates its generation, outbox, or awareness projection. Stale
    /// generation admission runs before this check at the session boundary.
    pub(crate) fn check_now_millis(
        &self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<(), SessionError> {
        if now_millis < self.awareness.now_millis {
            return Err(time_regression_error(
                request_id,
                now_millis,
                self.awareness.now_millis,
            ));
        }
        Ok(())
    }

    /// Test-only raw-state fixture seam: validated JSON, bounded by the
    /// per-peer and aggregate awareness byte ceilings at set time
    /// (atomic rejection through the codec), retained across every transport
    /// teardown, and broadcast immediately while `Synchronized`. A broadcast
    /// reservation refusal keeps the state set — the renewal clock heals the
    /// missed broadcast — and reports the retryable error.
    #[cfg(test)]
    pub(crate) fn set_desired_awareness_for_test(
        &mut self,
        request_id: u64,
        state_json: &str,
        context: AwarenessContext<'_>,
    ) -> Result<(), SessionError> {
        let AwarenessContext {
            engine,
            transport_state,
            limits,
        } = context;
        let value: Value = serde_json::from_str(state_json).map_err(|error| {
            awareness_state_invalid(
                request_id,
                format!("desired awareness state is not valid JSON: {error}"),
            )
        })?;
        self.set_desired_awareness_value(request_id, value, engine, transport_state, limits)
    }

    /// Validates a closed caller intent before building the Rust-owned
    /// published state. Every fallible operation happens before the codec,
    /// runtime desired state, and outbox can be mutated.
    pub(crate) fn set_awareness_intent(
        &mut self,
        request_id: u64,
        intent_json: &str,
        context: AwarenessContext<'_>,
    ) -> Result<(), SessionError> {
        let AwarenessContext {
            engine,
            transport_state,
            limits,
        } = context;
        // Bound the untrusted serialized envelope before serde_json can
        // deserialize it or recurse through its state. The codec keeps the
        // exact check on the final published state payload below, including
        // any Rust-owned sticky cursor expansion.
        if intent_json.len() > limits.max_awareness_peer_bytes {
            return Err(awareness_peer_bytes_limit_error(
                request_id,
                limits.max_awareness_peer_bytes,
                intent_json.len(),
            ));
        }
        let intent: LocalAwarenessIntent = serde_json::from_str(intent_json).map_err(|error| {
            awareness_state_invalid(
                request_id,
                format!("local awareness intent is invalid: {error}"),
            )
        })?;
        // Published at the awareness root, per the y-protocols convention.
        let Value::Object(mut published) = intent.state else {
            return Err(awareness_state_invalid(
                request_id,
                "local awareness intent state must be an object",
            ));
        };
        if published
            .iter()
            .any(|(key, value)| key == "cursor" || contains_reserved_cursor(value))
        {
            return Err(awareness_state_invalid(
                request_id,
                "local awareness intent must not contain the reserved cursor key",
            ));
        }
        if published.contains_key("focused") {
            return Err(awareness_state_invalid(
                request_id,
                "local awareness intent must not contain the reserved focused key",
            ));
        }
        let cursor = match intent.selection {
            LocalAwarenessSelectionWire::Present(LocalAwarenessSelection::Text {
                anchor,
                head,
            }) => engine
                .awareness_sticky_cursor(anchor, head)
                .ok_or_else(|| {
                    awareness_state_invalid(
                        request_id,
                        "local awareness selection is outside the current document",
                    )
                })?,
            LocalAwarenessSelectionWire::Clear => Value::Null,
            // Sticky indices survive every document mutation, so the cursor
            // already published stays correct without being restated.
            LocalAwarenessSelectionWire::Retain => self
                .awareness
                .desired_state
                .as_ref()
                .and_then(|state| state.get("cursor"))
                .cloned()
                .unwrap_or(Value::Null),
        };
        published.insert("focused".into(), Value::Bool(intent.focused));
        if !cursor.is_null() {
            published.insert("cursor".into(), cursor);
        }
        self.set_desired_awareness_value(
            request_id,
            Value::Object(published),
            engine,
            transport_state,
            limits,
        )
    }

    pub(crate) fn set_awareness_selection(
        &mut self,
        request_id: u64,
        selection_json: &str,
        context: AwarenessContext<'_>,
    ) -> Result<AwarenessSelectionOutcome, SessionError> {
        let selection: LocalAwarenessSelection =
            serde_json::from_str(selection_json).map_err(|error| {
                awareness_state_invalid(
                    request_id,
                    format!("local awareness selection is invalid: {error}"),
                )
            })?;
        let AwarenessContext {
            engine,
            transport_state,
            limits,
        } = context;
        let Some(Value::Object(current)) = self.awareness.desired_state.as_ref() else {
            return Ok(AwarenessSelectionOutcome {
                outbound_changed: false,
            });
        };
        let LocalAwarenessSelection::Text { anchor, head } = selection;
        let cursor = engine
            .awareness_sticky_cursor(anchor, head)
            .ok_or_else(|| {
                awareness_state_invalid(
                    request_id,
                    "local awareness selection is outside the current document",
                )
            })?;
        if current.get("cursor") == Some(&cursor) {
            return Ok(AwarenessSelectionOutcome {
                outbound_changed: false,
            });
        }
        let mut next = current.clone();
        next.insert("cursor".into(), cursor);
        self.set_desired_awareness_value(
            request_id,
            Value::Object(next),
            engine,
            transport_state,
            limits,
        )?;
        Ok(AwarenessSelectionOutcome {
            outbound_changed: transport_state == TransportState::Synchronized,
        })
    }

    fn set_desired_awareness_value(
        &mut self,
        request_id: u64,
        value: Value,
        engine: &mut YrsDocumentEngine,
        transport_state: TransportState,
        limits: &CollaborationLimits,
    ) -> Result<(), SessionError> {
        engine
            .awareness()
            .set_local_state(&value, &awareness_limits(limits))
            .map_err(|error| engine_error_with_request(error, request_id))?;
        self.awareness.desired_state = Some(value);
        self.awareness.pending_withdrawal = None;
        if transport_state == TransportState::Synchronized {
            self.broadcast_local_update(request_id, engine)?;
        }
        Ok(())
    }

    /// Withdraws the desired local awareness: the codec creates one local
    /// tombstone and the removal is broadcast while `Synchronized`. A
    /// refused reservation retains the exact framed tombstone for tick
    /// retry; repeated clears remain no-ops and cannot bump its clock.
    pub(crate) fn clear_desired_awareness(
        &mut self,
        request_id: u64,
        context: AwarenessContext<'_>,
    ) -> Result<(), SessionError> {
        let AwarenessContext {
            engine,
            transport_state,
            ..
        } = context;
        if self.awareness.desired_state.is_none() {
            return Ok(());
        }
        engine
            .awareness()
            .clear_local_state()
            .map_err(|error| engine_error_with_request(error, request_id))?;
        self.awareness.desired_state = None;
        self.awareness.last_local_publish_millis = None;
        let message = frame_awareness_message(
            &engine
                .awareness()
                .encode_local_update_v1()
                .map_err(|error| engine_error_with_request(error, request_id))?,
        );
        if transport_state == TransportState::Synchronized {
            if let Err(error) = self.enqueue_awareness_broadcast(request_id, message.clone()) {
                self.awareness.pending_withdrawal =
                    Some(PendingWithdrawal::new(message, self.awareness.now_millis));
                return Err(error);
            }
        } else {
            self.awareness.pending_withdrawal =
                Some(PendingWithdrawal::new(message, self.awareness.now_millis));
        }
        Ok(())
    }

    /// The retained desired local awareness state, if any.
    pub(crate) fn desired_awareness(&self) -> Option<&Value> {
        self.awareness.desired_state.as_ref()
    }

    /// Public peer projections: every live awareness entry (local included,
    /// flagged) with its resolved cursor, recomputed against the current
    /// document on every read so projections follow every revision without
    /// an awareness re-receive. Tombstones are excluded by the codec.
    pub(crate) fn peers(&self, engine: &mut YrsDocumentEngine) -> Vec<AwarenessPeerProjection> {
        let snapshot = engine.awareness().peer_snapshot();
        snapshot
            .into_iter()
            .map(|peer| {
                let cursor = peer_cursor_projection(engine, &peer.state);
                AwarenessPeerProjection {
                    client_id: peer.client_id,
                    clock: peer.clock,
                    is_local: peer.is_local,
                    state: peer.state,
                    cursor,
                }
            })
            .collect()
    }

    /// Transport-scoped teardown on generation close, detach, and reattach:
    /// remote peers (and their activity deadlines) are cleared, the local
    /// entry is tombstoned by the codec with its clock preserved for the
    /// reconnect re-publish, and the desired state is retained.
    pub(crate) fn clear_transport_peers(&mut self, engine: &mut YrsDocumentEngine) -> bool {
        let peers_changed = !engine.awareness().peer_snapshot().is_empty();
        // Live local publications always retain one checked tombstone clock,
        // and local-client remote echoes never mutate it. Exhaustion is thus
        // unreachable in production cleanup; the guarded failure arm keeps
        // injected/corrupt state atomic and non-panicking.
        if engine.awareness().clear_transport_states().is_ok() {
            self.awareness.peer_activity.clear();
        }
        peers_changed
    }

    /// Applies one inbound awareness update payload through the codec and
    /// stamps deterministic activity deadlines from what it actually
    /// touched. Local-client entries never enter the activity map.
    pub(crate) fn apply_awareness_frame(
        &mut self,
        engine: &mut YrsDocumentEngine,
        limits: &CollaborationLimits,
        payload: &[u8],
    ) -> Result<(), YrsEngineError> {
        let applied = engine
            .awareness()
            .apply_remote_update_v1(payload, &awareness_limits(limits))?;
        let local_client = engine.awareness().client_id();
        let now_millis = self.awareness.now_millis;
        for client in applied.touched_clients {
            if client != local_client {
                self.awareness.peer_activity.insert(client, now_millis);
            }
        }
        for client in applied.removed_clients {
            self.awareness.peer_activity.remove(&client);
        }
        Ok(())
    }

    /// Prebuilds the handshake-completion re-publish frame: the desired
    /// state (if any) is re-set through the codec for a fresh clock and the
    /// framed local update is returned for Step-1-idiom reservation. The
    /// fresh clock strictly exceeds any removal tombstone a peer holds.
    pub(crate) fn prepare_handshake_republish(
        &mut self,
        engine: &mut YrsDocumentEngine,
        limits: &CollaborationLimits,
    ) -> Result<Option<Vec<u8>>, YrsEngineError> {
        let Some(desired) = self.awareness.desired_state.clone() else {
            return Ok(None);
        };
        engine
            .awareness()
            .set_local_state(&desired, &awareness_limits(limits))?;
        Ok(Some(frame_awareness_message(
            &engine.awareness().encode_local_update_v1()?,
        )))
    }

    /// Records that the desired state reached the outbox (immediate
    /// broadcast, handshake re-publish, or renewal): the renewal clock
    /// restarts from the current deterministic time.
    pub(crate) fn mark_local_awareness_published(&mut self) {
        self.awareness.last_local_publish_millis = Some(self.awareness.now_millis);
    }

    /// Deterministic clock work. `now_millis` is the ONLY time source of the
    /// awareness layer: remote peers whose last activity is at least
    /// [`AWARENESS_EXPIRY_MILLIS`] old are expired (standard removal
    /// tombstones, so they leave `peers()` and every query answer), and a
    /// synchronized transport re-publishes the desired state once at least
    /// [`AWARENESS_RENEWAL_INTERVAL_MILLIS`] have elapsed since the last
    /// broadcast. An already-clocked withdrawal is retried first when due;
    /// reservation refusal preserves its exact frame and defers the next
    /// attempt by one renewal interval to avoid a zero-delay retry loop.
    /// Expiry is idempotent, so a retried tick converges.
    pub(crate) fn tick(
        &mut self,
        request_id: u64,
        now_millis: u64,
        context: AwarenessContext<'_>,
    ) -> Result<TickOutcome, SessionError> {
        let AwarenessContext {
            engine,
            transport_state,
            limits,
        } = context;
        self.check_now_millis(request_id, now_millis)?;
        self.awareness.now_millis = now_millis;

        let expired_peers: Vec<u64> = self
            .awareness
            .peer_activity
            .iter()
            .filter(|(_, seen)| now_millis.saturating_sub(**seen) >= AWARENESS_EXPIRY_MILLIS)
            .map(|(client, _)| *client)
            .collect();
        for client in &expired_peers {
            engine.awareness().remove_remote_state(*client);
            self.awareness.peer_activity.remove(client);
        }

        let mut renewed_local = false;
        let mut withdrawal_enqueued = false;
        if transport_state == TransportState::Synchronized {
            let withdrawal_due = self
                .awareness
                .pending_withdrawal
                .as_ref()
                .is_some_and(|pending| pending.is_due(now_millis));
            if withdrawal_due {
                let frame = self
                    .awareness
                    .pending_withdrawal
                    .as_ref()
                    .expect("a due withdrawal remains pending")
                    .frame
                    .clone();
                if let Err(error) = self.enqueue_awareness_broadcast(request_id, frame) {
                    self.awareness
                        .pending_withdrawal
                        .as_mut()
                        .expect("a refused withdrawal remains pending")
                        .defer_retry(now_millis);
                    return Err(error);
                }
                self.awareness.pending_withdrawal = None;
                withdrawal_enqueued = true;
            } else if let Some(desired) = self.awareness.desired_state.clone() {
                let renewal_due =
                    self.awareness
                        .last_local_publish_millis
                        .is_none_or(|published| {
                            now_millis.saturating_sub(published)
                                >= AWARENESS_RENEWAL_INTERVAL_MILLIS
                        });
                if renewal_due {
                    engine
                        .awareness()
                        .set_local_state(&desired, &awareness_limits(limits))
                        .map_err(|error| engine_error_with_request(error, request_id))?;
                    self.broadcast_local_update(request_id, engine)?;
                    renewed_local = true;
                }
            }
        }

        Ok(TickOutcome {
            renewed_local,
            outbound_changed: renewed_local || withdrawal_enqueued,
            peers_changed: renewed_local || !expired_peers.is_empty(),
            expired_peers,
            next_deadline_millis: self.awareness.next_deadline_millis(transport_state),
        })
    }

    /// Frames and enqueues the codec's current local update, then restarts
    /// the renewal clock.
    fn broadcast_local_update(
        &mut self,
        request_id: u64,
        engine: &mut YrsDocumentEngine,
    ) -> Result<(), SessionError> {
        let message = frame_awareness_message(
            &engine
                .awareness()
                .encode_local_update_v1()
                .map_err(|error| engine_error_with_request(error, request_id))?,
        );
        self.enqueue_awareness_broadcast(request_id, message)?;
        self.mark_local_awareness_published();
        Ok(())
    }

    /// Reserve-then-install one framed awareness broadcast after any document
    /// update it may reference.
    fn enqueue_awareness_broadcast(
        &mut self,
        request_id: u64,
        message: Vec<u8>,
    ) -> Result<(), SessionError> {
        let reservation = self
            .outbox
            .reserve_awareness_broadcast(message.len())
            .map_err(|error| broadcast_reservation_error(error, request_id))?;
        self.outbox
            .install_awareness_broadcast(reservation, request_id, message);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const REQUEST_ID: u64 = 88;

    fn runtime() -> CollaborationRuntime {
        CollaborationRuntime::new(&CollaborationLimits::default())
    }

    fn engine() -> YrsDocumentEngine {
        use crate::boundary::ResourceLimits;
        use crate::yrs_engine::{EditingLimits, InitializationMode, YrsEngineConfig};
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: crate::schema::presets::tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap()
    }

    fn context<'a>(
        engine: &'a mut YrsDocumentEngine,
        transport_state: TransportState,
        limits: &'a CollaborationLimits,
    ) -> AwarenessContext<'a> {
        AwarenessContext {
            engine,
            transport_state,
            limits,
        }
    }

    #[test]
    fn awareness_limits_mirror_the_session_collaboration_limit_fields() {
        let limits = CollaborationLimits {
            max_awareness_peers: 3,
            max_awareness_peer_bytes: 64,
            max_awareness_bytes: 256,
            ..CollaborationLimits::default()
        };
        assert_eq!(
            awareness_limits(&limits),
            AwarenessLimits {
                max_awareness_peers: 3,
                max_awareness_peer_bytes: 64,
                max_awareness_bytes: 256,
            },
        );
    }

    #[test]
    fn local_intent_peer_byte_ceiling_rejects_before_json_deserialization() {
        let mut runtime = runtime();
        let mut engine = engine();
        let limits = CollaborationLimits {
            max_awareness_peer_bytes: 64,
            ..CollaborationLimits::default()
        };
        // This is deliberately invalid JSON. If serde_json is reached, the
        // refusal changes to AWARENESS_STATE_INVALID instead of the frozen
        // maxAwarenessPeerBytes resource-limit envelope.
        let oversized_intent = "[".repeat(limits.max_awareness_peer_bytes + 1);

        let error = runtime
            .set_awareness_intent(
                REQUEST_ID,
                &oversized_intent,
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap_err();

        assert_eq!(error.domain, ErrorDomain::Boundary, "{error:?}");
        assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
        assert_eq!(error.request_id, Some(REQUEST_ID), "{error:?}");
        assert_eq!(error.limit, Some(64), "{error:?}");
        assert_eq!(error.actual, Some(65), "{error:?}");
        assert_eq!(error.message, "input exceeds limit 64: 65", "{error:?}");
        assert_eq!(
            error.details.as_ref().unwrap()["field"],
            "maxAwarenessPeerBytes",
            "{error:?}",
        );
        assert_eq!(runtime.desired_awareness(), None);
        assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
    }

    #[test]
    fn next_deadline_is_the_minimum_of_renewal_and_earliest_expiry() {
        let mut state = AwarenessRuntimeState::new();
        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            None
        );

        state.peer_activity.insert(7, 1_000);
        state.peer_activity.insert(8, 2_000);
        assert_eq!(
            state.next_deadline_millis(TransportState::Disconnected),
            Some(1_000 + AWARENESS_EXPIRY_MILLIS),
        );

        state.desired_state = Some(json!({"n": 1}));
        state.last_local_publish_millis = Some(4_000);
        // Renewal is earlier than the earliest expiry.
        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            Some(4_000 + AWARENESS_RENEWAL_INTERVAL_MILLIS),
        );
        // A disconnected transport never schedules renewal.
        assert_eq!(
            state.next_deadline_millis(TransportState::Disconnected),
            Some(1_000 + AWARENESS_EXPIRY_MILLIS),
        );
        // An unpublished desired state on a synchronized transport is due at
        // the current deterministic time.
        state.last_local_publish_millis = None;
        state.now_millis = 9_000;
        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            Some(9_000),
        );
    }

    #[test]
    fn task8_fourth_remediation_local_max_timestamp_has_no_deadline() {
        let mut state = AwarenessRuntimeState::new();
        state.now_millis = u64::MAX;
        state.desired_state = Some(json!({"name": "local"}));
        state.last_local_publish_millis = Some(u64::MAX);

        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            None,
        );
    }

    #[test]
    fn task8_fourth_remediation_remote_max_timestamp_has_no_deadline() {
        let mut state = AwarenessRuntimeState::new();
        state.now_millis = u64::MAX;
        state.peer_activity.insert(7, u64::MAX);

        assert_eq!(
            state.next_deadline_millis(TransportState::Disconnected),
            None,
        );
    }

    #[test]
    fn task8_fourth_remediation_mixed_deadlines_keep_the_representable_candidate() {
        let mut state = AwarenessRuntimeState::new();
        state.desired_state = Some(json!({"name": "local"}));
        state.last_local_publish_millis = Some(u64::MAX);
        state.peer_activity.insert(7, 1_000);
        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            Some(1_000 + AWARENESS_EXPIRY_MILLIS),
        );

        state.last_local_publish_millis = Some(2_000);
        state.peer_activity.clear();
        state.peer_activity.insert(7, u64::MAX);
        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            Some(2_000 + AWARENESS_RENEWAL_INTERVAL_MILLIS),
        );
    }

    #[test]
    fn task8_fourth_remediation_tick_at_equal_max_does_no_false_clock_work() {
        let mut runtime = runtime();
        let mut engine = engine();
        let limits = CollaborationLimits::default();
        runtime.awareness.now_millis = u64::MAX;
        runtime.awareness.desired_state = Some(json!({"name": "local"}));
        runtime.awareness.last_local_publish_millis = Some(u64::MAX);
        runtime.awareness.peer_activity.insert(7, u64::MAX);

        let outcome = runtime
            .tick(
                REQUEST_ID,
                u64::MAX,
                context(&mut engine, TransportState::Synchronized, &limits),
            )
            .unwrap();

        assert!(!outcome.renewed_local, "{outcome:?}");
        assert!(!outcome.outbound_changed, "{outcome:?}");
        assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
        assert!(!outcome.peers_changed, "{outcome:?}");
        assert_eq!(outcome.next_deadline_millis, None, "{outcome:?}");
        assert_eq!(runtime.awareness.last_local_publish_millis, Some(u64::MAX));
        assert_eq!(runtime.awareness.peer_activity.get(&7), Some(&u64::MAX));
        assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
    }

    #[test]
    fn reset_for_restore_clears_peer_bookkeeping_and_restarts_the_renewal_clock() {
        let mut state = AwarenessRuntimeState::new();
        state.now_millis = 9_000;
        state.desired_state = Some(json!({"n": 1}));
        state.last_local_publish_millis = Some(4_000);
        state.peer_activity.insert(7, 1_000);
        state.peer_activity.insert(8, 2_000);

        state.reset_for_restore();

        // The desired state survives; prior-store deadlines are gone.
        assert_eq!(state.desired_state, Some(json!({"n": 1})));
        assert!(state.peer_activity.is_empty());
        assert_eq!(state.last_local_publish_millis, None);
        // With no broadcast for the new store, a synchronized renewal is due
        // at the current deterministic time, never on the prior store's
        // schedule.
        assert_eq!(
            state.next_deadline_millis(TransportState::Synchronized),
            Some(9_000),
        );
        assert_eq!(
            state.next_deadline_millis(TransportState::Disconnected),
            None
        );
    }

    #[test]
    fn peer_cursor_projection_requires_two_resolvable_sticky_points() {
        let mut engine = engine();
        engine
            .import_json(
                r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"unit seed"}]}]}"#,
                crate::yrs_engine::TransactionOrigin::DocumentImport,
            )
            .unwrap();

        // Non-object states, missing cursors, and malformed points degrade.
        assert_eq!(peer_cursor_projection(&engine, &json!("plain")), None);
        assert_eq!(peer_cursor_projection(&engine, &json!({"name": "x"})), None);
        assert_eq!(
            peer_cursor_projection(
                &engine,
                &json!({"cursor": {"anchor": {"bogus": true}, "head": {"bogus": true}}}),
            ),
            None,
        );
        assert_eq!(
            peer_cursor_projection(&engine, &json!({"cursor": {"anchor": 1}})),
            None,
            "a cursor missing its head never resolves half a projection",
        );
    }

    #[test]
    fn broadcast_reservation_errors_split_per_the_saturation_ruling() {
        let saturated = broadcast_reservation_error(
            OutboxReservationError::Saturated {
                field: super::super::outbox::OUTBOX_MESSAGES_FIELD,
                limit: 2,
                actual: 3,
            },
            REQUEST_ID,
        );
        assert_eq!(saturated.code, TRANSPORT_REPLY_LIMIT_EXCEEDED);
        assert_eq!(saturated.domain, ErrorDomain::Transport);
        assert_eq!(saturated.request_id, Some(REQUEST_ID));
        assert_eq!(saturated.limit, Some(2));
        assert_eq!(saturated.actual, Some(3));

        let allocation =
            broadcast_reservation_error(OutboxReservationError::Allocation, REQUEST_ID);
        assert_eq!(allocation.code, TRANSPORT_RESOURCE_EXHAUSTED);
        assert_eq!(allocation.request_id, Some(REQUEST_ID));
    }

    #[test]
    fn set_desired_awareness_rejects_invalid_json_atomically() {
        let mut runtime = runtime();
        let mut engine = engine();
        let limits = CollaborationLimits::default();

        let error = runtime
            .set_desired_awareness_for_test(
                REQUEST_ID,
                "{broken",
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap_err();
        assert_eq!(error.code, AWARENESS_STATE_INVALID);
        assert_eq!(error.request_id, Some(REQUEST_ID));
        assert_eq!(runtime.desired_awareness(), None);

        runtime
            .set_desired_awareness_for_test(
                REQUEST_ID,
                r#"{"name":"kept"}"#,
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap();
        assert_eq!(runtime.desired_awareness(), Some(&json!({"name": "kept"})));
        // Disconnected transports retain without broadcasting.
        assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
    }

    #[test]
    fn tick_expires_tracked_peers_and_reports_the_next_deadline() {
        let mut runtime = runtime();
        let mut engine = engine();
        let limits = CollaborationLimits::default();

        // Install two peers through the runtime path so activity is tracked.
        runtime
            .tick(
                REQUEST_ID,
                0,
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap();
        let update = |client: u64, clock: u32| {
            use yrs::updates::encoder::Encode as _;
            let mut clients = std::collections::HashMap::new();
            clients.insert(
                yrs::ClientID::new(client),
                yrs::sync::awareness::AwarenessUpdateEntry {
                    clock,
                    json: r#"{"u":1}"#.into(),
                },
            );
            yrs::sync::awareness::AwarenessUpdate { clients }.encode_v1()
        };
        runtime
            .apply_awareness_frame(&mut engine, &limits, &update(21, 1))
            .unwrap();
        let outcome = runtime
            .tick(
                REQUEST_ID,
                10_000,
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap();
        assert_eq!(outcome.expired_peers, Vec::<u64>::new());
        runtime
            .apply_awareness_frame(&mut engine, &limits, &update(22, 1))
            .unwrap();

        // Exactly at the boundary peer 21 expires; peer 22 (seen at 10s)
        // stays and owns the next deadline.
        let outcome = runtime
            .tick(
                REQUEST_ID,
                AWARENESS_EXPIRY_MILLIS,
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap();
        assert_eq!(outcome.expired_peers, vec![21]);
        assert!(!outcome.renewed_local);
        assert_eq!(
            outcome.next_deadline_millis,
            Some(10_000 + AWARENESS_EXPIRY_MILLIS),
        );
        assert_eq!(runtime.peers(&mut engine).len(), 1);
    }

    #[test]
    fn task8_third_remediation_runtime_renewal_marks_local_peer_changed() {
        let mut runtime = runtime();
        let mut engine = engine();
        let limits = CollaborationLimits::default();

        runtime
            .set_desired_awareness_for_test(
                REQUEST_ID,
                r#"{"name":"renewed"}"#,
                context(&mut engine, TransportState::Synchronized, &limits),
            )
            .unwrap();
        let before = runtime.peers(&mut engine);

        let outcome = runtime
            .tick(
                REQUEST_ID,
                AWARENESS_RENEWAL_INTERVAL_MILLIS,
                context(&mut engine, TransportState::Synchronized, &limits),
            )
            .unwrap();
        let after = runtime.peers(&mut engine);

        assert!(outcome.renewed_local, "{outcome:?}");
        assert!(outcome.outbound_changed, "{outcome:?}");
        assert!(outcome.expired_peers.is_empty(), "{outcome:?}");
        assert!(outcome.peers_changed, "{outcome:?}");
        assert_eq!(before.len(), 1);
        assert_eq!(after.len(), 1);
        assert!(after[0].clock > before[0].clock, "{before:?} -> {after:?}");
    }

    fn assert_clock_exhausted(error: &SessionError) {
        assert_eq!(error.domain, ErrorDomain::Transport, "{error:?}");
        assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
        assert_eq!(error.request_id, Some(REQUEST_ID), "{error:?}");
        assert!(
            error.message.contains("fresh editor identity is required"),
            "{error:?}",
        );
        assert_eq!(
            error.details.as_ref().unwrap()["requiresFreshEditorIdentity"],
            true,
            "{error:?}",
        );
        assert_eq!(
            error.details.as_ref().unwrap()["retryable"],
            false,
            "{error:?}",
        );
    }

    fn runtime_with_clock(clock: u32) -> (CollaborationRuntime, YrsDocumentEngine) {
        let mut runtime = runtime();
        let mut engine = engine();
        let limits = CollaborationLimits::default();
        runtime
            .set_desired_awareness_for_test(
                REQUEST_ID,
                r#"{"name":"before"}"#,
                context(&mut engine, TransportState::Disconnected, &limits),
            )
            .unwrap();
        engine.awareness().set_live_local_clock_for_test(clock);
        (runtime, engine)
    }

    #[test]
    fn set_and_renew_report_clock_exhaustion_without_mutating_state_or_outbox() {
        let limits = CollaborationLimits::default();
        for clock in [u32::MAX - 1, u32::MAX] {
            let (mut runtime, mut engine) = runtime_with_clock(clock);
            let before_peers = runtime.peers(&mut engine);
            let before_replies = runtime.outbox().pending_protocol_reply_count();

            let error = runtime
                .set_desired_awareness_for_test(
                    REQUEST_ID,
                    r#"{"name":"after"}"#,
                    context(&mut engine, TransportState::Synchronized, &limits),
                )
                .unwrap_err();
            assert_clock_exhausted(&error);
            assert_eq!(runtime.peers(&mut engine), before_peers);
            assert_eq!(
                runtime.outbox().pending_protocol_reply_count(),
                before_replies,
            );

            runtime.awareness.last_local_publish_millis = Some(0);
            let error = runtime
                .tick(
                    REQUEST_ID,
                    AWARENESS_RENEWAL_INTERVAL_MILLIS,
                    context(&mut engine, TransportState::Synchronized, &limits),
                )
                .unwrap_err();
            assert_clock_exhausted(&error);
            assert_eq!(runtime.peers(&mut engine), before_peers);
            assert_eq!(
                runtime.outbox().pending_protocol_reply_count(),
                before_replies,
            );
        }
    }

    #[test]
    fn reconnect_publication_reports_clock_exhaustion_without_enqueuing() {
        let limits = CollaborationLimits::default();
        for clock in [u32::MAX - 1, u32::MAX] {
            let (mut runtime, mut engine) = runtime_with_clock(clock);
            let before_peers = runtime.peers(&mut engine);

            let error = runtime
                .prepare_handshake_republish(&mut engine, &limits)
                .unwrap_err();

            assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
            assert_eq!(runtime.peers(&mut engine), before_peers);
            assert_eq!(runtime.outbox().pending_protocol_reply_count(), 0);
        }
    }
}
