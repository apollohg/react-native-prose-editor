//! Bounded pre-commit outbound document-update queue.
//!
//! The outbox owns only captured Update-v1 bytes and bookkeeping — never a
//! `yrs::Doc`, a transaction handle, or any way to apply Yrs mutations. Its
//! contract is reservation-before-irreversible-write: every durable local
//! commit reserves count, bytes, and queue storage from a conservative upper
//! bound while failure is still recoverable, and the post-commit
//! [`CollaborationOutbox::install`] is infallible by construction.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::session::CollaborationLimits;

/// `CollaborationLimits` field charged for pending/reserved outbox messages.
pub const OUTBOX_MESSAGES_FIELD: &str = "maxPendingOutboxMessages";
/// `CollaborationLimits` field charged for pending/reserved outbox bytes.
pub const OUTBOX_BYTES_FIELD: &str = "maxPendingOutboxBytes";

thread_local! {
    static FAIL_RESERVATION_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}

/// Simulate an allocation failure inside the next outbox reservations.
/// Mirrors the history-module failpoint idiom for atomicity coverage.
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
pub fn set_reservation_allocation_failure_for_test(enabled: bool) {
    FAIL_RESERVATION_ALLOCATION.with(|cell| cell.set(enabled));
}

fn reservation_allocation_failure_armed() -> bool {
    FAIL_RESERVATION_ALLOCATION.with(Cell::get)
}

/// Reservation failure surface. `Saturated` is a deterministic configured
/// ceiling; `Allocation` is a recoverable storage-reservation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxReservationError {
    Saturated {
        field: &'static str,
        limit: usize,
        actual: usize,
    },
    Allocation,
}

/// Shared reservation accounting. Live in an `Arc` so an unconsumed
/// reservation releases its capacity on drop even if the owning commit path
/// unwinds through an error return.
#[derive(Debug, Default)]
struct ReservationLedger {
    reserved_messages: AtomicUsize,
    reserved_bytes: AtomicUsize,
}

impl ReservationLedger {
    fn charge(&self, messages: usize, bytes: usize) {
        self.reserved_messages
            .fetch_add(messages, Ordering::Relaxed);
        self.reserved_bytes.fetch_add(bytes, Ordering::Relaxed);
    }

    fn release(&self, messages: usize, bytes: usize) {
        self.reserved_messages
            .fetch_sub(messages, Ordering::Relaxed);
        self.reserved_bytes.fetch_sub(bytes, Ordering::Relaxed);
    }
}

/// One admitted, one-shot document-update reservation. Private fields,
/// deliberately non-`Clone`; consumed exactly once by
/// [`CollaborationOutbox::install`] or released on drop.
#[derive(Debug)]
pub struct OutboxReservation {
    ledger: Arc<ReservationLedger>,
    request_id: u64,
    upper_bound_bytes: usize,
    consumed: bool,
}

impl Drop for OutboxReservation {
    fn drop(&mut self) {
        if !self.consumed {
            self.ledger.release(1, self.upper_bound_bytes);
        }
    }
}

/// Reserved capacity for protocol replies (Sync Step responses). Replies
/// share the outbox ceilings so a saturated queue rejects before any
/// irreversible protocol work; Task 9 consumes a reservation through the
/// infallible [`CollaborationOutbox::install_protocol_replies`].
#[derive(Debug)]
pub struct ProtocolReplyReservation {
    ledger: Arc<ReservationLedger>,
    reply_count: usize,
    upper_bound_bytes: usize,
    consumed: bool,
}

impl ProtocolReplyReservation {
    /// Number of replies this reservation admits.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn reply_count(&self) -> usize {
        self.reply_count
    }

    /// Aggregate byte bound this reservation admits.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn upper_bound_bytes(&self) -> usize {
        self.upper_bound_bytes
    }
}

impl Drop for ProtocolReplyReservation {
    fn drop(&mut self) {
        if !self.consumed {
            self.ledger
                .release(self.reply_count, self.upper_bound_bytes);
        }
    }
}

/// One completely built, admitted, framed protocol reply awaiting transport
/// pickup. Protocol entries are accounted separately from document updates:
/// remote handling never creates document entries (no echo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxProtocolReply {
    pub request_id: u64,
    pub message: Vec<u8>,
}

/// One captured outbound document update awaiting transport pickup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxDocumentUpdate {
    pub request_id: u64,
    pub update_v1: Vec<u8>,
}

/// Opaque identity for one retained outbound handoff lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutboundLeaseId(u64);

impl OutboundLeaseId {
    pub(crate) fn value(self) -> u64 {
        self.0
    }

    pub(crate) fn from_value(value: u64) -> Self {
        Self(value)
    }
}

/// Bytes handed to the transport under an outbound lease. Protocol replies
/// are already framed; document updates are raw Update-v1 bytes and are
/// framed at the session/protocol boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OutboundLeasePayload {
    ProtocolReply(Vec<u8>),
    DocumentUpdate(Vec<u8>),
}

/// One retained outbound queue-front lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OutboundLease {
    pub(crate) lease_id: OutboundLeaseId,
    pub(crate) payload: OutboundLeasePayload,
}

/// Lease lifecycle failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutboundLeaseError {
    LeaseIdExhausted,
    NoActiveLease,
    LeaseMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboundLeaseKind {
    ProtocolReply,
    DocumentUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ActiveOutboundLease {
    id: OutboundLeaseId,
    kind: OutboundLeaseKind,
}

/// Bounded outbound document-update queue owned by the collaboration
/// runtime. Ceilings come from the session's validated
/// [`CollaborationLimits`].
#[derive(Debug)]
pub struct CollaborationOutbox {
    max_pending_messages: usize,
    max_pending_bytes: usize,
    pending: VecDeque<OutboxDocumentUpdate>,
    pending_bytes: usize,
    pending_protocol: VecDeque<OutboxProtocolReply>,
    pending_protocol_bytes: usize,
    next_lease_id: Option<u64>,
    active_lease: Option<ActiveOutboundLease>,
    ledger: Arc<ReservationLedger>,
    last_reserved_upper_bound: Option<usize>,
}

impl CollaborationOutbox {
    /// Build an outbox with explicit ceilings (message count / total bytes).
    pub fn with_ceilings(max_pending_messages: usize, max_pending_bytes: usize) -> Self {
        Self {
            max_pending_messages,
            max_pending_bytes,
            pending: VecDeque::new(),
            pending_bytes: 0,
            pending_protocol: VecDeque::new(),
            pending_protocol_bytes: 0,
            next_lease_id: Some(1),
            active_lease: None,
            ledger: Arc::new(ReservationLedger::default()),
            last_reserved_upper_bound: None,
        }
    }

    /// Build an outbox from the session's validated collaboration limits.
    pub(crate) fn from_limits(limits: &CollaborationLimits) -> Self {
        Self::with_ceilings(
            limits.max_pending_outbox_messages,
            limits.max_pending_outbox_bytes,
        )
    }

    /// Fallible reservation of one document-update slot plus its conservative
    /// byte bound, performed BEFORE the irreversible Yrs write. Reserves the
    /// queue storage for the later infallible [`Self::install`].
    pub fn reserve_document_update(
        &mut self,
        request_id: u64,
        upper_bound_bytes: usize,
    ) -> Result<OutboxReservation, OutboxReservationError> {
        self.admit_reservation(1, upper_bound_bytes)?;
        if self.pending.try_reserve(1).is_err() {
            return Err(OutboxReservationError::Allocation);
        }
        self.ledger.charge(1, upper_bound_bytes);
        self.last_reserved_upper_bound = Some(upper_bound_bytes);
        Ok(OutboxReservation {
            ledger: Arc::clone(&self.ledger),
            request_id,
            upper_bound_bytes,
            consumed: false,
        })
    }

    /// Fallible reservation of protocol-reply capacity against the same
    /// ceilings, including the queue storage for the later infallible
    /// [`Self::install_protocol_replies`].
    pub fn reserve_protocol_replies(
        &mut self,
        reply_count: usize,
        upper_bound_bytes: usize,
    ) -> Result<ProtocolReplyReservation, OutboxReservationError> {
        self.admit_reservation(reply_count, upper_bound_bytes)?;
        if self.pending_protocol.try_reserve(reply_count).is_err() {
            return Err(OutboxReservationError::Allocation);
        }
        self.ledger.charge(reply_count, upper_bound_bytes);
        Ok(ProtocolReplyReservation {
            ledger: Arc::clone(&self.ledger),
            reply_count,
            upper_bound_bytes,
            consumed: false,
        })
    }

    /// Infallible post-commit append of the captured Update-v1. The queue
    /// slot was reserved with the reservation and the captured length is
    /// enforced against the admitted bound.
    pub fn install(&mut self, mut reservation: OutboxReservation, update_v1: Vec<u8>) {
        debug_assert!(
            Arc::ptr_eq(&self.ledger, &reservation.ledger),
            "an outbox reservation can only be installed into its own outbox",
        );
        debug_assert!(
            update_v1.len() <= reservation.upper_bound_bytes,
            "captured Update-v1 exceeds its reserved outbox bound: {} > {}",
            update_v1.len(),
            reservation.upper_bound_bytes,
        );
        reservation.consumed = true;
        self.ledger.release(1, reservation.upper_bound_bytes);
        self.pending_bytes = self.pending_bytes.saturating_add(update_v1.len());
        self.pending.push_back(OutboxDocumentUpdate {
            request_id: reservation.request_id,
            update_v1,
        });
    }

    /// Infallible post-commit installation of the completely built protocol
    /// replies a reservation admitted. The queue storage was reserved with
    /// the reservation and every framed reply is enforced against the
    /// admitted count/byte bounds.
    pub fn install_protocol_replies(
        &mut self,
        mut reservation: ProtocolReplyReservation,
        request_id: u64,
        messages: Vec<Vec<u8>>,
    ) {
        debug_assert!(
            Arc::ptr_eq(&self.ledger, &reservation.ledger),
            "a protocol-reply reservation can only be installed into its own outbox",
        );
        debug_assert_eq!(
            messages.len(),
            reservation.reply_count,
            "installed protocol replies must match their reserved count",
        );
        debug_assert!(
            messages.iter().map(Vec::len).sum::<usize>() <= reservation.upper_bound_bytes,
            "installed protocol replies exceed their reserved byte bound: {} > {}",
            messages.iter().map(Vec::len).sum::<usize>(),
            reservation.upper_bound_bytes,
        );
        reservation.consumed = true;
        self.ledger
            .release(reservation.reply_count, reservation.upper_bound_bytes);
        for message in messages {
            self.pending_protocol_bytes = self.pending_protocol_bytes.saturating_add(message.len());
            self.pending_protocol.push_back(OutboxProtocolReply {
                request_id,
                message,
            });
        }
    }

    /// Lease the transport-priority queue front without consuming it.
    /// Repeated calls while a lease is active return the same queue front and
    /// lease identity; only an exact ACK releases the queued accounting.
    pub(crate) fn lease_next(&mut self) -> Result<Option<OutboundLease>, OutboundLeaseError> {
        if let Some(active_lease) = self.active_lease {
            return Ok(Some(self.clone_leased_front(active_lease)));
        }

        let kind = if self.pending_protocol.front().is_some() {
            OutboundLeaseKind::ProtocolReply
        } else if self.pending.front().is_some() {
            OutboundLeaseKind::DocumentUpdate
        } else {
            return Ok(None);
        };
        let lease_id = OutboundLeaseId(
            self.next_lease_id
                .ok_or(OutboundLeaseError::LeaseIdExhausted)?,
        );
        self.next_lease_id = self.next_lease_id.and_then(|id| id.checked_add(1));
        let active_lease = ActiveOutboundLease { id: lease_id, kind };
        self.active_lease = Some(active_lease);
        Ok(Some(self.clone_leased_front(active_lease)))
    }

    /// Confirm transport delivery of the active queue front. The queue entry
    /// and its accounting are released exactly once after an exact ID match.
    pub(crate) fn ack_lease(
        &mut self,
        lease_id: OutboundLeaseId,
    ) -> Result<(), OutboundLeaseError> {
        let active_lease = self.require_matching_lease(lease_id)?;
        match active_lease.kind {
            OutboundLeaseKind::ProtocolReply => {
                let entry = self
                    .pending_protocol
                    .front()
                    .expect("an active protocol lease requires a queued protocol reply");
                let remaining_bytes = self
                    .pending_protocol_bytes
                    .checked_sub(entry.message.len())
                    .expect("active protocol lease accounting underflow");
                let _ = self.pending_protocol.pop_front();
                self.pending_protocol_bytes = remaining_bytes;
            }
            OutboundLeaseKind::DocumentUpdate => {
                let entry = self
                    .pending
                    .front()
                    .expect("an active document lease requires a queued document update");
                let remaining_bytes = self
                    .pending_bytes
                    .checked_sub(entry.update_v1.len())
                    .expect("active document lease accounting underflow");
                let _ = self.pending.pop_front();
                self.pending_bytes = remaining_bytes;
            }
        }
        self.active_lease = None;
        Ok(())
    }

    /// Reject the active transport handoff while preserving the queue front
    /// and all pending accounting for a later lease.
    pub(crate) fn nack_lease(
        &mut self,
        lease_id: OutboundLeaseId,
    ) -> Result<(), OutboundLeaseError> {
        self.require_matching_lease(lease_id)?;
        self.active_lease = None;
        Ok(())
    }

    /// Clear an active lease without consuming its retained queue front.
    pub(crate) fn release_lease(&mut self) {
        self.active_lease = None;
    }

    /// Task 11 teardown-on-restore: drop every pending framed protocol
    /// reply. Protocol entries are transport-scoped and minted against the
    /// prior store — a restored session resynchronizes from Sync Step 1, so
    /// they can never become deliverable. Pending *document* updates are
    /// untouched: restore rejects while any exist, so none can be here by
    /// the time this runs. Infallible by construction.
    pub fn clear_protocol_replies(&mut self) {
        if matches!(
            self.active_lease,
            Some(ActiveOutboundLease {
                kind: OutboundLeaseKind::ProtocolReply,
                ..
            })
        ) {
            self.release_lease();
        }
        self.pending_protocol.clear();
        self.pending_protocol_bytes = 0;
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn pending_protocol_reply_count(&self) -> usize {
        self.pending_protocol.len()
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn pending_protocol_reply_bytes(&self) -> usize {
        self.pending_protocol_bytes
    }

    pub fn has_pending_document_updates(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Test-only observability for a retained document front. The production
    /// handoff deliberately exposes bytes and lease identity only; the native
    /// transaction fixture needs the originating request id to preserve its
    /// existing assertions while it drives the explicit ACK path.
    #[cfg(test)]
    pub(crate) fn pending_document_update_request_id_for_leased_front(&self) -> Option<u64> {
        match self.active_lease {
            Some(ActiveOutboundLease {
                kind: OutboundLeaseKind::DocumentUpdate,
                ..
            }) => self.pending.front().map(|entry| entry.request_id),
            _ => None,
        }
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn pending_document_update_count(&self) -> usize {
        self.pending.len()
    }

    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn pending_document_update_bytes(&self) -> usize {
        self.pending_bytes
    }

    /// Messages currently held by live (uninstalled) reservations.
    pub fn reserved_messages(&self) -> usize {
        self.ledger.reserved_messages.load(Ordering::Relaxed)
    }

    /// Bytes currently held by live (uninstalled) reservations.
    pub fn reserved_bytes(&self) -> usize {
        self.ledger.reserved_bytes.load(Ordering::Relaxed)
    }

    /// The most recent successfully admitted document-update bound; test
    /// observability for the actual-length-within-bound property.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn last_reserved_upper_bound_for_test(&self) -> Option<usize> {
        self.last_reserved_upper_bound
    }

    /// Test-only ceiling override for saturation matrices, mirroring the
    /// session `set_transport_state_for_test` idiom.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn set_ceilings_for_test(&mut self, max_pending_messages: usize, max_pending_bytes: usize) {
        self.max_pending_messages = max_pending_messages;
        self.max_pending_bytes = max_pending_bytes;
    }

    fn admit_reservation(
        &self,
        messages: usize,
        upper_bound_bytes: usize,
    ) -> Result<(), OutboxReservationError> {
        if reservation_allocation_failure_armed() {
            return Err(OutboxReservationError::Allocation);
        }
        let requested_messages = self
            .pending
            .len()
            .saturating_add(self.pending_protocol.len())
            .saturating_add(self.reserved_messages())
            .saturating_add(messages);
        if requested_messages > self.max_pending_messages {
            return Err(OutboxReservationError::Saturated {
                field: OUTBOX_MESSAGES_FIELD,
                limit: self.max_pending_messages,
                actual: requested_messages,
            });
        }
        let requested_bytes = self
            .pending_bytes
            .saturating_add(self.pending_protocol_bytes)
            .saturating_add(self.reserved_bytes())
            .saturating_add(upper_bound_bytes);
        if requested_bytes > self.max_pending_bytes {
            return Err(OutboxReservationError::Saturated {
                field: OUTBOX_BYTES_FIELD,
                limit: self.max_pending_bytes,
                actual: requested_bytes,
            });
        }
        Ok(())
    }

    fn require_matching_lease(
        &self,
        lease_id: OutboundLeaseId,
    ) -> Result<ActiveOutboundLease, OutboundLeaseError> {
        match self.active_lease {
            None => Err(OutboundLeaseError::NoActiveLease),
            Some(active_lease) if active_lease.id != lease_id => {
                Err(OutboundLeaseError::LeaseMismatch)
            }
            Some(active_lease) => Ok(active_lease),
        }
    }

    fn clone_leased_front(&self, active_lease: ActiveOutboundLease) -> OutboundLease {
        let payload = match active_lease.kind {
            OutboundLeaseKind::ProtocolReply => OutboundLeasePayload::ProtocolReply(
                self.pending_protocol
                    .front()
                    .expect("an active protocol lease requires a queued protocol reply")
                    .message
                    .clone(),
            ),
            OutboundLeaseKind::DocumentUpdate => OutboundLeasePayload::DocumentUpdate(
                self.pending
                    .front()
                    .expect("an active document lease requires a queued document update")
                    .update_v1
                    .clone(),
            ),
        };
        OutboundLease {
            lease_id: active_lease.id,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_limits_uses_the_configured_outbox_ceilings() {
        let limits = CollaborationLimits {
            max_pending_outbox_messages: 3,
            max_pending_outbox_bytes: 32,
            ..CollaborationLimits::default()
        };
        let mut outbox = CollaborationOutbox::from_limits(&limits);
        for request in 0..3 {
            let reservation = outbox.reserve_document_update(request, 10).unwrap();
            outbox.install(reservation, vec![0; 10]);
        }
        assert_eq!(
            outbox.reserve_document_update(4, 1).unwrap_err(),
            OutboxReservationError::Saturated {
                field: OUTBOX_MESSAGES_FIELD,
                limit: 3,
                actual: 4,
            },
        );
        assert_eq!(outbox.pending_document_update_bytes(), 30);
    }

    #[test]
    fn install_charges_actual_length_and_acknowledged_lease_restores_it() {
        let mut outbox = CollaborationOutbox::with_ceilings(2, 64);
        let reservation = outbox.reserve_document_update(7, 40).unwrap();
        assert_eq!(outbox.reserved_bytes(), 40);
        outbox.install(reservation, vec![1; 5]);
        assert_eq!(outbox.reserved_bytes(), 0);
        assert_eq!(outbox.pending_document_update_bytes(), 5);
        let lease = outbox.lease_next().unwrap().unwrap();
        assert_eq!(
            outbox.pending_document_update_request_id_for_leased_front(),
            Some(7)
        );
        assert!(matches!(
            lease.payload,
            OutboundLeasePayload::DocumentUpdate(ref update) if update.len() == 5
        ));
        outbox.ack_lease(lease.lease_id).unwrap();
        assert_eq!(outbox.pending_document_update_bytes(), 0);
        assert!(outbox.lease_next().unwrap().is_none());
    }

    #[test]
    fn dropping_reservations_releases_both_reservation_kinds() {
        let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
        let document = outbox.reserve_document_update(1, 16).unwrap();
        let replies = outbox.reserve_protocol_replies(2, 20).unwrap();
        assert_eq!(replies.reply_count(), 2);
        assert_eq!(replies.upper_bound_bytes(), 20);
        assert_eq!(outbox.reserved_messages(), 3);
        assert_eq!(outbox.reserved_bytes(), 36);
        drop(document);
        drop(replies);
        assert_eq!(outbox.reserved_messages(), 0);
        assert_eq!(outbox.reserved_bytes(), 0);
    }

    #[test]
    fn protocol_reply_install_and_acknowledged_lease_keep_separate_accounting() {
        let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
        let reservation = outbox.reserve_protocol_replies(2, 20).unwrap();
        assert_eq!(outbox.reserved_messages(), 2);
        assert_eq!(outbox.reserved_bytes(), 20);

        outbox.install_protocol_replies(reservation, 9, vec![vec![1; 6], vec![2; 4]]);
        // Consumed reservation releases exactly once; pending accounting
        // charges actual framed lengths, separate from document entries.
        assert_eq!(outbox.reserved_messages(), 0);
        assert_eq!(outbox.reserved_bytes(), 0);
        assert_eq!(outbox.pending_protocol_reply_count(), 2);
        assert_eq!(outbox.pending_protocol_reply_bytes(), 10);
        assert_eq!(outbox.pending_document_update_count(), 0);
        assert_eq!(outbox.pending_document_update_bytes(), 0);

        let first = outbox.lease_next().unwrap().unwrap();
        assert!(matches!(
            first.payload,
            OutboundLeasePayload::ProtocolReply(ref reply) if reply == &vec![1; 6]
        ));
        outbox.ack_lease(first.lease_id).unwrap();
        assert_eq!(outbox.pending_protocol_reply_bytes(), 4);
        let second = outbox.lease_next().unwrap().unwrap();
        assert!(matches!(
            second.payload,
            OutboundLeasePayload::ProtocolReply(ref reply) if reply == &vec![2; 4]
        ));
        outbox.ack_lease(second.lease_id).unwrap();
        assert_eq!(outbox.pending_protocol_reply_count(), 0);
        assert_eq!(outbox.pending_protocol_reply_bytes(), 0);
        assert!(outbox.lease_next().unwrap().is_none());
    }

    #[test]
    fn pending_protocol_replies_share_the_configured_outbox_ceilings() {
        let mut outbox = CollaborationOutbox::with_ceilings(2, 16);
        let reservation = outbox.reserve_protocol_replies(1, 10).unwrap();
        outbox.install_protocol_replies(reservation, 3, vec![vec![7; 10]]);

        // Message-count ceiling counts the pending protocol entry.
        let reservation = outbox.reserve_document_update(4, 1).unwrap();
        assert_eq!(
            outbox.reserve_protocol_replies(1, 1).unwrap_err(),
            OutboxReservationError::Saturated {
                field: OUTBOX_MESSAGES_FIELD,
                limit: 2,
                actual: 3,
            },
        );
        drop(reservation);

        // Byte ceiling counts the pending protocol bytes.
        assert_eq!(
            outbox.reserve_document_update(5, 7).unwrap_err(),
            OutboxReservationError::Saturated {
                field: OUTBOX_BYTES_FIELD,
                limit: 16,
                actual: 17,
            },
        );
        outbox.reserve_document_update(5, 6).unwrap();
    }

    #[test]
    fn clear_protocol_replies_drops_only_transport_scoped_entries() {
        let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
        let document = outbox.reserve_document_update(1, 8).unwrap();
        outbox.install(document, vec![1; 8]);
        let replies = outbox.reserve_protocol_replies(2, 12).unwrap();
        outbox.install_protocol_replies(replies, 2, vec![vec![2; 7], vec![3; 5]]);
        assert_eq!(outbox.pending_protocol_reply_count(), 2);
        assert_eq!(outbox.pending_protocol_reply_bytes(), 12);

        outbox.clear_protocol_replies();

        assert_eq!(outbox.pending_protocol_reply_count(), 0);
        assert_eq!(outbox.pending_protocol_reply_bytes(), 0);
        // Document entries and their accounting are never touched.
        assert_eq!(outbox.pending_document_update_count(), 1);
        assert_eq!(outbox.pending_document_update_bytes(), 8);
        assert!(outbox.has_pending_document_updates());
        let document_lease = outbox.lease_next().unwrap().unwrap();
        assert!(matches!(
            document_lease.payload,
            OutboundLeasePayload::DocumentUpdate(ref update) if update == &vec![1; 8]
        ));
        outbox.ack_lease(document_lease.lease_id).unwrap();
        // Clearing an already empty queue is a no-op.
        outbox.clear_protocol_replies();
        assert_eq!(outbox.pending_protocol_reply_count(), 0);
    }

    #[test]
    fn allocation_failpoint_rejects_both_reservation_kinds_atomically() {
        let mut outbox = CollaborationOutbox::with_ceilings(4, 64);
        set_reservation_allocation_failure_for_test(true);
        assert_eq!(
            outbox.reserve_document_update(1, 8).unwrap_err(),
            OutboxReservationError::Allocation,
        );
        assert_eq!(
            outbox.reserve_protocol_replies(1, 8).unwrap_err(),
            OutboxReservationError::Allocation,
        );
        set_reservation_allocation_failure_for_test(false);
        assert_eq!(outbox.reserved_messages(), 0);
        assert_eq!(outbox.reserved_bytes(), 0);
        assert_eq!(outbox.last_reserved_upper_bound_for_test(), None);
    }

    #[test]
    fn lease_id_exhaustion_never_wraps() {
        let mut outbox = CollaborationOutbox::with_ceilings(2, 16);
        let first = outbox.reserve_document_update(1, 1).unwrap();
        outbox.install(first, vec![1]);
        let second = outbox.reserve_document_update(2, 1).unwrap();
        outbox.install(second, vec![2]);
        outbox.next_lease_id = Some(u64::MAX);

        let final_lease = outbox.lease_next().unwrap().unwrap();
        assert_eq!(final_lease.lease_id, OutboundLeaseId(u64::MAX));
        outbox.ack_lease(final_lease.lease_id).unwrap();
        assert_eq!(outbox.next_lease_id, None);
        assert_eq!(outbox.pending_document_update_count(), 1);
        assert_eq!(outbox.pending_document_update_bytes(), 1);
        assert_eq!(
            outbox.lease_next(),
            Err(OutboundLeaseError::LeaseIdExhausted)
        );
        assert_eq!(outbox.pending_document_update_count(), 1);
        assert_eq!(outbox.pending_document_update_bytes(), 1);
    }

    #[test]
    #[should_panic(expected = "active document lease accounting underflow")]
    fn ack_lease_rejects_document_accounting_underflow() {
        let mut outbox = CollaborationOutbox::with_ceilings(1, 16);
        let reservation = outbox.reserve_document_update(1, 2).unwrap();
        outbox.install(reservation, vec![1, 2]);
        let lease = outbox.lease_next().unwrap().unwrap();
        outbox.pending_bytes = 0;

        outbox.ack_lease(lease.lease_id).unwrap();
    }

    #[test]
    #[should_panic(expected = "active protocol lease accounting underflow")]
    fn ack_lease_rejects_protocol_accounting_underflow() {
        let mut outbox = CollaborationOutbox::with_ceilings(1, 16);
        let reservation = outbox.reserve_protocol_replies(1, 2).unwrap();
        outbox.install_protocol_replies(reservation, 1, vec![vec![1, 2]]);
        let lease = outbox.lease_next().unwrap().unwrap();
        outbox.pending_protocol_bytes = 0;

        outbox.ack_lease(lease.lease_id).unwrap();
    }
}
