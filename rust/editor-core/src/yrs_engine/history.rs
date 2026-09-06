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
    static FAIL_ROLL_BASELINE_RESERVATION: Cell<bool> = const { Cell::new(false) };
    static FAIL_ACCEPTED_ACTION_RESERVATION: Cell<bool> = const { Cell::new(false) };
    static FAIL_CANDIDATE_EVENTS_RESERVATION: Cell<bool> = const { Cell::new(false) };
    static FAIL_EVENT_REPLACEMENT_RESERVATION: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn set_boundary_reservation_failure_for_test(enabled: bool) {
    FAIL_BOUNDARY_RESERVATION.with(|fail| fail.set(enabled));
}

#[cfg(test)]
pub(crate) fn set_replay_update_allocation_failure_for_test(enabled: bool) {
    FAIL_REPLAY_UPDATE_ALLOCATION.with(|fail| fail.set(enabled));
}

#[cfg(test)]
fn set_roll_baseline_reservation_failure_for_test(enabled: bool) {
    FAIL_ROLL_BASELINE_RESERVATION.with(|fail| fail.set(enabled));
}

#[cfg(test)]
fn set_accepted_action_reservation_failure_for_test(enabled: bool) {
    FAIL_ACCEPTED_ACTION_RESERVATION.with(|fail| fail.set(enabled));
}

#[cfg(test)]
fn set_candidate_events_reservation_failure_for_test(enabled: bool) {
    FAIL_CANDIDATE_EVENTS_RESERVATION.with(|fail| fail.set(enabled));
}

#[cfg(test)]
fn set_event_replacement_reservation_failure_for_test(enabled: bool) {
    FAIL_EVENT_REPLACEMENT_RESERVATION.with(|fail| fail.set(enabled));
}

/// Fallible allocation seam: reserving the replay roll baseline is a genuine
/// allocation site, so it keeps the allocation-class
/// `OPERATION_RESOURCE_EXHAUSTED` code.
fn reserve_replay_roll_baseline(
    request_id: u64,
    current_encoded_state: &[u8],
) -> OperationResult<Vec<u8>> {
    #[cfg(test)]
    if FAIL_ROLL_BASELINE_RESERVATION.with(Cell::get) {
        return Err(OperationError::operation_resource_exhausted(
            request_id,
            "historyReplay",
            "injected replay roll baseline reservation failure",
        ));
    }
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

include!("history/recording.rs");
include!("history/capture.rs");
include!("history/replay.rs");

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
#[path = "history/tests.rs"]
mod tests;
