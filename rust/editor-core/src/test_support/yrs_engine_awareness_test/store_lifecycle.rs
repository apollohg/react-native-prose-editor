#[test]
fn snapshot_restore_rebinds_awareness_to_the_new_store() {
    let mut source = engine_with_scope(InitializationMode::LocalEmpty, Some(scope()));
    source
        .import_json(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"restored"}]}]})
                .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();
    let snapshot = source.export_snapshot().unwrap();

    let mut engine = engine_with_scope(InitializationMode::LocalEmpty, Some(scope()));
    let limits = session_default_limits();
    let old_client = engine.client_id();
    let desired = json!({"name": "survivor", "color": "#123456"});
    // Two writes so the pre-restore clock is provably beyond a fresh one.
    engine
        .awareness()
        .set_local_state(&json!({"name": "first"}), &limits)
        .unwrap();
    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap();
    let mut raw = raw_awareness();
    raw.set_local_state(json!({"name": "stale-peer"})).unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw), &limits)
        .unwrap();
    assert_eq!(engine.awareness().peer_snapshot().len(), 2);
    engine
        .awareness()
        .set_live_local_clock_for_test(u32::MAX - 1);
    engine.awareness().clear_transport_states().unwrap();

    assert!(engine.restore_snapshot(&snapshot).unwrap().changed);

    let new_client = engine.client_id();
    assert_ne!(new_client, old_client);
    assert_eq!(engine.awareness().client_id(), new_client);
    // Stale remote peers are dropped; even an exhausted old-identity
    // tombstone is recovered only by the fresh client identity, where the
    // desired state is re-encoded at clock one.
    assert_eq!(engine.awareness().local_state(), Some(&desired));
    let peers = engine.awareness().peer_snapshot();
    assert_eq!(peers.len(), 1);
    assert!(peers[0].is_local);
    assert_eq!(peers[0].client_id, new_client);
    let entries = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(entries.len(), 1);
    let (clock, state) = &entries[&new_client];
    assert_eq!(*clock, 1);
    assert_eq!(serde_json::from_str::<Value>(state).unwrap(), desired);
}

#[test]
fn import_store_swap_rebinds_awareness_and_drops_stale_peers() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let old_client = engine.client_id();
    let desired = json!({"name": "import-survivor"});
    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap();
    let mut raw = raw_awareness();
    raw.set_local_state(json!({"name": "pre-import-peer"}))
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw), &limits)
        .unwrap();
    engine
        .awareness()
        .set_live_local_clock_for_test(u32::MAX - 1);
    engine.awareness().clear_transport_states().unwrap();

    engine
        .import_json(
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"imported"}]}]})
                .to_string(),
            TransactionOrigin::DocumentImport,
        )
        .unwrap();

    let new_client = engine.client_id();
    assert_ne!(new_client, old_client);
    assert_eq!(engine.awareness().client_id(), new_client);
    assert_eq!(engine.awareness().local_state(), Some(&desired));
    let peers = engine.awareness().peer_snapshot();
    assert_eq!(peers.len(), 1);
    assert!(peers[0].is_local);
    let entries = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(entries[&new_client].0, 1);
}

#[test]
fn same_store_replacement_preserves_awareness_binding_and_peers() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let client = engine.client_id();
    let desired = json!({"name": "replacement-survivor"});
    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap();
    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap(); // clock 2
    let mut raw = raw_awareness();
    let raw_client = raw.client_id().get();
    raw.set_local_state(json!({"name": "kept-peer"})).unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw), &limits)
        .unwrap();

    // Same-store whole-document replacement must NOT drop awareness.
    engine
        .prepare_root_replacement_json(
            20,
            &json!({"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"replaced"}]}]})
                .to_string(),
            ReplacementHistory::ResetAndClear,
        )
        .unwrap();

    assert_eq!(engine.client_id(), client);
    assert_eq!(engine.awareness().client_id(), client);
    assert_eq!(engine.awareness().local_state(), Some(&desired));
    let peers = engine.awareness().peer_snapshot();
    assert!(peers.iter().any(|peer| peer.client_id == raw_client));
    // Same store, same awareness instance: the local clock is untouched.
    let entries = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(entries[&client].0, 2);
}

#[test]
fn undo_redo_store_swap_preserves_awareness_peers_and_local_state() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let client = engine.client_id();
    engine
        .apply_command(
            30,
            TypedCommand::InsertText {
                text: "undoable".into(),
            },
        )
        .unwrap();
    let desired = json!({"name": "undo-survivor"});
    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap();
    let mut raw = raw_awareness();
    let raw_client = raw.client_id().get();
    raw.set_local_state(json!({"name": "peer-through-undo"}))
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw), &limits)
        .unwrap();
    let entries_before = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());

    engine.undo(31).unwrap().expect("undo applies");

    assert_eq!(engine.client_id(), client);
    assert_eq!(engine.awareness().client_id(), client);
    assert_eq!(engine.awareness().local_state(), Some(&desired));
    assert!(engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == raw_client));
    // Same logical session: clocks are preserved across the internal doc swap.
    assert_eq!(
        decode_entries(&engine.awareness().encode_local_update_v1().unwrap()),
        entries_before
    );

    engine.redo(32).unwrap().expect("redo applies");
    assert_eq!(engine.awareness().client_id(), client);
    assert!(engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == raw_client));

    // Awareness stays operational after the swaps.
    let mut late_raw = raw_awareness();
    late_raw
        .set_local_state(json!({"name": "post-undo-peer"}))
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&late_raw), &limits)
        .unwrap();
    assert!(engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == late_raw.client_id().get()));
}

#[test]
fn undo_redo_after_transport_cleanup_preserves_local_tombstone_ordering() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let client = engine.client_id();
    engine
        .apply_command(
            33,
            TypedCommand::InsertText {
                text: "undoable cleanup".into(),
            },
        )
        .unwrap();
    let desired = json!({"name": "returns after undo"});
    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap();
    engine.awareness().clear_transport_states().unwrap();
    let tombstone_clock =
        decode_entries(&engine.awareness().encode_local_update_v1().unwrap())[&client].0;
    assert_eq!(tombstone_clock, 2);

    engine.undo(34).unwrap().expect("undo applies");
    let after_undo = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(after_undo[&client], (tombstone_clock, "null".into()));

    engine
        .awareness()
        .set_local_state(&desired, &limits)
        .unwrap();
    assert_eq!(
        decode_entries(&engine.awareness().encode_local_update_v1().unwrap())[&client].0,
        tombstone_clock + 1,
    );

    engine.redo(35).unwrap().expect("redo applies");
    assert_eq!(
        decode_entries(&engine.awareness().encode_local_update_v1().unwrap())[&client].0,
        tombstone_clock + 1,
    );
}

#[test]
fn undo_after_exhausting_transport_cleanup_requires_a_fresh_identity() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let client = engine.client_id();
    engine
        .apply_command(
            36,
            TypedCommand::InsertText {
                text: "undoable exhausted cleanup".into(),
            },
        )
        .unwrap();
    engine
        .awareness()
        .set_local_state(&json!({"name": "at the edge"}), &limits)
        .unwrap();
    engine
        .awareness()
        .set_live_local_clock_for_test(u32::MAX - 1);
    engine.awareness().clear_transport_states().unwrap();

    engine.undo(37).unwrap().expect("undo applies");

    let after_undo = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(after_undo[&client], (u32::MAX, "null".into()));
    let error = engine
        .awareness()
        .set_local_state(&json!({"name": "cannot return"}), &limits)
        .unwrap_err();
    assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
    assert_eq!(
        error.details.as_ref().unwrap()["requiresFreshEditorIdentity"],
        true,
    );
}

#[test]
fn query_awareness_answer_covers_all_alive_states() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let client = engine.client_id();
    engine
        .awareness()
        .set_local_state(&json!({"name": "answering"}), &limits)
        .unwrap();

    let mut peer_one = raw_awareness();
    peer_one.set_local_state(json!({"name": "one"})).unwrap();
    let mut peer_two = raw_awareness();
    peer_two.set_local_state(json!({"name": "two"})).unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&peer_one), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&peer_two), &limits)
        .unwrap();

    // Peer two disconnects; the tombstone must not appear in the answer.
    peer_two.clean_local_state();
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_two
                .update_with_clients([peer_two.client_id()])
                .unwrap()
                .encode_v1(),
            &limits,
        )
        .unwrap();

    let answer = engine.awareness().encode_full_update_v1().unwrap();
    let mut observer = raw_awareness();
    observer
        .apply_update(AwarenessUpdate::decode_v1(&answer).unwrap())
        .unwrap();
    assert_eq!(
        observer.state::<Value>(cid(client)),
        Some(json!({"name": "answering"}))
    );
    assert_eq!(
        observer.state::<Value>(peer_one.client_id()),
        Some(json!({"name": "one"}))
    );
    assert_eq!(observer.state::<Value>(peer_two.client_id()), None::<Value>);

    let entries = decode_entries(&answer);
    assert_eq!(entries.len(), 2);
}

// ---------------------------------------------------------------------------:
// security-review findings C1 (unbounded remote tombstones), I1 (remote clock
// overflow), I2 (presence suppression via clock squatting).

#[test]
fn unknown_tombstone_storms_are_dropped_without_growing_state() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();

    // A removal tombstone for a never-seen client is a protocol no-op —
    // there is nothing to remove — and must never mint a permanently stored
    // entry (the review's C1 unbounded-memory vector).
    for round in 0..4u64 {
        let mut clients = HashMap::new();
        for index in 0..64u64 {
            let client = 500_000 + round * 64 + index;
            clients.insert(
                cid(client),
                AwarenessUpdateEntry {
                    clock: 1_000_000 + index as u32,
                    json: "null".into(),
                },
            );
        }
        let applied = engine
            .awareness()
            .apply_remote_update_v1(&AwarenessUpdate { clients }.encode_v1(), &limits)
            .unwrap();
        assert_eq!(
            applied,
            crate::yrs_engine::AwarenessApplied::default(),
            "an unknown-removal storm touches nothing",
        );
    }
    assert_eq!(
        engine.awareness().stored_entry_count(),
        0,
        "unknown tombstones never accumulate",
    );
    assert!(engine.awareness().peer_snapshot().is_empty());

    // Full-update answers carry no trace of the storm: only genuinely live
    // states are ever replayed to querying peers.
    engine
        .awareness()
        .set_local_state(&json!({"name": "local"}), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(6_500, 1, r#"{"live":true}"#), &limits)
        .unwrap();
    let entries = decode_entries(&engine.awareness().encode_full_update_v1().unwrap());
    assert_eq!(entries.len(), 2, "{entries:?}");
    assert!(
        !entries
            .keys()
            .any(|client| (500_000..500_256).contains(client)),
        "storm clients are never replayed",
    );
}

#[test]
fn known_client_removal_and_reannounce_survive_the_unknown_tombstone_drop() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();

    // A legitimate removal of a KNOWN client still works...
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(7_700, 1, r#"{"name":"known"}"#), &limits)
        .unwrap();
    assert_eq!(engine.awareness().peer_snapshot().len(), 1);
    let applied = engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(7_700, 1, "null"), &limits)
        .unwrap();
    assert_eq!(applied.removed_clients, vec![7_700]);
    assert!(engine.awareness().peer_snapshot().is_empty());
    assert_eq!(
        engine.awareness().stored_entry_count(),
        1,
        "the known client's tombstone is retained for clock ordering",
    );

    // ...and the known client's later re-announce with a strictly higher
    // clock revives its presence.
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(7_700, 2, r#"{"name":"revived"}"#),
            &limits,
        )
        .unwrap();
    let peers = engine.awareness().peer_snapshot();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].clock, 2);

    // An unknown-client tombstone at a huge clock is dropped, so it cannot
    // squat the victim's clock space: the victim's first clock-1 announce
    // still lands.
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(7_701, 1_000_000, "null"), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(7_701, 1, r#"{"name":"victim"}"#),
            &limits,
        )
        .unwrap();
    let peers = engine.awareness().peer_snapshot();
    assert!(
        peers.iter().any(|peer| peer.client_id == 7_701),
        "the dropped tombstone must not suppress the victim: {peers:?}",
    );
}

#[test]
fn remote_clock_ceiling_accepts_max_minus_one_and_rejects_max_atomically() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();

    // u32::MAX - 1 is the highest admissible clock, live and tombstoned.
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(8_800, u32::MAX - 1, r#"{"name":"edge"}"#),
            &limits,
        )
        .unwrap();
    let peers = engine.awareness().peer_snapshot();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].clock, u32::MAX - 1);
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_800, u32::MAX - 1, "null"), &limits)
        .unwrap();
    assert!(
        engine.awareness().peer_snapshot().is_empty(),
        "a max-1 tombstone of a known client is a legitimate removal",
    );

    // u32::MAX is rejected for live states: yrs's own `clock += 1` paths
    // (remote removal of the local state, deterministic expiry) would
    // overflow on it — release builds wrap, overflow-checked builds panic
    // across UniFFI.
    let error = engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(8_801, u32::MAX, r#"{"name":"over"}"#),
            &limits,
        )
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some((u32::MAX - 1) as usize));
    assert_eq!(error.actual, Some(u32::MAX as usize));
    assert_eq!(error.details.as_ref().unwrap()["field"], "awarenessClock");
    assert!(engine.awareness().peer_snapshot().is_empty());

    // u32::MAX is rejected for tombstones too, even of a known client: the
    // known client keeps its live state (atomic rejection).
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_802, 4, r#"{"name":"kept"}"#), &limits)
        .unwrap();
    let error = engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_802, u32::MAX, "null"), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    let peers = engine.awareness().peer_snapshot();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].client_id, 8_802);
    assert_eq!(peers[0].clock, 4, "the rejected tombstone changed nothing");

    // A batch mixing a valid entry with a u32::MAX entry rejects atomically.
    let mut clients = HashMap::new();
    clients.insert(
        cid(8_803),
        AwarenessUpdateEntry {
            clock: 1,
            json: r#"{"name":"valid"}"#.into(),
        },
    );
    clients.insert(
        cid(8_804),
        AwarenessUpdateEntry {
            clock: u32::MAX,
            json: r#"{"name":"over"}"#.into(),
        },
    );
    let error = engine
        .awareness()
        .apply_remote_update_v1(&AwarenessUpdate { clients }.encode_v1(), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert!(
        !engine
            .awareness()
            .peer_snapshot()
            .iter()
            .any(|peer| peer.client_id == 8_803),
        "the valid batch entry must not land either",
    );
}

#[test]
fn expiry_at_the_clock_ceiling_never_panics_or_wraps() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();

    // A peer admitted at u32::MAX - 1 who goes silent is expired by our own
    // tick: the tombstone lands exactly at u32::MAX without overflowing.
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(9_100, u32::MAX - 1, r#"{"name":"quiet"}"#),
            &limits,
        )
        .unwrap();
    engine.awareness().remove_remote_state(9_100);
    assert!(
        engine.awareness().peer_snapshot().is_empty(),
        "the expired peer leaves the snapshot",
    );

    // A repeated expiry of the already-tombstoned peer is a no-op — it must
    // never bump the clock past u32::MAX (panic in overflow-checked builds,
    // wrap in release).
    engine.awareness().remove_remote_state(9_100);

    // The retained u32::MAX tombstone still wins over a lower re-announce,
    // and a u32::MAX re-announce is itself rejected at admission.
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(9_100, u32::MAX - 1, r#"{"name":"back"}"#),
            &limits,
        )
        .unwrap();
    assert!(
        engine.awareness().peer_snapshot().is_empty(),
        "the expiry tombstone's clock ordering is intact",
    );
    let error = engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(9_100, u32::MAX, r#"{"name":"back"}"#),
            &limits,
        )
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
}

#[test]
fn remote_cannot_bump_the_local_clock_toward_the_ceiling() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let local_client = engine.client_id();

    engine
        .awareness()
        .set_local_state(&json!({"name": "local"}), &limits)
        .unwrap();
    let before = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());

    // A greater remote record for our client is rejected before Yrs can
    // transfer clock ownership, even when it is a removal tombstone.
    let error = engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(local_client, u32::MAX - 1, "null"),
            &limits,
        )
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.details.as_ref().unwrap()["field"], "awarenessClock");

    // Atomic: the locally owned clock/state are untouched.
    let after = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(after, before);
    assert!(
        engine
            .awareness()
            .peer_snapshot()
            .iter()
            .any(|peer| peer.is_local),
        "the local state survives the remote removal attempt",
    );

    // The checked local clear still owns and advances the next clock.
    engine.awareness().clear_local_state().unwrap();
    assert_eq!(engine.awareness().local_state(), None);
}

#[test]
fn pre_seeded_max_clock_tombstones_cannot_squat_a_victim() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();

    // The review's I2 vector: a pre-seeded u32::MAX tombstone for a victim's
    // client ID would suppress every legitimate re-announce for the whole
    // generation. It is rejected at admission...
    let error = engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(77_777, u32::MAX, "null"), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.details.as_ref().unwrap()["field"], "awarenessClock");

    // ...so the victim's legitimate first announce succeeds.
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(77_777, 1, r#"{"name":"victim"}"#),
            &limits,
        )
        .unwrap();
    let peers = engine.awareness().peer_snapshot();
    assert_eq!(peers.len(), 1, "{peers:?}");
    assert_eq!(peers[0].client_id, 77_777);

    // A pre-seeded tombstone just under the ceiling for a never-seen victim
    // is dropped as an unknown removal (C1), closing the same squat below
    // the ceiling.
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(88_888, u32::MAX - 1, "null"), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(
            &peer_update_bytes(88_888, 1, r#"{"name":"victim-two"}"#),
            &limits,
        )
        .unwrap();
    let peers = engine.awareness().peer_snapshot();
    assert!(
        peers.iter().any(|peer| peer.client_id == 88_888),
        "the below-ceiling pre-seed must not suppress the victim either: {peers:?}",
    );
}
