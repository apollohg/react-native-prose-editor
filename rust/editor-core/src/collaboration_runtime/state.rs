//! Generation-owned transport state machine.
//!
//! Scope: the machine is the single writer of a session's transport state.
//! Every transition runs under the session lock (callers reach it only
//! through `EditorSession`, which the registry hands out via the
//! `with_alive` idiom), every accepted connection attempt increments the
//! monotonic [`TransportGeneration`] exactly once, and callbacks carrying a
//! stale generation are refused as observable no-ops. Rust owns retry
//! eligibility, the backoff index, and its absolute monotonic deadline; the
//! host only invokes the Rust drive operation when that deadline arrives.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use crate::ffi_v2::types::decimal_u64;
use crate::session::{ErrorDomain, SessionError, TransportState};

/// Refusal code for callbacks whose generation is not the live attempt
/// (superseded, closed, fabricated, or never issued).
pub const TRANSPORT_STALE_GENERATION: &str = "TRANSPORT_STALE_GENERATION";
/// Refusal code for actions that are not legal transitions from the current
/// transport state; the native host follows the returned Rust directive.
pub const TRANSPORT_INVALID_TRANSITION: &str = "TRANSPORT_INVALID_TRANSITION";
/// Refusal code for a test-only immediate connect attempt while
/// deterministically incompatible;
/// only configuration/server changes or detach/reattach reopen the row.
pub const TRANSPORT_INCOMPATIBLE: &str = "TRANSPORT_INCOMPATIBLE";
/// Refusal code for connection-shaped actions on a local-only session that
/// has no room binding to connect to.
pub const TRANSPORT_NOT_ROOM_BOUND: &str = "TRANSPORT_NOT_ROOM_BOUND";
/// Terminal refusal code when no fresh transport generation can be issued.
pub const TRANSPORT_GENERATION_EXHAUSTED: &str = "TRANSPORT_GENERATION_EXHAUSTED";

/// Generation value before any connection attempt; the first due drive
/// issues `INITIAL_TRANSPORT_GENERATION + 1`.
const INITIAL_TRANSPORT_GENERATION: u64 = 0;

const RETRY_DELAYS_MILLIS: [u64; 7] = [500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000];
const FINAL_RETRY_DELAY_INDEX: u8 = (RETRY_DELAYS_MILLIS.len() - 1) as u8;

/// Monotonic identity of one connection attempt. Issued only by
/// [`TransportStateMachine::drive`]; native carries the raw value through
/// its socket callbacks and the boundary rebuilds it with
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

/// Rust-owned disposition of a socket close, decided by the caller's error
/// classification or reported transparently by the native host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketCloseDisposition {
    /// The close is retryable: the transport returns to `Disconnected`,
    /// where a future due [`TransportStateMachine::drive`] may issue a
    /// generation.
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
    /// Index of the delay to use after the next retryable close. It advances
    /// only after a close and remains capped at the final 30-second entry.
    retry_attempt_index: u8,
    /// The next point at which a disconnected room may issue a generation.
    /// `None` means a fresh attached/reattached session is due immediately.
    next_retry_deadline_millis: Option<u64>,
    /// A retryable close whose `now + delay` cannot be represented. It is
    /// deliberately distinct from a deadline at `u64::MAX`: no drive may
    /// treat it as due until a lifecycle reset creates a fresh retry state.
    retry_schedule_exhausted: bool,
}

impl TransportStateMachine {
    pub(crate) fn new(initial: TransportState) -> Self {
        Self {
            state: initial,
            live_attempt: None,
            last_issued: INITIAL_TRANSPORT_GENERATION,
            retry_attempt_index: 0,
            next_retry_deadline_millis: None,
            retry_schedule_exhausted: false,
        }
    }

    pub(crate) fn state(&self) -> TransportState {
        self.state
    }

    /// The sole production issuance path. A room-bound disconnected runtime
    /// issues one generation only once Rust's own absolute retry deadline is
    /// due; initial attachment and reattachment use `None` to mean due now.
    /// Every other state returns no generation and leaves retry policy
    /// untouched, including `Incompatible`.
    pub(crate) fn drive(
        &mut self,
        request_id: u64,
        room_bound: bool,
        now_millis: u64,
    ) -> Result<Option<TransportGeneration>, SessionError> {
        if self.state != TransportState::Disconnected || !room_bound {
            return Ok(None);
        }
        if self.retry_schedule_exhausted {
            return Ok(None);
        }
        if self
            .next_retry_deadline_millis
            .is_some_and(|deadline| now_millis < deadline)
        {
            return Ok(None);
        }
        self.issue_generation(request_id, room_bound, "collaborationDrive")
            .map(Some)
    }

    /// Test-only direct generation constructor for isolated state-machine
    /// tests. Production code can issue a generation only through
    /// [`Self::drive`].
    #[cfg(test)]
    pub(crate) fn issue_generation_for_test(
        &mut self,
        request_id: u64,
        room_bound: bool,
    ) -> Result<TransportGeneration, SessionError> {
        self.issue_generation(request_id, room_bound, "issueGenerationForTest")
    }

    fn issue_generation(
        &mut self,
        request_id: u64,
        room_bound: bool,
        action: &'static str,
    ) -> Result<TransportGeneration, SessionError> {
        if !room_bound {
            return Err(not_room_bound(request_id, action, self.state));
        }
        match self.state {
            TransportState::Disconnected => {
                let next_generation = self.last_issued.checked_add(1).ok_or_else(|| {
                    SessionError::new(
                        ErrorDomain::Transport,
                        TRANSPORT_GENERATION_EXHAUSTED,
                        "transport generation space is exhausted",
                    )
                    .with_transport_context(request_id, action, self.state)
                })?;
                let generation = TransportGeneration(next_generation);
                self.last_issued = next_generation;
                self.live_attempt = Some(generation);
                self.state = TransportState::Connecting;
                self.next_retry_deadline_millis = None;
                Ok(generation)
            }
            TransportState::Incompatible => Err(SessionError::new(
                ErrorDomain::Transport,
                TRANSPORT_INCOMPATIBLE,
                "transport is deterministically incompatible; reconnect requires \
                 detach and reattach",
            )
            .with_transport_context(request_id, action, self.state)),
            _ => Err(invalid_transition(request_id, action, self.state)),
        }
    }

    /// Read-only admission for socket-open work. The session builds and
    /// reserves Sync Step 1 before calling [`Self::socket_opened`] so an
    /// allocation/refusal cannot leave a handshaking generation without its
    /// ordinary protocol-priority frame.
    pub(crate) fn admit_socket_open(
        &self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<(), SessionError> {
        self.require_live_attempt(request_id, "socketOpened", generation)?;
        if self.state != TransportState::Connecting {
            return Err(invalid_transition(request_id, "socketOpened", self.state));
        }
        Ok(())
    }

    /// Current `Connecting` -> `Handshaking`. Sync Step 1 has already been
    /// reserved into the shared lease path by the session. Stale generations
    /// are observationally pure.
    pub(crate) fn socket_opened(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<(), SessionError> {
        self.admit_socket_open(request_id, generation)?;
        self.state = TransportState::Handshaking;
        Ok(())
    }

    /// Current-generation close: the live attempt ends and the disposition
    /// decides `Disconnected` (retry eligible) or `Incompatible`. Stale
    /// generations are observable no-ops that can neither advance nor
    /// regress the transport.
    pub(crate) fn admit_socket_close(
        &self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<(), SessionError> {
        self.require_live_attempt(request_id, "socketClosed", generation)
    }

    pub(crate) fn socket_closed(
        &mut self,
        request_id: u64,
        generation: TransportGeneration,
        disposition: SocketCloseDisposition,
        now_millis: u64,
    ) -> Result<TransportState, SessionError> {
        self.admit_socket_close(request_id, generation)?;
        let retry_deadline = match disposition {
            SocketCloseDisposition::Retryable => self.next_retry_deadline_after_close(now_millis),
            SocketCloseDisposition::Incompatible => {
                self.retry_schedule_exhausted = false;
                None
            }
        };
        self.live_attempt = None;
        self.state = match disposition {
            SocketCloseDisposition::Retryable => TransportState::Disconnected,
            SocketCloseDisposition::Incompatible => TransportState::Incompatible,
        };
        self.next_retry_deadline_millis = retry_deadline;
        Ok(self.state)
    }

    /// Local intent: close the live attempt -> `Disconnected`; the closed
    /// generation's late callbacks become stale and retries stop until the
    /// caller performs a fresh lifecycle reset.
    #[cfg(test)]
    pub(crate) fn disconnect(&mut self, request_id: u64) -> Result<(), SessionError> {
        if !matches!(
            self.state,
            TransportState::Connecting | TransportState::Handshaking | TransportState::Synchronized
        ) {
            return Err(invalid_transition(request_id, "disconnect", self.state));
        }
        self.live_attempt = None;
        self.state = TransportState::Disconnected;
        self.next_retry_deadline_millis = None;
        self.retry_schedule_exhausted = true;
        Ok(())
    }

    /// Any live transport state -> `Detached`; repeated detaches are no-ops.
    /// Tears down only transport state: the runtime, its outbox, and the
    /// document are untouched by design.
    pub(crate) fn detach(&mut self, _request_id: u64) -> Result<(), SessionError> {
        if self.state == TransportState::Detached {
            return Ok(());
        }
        self.live_attempt = None;
        self.state = TransportState::Detached;
        self.reset_retry_for_fresh_drive();
        Ok(())
    }

    /// The explicit reattach half of the `Incompatible` escape hatch:
    /// `Detached` -> `Disconnected` on a room-bound session; repeated
    /// room-bound reattaches from `Disconnected` are no-ops. The successful
    /// detached-to-disconnected transition resets a fresh [`Self::drive`].
    pub(crate) fn reattach(
        &mut self,
        request_id: u64,
        room_bound: bool,
    ) -> Result<(), SessionError> {
        if !room_bound {
            return Err(not_room_bound(request_id, "reattach", self.state));
        }
        match self.state {
            TransportState::Detached => {
                self.state = TransportState::Disconnected;
                self.reset_retry_for_fresh_drive();
                Ok(())
            }
            TransportState::Disconnected => Ok(()),
            _ => Err(invalid_transition(request_id, "reattach", self.state)),
        }
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
        self.reset_retry_for_fresh_drive();
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

    /// Read-only outbound-lease admission gate: retaining the next outbound
    /// frame is generation-scoped wire work with the same
    /// admission law as [`Self::admit_receive`] (live generation while
    /// `Handshaking`/`Synchronized`), under its own action label.
    pub(crate) fn admit_outbound_lease(
        &self,
        request_id: u64,
        generation: TransportGeneration,
    ) -> Result<TransportState, SessionError> {
        self.require_live_attempt(request_id, "leaseOutbound", generation)?;
        match self.state {
            TransportState::Handshaking | TransportState::Synchronized => Ok(self.state),
            state => Err(invalid_transition(request_id, "leaseOutbound", state)),
        }
    }

    /// Task 11 snapshot-restore settle: `Detached`/`Disconnected` ->
    /// `Disconnected`. The session restore gate admits only those two
    /// states, so no attempt can be live here; this is the designed
    /// "sync-generation state cleared" write — the transport returns to the
    /// room's disconnected row, where a fresh [`Self::drive`] is due again.
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
        self.reset_retry_for_fresh_drive();
    }

    /// Lifecycle teardown writer used by session destroy: transport state
    /// becomes `Destroyed` and any live attempt is retired. Not a
    /// request-scoped transition — destroy wins from every state.
    pub(crate) fn teardown_destroyed(&mut self) {
        self.live_attempt = None;
        self.state = TransportState::Destroyed;
        self.next_retry_deadline_millis = None;
        self.retry_schedule_exhausted = false;
    }

    /// Test-only state injection for policy-matrix cells that are
    /// unreachable through real transitions (see
    /// `EditorSession::set_transport_state_for_test`). Forced states carry
    /// no live attempt, so every generation-carrying callback on them is
    /// stale by construction and the machine's invariants hold.
    pub(crate) fn set_state_for_test(&mut self, state: TransportState) {
        self.live_attempt = None;
        self.state = state;
        self.reset_retry_for_fresh_drive();
    }

    pub(crate) fn next_retry_deadline_millis(&self) -> Option<u64> {
        self.next_retry_deadline_millis
    }

    fn next_retry_deadline_after_close(&mut self, now_millis: u64) -> Option<u64> {
        let delay_index = usize::from(self.retry_attempt_index).min(RETRY_DELAYS_MILLIS.len() - 1);
        let Some(deadline) = now_millis.checked_add(RETRY_DELAYS_MILLIS[delay_index]) else {
            self.retry_schedule_exhausted = true;
            return None;
        };
        self.retry_attempt_index = if self.retry_attempt_index < FINAL_RETRY_DELAY_INDEX {
            self.retry_attempt_index
                .checked_add(1)
                .unwrap_or(FINAL_RETRY_DELAY_INDEX)
        } else {
            FINAL_RETRY_DELAY_INDEX
        };
        Some(deadline)
    }

    fn reset_retry_for_fresh_drive(&mut self) {
        self.retry_attempt_index = 0;
        self.next_retry_deadline_millis = None;
        self.retry_schedule_exhausted = false;
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
        details["presentedGeneration"] = decimal_u64(presented.value());
        details["liveGeneration"] = match live {
            Some(generation) => decimal_u64(generation.value()),
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
        let generation = machine
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap();
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
    fn issue_generation_for_test_issues_monotonic_generations_and_refusals_never_consume_one() {
        let (mut machine, first) = connected_machine();
        assert_eq!(machine.state(), TransportState::Connecting);
        assert_eq!(first.value(), INITIAL_TRANSPORT_GENERATION + 1);

        // Refused while an attempt is live: no state change, no generation.
        let error = machine
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        assert_eq!(error.request_id, Some(REQUEST_ID));
        assert_eq!(machine.state(), TransportState::Connecting);

        machine
            .socket_closed(REQUEST_ID, first, SocketCloseDisposition::Retryable, 0)
            .unwrap();
        let second = machine
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap();
        assert_eq!(second.value(), first.value() + 1);
    }

    #[test]
    fn task8_third_remediation_generation_exhaustion_is_terminal_and_atomic() {
        let mut machine = TransportStateMachine {
            state: TransportState::Disconnected,
            live_attempt: None,
            last_issued: u64::MAX,
            retry_attempt_index: 0,
            next_retry_deadline_millis: None,
            retry_schedule_exhausted: false,
        };

        for request_id in [REQUEST_ID, REQUEST_ID + 1] {
            let error = machine
                .issue_generation_for_test(request_id, ROOM_BOUND)
                .unwrap_err();
            assert_eq!(error.domain, ErrorDomain::Transport, "{error:?}");
            assert_eq!(error.code, "TRANSPORT_GENERATION_EXHAUSTED", "{error:?}");
            assert_eq!(error.request_id, Some(request_id), "{error:?}");
            let details = error.details.expect("exhaustion carries transport context");
            assert_eq!(details["action"], "issueGenerationForTest");
            assert_eq!(details["transportState"], "Disconnected");
            assert_eq!(machine.state, TransportState::Disconnected);
            assert_eq!(machine.live_attempt, None);
            assert_eq!(machine.last_issued, u64::MAX);
        }
    }

    #[test]
    fn issue_generation_for_test_refuses_local_only_detached_and_incompatible_rows() {
        let mut local = TransportStateMachine::new(TransportState::Disconnected);
        let error = local
            .issue_generation_for_test(REQUEST_ID, LOCAL_ONLY)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_NOT_ROOM_BOUND, "{error:?}");
        assert_eq!(local.state(), TransportState::Disconnected);
        // The refusal details report the machine's actual state, not a
        // hardcoded one.
        let details = error.details.expect("refusal must carry details");
        assert_eq!(details["action"], "issueGenerationForTest");
        assert_eq!(details["transportState"], "Disconnected");

        let mut detached = TransportStateMachine::new(TransportState::Detached);
        let error = detached
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");

        let (mut machine, generation) = connected_machine();
        machine
            .socket_closed(
                REQUEST_ID,
                generation,
                SocketCloseDisposition::Incompatible,
                0,
            )
            .unwrap();
        let error = machine
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap_err();
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

        machine.socket_opened(REQUEST_ID, generation).unwrap();
        assert_eq!(machine.state(), TransportState::Handshaking);

        // A duplicate open of the live generation is a wrong-state refusal.
        let error = machine.socket_opened(REQUEST_ID, generation).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        assert_eq!(machine.state(), TransportState::Handshaking);
    }

    #[test]
    fn nested_u64_error_details_are_canonical_decimal_strings() {
        let mut machine = TransportStateMachine::new(TransportState::Connecting);
        machine.live_attempt = Some(TransportGeneration::from_value(u64::MAX));

        let error = machine
            .socket_opened(
                REQUEST_ID,
                TransportGeneration::from_value(9_007_199_254_740_993),
            )
            .unwrap_err();
        let ffi_error = crate::ffi_v2::types::FfiError::from(error);
        let details: serde_json::Value = serde_json::from_str(
            ffi_error
                .details_json
                .as_deref()
                .expect("stale generation must preserve details"),
        )
        .expect("details JSON must be valid");

        assert_eq!(
            details,
            serde_json::json!({
                "action": "socketOpened",
                "transportState": "Connecting",
                "presentedGeneration": "9007199254740993",
                "liveGeneration": "18446744073709551615",
            }),
        );
    }

    #[test]
    fn socket_closed_applies_the_rust_owned_disposition_and_retires_the_attempt() {
        let (mut machine, generation) = connected_machine();
        assert_eq!(
            machine
                .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable, 0)
                .unwrap(),
            TransportState::Disconnected,
        );
        // The retired generation is stale for every later callback.
        let error = machine
            .socket_closed(
                REQUEST_ID,
                generation,
                SocketCloseDisposition::Incompatible,
                0,
            )
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        assert_eq!(machine.state(), TransportState::Disconnected);

        let (mut machine, generation) = connected_machine();
        machine.socket_opened(REQUEST_ID, generation).unwrap();
        machine.mark_synchronized(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine
                .socket_closed(
                    REQUEST_ID,
                    generation,
                    SocketCloseDisposition::Incompatible,
                    0
                )
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
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable, 0)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        let error = machine.disconnect(REQUEST_ID).unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
    }

    #[test]
    fn task8_lifecycle_contract_direct_state_idempotence_and_refusals() {
        let (mut machine, generation) = connected_machine();
        machine
            .socket_closed(
                REQUEST_ID,
                generation,
                SocketCloseDisposition::Incompatible,
                0,
            )
            .unwrap();
        machine.detach(REQUEST_ID).unwrap();
        assert_eq!(machine.state(), TransportState::Detached);
        machine.detach(REQUEST_ID).unwrap();
        assert_eq!(machine.state(), TransportState::Detached);

        let error = machine.reattach(REQUEST_ID, LOCAL_ONLY).unwrap_err();
        assert_eq!(error.code, TRANSPORT_NOT_ROOM_BOUND, "{error:?}");
        machine.reattach(REQUEST_ID, ROOM_BOUND).unwrap();
        assert_eq!(machine.state(), TransportState::Disconnected);
        machine.reattach(REQUEST_ID, ROOM_BOUND).unwrap();
        assert_eq!(machine.state(), TransportState::Disconnected);

        let next = machine
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap();
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
            .socket_closed(
                REQUEST_ID,
                generation,
                SocketCloseDisposition::Incompatible,
                0,
            )
            .unwrap();
        let error = machine.admit_receive(REQUEST_ID, generation).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
    }

    #[test]
    fn admit_outbound_lease_mirrors_receive_admission_under_its_own_label() {
        // Connecting: live generation, wrong state; stale is stale.
        let (machine, generation) = connected_machine();
        let error = machine
            .admit_outbound_lease(REQUEST_ID, generation)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_INVALID_TRANSITION, "{error:?}");
        assert_eq!(
            error.details.as_ref().unwrap()["action"],
            serde_json::json!("leaseOutbound"),
            "{error:?}",
        );
        let stale = TransportGeneration::from_value(generation.value() + 100);
        let error = machine.admit_outbound_lease(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        // Handshaking and Synchronized admit the live generation without
        // transitioning anything.
        let (mut machine, generation) = connected_machine();
        machine.socket_opened(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine
                .admit_outbound_lease(REQUEST_ID, generation)
                .unwrap(),
            TransportState::Handshaking,
        );
        machine.mark_synchronized(REQUEST_ID, generation).unwrap();
        assert_eq!(
            machine
                .admit_outbound_lease(REQUEST_ID, generation)
                .unwrap(),
            TransportState::Synchronized,
        );
        assert_eq!(machine.state(), TransportState::Synchronized);
        let error = machine.admit_outbound_lease(REQUEST_ID, stale).unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");

        // A closed generation is stale for outbound pickup too.
        machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable, 0)
            .unwrap();
        let error = machine
            .admit_outbound_lease(REQUEST_ID, generation)
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
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable, 0)
            .unwrap();
        machine.settle_for_restore();
        assert_eq!(machine.state(), TransportState::Disconnected);
        let error = machine
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable, 0)
            .unwrap_err();
        assert_eq!(error.code, TRANSPORT_STALE_GENERATION, "{error:?}");
        let next = machine
            .issue_generation_for_test(REQUEST_ID, ROOM_BOUND)
            .unwrap();
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
            .socket_closed(REQUEST_ID, generation, SocketCloseDisposition::Retryable, 0)
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
        assert_eq!(
            details["presentedGeneration"],
            serde_json::Value::String("9999".into())
        );
        assert_eq!(
            details["liveGeneration"],
            serde_json::Value::String(generation.value().to_string())
        );
    }
}
