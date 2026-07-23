//! Task 12: UniFFI v2 collaboration entry points (production since the
//! Task 16C cutover removed the staging gate).
//!
//! The generation flow is exactly the Task 8–10 session surface: transport
//! callbacks carry the raw generation value, `socket_open` returns the owed
//! framed Sync Step 1 as direct bytes, `receive` reports the structured
//! outcome (including generation-closing failures as a nested error
//! object), and `take_outbound` returns ONE frame per call — protocol
//! replies before document updates — with an empty queue reported as the
//! documented empty value (empty bytes). Rust owns retry eligibility: a
//! reported close is retryable unless it carries the WebSocket
//! policy-violation code (1008), which parks the transport `Incompatible`.

#![allow(
    clippy::result_large_err,
    reason = "SessionError is the established unboxed session error envelope"
)]

use crate::collaboration_runtime::protocol::ReceiveDisposition;
use crate::collaboration_runtime::state::{SocketCloseDisposition, TransportGeneration};

use super::editor::{
    json_result, session_error_json, unit_result, with_editor, INTERNAL_UNCORRELATED_REQUEST_ID,
};
use super::types::{
    decimal_u64, parse_canonical_u64, FfiBytesResult, FfiError, FfiJsonResult, FfiUnitResult,
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
            format!("malformed awareness nowMillis: {now_millis:?}"),
        )
    })
}

fn bytes_result(result: Result<Vec<u8>, super::types::FfiError>) -> FfiBytesResult {
    match result {
        Ok(value) => FfiBytesResult::ok(value),
        Err(error) => FfiBytesResult::err(error),
    }
}

#[uniffi::export]
pub fn editor_v2_collaboration_begin_connect(editor_id: String) -> FfiJsonResult {
    json_result(with_editor(&editor_id, |session| {
        session
            .begin_connect(INTERNAL_UNCORRELATED_REQUEST_ID)
            .map(|generation| {
                serde_json::json!({ "generation": decimal_u64(generation.value()) }).to_string()
            })
    }))
}

/// On acceptance the socket owes Sync Step 1 immediately; the framed
/// message rides back as direct bytes.
#[uniffi::export]
pub fn editor_v2_collaboration_socket_open(
    editor_id: String,
    generation: String,
) -> FfiBytesResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiBytesResult::err(error),
    };
    bytes_result(with_editor(&editor_id, |session| {
        session.socket_opened(
            INTERNAL_UNCORRELATED_REQUEST_ID,
            TransportGeneration::from_value(generation),
        )?;
        session.sync_step1_message(INTERNAL_UNCORRELATED_REQUEST_ID)
    }))
}

#[uniffi::export]
pub fn editor_v2_collaboration_receive(
    editor_id: String,
    generation: String,
    message: Vec<u8>,
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        let outcome = session.receive_message(
            INTERNAL_UNCORRELATED_REQUEST_ID,
            TransportGeneration::from_value(generation),
            &message,
        )?;
        let close = match &outcome.disposition {
            ReceiveDisposition::Continue => serde_json::Value::Null,
            ReceiveDisposition::CloseGeneration { close, error } => serde_json::json!({
                "disposition": match close {
                    SocketCloseDisposition::Retryable => "retryable",
                    SocketCloseDisposition::Incompatible => "incompatible",
                },
                "error": session_error_json(error),
            }),
        };
        Ok(serde_json::json!({
            "framesDecoded": outcome.frames_decoded,
            "repliesEnqueued": outcome.replies_enqueued,
            "replyBytesEnqueued": outcome.reply_bytes_enqueued,
            "remoteCommitApplied": outcome.remote_commit_applied,
            "documentPromoted": outcome.document_promoted,
            "transportState": outcome.transport_state.as_str(),
            "close": close,
        })
        .to_string())
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
) -> FfiJsonResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiJsonResult::err(error),
    };
    let _ = reason;
    let disposition = match code {
        Some(1008) => SocketCloseDisposition::Incompatible,
        _ => SocketCloseDisposition::Retryable,
    };
    json_result(with_editor(&editor_id, |session| {
        session
            .socket_closed(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
                disposition,
            )
            .map(|state| serde_json::json!({ "transportState": state.as_str() }).to_string())
    }))
}

/// ONE outbound frame per call: pending protocol replies first, then
/// document updates (raw outbox updates wrapped in standard Sync Update
/// framing at pickup, so every frame is a complete y-protocols message);
/// an empty queue returns the documented empty value (empty bytes).
#[uniffi::export]
pub fn editor_v2_collaboration_take_outbound(
    editor_id: String,
    generation: String,
) -> FfiBytesResult {
    let generation = match parse_generation(&generation) {
        Ok(generation) => generation,
        Err(error) => return FfiBytesResult::err(error),
    };
    bytes_result(with_editor(&editor_id, |session| {
        session
            .take_next_outbound_frame(
                INTERNAL_UNCORRELATED_REQUEST_ID,
                TransportGeneration::from_value(generation),
            )
            .map(Option::unwrap_or_default)
    }))
}

/// Publishes the desired local awareness state; the literal JSON `null`
/// withdraws it (standard tombstone broadcast).
#[uniffi::export]
pub fn editor_v2_collaboration_set_awareness(
    editor_id: String,
    awareness_json: String,
) -> FfiUnitResult {
    unit_result(with_editor(&editor_id, |session| {
        if awareness_json.trim() == "null" {
            session.clear_desired_awareness(INTERNAL_UNCORRELATED_REQUEST_ID)
        } else {
            session.set_desired_awareness(INTERNAL_UNCORRELATED_REQUEST_ID, &awareness_json)
        }
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

/// Performs deterministic awareness renewal and expiry work. `now_millis`
/// must be a canonical decimal u64 because JavaScript cannot safely carry
/// the full clock range as a number.
#[uniffi::export]
pub fn editor_v2_collaboration_tick(editor_id: String, now_millis: String) -> FfiJsonResult {
    let now_millis = match parse_now_millis(&now_millis) {
        Ok(now_millis) => now_millis,
        Err(error) => return FfiJsonResult::err(error),
    };
    json_result(with_editor(&editor_id, |session| {
        let outcome = session.awareness_tick(INTERNAL_UNCORRELATED_REQUEST_ID, now_millis)?;
        Ok(serde_json::json!({
            "nextDeadlineMillis": outcome.next_deadline_millis.map(decimal_u64),
            "renewedLocal": outcome.renewed_local,
            "expiredPeers": outcome.expired_peers.into_iter().map(decimal_u64).collect::<Vec<_>>(),
            "outboundChanged": outcome.outbound_changed,
            "peersChanged": outcome.peers_changed,
        })
        .to_string())
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
