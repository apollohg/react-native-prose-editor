impl MutationLookupSeed {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_import_materialization<T: ReadTxn>(
        request_id: u64,
        materialization: ImportLookupMaterialization,
        txn: &T,
        fragment: &XmlFragmentRef,
        source_document: Document,
        canonical_artifact: CanonicalArtifact,
        resource_limits: ResourceLimits,
        editing_limits: EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Self> {
        probe_lookup_seed_publication(
            request_id,
            "bindingPublication",
            std::mem::size_of::<MutationLookupBinding>(),
        )?;
        probe_lookup_seed_publication(request_id, "bindingPublication", schema_fingerprint.len())?;
        Ok(Self {
            binding: MutationLookupBinding {
                source_document,
                canonical_artifact: Some(canonical_artifact),
                resource_limits,
                editing_limits,
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Ready(materialization.0),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn unavailable_for_validated_import<T: ReadTxn>(
        txn: &T,
        fragment: &XmlFragmentRef,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> Self {
        Self::unavailable(
            txn,
            fragment,
            source_document,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
    }

    pub(crate) fn prepare_history_store_snapshot<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        snapshot_scan_reservation: usize,
    ) -> OperationResult<HistoryStoreSnapshotEvidence> {
        // Yrs Snapshot construction owns proportional StateVector/IdSet maps
        // through an infallible upstream API. Apply the established admitted
        // CRDT clock-scan policy immediately before that unavoidable
        // allocation. Probe only the subsequent fixed Arc allocation.
        let admitted_clock_scan_work =
            crdt_clock_scan_reservation(request_id, txn, snapshot_scan_reservation)?;
        let snapshot = txn.snapshot();
        probe_lookup_seed_publication_for_stage(
            request_id,
            "bindingPublication",
            "historyStoreSnapshotPublication",
            std::mem::size_of::<yrs::Snapshot>(),
        )?;
        Ok(HistoryStoreSnapshotEvidence {
            snapshot: Arc::new(snapshot),
            admitted_clock_scan_work,
        })
    }

    pub(crate) fn from_admitted_history_proof(
        proof: super::super::derived_state::AdmittedHistoryMutationLookupProof,
    ) -> Self {
        let (
            source_document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            store_token,
            fragment_id,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
            history_store_snapshot,
        ) = proof.into_seed_parts();
        Self {
            binding: MutationLookupBinding {
                source_document,
                canonical_artifact: Some(canonical_artifact),
                resource_limits,
                editing_limits,
                max_length,
                store_token,
                fragment_id,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: Some(history_store_snapshot),
            },
            state: MutationLookupSeedState::Unavailable,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn unavailable<T: ReadTxn>(
        txn: &T,
        fragment: &XmlFragmentRef,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> Self {
        Self {
            binding: MutationLookupBinding {
                source_document: source_document.clone(),
                canonical_artifact: None,
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Unavailable,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Self> {
        Self::build_with_capacity_hint(
            request_id,
            txn,
            fragment,
            schema,
            source_document,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn hydrate_with_target_capacity_hint<T: ReadTxn>(
        &self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
        target_capacity_hint: usize,
    ) -> OperationResult<Self> {
        #[cfg(test)]
        LOOKUP_SEED_BUILD_COUNT.set(LOOKUP_SEED_BUILD_COUNT.get().saturating_add(1));
        let payload = build_lookup_seed_payload(
            request_id,
            txn,
            fragment,
            schema,
            Some(target_capacity_hint),
        )?;
        probe_lookup_seed_publication(
            request_id,
            "bindingPublication",
            std::mem::size_of::<MutationLookupBinding>(),
        )?;
        let schema_fingerprint = if self.binding.schema_fingerprint.as_ref() == schema_fingerprint {
            Arc::clone(&self.binding.schema_fingerprint)
        } else {
            // Arc::try_new/try_from are not stable. Reserve the complete
            // proportional payload fallibly, then apply the crate's Arc
            // publication-probe policy before the unavoidable Arc::from.
            probe_lookup_seed_publication(
                request_id,
                "bindingPublication",
                schema_fingerprint.len(),
            )?;
            Arc::from(schema_fingerprint)
        };
        Ok(Self {
            binding: MutationLookupBinding {
                source_document: source_document.clone(),
                canonical_artifact: self.binding.canonical_artifact.clone(),
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Ready(payload),
        })
    }

    pub(crate) fn try_publish_hydrated(self, request_id: u64) -> OperationResult<Arc<Self>> {
        probe_lookup_seed_publication(request_id, "seedPublication", std::mem::size_of::<Self>())?;
        Ok(Arc::new(self))
    }

    pub(crate) fn try_publish_history_unavailable(
        mut self,
        request_id: u64,
    ) -> OperationResult<Arc<Self>> {
        if !matches!(&self.state, MutationLookupSeedState::Unavailable)
            || self.binding.history_store_snapshot.is_none()
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history unavailable mutation lookup publication capability is invalid",
            ));
        }
        // Installed general seeds are Clone for normal lookup lifecycle use.
        // Strip the one-shot store seal before exposing that general type so a
        // clone cannot replay candidate publication.
        self.binding.history_store_snapshot = None;
        probe_lookup_seed_publication_for_stage(
            request_id,
            "seedPublication",
            "historyUnavailableSeedPublication",
            std::mem::size_of::<Self>(),
        )?;
        Ok(Arc::new(self))
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(dead_code)] // Candidate publication is consumed by the next candidate-swap slice.
    pub(crate) fn prepare_candidate_publication<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        source_document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Arc<Self>> {
        let claims_are_exact = matches!(&self.state, MutationLookupSeedState::Unavailable)
            && self
                .binding
                .canonical_artifact
                .as_ref()
                .is_some_and(|sealed| {
                    sealed.ptr_eq(canonical_artifact)
                        && sealed.matches_exact_source_document(source_document)
                })
            && canonical_artifact.matches_exact_source_document(source_document)
            && canonical_artifact.schema_fingerprint() == schema_fingerprint
            && canonical_artifact.format_version()
                == super::super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            && crate::schema::schema_fingerprint(schema) == schema_fingerprint
            && self.binding_matches_context(
                source_document,
                resource_limits,
                editing_limits,
                max_length,
            )
            && self.binding_matches_storage(
                txn,
                fragment,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            );
        if !claims_are_exact {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history candidate mutation lookup evidence is stale or contradictory",
            ));
        }
        // Repeat the same ceiling admission before reconstructing the exact
        // state-vector/delete-set seal for validation. Keep this outside the
        // boolean above so allocation-limit errors retain their own precedence
        // and no proportional snapshot is hidden behind a predicate.
        let admitted_clock_scan_work =
            crdt_clock_scan_reservation(request_id, txn, resource_limits.max_encoded_state_bytes)?;
        let current_snapshot = txn.snapshot();
        let store_state_is_exact =
            self.binding
                .history_store_snapshot
                .as_ref()
                .is_some_and(|sealed| {
                    sealed.admitted_clock_scan_work == admitted_clock_scan_work
                        && sealed.snapshot.as_ref() == &current_snapshot
                });
        if !store_state_is_exact {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history candidate mutation lookup evidence is stale or contradictory",
            ));
        }
        #[cfg(test)]
        LOOKUP_SEED_BUILD_COUNT.set(LOOKUP_SEED_BUILD_COUNT.get().saturating_add(1));
        let payload = build_lookup_seed_payload(request_id, txn, fragment, schema, None)?;
        probe_lookup_seed_publication_for_stage(
            request_id,
            "bindingPublication",
            "candidateBindingPublication",
            std::mem::size_of::<MutationLookupBinding>(),
        )?;
        probe_lookup_seed_publication_for_stage(
            request_id,
            "bindingPublication",
            "candidateBindingPublication",
            schema_fingerprint.len(),
        )?;
        let seed = Self {
            binding: MutationLookupBinding {
                source_document: source_document.clone(),
                canonical_artifact: Some(canonical_artifact.clone()),
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Ready(payload),
        };
        probe_lookup_seed_publication_for_stage(
            request_id,
            "seedPublication",
            "candidateSeedPublication",
            std::mem::size_of::<Self>(),
        )?;
        let published = Arc::new(seed);
        #[cfg(test)]
        super::super::observability::record_staged_seed_preparation();
        Ok(published)
    }

    #[allow(clippy::too_many_arguments)]
    fn build_with_capacity_hint<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
        target_capacity_hint: Option<usize>,
    ) -> OperationResult<Self> {
        #[cfg(test)]
        LOOKUP_SEED_BUILD_COUNT.set(LOOKUP_SEED_BUILD_COUNT.get().saturating_add(1));
        let payload =
            build_lookup_seed_payload(request_id, txn, fragment, schema, target_capacity_hint)?;
        Ok(Self {
            binding: MutationLookupBinding {
                source_document: source_document.clone(),
                canonical_artifact: None,
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Ready(payload),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn matches<T: ReadTxn>(
        &self,
        txn: &T,
        fragment: &XmlFragmentRef,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> bool {
        self.ready_payload().is_some()
            && self.binding_matches_context(
                source_document,
                resource_limits,
                editing_limits,
                max_length,
            )
            && self.binding_matches_storage(
                txn,
                fragment,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
    }

    pub(crate) fn with_canonical_artifact(mut self, artifact: &CanonicalArtifact) -> Self {
        self.binding.canonical_artifact = Some(artifact.clone());
        self
    }

    pub(crate) fn matches_canonical_artifact(&self, artifact: &CanonicalArtifact) -> bool {
        self.binding
            .canonical_artifact
            .as_ref()
            .is_some_and(|sealed| sealed.ptr_eq(artifact))
    }

    #[allow(dead_code)]
    pub(crate) fn matches_context(
        &self,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
    ) -> bool {
        self.ready_payload().is_some()
            && self.binding_matches_context(
                source_document,
                resource_limits,
                editing_limits,
                max_length,
            )
    }

    fn binding_matches_context(
        &self,
        source_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
    ) -> bool {
        self.binding
            .source_document
            .shares_root_storage_with(source_document)
            && self.binding.resource_limits == *resource_limits
            && self.binding.editing_limits == *editing_limits
            && self.binding.max_length == max_length
    }

    fn matches_storage<T: ReadTxn>(
        &self,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> bool {
        self.ready_payload().is_some()
            && self.binding_matches_storage(
                txn,
                fragment,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
    }

    fn binding_matches_storage<T: ReadTxn>(
        &self,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> bool {
        self.binding.store_token == txn.store() as *const _ as usize
            && self.binding.fragment_id == AsRef::<Branch>::as_ref(fragment).id()
            && self.binding.schema_fingerprint.as_ref() == schema_fingerprint
            && self.binding.yrs_state_epoch == yrs_state_epoch
            && self.binding.document_revision == document_revision
    }

    fn ready_payload(&self) -> Option<&MutationLookupPayload> {
        match &self.state {
            MutationLookupSeedState::Ready(payload) => Some(payload),
            MutationLookupSeedState::Unavailable => None,
        }
    }

    pub(crate) fn is_unavailable(&self) -> bool {
        matches!(self.state, MutationLookupSeedState::Unavailable)
    }

    #[cfg(test)]
    pub(crate) fn is_ready_for_test(&self) -> bool {
        self.ready_payload().is_some()
    }

    #[cfg(test)]
    pub(crate) fn has_same_ready_payload_for_test(&self, other: &Self) -> bool {
        match (self.ready_payload(), other.ready_payload()) {
            (Some(left), Some(right)) => {
                left.target_count == right.target_count
                    && left.pending_traversal_work == right.pending_traversal_work
                    && left.path_parent_widths == right.path_parent_widths
                    && left.target_materialization_work == right.target_materialization_work
            }
            _ => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_unavailable_for_test(&self) -> bool {
        self.is_unavailable()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_promotion<T: ReadTxn>(
        &self,
        txn: &T,
        fragment: &XmlFragmentRef,
        promotion: &MutationLookupPromotion,
        current_document: &Document,
        next_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        current_yrs_state_epoch: u64,
        current_document_revision: u64,
        next_yrs_state_epoch: u64,
        next_document_revision: u64,
    ) -> OperationResult<Self> {
        let Some(payload) = self.ready_payload() else {
            return Err(OperationError::engine_invariant_failed(
                promotion.request_id,
                None,
                "localized mutation lookup promotion seed is unavailable",
            ));
        };
        if !self.matches(
            txn,
            fragment,
            current_document,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            current_yrs_state_epoch,
            current_document_revision,
        ) {
            return Err(OperationError::engine_invariant_failed(
                promotion.request_id,
                None,
                "localized mutation lookup promotion seed is stale",
            ));
        }
        let promotion_shape_matches = match promotion.source {
            MutationLookupPromotionSource::ExistingInsert => {
                promotion.materialization_work_updates.len() == 1
            }
            MutationLookupPromotionSource::ExistingFormat => {
                !promotion.materialization_work_updates.is_empty()
            }
        };
        if !promotion_shape_matches {
            return Err(OperationError::engine_invariant_failed(
                promotion.request_id,
                None,
                "localized mutation lookup promotion has an invalid source shape",
            ));
        }
        let mut target_materialization_work = HashMap::new();
        target_materialization_work
            .try_reserve(payload.target_materialization_work.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    promotion.request_id,
                    None,
                    "localized mutation lookup promotion allocation failed",
                )
            })?;
        target_materialization_work.extend(
            payload
                .target_materialization_work
                .iter()
                .map(|(target, work)| (target.clone(), *work)),
        );
        for (target_id, old_work, new_work) in &promotion.materialization_work_updates {
            if target_materialization_work.get(target_id).copied() != Some(*old_work) {
                return Err(OperationError::engine_invariant_failed(
                    promotion.request_id,
                    None,
                    "localized mutation lookup promotion does not match its seed",
                ));
            }
            target_materialization_work.insert(target_id.clone(), *new_work);
        }
        #[cfg(test)]
        if promotion.source == MutationLookupPromotionSource::ExistingInsert {
            LOOKUP_SEED_PROMOTION_COUNT.set(LOOKUP_SEED_PROMOTION_COUNT.get().saturating_add(1));
        }
        Ok(Self {
            binding: MutationLookupBinding {
                source_document: next_document.clone(),
                canonical_artifact: self.binding.canonical_artifact.clone(),
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch: next_yrs_state_epoch,
                document_revision: next_document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Ready(MutationLookupPayload {
                target_count: payload.target_count,
                pending_traversal_work: promotion.next_pending_traversal_work,
                path_parent_widths: payload.path_parent_widths.clone(),
                target_materialization_work: Arc::new(target_materialization_work),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_unavailable_transition<T: ReadTxn>(
        &self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        current_document: &Document,
        next_document: &Document,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        current_yrs_state_epoch: u64,
        current_document_revision: u64,
        next_yrs_state_epoch: u64,
        next_document_revision: u64,
    ) -> OperationResult<Self> {
        if self.ready_payload().is_none()
            || !self.matches(
                txn,
                fragment,
                current_document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                current_yrs_state_epoch,
                current_document_revision,
            )
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized mutation lookup invalidation seed is stale or unavailable",
            ));
        }
        Ok(Self {
            binding: MutationLookupBinding {
                source_document: next_document.clone(),
                canonical_artifact: None,
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch: next_yrs_state_epoch,
                document_revision: next_document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Unavailable,
        })
    }

    pub(crate) fn rebind_authoritative_store<T: ReadTxn>(
        &self,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> Self {
        Self {
            binding: MutationLookupBinding {
                source_document: self.binding.source_document.clone(),
                canonical_artifact: self.binding.canonical_artifact.clone(),
                resource_limits: self.binding.resource_limits.clone(),
                editing_limits: self.binding.editing_limits.clone(),
                max_length: self.binding.max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: self.state.clone(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_authoritative_store_rebind<C: ReadTxn, L: ReadTxn>(
        &self,
        request_id: u64,
        candidate_txn: &C,
        candidate_fragment: &XmlFragmentRef,
        source_document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
        live_txn: &L,
        live_fragment: &XmlFragmentRef,
    ) -> OperationResult<Arc<Self>> {
        if !self.matches_canonical_artifact(canonical_artifact)
            || !self.matches(
                candidate_txn,
                candidate_fragment,
                source_document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
            || AsRef::<Branch>::as_ref(live_fragment).id() != self.binding.fragment_id
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "authoritative-store mutation lookup rebind source is stale or foreign",
            ));
        }
        let payload = self.ready_payload().expect("matching seed is ready");
        probe_lookup_seed_publication_for_stage(
            request_id,
            "bindingPublication",
            "authoritativeStoreBindingPublication",
            std::mem::size_of::<MutationLookupBinding>(),
        )?;
        let schema_fingerprint = if self.binding.schema_fingerprint.as_ref() == schema_fingerprint {
            Arc::clone(&self.binding.schema_fingerprint)
        } else {
            probe_lookup_seed_publication_for_stage(
                request_id,
                "bindingPublication",
                "authoritativeStoreBindingPublication",
                schema_fingerprint.len(),
            )?;
            Arc::from(schema_fingerprint)
        };
        let rebound = Self {
            binding: MutationLookupBinding {
                source_document: self.binding.source_document.clone(),
                canonical_artifact: self.binding.canonical_artifact.clone(),
                resource_limits: self.binding.resource_limits.clone(),
                editing_limits: self.binding.editing_limits.clone(),
                max_length: self.binding.max_length,
                store_token: live_txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(live_fragment).id(),
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
                history_store_snapshot: None,
            },
            state: MutationLookupSeedState::Ready(payload.clone()),
        };
        probe_lookup_seed_publication_for_stage(
            request_id,
            "seedPublication",
            "authoritativeStoreSeedPublication",
            std::mem::size_of::<Self>(),
        )?;
        Ok(Arc::new(rebound))
    }
}
