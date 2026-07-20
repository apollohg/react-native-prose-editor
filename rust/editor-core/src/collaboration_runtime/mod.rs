//! Collaboration runtime host (staging).
//!
//! Task 7 added the bounded pre-commit document outbox plus attachment
//! plumbing on `EditorSession`; Task 8 added the generation-owned transport
//! state machine ([`state::TransportStateMachine`], session-owned per the
//! established split); Task 9 added strict standard y-sync protocol
//! handling ([`protocol`]) that composes those seams with the engine's
//! sealed remote-update surface; Task 10 added runtime awareness ownership
//! ([`awareness`]): desired local state, peer projections, and the
//! deterministic renewal/expiry clocks, all wired through the engine-owned
//! `AwarenessCodec`. The runtime owns no `yrs::Doc`, no awareness object,
//! and cannot apply Yrs mutations directly; for dependency-pending updates
//! it retains only byte-unit work accounting — the payload bytes stay
//! quarantined inside the engine.

pub mod awareness;
pub mod outbox;
pub mod protocol;
pub mod state;

pub(crate) use outbox::CollaborationOutbox;

use crate::session::CollaborationLimits;

/// Per-session collaboration runtime. Attached explicitly; detached
/// (local-only) sessions own no runtime and therefore no outbox, which is
/// what makes their local editing behavior identical to pre-runtime staging
/// behavior by construction.
pub(crate) struct CollaborationRuntime {
    outbox: CollaborationOutbox,
    /// Byte-unit work charged for dependency-pending remote updates while
    /// the engine's quarantine is non-empty; reset when it drains. This is
    /// accounting metadata only — never a payload copy.
    remote_dependency_work: u64,
    /// Task 10 awareness ownership: desired-state JSON, deterministic
    /// deadlines, and projection bookkeeping (never wire or clock state —
    /// that stays in the engine-owned codec).
    awareness: awareness::AwarenessRuntimeState,
}

impl CollaborationRuntime {
    pub(crate) fn new(limits: &CollaborationLimits) -> Self {
        Self {
            outbox: CollaborationOutbox::from_limits(limits),
            remote_dependency_work: 0,
            awareness: awareness::AwarenessRuntimeState::new(),
        }
    }

    pub(crate) fn outbox(&self) -> &CollaborationOutbox {
        &self.outbox
    }

    pub(crate) fn outbox_mut(&mut self) -> &mut CollaborationOutbox {
        &mut self.outbox
    }

    /// Accumulated dependency-quarantine work in encoded-byte units.
    pub(crate) fn remote_dependency_work(&self) -> u64 {
        self.remote_dependency_work
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_hosts_an_outbox_sized_from_the_session_limits() {
        let limits = CollaborationLimits::default();
        let mut runtime = CollaborationRuntime::new(&limits);
        assert!(!runtime.outbox().has_pending_document_updates());
        assert_eq!(runtime.remote_dependency_work(), 0);
        let reservation = runtime.outbox_mut().reserve_document_update(1, 4).unwrap();
        runtime.outbox_mut().install(reservation, vec![0; 4]);
        assert_eq!(runtime.outbox().pending_document_update_count(), 1);
    }
}
