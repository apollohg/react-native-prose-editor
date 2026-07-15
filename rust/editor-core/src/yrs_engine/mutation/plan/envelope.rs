pub(crate) fn crdt_envelope<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    snapshot_scan_reservation: usize,
) -> OperationResult<CrdtEnvelope> {
    // Reserve against the already-admitted encoded-state ceiling before the
    // public snapshot API traverses the store. The measured public metadata
    // scan is reconciled into the compiler's input-work budget by the caller.
    if snapshot_scan_reservation == 0 {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            None,
            "maxEncodedStateBytes",
            0,
            1,
        ));
    }
    let snapshot = txn.snapshot();
    let mut clients = HashSet::new();
    let mut state_clock_units = 0u64;
    let mut scan_work = 0usize;
    for (client, clock) in snapshot.state_map.iter() {
        clients.insert(*client);
        state_clock_units = state_clock_units
            .checked_add(u64::from(*clock))
            .ok_or_else(|| snapshot_metric_overflow(request_id))?;
        scan_work = scan_work
            .checked_add(usize::try_from(*clock).unwrap_or(usize::MAX))
            .and_then(|work| work.checked_add(1))
            .ok_or_else(|| snapshot_metric_overflow(request_id))?;
    }
    let mut deleted_clock_units = 0u64;
    for (client, ranges) in snapshot.delete_set.iter() {
        clients.insert(*client);
        scan_work = scan_work
            .checked_add(1)
            .ok_or_else(|| snapshot_metric_overflow(request_id))?;
        for range in ranges {
            let range_len = range
                .end
                .checked_sub(range.start)
                .ok_or_else(|| snapshot_metric_overflow(request_id))?;
            deleted_clock_units = deleted_clock_units
                .checked_add(u64::from(range_len))
                .ok_or_else(|| snapshot_metric_overflow(request_id))?;
            scan_work = scan_work
                .checked_add(1)
                .ok_or_else(|| snapshot_metric_overflow(request_id))?;
        }
    }
    let live_clock_units = state_clock_units
        .checked_sub(deleted_clock_units)
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs snapshot delete clocks exceed observed state clocks",
            )
        })?;
    if scan_work > snapshot_scan_reservation {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            None,
            "maxEncodedStateBytes",
            u64::try_from(snapshot_scan_reservation).unwrap_or(u64::MAX),
            u64::try_from(scan_work).unwrap_or(u64::MAX),
        ));
    }
    Ok(CrdtEnvelope {
        live_clock_units,
        client_count: clients.len(),
        scan_work,
    })
}

pub(crate) fn crdt_clock_scan_reservation<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    reservation: usize,
) -> OperationResult<usize> {
    if txn.store().pending_update().is_some() || txn.store().pending_ds().is_some() {
        return Err(OperationError::engine_not_ready(request_id));
    }
    let state = txn.state_vector();
    let work = state.iter().try_fold(0usize, |work, (_, clock)| {
        work.checked_add(usize::try_from(*clock).ok()?)?
            .checked_add(1)
    });
    let work = work.ok_or_else(|| snapshot_metric_overflow(request_id))?;
    if work > reservation {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            None,
            "maxEncodedStateBytes",
            u64::try_from(reservation).unwrap_or(u64::MAX),
            u64::try_from(work).unwrap_or(u64::MAX),
        ));
    }
    Ok(work)
}

pub(super) fn capture_document_guard<T: ReadTxn>(
    request_id: u64,
    txn: &T,
) -> OperationResult<DocumentGuard> {
    if txn.store().pending_update().is_some() || txn.store().pending_ds().is_some() {
        return Err(OperationError::engine_not_ready(request_id));
    }
    let snapshot = txn.snapshot();
    let state_clock_work = snapshot_state_clock_work(request_id, &snapshot.state_map)?;
    Ok(DocumentGuard {
        store_token: txn.store() as *const _ as usize,
        snapshot,
        state_clock_work,
    })
}

pub(super) fn snapshot_state_clock_work(
    request_id: u64,
    state: &yrs::StateVector,
) -> OperationResult<usize> {
    state
        .iter()
        .try_fold(0usize, |work, (_, clock)| {
            work.checked_add(usize::try_from(*clock).ok()?)?
                .checked_add(1)
        })
        .ok_or_else(|| snapshot_metric_overflow(request_id))
}

fn snapshot_metric_overflow(request_id: u64) -> OperationError {
    OperationError::engine_invariant_failed(request_id, None, "Yrs snapshot metric overflow")
}
