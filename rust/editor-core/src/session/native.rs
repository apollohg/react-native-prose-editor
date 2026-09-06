impl EditorSession {
    pub(crate) fn new(
        engine: YrsDocumentEngine,
        policy: SessionPolicy,
        document_state: DocumentState,
        collaboration_limits: CollaborationLimits,
    ) -> Result<Self, SessionError> {
        collaboration_limits.validate()?;
        let transport_state = match document_state {
            DocumentState::LocalReady => TransportState::Detached,
            DocumentState::AwaitRemote | DocumentState::RoomReady => TransportState::Disconnected,
        };
        Ok(Self {
            engine,
            policy,
            native_bridge: NativeBridgeLifecycle {
                active: true,
                lifecycle_test_calls: 0,
            },
            document_state,
            collaboration: CollaborationLifecycle {
                active: true,
                limits: collaboration_limits,
                transport: crate::collaboration_runtime::state::TransportStateMachine::new(
                    transport_state,
                ),
                runtime: None,
                lifecycle_test_teardowns: 0,
            },
            position_epochs: crate::position_epoch::PositionEpochStore::new(
                crate::position_epoch::PositionEpochLimits::default(),
            ),
            native_request_ledgers: std::collections::BTreeMap::new(),
            native_render_cursors: std::collections::BTreeMap::new(),
        })
    }

    pub(crate) fn teardown(&mut self) {
        self.position_epochs.clear();
        self.native_request_ledgers.clear();
        self.native_render_cursors.clear();
        self.native_bridge.teardown();
        self.collaboration.teardown();
    }

    pub(crate) fn transport_state(&self) -> TransportState {
        self.collaboration.transport.state()
    }

    pub(crate) fn render_state(&self) -> EngineRenderState {
        self.engine.render_state()
    }

    pub(crate) fn pin_position_epoch(
        &mut self,
        owner_id: u64,
        document_revision: u64,
    ) -> Result<u64, SessionError> {
        if self.engine.revision() != document_revision {
            return Err(SessionError::new(
                ErrorDomain::Operation,
                "REVISION_MISMATCH",
                "position epoch revision does not match the current document",
            ));
        }
        let count = usize::try_from(
            self.engine
                .position_map()
                .ok_or_else(engine_not_ready)?
                .total_scalars(),
        )
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            SessionError::new(
                ErrorDomain::Boundary,
                "POSITION_EPOCH_LIMIT_EXCEEDED",
                "position epoch boundary count overflowed",
            )
        })?;
        self.position_epochs.admit_boundary_count(count)?;
        let boundaries = self
            .engine
            .build_position_epoch_boundaries()
            .ok_or_else(|| {
                SessionError::new(
                    ErrorDomain::Operation,
                    "ENGINE_INVARIANT_FAILED",
                    "authoritative position epoch could not be built",
                )
            })?;
        let epoch = self.position_epochs.install(
            owner_id,
            self.engine.client_id(),
            document_revision,
            boundaries,
        )?;
        let render_blocks = self
            .engine
            .cached_render_blocks()
            .ok_or_else(engine_not_ready)?;
        self.retain_native_render_cursor(owner_id, document_revision, render_blocks);
        Ok(epoch)
    }

    pub(crate) fn resolve_epoch_range(
        &self,
        owner_id: u64,
        epoch_id: u64,
        anchor: u32,
        head: u32,
    ) -> Result<crate::position_epoch::ResolvedEpochRange, SessionError> {
        let affinity = if anchor == head {
            crate::yrs_engine::Affinity::After
        } else {
            crate::yrs_engine::Affinity::Before
        };
        let (anchor_boundary, epoch_revision) =
            self.position_epochs
                .boundary(owner_id, epoch_id, self.engine.client_id(), anchor)?;
        let (head_boundary, head_epoch_revision) =
            self.position_epochs
                .boundary(owner_id, epoch_id, self.engine.client_id(), head)?;
        debug_assert_eq!(epoch_revision, head_epoch_revision);
        if epoch_revision == self.engine.revision() {
            return Ok(crate::position_epoch::ResolvedEpochRange {
                anchor,
                head,
                fallback: false,
            });
        }
        let (resolved_anchor, anchor_fallback) = self
            .engine
            .resolve_position_epoch_boundary(anchor_boundary, affinity, anchor)
            .ok_or_else(engine_not_ready)?;
        let (resolved_head, head_fallback) = self
            .engine
            .resolve_position_epoch_boundary(head_boundary, affinity, head)
            .ok_or_else(engine_not_ready)?;
        Ok(crate::position_epoch::ResolvedEpochRange {
            anchor: resolved_anchor,
            head: resolved_head,
            fallback: anchor_fallback || head_fallback,
        })
    }

    pub(crate) fn release_position_epoch_owner(&mut self, owner_id: u64) {
        self.position_epochs.release_owner(owner_id);
        self.native_render_cursors.remove(&owner_id);
    }

    pub(crate) fn native_render_cursor(&self, owner_id: u64) -> Option<NativeRenderCursor> {
        self.native_render_cursors.get(&owner_id).cloned()
    }

    pub(crate) fn retain_native_render_cursor(
        &mut self,
        owner_id: u64,
        document_revision: u64,
        render_blocks: std::sync::Arc<crate::render::incremental::CachedRenderBlocks>,
    ) {
        self.native_render_cursors.insert(
            owner_id,
            NativeRenderCursor {
                document_revision,
                render_blocks,
            },
        );
    }

    pub(crate) fn native_request_outcome(
        &self,
        owner_id: u64,
        request_id: u64,
    ) -> Result<Option<&str>, SessionError> {
        let Some(ledger) = self.native_request_ledgers.get(&owner_id) else {
            return Ok(None);
        };
        if let Some(outcome) = ledger.recent.get(&request_id) {
            return Ok(Some(outcome));
        }
        if ledger
            .high_water
            .is_some_and(|high_water| request_id <= high_water)
        {
            let mut error = SessionError::new(
                ErrorDomain::Boundary,
                "EXPIRED_NATIVE_REQUEST",
                "native request is older than the retained idempotency window",
            );
            error.request_id = Some(request_id);
            return Err(error);
        }
        Ok(None)
    }

    pub(crate) fn retain_native_request_outcome(
        &mut self,
        owner_id: u64,
        request_id: u64,
        outcome: String,
    ) {
        let ledger = self.native_request_ledgers.entry(owner_id).or_default();
        ledger.high_water = Some(
            ledger
                .high_water
                .map_or(request_id, |value| value.max(request_id)),
        );
        ledger.recent.insert(request_id, outcome);
        ledger.order.push_back(request_id);
        while ledger.order.len() > NATIVE_REQUEST_CACHE_LIMIT {
            if let Some(expired) = ledger.order.pop_front() {
                ledger.recent.remove(&expired);
            }
        }
    }

    pub(crate) fn release_native_binding(&mut self, owner_id: u64) {
        self.position_epochs.release_owner(owner_id);
        self.native_request_ledgers.remove(&owner_id);
        self.native_render_cursors.remove(&owner_id);
    }
}
