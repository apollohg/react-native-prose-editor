impl EditorSession {
    pub(crate) fn get_json(&self) -> Result<serde_json::Value, SessionError> {
        self.engine.document_json().ok_or_else(engine_not_ready)
    }

    pub(crate) fn get_json_string(&self) -> Result<String, SessionError> {
        self.engine
            .document_json_string()
            .ok_or_else(engine_not_ready)
    }

    pub(crate) fn get_html(&self) -> Result<String, SessionError> {
        self.engine.document_html().ok_or_else(engine_not_ready)
    }

    /// Same-store whole-document replacement from ProseMirror JSON, behind
    /// the session policy gate. An attached collaboration runtime captures
    /// the replacement's outbound update through its bounded outbox.
    pub(crate) fn replace_document_json(
        &mut self,
        request_id: u64,
        json: &str,
        history: crate::yrs_engine::ReplacementHistory,
    ) -> Result<crate::yrs_engine::TransactionCommit, SessionError> {
        self.admit_whole_document_replacement(request_id)?;
        let (engine, outbox) = split_engine_and_outbox(&mut self.engine, &mut self.collaboration);
        engine
            .prepare_root_replacement_json_with_outbox(request_id, json, history, outbox)
            .map_err(|error| replacement_session_error(error, request_id))
    }

    /// Same-store whole-document replacement from HTML, behind the session
    /// policy gate.
    pub(crate) fn replace_document_html(
        &mut self,
        request_id: u64,
        html: &str,
        history: crate::yrs_engine::ReplacementHistory,
    ) -> Result<crate::yrs_engine::TransactionCommit, SessionError> {
        self.admit_whole_document_replacement(request_id)?;
        let options = crate::serialize::FromHtmlOptions {
            strict: false,
            allow_base64_images: self.policy.allow_base64_images,
        };
        let (engine, outbox) = split_engine_and_outbox(&mut self.engine, &mut self.collaboration);
        engine
            .prepare_root_replacement_html_with_outbox(request_id, html, &options, history, outbox)
            .map_err(|error| replacement_session_error(error, request_id))
    }

    /// Session-level snapshot export: read-only and allowed in every
    /// transport state, including connected ones (design "Snapshot and
    /// Persistence Flow": "Export is read-only and available while
    /// connected"). The engine owns manifest construction and its
    /// scope/readiness refusals; the session stamps the request id.
    pub(crate) fn export_snapshot(
        &self,
        request_id: u64,
    ) -> Result<DocumentSnapshot, SessionError> {
        self.engine
            .export_snapshot()
            .map_err(|error| snapshot_session_error(error, request_id))
    }

    /// Session-level snapshot restore behind the lifecycle policy gate
    /// (design "Whole-Document Replacement Policy" / "Snapshot and
    /// Persistence Flow"):
    ///
    /// 1. Transport gate: restore is `Detached`/`Disconnected`-only.
    ///    `Connecting`/`Handshaking`/`Synchronized` reject the frozen
    ///    snapshot-domain `SNAPSHOT_RESTORE_CONNECTED`. `Incompatible`
    ///    rejects with the same code by decision: it is not a live
    ///    transport, but it changes only through an explicit
    ///    detach/reattach (design line 124) and restore must not smuggle
    ///    a transition out of it; the frozen error contract adds exactly
    ///    one restore-gate code, which covers every non-quiescent
    ///    transport. `Destroying`/`Destroyed` are unreachable through
    ///    `with_alive`.
    /// 2. Outbox gate: pending local *document* updates reject
    ///    `SNAPSHOT_OUTBOX_NOT_EMPTY`. Pending protocol/awareness replies
    ///    never block — they are transport-scoped and cleared on success.
    /// 3. Engine restore: every manifest field validates before any decode
    ///    or mutation, the candidate installs atomically under a fresh
    ///    client identity (existing engine contract).
    ///
    /// On success the session clears the prior-store residue:
    /// `AwaitRemote` promotes to `RoomReady`, the transport settles to
    /// `Disconnected` (the design's restore rows end there; stale
    /// generations stay refused and issuance remains monotonic), and the
    /// runtime drops pending protocol replies, dependency-quarantine work
    /// accounting, and peer bookkeeping. Desired local awareness is
    /// retained — the engine's store-swap rebind re-published it under the
    /// fresh identity — and cursor projections recompute against the
    /// restored store on every read. Every failure above leaves session,
    /// engine, outbox, and runtime untouched.
    pub(crate) fn restore_snapshot(
        &mut self,
        request_id: u64,
        snapshot: &DocumentSnapshot,
    ) -> Result<crate::yrs_engine::EngineCommit, SessionError> {
        self.admit_snapshot_restore(request_id)?;
        let commit = self
            .engine
            .restore_snapshot(snapshot)
            .map_err(|error| snapshot_session_error(error, request_id))?;
        self.position_epochs.clear();
        self.native_request_ledgers.clear();
        self.native_render_cursors.clear();
        if self.document_state == DocumentState::AwaitRemote {
            self.document_state = DocumentState::RoomReady;
        }
        self.collaboration.transport.settle_for_restore();
        if let Some(runtime) = self.collaboration.runtime.as_mut() {
            runtime.reset_for_restore();
        }
        Ok(commit)
    }

    /// Document-state x transport-state policy matrix for snapshot restore,
    /// checked before any engine work. Any document state may restore
    /// (`AwaitRemote` promotes on success; `LocalReady` has no scope and
    /// fails inside the engine), so the gate is transport-first, then the
    /// document-outbox check via
    /// [`crate::collaboration_runtime::CollaborationOutbox::has_pending_document_updates`]
    /// without peeking at queue internals.
    fn admit_snapshot_restore(&self, request_id: u64) -> Result<(), SessionError> {
        match self.transport_state() {
            TransportState::Detached | TransportState::Disconnected => {}
            TransportState::Connecting
            | TransportState::Handshaking
            | TransportState::Synchronized
            | TransportState::Incompatible => {
                let mut error = SessionError::new(
                    ErrorDomain::Snapshot,
                    "SNAPSHOT_RESTORE_CONNECTED",
                    format!(
                        "snapshot restore is rejected while the collaboration \
                         transport is {}",
                        self.transport_state().as_str()
                    ),
                );
                error.request_id = Some(request_id);
                return Err(error);
            }
            TransportState::Destroying | TransportState::Destroyed => unreachable!(
                "with_alive rejects destroying/destroyed sessions before policy evaluation"
            ),
        }
        if let Some(outbox) = self.collaboration_outbox() {
            if outbox.has_pending_document_updates() {
                let mut error = SessionError::new(
                    ErrorDomain::Snapshot,
                    "SNAPSHOT_OUTBOX_NOT_EMPTY",
                    "snapshot restore is rejected while unsent local document \
                     updates are pending in the collaboration outbox",
                );
                error.request_id = Some(request_id);
                return Err(error);
            }
        }
        Ok(())
    }
}
