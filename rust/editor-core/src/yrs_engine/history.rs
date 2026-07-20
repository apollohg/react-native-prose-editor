#[cfg(test)]
use std::cell::Cell;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

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

#[cfg(test)]
thread_local! {
    static FAIL_BOUNDARY_RESERVATION: Cell<bool> = const { Cell::new(false) };
    static FAIL_REPLAY_UPDATE_ALLOCATION: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_boundary_reservation_failure_for_test(enabled: bool) {
    FAIL_BOUNDARY_RESERVATION.with(|fail| fail.set(enabled));
}

#[cfg(test)]
pub(crate) fn set_replay_update_allocation_failure_for_test(enabled: bool) {
    FAIL_REPLAY_UPDATE_ALLOCATION.with(|fail| fail.set(enabled));
}

#[derive(Debug, Clone)]
pub(crate) struct HistorySnapshot {
    pub relative_selection: RelativeSelection,
    pub resolved_selection: ResolvedSelection,
    pub stored_marks: Option<Vec<Mark>>,
    pub text_length: u64,
    pub canonical_fingerprint: [u8; 32],
    pub derived_output_bytes: usize,
    pub metadata_bytes: usize,
    pub document_snapshot: Option<Arc<super::derived_state::HistoryDocumentSnapshot>>,
}

impl PartialEq for HistorySnapshot {
    fn eq(&self, other: &Self) -> bool {
        // The document snapshot is a sealed cache identity, not semantic
        // document data. Distinct but structurally equivalent allocations must
        // compare unequal so metadata slot comparisons cannot conflate seals.
        self.relative_selection == other.relative_selection
            && self.resolved_selection == other.resolved_selection
            && self.stored_marks == other.stored_marks
            && self.text_length == other.text_length
            && self.canonical_fingerprint == other.canonical_fingerprint
            && self.derived_output_bytes == other.derived_output_bytes
            && self.metadata_bytes == other.metadata_bytes
            && match (&self.document_snapshot, &other.document_snapshot) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for HistorySnapshot {}

pub(crate) type HistoryLocalState = HistorySnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparedHistoryLimits {
    pub(crate) now_millis: u64,
    pub(crate) standalone_metadata_bytes: usize,
    pub(crate) prospective_metadata_increment: usize,
    pub(crate) compatible: bool,
    pub(crate) should_roll: bool,
}

#[derive(Debug)]
pub(crate) struct HistorySnapshotTemplate {
    pub stored_marks: Option<Vec<Mark>>,
    pub text_length: u64,
    pub canonical_fingerprint: [u8; 32],
    pub derived_output_bytes: usize,
    pub metadata_bytes: usize,
    pub document_snapshot_retained_bytes:
        Option<super::derived_state::HistoryDocumentSnapshotRetainedBytes>,
}

impl HistorySnapshotTemplate {
    pub(crate) fn seal(
        self,
        relative_selection: RelativeSelection,
        resolved_selection: ResolvedSelection,
        document_snapshot: Option<Arc<super::derived_state::HistoryDocumentSnapshot>>,
    ) -> HistorySnapshot {
        debug_assert_eq!(
            self.document_snapshot_retained_bytes
                .map(super::derived_state::HistoryDocumentSnapshotRetainedBytes::get),
            document_snapshot
                .as_deref()
                .map(super::derived_state::HistoryDocumentSnapshot::retained_bytes)
        );
        HistorySnapshot {
            relative_selection,
            resolved_selection,
            stored_marks: self.stored_marks,
            text_length: self.text_length,
            canonical_fingerprint: self.canonical_fingerprint,
            derived_output_bytes: self.derived_output_bytes,
            metadata_bytes: self.metadata_bytes,
            document_snapshot,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct HistoryMetadataSlots {
    before: Option<HistorySnapshotSlot>,
    after: Option<HistorySnapshotSlot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HistorySnapshotSlot(Arc<OnceLock<HistorySnapshot>>);

impl PartialEq for HistorySnapshotSlot {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}

impl Eq for HistorySnapshotSlot {}

impl HistorySnapshotSlot {
    fn initialized(value: HistorySnapshot) -> Self {
        let slot = Self::empty();
        slot.set(value);
        slot
    }

    fn empty() -> Self {
        Self::default()
    }

    pub(crate) fn get(&self) -> Option<&HistorySnapshot> {
        self.0.get()
    }

    fn set(&self, value: HistorySnapshot) {
        self.0
            .set(value)
            .unwrap_or_else(|_| panic!("history snapshot slot can only be sealed once"));
    }

    fn identity(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
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

    fn slots(&self) -> HistoryMetadataSlots {
        self.0
            .lock()
            .expect("history metadata lock poisoned")
            .clone()
    }

    fn replace_slots(&self, slots: HistoryMetadataSlots) {
        *self.0.lock().expect("history metadata lock poisoned") = slots;
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

    fn shared_wrapper(&self) -> Self {
        Self(Arc::new(Mutex::new(self.slots())))
    }

    #[cfg(test)]
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
    pub restored: Option<HistorySnapshotSlot>,
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
    Boundary,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplayLedgerAllocationAudit {
    len: usize,
    capacity: usize,
    allocation: usize,
    events: Vec<ReplayEventAllocationIdentity>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayEventAllocationIdentity {
    kind: u8,
    update_allocation: usize,
    update_len: usize,
    update_capacity: usize,
    metadata_identity: usize,
}

pub(crate) struct PreparedBoundary {
    accepted_encoded_state: Vec<u8>,
    roll_epoch: bool,
}

impl ReplayEvent {
    fn encoded_bytes(&self) -> usize {
        const TAG_BYTES: usize = 1;
        match self {
            Self::Recorded { update, .. } => TAG_BYTES.saturating_add(update.capacity()),
            Self::Excluded { update, .. } => TAG_BYTES.saturating_add(update.capacity()),
            Self::Action(_) => TAG_BYTES,
            Self::Boundary => TAG_BYTES,
        }
    }

    fn work_units(&self) -> u64 {
        match self {
            Self::Recorded {
                undo_units_bound, ..
            } => *undo_units_bound,
            Self::Excluded { work_units, .. } => *work_units,
            Self::Action(_) => 0,
            Self::Boundary => 0,
        }
    }
}

#[derive(Debug)]
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
        replay_slot: PreparedReplayEventSlot,
    },
}

#[derive(Debug)]
enum PreparedReplayEventSlot {
    ExistingSpare,
    Replacement(Vec<ReplayEvent>),
}

pub(crate) enum ExcludedReplayDisposition {
    Append,
    Roll { owned_baseline: Vec<u8> },
    InvalidateAfterCommit,
}

pub(crate) struct PreparedExcludedHistoryAdmission {
    origin: TransactionOrigin,
    work_units: u64,
    reserved_update: Vec<u8>,
    disposition: ExcludedReplayDisposition,
    replay_slot: Option<PreparedReplayEventSlot>,
}

pub(crate) struct PreparedRecordedHistoryAdmission {
    origin: TransactionOrigin,
    policy: HistoryPolicy,
    class: HistoryClass,
    undo_units_bound: u64,
    capture_millis: u64,
    metadata: HistoryMetadata,
    reserved_update: Vec<u8>,
    metadata_increment: usize,
    owned_baseline: Option<Vec<u8>>,
    compatible: bool,
    replay_slot: PreparedReplayEventSlot,
}

impl PreparedRecordedHistoryAdmission {
    pub(crate) fn yrs_origin(&self) -> Origin {
        YrsHistory::recorded_origin(self.origin)
    }
}

impl PreparedExcludedHistoryAdmission {
    pub(crate) fn yrs_origin(&self) -> Origin {
        self.origin.as_yrs_origin()
    }
}

pub(crate) struct YrsHistory {
    manager: UndoManager<HistoryMetadata>,
    limits: EditingLimits,
    clock: Arc<LatchingClock>,
    pending_capture: Arc<Mutex<Option<HistoryMetadata>>>,
    pending_pop: Arc<Mutex<Option<HistoryMetadata>>>,
    popped: Arc<Mutex<Option<(EventKind, HistoryMetadataSlots)>>>,
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
            .then(|| {
                let mut baseline = Vec::new();
                baseline
                    .try_reserve_exact(current_encoded_state.len())
                    .map_err(|error| {
                        OperationError::operation_resource_exhausted(
                            request_id,
                            "historyReplay",
                            format!("cannot reserve replay roll baseline: {error}"),
                        )
                    })?;
                baseline.extend_from_slice(current_encoded_state);
                Ok(baseline)
            })
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
            .then(|| {
                let mut baseline = Vec::new();
                baseline
                    .try_reserve_exact(current_encoded_state.len())
                    .map_err(|error| {
                        OperationError::operation_resource_exhausted(
                            request_id,
                            "historyReplay",
                            format!("cannot reserve replay roll baseline: {error}"),
                        )
                    })?;
                baseline.extend_from_slice(current_encoded_state);
                Ok(baseline)
            })
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
                    let mut owned_baseline = Vec::new();
                    owned_baseline
                        .try_reserve_exact(current_encoded_state.len())
                        .map_err(|error| {
                            OperationError::operation_resource_exhausted(
                                request_id,
                                "historyReplay",
                                format!("cannot reserve replay roll baseline: {error}"),
                            )
                        })?;
                    owned_baseline.extend_from_slice(current_encoded_state);
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
                let mut owned_baseline = Vec::new();
                owned_baseline
                    .try_reserve_exact(current_encoded_state.len())
                    .map_err(|error| {
                        OperationError::operation_resource_exhausted(
                            request_id,
                            "historyReplay",
                            format!("cannot reserve replay roll baseline: {error}"),
                        )
                    })?;
                owned_baseline.extend_from_slice(current_encoded_state);
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
    let mut seen = HashSet::new();
    for item in stack {
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
                total = total
                    .checked_add(snapshot.metadata_bytes)
                    .ok_or_else(|| metadata_limit_error(request_id, limits, usize::MAX))?;
            }
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

    use super::{
        add_id_set_units, EditingLimits, HistoryClass, HistoryMetadata, HistoryMetadataSlots,
        HistoryPolicy, HistorySnapshot, HistorySnapshotSlot, PendingReplayEvent, RelativeSelection,
        ReplayEvent, ResolvedSelection, TransactionOrigin, YrsHistory, INPUT_ORIGIN,
    };

    fn history_snapshot(metadata_bytes: usize) -> HistorySnapshot {
        HistorySnapshot {
            relative_selection: RelativeSelection::All,
            resolved_selection: ResolvedSelection::All,
            stored_marks: None,
            text_length: 0,
            canonical_fingerprint: [0; 32],
            derived_output_bytes: 0,
            metadata_bytes,
            document_snapshot: None,
        }
    }

    fn compatible_history_requiring_reservation_roll(metadata_limit: usize) -> (Doc, YrsHistory) {
        let doc = Doc::new();
        let fragment = doc.get_or_insert_xml_fragment("history-test");
        let limits = EditingLimits {
            max_derived_output_bytes: metadata_limit,
            ..EditingLimits::default()
        };
        let mut history = YrsHistory::new(&doc, &fragment, limits, usize::MAX, Arc::new(|| 10_000));
        let origin = history
            .prepare_capture(
                50,
                TransactionOrigin::LocalInput,
                HistoryPolicy::Auto,
                HistoryClass::Insert,
                1,
                Some(history_snapshot(1)),
                1,
                &[],
                0,
            )
            .unwrap();
        {
            let mut txn = doc.transact_mut_with(origin);
            fragment.push_back(&mut txn, XmlTextPrelim::new("a"));
        }
        history.finish_capture(history_snapshot(1), Vec::new());
        assert!(history.capture_is_compatible(
            TransactionOrigin::LocalInput,
            HistoryPolicy::Auto,
            HistoryClass::Insert,
            10_000,
        ));
        history.rebase_before_next_event = true;
        (doc, history)
    }

    #[test]
    fn excluded_event_accounts_reserved_capacity_not_only_encoded_length() {
        let mut update = Vec::with_capacity(64);
        update.extend_from_slice(&[1, 2, 3]);
        let event = ReplayEvent::Excluded {
            update,
            origin: TransactionOrigin::LocalApi,
            work_units: 3,
        };
        assert_eq!(event.encoded_bytes(), 65);
    }

    #[test]
    fn candidate_metadata_wrapper_shares_immutable_snapshot_slots() {
        let before = HistorySnapshotSlot::empty();
        let after = HistorySnapshotSlot::empty();
        let metadata = HistoryMetadata(Arc::new(std::sync::Mutex::new(HistoryMetadataSlots {
            before: Some(before),
            after: Some(after),
        })));

        let candidate = metadata.shared_wrapper();
        assert_ne!(metadata.identity(), candidate.identity());
        let live_slots = metadata.slots();
        let candidate_slots = candidate.slots();
        assert_eq!(
            live_slots.before.unwrap().identity(),
            candidate_slots.before.unwrap().identity()
        );
        assert_eq!(
            live_slots.after.unwrap().identity(),
            candidate_slots.after.unwrap().identity()
        );
    }

    #[test]
    fn cumulative_excluded_events_charge_reserved_payload_capacity() {
        let doc = Doc::new();
        let fragment = doc.get_or_insert_xml_fragment("history-test");
        let mut history = YrsHistory::new(
            &doc,
            &fragment,
            EditingLimits::default(),
            usize::MAX,
            Arc::new(|| 10_000),
        );

        let mut first = history.reserve_replay_event(1, &[], 9, 1, true).unwrap();
        first.push(1);
        let event_bytes = first.capacity() + 1;
        history.max_encoded_state_bytes = event_bytes * 2;
        history.push_replay_event(ReplayEvent::Excluded {
            update: first,
            origin: TransactionOrigin::LocalApi,
            work_units: 1,
        });

        let mut second = history.reserve_replay_event(2, &[], 9, 1, true).unwrap();
        second.push(2);
        history.push_replay_event(ReplayEvent::Excluded {
            update: second,
            origin: TransactionOrigin::LocalApi,
            work_units: 1,
        });
        assert_eq!(history.replay_bytes, event_bytes * 2);
        assert_eq!(history.replay_events.len(), 2);

        let third = history.reserve_replay_event(3, &[], 9, 1, true).unwrap();
        assert!(third.capacity() < history.max_encoded_state_bytes);
        assert!(history.replay_events.is_empty());
        assert_eq!(history.replay_bytes, 0);
    }

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

    #[test]
    fn reserve_induced_compatible_roll_rejects_one_over_standalone_metadata_atomically() {
        let (_doc, mut history) = compatible_history_requiring_reservation_roll(100);
        // This is the state produced when a prior excluded event requires the
        // next recorded event to start a fresh replay epoch. The capture still
        // starts compatible, so only reservation discovers the rollover.
        let undo_groups = history.manager.undo_stack().len();
        let replay_events = history.replay_events.len();
        let replay_bytes = history.replay_bytes;
        let replay_work_units = history.replay_work_units;
        let replay_metadata_bytes = history.replay_metadata_bytes;
        let epoch_baseline = history.epoch_baseline.clone();

        let error = history
            .prepare_capture(
                51,
                TransactionOrigin::LocalInput,
                HistoryPolicy::Auto,
                HistoryClass::Insert,
                1,
                Some(history_snapshot(60)),
                41,
                &[],
                0,
            )
            .unwrap_err();
        assert_eq!(error.code, "DOCUMENT_LIMIT_EXCEEDED");
        assert_eq!(error.limit, Some(100));
        assert_eq!(error.actual, Some(101));
        assert_eq!(history.manager.undo_stack().len(), undo_groups);
        assert!(history.manager.can_undo());
        assert_eq!(history.replay_events.len(), replay_events);
        assert_eq!(history.replay_bytes, replay_bytes);
        assert_eq!(history.replay_work_units, replay_work_units);
        assert_eq!(history.replay_metadata_bytes, replay_metadata_bytes);
        assert_eq!(history.epoch_baseline, epoch_baseline);
        assert!(history.rebase_before_next_event);
        assert!(history.pending_replay_event.is_none());
        assert!(history
            .pending_capture
            .lock()
            .expect("pending capture lock")
            .is_none());
    }

    #[test]
    fn reserve_induced_compatible_roll_accepts_exact_standalone_metadata_boundary() {
        let (_doc, mut history) = compatible_history_requiring_reservation_roll(100);

        history
            .prepare_capture(
                52,
                TransactionOrigin::LocalInput,
                HistoryPolicy::Auto,
                HistoryClass::Insert,
                1,
                Some(history_snapshot(59)),
                41,
                &[],
                0,
            )
            .unwrap();

        assert!(!history.rebase_before_next_event);
        assert!(history.manager.undo_stack().is_empty());
        assert!(history.replay_events.is_empty());
        assert!(matches!(
            history.pending_replay_event,
            Some(PendingReplayEvent::Recorded {
                metadata_increment: 100,
                ..
            })
        ));
    }
}
