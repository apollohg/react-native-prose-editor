impl YrsHistory {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pre_admit_capture_limits(
        &self,
        request_id: u64,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        before_metadata_bytes: usize,
        after_metadata_bytes: usize,
    ) -> OperationResult<PreparedHistoryLimits> {
        self.pre_admit_capture_limits_at(
            request_id,
            origin,
            policy,
            class,
            undo_units_bound,
            before_metadata_bytes,
            after_metadata_bytes,
            self.clock.now(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn pre_admit_capture_limits_at(
        &self,
        request_id: u64,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        before_metadata_bytes: usize,
        after_metadata_bytes: usize,
        now_millis: u64,
    ) -> OperationResult<PreparedHistoryLimits> {
        debug_assert_ne!(policy, HistoryPolicy::Skip);
        debug_assert_ne!(class, HistoryClass::Skip);
        let standalone_metadata_bytes = before_metadata_bytes
            .checked_add(after_metadata_bytes)
            .ok_or_else(|| metadata_limit_error(request_id, &self.limits, usize::MAX))?;
        if undo_units_bound > self.limits.max_undo_retained_units {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxUndoRetainedUnits",
                self.limits.max_undo_retained_units,
                undo_units_bound,
            ));
        }
        let compatible = self.capture_is_compatible(origin, policy, class, now_millis);
        let prospective_metadata_increment = after_metadata_bytes
            .checked_add(if compatible { 0 } else { before_metadata_bytes })
            .ok_or_else(|| metadata_limit_error(request_id, &self.limits, usize::MAX))?;
        let pending_metadata_bytes = self
            .replay_metadata_bytes
            .checked_add(self.unmirrored_stack_metadata_bytes(request_id)?)
            .and_then(|bytes| bytes.checked_add(prospective_metadata_increment))
            .unwrap_or(usize::MAX);
        let should_roll = pending_metadata_bytes > self.limits.max_derived_output_bytes
            || self.capture_would_roll(
                request_id,
                origin,
                policy,
                class,
                undo_units_bound,
                before_metadata_bytes,
                after_metadata_bytes,
                now_millis,
            )?;
        if (!compatible || should_roll)
            && standalone_metadata_bytes > self.limits.max_derived_output_bytes
        {
            return Err(metadata_limit_error(
                request_id,
                &self.limits,
                standalone_metadata_bytes,
            ));
        }
        Ok(PreparedHistoryLimits {
            now_millis,
            standalone_metadata_bytes,
            prospective_metadata_increment,
            compatible,
            should_roll,
        })
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub(crate) fn prepare_capture(
        &mut self,
        request_id: u64,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        before: Option<HistoryLocalState>,
        after_metadata_bytes: usize,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
    ) -> OperationResult<Origin> {
        self.prepare_capture_with_limits(
            request_id,
            origin,
            policy,
            class,
            undo_units_bound,
            before,
            after_metadata_bytes,
            current_encoded_state,
            update_bytes_bound,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)]
    pub(crate) fn prepare_capture_with_limits(
        &mut self,
        request_id: u64,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        before: Option<HistoryLocalState>,
        after_metadata_bytes: usize,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
        prepared_limits: Option<PreparedHistoryLimits>,
    ) -> OperationResult<Origin> {
        debug_assert_ne!(policy, HistoryPolicy::Skip);
        debug_assert_ne!(class, HistoryClass::Skip);
        let before = before.ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "recorded history transaction has no local state metadata",
            )
        })?;
        let limits = if let Some(prepared) = prepared_limits {
            let live = self.pre_admit_capture_limits_at(
                request_id,
                origin,
                policy,
                class,
                undo_units_bound,
                before.metadata_bytes,
                after_metadata_bytes,
                prepared.now_millis,
            )?;
            if live != prepared {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared history limit admission is stale",
                ));
            }
            prepared
        } else {
            self.pre_admit_capture_limits(
                request_id,
                origin,
                policy,
                class,
                undo_units_bound,
                before.metadata_bytes,
                after_metadata_bytes,
            )?
        };
        let now = limits.now_millis;
        let standalone_metadata_bytes = limits.standalone_metadata_bytes;
        let should_roll = limits.should_roll;
        let reserved_update = self.allocate_replay_update(request_id, update_bytes_bound)?;
        let event_bytes_bound =
            self.replay_event_bytes_bound(request_id, reserved_update.capacity())?;
        let reservation_would_roll =
            self.replay_event_would_roll(event_bytes_bound, undo_units_bound);
        if reservation_would_roll
            && standalone_metadata_bytes > self.limits.max_derived_output_bytes
        {
            return Err(metadata_limit_error(
                request_id,
                &self.limits,
                standalone_metadata_bytes,
            ));
        }
        let rolls = should_roll || reservation_would_roll;
        let owned_baseline = rolls
            .then(|| reserve_replay_roll_baseline(request_id, current_encoded_state))
            .transpose()?;
        let replay_slot = self.prepare_replay_event_slot(request_id, rolls)?;
        if let Some(baseline) = owned_baseline {
            self.roll_epoch(baseline);
        }
        self.clock.latch_at(now);
        let compatible = self.capture_is_compatible(origin, policy, class, now);
        let metadata_increment = after_metadata_bytes
            .checked_add(if compatible { 0 } else { before.metadata_bytes })
            .expect("standalone history metadata was checked before capture");
        let metadata = HistoryMetadata::capture(before.clone());
        let origin_value = self.begin_capture(origin, policy, class, Some(now), metadata.clone());
        self.pending_replay_event = Some(PendingReplayEvent::Recorded {
            origin,
            policy,
            class,
            undo_units_bound,
            capture_millis: now,
            metadata,
            update: reserved_update,
            metadata_increment,
            replay_slot,
        });
        Ok(origin_value)
    }

    fn begin_capture(
        &mut self,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        capture_millis: Option<u64>,
        metadata: HistoryMetadata,
    ) -> Origin {
        let now = capture_millis.unwrap_or_else(|| self.clock.latch());
        self.clock.latch_at(now);
        let compatible = policy == HistoryPolicy::Auto
            && !self.force_next_boundary
            && origin == TransactionOrigin::LocalInput
            && self.last_origin == Some(TransactionOrigin::LocalInput)
            && self.last_class == Some(class)
            && matches!(class, HistoryClass::Insert | HistoryClass::Delete)
            && self
                .last_capture_millis
                .is_some_and(|last| now >= last && now - last < CAPTURE_TIMEOUT_MILLIS);
        if !compatible {
            self.manager.reset();
        }

        *self
            .pending_capture
            .lock()
            .expect("pending history capture lock poisoned") = Some(metadata);
        self.last_capture_millis = Some(now);
        self.last_class = Some(class);
        self.last_origin = Some(origin);
        self.force_next_boundary = policy == HistoryPolicy::Boundary;
        Self::recorded_origin(origin)
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_would_roll(
        &self,
        request_id: u64,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        before_metadata_bytes: usize,
        after_metadata_bytes: usize,
        now: u64,
    ) -> OperationResult<bool> {
        let compatible = self.capture_is_compatible(origin, policy, class, now);
        let next_units =
            stack_units(self.manager.undo_stack(), request_id)?.saturating_add(undo_units_bound);
        let next_groups = self
            .manager
            .undo_stack()
            .len()
            .saturating_add(usize::from(!compatible));
        let current_metadata_bytes =
            stack_metadata_bytes(self.manager.undo_stack(), request_id, &self.limits)?;
        let standalone_metadata_bytes = before_metadata_bytes.saturating_add(after_metadata_bytes);
        let next_metadata_bytes = if compatible {
            let replaced_after = self
                .manager
                .undo_stack()
                .last()
                .and_then(|item| item.meta().slots().after)
                .and_then(|slot| slot.get().map(|snapshot| snapshot.metadata_bytes))
                .unwrap_or(0);
            current_metadata_bytes
                .checked_sub(replaced_after)
                .and_then(|value| value.checked_add(after_metadata_bytes))
                .unwrap_or(usize::MAX)
        } else {
            current_metadata_bytes.saturating_add(standalone_metadata_bytes)
        };
        Ok(next_units > self.limits.max_undo_retained_units
            || next_groups > self.limits.max_undo_groups
            || next_metadata_bytes > self.limits.max_derived_output_bytes)
    }

    fn capture_is_compatible(
        &self,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        now: u64,
    ) -> bool {
        policy == HistoryPolicy::Auto
            && !self.force_next_boundary
            && origin == TransactionOrigin::LocalInput
            && self.last_origin == Some(TransactionOrigin::LocalInput)
            && self.last_class == Some(class)
            && matches!(class, HistoryClass::Insert | HistoryClass::Delete)
            && self
                .last_capture_millis
                .is_some_and(|last| now >= last && now - last < CAPTURE_TIMEOUT_MILLIS)
    }

    fn unmirrored_stack_metadata_bytes(&self, request_id: u64) -> OperationResult<usize> {
        let mut seen = self
            .replay_events
            .iter()
            .flat_map(|event| match event {
                ReplayEvent::Recorded { metadata, .. } => {
                    let slots = metadata.slots();
                    [slots.before, slots.after]
                }
                ReplayEvent::Excluded { .. } | ReplayEvent::Action(_) | ReplayEvent::Boundary => {
                    [None, None]
                }
            })
            .flatten()
            .map(|slot| slot.identity())
            .collect::<HashSet<_>>();
        let mut total = 0usize;
        for item in self
            .manager
            .undo_stack()
            .iter()
            .chain(self.manager.redo_stack())
        {
            let slots = item.meta().slots();
            for slot in [slots.before, slots.after].into_iter().flatten() {
                if seen.insert(slot.identity()) {
                    let snapshot = slot.get().ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "retained history contains an unsealed snapshot slot",
                        )
                    })?;
                    total = total.checked_add(snapshot.metadata_bytes).ok_or_else(|| {
                        metadata_limit_error(request_id, &self.limits, usize::MAX)
                    })?;
                }
            }
        }
        Ok(total)
    }

    fn finish_replay_capture(&mut self) {
        self.pending_capture
            .lock()
            .expect("pending history capture lock poisoned")
            .take()
            .expect("replayed capture creates history metadata");
        self.pending_replay_event = None;
        self.clock.release();
        if self.force_next_boundary {
            self.manager.reset();
        }
    }

    pub(crate) fn finish_capture(&mut self, after: HistoryLocalState, update: Vec<u8>) {
        let pending = self
            .pending_capture
            .lock()
            .expect("pending history capture lock poisoned")
            .take();
        if let Some(metadata) = pending {
            metadata.set_after(after);
            if self.recording_replay_events {
                let Some(PendingReplayEvent::Recorded {
                    origin,
                    policy,
                    class,
                    undo_units_bound,
                    capture_millis,
                    metadata,
                    update: mut reserved_update,
                    metadata_increment,
                    replay_slot,
                }) = self.pending_replay_event.take()
                else {
                    self.invalidate_replay_after_mutation();
                    return;
                };
                assert!(
                    update.len() <= reserved_update.capacity(),
                    "admitted history update exceeds its exact pre-write reservation"
                );
                reserved_update.extend_from_slice(&update);
                let event = ReplayEvent::Recorded {
                    update: reserved_update,
                    origin,
                    policy,
                    class,
                    undo_units_bound,
                    capture_millis,
                    metadata,
                };
                self.push_prepared_replay_event(replay_slot, event);
                self.replay_metadata_bytes = self
                    .replay_metadata_bytes
                    .checked_add(metadata_increment)
                    .expect("history metadata reservation checked aggregate capacity");
            }
        }
        self.clock.release();
        if self.force_next_boundary {
            self.manager.reset();
        }
    }
}
