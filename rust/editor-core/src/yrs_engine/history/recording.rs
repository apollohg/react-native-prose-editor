impl YrsHistory {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pre_admit_recorded(
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
    ) -> OperationResult<PreparedRecordedHistoryAdmission> {
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
        let reserved_update = self.allocate_replay_update(request_id, update_bytes_bound)?;
        let event_bytes_bound =
            self.replay_event_bytes_bound(request_id, reserved_update.capacity())?;
        let reservation_would_roll =
            self.replay_event_would_roll(event_bytes_bound, undo_units_bound);
        if reservation_would_roll
            && limits.standalone_metadata_bytes > self.limits.max_derived_output_bytes
        {
            return Err(metadata_limit_error(
                request_id,
                &self.limits,
                limits.standalone_metadata_bytes,
            ));
        }
        let rolls = limits.should_roll || reservation_would_roll;
        let owned_baseline = rolls
            .then(|| reserve_replay_roll_baseline(request_id, current_encoded_state))
            .transpose()?;
        let replay_slot = self.prepare_replay_event_slot(request_id, rolls)?;
        let compatible =
            !rolls && self.capture_is_compatible(origin, policy, class, limits.now_millis);
        let metadata_increment = after_metadata_bytes
            .checked_add(if compatible { 0 } else { before.metadata_bytes })
            .expect("standalone history metadata was checked before capture");
        Ok(PreparedRecordedHistoryAdmission {
            origin,
            policy,
            class,
            undo_units_bound,
            capture_millis: limits.now_millis,
            metadata: HistoryMetadata::capture(before),
            reserved_update,
            metadata_increment,
            owned_baseline,
            compatible,
            replay_slot,
        })
    }

    pub(crate) fn begin_prepared_recorded(&mut self, prepared: PreparedRecordedHistoryAdmission) {
        let PreparedRecordedHistoryAdmission {
            origin,
            policy,
            class,
            undo_units_bound,
            capture_millis,
            metadata,
            reserved_update,
            metadata_increment,
            owned_baseline,
            compatible,
            replay_slot,
        } = prepared;
        if let Some(baseline) = owned_baseline {
            self.roll_epoch(baseline);
        }
        self.clock.latch_at(capture_millis);
        if !compatible {
            self.manager.reset();
        }
        *self
            .pending_capture
            .lock()
            .expect("pending history capture lock poisoned") = Some(metadata.clone());
        self.last_capture_millis = Some(capture_millis);
        self.last_class = Some(class);
        self.last_origin = Some(origin);
        self.force_next_boundary = policy == HistoryPolicy::Boundary;
        self.pending_replay_event = Some(PendingReplayEvent::Recorded {
            origin,
            policy,
            class,
            undo_units_bound,
            capture_millis,
            metadata,
            update: reserved_update,
            metadata_increment,
            replay_slot,
        });
    }
    pub(crate) fn new(
        doc: &Doc,
        fragment: &XmlFragmentRef,
        limits: EditingLimits,
        max_encoded_state_bytes: usize,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let clock = Arc::new(LatchingClock::new(clock));
        let baseline = encode_full_state(doc);
        Self::from_stacks(
            doc,
            fragment,
            limits,
            max_encoded_state_bytes,
            clock,
            Vec::new(),
            Vec::new(),
            baseline,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_stacks(
        doc: &Doc,
        fragment: &XmlFragmentRef,
        limits: EditingLimits,
        max_encoded_state_bytes: usize,
        clock: Arc<LatchingClock>,
        undo: Vec<StackItem<HistoryMetadata>>,
        redo: Vec<StackItem<HistoryMetadata>>,
        epoch_baseline: Vec<u8>,
        recording_replay_events: bool,
    ) -> Self {
        let pending_capture = Arc::new(Mutex::new(None::<HistoryMetadata>));
        let pending_pop = Arc::new(Mutex::new(None::<HistoryMetadata>));
        let popped = Arc::new(Mutex::new(None::<(EventKind, HistoryMetadataSlots)>));
        let mut tracked_origins = HashSet::new();
        tracked_origins.insert(Origin::from(INPUT_ORIGIN));
        tracked_origins.insert(Origin::from(COMMAND_ORIGIN));
        tracked_origins.insert(Origin::from(API_ORIGIN));
        let mut manager = UndoManager::with_options(UndoOptions {
            capture_timeout_millis: CAPTURE_TIMEOUT_MILLIS,
            tracked_origins,
            capture_transaction: None,
            timestamp: clock.clone(),
            init_undo_stack: undo,
            init_redo_stack: redo,
        });
        manager.expand_scope(doc, fragment);

        let added_capture = pending_capture.clone();
        let added_pop = pending_pop.clone();
        manager.observe_item_added_with(ADDED_OBSERVER, move |_, event| {
            if let Some(metadata) = added_capture
                .lock()
                .expect("pending history capture lock poisoned")
                .clone()
            {
                *event.meta_mut() = metadata;
            } else if let Some(metadata) = added_pop
                .lock()
                .expect("pending history pop lock poisoned")
                .clone()
            {
                *event.meta_mut() = metadata;
            }
        });

        let updated_capture = pending_capture.clone();
        manager.observe_item_updated_with(UPDATED_OBSERVER, move |_, event| {
            let pending = updated_capture
                .lock()
                .expect("pending history capture lock poisoned")
                .clone();
            if let Some(metadata) = pending {
                metadata.preserve_before_from(event.meta());
                *event.meta_mut() = metadata;
            }
        });

        let popped_target = pending_pop.clone();
        let popped_result = popped.clone();
        manager.observe_item_popped_with(POPPED_OBSERVER, move |_, event| {
            let slots = event.meta().slots();
            if let Some(target) = popped_target
                .lock()
                .expect("pending history pop lock poisoned")
                .clone()
            {
                target.replace_slots(slots.clone());
            }
            *popped_result
                .lock()
                .expect("popped history metadata lock poisoned") = Some((event.kind(), slots));
        });

        Self {
            manager,
            limits,
            clock,
            pending_capture,
            pending_pop,
            popped,
            last_capture_millis: None,
            last_class: None,
            last_origin: None,
            force_next_boundary: false,
            epoch_baseline,
            replay_events: Vec::new(),
            replay_bytes: 0,
            replay_work_units: 0,
            replay_metadata_bytes: 0,
            max_encoded_state_bytes,
            pending_replay_event: None,
            rebase_before_next_event: false,
            recording_replay_events,
        }
    }

    pub(crate) fn rebind(&mut self, doc: &Doc, fragment: &XmlFragmentRef) {
        let limits = self.limits.clone();
        let clock = self.clock.clone();
        let max_encoded_state_bytes = self.max_encoded_state_bytes;
        let baseline = encode_full_state(doc);
        *self = Self::from_stacks(
            doc,
            fragment,
            limits,
            max_encoded_state_bytes,
            clock,
            Vec::new(),
            Vec::new(),
            baseline,
            true,
        );
    }

    pub(crate) fn replay_into(
        &self,
        request_id: u64,
        doc: &Doc,
        fragment: &XmlFragmentRef,
    ) -> OperationResult<Self> {
        self.retained_units(request_id)?;
        #[cfg(test)]
        if FAIL_CANDIDATE_EVENTS_RESERVATION.with(Cell::get) {
            return Err(OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                "injected candidate history events reservation failure",
            ));
        }
        let mut replayed_events = Vec::new();
        replayed_events
            .try_reserve_exact(self.replay_events.len())
            .map_err(|error| {
                OperationError::operation_resource_exhausted(
                    request_id,
                    "historyReplay",
                    format!("cannot reserve candidate history events: {error}"),
                )
            })?;
        let mut candidate = Self::from_stacks(
            doc,
            fragment,
            self.limits.clone(),
            self.max_encoded_state_bytes,
            self.clock.clone(),
            Vec::new(),
            Vec::new(),
            self.epoch_baseline.clone(),
            false,
        );
        for event in &self.replay_events {
            match event {
                ReplayEvent::Recorded {
                    update,
                    origin,
                    policy,
                    class,
                    undo_units_bound,
                    capture_millis,
                    metadata,
                } => {
                    let replay_metadata = metadata.shared_wrapper();
                    let replay_slots = replay_metadata.slots();
                    assert!(
                        replay_slots
                            .before
                            .as_ref()
                            .and_then(HistorySnapshotSlot::get)
                            .is_some(),
                        "recorded replay metadata has before state"
                    );
                    assert!(
                        replay_slots
                            .after
                            .as_ref()
                            .and_then(HistorySnapshotSlot::get)
                            .is_some(),
                        "recorded replay metadata has after state"
                    );
                    let replay_origin = candidate.begin_capture(
                        *origin,
                        *policy,
                        *class,
                        Some(*capture_millis),
                        replay_metadata,
                    );
                    apply_update_bytes_with_origin(request_id, doc, update, replay_origin)?;
                    candidate.finish_replay_capture();
                    replayed_events.push(ReplayEvent::Recorded {
                        update: update.clone(),
                        origin: *origin,
                        policy: *policy,
                        class: *class,
                        undo_units_bound: *undo_units_bound,
                        capture_millis: *capture_millis,
                        metadata: candidate
                            .manager
                            .undo_stack()
                            .last()
                            .expect("replayed capture creates an undo item")
                            .meta()
                            .clone(),
                    });
                }
                ReplayEvent::Excluded { update, origin, .. } => {
                    apply_update_bytes(request_id, doc, update, *origin)?;
                    replayed_events.push(event.clone());
                }
                ReplayEvent::Action(action) => {
                    if candidate.perform(*action).is_none() {
                        return Err(OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "history replay cannot reproduce an accepted pop",
                        ));
                    }
                    replayed_events.push(event.clone());
                }
                ReplayEvent::Boundary => {
                    candidate.force_next_capture_boundary();
                    replayed_events.push(event.clone());
                }
            }
        }
        candidate.epoch_baseline = self.epoch_baseline.clone();
        candidate.replay_events = replayed_events;
        candidate.replay_bytes = self.replay_bytes;
        candidate.replay_work_units = self.replay_work_units;
        candidate.replay_metadata_bytes = self.replay_metadata_bytes;
        candidate.recording_replay_events = true;
        Ok(candidate)
    }

    pub(crate) fn seed_candidate(&self, request_id: u64, doc: &Doc) -> OperationResult<()> {
        apply_update_bytes(
            request_id,
            doc,
            &self.epoch_baseline,
            TransactionOrigin::RemoteSync,
        )
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.manager.can_undo()
    }

    pub(crate) fn can_redo(&self) -> bool {
        self.manager.can_redo()
    }

    #[cfg(test)]
    pub(crate) fn replay_metadata_bytes_for_test(&self) -> usize {
        self.replay_metadata_bytes
    }

    pub(crate) fn recorded_origin(origin: TransactionOrigin) -> Origin {
        Origin::from(match origin {
            TransactionOrigin::LocalInput => INPUT_ORIGIN,
            TransactionOrigin::LocalCommand => COMMAND_ORIGIN,
            TransactionOrigin::LocalApi => API_ORIGIN,
            _ => origin.as_tag(),
        })
    }
}
