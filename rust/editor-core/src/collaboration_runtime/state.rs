//! Generation-owned transport state machine (production since the Task 16C
//! cutover removed the staging gate).
//!
//! Task 8 scope: the machine is the single writer of a session's transport
//! state. Every transition runs under the session lock (callers reach it
//! only through `EditorSession`, which the registry hands out via the Task 3
//! `with_alive` idiom), every accepted connection attempt increments the
//! monotonic [`TransportGeneration`] exactly once, and callbacks carrying a
//! stale generation are refused as observable no-ops. Rust alone owns retry
//! eligibility: a retryable close returns to `Disconnected` (where
//! `begin_connect` is accepted again) while deterministic incompatibility
//! parks the transport in `Incompatible`, from which JavaScript cannot force
//! a reconnect — only the explicit detach/reattach cycle reopens it.
//!
//! No protocol framing lives here: `socket_opened` only *reports* that a
//! Sync Step 1 send is owed ([`HandshakeDirective::SendSyncStep1`], built in
//! Task 9), and `mark_synchronized` is the crate-private seam Task 9 drives
//! from an accepted current-generation Step 2.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use crate::session::{ErrorDomain, SessionError, TransportState};

/// Refusal code for callbacks whose generation is not the live attempt
/// (superseded, closed, fabricated, or never issued).
pub const TRANSPORT_STALE_GENERATION: &str = "TRANSPORT_STALE_GENERATION";
/// Refusal code for actions that are not legal transitions from the current
/// transport state; JavaScript retry timers stop when they receive it.
pub const TRANSPORT_INVALID_TRANSITION: &str = "TRANSPORT_INVALID_TRANSITION";
/// Refusal code for `begin_connect` while deterministically incompatible;
/// only configuration/server changes or detach/reattach reopen the row.
pub const TRANSPORT_INCOMPATIBLE: &str = "TRANSPORT_INCOMPATIBLE";
/// Refusal code for connection-shaped actions on a local-only session that
/// has no room binding to connect to.
pub const TRANSPORT_NOT_ROOM_BOUND: &str = "TRANSPORT_NOT_ROOM_BOUND";

/// Generation value before any connection attempt; the first accepted
/// `begin_connect` issues `INITIAL_TRANSPORT_GENERATION + 1`.
const INITIAL_TRANSPORT_GENERATION: u64 = 0;

/// Monotonic identity of one connection attempt. Issued only by
/// [`TransportStateMachine::begin_connect`]; JavaScript carries the raw
/// value through its socket callbacks and the boundary rebuilds it with
/// [`TransportGeneration::from_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportGeneration(u64);

impl TransportGeneration {
    /// Raw value handed across the boundary (and asserted in tests).
    pub fn value(self) -> u64 {
        self.0
    }

    /// Rebuild a generation from the raw value a callback carried back.
    /// A value that was never issued simply never matches the live attempt.
    pub(crate) fn from_value(value: u64) -> Self {
        Self(value)
    }
}

/// What the caller owes after an accepted transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeDirective {
    /// The opened socket must immediately send Sync Step 1 (framed in
    /// Task 9). Socket open means `Handshaking`, never `Synchronized`.
    SendSyncStep1,
}

/// Rust-owned disposition of a socket close, decided by the caller's error
/// classification (Task 9) or reported transparently from JavaScript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketCloseDisposition {
    /// The close is retryable: the transport returns to `Disconnected`,
    /// where a new `begin_connect` is accepted.
    Retryable,
    /// Deterministic incompatibility: retrying cannot change the result, so
    /// the transport parks in `Incompatible`.
    Incompatible,
}

/// Session-owned transport state machine: the sole writer of the transport
/// state and the sole issuer of generations. Refusals are structured
/// transport-domain [`SessionError`]s that leave both the state and the
/// live attempt untouched.
#[derive(Debug)]
pub(crate) struct TransportStateMachine {
    state: TransportState,
    /// The generation of the live attempt while `Connecting`/`Handshaking`/
    /// `Synchronized`; `None` once that attempt is closed or none started.
    live_attempt: Option<TransportGeneration>,
    last_issued: u64,
}

impl TransportStateMachine {
    pub(crate) fn new(initial: TransportState) -> Self {
        Self {
            state: initial,
            live_attempt: None,
            last_issued: INITIAL_TRANSPORT_GENERATION,
        }
    }

    pub(crate) fn state(&self) -> TransportState {
        self.state
    }

    /// `Disconnected` -> `Connecting` on a room-bound session; issues the
    /// generation JavaScript must carry through its socket callbacks.
    /// Refused (without consuming a generation) while `Detached` (reattach
    /// first), while an attempt is live, and — with its dedicated code —
    /// while `Incompatible`, so a JS retry timer can never override
    /// deterministic incompatibility.
    pub(crate) fn begin_connect(
        &mut self,
        request_id: u64,
        room_bound: bool,
    ) -> Result<TransportGeneration, SessionError> {
        if !room_bound {
            return Err(not_room_bound(request_id, "beginConnect", self.state));
        }
        match self.state {
            TransportState::Disconnected => {
                self.last_issued += 1;
                let generation = TransportGeneration(self.last_issued);
                self.live_attempt = Some(generation);
                self.state = TransportState::Connecting;
                Ok(generation)
            }
            TransportState::Incompatible => Err(SessionError::new(
                ErrorDomain::Transport,
                TRANSPORT_INCOMPATIBLE,
                "transport is deterministically incompatible; reconnect requires \
                 detach and reattach",
            )
            .with_transport_context(request_id, "beginConnect", self.state)),
            _ => Err(invalid_transition(request_id, "beginConnect", self.state)),
        }
    }

    /// Current `Connecting` -> `Handshaking`; the caller owes a Sync Step 1
    /// send. Stale generations are observable no-ops.
    pub(crate) fn socket_opened(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<HandshakeDirective, SessionError> {
        self.require_live_attempt(request_id, "socketOpened", generation)?;
        if self.state != TransportState::Connecting {
            return Err(invalid_transition(request_id, "socketOpened", self.state));
        }
        self.state = TransportState::Handshaking;
        Ok(HandshakeDirective::SendSyncStep1)
    }

    /// Current-generation close: the live attempt ends and the disposition
    /// decides `Disconnected` (retry eligible) or `Incompatible`. Stale
    /// generations are observable no-ops that can neither advance nor
    /// regress the transport.
    pub(crate) fn socket_closed(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
        disposition: SocketCloseDisposition,
    ) -> Result<TransportState, SessionError> {
        self.require_live_attempt(request_id, "socketClosed", generation)?;
        self.live_attempt = None;
        self.state = match disposition {
            SocketCloseDisposition::Retryable => TransportState::Disconnected,
            SocketCloseDisposition::Incompatible => TransportState::Incompatible,
        };
        Ok(self.state)
    }

    /// Local intent: close the live attempt -> `Disconnected`; the closed
    /// generation's late callbacks become stale and retries stop until the
    /// caller asks to connect again.
    pub(crate) fn disconnect(&mut self, request_id: u64) -> Result<(), SessionError> {
        if !matches!(
            self.state,
            TransportState::Connecting | TransportState::Handshaking | TransportState::Synchronized
        ) {
            return Err(invalid_transition(request_id, "disconnect", self.state));
        }
        self.live_attempt = None;
        self.state = TransportState::Disconnected;
        Ok(())
    }

    /// Any live transport state except `Detached` -> `Detached`. Tears down
    /// only transport state: the runtime, its outbox, and the document are
    /// untouched by design.
    pub(crate) fn detach(&mut self, request_id: u64) -> Result<(), SessionError> {
        if self.state == TransportState::Detached {
            return Err(invalid_transition(request_id, "detach", self.state));
        }
        self.live_attempt = None;
        self.state = TransportState::Detached;
        Ok(())
    }

    /// The explicit reattach half of the `Incompatible` escape hatch:
    /// `Detached` -> `Disconnected` on a room-bound session, after which
    /// `begin_connect` is accepted again.
    pub(crate) fn reattach(
        &mut self,
        request_id: u64,
        room_bound: bool,
    ) -> Result<(), SessionError> {
        if !room_bound {
            return Err(not_room_bound(request_id, "reattach", self.state));
        }
        if self.state != TransportState::Detached {
            return Err(invalid_transition(request_id, "reattach", self.state));
        }
        self.state = TransportState::Disconnected;
        Ok(())
    }

    /// Crate-private Task 9 seam: an accepted current-generation Sync
    /// Step 2 turns `Handshaking` into `Synchronized`. Generation-checked
    /// like every callback; the attempt stays live so its close callbacks
    /// remain current. Never touches document state.
    pub(crate) fn mark_synchronized(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<(), SessionError> {
        self.require_live_attempt(request_id, "markSynchronized", generation)?;
        if self.state != TransportState::Handshaking {
            return Err(invalid_transition(
                request_id,
                "markSynchronized",
                self.state,
            ));
        }
        self.state = TransportState::Synchronized;
        Ok(())
    }

    /// Task 9 read-only frame admission gate, checked before ANY decode
    /// work: a frame is processed only when it carries the live generation
    /// and the transport is `Handshaking` or `Synchronized`. Never
    /// transitions — closing a generation on protocol failure goes through
    /// [`Self::socket_closed`] with a Rust-owned disposition.
    pub(crate) fn admit_receive(
        &self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<TransportState, SessionError> {
        self.require_live_attempt(request_id, "receiveMessage", generation)?;
        match self.state {
            TransportState::Handshaking | TransportState::Synchronized => Ok(self.state),
            state => Err(invalid_transition(request_id, "receiveMessage", state)),
        }
    }

    /// Task 12 read-only outbound-pickup admission gate: draining the next
    /// outbound frame is generation-scoped wire work with the same
    /// admission law as [`Self::admit_receive`] (live generation while
    /// `Handshaking`/`Synchronized`), under its own action label.
    pub(crate) fn admit_outbound_take(
        &self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<TransportState, SessionError> {
        self.require_live_attempt(request_id, "takeOutbound", generation)?;
        match self.state {
            TransportState::Handshaking | TransportState::Synchronized => Ok(self.state),
            state => Err(invalid_transition(request_id, "takeOutbound", state)),
        }
    }

    /// Task 11 snapshot-restore settle: `Detached`/`Disconnected` ->
    /// `Disconnected`. The session restore gate admits only those two
    /// states, so no attempt can be live here; this is the designed
    /// "sync-generation state cleared" write — the transport returns to the
    /// room's disconnected row, where `begin_connect` is accepted again.
    /// `last_issued` deliberately stays monotonic: resetting it would
    /// reissue generation values and revive stale callbacks. Infallible —
    /// restore runs it only after the engine candidate installed.
    pub(crate) fn settle_for_restore(&mut self) {
        debug_assert!(
            matches!(
                self.state,
                TransportState::Detached | TransportState::Disconnected
            ),
            "the session restore gate admits only Detached/Disconnected transports, \
             found {:?}",
            self.state,
        );
        self.live_attempt = None;
        self.state = TransportState::Disconnected;
    }

    /// Lifecycle teardown writer used by session destroy: transport state
    /// becomes `Destroyed` and any live attempt is retired. Not a
    /// request-scoped transition — destroy wins from every state.
    pub(crate) fn teardown_destroyed(&mut self) {
        self.live_attempt = None;
        self.state = TransportState::Destroyed;
    }

    /// Test-only state injection for policy-matrix cells that are
    /// unreachable through real transitions (see
    /// `EditorSession::set_transport_state_for_test`). Forced states carry
    /// no live attempt, so every generation-carrying callback on them is
    /// stale by construction and the machine's invariants hold.
    pub(crate) fn set_state_for_test(&mut self, state: TransportState) {
        self.live_attempt = None;
        self.state = state;
    }

    fn require_live_attempt(
        &self,
        request_id: u64,
        action: &'static str,
        generation: TransportGeneration,
    ) -> Result<(), SessionError> {
        if self.live_attempt == Some(generation) {
            return Ok(());
        }
        Err(SessionError::new(
            ErrorDomain::Transport,
            TRANSPORT_STALE_GENERATION,
            "callback generation is not the live connection attempt",
        )
        .with_transport_context(request_id, action, self.state)
        .with_transport_generations(generation, self.live_attempt))
    }
}

fn invalid_transition(
    request_id: u64,
    action: &'static str,
    state: TransportState,
) -> SessionError {
    SessionError::new(
        ErrorDomain::Transport,
        TRANSPORT_INVALID_TRANSITION,
        format!(
            "{action} is not a legal transition while the transport is {}",
            state.as_str()
        ),
    )
    .with_transport_context(request_id, action, state)
}

fn not_room_bound(request_id: u64, action: &'static str, state: TransportState) -> SessionError {
    SessionError::new(
        ErrorDomain::Transport,
        TRANSPORT_NOT_ROOM_BOUND,
        "local-only sessions have no room binding to connect to",
    )
    .with_transport_context(request_id, action, state)
}

impl SessionError {
    fn with_transport_context(
        mut self,
        request_id: u64,
        action: &'static str,
        state: TransportState,
    ) -> Self {
        self.request_id = Some(request_id);
        let details = self.details.get_or_insert_with(|| serde_json::json!({}));
        details["action"] = serde_json::Value::String(action.into());
        details["transportState"] = serde_json::Value::String(state.as_str().into());
        self
    }

    fn with_transport_generations(
        mut self,
        presented: TransportGeneration,
        live: Option<TransportGeneration>,
    ) -> Self {
        let details = self.details.get_or_insert_with(|| serde_json::json!({}));
        details["presentedGeneration"] = serde_json::json!(presented.value());
        details["liveGeneration"] = match live {
            Some(generation) => serde_json::json!(generation.value()),
            None => serde_json::Value::Null,
        };
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM_BOUND: bool = true;
    const LOCAL_ONLY: bool = false;
    const REQUEST_ID: u64 = 42;

    fn connected_machine() -> (TransportStateMachine, TransportGeneration) {
        let mut machine = TransportStateMachine::new(TransportState::Disconnected);
        let generation = machine.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap();
        (machine, generation)
    }

    #[test]
    fn new_machine_reflects_the_initial_state_with_no_live_attempt() {
        let machine = TransportStateMachine::new(TransportState::Detached);
        assert_eq!(machine.state(), TransportState::Detached);
        assert!(machine.live_attempt.is_none());
        assert_eq!(machine.last_issued, INITIAL_TRANSPORT_GENERATION);
    }

    #[test]
    fn begin_connect_issues_monotonic_generations_and_refusals_never_consume_one() {
        let (mut machine, first) = connected_machine();
        assert_eq!(machine.state(), TransportState::Connecting);
        assert_eq!(first.value(), INITIAL_TRANSPORT_GENERATION + 1);

        // Refused while an attempt is live: no state change, no generation.
        let error = machine.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        assert_eq!(error.request_id, Some(REQUEST_ID));
        assert_eq!(machine.state(), TransportState::Connecting);

        machine
            .socket_closed(REQUEST_ID, first, SocketCloseDisposition::Retryable)
            .unwrap();
        let second = machine.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap();
        assert_eq!(second.value(), first.value() + 1);
    }

    #[test]
    fn begin_connect_refuses_local_only_detached_and_incompatible_rows() {
        let mut local = TransportStateMachine::new(TransportState::Disconnected);
        let error = local.begin_connect(REQUEST_ID, LOCAL_ONLY).unwrap_err();
        assert_eq!(error.code, TRANSPORT_NOT_ROOM_BOUND, "{error:?}");
        assert_eq!(local.state(), TransportState::Disconnected);
        // The refusal details report the machine's actual state, not a
        // hardcoded one.
        let details = error.details.expect("refusal must carry details");
        assert_eq!(details["action"], "beginConnect");
        assert_eq!(details["transportState"], "Disconnected");

        let mut detached = TransportStateMachine::new(TransportState::Detached);
        let error = detached.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");

        let (mut machine, generation) = connected_machine();
        machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Incompatible)
            .unwrap();
        let error = machine.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INCOMPATIBLE, "{error:?}");
        assert_eq!(machine.state(), TransportState::Incompatible);
    }

    #[test]
    fn socket_opened_means_handshaking_only_for_the_live_generation() {
        let (mut machine, generation) = connected_machine();
        let stale = TransportGeneration::from_value(generation.value() + 100);
        let error = machine.socket_opened(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        assert_eq!(machine.state(), TransportState::Connecting);

        assert_eq!(
            machine.socket_opened(REQUEST_ID, generation).unwrap(),
            HandshakeDirective::SendSyncStep1,
        );
        assert_eq!(machine.state(), TransportState::Handshaking);

        // A duplicate open of the live generation is a wrong-state refusal.
        let error = machine.socket_opened(REQUEST_ID, generation).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        assert_eq!(machine.state(), TransportState::Handshaking);
    }

    #[test]
    fn socket_closed_applies_the_rust_owned_disposition_and_retires_the_attempt() {
        let (mut machine, generation) = connected_machine();
        assert_eq!(
            machine
                .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable)
                .unwrap(),
            TransportState::Disconnected,
        );
        // The retired generation is stale for every later callback.
        let error = machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Incompatible)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        assert_eq!(machine.state(), TransportState::Disconnected);

        let (mut machine, generation) = connected_machine();
        machine.socket_opened(REQUEST_ID, generation).unwrap();
        machine.mark_synchronized(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine
                .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Incompatible)
                .unwrap(),
            TransportState::Incompatible,
        );
    }

    #[test]
    fn disconnect_retires_the_live_attempt_and_refuses_when_none_exists() {
        let (mut machine, generation) = connected_machine();
        machine.disconnect(REQUEST_ID).unwrap();
        assert_eq!(machine.state(), TransportState::Disconnected);
        let error = machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        let error = machine.disconnect(REQUEST_ID).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
    }

    #[test]
    fn detach_and_reattach_form_the_incompatible_escape_hatch() {
        let (mut machine, generation) = connected_machine();
        machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Incompatible)
            .unwrap();
        machine.detach(REQUEST_ID).unwrap();
        assert_eq!(machine.state(), TransportState::Detached);
        let error = machine.detach(REQUEST_ID).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");

        let error = machine.reattach(REQUEST_ID, LOCAL_ONLY).unwrap_err();
        assert_eq!(error.code, TRANSPORT_NOT_ROOM_BOUND, "{error:?}");
        machine.reattach(REQUEST_ID, ROOM_BOUND).unwrap();
        assert_eq!(machine.state(), TransportState::Disconnected);
        let error = machine.reattach(REQUEST_ID, ROOM_BOUND).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");

        let next = machine.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap();
        assert_eq!(next.value(), generation.value() + 1);
    }

    #[test]
    fn mark_synchronized_requires_the_live_generation_in_handshaking() {
        let (mut machine, generation) = connected_machine();
        // Step 2 before socket open is a wrong-state refusal, not stale.
        let error = machine
            .mark_synchronized(REQUEST_ID, generation)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");

        machine.socket_opened(REQUEST_ID, generation).unwrap();
        machine.mark_synchronized(REQUEST_ID, generation).unwrap();
        assert_eq!(machine.state(), TransportState::Synchronized);

        // A second Step 2 for the same live generation is a wrong-state
        // refusal; a stale generation is stale even in Synchronized.
        let error = machine
            .mark_synchronized(REQUEST_ID, generation)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        let stale = TransportGeneration::from_value(generation.value() + 100);
        let error = machine.mark_synchronized(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    }

    #[test]
    fn admit_receive_accepts_only_live_handshaking_or_synchronized_frames() {
        // Connecting: live generation, wrong state; stale is stale.
        let (machine, generation) = connected_machine();
        let error = machine.admit_receive(REQUEST_ID, generation).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        let stale = TransportGeneration::from_value(generation.value() + 100);
        let error = machine.admit_receive(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        // Handshaking and Synchronized admit the live generation and report
        // the state without transitioning anything.
        let (mut machine, generation) = connected_machine();
        machine.socket_opened(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine.admit_receive(REQUEST_ID, generation).unwrap(),
            TransportState::Handshaking,
        );
        machine.mark_synchronized(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine.admit_receive(REQUEST_ID, generation).unwrap(),
            TransportState::Synchronized,
        );
        assert_eq!(machine.state(), TransportState::Synchronized);
        let error = machine.admit_receive(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        // A closed generation is stale for admission in every parked state.
        machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Incompatible)
            .unwrap();
        let error = machine.admit_receive(REQUEST_ID, generation).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    }

    #[test]
    fn admit_outbound_take_mirrors_receive_admission_under_its_own_label() {
        // Connecting: live generation, wrong state; stale is stale.
        let (machine, generation) = connected_machine();
        let error = machine
            .admit_outbound_take(REQUEST_ID, generation)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        assert_eq!(
            error.details.as_ref().unwrap()["action"],
            serde_json::json!("takeOutbound"),
            "{error:?}",
        );
        let stale = TransportGeneration::from_value(generation.value() + 100);
        let error = machine.admit_outbound_take(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        // Handshaking and Synchronized admit the live generation without
        // transitioning anything.
        let (mut machine, generation) = connected_machine();
        machine.socket_opened(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine.admit_outbound_take(REQUEST_ID, generation).unwrap(),
            TransportState::Handshaking,
        );
        machine.mark_synchronized(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine.admit_outbound_take(REQUEST_ID, generation).unwrap(),
            TransportState::Synchronized,
        );
        assert_eq!(machine.state(), TransportState::Synchronized);
        let error = machine.admit_outbound_take(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        // A closed generation is stale for outbound pickup too.
        machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable)
            .unwrap();
        let error = machine
            .admit_outbound_take(REQUEST_ID, generation)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    }

    #[test]
    fn settle_for_restore_parks_disconnected_without_reissuing_generations() {
        // Disconnected with a retired attempt: settle keeps the state, the
        // retired generation stays stale, and the next issued generation is
        // strictly monotonic (never reissued).
        let (mut machine, generation) = connected_machine();
        machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable)
            .unwrap();
        machine.settle_for_restore();
        assert_eq!(machine.state(), TransportState::Disconnected);
        let error = machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        let next = machine.begin_connect(REQUEST_ID, ROOM_BOUND).unwrap();
        assert!(next.value() > generation.value());

        // Detached settles to Disconnected: restore of a detached room is
        // the designed re-entry into the disconnected row.
        let mut detached = TransportStateMachine::new(TransportState::Detached);
        detached.settle_for_restore();
        assert_eq!(detached.state(), TransportState::Disconnected);
        assert!(detached.live_attempt.is_none());
    }

    #[test]
    fn teardown_and_test_injection_retire_the_live_attempt() {
        let (mut machine, generation) = connected_machine();
        machine.set_state_for_test(TransportState::Synchronized);
        let error = machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        assert_eq!(machine.state(), TransportState::Synchronized);

        let (mut machine, _) = connected_machine();
        machine.teardown_destroyed();
        assert_eq!(machine.state(), TransportState::Destroyed);
        assert!(machine.live_attempt.is_none());
    }

    #[test]
    fn stale_refusals_carry_structured_generation_details() {
        let (mut machine, generation) = connected_machine();
        let stale = TransportGeneration::from_value(9_999);
        let error = machine.socket_opened(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.domain, ErrorDomain::Transport);
        let details = error.details.expect("stale refusal must carry details");
        assert_eq!(details["action"], "socketOpened");
        assert_eq!(details["transportState"], "Connecting");
        assert_eq!(details["presentedGeneration"], 9_999);
        assert_eq!(details["liveGeneration"], generation.value());
    }
}
