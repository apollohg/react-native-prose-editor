impl MutationCompiler {
    pub(crate) fn remaining_scan_work(&self) -> usize {
        self.scan_limit.saturating_sub(self.scan_work)
    }

    pub(crate) fn charge_position_resolver_work(
        &mut self,
        operation_index: usize,
        amount: usize,
    ) -> OperationResult<()> {
        self.charge_scan_work(operation_index, amount)?;
        #[cfg(test)]
        {
            self.position_resolver_work = self
                .position_resolver_work
                .checked_add(amount)
                .expect("successful resolver work is bounded by scan_limit");
        }
        Ok(())
    }

    fn push_action(&mut self, action: YrsMutationAction) -> usize {
        let slot = self.actions.len();
        self.actions.push(ActionSlot::concrete(action));
        slot
    }

    fn queue_prepared_insert(&mut self, insert: PendingPreparedInsert) -> usize {
        let insert_id = self.prepared_inserts.len();
        self.prepared_inserts.push(Some(insert));
        self.actions.push(ActionSlot::PreparedInsert(insert_id));
        insert_id
    }

    fn prepared_node_mut(
        &mut self,
        handle: &PreparedHandle,
        operation_index: usize,
    ) -> OperationResult<&mut PreparedXmlNode> {
        let insert = self
            .prepared_inserts
            .get_mut(handle.insert_id)
            .and_then(Option::as_mut)
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "prepared insertion handle has no owned blueprint",
                )
            })?;
        let Some((&root, descendants)) = handle.ordinal_path.split_first() else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        let mut node = &mut insert
            .nodes
            .get_mut(root)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
            .node;
        for &ordinal in descendants {
            let PreparedXmlNode::Element { children, .. } = node else {
                return Err(invalid_action_range(self.request_id, operation_index));
            };
            node = &mut children
                .get_mut(ordinal)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
                .node;
        }
        Ok(node)
    }

    fn replace_prepared_element_with_children(
        &mut self,
        operation_index: usize,
        handle: &PreparedHandle,
        replacement: Vec<PreparedXmlChild>,
    ) -> OperationResult<()> {
        let insert = self
            .prepared_inserts
            .get_mut(handle.insert_id)
            .and_then(Option::as_mut)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let Some((&root_ordinal, descendants)) = handle.ordinal_path.split_first() else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        if descendants.is_empty() {
            if root_ordinal >= insert.nodes.len() {
                return Err(invalid_action_range(self.request_id, operation_index));
            }
            insert
                .nodes
                .splice(root_ordinal..=root_ordinal, replacement);
            for (ordinal, child) in insert.nodes.iter_mut().enumerate() {
                child.index = insert
                    .child_index
                    .checked_add(
                        u32::try_from(ordinal)
                            .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                    )
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            return Ok(());
        }
        let mut node = &mut insert
            .nodes
            .get_mut(root_ordinal)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
            .node;
        for ordinal in &descendants[..descendants.len() - 1] {
            let PreparedXmlNode::Element { children, .. } = node else {
                return Err(invalid_action_range(self.request_id, operation_index));
            };
            node = &mut children
                .get_mut(*ordinal)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
                .node;
        }
        let PreparedXmlNode::Element { children, .. } = node else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        let ordinal = descendants[descendants.len() - 1];
        if ordinal >= children.len() {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        children.splice(ordinal..=ordinal, replacement);
        for (ordinal, child) in children.iter_mut().enumerate() {
            child.index = u32::try_from(ordinal)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        }
        Ok(())
    }

    pub(crate) fn new<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
    ) -> OperationResult<Self> {
        let document_guard = capture_document_guard(request_id, txn)?;
        Self::new_eager_guarded(
            request_id,
            txn,
            fragment,
            schema,
            action_limit,
            scan_limit,
            scan_work,
            document_guard,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_eager_guarded<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
        document_guard: DocumentGuard,
    ) -> OperationResult<Self> {
        let mut located = Vec::new();
        let mut materialized_texts = HashMap::new();
        let mut traversal_work = 0usize;
        let text_target_context = TextTargetContext {
            request_id,
            txn,
            schema,
        };
        #[cfg(test)]
        if !SUPPRESS_RANGE_FORMAT_LOWERING_COUNTS.get() {
            EAGER_RANGE_TEXT_COLLECTION_COUNT
                .set(EAGER_RANGE_TEXT_COLLECTION_COUNT.get().saturating_add(1));
        }
        collect_text_targets(
            &text_target_context,
            (0u32..).zip(fragment.children(txn)),
            TextTargetParent {
                id: <XmlFragmentRef as AsRef<Branch>>::as_ref(fragment).id(),
                ancestors: &[],
            },
            0,
            &mut traversal_work,
            &mut materialized_texts,
            &mut located,
        )?;
        let mut structural_parents = HashMap::new();
        #[cfg(test)]
        if !SUPPRESS_RANGE_FORMAT_LOWERING_COUNTS.get() {
            EAGER_RANGE_PARENT_COLLECTION_COUNT
                .set(EAGER_RANGE_PARENT_COLLECTION_COUNT.get().saturating_add(1));
        }
        collect_structural_parents(
            request_id,
            txn,
            XmlParentRef::Fragment(fragment.clone()),
            Vec::new(),
            Vec::new(),
            schema,
            &mut traversal_work,
            &materialized_texts,
            &mut structural_parents,
        )?;
        let mut targets = Vec::with_capacity(located.len());
        let text_runs = structural_parents
            .values()
            .flat_map(|parent| parent.storage_children.iter())
            .filter_map(|child| match child {
                StorageChildKind::Text { target, runs, .. } => {
                    Some((AsRef::<Branch>::as_ref(target).id(), runs.clone()))
                }
                StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => None,
            })
            .collect::<HashMap<_, _>>();
        let mut previous_end = 0u32;
        for located in located {
            let (start, scalar_len) = match &located {
                LocatedTarget::Existing {
                    start, scalar_len, ..
                } => (*start, *scalar_len),
                LocatedTarget::Missing { start, .. } => (*start, 0),
            };
            let gap_before = start.checked_sub(previous_end).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs text targets overlap in document order",
                )
            })?;
            previous_end = start.checked_add(scalar_len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs text target position overflow",
                )
            })?;
            let target = match located {
                LocatedTarget::Existing {
                    target,
                    text,
                    signature,
                    ..
                } => ResolvedText {
                    base_runs: text_runs
                        .get(&AsRef::<Branch>::as_ref(&target).id())
                        .cloned()
                        .unwrap_or_else(|| {
                            vec![PreparedTextRun {
                                index_utf16: 0,
                                text: text.clone(),
                                attrs: Attrs::default(),
                            }]
                        }),
                    current_runs: text_runs
                        .get(&AsRef::<Branch>::as_ref(&target).id())
                        .cloned()
                        .unwrap_or_else(|| {
                            vec![PreparedTextRun {
                                index_utf16: 0,
                                text: text.clone(),
                                attrs: Attrs::default(),
                            }]
                        }),
                    action_slots: Vec::new(),
                    kind: ResolvedTargetKind::Existing { signature, target },
                    gap_before,
                    text,
                    scalar_len,
                },
                LocatedTarget::Missing {
                    parent,
                    child_index,
                    signature,
                    ..
                } => ResolvedText {
                    base_runs: Vec::new(),
                    current_runs: Vec::new(),
                    action_slots: Vec::new(),
                    kind: ResolvedTargetKind::Missing {
                        signature,
                        parent,
                        child_index,
                        create_action: None,
                    },
                    gap_before,
                    text: String::new(),
                    scalar_len: 0,
                },
            };
            targets.push(target);
        }
        Ok(Self {
            request_id,
            document_guard,
            targets,
            structural_parents,
            actions: Vec::new(),
            prepared_inserts: Vec::new(),
            prepared_elements: HashMap::new(),
            charged_work: 0,
            pending_traversal_work: traversal_work,
            localized_position_target_count: None,
            explicit_path_parent_widths: None,
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
        })
    }

    /// Builds the existing eager compiler first, then replaces only its text
    /// target index with an independently resolved single-block target when
    /// that resolution is unambiguous. This is deliberately not wired into
    /// production yet: the eager result preserves today's guard and logical
    /// work accounting and is the ready fallback for every cold path.
    #[allow(clippy::too_many_arguments, dead_code)]
    pub(crate) fn new_localized_insert_or_eager<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
        locator: LocalizedInsertLocator<'_>,
    ) -> OperationResult<(Self, MutationCompilerBuild)> {
        let eager = Self::new(
            request_id,
            txn,
            fragment,
            schema,
            action_limit,
            scan_limit,
            scan_work,
        )?;
        let Some(LocalizedTextblockTargets {
            targets,
            path_parent_widths,
        }) = localized_existing_textblock_targets(
            request_id,
            txn,
            fragment,
            schema,
            LocalizedTextblockLocator::Insert(locator),
        )?
        else {
            return Ok((eager, MutationCompilerBuild::EagerFallback));
        };

        let localized = Self {
            request_id,
            document_guard: eager.document_guard.clone(),
            targets,
            structural_parents: HashMap::new(),
            actions: Vec::new(),
            prepared_inserts: Vec::new(),
            prepared_elements: HashMap::new(),
            charged_work: 0,
            pending_traversal_work: eager.pending_traversal_work,
            localized_position_target_count: Some(eager.targets.len()),
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
        };
        Ok((localized, MutationCompilerBuild::Localized))
    }
}
