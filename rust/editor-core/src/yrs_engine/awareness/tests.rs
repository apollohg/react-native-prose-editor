use super::*;
use yrs::encoding::write::Write as _;
use yrs::updates::encoder::{Encoder, EncoderV1};
use yrs::Options;

fn limits() -> AwarenessLimits {
    AwarenessLimits {
        max_awareness_peers: 16,
        max_awareness_peer_bytes: 1_024,
        max_awareness_bytes: 8_192,
    }
}

fn codec() -> AwarenessCodec {
    AwarenessCodec::bind(&Doc::new())
}

fn remote_update(client_id: u64, clock: u32, json: &str) -> Vec<u8> {
    use yrs::sync::awareness::AwarenessUpdateEntry;
    use yrs::updates::encoder::Encode as _;
    let mut clients = HashMap::new();
    clients.insert(
        ClientID::new(client_id),
        AwarenessUpdateEntry {
            clock,
            json: json.into(),
        },
    );
    AwarenessUpdate { clients }.encode_v1()
}

fn ordered_remote_update(entries: &[(u64, u32, &str)]) -> Vec<u8> {
    let mut encoder = EncoderV1::new();
    encoder.write_var(entries.len());
    for (client_id, clock, json) in entries {
        encoder.write_var(*client_id);
        encoder.write_var(*clock);
        encoder.write_string(json);
    }
    encoder.to_vec()
}

fn assert_empty_after_refusal(codec: &AwarenessCodec) {
    assert!(codec.peer_snapshot().is_empty());
    assert_eq!(codec.stored_entry_count(), 0);
}

#[test]
fn task8_seventh_remediation_terminal_clock_precedes_bytes_and_json_across_wire_orders() {
    let limits = AwarenessLimits {
        max_awareness_peer_bytes: 8,
        ..limits()
    };
    let forward = [
        (300, u32::MAX, r#"{"terminal":true}"#),
        (100, 1, r#"{"oversize":true}"#),
        (200, 1, "{"),
    ];
    let reverse = [forward[2], forward[1], forward[0]];

    for entries in [&forward[..], &reverse[..]] {
        let bytes = ordered_remote_update(entries);
        for _ in 0..16 {
            let mut codec = codec();
            let error = codec.apply_remote_update_v1(&bytes, &limits).unwrap_err();
            assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
            assert_eq!(error.limit, Some((u32::MAX - 1) as usize));
            assert_eq!(error.actual, Some(u32::MAX as usize));
            assert_eq!(
                error.message,
                format!("input exceeds limit {}: {}", u32::MAX - 1, u32::MAX)
            );
            assert_eq!(error.details.as_ref().unwrap()["field"], "awarenessClock");
            assert_empty_after_refusal(&codec);
        }
    }
}

#[test]
fn task8_seventh_remediation_peer_bytes_precede_json_across_wire_orders() {
    let limits = AwarenessLimits {
        max_awareness_peer_bytes: 8,
        ..limits()
    };
    let forward = [(200, 1, r#"{"oversize":true}"#), (100, 1, "{")];
    let reverse = [forward[1], forward[0]];

    for entries in [&forward[..], &reverse[..]] {
        let bytes = ordered_remote_update(entries);
        for _ in 0..16 {
            let mut codec = codec();
            let error = codec.apply_remote_update_v1(&bytes, &limits).unwrap_err();
            assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
            assert_eq!(error.limit, Some(8));
            assert_eq!(error.actual, Some(r#"{"oversize":true}"#.len()));
            assert_eq!(
                error.message,
                format!("input exceeds limit 8: {}", r#"{"oversize":true}"#.len())
            );
            assert_eq!(
                error.details.as_ref().unwrap()["field"],
                "maxAwarenessPeerBytes",
            );
            assert_empty_after_refusal(&codec);
        }
    }
}

#[test]
fn task8_seventh_remediation_peer_byte_phase_chooses_lowest_client() {
    let limits = AwarenessLimits {
        max_awareness_peer_bytes: 4,
        ..limits()
    };
    let forward = [(200, 1, "123456789"), (100, 1, "12345")];
    let reverse = [forward[1], forward[0]];

    for entries in [&forward[..], &reverse[..]] {
        let bytes = ordered_remote_update(entries);
        for _ in 0..16 {
            let mut codec = codec();
            let error = codec.apply_remote_update_v1(&bytes, &limits).unwrap_err();
            assert_eq!(error.code, "INPUT_LIMIT_EXCEEDED", "{error:?}");
            assert_eq!(error.limit, Some(4));
            assert_eq!(error.actual, Some(5));
            assert_eq!(error.message, "input exceeds limit 4: 5");
            assert_eq!(
                error.details.as_ref().unwrap()["field"],
                "maxAwarenessPeerBytes",
            );
            assert_empty_after_refusal(&codec);
        }
    }
}

#[test]
fn task8_seventh_remediation_json_phase_chooses_lowest_client() {
    let forward = [(400, 1, "[high"), (300, 1, "{low")];
    let reverse = [forward[1], forward[0]];

    for entries in [&forward[..], &reverse[..]] {
        let bytes = ordered_remote_update(entries);
        for _ in 0..16 {
            let mut codec = codec();
            let error = codec.apply_remote_update_v1(&bytes, &limits()).unwrap_err();
            assert_eq!(error.code, "COLLABORATION_DECODE_FAILED", "{error:?}");
            assert_eq!(
                error.message,
                "awareness state for client 300 is not valid JSON",
            );
            assert_eq!(error.limit, None);
            assert_eq!(error.actual, None);
            assert_eq!(error.details, None);
            assert_empty_after_refusal(&codec);
        }
    }
}

#[test]
fn apply_remote_update_reports_sorted_touched_and_removed_clients() {
    let mut codec = codec();
    let applied = codec
        .apply_remote_update_v1(&remote_update(9_002, 1, r#"{"i":2}"#), &limits())
        .unwrap();
    assert_eq!(applied.touched_clients, vec![9_002]);
    assert!(applied.removed_clients.is_empty());

    let applied = codec
        .apply_remote_update_v1(&remote_update(9_002, 2, "null"), &limits())
        .unwrap();
    assert!(applied.touched_clients.is_empty());
    assert_eq!(applied.removed_clients, vec![9_002]);

    // A stale (equal-clock) echo touches nothing.
    let applied = codec
        .apply_remote_update_v1(&remote_update(9_002, 2, r#"{"i":3}"#), &limits())
        .unwrap();
    assert_eq!(applied, AwarenessApplied::default());
}

#[test]
fn remove_remote_state_tombstones_known_remote_clients_only() {
    let mut codec = codec();
    codec
        .set_local_state(&json!({"me": true}), &limits())
        .unwrap();
    codec
        .apply_remote_update_v1(&remote_update(9_010, 4, r#"{"peer":true}"#), &limits())
        .unwrap();
    assert_eq!(codec.peer_snapshot().len(), 2);

    // Local and unknown clients are ignored.
    codec.remove_remote_state(codec.client_id());
    codec.remove_remote_state(424_242);
    assert_eq!(codec.peer_snapshot().len(), 2);

    // A known remote tombstones with a bumped clock: an equal-clock
    // re-announce loses, a strictly newer clock reappears.
    codec.remove_remote_state(9_010);
    assert_eq!(codec.peer_snapshot().len(), 1);
    codec
        .apply_remote_update_v1(&remote_update(9_010, 5, r#"{"peer":true}"#), &limits())
        .unwrap();
    assert_eq!(codec.peer_snapshot().len(), 1, "tombstone clock preserved");
    codec
        .apply_remote_update_v1(&remote_update(9_010, 6, r#"{"peer":true}"#), &limits())
        .unwrap();
    assert_eq!(codec.peer_snapshot().len(), 2);
}

#[test]
fn clear_transport_states_drops_remotes_and_tombstones_the_live_local_entry() {
    let mut codec = codec();
    let state = json!({"me": true});
    codec.set_local_state(&state, &limits()).unwrap();
    codec
        .apply_remote_update_v1(&remote_update(9_020, 7, r#"{"peer":true}"#), &limits())
        .unwrap();

    codec.clear_transport_states().unwrap();
    assert!(
        codec.peer_snapshot().is_empty(),
        "remote entries and the offline local entry leave the snapshot",
    );
    assert_eq!(
        codec.local_state(),
        Some(&state),
        "the desired local state survives the transport reset",
    );

    // The dropped remote may re-announce at ANY clock: no stale
    // tombstone lingers from the dead generation.
    codec
        .apply_remote_update_v1(&remote_update(9_020, 1, r#"{"peer":true}"#), &limits())
        .unwrap();
    assert_eq!(codec.peer_snapshot().len(), 1);

    // Re-publishing bumps the clock past the transport-close tombstone,
    // so a peer that saw us at clock N (and tombstoned us at N + 1)
    // observes the re-publish at N + 2.
    let before_clock = {
        let update = AwarenessUpdate::decode_v1(&codec.encode_local_update_v1().unwrap()).unwrap();
        update.clients[&ClientID::new(codec.client_id())].clock
    };
    codec.set_local_state(&state, &limits()).unwrap();
    let after_clock = {
        let update = AwarenessUpdate::decode_v1(&codec.encode_local_update_v1().unwrap()).unwrap();
        update.clients[&ClientID::new(codec.client_id())].clock
    };
    assert_eq!(after_clock, before_clock + 1);

    // Idempotent when nothing local was ever published.
    let mut fresh = super::AwarenessCodec::bind(&Doc::new());
    fresh.clear_transport_states().unwrap();
    assert!(fresh.peer_snapshot().is_empty());
    assert_eq!(fresh.local_state(), None);
}

#[test]
fn remove_remote_state_at_the_clock_ceiling_never_bumps_past_max() {
    let mut codec = codec();
    codec
        .apply_remote_update_v1(
            &remote_update(9_030, u32::MAX - 1, r#"{"peer":true}"#),
            &limits(),
        )
        .unwrap();
    let stored_clock = |codec: &AwarenessCodec| {
        codec
            .awareness
            .iter()
            .find(|(client, _)| *client == ClientID::new(9_030))
            .map(|(_, state)| (state.clock, state.data.is_some()))
    };

    // Expiry of a u32::MAX - 1 peer lands the tombstone exactly at
    // u32::MAX without overflowing.
    codec.remove_remote_state(9_030);
    assert_eq!(stored_clock(&codec), Some((u32::MAX, false)));

    // Re-expiring an already-tombstoned client is a no-op: the clock can
    // never advance past u32::MAX (panic in overflow-checked builds,
    // wrap in release).
    codec.remove_remote_state(9_030);
    assert_eq!(stored_clock(&codec), Some((u32::MAX, false)));
}

fn local_clock(codec: &AwarenessCodec) -> u32 {
    codec
        .awareness
        .meta(codec.awareness.client_id())
        .map_or(0, |(clock, _)| clock)
}

fn live_codec_at(clock: u32) -> AwarenessCodec {
    let mut codec = codec();
    codec
        .set_local_state(&json!({"name": "before"}), &limits())
        .unwrap();
    codec.set_live_local_clock_for_test(clock);
    codec
}

fn same_identity_doc(codec: &AwarenessCodec) -> Doc {
    Doc::with_options(Options {
        client_id: ClientID::new(codec.client_id()),
        ..Options::default()
    })
}

fn assert_clock_exhausted(error: &YrsEngineError) {
    assert_eq!(error.code, "AWARENESS_CLOCK_EXHAUSTED", "{error:?}");
    assert!(
        error.message.contains("fresh editor identity is required"),
        "{error:?}",
    );
    assert_eq!(
        error.details.as_ref().unwrap()["requiresFreshEditorIdentity"],
        true,
        "{error:?}",
    );
}

#[test]
fn local_publish_reserves_the_final_clock_for_a_tombstone() {
    for clock in [u32::MAX - 1, u32::MAX] {
        let mut codec = live_codec_at(clock);
        let before = codec.peer_snapshot();

        let error = codec
            .set_local_state(&json!({"name": "after"}), &limits())
            .unwrap_err();

        assert_clock_exhausted(&error);
        assert_eq!(local_clock(&codec), clock);
        assert_eq!(codec.peer_snapshot(), before);
    }
}

#[test]
fn local_clear_uses_the_final_clock_once_then_reports_exhaustion() {
    let mut final_clock = live_codec_at(u32::MAX - 1);
    final_clock.clear_local_state().unwrap();
    assert_eq!(local_clock(&final_clock), u32::MAX);
    assert!(final_clock.peer_snapshot().is_empty());

    let mut exhausted = live_codec_at(u32::MAX);
    let before = exhausted.peer_snapshot();
    let error = exhausted.clear_local_state().unwrap_err();
    assert_clock_exhausted(&error);
    assert_eq!(local_clock(&exhausted), u32::MAX);
    assert_eq!(exhausted.peer_snapshot(), before);
}

#[test]
fn transport_cleanup_uses_the_final_clock_and_is_atomic_when_exhausted() {
    let mut final_clock = live_codec_at(u32::MAX - 1);
    final_clock
        .apply_remote_update_v1(&remote_update(9_040, 3, r#"{"peer":true}"#), &limits())
        .unwrap();
    final_clock.clear_transport_states().unwrap();
    assert_eq!(local_clock(&final_clock), u32::MAX);
    assert!(final_clock.peer_snapshot().is_empty());

    let mut exhausted = live_codec_at(u32::MAX);
    exhausted
        .apply_remote_update_v1(&remote_update(9_041, 3, r#"{"peer":true}"#), &limits())
        .unwrap();
    let before = exhausted.peer_snapshot();
    let error = exhausted.clear_transport_states().unwrap_err();
    assert_clock_exhausted(&error);
    assert_eq!(local_clock(&exhausted), u32::MAX);
    assert_eq!(exhausted.peer_snapshot(), before);
}

#[test]
fn same_identity_rebind_preserves_transport_tombstone_ordering() {
    let mut codec = codec();
    let state = json!({"name": "returns"});
    codec.set_local_state(&state, &limits()).unwrap();
    codec.clear_transport_states().unwrap();
    let tombstone_clock = local_clock(&codec);
    assert_eq!(tombstone_clock, 2);

    codec.rebind_preserving_peers(&same_identity_doc(&codec));

    assert_eq!(local_clock(&codec), tombstone_clock);
    assert!(codec.awareness.local_state_raw().is_none());
    codec.set_local_state(&state, &limits()).unwrap();
    assert_eq!(local_clock(&codec), tombstone_clock + 1);
}

#[test]
fn same_identity_rebind_preserves_exhausted_transport_tombstone() {
    let mut codec = live_codec_at(u32::MAX - 1);
    codec.clear_transport_states().unwrap();
    assert_eq!(local_clock(&codec), u32::MAX);

    codec.rebind_preserving_peers(&same_identity_doc(&codec));

    assert_eq!(local_clock(&codec), u32::MAX);
    let before = codec.peer_snapshot();
    let error = codec
        .set_local_state(&json!({"name": "cannot return"}), &limits())
        .unwrap_err();
    assert_clock_exhausted(&error);
    assert_eq!(local_clock(&codec), u32::MAX);
    assert_eq!(codec.peer_snapshot(), before);
}

#[test]
fn fresh_identity_rebind_recovers_from_an_exhausted_tombstone_at_clock_one() {
    let mut codec = live_codec_at(u32::MAX - 1);
    codec.clear_transport_states().unwrap();
    let old_client = codec.client_id();
    let fresh_doc = Doc::with_options(Options {
        client_id: ClientID::new(old_client ^ 1),
        ..Options::default()
    });

    codec.rebind_for_store_swap(&fresh_doc);

    assert_eq!(codec.client_id(), fresh_doc.client_id().get());
    assert_eq!(local_clock(&codec), 1);
    assert_eq!(codec.local_state(), Some(&json!({"name": "before"})));
}
