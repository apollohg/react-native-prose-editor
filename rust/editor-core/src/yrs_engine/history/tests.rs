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

// Task 16B: the retained fallible allocation/reservation sites keep the
// allocation-class OPERATION_RESOURCE_EXHAUSTED code.
#[test]
fn roll_baseline_reservation_failure_keeps_resource_exhausted() {
    let (_doc, mut history) = compatible_history_requiring_reservation_roll(100);
    super::set_roll_baseline_reservation_failure_for_test(true);
    let error = history
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
        .unwrap_err();
    super::set_roll_baseline_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "historyReplay" }))
    );
    // Recovery: the identical capture succeeds once allocation recovers.
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
}

#[test]
fn accepted_action_reservation_failure_keeps_resource_exhausted() {
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment("history-test");
    let mut history = YrsHistory::new(
        &doc,
        &fragment,
        EditingLimits::default(),
        usize::MAX,
        Arc::new(|| 10_000),
    );
    super::set_accepted_action_reservation_failure_for_test(true);
    let error = history
        .accept_action(41, super::HistoryAction::Undo, Vec::new())
        .unwrap_err();
    super::set_accepted_action_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "historyReplay" }))
    );
    history
        .accept_action(41, super::HistoryAction::Undo, Vec::new())
        .unwrap();
}

#[test]
fn candidate_events_reservation_failure_keeps_resource_exhausted() {
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment("history-test");
    let history = YrsHistory::new(
        &doc,
        &fragment,
        EditingLimits::default(),
        usize::MAX,
        Arc::new(|| 10_000),
    );
    super::set_candidate_events_reservation_failure_for_test(true);
    let Err(error) = history.replay_into(41, &doc, &fragment) else {
        panic!("injected candidate events failure must reject")
    };
    super::set_candidate_events_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "historyReplay" }))
    );
    history.replay_into(41, &doc, &fragment).unwrap();
}

#[test]
fn event_replacement_reservation_failure_keeps_resource_exhausted() {
    let doc = Doc::new();
    let fragment = doc.get_or_insert_xml_fragment("history-test");
    let history = YrsHistory::new(
        &doc,
        &fragment,
        EditingLimits::default(),
        usize::MAX,
        Arc::new(|| 10_000),
    );
    super::set_event_replacement_reservation_failure_for_test(true);
    let Err(error) = history.prepare_replay_event_slot(41, false) else {
        panic!("injected replacement failure must reject")
    };
    super::set_event_replacement_reservation_failure_for_test(false);
    assert_eq!(error.code, "OPERATION_RESOURCE_EXHAUSTED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "historyReplay" }))
    );
    history.prepare_replay_event_slot(41, false).unwrap();
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
