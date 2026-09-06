impl LocalizedRootWindowCompiler {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
        locator: LocalizedRootWindowLocator<'_>,
    ) -> OperationResult<Option<Self>> {
        if !locator.seed.matches_storage(
            txn,
            fragment,
            locator.seed.binding.schema_fingerprint.as_ref(),
            locator.seed.binding.yrs_state_epoch,
            locator.seed.binding.document_revision,
        ) {
            return Ok(None);
        }
        let Some(seed_payload) = locator.seed.ready_payload() else {
            return Ok(None);
        };
        let document_guard = capture_document_guard(request_id, txn)?;
        let Some(root) = localized_root_structural_parent(
            request_id,
            txn,
            fragment,
            locator.document,
            schema,
            &locator.seed.binding.resource_limits,
        )?
        else {
            return Ok(None);
        };
        if seed_payload
            .path_parent_widths
            .get(&root.signature.parent)
            .copied()
            != Some(root.signature.children.len())
        {
            return Ok(None);
        }
        let root_parent = root.signature.parent.clone();
        let root_child_count = root.signature.children.len();
        let mut structural_parents = HashMap::new();
        structural_parents.try_reserve(1).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized root parent map allocation failed",
            )
        })?;
        structural_parents.insert(Vec::new(), root);
        let mut root_width = HashMap::new();
        root_width.try_reserve(1).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized root width map allocation failed",
            )
        })?;
        root_width.insert(root_parent, root_child_count);
        #[cfg(test)]
        LOCALIZED_ROOT_WINDOW_HIT_COUNT
            .set(LOCALIZED_ROOT_WINDOW_HIT_COUNT.get().saturating_add(1));
        Ok(Some(Self {
            compiler: MutationCompiler {
                request_id,
                document_guard,
                targets: Vec::new(),
                structural_parents,
                actions: Vec::new(),
                prepared_inserts: Vec::new(),
                prepared_elements: HashMap::new(),
                charged_work: 0,
                pending_traversal_work: seed_payload.pending_traversal_work,
                localized_position_target_count: Some(seed_payload.target_count),
                explicit_path_parent_widths: Some(root_width),
                action_limit,
                scan_work,
                scan_limit,
                #[cfg(test)]
                position_resolver_work: 0,
                created_gap_shifts: HashMap::new(),
                pending_element_attrs: HashMap::new(),
                wrap_checkpoints: HashMap::new(),
                #[cfg(test)]
                virtual_delete_visits: 0,
            },
            document: locator.document.clone(),
            expected_preview: locator.expected_preview.clone(),
            from_child: locator.from_child,
            to_child: locator.to_child,
            expected_content: locator.expected_content,
        }))
    }

    pub(crate) fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        self.compiler.charge_boundary_node(operation_index)
    }

    pub(crate) fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        self.compiler
            .charge_boundary_text(operation_index, text_bytes)
    }

    pub(crate) fn replace_structural_range(
        mut self,
        operation_index: usize,
        context: MutationDocumentContext<'_>,
        replacement: ReplacementInput<'_>,
    ) -> OperationResult<YrsMutationPlan> {
        let root = self.document.root().content().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized root-window source has no root content",
            )
        })?;
        let child_offset = |end: u32| {
            root.iter()
                .take(usize::try_from(end).ok()?)
                .try_fold(0u32, |offset, child| offset.checked_add(child.node_size()))
        };
        if replacement.content != &self.expected_content {
            return Err(OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized root-window replacement content is stale",
            ));
        }
        if !context.before.shares_root_storage_with(&self.document)
            || context.after != &self.expected_preview
            || self.from_child >= self.to_child
            || child_offset(self.from_child) != Some(replacement.from)
            || child_offset(self.to_child) != Some(replacement.to)
        {
            return Err(OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized root-window candidate is stale",
            ));
        }
        self.compiler
            .replace_structural_range(operation_index, context, replacement)?;
        self.compiler.finish(Some(operation_index))
    }
}

impl LocalizedInsertCompiler {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
        locator: LocalizedInsertLocator<'_>,
        seed: &MutationLookupSeed,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Option<Self>> {
        if !seed.matches(
            txn,
            fragment,
            locator.document,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        ) {
            return Ok(None);
        }
        let Some(seed_payload) = seed.ready_payload() else {
            return Ok(None);
        };
        let document_guard = capture_document_guard(request_id, txn)?;
        let Some(LocalizedTextblockTargets {
            targets,
            path_parent_widths,
            ..
        }) = localized_existing_textblock_targets(
            request_id,
            txn,
            fragment,
            schema,
            LocalizedTextblockLocator::Insert(locator),
        )?
        else {
            return Ok(None);
        };
        if path_parent_widths.iter().any(|(parent, width)| {
            seed_payload.path_parent_widths.get(parent).copied() != Some(*width)
        }) || targets.iter().any(|target| match &target.kind {
            ResolvedTargetKind::Existing { signature, .. } => {
                seed_payload
                    .target_materialization_work
                    .get(&signature.target)
                    .copied()
                    != Some(signature.capture_work)
            }
            ResolvedTargetKind::Missing { .. } | ResolvedTargetKind::Prepared { .. } => true,
        }) {
            return Ok(None);
        }
        #[cfg(test)]
        LOCALIZED_INSERT_HIT_COUNT.set(LOCALIZED_INSERT_HIT_COUNT.get().saturating_add(1));
        Ok(Some(Self {
            compiler: MutationCompiler {
                request_id,
                document_guard,
                targets,
                structural_parents: HashMap::new(),
                actions: Vec::new(),
                prepared_inserts: Vec::new(),
                prepared_elements: HashMap::new(),
                charged_work: 0,
                pending_traversal_work: seed_payload.pending_traversal_work,
                localized_position_target_count: Some(seed_payload.target_count),
                explicit_path_parent_widths: Some(path_parent_widths),
                action_limit,
                scan_work,
                scan_limit,
                #[cfg(test)]
                position_resolver_work: 0,
                created_gap_shifts: HashMap::new(),
                pending_element_attrs: HashMap::new(),
                wrap_checkpoints: HashMap::new(),
                #[cfg(test)]
                virtual_delete_visits: 0,
            },
        }))
    }

    #[cfg(test)]
    pub(crate) fn compile(
        self,
        operation_index: usize,
        position: u32,
        text: &str,
        marks: &[Mark],
    ) -> OperationResult<YrsMutationPlan> {
        self.compile_with_promotion(operation_index, position, text, marks)
            .map(|(plan, _)| plan)
    }

    pub(crate) fn compile_with_promotion(
        mut self,
        operation_index: usize,
        position: u32,
        text: &str,
        marks: &[Mark],
    ) -> OperationResult<(YrsMutationPlan, MutationLookupPromotion)> {
        let base_pending_traversal_work = self.compiler.pending_traversal_work;
        self.compiler
            .insert(operation_index, position, text, marks)?;
        let (target_id, previous_materialization_work) = self
            .compiler
            .actions
            .iter()
            .find_map(|slot| match slot {
                ActionSlot::Concrete(action) => match action.as_ref() {
                    YrsMutationAction::InsertText {
                        target: _,
                        signature,
                        ..
                    } => Some((signature.target.clone(), signature.capture_work)),
                    _ => None,
                },
                ActionSlot::PreparedInsert(_) | ActionSlot::Tombstone => None,
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized insert produced no existing-text action",
                )
            })?;
        let next_materialization_work = self
            .compiler
            .targets
            .iter()
            .find_map(|resolved| match &resolved.kind {
                ResolvedTargetKind::Existing { signature, .. } if signature.target == target_id => {
                    Some(prepared_materialization_work(
                        self.compiler.request_id,
                        &resolved.current_runs,
                    ))
                }
                _ => None,
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized insert promotion target is absent from the sealed compiler",
                )
            })??;
        let next_pending_traversal_work = base_pending_traversal_work
            .checked_sub(previous_materialization_work)
            .and_then(|work| work.checked_add(next_materialization_work))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized insert promotion work overflow",
                )
            })?;
        if self
            .compiler
            .targets
            .iter()
            .find_map(|resolved| match &resolved.kind {
                ResolvedTargetKind::Existing { signature, .. } if signature.target == target_id => {
                    Some(signature.capture_work)
                }
                _ => None,
            })
            != Some(previous_materialization_work)
        {
            return Err(OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized insert promotion signature is inconsistent",
            ));
        }
        let mut materialization_work_updates = Vec::new();
        materialization_work_updates.try_reserve(1).map_err(|_| {
            OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized insert promotion allocation failed",
            )
        })?;
        materialization_work_updates.push((
            target_id,
            previous_materialization_work,
            next_materialization_work,
        ));
        let promotion = MutationLookupPromotion {
            request_id: self.compiler.request_id,
            source: MutationLookupPromotionSource::ExistingInsert,
            materialization_work_updates,
            next_pending_traversal_work,
        };
        self.compiler
            .finish(Some(operation_index))
            .map(|plan| (plan, promotion))
    }
}
