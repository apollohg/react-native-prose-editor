//! Engine-owned awareness codec.
//!
//! The collaboration runtime never receives a `yrs::Doc`, a transaction, or a
//! raw `yrs::sync::Awareness` handle. All awareness work that requires the
//! document handle is sealed behind [`AwarenessCodec`], which owns the sole
//! `Awareness` instance bound to the engine's authoritative `Doc`.
//!
//! Ceilings are taken per call as [`AwarenessLimits`] — the values come from
//! the session-level `CollaborationLimits` fields of the same names; the
//! engine deliberately does not own the session's limit struct.

use std::collections::HashMap;

use serde_json::{json, Value};
use yrs::sync::awareness::{Awareness, AwarenessUpdate};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ClientID, Doc};

use super::{YrsEngineError, YrsEngineResult};

/// The y-protocols awareness tombstone payload marking a removed state.
const AWARENESS_TOMBSTONE_JSON: &str = "null";

/// The highest clock a remote awareness entry may carry. yrs advances
/// stored clocks with `+= 1` (a remote removal of the local state bumps the
/// local clock; `remove_state` bumps the removed client's clock), so
/// admitting `u32::MAX` would let a remote peer drive those paths into an
/// overflow — a wrap to 0 in release builds, a UniFFI-crossing panic in
/// overflow-checked builds.
const MAX_ADMITTED_AWARENESS_CLOCK: u32 = u32::MAX - 1;

/// `details.field` of the clock-ceiling refusal. The ceiling is a
/// protocol-safety constant, not a configurable `CollaborationLimits`
/// field, hence the distinct naming.
const AWARENESS_CLOCK_FIELD: &str = "awarenessClock";

/// Awareness ceilings, mirroring the `max_awareness_*` fields of the
/// session-level `CollaborationLimits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwarenessLimits {
    /// Maximum number of tracked non-local peers with a live state.
    pub max_awareness_peers: usize,
    /// Maximum byte length of a single client's JSON state payload.
    pub max_awareness_peer_bytes: usize,
    /// Maximum aggregate byte length of all live JSON state payloads; also
    /// bounds a raw incoming awareness update before any decode work.
    pub max_awareness_bytes: usize,
}

/// A projected awareness entry: client identity, protocol clock, and the
/// validated JSON state. Tombstoned (removed) clients are never projected.
#[derive(Debug, Clone, PartialEq)]
pub struct AwarenessPeer {
    pub client_id: u64,
    pub clock: u32,
    pub is_local: bool,
    pub state: Value,
}

/// What one admitted remote awareness update changed, projected from the
/// `yrs` application summary. Client lists are sorted so callers never
/// observe hash-map iteration order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AwarenessApplied {
    /// Clients whose live state was installed or refreshed (added or
    /// updated with a newer clock).
    pub touched_clients: Vec<u64>,
    /// Clients removed (tombstoned) by the update.
    pub removed_clients: Vec<u64>,
}

/// The desired local presence, retained beside the live `Awareness` so it
/// survives store swaps (snapshot restore, import) with a fresh clock.
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
struct DesiredLocalState {
    value: Value,
    raw: String,
}

pub struct AwarenessCodec {
    awareness: Awareness,
    desired_local_state: Option<DesiredLocalState>,
}

fn awareness_limit_error(field: &'static str, limit: usize, actual: usize) -> YrsEngineError {
    YrsEngineError::limit("INPUT_LIMIT_EXCEEDED", limit, actual)
        .with_details(json!({ "field": field }))
}

fn awareness_decode_error(message: impl Into<String>) -> YrsEngineError {
    YrsEngineError::new("COLLABORATION_DECODE_FAILED", message)
}

fn awareness_apply_error(message: impl Into<String>) -> YrsEngineError {
    YrsEngineError::new("COLLABORATION_APPLY_FAILED", message)
}

impl AwarenessCodec {
    /// Binds the codec to the engine's authoritative document handle. The
    /// engine constructs exactly one codec and rebinds it on store swaps, so
    /// this is the only `Awareness` over the engine's `Doc`.
    pub(crate) fn bind(doc: &Doc) -> Self {
        Self {
            awareness: Awareness::new(doc.clone()),
            desired_local_state: None,
        }
    }

    pub fn client_id(&self) -> u64 {
        self.awareness.client_id().get()
    }

    /// The desired local presence state, if one is currently published.
    // Not reachable from production call paths after the Task 16C legacy runtime
    // removal; exercised by crate tests.
    #[allow(dead_code)]
    pub fn local_state(&self) -> Option<&Value> {
        self.desired_local_state.as_ref().map(|state| &state.value)
    }

    /// Publishes the local presence state, bounded by the per-peer and
    /// aggregate awareness byte ceilings. Rejections are atomic: the previous
    /// state and its clock are untouched.
    pub fn set_local_state(
        &mut self,
        state: &Value,
        limits: &AwarenessLimits,
    ) -> YrsEngineResult<()> {
        let raw = state.to_string();
        if raw.len() > limits.max_awareness_peer_bytes {
            return Err(awareness_limit_error(
                "maxAwarenessPeerBytes",
                limits.max_awareness_peer_bytes,
                raw.len(),
            ));
        }
        let local_client = self.awareness.client_id();
        if self
            .awareness
            .meta(local_client)
            .is_some_and(|(clock, _)| clock == u32::MAX)
        {
            // The next publish would overflow yrs's `clock += 1`. The clock
            // can only reach u32::MAX through a remote clock-squatting
            // tombstone for the local client; refuse structurally instead of
            // wrapping to 0 (release) or panicking across UniFFI
            // (overflow-checked builds).
            return Err(awareness_limit_error(
                AWARENESS_CLOCK_FIELD,
                MAX_ADMITTED_AWARENESS_CLOCK as usize,
                u32::MAX as usize,
            ));
        }
        let remote_alive_bytes: usize = self
            .awareness
            .iter()
            .filter(|(client, _)| *client != local_client)
            .filter_map(|(_, state)| state.data.map(|data| data.len()))
            .sum();
        let aggregate = remote_alive_bytes.saturating_add(raw.len());
        if aggregate > limits.max_awareness_bytes {
            return Err(awareness_limit_error(
                "maxAwarenessBytes",
                limits.max_awareness_bytes,
                aggregate,
            ));
        }
        self.awareness.set_local_state_raw(raw.clone());
        self.desired_local_state = Some(DesiredLocalState {
            value: state.clone(),
            raw,
        });
        Ok(())
    }

    /// Withdraws the local presence state, broadcasting a removal tombstone
    /// through the next [`Self::encode_local_update_v1`].
    pub fn clear_local_state(&mut self) {
        // At the u32::MAX clock ceiling (reachable only through a remote
        // clock-squatting tombstone for the local client) the removal
        // tombstone cannot advance the clock; dropping the desired state
        // still withdraws presence logically and the next transport-scoped
        // reset retires the pinned entry.
        let at_ceiling = self
            .awareness
            .meta(self.awareness.client_id())
            .is_some_and(|(clock, _)| clock == u32::MAX);
        if !at_ceiling {
            self.awareness.clean_local_state();
        }
        self.desired_local_state = None;
    }

    /// Encodes the local client's awareness entry (state or removal
    /// tombstone). Before any local state was ever published the encoded
    /// update is empty.
    pub fn encode_local_update_v1(&self) -> YrsEngineResult<Vec<u8>> {
        let local_client = self.awareness.client_id();
        let update = if self.awareness.meta(local_client).is_some() {
            self.awareness
                .update_with_clients([local_client])
                .map_err(|error| {
                    awareness_apply_error(format!(
                        "local awareness state cannot be encoded: {error}"
                    ))
                })?
        } else {
            AwarenessUpdate {
                clients: HashMap::new(),
            }
        };
        Ok(update.encode_v1())
    }

    /// Encodes every live awareness state — the answer to a query-awareness
    /// protocol message. Removed clients are excluded, per y-protocols.
    pub fn encode_full_update_v1(&self) -> YrsEngineResult<Vec<u8>> {
        let update = self.awareness.update().map_err(|error| {
            awareness_apply_error(format!("awareness states cannot be encoded: {error}"))
        })?;
        Ok(update.encode_v1())
    }

    /// Applies a remote awareness update. The raw payload is bounded before
    /// any decode work; every entry is then validated (JSON payload, per-peer
    /// bytes, clock ceiling) and admitted before anything is applied, so
    /// rejections are atomic — a `u32::MAX` clock is refused even inside an
    /// otherwise-droppable tombstone. Removal tombstones for never-seen
    /// clients are dropped as protocol no-ops after admission, so they never
    /// mint stored entries. The returned [`AwarenessApplied`] reports which
    /// clients the update actually touched, so the runtime can stamp
    /// deterministic activity deadlines without duplicating the codec's
    /// clock state.
    pub fn apply_remote_update_v1(
        &mut self,
        update_v1: &[u8],
        limits: &AwarenessLimits,
    ) -> YrsEngineResult<AwarenessApplied> {
        if update_v1.len() > limits.max_awareness_bytes {
            return Err(awareness_limit_error(
                "maxAwarenessBytes",
                limits.max_awareness_bytes,
                update_v1.len(),
            ));
        }
        let update = AwarenessUpdate::decode_v1(update_v1).map_err(|error| {
            awareness_decode_error(format!("awareness update cannot decode: {error}"))
        })?;
        self.admit_update(&update, limits)?;
        let update = self.without_unknown_tombstones(update);
        let summary = self
            .awareness
            .apply_update_summary(update)
            .map_err(|error| {
                awareness_apply_error(format!("admitted awareness update cannot apply: {error}"))
            })?;
        Ok(match summary {
            Some(summary) => {
                let mut touched_clients: Vec<u64> = summary
                    .added
                    .iter()
                    .chain(summary.updated.iter())
                    .map(|client| client.get())
                    .collect();
                touched_clients.sort_unstable();
                touched_clients.dedup();
                let mut removed_clients: Vec<u64> =
                    summary.removed.iter().map(|client| client.get()).collect();
                removed_clients.sort_unstable();
                AwarenessApplied {
                    touched_clients,
                    removed_clients,
                }
            }
            None => AwarenessApplied::default(),
        })
    }

    /// Projects every live awareness state, sorted by client ID.
    pub fn peer_snapshot(&self) -> Vec<AwarenessPeer> {
        let local_client = self.awareness.client_id();
        let mut peers: Vec<AwarenessPeer> = self
            .awareness
            .iter()
            .filter_map(|(client, state)| {
                let data = state.data.as_ref()?;
                // Stored states passed ingress validation, so this parse
                // cannot fail for codec-admitted data.
                let value = serde_json::from_str(data).ok()?;
                Some(AwarenessPeer {
                    client_id: client.get(),
                    clock: state.clock,
                    is_local: client == local_client,
                    state: value,
                })
            })
            .collect();
        peers.sort_by_key(|peer| peer.client_id);
        peers
    }

    /// Drops removal tombstones for clients this node has never seen. A
    /// removal of an unknown client is a protocol no-op — there is nothing
    /// to remove, matching the y-protocols reference behavior — and yrs
    /// would otherwise permanently store the tombstone entry, bypassing the
    /// live-state ceilings (unbounded remote memory growth) and squatting
    /// the victim's clock space. Tombstones for KNOWN clients are kept:
    /// they carry the clock ordering a later re-announce must beat.
    fn without_unknown_tombstones(&self, update: AwarenessUpdate) -> AwarenessUpdate {
        let mut clients = HashMap::with_capacity(update.clients.len());
        for (client_id, entry) in update.clients {
            if entry.json.as_ref() == AWARENESS_TOMBSTONE_JSON
                && self.awareness.meta(client_id).is_none()
            {
                continue;
            }
            clients.insert(client_id, entry);
        }
        AwarenessUpdate { clients }
    }

    /// Validates an already-decoded update against every ceiling by mirroring
    /// the exact `yrs` application rule over a projection of the current
    /// states, so admission can never diverge from what `apply_update` would
    /// install.
    fn admit_update(
        &self,
        update: &AwarenessUpdate,
        limits: &AwarenessLimits,
    ) -> YrsEngineResult<()> {
        let local_client = self.awareness.client_id();
        let mut projected: HashMap<ClientID, (u32, Option<usize>)> = self
            .awareness
            .iter()
            .map(|(client, state)| (client, (state.clock, state.data.map(|data| data.len()))))
            .collect();
        for (client_id, entry) in &update.clients {
            if entry.clock > MAX_ADMITTED_AWARENESS_CLOCK {
                // yrs advances admitted clocks with `+= 1` (remote removal of
                // the local state; expiry through `remove_state`), so
                // u32::MAX is inadmissible regardless of payload.
                return Err(awareness_limit_error(
                    AWARENESS_CLOCK_FIELD,
                    MAX_ADMITTED_AWARENESS_CLOCK as usize,
                    entry.clock as usize,
                ));
            }
            let incoming_alive = entry.json.as_ref() != AWARENESS_TOMBSTONE_JSON;
            if incoming_alive {
                if entry.json.len() > limits.max_awareness_peer_bytes {
                    return Err(awareness_limit_error(
                        "maxAwarenessPeerBytes",
                        limits.max_awareness_peer_bytes,
                        entry.json.len(),
                    ));
                }
                if serde_json::from_str::<serde::de::IgnoredAny>(entry.json.as_ref()).is_err() {
                    return Err(awareness_decode_error(format!(
                        "awareness state for client {client_id} is not valid JSON"
                    )));
                }
            }
            let incoming = (entry.clock, incoming_alive.then_some(entry.json.len()));
            match projected.get_mut(client_id) {
                Some(current) => {
                    let (current_clock, current_data) = *current;
                    let is_removed =
                        current_clock == entry.clock && !incoming_alive && current_data.is_some();
                    if current_clock < entry.clock || is_removed {
                        if !incoming_alive && *client_id == local_client && current_data.is_some() {
                            // A remote peer never removes the local state; the
                            // local clock is bumped instead (yrs semantics).
                            // The admission clock ceiling keeps this bump —
                            // and yrs's own `clock += 1` — overflow-free, so
                            // the mirror never diverges from yrs here.
                            *current = (entry.clock.saturating_add(1), current_data);
                        } else {
                            *current = incoming;
                        }
                    }
                }
                None => {
                    projected.insert(*client_id, incoming);
                }
            }
        }
        let alive_peers = projected
            .iter()
            .filter(|(client, (_, data))| **client != local_client && data.is_some())
            .count();
        if alive_peers > limits.max_awareness_peers {
            return Err(awareness_limit_error(
                "maxAwarenessPeers",
                limits.max_awareness_peers,
                alive_peers,
            ));
        }
        let aggregate: usize = projected.values().filter_map(|(_, data)| *data).sum();
        if aggregate > limits.max_awareness_bytes {
            return Err(awareness_limit_error(
                "maxAwarenessBytes",
                limits.max_awareness_bytes,
                aggregate,
            ));
        }
        Ok(())
    }

    /// Test seam for the same-doc binding invariant; never part of the
    /// public surface.
    #[cfg(test)]
    pub(crate) fn doc_for_test(&self) -> &Doc {
        self.awareness.doc()
    }

    /// Test seam: total stored awareness entries, live and tombstoned, so
    /// tests can prove admission never accumulates hidden state.
    #[cfg(test)]
    pub(crate) fn stored_entry_count(&self) -> usize {
        self.awareness.iter().count()
    }

    /// Rebinds the codec after the engine replaced its store with a different
    /// document (snapshot restore, import). Stale remote peers are dropped,
    /// the desired local state is re-published under the new client identity
    /// with a fresh clock, and the previous `Doc` handle is released.
    pub(crate) fn rebind_for_store_swap(&mut self, doc: &Doc) {
        let mut next = Awareness::new(doc.clone());
        if let Some(desired) = self.desired_local_state.as_ref() {
            next.set_local_state_raw(desired.raw.clone());
        }
        self.awareness = next;
    }

    /// Tombstones one remote client's state (deterministic expiry): the
    /// standard y-protocols removal that bumps the client's clock by one, so
    /// only a strictly newer re-announce reappears. The local client is
    /// never expired, unknown clients are ignored (a vacant removal would
    /// otherwise mint a spurious clock-1 tombstone), and already-tombstoned
    /// clients are ignored (a second bump has no protocol effect and would
    /// overflow at the u32::MAX ceiling).
    pub(crate) fn remove_remote_state(&mut self, client_id: u64) {
        let client = ClientID::new(client_id);
        if client == self.awareness.client_id() {
            return;
        }
        let has_live_state = self
            .awareness
            .iter()
            .any(|(known, state)| known == client && state.data.is_some());
        if has_live_state {
            self.awareness.remove_state(client);
        }
    }

    /// Transport-scoped reset on generation close/detach/reattach: every
    /// remote entry — live or tombstoned — is dropped entirely, and a
    /// still-live local entry is tombstoned with a bumped clock (the
    /// standard y-protocols disconnect semantics) while the desired local
    /// state is retained for the reconnect re-publish. That re-publish bumps
    /// the clock once more, so a peer that tombstoned us at `clock + 1`
    /// observes the reappearance at `clock + 2` — the designed mitigation
    /// for the undo/redo tombstone-migration gap.
    pub(crate) fn clear_transport_states(&mut self) {
        let local_client = self.awareness.client_id();
        let migrated = self
            .awareness
            .meta(local_client)
            .is_some()
            .then(|| self.awareness.update_with_clients([local_client]))
            .and_then(Result::ok);
        let mut next = Awareness::new(self.awareness.doc().clone());
        if let Some(update) = migrated {
            // Unreachable-in-practice failure arm, mirroring
            // `rebind_preserving_peers`: `apply_update` has no failing path
            // in yrs 0.27.2; if that ever changes the local clock restarts
            // and the reconnect re-publish still heals within one renewal.
            let _ = next.apply_update(update);
        }
        if next.local_state_raw().is_some() {
            next.clean_local_state();
        }
        self.awareness = next;
    }

    /// Rebinds the codec across an internal same-identity store swap
    /// (undo/redo candidate installation): the logical session continues, so
    /// every live state — local and remote — migrates with its clock intact.
    pub(crate) fn rebind_preserving_peers(&mut self, doc: &Doc) {
        let migrated = self.awareness.update();
        let mut next = Awareness::new(doc.clone());
        // Unreachable-in-practice in yrs 0.27.2: `Awareness::update()` can
        // only fail with `Error::ClientNotFound`, and it enumerates its own
        // live clients; `apply_update` has no failing path at all. The
        // degraded arm is kept deliberately as a non-panicking guard against
        // a future yrs semantic change: if migration ever fails, remote peers
        // are dropped (they re-announce over the awareness protocol) and the
        // desired local state is re-published with a fresh clock.
        let migration_applied = match migrated {
            Ok(update) => next.apply_update(update).is_ok(),
            Err(_) => false,
        };
        if !migration_applied {
            if let Some(desired) = self.desired_local_state.as_ref() {
                next.set_local_state_raw(desired.raw.clone());
            }
        }
        self.awareness = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        codec.clear_transport_states();
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
            let update =
                AwarenessUpdate::decode_v1(&codec.encode_local_update_v1().unwrap()).unwrap();
            update.clients[&ClientID::new(codec.client_id())].clock
        };
        codec.set_local_state(&state, &limits()).unwrap();
        let after_clock = {
            let update =
                AwarenessUpdate::decode_v1(&codec.encode_local_update_v1().unwrap()).unwrap();
            update.clients[&ClientID::new(codec.client_id())].clock
        };
        assert_eq!(after_clock, before_clock + 1);

        // Idempotent when nothing local was ever published.
        let mut fresh = super::AwarenessCodec::bind(&Doc::new());
        fresh.clear_transport_states();
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
}
