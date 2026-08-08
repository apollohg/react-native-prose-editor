//! UniFFI v2 collaboration entry points.
//!
//! Native owns socket I/O; this module exposes Rust's one directive driver
//! plus a retained ACK/NACK outbound lease. Rust alone owns generations,
//! retry/deadline policy, protocol framing, and queue ordering.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use crate::collaboration_runtime::state::{SocketCloseDisposition, TransportGeneration};

use super::editor::{json_result, unit_result, with_editor, INTERNAL_UNCORRELATED_REQUEST_ID};
use super::types::{
    decimal_u64, parse_canonical_u64, FfiError, FfiJsonResult, FfiOutboundLease,
    FfiOutboundLeaseResult, FfiUnitResult,
};

fn parse_generation(generation: &str) -> Result<u64, FfiError> {
    parse_canonical_u64(generation).ok_or_else(|| {
        FfiError::new(
            crate::session::ErrorDomain::Boundary,
            "CONFIG_INVALID",
            format!("malformed transport generation: {generation:?}"),
        )
    })
}

fn parse_now_millis(now_millis: &str) -> Result<u64, FfiError> {
    parse_canonical_u64(now_millis).ok_or_else(|| {
        FfiError::new(
            crate::session::ErrorDomain::Boundary,
            "CONFIG_INVALID",
            format!("malformed transport nowMillis: {now_millis:?}"),
        )
    })
}

fn directive_json(directive: crate::session::CollaborationTransportDirective) -> String {
    serde_json::json!({
        "transportState": directive.transport_state.as_str(),
        "generationToOpen": directive.generation_to_open.map(|generation| decimal_u64(generation.value())),
        "nextDeadlineMillis": directive.next_deadline_millis.map(decimal_u64),
        "remoteCommitApplied": directive.remote_commit_applied,
        "peersChanged": directive.peers_changed,
        "renewedLocal": directive.renewed_local,
        "expiredPeers": directive.expired_peers.into_iter().map(decimal_u64).collect::<Vec<_>>(),
    })
    .to_string()
}

/// Drive initial connection, reconnect eligibility, local-awareness renewal,
/// and remote-peer expiry from one Rust-owned monotonic clock operation.
#[uniffi::export]
pub fn editor_v2_collaboration_drive(editor_id: String, now_millis: String) -> FfiJsonResult {
    let now_millis = match parse_now_millis(&now_millis) {
        Ok(now_millis) => now_millis,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .collaboration_drive(INTERNAL_UNCORRELATED_REQUEST_ID, now_millis)
            .map(directive_json)
    }))
}

/// Socket open queues framed Sync Step 1 at protocol priority and returns
/// only the frozen directive; bytes are retrieved through `lease_outbound`.
#[uniffi::export]
pub fn editor_v2_collaboration_socket_open(
    editor_id: String,
    generation: String,
    now_millis: String,
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    let now_millis = match parse_now_millis(&now_millis) {
        Ok(now_millis) => now_millis,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .collaboration_socket_open(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
                now_millis,
            )
            .map(directive_json)
    }))
}

#[uniffi::export]
pub fn editor_v2_collaboration_receive(
    editor_id: String,
    generation: String,
    message: Vec<u8>,
    now_millis: String,
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    let now_millis = match parse_now_millis(&now_millis) {
        Ok(now_millis) => now_millis,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .collaboration_receive(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
                &message,
                now_millis,
            )
            .map(directive_json)
    }))
}

/// `reason` carries no classification weight (Rust alone owns retry
/// eligibility); a policy-violation close code (1008) parks the transport
/// `Incompatible`, every other reported close is retryable.
#[uniffi::export]
pub fn editor_v2_collaboration_socket_close(
    editor_id: String,
    generation: String,
    code: Option<u32>,
    reason: Option<String>,
    now_millis: String,
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    let now_millis = match parse_now_millis(&now_millis) {
        Ok(now_millis) => now_millis,
        Err(error) => return FfiJsonResult::err(error),
    };
    let _ = reason;
    let disposition = match code {
        Some(1008) => SocketCloseDisposition::Incompatible,
        _ => SocketCloseDisposition::Retryable,
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .collaboration_socket_close(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
                disposition,
                now_millis,
            )
            .map(directive_json)
    }))
}

/// Lease one retained complete outbound frame. Exactly one of value, empty,
/// or error is selected; protocol replies retain priority over document
/// updates unless a frame was already leased.
#[uniffi::export]
pub fn editor_v2_collaboration_lease_outbound(
    editor_id: String,
    generation: String,
) -> FfiOutboundLeaseResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiOutboundLeaseResult::err(error),
    };
    match with_editor(&editor_id, |session| {
        session.lease_outbound(
            INTERNAL_UNCORRELATED_REQUEST_ID,
            TransportGeneration::from_value(generation),
        )
    }) {
        Ok(Some(lease)) => FfiOutboundLeaseResult::ok(FfiOutboundLease {
            lease_id: lease.lease_id.to_string(),
            frame: lease.frame,
        }),
        Ok(None) => FfiOutboundLeaseResult::empty(),
        Err(error) => FfiOutboundLeaseResult::err(error),
    }
}

#[uniffi::export]
pub fn editor_v2_collaboration_ack_outbound(
    editor_id: String,
    generation: String,
    lease_id: String,
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    let lease_id = match parse_generation(&lease_id) {
        Ok(lease_id) => lease_id,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .ack_outbound(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
                lease_id,
            )
            .map(|()| "{}".into())
    }))
}

#[uniffi::export]
pub fn editor_v2_collaboration_nack_outbound(
    editor_id: String,
    generation: String,
    lease_id: String,
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    let lease_id = match parse_generation(&lease_id) {
        Ok(lease_id) => lease_id,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .nack_outbound(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
                lease_id,
            )
            .map(|()| "{}".into())
    }))
}

/// Publishes a typed local awareness intent; the literal JSON `null`
/// withdraws it (standard tombstone broadcast). Rust validates the closed
/// shape and owns any published sticky cursor representation.
#[uniffi::export]
pub fn editor_v2_collaboration_set_awareness(
    editor_id: String,
    awareness_json: String,
) -> FfiUnitResult {
    unit_result(with_editor(&editor_id, |session| {
        if awareness_json.trim() == "null" {
            session.clear_desired_awareness(INTERNAL_UNCORRELATED_REQUEST_ID)
        } else {
            session.set_awareness_intent(INTERNAL_UNCORRELATED_REQUEST_ID, &awareness_json)
        }
    }))
}

#[uniffi::export]
pub fn editor_v2_collaboration_set_awareness_selection(
    editor_id: String,
    selection_json: String,
) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        session
            .set_awareness_selection(INTERNAL_UNCORRELATED_REQUEST_ID, &selection_json)
            .map(|outcome| {
                serde_json::json!({
                    "outboundChanged": outcome.outbound_changed,
                })
                .to_string()
            })
    }))
}

/// Live awareness peer projections; client ids are decimal strings so full
/// u64 ids survive the JSON round-trip.
#[uniffi::export]
pub fn editor_v2_collaboration_peers(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        let peers = session
            .awareness_peers()?
            .into_iter()
            .map(|peer| {
                serde_json::json!({
                    "clientId": decimal_u64(peer.client_id),
                    "clock": peer.clock,
                    "isLocal": peer.is_local,
                    "state": peer.state,
                    "cursor": peer.cursor.map(|cursor| serde_json::json!({
                        "anchor": cursor.anchor,
                        "head": cursor.head,
                    })),
                })
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({ "peers": peers }).to_string())
    }))
}

/// Tears down transport state while retaining the editor, document, desired
/// awareness state, and pending outbox entries.
#[uniffi::export]
pub fn editor_v2_collaboration_detach(editor_id: String) -> FfiUnitResult {
    unit_result(with_editor(&editor_id, |session| {
        session.detach(INTERNAL_UNCORRELATED_REQUEST_ID)
    }))
}

/// Reopens the transport lifecycle after an explicit detach.
#[uniffi::export]
pub fn editor_v2_collaboration_reattach(editor_id: String) -> FfiUnitResult {
    unit_result(with_editor(&editor_id, |session| {
        session.reattach(INTERNAL_UNCORRELATED_REQUEST_ID)
    }))
}
