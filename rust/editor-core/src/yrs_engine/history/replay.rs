impl YrsHistory {
    pub(crate) fn pre_admit_excluded(
        &mut self,
        request_id: u64,
        origin: TransactionOrigin,
        work_units: u64,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
    ) -> OperationResult<PreparedExcludedHistoryAdmission> {
        let reserved_update = self.allocate_replay_update(request_id, update_bytes_bound)?;
        let event_bytes_bound =
            self.replay_event_bytes_bound(request_id, reserved_update.capacity())?;
        let (disposition, replay_slot) =
            if work_units > self.limits.max_undo_retained_units || self.rebase_before_next_event {
                (ExcludedReplayDisposition::InvalidateAfterCommit, None)
            } else {
                let rolls = self.replay_event_would_roll(event_bytes_bound, work_units);
                let replay_slot = self.prepare_replay_event_slot(request_id, rolls)?;
                let disposition = if rolls {
                    let owned_baseline =
                        reserve_replay_roll_baseline(request_id, current_encoded_state)?;
                    ExcludedReplayDisposition::Roll { owned_baseline }
                } else {
                    ExcludedReplayDisposition::Append
                };
                (disposition, Some(replay_slot))
            };
        Ok(PreparedExcludedHistoryAdmission {
            origin,
            work_units,
            reserved_update,
            disposition,
            replay_slot,
        })
    }

    /// Compiled local mutations preserve the historical bounded-exclusion
    /// semantics: a pending rebase rolls to the admitted live baseline and the
    /// newly compiled excluded event is retained in the new epoch. Remote
    /// callers keep using `pre_admit_excluded`, where a pending rebase remains
    /// an invalidate-after-commit signal.
    pub(crate) fn pre_admit_compiled_excluded(
        &mut self,
        request_id: u64,
        origin: TransactionOrigin,
        work_units: u64,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
    ) -> OperationResult<PreparedExcludedHistoryAdmission> {
        let reserved_update = self.allocate_replay_update(request_id, update_bytes_bound)?;
        let event_bytes_bound =
            self.replay_event_bytes_bound(request_id, reserved_update.capacity())?;
        let (disposition, replay_slot) = if work_units > self.limits.max_undo_retained_units {
            (ExcludedReplayDisposition::InvalidateAfterCommit, None)
        } else {
            let rolls = self.replay_event_would_roll(event_bytes_bound, work_units);
            let replay_slot = self.prepare_replay_event_slot(request_id, rolls)?;
            let disposition = if rolls {
                let owned_baseline =
                    reserve_replay_roll_baseline(request_id, current_encoded_state)?;
                ExcludedReplayDisposition::Roll { owned_baseline }
            } else {
                ExcludedReplayDisposition::Append
            };
            (disposition, Some(replay_slot))
        };
        Ok(PreparedExcludedHistoryAdmission {
            origin,
            work_units,
            reserved_update,
            disposition,
            replay_slot,
        })
    }

    pub(crate) fn finish_prepared_excluded(
        &mut self,
        admission: PreparedExcludedHistoryAdmission,
        accepted_update: Vec<u8>,
    ) {
        let PreparedExcludedHistoryAdmission {
            origin,
            work_units,
            mut reserved_update,
            disposition,
            replay_slot,
        } = admission;
        match disposition {
            ExcludedReplayDisposition::InvalidateAfterCommit => {
                self.invalidate_replay_after_mutation();
                return;
            }
            ExcludedReplayDisposition::Roll { owned_baseline } => {
                self.roll_epoch(owned_baseline);
            }
            ExcludedReplayDisposition::Append => {}
        }
        assert!(
            accepted_update.len() <= reserved_update.capacity(),
            "admitted excluded update exceeds its exact pre-write reservation"
        );
        reserved_update.extend_from_slice(&accepted_update);
        self.push_prepared_replay_event(
            replay_slot.expect("retained excluded admission owns a replay slot"),
            ReplayEvent::Excluded {
                update: reserved_update,
                origin,
                work_units,
            },
        );
    }

    /// Records a command boundary even when the command changed only editor
    /// state (for example, collapsed stored marks) and captured no Yrs structs.
    pub(crate) fn force_next_capture_boundary(&mut self) {
        self.manager.reset();
        self.force_next_boundary = true;
    }

    pub(crate) fn prepare_boundary(
        &mut self,
        request_id: u64,
        accepted_encoded_state: Vec<u8>,
    ) -> OperationResult<PreparedBoundary> {
        #[cfg(test)]
        if FAIL_BOUNDARY_RESERVATION.with(Cell::get) {
            return Err(OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                "injected command-boundary reservation failure",
            ));
        }
        self.replay_events.try_reserve(1).map_err(|error| {
            OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                format!("cannot reserve command boundary: {error}"),
            )
        })?;
        let next_count = self.replay_events.len().saturating_add(1);
        let next_bytes = self
            .replay_bytes
            .saturating_add(ReplayEvent::Boundary.encoded_bytes());
        Ok(PreparedBoundary {
            accepted_encoded_state,
            roll_epoch: next_count >= self.event_ceiling()
                || next_bytes > self.max_encoded_state_bytes,
        })
    }

    pub(crate) fn commit_boundary(&mut self, prepared: PreparedBoundary) {
        if prepared.roll_epoch {
            self.roll_epoch(prepared.accepted_encoded_state);
        } else {
            self.force_next_capture_boundary();
            self.push_replay_event(ReplayEvent::Boundary);
        }
    }

    #[cfg(test)]
    pub(crate) fn replay_audit_for_test(&self) -> (usize, usize, bool) {
        (
            self.replay_events.len(),
            self.replay_bytes,
            self.force_next_boundary,
        )
    }

    #[cfg(test)]
    pub(crate) fn compact_replay_event_capacity_for_test(&mut self) {
        self.replay_events.shrink_to_fit();
        assert_eq!(self.replay_events.len(), self.replay_events.capacity());
    }

    #[cfg(test)]
    pub(crate) fn replay_ledger_allocation_audit_for_test(&self) -> ReplayLedgerAllocationAudit {
        let events = self
            .replay_events
            .iter()
            .map(|event| match event {
                ReplayEvent::Recorded {
                    update, metadata, ..
                } => ReplayEventAllocationIdentity {
                    kind: 1,
                    update_allocation: update.as_ptr() as usize,
                    update_len: update.len(),
                    update_capacity: update.capacity(),
                    metadata_identity: metadata.identity(),
                },
                ReplayEvent::Excluded { update, .. } => ReplayEventAllocationIdentity {
                    kind: 2,
                    update_allocation: update.as_ptr() as usize,
                    update_len: update.len(),
                    update_capacity: update.capacity(),
                    metadata_identity: 0,
                },
                ReplayEvent::Action(_) => ReplayEventAllocationIdentity {
                    kind: 3,
                    update_allocation: 0,
                    update_len: 0,
                    update_capacity: 0,
                    metadata_identity: 0,
                },
                ReplayEvent::Boundary => ReplayEventAllocationIdentity {
                    kind: 4,
                    update_allocation: 0,
                    update_len: 0,
                    update_capacity: 0,
                    metadata_identity: 0,
                },
            })
            .collect();
        ReplayLedgerAllocationAudit {
            len: self.replay_events.len(),
            capacity: self.replay_events.capacity(),
            allocation: self.replay_events.as_ptr() as usize,
            events,
        }
    }

    #[cfg(test)]
    pub(crate) fn force_rebase_before_next_event_for_test(&mut self) {
        self.rebase_before_next_event = true;
    }

    #[cfg(test)]
    pub(crate) fn replace_next_undo_stored_marks_for_test(&mut self, marks: Vec<Mark>) {
        let metadata = self
            .manager
            .undo_stack()
            .last()
            .expect("history stored-mark tamper requires one undo item")
            .meta();
        let mut slots = metadata.slots();
        let mut before = slots
            .before
            .as_ref()
            .and_then(HistorySnapshotSlot::get)
            .cloned()
            .expect("history stored-mark tamper requires a sealed before snapshot");
        before.stored_marks = Some(marks);
        slots.before = Some(HistorySnapshotSlot::initialized(before));
        metadata.replace_slots(slots);
    }

    #[cfg(test)]
    pub(crate) fn compiled_excluded_rebase_audit_for_test(&self) -> (bool, &[u8], usize, bool) {
        (
            self.rebase_before_next_event,
            &self.epoch_baseline,
            self.replay_events.len(),
            matches!(
                self.replay_events.last(),
                Some(ReplayEvent::Excluded { .. })
            ),
        )
    }

    pub(crate) fn perform(&mut self, action: HistoryAction) -> Option<HistorySnapshotSlot> {
        let available = match action {
            HistoryAction::Undo => self.manager.can_undo(),
            HistoryAction::Redo => self.manager.can_redo(),
        };
        if !available {
            return None;
        }
        *self
            .popped
            .lock()
            .expect("popped history metadata lock poisoned") = None;
        *self
            .pending_pop
            .lock()
            .expect("pending history pop lock poisoned") = Some(HistoryMetadata::default());
        let changed = match action {
            HistoryAction::Undo => self.manager.undo_blocking(),
            HistoryAction::Redo => self.manager.redo_blocking(),
        };
        self.pending_pop
            .lock()
            .expect("pending history pop lock poisoned")
            .take();
        if !changed {
            return None;
        }
        let (kind, value) = self
            .popped
            .lock()
            .expect("popped history metadata lock poisoned")
            .take()
            .expect("changed Yrs history pop supplies metadata");
        self.reset_grouping();
        match kind {
            EventKind::Undo => value.before,
            EventKind::Redo => value.after,
        }
    }

    pub(crate) fn undo(&mut self) -> HistoryPop {
        let restored = self.perform(HistoryAction::Undo);
        HistoryPop {
            changed: restored.is_some(),
            restored,
        }
    }

    pub(crate) fn redo(&mut self) -> HistoryPop {
        let restored = self.perform(HistoryAction::Redo);
        HistoryPop {
            changed: restored.is_some(),
            restored,
        }
    }

    pub(crate) fn retained_units(&self, request_id: u64) -> OperationResult<u64> {
        stack_units(self.manager.undo_stack(), request_id)?
            .checked_add(stack_units(self.manager.redo_stack(), request_id)?)
            .ok_or_else(|| {
                OperationError::operation_limit_exceeded(
                    request_id,
                    None,
                    "maxUndoRetainedUnits",
                    self.limits.max_undo_retained_units,
                    u64::MAX,
                )
            })
    }

    pub(crate) fn accept_action(
        &mut self,
        request_id: u64,
        action: HistoryAction,
        accepted_encoded_state: Vec<u8>,
    ) -> OperationResult<()> {
        #[cfg(test)]
        if FAIL_ACCEPTED_ACTION_RESERVATION.with(Cell::get) {
            return Err(OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                "injected accepted history action reservation failure",
            ));
        }
        self.replay_events.try_reserve(1).map_err(|error| {
            OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                format!("cannot reserve accepted history action: {error}"),
            )
        })?;
        let event = ReplayEvent::Action(action);
        let event_ceiling = self.event_ceiling();
        let next_count = self.replay_events.len().saturating_add(1);
        let next_bytes = self.replay_bytes.saturating_add(event.encoded_bytes());
        let retained_metadata = self
            .replay_metadata_bytes
            .saturating_add(self.unmirrored_stack_metadata_bytes(request_id)?);
        if next_count >= event_ceiling
            || next_bytes > self.max_encoded_state_bytes
            || retained_metadata > self.limits.max_derived_output_bytes
        {
            self.roll_epoch(accepted_encoded_state);
        } else {
            self.push_replay_event(event);
        }
        Ok(())
    }

    #[cfg(test)]
    fn reserve_replay_event(
        &mut self,
        request_id: u64,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
        work_units: u64,
        excluded: bool,
    ) -> OperationResult<Vec<u8>> {
        let update = self.allocate_replay_update(request_id, update_bytes_bound)?;
        let event_bytes_bound = self.replay_event_bytes_bound(request_id, update.capacity())?;
        if work_units > self.limits.max_undo_retained_units {
            if excluded {
                // The mutation itself is allowed, but cannot be retained in a
                // replay epoch. Preserve live history until it commits, then
                // invalidate it atomically in `finish_excluded`.
                self.rebase_before_next_event = true;
                return Ok(update);
            }
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxUndoRetainedUnits",
                self.limits.max_undo_retained_units,
                work_units,
            ));
        }
        self.reserve_replay_event_slot(request_id)?;
        if self.replay_event_would_roll(event_bytes_bound, work_units) {
            self.roll_epoch(current_encoded_state.to_vec());
        }
        Ok(update)
    }

    fn allocate_replay_update(
        &self,
        request_id: u64,
        update_bytes_bound: usize,
    ) -> OperationResult<Vec<u8>> {
        #[cfg(test)]
        if FAIL_REPLAY_UPDATE_ALLOCATION.with(Cell::get) {
            return Err(OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                "injected history replay update allocation failure",
            ));
        }
        let mut update = Vec::new();
        update
            .try_reserve_exact(update_bytes_bound)
            .map_err(|error| {
                OperationError::operation_resource_exhausted(
                    request_id,
                    "historyReplay",
                    format!("cannot reserve bounded history update payload: {error}"),
                )
            })?;
        Ok(update)
    }

    fn replay_event_bytes_bound(
        &self,
        request_id: u64,
        update_capacity: usize,
    ) -> OperationResult<usize> {
        let event_bytes_bound = update_capacity.checked_add(1).ok_or_else(|| {
            encoded_limit_error(request_id, self.max_encoded_state_bytes, usize::MAX)
        })?;
        if event_bytes_bound > self.max_encoded_state_bytes {
            return Err(encoded_limit_error(
                request_id,
                self.max_encoded_state_bytes,
                event_bytes_bound,
            ));
        }
        Ok(event_bytes_bound)
    }

    #[cfg(test)]
    fn reserve_replay_event_slot(&mut self, request_id: u64) -> OperationResult<()> {
        self.replay_events.try_reserve(1).map_err(|error| {
            OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                format!("cannot reserve bounded history event slot: {error}"),
            )
        })
    }

    fn prepare_replay_event_slot(
        &self,
        request_id: u64,
        clears_before_install: bool,
    ) -> OperationResult<PreparedReplayEventSlot> {
        let required_len = if clears_before_install {
            1
        } else {
            self.replay_events.len().checked_add(1).ok_or_else(|| {
                OperationError::operation_resource_exhausted(
                    request_id,
                    "historyReplay",
                    "bounded history event slot length overflow",
                )
            })?
        };
        if self.replay_events.capacity() >= required_len {
            return Ok(PreparedReplayEventSlot::ExistingSpare);
        }
        #[cfg(test)]
        if FAIL_EVENT_REPLACEMENT_RESERVATION.with(Cell::get) {
            return Err(OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                "injected bounded history event replacement reservation failure",
            ));
        }
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(required_len)
            .map_err(|error| {
                OperationError::operation_resource_exhausted(
                    request_id,
                    "historyReplay",
                    format!("cannot reserve bounded history event replacement: {error}"),
                )
            })?;
        Ok(PreparedReplayEventSlot::Replacement(replacement))
    }

    fn push_prepared_replay_event(
        &mut self,
        replay_slot: PreparedReplayEventSlot,
        event: ReplayEvent,
    ) {
        match replay_slot {
            PreparedReplayEventSlot::ExistingSpare => {
                assert!(
                    self.replay_events.len() < self.replay_events.capacity(),
                    "admitted history event lost its existing replay slot"
                );
            }
            PreparedReplayEventSlot::Replacement(mut replacement) => {
                assert!(
                    replacement.capacity() > self.replay_events.len(),
                    "admitted history replacement lost its replay slot"
                );
                replacement.append(&mut self.replay_events);
                self.replay_events = replacement;
            }
        }
        self.push_replay_event(event);
    }

    fn replay_event_would_roll(&self, event_bytes_bound: usize, work_units: u64) -> bool {
        self.rebase_before_next_event
            || self.replay_bytes.saturating_add(event_bytes_bound) > self.max_encoded_state_bytes
            || self.replay_work_units.saturating_add(work_units)
                > self.limits.max_undo_retained_units
            || self.replay_events.len().saturating_add(1) > self.event_ceiling()
    }

    #[allow(clippy::manual_saturating_arithmetic)]
    fn event_ceiling(&self) -> usize {
        const EVENTS_PER_GROUP: usize = 8;
        self.limits
            .max_undo_groups
            .checked_mul(EVENTS_PER_GROUP)
            .unwrap_or(usize::MAX)
    }

    fn push_replay_event(&mut self, event: ReplayEvent) {
        self.replay_bytes = self.replay_bytes.saturating_add(event.encoded_bytes());
        self.replay_work_units = self.replay_work_units.saturating_add(event.work_units());
        self.replay_events.push(event);
    }

    fn invalidate_replay_after_mutation(&mut self) {
        self.manager.clear_all();
        self.manager.reset();
        self.reset_grouping();
        self.replay_events.clear();
        self.replay_bytes = 0;
        self.replay_work_units = 0;
        self.replay_metadata_bytes = 0;
        self.pending_replay_event = None;
        self.rebase_before_next_event = true;
    }

    fn roll_epoch(&mut self, baseline: Vec<u8>) {
        self.manager.clear_all();
        self.manager.reset();
        self.reset_grouping();
        self.epoch_baseline = baseline;
        self.replay_events.clear();
        self.replay_bytes = 0;
        self.replay_work_units = 0;
        self.replay_metadata_bytes = 0;
        self.pending_replay_event = None;
        self.rebase_before_next_event = false;
    }

    fn reset_grouping(&mut self) {
        self.last_capture_millis = None;
        self.last_class = None;
        self.last_origin = None;
        self.force_next_boundary = false;
        self.clock.release();
    }
}
