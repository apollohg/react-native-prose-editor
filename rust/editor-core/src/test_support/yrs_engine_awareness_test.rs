//! Task 6: engine-owned awareness codec sealed behind `YrsDocumentEngine`.
//!
//! Wire-compatibility fixtures use an independent raw `yrs::sync::Awareness`
//! instance so both encode/apply directions are proven against the standard
//! y-protocols awareness encoding, including removal tombstones.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::tiptap_schema;
use crate::yrs_engine::{
    AwarenessLimits, DocumentScope, EditingLimits, InitializationMode, ReplacementHistory,
    ResolvedSelection, TransactionOrigin, TypedCommand, YrsDocumentEngine, YrsEngineConfig,
};
use serde_json::{json, Value};
use yrs::sync::awareness::{Awareness, AwarenessUpdate, AwarenessUpdateEntry};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ClientID, Doc};

fn cid(client_id: u64) -> ClientID {
    ClientID::new(client_id)
}

fn engine_with_scope(mode: InitializationMode, scope: Option<DocumentScope>) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope,
    })
    .unwrap()
}

fn engine(mode: InitializationMode) -> YrsDocumentEngine {
    engine_with_scope(mode, None)
}

fn scope() -> DocumentScope {
    DocumentScope {
        document_id: "doc-awareness".into(),
        lineage_id: "lineage-awareness".into(),
    }
}

/// Mirrors the `CollaborationLimits` awareness defaults in `session.rs`.
fn session_default_limits() -> AwarenessLimits {
    AwarenessLimits {
        max_awareness_peers: 1_024,
        max_awareness_peer_bytes: 64 * 1024,
        max_awareness_bytes: 10 * 1024 * 1024,
    }
}

#[derive(Debug, PartialEq)]
struct Audit {
    encoded: Vec<u8>,
    json: Option<Value>,
    html: Option<String>,
    revision: u64,
    state_revision: u64,
    selection: Option<ResolvedSelection>,
    can_undo: bool,
    can_redo: bool,
    origin: Option<TransactionOrigin>,
}

fn audit(engine: &YrsDocumentEngine) -> Audit {
    Audit {
        encoded: engine.encoded_state().unwrap(),
        json: engine.document_json(),
        html: engine.document_html(),
        revision: engine.revision(),
        state_revision: engine.state_revision(),
        selection: engine.resolved_selection().cloned(),
        can_undo: engine.can_undo(),
        can_redo: engine.can_redo(),
        origin: engine.last_committed_origin(),
    }
}

fn raw_awareness() -> Awareness {
    Awareness::new(Doc::new())
}

fn raw_update_bytes(awareness: &Awareness) -> Vec<u8> {
    awareness.update().unwrap().encode_v1()
}

fn peer_update_bytes(client_id: u64, clock: u32, state_json: &str) -> Vec<u8> {
    let mut clients = HashMap::new();
    clients.insert(
        cid(client_id),
        AwarenessUpdateEntry {
            clock,
            json: state_json.into(),
        },
    );
    AwarenessUpdate { clients }.encode_v1()
}

fn decode_entries(bytes: &[u8]) -> HashMap<u64, (u32, String)> {
    AwarenessUpdate::decode_v1(bytes)
        .unwrap()
        .clients
        .into_iter()
        .map(|(client, entry)| (client.get(), (entry.clock, entry.json.to_string())))
        .collect()
}

#[test]
fn codec_shares_the_engine_client_identity() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let engine_client = engine.client_id();
    let limits = session_default_limits();

    assert_eq!(engine.awareness().client_id(), engine_client);

    // A local durable edit does not disturb the binding.
    engine
        .apply_command(1, TypedCommand::InsertText { text: "hi".into() })
        .unwrap();
    assert_eq!(engine.client_id(), engine_client);
    assert_eq!(engine.awareness().client_id(), engine_client);

    // The encoded local update carries the engine's client identity.
    engine
        .awareness()
        .set_local_state(&json!({"name": "local"}), &limits)
        .unwrap();
    let entries = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());
    assert_eq!(entries.len(), 1);
    assert!(entries.contains_key(&engine_client));
}

#[test]
fn local_state_round_trips_into_a_raw_yrs_awareness() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let state = json!({"name": "alice", "cursor": {"anchor": 3, "head": 5}});

    engine.awareness().set_local_state(&state, &limits).unwrap();
    assert_eq!(engine.awareness().local_state(), Some(&state));

    let mut raw = raw_awareness();
    raw.apply_update(
        AwarenessUpdate::decode_v1(&engine.awareness().encode_local_update_v1().unwrap()).unwrap(),
    )
    .unwrap();
    let engine_client = engine.client_id();
    assert_eq!(raw.state::<Value>(cid(engine_client)), Some(state.clone()));
    assert_eq!(raw.meta(cid(engine_client)).unwrap().0, 1);

    // Updating the local state advances the awareness clock.
    let updated = json!({"name": "alice", "cursor": {"anchor": 6, "head": 6}});
    engine
        .awareness()
        .set_local_state(&updated, &limits)
        .unwrap();
    raw.apply_update(
        AwarenessUpdate::decode_v1(&engine.awareness().encode_local_update_v1().unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(raw.state::<Value>(cid(engine_client)), Some(updated));
    assert_eq!(raw.meta(cid(engine_client)).unwrap().0, 2);
}

#[test]
fn remote_updates_from_a_raw_yrs_awareness_project_into_peer_snapshots() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();

    let mut raw = raw_awareness();
    let raw_client = raw.client_id().get();
    let peer_state = json!({"name": "bob", "color": "#00ff00"});
    raw.set_local_state(peer_state.clone()).unwrap();

    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw), &limits)
        .unwrap();

    let peers = engine.awareness().peer_snapshot();
    let bob = peers
        .iter()
        .find(|peer| peer.client_id == raw_client)
        .expect("raw peer projected into the snapshot");
    assert_eq!(bob.clock, 1);
    assert!(!bob.is_local);
    assert_eq!(bob.state, peer_state);

    // The local entry projects too, flagged as local.
    engine
        .awareness()
        .set_local_state(&json!({"name": "local"}), &limits)
        .unwrap();
    let peers = engine.awareness().peer_snapshot();
    let local_client = engine.client_id();
    let local = peers
        .iter()
        .find(|peer| peer.client_id == local_client)
        .expect("local state projected into the snapshot");
    assert!(local.is_local);
    assert_eq!(local.state, json!({"name": "local"}));
}

#[test]
fn removal_tombstones_round_trip_in_both_directions() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    let engine_client = engine.client_id();

    // Ours -> raw: clearing the local state broadcasts a tombstone.
    engine
        .awareness()
        .set_local_state(&json!({"name": "gone-soon"}), &limits)
        .unwrap();
    let mut raw = raw_awareness();
    raw.apply_update(
        AwarenessUpdate::decode_v1(&engine.awareness().encode_local_update_v1().unwrap()).unwrap(),
    )
    .unwrap();
    assert!(raw.state::<Value>(cid(engine_client)).is_some());

    engine.awareness().clear_local_state().unwrap();
    assert_eq!(engine.awareness().local_state(), None);
    let tombstone = engine.awareness().encode_local_update_v1().unwrap();
    let entries = decode_entries(&tombstone);
    assert_eq!(entries[&engine_client].1, "null");
    raw.apply_update(AwarenessUpdate::decode_v1(&tombstone).unwrap())
        .unwrap();
    assert_eq!(raw.state::<Value>(cid(engine_client)), None::<Value>);

    // Raw -> ours: a raw client's removal drops the projected peer.
    let mut raw_peer = raw_awareness();
    let raw_client = raw_peer.client_id().get();
    raw_peer.set_local_state(json!({"name": "raw"})).unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw_peer), &limits)
        .unwrap();
    assert!(engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == raw_client));

    raw_peer.clean_local_state();
    let raw_tombstone = raw_peer
        .update_with_clients([cid(raw_client)])
        .unwrap()
        .encode_v1();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_tombstone, &limits)
        .unwrap();
    assert!(!engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == raw_client));
}

#[test]
fn awareness_traffic_never_mutates_durable_engine_state() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    engine
        .apply_command(
            10,
            TypedCommand::InsertText {
                text: "base".into(),
            },
        )
        .unwrap();
    let baseline = audit(&engine);

    engine
        .awareness()
        .set_local_state(&json!({"name": "auditor"}), &limits)
        .unwrap();
    assert_eq!(audit(&engine), baseline);

    let mut raw = raw_awareness();
    raw.set_local_state(json!({"name": "peer"})).unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&raw_update_bytes(&raw), &limits)
        .unwrap();
    assert_eq!(audit(&engine), baseline);

    engine.awareness().encode_local_update_v1().unwrap();
    engine.awareness().encode_full_update_v1().unwrap();
    engine.awareness().peer_snapshot();
    assert_eq!(audit(&engine), baseline);

    engine.awareness().clear_local_state().unwrap();
    assert_eq!(audit(&engine), baseline);
}

#[test]
fn local_state_ceilings_accept_exact_and_reject_one_over() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let state = json!({"n": "abcdefgh"});
    let exact = serde_json::to_string(&state).unwrap().len();

    let exact_limits = AwarenessLimits {
        max_awareness_peer_bytes: exact,
        ..session_default_limits()
    };
    engine
        .awareness()
        .set_local_state(&state, &exact_limits)
        .unwrap();
    assert_eq!(engine.awareness().local_state(), Some(&state));
    let before_entries = decode_entries(&engine.awareness().encode_local_update_v1().unwrap());

    let one_under_limits = AwarenessLimits {
        max_awareness_peer_bytes: exact - 1,
        ..session_default_limits()
    };
    let bigger = json!({"n": "abcdefgi"});
    let error = engine
        .awareness()
        .set_local_state(&bigger, &one_under_limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact - 1));
    assert_eq!(error.actual, Some(exact));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxAwarenessPeerBytes"
    );
    // Rejection is atomic: the previous state and clock are untouched.
    assert_eq!(engine.awareness().local_state(), Some(&state));
    assert_eq!(
        decode_entries(&engine.awareness().encode_local_update_v1().unwrap()),
        before_entries
    );
}

#[test]
fn remote_peer_byte_ceiling_accepts_exact_and_rejects_one_over_atomically() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let exact_state = serde_json::to_string(&json!({"p": "12345678"})).unwrap();
    let limits = AwarenessLimits {
        max_awareness_peer_bytes: exact_state.len(),
        ..session_default_limits()
    };

    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(7_001, 1, &exact_state), &limits)
        .unwrap();
    assert!(engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == 7_001));

    let over_state = serde_json::to_string(&json!({"p": "123456789"})).unwrap();
    let error = engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(7_002, 1, &over_state), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(exact_state.len()));
    assert_eq!(error.actual, Some(over_state.len()));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxAwarenessPeerBytes"
    );
    assert!(!engine
        .awareness()
        .peer_snapshot()
        .iter()
        .any(|peer| peer.client_id == 7_002));
}

#[test]
fn remote_peer_count_ceiling_accepts_exact_and_rejects_one_over_atomically() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = AwarenessLimits {
        max_awareness_peers: 2,
        ..session_default_limits()
    };

    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_001, 1, "{\"i\":1}"), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_002, 1, "{\"i\":2}"), &limits)
        .unwrap();
    assert_eq!(engine.awareness().peer_snapshot().len(), 2);

    let error = engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_003, 1, "{\"i\":3}"), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(2));
    assert_eq!(error.actual, Some(3));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxAwarenessPeers"
    );
    assert_eq!(engine.awareness().peer_snapshot().len(), 2);

    // A single over-count batch is rejected atomically: none of its entries land.
    let mut clients = HashMap::new();
    for (index, client) in [8_004u64, 8_005, 8_006].iter().enumerate() {
        clients.insert(
            cid(*client),
            AwarenessUpdateEntry {
                clock: 1,
                json: format!("{{\"i\":{index}}}").into(),
            },
        );
    }
    let mut fresh = engine_with_scope(InitializationMode::LocalEmpty, None);
    let error = fresh
        .awareness()
        .apply_remote_update_v1(&AwarenessUpdate { clients }.encode_v1(), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(fresh.awareness().peer_snapshot().len(), 0);

    // Removing a tracked peer frees the slot again.
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_001, 1, "null"), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(8_003, 1, "{\"i\":3}"), &limits)
        .unwrap();
    assert_eq!(engine.awareness().peer_snapshot().len(), 2);
}

#[test]
fn aggregate_awareness_byte_ceiling_accepts_exact_and_rejects_one_over() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let local_state = json!({"name": "aggregate-local"});
    let peer_a = serde_json::to_string(&json!({"peer": "a-state"})).unwrap();
    let local_len = serde_json::to_string(&local_state).unwrap().len();

    // Choose the exact ceiling so local + peer A + peer B (exact) fills it.
    let peer_b_exact = serde_json::to_string(&json!({"peer": "b-fill"})).unwrap();
    let ceiling = local_len + peer_a.len() + peer_b_exact.len();
    let limits = AwarenessLimits {
        max_awareness_bytes: ceiling,
        ..session_default_limits()
    };

    engine
        .awareness()
        .set_local_state(&local_state, &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(9_001, 1, &peer_a), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(9_002, 1, &peer_b_exact), &limits)
        .unwrap();
    assert_eq!(engine.awareness().peer_snapshot().len(), 3);

    // Growing peer B by one byte pushes the aggregate one over the ceiling.
    let peer_b_over = serde_json::to_string(&json!({"peer": "b-fill2"})).unwrap();
    assert_eq!(peer_b_over.len(), peer_b_exact.len() + 1);
    let error = engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(9_002, 2, &peer_b_over), &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(ceiling));
    assert_eq!(error.actual, Some(ceiling + 1));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxAwarenessBytes"
    );

    // Atomic: peer B keeps its admitted state and clock.
    let peers = engine.awareness().peer_snapshot();
    let peer_b = peers.iter().find(|peer| peer.client_id == 9_002).unwrap();
    assert_eq!(peer_b.clock, 1);
    assert_eq!(serde_json::to_string(&peer_b.state).unwrap(), peer_b_exact);
}

#[test]
fn oversized_awareness_payloads_reject_before_decode_work() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = AwarenessLimits {
        max_awareness_bytes: 64,
        ..session_default_limits()
    };

    // One over the raw ingress bound: rejected as a limit, not a decode error,
    // even though the payload is garbage.
    let error = engine
        .awareness()
        .apply_remote_update_v1(&[0xff; 65], &limits)
        .unwrap_err();
    assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED");
    assert_eq!(error.limit, Some(64));
    assert_eq!(error.actual, Some(65));
    assert_eq!(
        error.details.as_ref().unwrap()["field"],
        "maxAwarenessBytes"
    );

    // At the exact bound the length gate passes and the garbage reaches the
    // decoder, which rejects it structurally.
    let error = engine
        .awareness()
        .apply_remote_update_v1(&[0xff; 64], &limits)
        .unwrap_err();
    assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
    assert_eq!(engine.awareness().peer_snapshot().len(), 0);
}

#[test]
fn malformed_awareness_bytes_reject_without_state_change() {
    let mut engine = engine(InitializationMode::LocalEmpty);
    let limits = session_default_limits();
    engine
        .awareness()
        .set_local_state(&json!({"name": "steady"}), &limits)
        .unwrap();
    engine
        .awareness()
        .apply_remote_update_v1(&peer_update_bytes(11_001, 1, "{\"ok\":true}"), &limits)
        .unwrap();
    let baseline_peers = engine.awareness().peer_snapshot();
    let baseline_audit = audit(&engine);

    for corrupt in [&[0xff][..], &[1][..], &[2, 0, 0][..]] {
        let error = engine
            .awareness()
            .apply_remote_update_v1(corrupt, &limits)
            .unwrap_err();
        assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");
    }

    // A structurally valid update whose state payload is not JSON is rejected
    // atomically alongside every other entry in the batch.
    let mut clients = HashMap::new();
    clients.insert(
        cid(11_002),
        AwarenessUpdateEntry {
            clock: 1,
            json: "{not-json".into(),
        },
    );
    clients.insert(
        cid(11_003),
        AwarenessUpdateEntry {
            clock: 1,
            json: "{\"fine\":true}".into(),
        },
    );
    let error = engine
        .awareness()
        .apply_remote_update_v1(&AwarenessUpdate { clients }.encode_v1(), &limits)
        .unwrap_err();
    assert_eq!(error.code, "COLLABORATION_DECODE_FAILED");

    assert_eq!(engine.awareness().peer_snapshot(), baseline_peers);
    assert_eq!(audit(&engine), baseline_audit);
}

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

    // Task 5 same-store whole-document replacement must NOT drop awareness.
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

// ---------------------------------------------------------------------------
// Task 18A: security-review findings C1 (unbounded remote tombstones), I1
// (remote clock overflow), I2 (presence suppression via clock squatting).
// ---------------------------------------------------------------------------

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
