#[cfg(test)]
use super::test_hooks::FAIL_OUTBOUND_STAGING_COPY;
use crate::yrs_engine;

/// Outbound Update-v1 capture seam for one durable local commit.
///
/// A detached sink is a free no-op, so shipped default-feature paths keep
/// byte-identical behavior and cost by construction. An attached sink
/// (the production collaboration runtime) reserves bounded outbox count/bytes/queue
/// storage from the compiler's conservative `outbound_update_upper_bound`
/// and stages a copy of the captured Update-v1 strictly BEFORE the
/// irreversible Yrs write; after the commit installs, the append is
/// infallible. Dropping the sink without committing releases the
/// reservation, keeping rejected operations atomic.
pub(crate) struct OutboundUpdateSink<'a> {
    target: Option<OutboundSinkTarget<'a>>,
}

struct OutboundSinkTarget<'a> {
    outbox: &'a mut crate::collaboration_runtime::CollaborationOutbox,
    staged: Option<(
        crate::collaboration_runtime::outbox::OutboxReservation,
        Vec<u8>,
    )>,
}

impl<'a> OutboundUpdateSink<'a> {
    pub(crate) fn detached() -> Self {
        Self { target: None }
    }

    pub(crate) fn attached(
        outbox: &'a mut crate::collaboration_runtime::CollaborationOutbox,
    ) -> Self {
        Self {
            target: Some(OutboundSinkTarget {
                outbox,
                staged: None,
            }),
        }
    }

    /// Sink over an optionally attached collaboration outbox: sessions
    /// without a runtime edit through a detached (no-op) sink.
    pub(crate) fn from_optional_outbox(
        outbox: Option<&'a mut crate::collaboration_runtime::CollaborationOutbox>,
    ) -> Self {
        match outbox {
            Some(outbox) => Self::attached(outbox),
            None => Self::detached(),
        }
    }

    /// True when a collaboration outbox is attached; callers skip
    /// capture-only encoding work when detached.
    pub(crate) fn is_attached(&self) -> bool {
        self.target.is_some()
    }

    /// Fallible pre-write step: admit outbox count/bytes/storage from the
    /// conservative bound and stage a bounded copy of the captured update.
    pub(crate) fn reserve_and_stage(
        &mut self,
        request_id: u64,
        upper_bound_bytes: usize,
        update_v1: &[u8],
    ) -> yrs_engine::OperationResult<()> {
        if let Some(target) = self.target.as_mut() {
            debug_assert!(
                target.staged.is_none(),
                "one durable commit stages at most one outbound update",
            );
            let reservation = target
                .outbox
                .reserve_document_update(request_id, upper_bound_bytes)
                .map_err(|error| outbox_reservation_operation_error(request_id, error))?;
            #[cfg(test)]
            if FAIL_OUTBOUND_STAGING_COPY.with(std::cell::Cell::get) {
                return Err(yrs_engine::OperationError::operation_resource_exhausted(
                    request_id,
                    "pendingOutboxUpdateBytes",
                    "injected outbound staging copy allocation failure",
                ));
            }
            let mut staged = Vec::new();
            staged.try_reserve_exact(update_v1.len()).map_err(|_| {
                yrs_engine::OperationError::operation_resource_exhausted(
                    request_id,
                    "pendingOutboxUpdateBytes",
                    "captured outbound update could not allocate its staging copy",
                )
            })?;
            staged.extend_from_slice(update_v1);
            target.staged = Some((reservation, staged));
        }
        Ok(())
    }

    /// Infallible post-commit append of the staged update. No-op when
    /// detached or when the commit reserved nothing.
    pub(crate) fn commit_staged(&mut self) {
        if let Some(target) = self.target.as_mut() {
            if let Some((reservation, update)) = target.staged.take() {
                target.outbox.install(reservation, update);
            }
        }
    }
}

/// Frozen error mapping for pre-write outbox reservation failures:
/// deterministic ceiling saturation is `OPERATION_LIMIT_EXCEEDED` on the
/// configured collaboration-limit field; storage-reservation failure is the
/// allocation-class `OPERATION_RESOURCE_EXHAUSTED`.
fn outbox_reservation_operation_error(
    request_id: u64,
    error: crate::collaboration_runtime::outbox::OutboxReservationError,
) -> yrs_engine::OperationError {
    use crate::collaboration_runtime::outbox::OutboxReservationError;
    match error {
        OutboxReservationError::Saturated {
            field,
            limit,
            actual,
        } => yrs_engine::OperationError::operation_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        ),
        OutboxReservationError::Allocation => {
            yrs_engine::OperationError::operation_resource_exhausted(
                request_id,
                "pendingOutboxReservation",
                "collaboration outbox reservation could not allocate storage",
            )
        }
    }
}

// The methods in this block carrying `#[allow(dead_code)]` are the engine's
// plain convenience surface and test-support probes: they are exercised by
// crate tests and the cfg(test) bridge/document-api test support, while
// production entry points reach the same behavior through the
// `_with_outbox`/prepared variants used by `ffi_v2`. The constructors and the
// production seams in this block are genuinely live and carry no allow.
