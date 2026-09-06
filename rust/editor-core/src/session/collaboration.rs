impl EditorSession {
    /// Attach the collaboration runtime (Task 7: the bounded document
    /// outbox) sized from the session's validated collaboration limits.
    /// Idempotent: an already attached runtime and its pending outbox
    /// entries are preserved.
    pub(crate) fn attach_collaboration_runtime(&mut self) {
        if self.collaboration.runtime.is_none() {
            self.collaboration.runtime = Some(
                crate::collaboration_runtime::CollaborationRuntime::new(&self.collaboration.limits),
            );
        }
    }

    /// The attached runtime's outbox, if any. Detached/local-only sessions
    /// return `None`: no outbox exists for them by construction.
    pub(crate) fn collaboration_outbox(
        &self,
    ) -> Option<&crate::collaboration_runtime::CollaborationOutbox> {
        self.collaboration
            .runtime
            .as_ref()
            .map(crate::collaboration_runtime::CollaborationRuntime::outbox)
    }

    /// Split borrow used by every durable local mutation path: the engine
    /// plus the optionally attached outbox for pre-write reservation.
    pub(crate) fn engine_and_outbox(
        &mut self,
    ) -> (
        &mut YrsDocumentEngine,
        Option<&mut crate::collaboration_runtime::CollaborationOutbox>,
    ) {
        split_engine_and_outbox(&mut self.engine, &mut self.collaboration)
    }

    /// Document-state x transport-state policy matrix for whole-document
    /// replacement, checked before any engine work:
    ///
    /// - `AwaitRemote` + any transport rejects `ENGINE_NOT_READY`.
    /// - Any document state + `Connecting`/`Handshaking`/`Synchronized`
    ///   rejects the frozen lifecycle `WHOLE_DOCUMENT_REPLACEMENT_CONNECTED`.
    /// - `LocalReady`/`RoomReady` + `Detached`/`Disconnected`/`Incompatible`
    ///   is allowed. `Destroying`/`Destroyed` are unreachable via `with_alive`.
    fn admit_whole_document_replacement(&self, request_id: u64) -> Result<(), SessionError> {
        if self.document_state == DocumentState::AwaitRemote {
            let mut error = engine_not_ready();
            error.request_id = Some(request_id);
            return Err(error);
        }
        match self.transport_state() {
            TransportState::Connecting
            | TransportState::Handshaking
            | TransportState::Synchronized => {
                let mut error = SessionError::new(
                    ErrorDomain::Lifecycle,
                    "WHOLE_DOCUMENT_REPLACEMENT_CONNECTED",
                    format!(
                        "whole-document replacement is rejected while the collaboration \
                         transport is {}",
                        self.transport_state().as_str()
                    ),
                );
                error.request_id = Some(request_id);
                Err(error)
            }
            TransportState::Detached
            | TransportState::Disconnected
            | TransportState::Incompatible => Ok(()),
            TransportState::Destroying | TransportState::Destroyed => unreachable!(
                "with_alive rejects destroying/destroyed sessions before policy evaluation"
            ),
        }
    }

    /// Whether this session is bound to a collaboration room. Local-only
    /// sessions (`LocalReady`) have no scope to connect to, so every
    /// connection-shaped transport action on them is refused.
    fn room_bound(&self) -> bool {
        self.document_state != DocumentState::LocalReady
    }

    /// Rust's unified scheduler. It advances awareness work, issues a due
    /// initial/retry generation, and returns the sole native directive.
    pub(crate) fn collaboration_drive(
        &mut self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.finish_transport_directive(request_id, now_millis, false, false)
    }

    /// Current native socket-open callback. Sync Step 1 is framed and
    /// reserved before the state transitions, then enters the same
    /// protocol-priority lease queue as every other outbound protocol frame.
    pub(crate) fn collaboration_socket_open(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.collaboration
            .transport
            .admit_socket_open(request_id, generation)?;
        self.check_collaboration_now(request_id, now_millis)?;

        let step1 =
            crate::collaboration_runtime::protocol::sync_step1_message(&self.engine, request_id)?;
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        let reservation = runtime
            .outbox_mut()
            .reserve_protocol_replies(1, step1.len())
            .map_err(|error| {
                crate::collaboration_runtime::protocol::protocol_reply_reservation_error(
                    request_id, error,
                )
            })?;
        transport.socket_opened(request_id, generation)?;
        runtime
            .outbox_mut()
            .install_protocol_replies(reservation, request_id, vec![step1]);

        self.finish_transport_directive(request_id, now_millis, false, false)
    }

    /// Current native socket-close callback. The accepted closure retires
    /// its lease without consuming the retained frame and schedules only
    /// Rust's retry deadline.
    pub(crate) fn collaboration_socket_close(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        disposition: crate::collaboration_runtime::state::SocketCloseDisposition,
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.collaboration
            .transport
            .admit_socket_close(request_id, generation)?;
        self.check_collaboration_now(request_id, now_millis)?;
        self.collaboration.transport.socket_closed(
            request_id,
            generation,
            disposition,
            now_millis,
        )?;
        let peers_changed = self.retire_transport_scope();
        self.finish_transport_directive(request_id, now_millis, false, peers_changed)
    }

    /// Test-only local close helper for legacy lifecycle coverage.
    #[cfg(test)]
    pub(crate) fn disconnect(&mut self, request_id: u64) -> Result<(), SessionError> {
        self.collaboration.transport.disconnect(request_id)?;
        self.retire_transport_scope();
        Ok(())
    }

    /// Tear down transport state only (-> `Detached`); the runtime, its
    /// pending outbox entries, and the document are untouched by design.
    /// Remote awareness peers are transport-scoped and clear here too.
    pub(crate) fn detach(&mut self, request_id: u64) -> Result<(), SessionError> {
        self.collaboration.transport.detach(request_id)?;
        self.retire_transport_scope();
        Ok(())
    }

    /// Explicit reattach half of the `Incompatible` escape hatch:
    /// `Detached` -> `Disconnected` on a room-bound session. Peer clearing
    /// is repeated defensively — reattach begins a fresh transport scope.
    pub(crate) fn reattach(&mut self, request_id: u64) -> Result<(), SessionError> {
        let room_bound = self.room_bound();
        self.collaboration
            .transport
            .reattach(request_id, room_bound)?;
        self.retire_transport_scope();
        Ok(())
    }

    /// Task 10 lifecycle rule shared by every accepted generation-closing
    /// transition: remote awareness peers are transport-scoped, desired
    /// local awareness is retained. Sessions without an attached runtime
    /// own no peers by construction.
    fn retire_transport_scope(&mut self) -> bool {
        if let Some(runtime) = self.collaboration.runtime.as_mut() {
            runtime.outbox_mut().release_lease();
            return runtime.clear_transport_peers(&mut self.engine);
        }
        false
    }

    /// Crate-private Task 9 seam: an accepted current-generation Sync
    /// Step 2 turns `Handshaking` into `Synchronized`. Transport-only —
    /// document-state promotion is Task 9's Step 2 handling.
    #[cfg(test)]
    pub(crate) fn mark_synchronized(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
    ) -> Result<(), SessionError> {
        self.collaboration
            .transport
            .mark_synchronized(request_id, generation)
    }

    /// Task 9 protocol entry point: one bounded inbound y-sync message for
    /// the given generation. The runtime composes the sealed seams (engine
    /// state-vector/diff and prepare/commit, outbox reservation, transport
    /// generation discipline); the session only performs the field-disjoint
    /// split borrow. Sessions without an attached runtime own no protocol
    /// surface and refuse like every other runtime-shaped operation.
    pub(crate) fn collaboration_receive(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
        now_millis: u64,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        self.collaboration
            .transport
            .admit_receive(request_id, generation)?;
        self.check_collaboration_now(request_id, now_millis)?;
        let outcome = self.receive_message_at(request_id, generation, bytes, now_millis)?;
        self.finish_transport_directive(
            request_id,
            now_millis,
            outcome.remote_commit_applied,
            outcome.peers_changed,
        )
    }

    /// Test-only detailed receive seam retained for protocol matrix tests.
    #[cfg(test)]
    pub(crate) fn receive_message(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
    ) -> Result<crate::collaboration_runtime::protocol::ReceiveOutcome, SessionError> {
        self.receive_message_at(request_id, generation, bytes, 0)
    }

    fn receive_message_at(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        bytes: &[u8],
        now_millis: u64,
    ) -> Result<crate::collaboration_runtime::protocol::ReceiveOutcome, SessionError> {
        let CollaborationLifecycle {
            transport,
            runtime,
            limits,
            ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        runtime.receive_message(
            request_id,
            generation,
            crate::collaboration_runtime::protocol::ReceiveContext {
                transport,
                engine: &mut self.engine,
                document_state: &mut self.document_state,
                limits,
                now_millis,
            },
            bytes,
        )
    }

    /// Retain at most one current-generation frame until exact ACK/NACK.
    pub(crate) fn lease_outbound(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
    ) -> Result<Option<OutboundFrameLease>, SessionError> {
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        transport.admit_outbound_lease(request_id, generation)?;
        let lease = runtime.outbox_mut().lease_next().map_err(|error| {
            outbound_lease_session_error(error, request_id, "leaseOutbound", None)
        })?;
        Ok(lease.map(|lease| {
            let frame = match lease.payload {
                crate::collaboration_runtime::outbox::OutboundLeasePayload::ProtocolReply(
                    frame,
                ) => frame,
                crate::collaboration_runtime::outbox::OutboundLeasePayload::DocumentUpdate(
                    update_v1,
                ) => crate::collaboration_runtime::protocol::frame_sync_update_message(&update_v1),
            };
            OutboundFrameLease {
                lease_id: lease.lease_id.value(),
                frame,
            }
        }))
    }

    /// Consume exactly the retained current-generation lease.
    pub(crate) fn ack_outbound(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        lease_id: u64,
    ) -> Result<(), SessionError> {
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        transport.admit_outbound_lease(request_id, generation)?;
        runtime
            .outbox_mut()
            .ack_lease(crate::collaboration_runtime::outbox::OutboundLeaseId::from_value(lease_id))
            .map_err(|error| {
                outbound_lease_session_error(error, request_id, "ackOutbound", Some(lease_id))
            })
    }

    /// Release exactly the retained current-generation lease without
    /// consuming the queue front.
    pub(crate) fn nack_outbound(
        &mut self,
        request_id: u64,
        generation: crate::collaboration_runtime::state::TransportGeneration,
        lease_id: u64,
    ) -> Result<(), SessionError> {
        let CollaborationLifecycle {
            transport, runtime, ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        transport.admit_outbound_lease(request_id, generation)?;
        runtime
            .outbox_mut()
            .nack_lease(crate::collaboration_runtime::outbox::OutboundLeaseId::from_value(lease_id))
            .map_err(|error| {
                outbound_lease_session_error(error, request_id, "nackOutbound", Some(lease_id))
            })
    }

    fn check_collaboration_now(
        &self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<(), SessionError> {
        match self.collaboration.runtime.as_ref() {
            Some(runtime) => runtime.check_now_millis(request_id, now_millis),
            None => Ok(()),
        }
    }

    fn drive_awareness(
        &mut self,
        request_id: u64,
        now_millis: u64,
    ) -> Result<crate::collaboration_runtime::awareness::TickOutcome, SessionError> {
        let CollaborationLifecycle {
            transport,
            runtime,
            limits,
            ..
        } = &mut self.collaboration;
        let Some(runtime) = runtime.as_mut() else {
            return Ok(crate::collaboration_runtime::awareness::TickOutcome {
                renewed_local: false,
                outbound_changed: false,
                expired_peers: Vec::new(),
                peers_changed: false,
                next_deadline_millis: None,
            });
        };
        runtime.tick(
            request_id,
            now_millis,
            crate::collaboration_runtime::awareness::AwarenessContext {
                engine: &mut self.engine,
                transport_state: transport.state(),
                limits,
            },
        )
    }

    fn finish_transport_directive(
        &mut self,
        request_id: u64,
        now_millis: u64,
        remote_commit_applied: bool,
        peers_changed: bool,
    ) -> Result<CollaborationTransportDirective, SessionError> {
        let awareness = self.drive_awareness(request_id, now_millis)?;
        let room_bound = self.room_bound();
        let generation_to_open = self
            .collaboration
            .transport
            .drive(request_id, room_bound, now_millis)?;
        let next_deadline_millis = minimum_deadline(
            self.collaboration.transport.next_retry_deadline_millis(),
            awareness.next_deadline_millis,
        );
        Ok(CollaborationTransportDirective {
            transport_state: self.collaboration.transport.state(),
            generation_to_open,
            next_deadline_millis,
            remote_commit_applied,
            peers_changed: peers_changed || awareness.peers_changed,
            renewed_local: awareness.renewed_local,
            expired_peers: awareness.expired_peers,
        })
    }

    /// Test-only raw awareness fixture seam. Production callers must use the
    /// typed intent path below.
    #[cfg(test)]
    pub(crate) fn set_desired_awareness_for_test(
        &mut self,
        request_id: u64,
        state_json: &str,
    ) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_desired_awareness_for_test(request_id, state_json, context)
    }

    /// Production local-awareness intent: Rust validates the closed caller
    /// shape and installs an engine-owned sticky cursor before publication.
    pub(crate) fn set_awareness_intent(
        &mut self,
        request_id: u64,
        intent_json: &str,
    ) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_awareness_intent(request_id, intent_json, context)
    }

    pub(crate) fn set_awareness_selection(
        &mut self,
        request_id: u64,
        selection_json: &str,
    ) -> Result<crate::collaboration_runtime::awareness::AwarenessSelectionOutcome, SessionError>
    {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.set_awareness_selection(request_id, selection_json, context)
    }

    /// Withdraw the desired local awareness state.
    pub(crate) fn clear_desired_awareness(&mut self, request_id: u64) -> Result<(), SessionError> {
        let (runtime, context) = self.awareness_runtime_and_context()?;
        runtime.clear_desired_awareness(request_id, context)
    }

    /// The retained desired local awareness state, if any.
    pub(crate) fn desired_awareness(&self) -> Result<Option<serde_json::Value>, SessionError> {
        let runtime = self
            .collaboration
            .runtime
            .as_ref()
            .ok_or_else(no_attached_runtime)?;
        Ok(runtime.desired_awareness().cloned())
    }

    /// Task 10: public awareness peer projections with cursors resolved
    /// against the current document (recomputed on every read).
    pub(crate) fn awareness_peers(
        &mut self,
    ) -> Result<Vec<crate::collaboration_runtime::awareness::AwarenessPeerProjection>, SessionError>
    {
        let runtime = self
            .collaboration
            .runtime
            .as_ref()
            .ok_or_else(no_attached_runtime)?;
        Ok(runtime.peers(&mut self.engine))
    }

    /// Field-disjoint split borrow for awareness operations, mirroring the
    /// Task 9 receive split: the runtime plus the engine/limits/transport
    /// context it composes.
    fn awareness_runtime_and_context(
        &mut self,
    ) -> Result<
        (
            &mut crate::collaboration_runtime::CollaborationRuntime,
            crate::collaboration_runtime::awareness::AwarenessContext<'_>,
        ),
        SessionError,
    > {
        let CollaborationLifecycle {
            transport,
            runtime,
            limits,
            ..
        } = &mut self.collaboration;
        let runtime = runtime.as_mut().ok_or_else(no_attached_runtime)?;
        Ok((
            runtime,
            crate::collaboration_runtime::awareness::AwarenessContext {
                engine: &mut self.engine,
                transport_state: transport.state(),
                limits,
            },
        ))
    }

    /// Engine-owned retained dependency bytes plus the runtime's charged
    /// work units (`0` work without an attached runtime by construction).
    pub(crate) fn remote_dependency_accounting(&self) -> (usize, u64) {
        let work = self.collaboration.runtime.as_ref().map_or(
            0,
            crate::collaboration_runtime::CollaborationRuntime::remote_dependency_work,
        );
        (self.engine.pending_remote_dependency_bytes(), work)
    }

    /// Test-only collaboration-limit override by wire field name, mirroring
    /// the outbox `set_ceilings_for_test` idiom for exact/one-over receive
    /// ceiling coverage.
    pub(crate) fn set_collaboration_limit_for_test(&mut self, field: &str, value: usize) {
        self.collaboration.limits.set_for_test(field, value);
    }

    #[cfg(test)]
    pub(crate) fn collaboration_limits(&self) -> &CollaborationLimits {
        &self.collaboration.limits
    }

    /// Test-only transport-state injection, routed through the state
    /// machine so its live-attempt invariant holds (forced states carry no
    /// live attempt). It remains alongside the real transitions for exactly
    /// one reason: policy-matrix cells that are unreachable by construction
    /// — `LocalReady` sessions are permanently `Detached`, yet the Task 5
    /// replacement gate deliberately covers them for every transport state.
    /// Room-bound tests must use the real transitions instead.
    pub(crate) fn set_transport_state_for_test(&mut self, state: TransportState) {
        self.collaboration.transport.set_state_for_test(state);
    }

    pub(crate) fn lifecycle_test_session(reject_admission: bool) -> Result<Self, SessionError> {
        use crate::schema::presets::tiptap_schema;
        use crate::yrs_engine::{InitializationMode, YrsEngineConfig};

        let mut collaboration_limits = CollaborationLimits::default();
        if reject_admission {
            collaboration_limits.max_frames_per_message = 0;
        }
        collaboration_limits.validate()?;

        let engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .map_err(SessionError::from)?;

        Self::new(
            engine,
            SessionPolicy {
                read_only: false,
                input_filter: None,
                input_filter_regex: std::sync::OnceLock::new(),
                allow_base64_images: false,
            },
            DocumentState::LocalReady,
            collaboration_limits,
        )
    }

    pub(crate) fn record_lifecycle_test_call(&mut self) -> usize {
        self.native_bridge.lifecycle_test_calls += 1;
        self.native_bridge.lifecycle_test_calls
    }

    pub(crate) fn lifecycle_test_call_count(&self) -> usize {
        self.native_bridge.lifecycle_test_calls
    }

    pub(crate) fn lifecycle_test_teardown_count(&self) -> usize {
        self.collaboration.lifecycle_test_teardowns
    }
}
