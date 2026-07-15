use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use yrs::sync::time::Clock;
use yrs::types::xml::XmlFragmentRef;
use yrs::undo::{EventKind, Options as UndoOptions, StackItem, UndoManager};
use yrs::updates::decoder::Decode;
use yrs::{Doc, IdSet, Origin, ReadTxn, StateVector, Transact, Update};

use crate::model::Mark;

use super::compiler::HistoryClass;
use super::{
    EditingLimits, HistoryPolicy, OperationError, OperationResult, RelativeSelection,
    ResolvedSelection, TransactionOrigin,
};

const CAPTURE_TIMEOUT_MILLIS: u64 = 500;
const INPUT_ORIGIN: &str = "native-editor/history/local-input";
const COMMAND_ORIGIN: &str = "native-editor/history/local-command";
const API_ORIGIN: &str = "native-editor/history/local-api";
const ADDED_OBSERVER: &str = "native-editor/history/observer-added";
const UPDATED_OBSERVER: &str = "native-editor/history/observer-updated";
const POPPED_OBSERVER: &str = "native-editor/history/observer-popped";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistorySnapshot {
    pub relative_selection: RelativeSelection,
    pub resolved_selection: ResolvedSelection,
    pub stored_marks: Option<Vec<Mark>>,
    pub text_length: u64,
    pub canonical_fingerprint: [u8; 32],
    pub derived_output_bytes: usize,
    pub metadata_bytes: usize,
}

pub(crate) type HistoryLocalState = HistorySnapshot;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HistoryMetadataValue {
    before: Option<HistorySnapshot>,
    after: Option<HistorySnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HistoryMetadataSlots {
    before: Option<HistorySnapshotSlot>,
    after: Option<HistorySnapshotSlot>,
}

#[derive(Debug, Clone, Default)]
struct HistorySnapshotSlot(Arc<Mutex<Option<HistorySnapshot>>>);

impl PartialEq for HistorySnapshotSlot {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}

impl Eq for HistorySnapshotSlot {}

impl HistorySnapshotSlot {
    fn initialized(value: HistorySnapshot) -> Self {
        Self(Arc::new(Mutex::new(Some(value))))
    }

    fn empty() -> Self {
        Self::default()
    }

    fn value(&self) -> Option<HistorySnapshot> {
        self.0
            .lock()
            .expect("history snapshot slot lock poisoned")
            .clone()
    }

    fn set(&self, value: HistorySnapshot) {
        *self.0.lock().expect("history snapshot slot lock poisoned") = Some(value);
    }
}

#[derive(Debug, Clone, Default)]
struct HistoryMetadata(Arc<Mutex<HistoryMetadataSlots>>);

impl HistoryMetadata {
    fn capture(before: HistorySnapshot) -> Self {
        let before = HistorySnapshotSlot::initialized(before);
        Self(Arc::new(Mutex::new(HistoryMetadataSlots {
            after: Some(HistorySnapshotSlot::empty()),
            before: Some(before),
        })))
    }

    fn value(&self) -> HistoryMetadataValue {
        let slots = self.slots();
        HistoryMetadataValue {
            before: slots.before.and_then(|slot| slot.value()),
            after: slots.after.and_then(|slot| slot.value()),
        }
    }

    fn resolved_value(&self) -> (Option<HistorySnapshot>, Option<HistorySnapshot>) {
        let slots = self.slots();
        (
            slots.before.and_then(|slot| slot.value()),
            slots.after.and_then(|slot| slot.value()),
        )
    }

    fn slots(&self) -> HistoryMetadataSlots {
        self.0
            .lock()
            .expect("history metadata lock poisoned")
            .clone()
    }

    fn replace(&self, value: HistoryMetadataValue) {
        *self.0.lock().expect("history metadata lock poisoned") = HistoryMetadataSlots {
            before: value.before.map(HistorySnapshotSlot::initialized),
            after: value.after.map(HistorySnapshotSlot::initialized),
        };
    }

    fn preserve_before_from(&self, existing: &Self) {
        self.0
            .lock()
            .expect("history metadata lock poisoned")
            .before = existing.slots().before;
    }

    fn set_after(&self, after: HistorySnapshot) {
        let after_slot = self
            .slots()
            .after
            .expect("captured history metadata has an after slot");
        after_slot.set(after);
    }

    fn deep_clone(&self) -> Self {
        let (before, after) = self.resolved_value();
        let clone = Self(Arc::new(Mutex::new(HistoryMetadataSlots {
            before: before.map(HistorySnapshotSlot::initialized),
            after: Some(HistorySnapshotSlot::empty()),
        })));
        if let Some(after) = after {
            clone.set_after(after);
        }
        clone
    }

    fn identity(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

struct LatchingClock {
    source: Arc<dyn Clock>,
    latched: Mutex<Option<u64>>,
}

impl LatchingClock {
    fn new(source: Arc<dyn Clock>) -> Self {
        Self {
            source,
            latched: Mutex::new(None),
        }
    }

    fn latch(&self) -> u64 {
        let now = self.source.now();
        self.latch_at(now);
        now
    }

    fn latch_at(&self, now: u64) {
        *self.latched.lock().expect("history clock lock poisoned") = Some(now);
    }

    fn release(&self) {
        *self.latched.lock().expect("history clock lock poisoned") = None;
    }
}

impl Clock for LatchingClock {
    fn now(&self) -> u64 {
        self.latched
            .lock()
            .expect("history clock lock poisoned")
            .unwrap_or_else(|| self.source.now())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistoryAction {
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryPop {
    pub changed: bool,
    pub restored: Option<HistoryLocalState>,
}

#[derive(Debug, Clone)]
enum ReplayEvent {
    // Update v1 deliberately omits `Item.redone`. Replaying every accepted
    // epoch event is therefore required to reconstruct later undo/redo pops;
    // cloning only the current encoded state cannot do so.
    Recorded {
        update: Vec<u8>,
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        capture_millis: u64,
        metadata: HistoryMetadata,
    },
    Excluded {
        update: Vec<u8>,
        origin: TransactionOrigin,
        work_units: u64,
    },
    Action(HistoryAction),
}

impl ReplayEvent {
    fn encoded_bytes(&self) -> usize {
        const TAG_BYTES: usize = 1;
        match self {
            Self::Recorded { update, .. } => TAG_BYTES.saturating_add(update.capacity()),
            Self::Excluded { update, .. } => TAG_BYTES.saturating_add(update.len()),
            Self::Action(_) => TAG_BYTES,
        }
    }

    fn work_units(&self) -> u64 {
        match self {
            Self::Recorded {
                undo_units_bound, ..
            } => *undo_units_bound,
            Self::Excluded { work_units, .. } => *work_units,
            Self::Action(_) => 0,
        }
    }
}

#[derive(Debug, Clone)]
enum PendingReplayEvent {
    Recorded {
        origin: TransactionOrigin,
        policy: HistoryPolicy,
        class: HistoryClass,
        undo_units_bound: u64,
        capture_millis: u64,
        metadata: HistoryMetadata,
        update: Vec<u8>,
        metadata_increment: usize,
    },
    Excluded {
        origin: TransactionOrigin,
        work_units: u64,
        update: Vec<u8>,
    },
}

pub(crate) struct YrsHistory {
    manager: UndoManager<HistoryMetadata>,
    limits: EditingLimits,
    clock: Arc<LatchingClock>,
    pending_capture: Arc<Mutex<Option<HistoryMetadata>>>,
    pending_pop: Arc<Mutex<Option<HistoryMetadata>>>,
    popped: Arc<Mutex<Option<(EventKind, HistoryMetadataValue)>>>,
    last_capture_millis: Option<u64>,
    last_class: Option<HistoryClass>,
    last_origin: Option<TransactionOrigin>,
    force_next_boundary: bool,
    epoch_baseline: Vec<u8>,
    replay_events: Vec<ReplayEvent>,
    replay_bytes: usize,
    replay_work_units: u64,
    replay_metadata_bytes: usize,
    max_encoded_state_bytes: usize,
    pending_replay_event: Option<PendingReplayEvent>,
    rebase_before_next_event: bool,
    recording_replay_events: bool,
}

impl YrsHistory {
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
        let popped = Arc::new(Mutex::new(None::<(EventKind, HistoryMetadataValue)>));
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
            let value = event.meta().value();
            if let Some(target) = popped_target
                .lock()
                .expect("pending history pop lock poisoned")
                .clone()
            {
                target.replace(value.clone());
            }
            *popped_result
                .lock()
                .expect("popped history metadata lock poisoned") = Some((event.kind(), value));
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
                    let replay_metadata = metadata.deep_clone();
                    let (before, after) = replay_metadata.resolved_value();
                    assert!(
                        before.is_some(),
                        "recorded replay metadata has before state"
                    );
                    let after = after.expect("recorded replay metadata has after state");
                    let replay_origin = candidate.begin_capture(
                        *origin,
                        *policy,
                        *class,
                        Some(*capture_millis),
                        replay_metadata,
                    );
                    apply_update_bytes_with_origin(request_id, doc, update, replay_origin)?;
                    candidate.finish_capture(after, Vec::new());
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

    pub(crate) fn recorded_origin(origin: TransactionOrigin) -> Origin {
        Origin::from(match origin {
            TransactionOrigin::LocalInput => INPUT_ORIGIN,
            TransactionOrigin::LocalCommand => COMMAND_ORIGIN,
            TransactionOrigin::LocalApi => API_ORIGIN,
            _ => origin.as_tag(),
        })
    }

    #[allow(clippy::too_many_arguments)]
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
        debug_assert_ne!(policy, HistoryPolicy::Skip);
        debug_assert_ne!(class, HistoryClass::Skip);
        let before = before.ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "recorded history transaction has no local state metadata",
            )
        })?;
        let standalone_metadata_bytes = before
            .metadata_bytes
            .checked_add(after_metadata_bytes)
            .ok_or_else(|| metadata_limit_error(request_id, &self.limits, usize::MAX))?;
        if standalone_metadata_bytes > self.limits.max_derived_output_bytes {
            return Err(metadata_limit_error(
                request_id,
                &self.limits,
                standalone_metadata_bytes,
            ));
        }
        if undo_units_bound > self.limits.max_undo_retained_units {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxUndoRetainedUnits",
                self.limits.max_undo_retained_units,
                undo_units_bound,
            ));
        }

        let now = self.clock.latch();
        let pending_metadata_bytes = self
            .replay_metadata_bytes
            .checked_add(self.unmirrored_stack_metadata_bytes(request_id)?)
            .and_then(|bytes| bytes.checked_add(standalone_metadata_bytes))
            .unwrap_or(usize::MAX);
        let should_roll = pending_metadata_bytes > self.limits.max_derived_output_bytes
            || self.capture_would_roll(
                request_id,
                origin,
                policy,
                class,
                undo_units_bound,
                &before,
                after_metadata_bytes,
                now,
            )?;
        let reserved_update = self.reserve_replay_event(
            request_id,
            current_encoded_state,
            update_bytes_bound,
            undo_units_bound,
            false,
        )?;
        if should_roll {
            self.roll_epoch(current_encoded_state.to_vec());
            self.clock.latch_at(now);
        }
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
        before: &HistoryLocalState,
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
        let standalone_metadata_bytes = before.metadata_bytes.saturating_add(after_metadata_bytes);
        let next_metadata_bytes = if compatible {
            let replaced_after = self
                .manager
                .undo_stack()
                .last()
                .and_then(|item| item.meta().value().after)
                .map(|snapshot| snapshot.metadata_bytes)
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
        let mirrored = self
            .replay_events
            .iter()
            .filter_map(|event| match event {
                ReplayEvent::Recorded { metadata, .. } => Some(metadata.identity()),
                ReplayEvent::Excluded { .. } | ReplayEvent::Action(_) => None,
            })
            .collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        let mut total = 0usize;
        for item in self
            .manager
            .undo_stack()
            .iter()
            .chain(self.manager.redo_stack())
        {
            let metadata = item.meta();
            if mirrored.contains(&metadata.identity()) || !seen.insert(metadata.identity()) {
                continue;
            }
            let value = metadata.value();
            for snapshot in [value.before, value.after].into_iter().flatten() {
                total = total
                    .checked_add(snapshot.metadata_bytes)
                    .ok_or_else(|| metadata_limit_error(request_id, &self.limits, usize::MAX))?;
            }
        }
        Ok(total)
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
                self.push_replay_event(event);
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

    pub(crate) fn prepare_excluded(
        &mut self,
        request_id: u64,
        origin: TransactionOrigin,
        work_units: u64,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
    ) -> OperationResult<Origin> {
        let update = self.reserve_replay_event(
            request_id,
            current_encoded_state,
            update_bytes_bound,
            work_units,
            true,
        )?;
        self.pending_replay_event = Some(PendingReplayEvent::Excluded {
            origin,
            work_units,
            update,
        });
        Ok(origin.as_yrs_origin())
    }

    pub(crate) fn finish_excluded(&mut self, update: Vec<u8>) {
        let Some(PendingReplayEvent::Excluded {
            origin,
            work_units,
            update: mut reserved_update,
        }) = self.pending_replay_event.take()
        else {
            self.invalidate_replay_after_mutation();
            return;
        };
        if self.rebase_before_next_event {
            self.manager.clear_all();
            self.manager.reset();
            self.replay_events.clear();
            self.replay_bytes = 0;
            self.replay_work_units = 0;
            self.replay_metadata_bytes = 0;
            return;
        }
        assert!(
            update.len() <= reserved_update.capacity(),
            "admitted excluded update exceeds its exact pre-write reservation"
        );
        reserved_update.extend_from_slice(&update);
        let event = ReplayEvent::Excluded {
            update: reserved_update,
            origin,
            work_units,
        };
        self.push_replay_event(event);
    }

    pub(crate) fn perform(&mut self, action: HistoryAction) -> Option<HistorySnapshot> {
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

    fn reserve_replay_event(
        &mut self,
        request_id: u64,
        current_encoded_state: &[u8],
        update_bytes_bound: usize,
        work_units: u64,
        excluded: bool,
    ) -> OperationResult<Vec<u8>> {
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
        self.replay_events.try_reserve(1).map_err(|error| {
            OperationError::operation_resource_exhausted(
                request_id,
                "historyReplay",
                format!("cannot reserve bounded history event slot: {error}"),
            )
        })?;
        let event_bytes_bound = update.capacity().checked_add(1).ok_or_else(|| {
            encoded_limit_error(request_id, self.max_encoded_state_bytes, usize::MAX)
        })?;
        if event_bytes_bound > self.max_encoded_state_bytes {
            return Err(encoded_limit_error(
                request_id,
                self.max_encoded_state_bytes,
                event_bytes_bound,
            ));
        }
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
        if self.rebase_before_next_event {
            self.roll_epoch(current_encoded_state.to_vec());
        }
        let next_bytes = self.replay_bytes.saturating_add(event_bytes_bound);
        let next_work = self.replay_work_units.saturating_add(work_units);
        let next_count = self.replay_events.len().saturating_add(1);
        if next_bytes > self.max_encoded_state_bytes
            || next_work > self.limits.max_undo_retained_units
            || next_count > self.event_ceiling()
        {
            self.roll_epoch(current_encoded_state.to_vec());
        }
        Ok(update)
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

fn stack_units(stack: &[StackItem<HistoryMetadata>], request_id: u64) -> OperationResult<u64> {
    let mut total = 0u64;
    for item in stack {
        total = add_id_set_units(total, item.insertions(), request_id)?;
        total = add_id_set_units(total, item.deletions(), request_id)?;
    }
    Ok(total)
}

fn stack_metadata_bytes(
    stack: &[StackItem<HistoryMetadata>],
    request_id: u64,
    limits: &EditingLimits,
) -> OperationResult<usize> {
    let mut total = 0usize;
    for item in stack {
        let value = item.meta().value();
        for snapshot in [value.before, value.after].into_iter().flatten() {
            total = total
                .checked_add(snapshot.metadata_bytes)
                .ok_or_else(|| metadata_limit_error(request_id, limits, usize::MAX))?;
        }
    }
    Ok(total)
}

fn metadata_limit_error(request_id: u64, limits: &EditingLimits, actual: usize) -> OperationError {
    OperationError::document_limit_exceeded(
        request_id,
        None,
        "maxDerivedOutputBytes",
        u64::try_from(limits.max_derived_output_bytes).unwrap_or(u64::MAX),
        u64::try_from(actual).unwrap_or(u64::MAX),
    )
}

fn encoded_limit_error(request_id: u64, limit: usize, actual: usize) -> OperationError {
    OperationError::document_limit_exceeded(
        request_id,
        None,
        "maxEncodedStateBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::try_from(actual).unwrap_or(u64::MAX),
    )
}

fn encode_full_state(doc: &Doc) -> Vec<u8> {
    let txn = doc.transact();
    if txn.state_vector().is_empty() {
        Vec::new()
    } else {
        txn.encode_state_as_update_v1(&StateVector::default())
    }
}

fn apply_update_bytes(
    request_id: u64,
    doc: &Doc,
    bytes: &[u8],
    origin: TransactionOrigin,
) -> OperationResult<()> {
    apply_update_bytes_with_origin(request_id, doc, bytes, origin.as_yrs_origin())
}

fn apply_update_bytes_with_origin(
    request_id: u64,
    doc: &Doc,
    bytes: &[u8],
    origin: Origin,
) -> OperationResult<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let update = Update::decode_v1(bytes).map_err(|error| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            format!("cannot decode bounded history replay event: {error}"),
        )
    })?;
    doc.transact_mut_with(origin)
        .apply_update(update)
        .map_err(|error| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                format!("cannot apply bounded history replay event: {error}"),
            )
        })
}

fn add_id_set_units(mut total: u64, set: &IdSet, request_id: u64) -> OperationResult<u64> {
    for (_, ranges) in set.iter() {
        for range in ranges {
            let units = range.end.checked_sub(range.start).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "history IdSet range end precedes its start",
                )
            })?;
            total = total.checked_add(u64::from(units)).ok_or_else(|| {
                OperationError::operation_limit_exceeded(
                    request_id,
                    None,
                    "maxUndoRetainedUnits",
                    u64::MAX,
                    u64::MAX,
                )
            })?;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yrs::block::ClientID;
    use yrs::types::xml::XmlFragment;
    use yrs::{Doc, GetString, IdSet, Transact, XmlTextPrelim};

    use super::{add_id_set_units, EditingLimits, TransactionOrigin, YrsHistory, INPUT_ORIGIN};

    #[test]
    fn id_set_accounting_counts_clock_ranges_not_clients() {
        let set = IdSet::from_iter([
            (ClientID::new(1), [2..5, 9..11]),
            (ClientID::new(2), [0..4, 4..4]),
        ]);
        assert_eq!(add_id_set_units(7, &set, 1).unwrap(), 16);
    }

    #[test]
    fn private_local_origin_is_captured_but_remote_origin_is_preserved_by_undo() {
        let doc = Doc::new();
        let fragment = doc.get_or_insert_xml_fragment("history-test");
        let mut history = YrsHistory::new(
            &doc,
            &fragment,
            EditingLimits::default(),
            usize::MAX,
            Arc::new(|| 10_000),
        );

        {
            let mut txn = doc.transact_mut_with(TransactionOrigin::RemoteSync.as_yrs_origin());
            fragment.push_back(&mut txn, XmlTextPrelim::new("remote"));
        }
        assert_eq!(history.manager.undo_stack().len(), 0);

        history.manager.reset();
        {
            let mut txn = doc.transact_mut_with(INPUT_ORIGIN);
            fragment.push_back(&mut txn, XmlTextPrelim::new("local"));
        }
        assert_eq!(history.manager.undo_stack().len(), 1);
        assert!(history.manager.undo_blocking());
        assert_eq!(fragment.get_string(&doc.transact()), "remote");
    }

    #[test]
    fn replay_reservation_is_fallible_and_does_not_clear_existing_history() {
        let doc = Doc::new();
        let fragment = doc.get_or_insert_xml_fragment("history-test");
        let mut history = YrsHistory::new(
            &doc,
            &fragment,
            EditingLimits::default(),
            usize::MAX,
            Arc::new(|| 10_000),
        );
        {
            let mut txn = doc.transact_mut_with(INPUT_ORIGIN);
            fragment.push_back(&mut txn, XmlTextPrelim::new("local"));
        }
        let undo_groups = history.manager.undo_stack().len();

        let error = history
            .reserve_replay_event(41, &[], usize::MAX, 1, false)
            .unwrap_err();
        assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
        assert_eq!(history.manager.undo_stack().len(), undo_groups);
        assert!(history.manager.can_undo());
    }
}
