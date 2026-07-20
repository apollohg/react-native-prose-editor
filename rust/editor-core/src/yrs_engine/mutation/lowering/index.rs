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

    pub(crate) fn delete_structural_range(
        &mut self,
        operation_index: usize,
        document: &Document,
        from: u32,
        to: u32,
    ) -> OperationResult<()> {
        let from_resolved = document.resolve(from).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "range", message)
        })?;
        let to_resolved = document.resolve(to).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "range", message)
        })?;
        if from_resolved.node_path != to_resolved.node_path {
            return self.delete_cross_parent_structural_range(
                operation_index,
                document,
                &from_resolved,
                &to_resolved,
            );
        }
        let parent = from_resolved.parent(document);
        let content = parent.content().ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "structural deletion parent has no content",
            )
        })?;
        let path = from_resolved.node_path.iter().copied().collect::<Vec<_>>();
        let target = self.structural_parents.get(&path).cloned().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "semantic structural parent has no tracked Yrs branch",
            )
        })?;
        let (child_index, child_count) = exact_storage_child_span(
            content.iter(),
            &target.storage_children,
            from_resolved.parent_offset,
            to_resolved.parent_offset,
        )
        .ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "structural deletion must cover complete XML children",
            )
        })?;
        self.charge_operation_work(
            operation_index,
            usize::try_from(child_count)
                .ok()
                .and_then(|count| count.checked_add(target.signature.children.len()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        self.push_action(YrsMutationAction::DeleteXmlChildren {
            parent: target.parent,
            child_index,
            child_count,
            signature: target.signature,
            operation_index,
        });
        Ok(())
    }

    fn delete_cross_parent_structural_range(
        &mut self,
        operation_index: usize,
        document: &Document,
        from_resolved: &crate::model::ResolvedPos,
        to_resolved: &crate::model::ResolvedPos,
    ) -> OperationResult<()> {
        let common_depth = from_resolved
            .node_path
            .iter()
            .zip(to_resolved.node_path.iter())
            .take_while(|(left, right)| left == right)
            .count();
        if from_resolved.node_path.len() != common_depth + 1
            || to_resolved.node_path.len() != common_depth + 1
        {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "cross-parent structural deletion endpoints must share a tracked direct parent",
            ));
        }
        let common_path = from_resolved.node_path[..common_depth].to_vec();
        let first_child = from_resolved.node_path[common_depth];
        let last_child = to_resolved.node_path[common_depth];
        if first_child >= last_child {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        let first_path = from_resolved.node_path.to_vec();
        let last_path = to_resolved.node_path.to_vec();
        let first_parent = from_resolved.parent(document);
        let last_parent = to_resolved.parent(document);
        let first_content = first_parent
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let last_content = last_parent
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let first_target = self
            .structural_parents
            .get(&first_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let last_target = self
            .structural_parents
            .get(&last_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let StorageInsertion::InsideText {
            child_index: first_storage_index,
            local_scalar: first_scalar,
            target: first_text,
            ..
        } = self
            .current_storage_insertion(
                first_content.iter(),
                &first_target.storage_children,
                from_resolved.parent_offset,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
        else {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "cross-parent structural deletion must start inside tracked text",
            ));
        };
        let StorageInsertion::InsideText {
            child_index: last_storage_index,
            local_scalar: last_scalar,
            target: last_text,
            ..
        } = self
            .current_storage_insertion(
                last_content.iter(),
                &last_target.storage_children,
                to_resolved.parent_offset,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
        else {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "cross-parent structural deletion must end inside tracked text",
            ));
        };
        let first_storage_len = u32::try_from(first_target.storage_children.len())
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        let last_storage_len = u32::try_from(last_target.storage_children.len())
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        if first_storage_index + 1 != first_storage_len
            || last_storage_index + 1 != last_storage_len
        {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "cross-parent structural deletion with non-text endpoint siblings is not lowered yet",
            ));
        }
        let first_id = AsRef::<Branch>::as_ref(&first_text).id();
        let last_id = AsRef::<Branch>::as_ref(&last_text).id();
        let first_virtual = self
            .targets
            .iter()
            .position(|candidate| {
                matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == first_id
                )
            })
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let last_virtual = self
            .targets
            .iter()
            .position(|candidate| {
                matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == last_id
                )
            })
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if first_virtual >= last_virtual {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        self.charge_scan_work(
            operation_index,
            self.targets[first_virtual]
                .text
                .len()
                .checked_mul(3)
                .and_then(|work| {
                    work.checked_add(self.targets[last_virtual].text.len().saturating_mul(2))
                })
                .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?,
        )?;
        let first_cut_utf16 = prepared_runs_utf16_at_scalar(
            self.request_id,
            operation_index,
            &self.targets[first_virtual].current_runs,
            first_scalar,
        )?;
        let (desired_left, _) =
            split_runs_utf16(&self.targets[first_virtual].current_runs, first_cut_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        self.normalize_existing_target_for_split(
            operation_index,
            first_virtual,
            first_cut_utf16,
            desired_left.clone(),
        )?;

        let last_cut_utf16 = prepared_runs_utf16_at_scalar(
            self.request_id,
            operation_index,
            &self.targets[last_virtual].current_runs,
            last_scalar,
        )?;
        let (_, copied_suffix) =
            split_runs_utf16(&self.targets[last_virtual].current_runs, last_cut_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        for slot in std::mem::take(&mut self.targets[last_virtual].action_slots) {
            self.actions[slot] = ActionSlot::Tombstone;
        }
        let (retained_text, retained_signature) = match &self.targets[first_virtual].kind {
            ResolvedTargetKind::Existing { target, signature } => {
                (target.clone(), signature.clone())
            }
            _ => return Err(invalid_action_range(self.request_id, operation_index)),
        };
        let mut insertion_utf16 = first_cut_utf16;
        for run in &copied_suffix {
            let len_utf16 = u32::try_from(run.text.encode_utf16().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let slot = self.push_action(YrsMutationAction::InsertText {
                target: retained_text.clone(),
                index_utf16: insertion_utf16,
                text: run.text.clone(),
                len_utf16,
                attrs: run.attrs.clone(),
                signature: retained_signature.clone(),
                operation_index,
            });
            self.targets[first_virtual].action_slots.push(slot);
            insertion_utf16 = insertion_utf16
                .checked_add(len_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }

        let lca_target = self
            .structural_parents
            .get(&common_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let delete_index = first_child
            .checked_add(1)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let delete_count = last_child
            .checked_sub(first_child)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        self.push_action(YrsMutationAction::DeleteXmlChildren {
            parent: lca_target.parent,
            child_index: delete_index,
            child_count: delete_count,
            signature: lca_target.signature.clone(),
            operation_index,
        });

        let copied_work = copied_suffix.iter().try_fold(0usize, |work, run| {
            work.checked_add(run.text.len())
                .and_then(|work| work.checked_add(attrs_work(&run.attrs)))
        });
        self.charge_operation_work(
            operation_index,
            first_target
                .signature
                .children
                .len()
                .checked_add(last_target.signature.children.len())
                .and_then(|work| work.checked_add(lca_target.signature.children.len()))
                .and_then(|work| work.checked_add(usize::try_from(delete_count).ok()?))
                .and_then(|work| work.checked_add(copied_work?))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;

        let deleted_ids = self
            .structural_parents
            .iter()
            .filter(|(path, _)| {
                path.len() > common_depth
                    && path[..common_depth] == common_path
                    && path[common_depth] > first_child
                    && path[common_depth] <= last_child
            })
            .flat_map(|(_, parent)| parent.storage_children.iter())
            .filter_map(|child| match child {
                StorageChildKind::Text { target, .. } => Some(AsRef::<Branch>::as_ref(target).id()),
                StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => None,
            })
            .collect::<HashSet<_>>();
        self.targets.retain(|candidate| {
            !matches!(
                &candidate.kind,
                ResolvedTargetKind::Existing { target, .. }
                    if deleted_ids.contains(&AsRef::<Branch>::as_ref(target).id())
            )
        });
        let retained_virtual = self
            .targets
            .iter()
            .position(|candidate| {
                matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == first_id
                )
            })
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let mut merged_runs = desired_left;
        merged_runs.extend(copied_suffix);
        self.targets[retained_virtual].current_runs = normalize_prepared_runs(merged_runs)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        self.targets[retained_virtual].text =
            prepared_runs_text(&self.targets[retained_virtual].current_runs);
        self.targets[retained_virtual].scalar_len =
            u32::try_from(self.targets[retained_virtual].text.chars().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        Ok(())
    }

    fn current_storage_insertion<'a>(
        &self,
        children: impl Iterator<Item = &'a Node>,
        storage_children: &[StorageChildKind],
        position: u32,
    ) -> Option<StorageInsertion> {
        let targets = self
            .targets
            .iter()
            .filter_map(|candidate| match &candidate.kind {
                ResolvedTargetKind::Existing { target, .. } => {
                    Some((AsRef::<Branch>::as_ref(target).id(), candidate))
                }
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let semantic_children = children.collect::<Vec<_>>();
        let mut semantic_index = 0usize;
        let mut semantic_text_offset = 0u32;
        let mut offset = 0u32;
        for (storage_index, storage) in storage_children.iter().enumerate() {
            if offset == position {
                return Some(StorageInsertion::Boundary(
                    u32::try_from(storage_index).ok()?,
                ));
            }
            match storage {
                StorageChildKind::Text {
                    target, signature, ..
                } => {
                    let current = targets.get(&AsRef::<Branch>::as_ref(target).id())?;
                    let start = offset;
                    let mut remaining = current.scalar_len;
                    while remaining > 0 {
                        let child = *semantic_children.get(semantic_index)?;
                        if !child.is_text() || semantic_text_offset >= child.node_size() {
                            return None;
                        }
                        let available = child.node_size() - semantic_text_offset;
                        let consumed = remaining.min(available);
                        offset = offset.checked_add(consumed)?;
                        remaining -= consumed;
                        semantic_text_offset += consumed;
                        if semantic_text_offset == child.node_size() {
                            semantic_index += 1;
                            semantic_text_offset = 0;
                        }
                    }
                    if position > start && position < offset {
                        return Some(StorageInsertion::InsideText {
                            child_index: u32::try_from(storage_index).ok()?,
                            local_scalar: position.checked_sub(start)?,
                            target: target.clone(),
                            signature: signature.clone(),
                            runs: current.current_runs.clone(),
                        });
                    }
                }
                StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => {
                    if semantic_text_offset != 0 {
                        return None;
                    }
                    let child = *semantic_children.get(semantic_index)?;
                    if child.is_text() {
                        return None;
                    }
                    offset = offset.checked_add(child.node_size())?;
                    semantic_index += 1;
                }
            }
        }
        (offset == position).then(|| {
            u32::try_from(storage_children.len())
                .ok()
                .map(StorageInsertion::Boundary)
        })?
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_cross_parent_replacement(
        &mut self,
        operation_index: usize,
        before: &Document,
        after: &Document,
        from: u32,
        from_resolved: &crate::model::ResolvedPos,
        content: &Fragment,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        if content.size() == 0 {
            return Ok(());
        }
        if content.iter().all(Node::is_text) {
            let pieces = inline_text_pieces(self.request_id, operation_index, content)?;
            let mut position = from;
            for (text, marks) in pieces {
                self.insert(operation_index, position, &text, &marks)?;
                position = position
                    .checked_add(checked_scalar_len(
                        self.request_id,
                        Some(operation_index),
                        &text,
                    )?)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            return Ok(());
        }

        let ResolvedInsertion {
            target_index,
            scalar_index,
        } = self.resolve_insertion(operation_index, from)?;
        let retained_id = match &self.targets[target_index].kind {
            ResolvedTargetKind::Existing { target, .. } => AsRef::<Branch>::as_ref(target).id(),
            _ => return Err(invalid_action_range(self.request_id, operation_index)),
        };
        let cut_utf16 = prepared_runs_utf16_at_scalar(
            self.request_id,
            operation_index,
            &self.targets[target_index].current_runs,
            scalar_index,
        )?;
        let (desired_left, _) =
            split_runs_utf16(&self.targets[target_index].current_runs, cut_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        self.normalize_existing_target_for_split(
            operation_index,
            target_index,
            cut_utf16,
            desired_left.clone(),
        )?;
        self.targets[target_index].current_runs = desired_left;
        self.targets[target_index].text =
            prepared_runs_text(&self.targets[target_index].current_runs);
        self.targets[target_index].scalar_len = scalar_index;

        let mut position = from;
        for node in content.iter().take_while(|node| node.is_text()) {
            let text = node
                .text_str()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.insert(operation_index, position, text, node.marks())?;
            position = position
                .checked_add(node.node_size())
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }

        let parent_path = from_resolved.node_path.to_vec();
        let final_parent = after
            .node_at(&parent_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let final_children = final_parent
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let first_structural_semantic = final_children
            .iter()
            .position(|node| !node.is_text())
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let json = final_children
            .iter()
            .skip(first_structural_semantic)
            .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
            .collect::<Vec<_>>();
        let mut batch = prepare_xml_nodes(&json, limits, parent_path.len().saturating_add(2))
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let parent_target = self
            .structural_parents
            .get(&parent_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let retained_storage_index = parent_target
            .storage_children
            .iter()
            .position(|child| {
                matches!(
                    child,
                    StorageChildKind::Text { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == retained_id
                )
            })
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let child_index = retained_storage_index
            .checked_add(1)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        for child in &mut batch.nodes {
            child.index = child_index
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let prepared_work = batch.work;
        let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
            parent: parent_target.parent,
            child_index,
            nodes: batch.nodes,
            signature: parent_target.signature.clone(),
            operation_index,
            semantic_parent_path: parent_path.clone(),
            first_semantic_index: u32::try_from(first_structural_semantic)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
        });
        self.register_prepared_children(
            operation_index,
            insert_id,
            &parent_path,
            u32::try_from(first_structural_semantic)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
            target_index,
            after,
        )?;
        self.charge_operation_work(
            operation_index,
            parent_target
                .signature
                .children
                .len()
                .checked_add(prepared_work)
                .and_then(|work| work.checked_add(json.len()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        let _ = before;
        Ok(())
    }

    fn register_prepared_children(
        &mut self,
        operation_index: usize,
        insert_id: usize,
        parent_path: &[u32],
        first_semantic_index: u32,
        after_target: usize,
        after: &Document,
    ) -> OperationResult<()> {
        let nodes = self
            .prepared_inserts
            .get(insert_id)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
            .nodes
            .clone();
        let mut elements = Vec::new();
        let mut texts = Vec::new();
        collect_prepared_child_handles(
            insert_id,
            &nodes,
            parent_path,
            first_semantic_index,
            Some(after),
            &mut elements,
            &mut texts,
        )?;
        for (path, handle) in elements {
            if self.prepared_elements.insert(path, handle).is_some() {
                return Err(OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "duplicate prepared element semantic path",
                ));
            }
        }
        let positions = self.positions()?;
        let mut previous_end = positions
            .get(after_target)
            .map(|(_, end)| *end)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        for (insertion, (path, handle, runs)) in (after_target + 1..).zip(texts) {
            let start = first_text_doc_position(after.root(), &path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let text = prepared_runs_text(&runs);
            let scalar_len = u32::try_from(text.chars().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let gap_before = start
                .checked_sub(previous_end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets.insert(
                insertion,
                ResolvedText {
                    kind: ResolvedTargetKind::Prepared { handle },
                    gap_before,
                    text,
                    scalar_len,
                    base_runs: Vec::new(),
                    current_runs: runs,
                    action_slots: Vec::new(),
                },
            );
            previous_end = start
                .checked_add(scalar_len)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        Ok(())
    }
}

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
        probe_lookup_seed_publication(
            request_id,
            "bindingPublication",
            schema_fingerprint.len(),
        )?;
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
        let admitted_clock_scan_work = crdt_clock_scan_reservation(
            request_id,
            txn,
            snapshot_scan_reservation,
        )?;
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
            && self.binding.canonical_artifact.as_ref().is_some_and(|sealed| {
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
        let admitted_clock_scan_work = crdt_clock_scan_reservation(
            request_id,
            txn,
            resource_limits.max_encoded_state_bytes,
        )?;
        let current_snapshot = txn.snapshot();
        let store_state_is_exact = self
            .binding
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

impl<'a> LocalizedFormatLocator<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mint<T: ReadTxn>(
        document: &'a Document,
        block_path: &'a [u32],
        from: u32,
        to: u32,
        seed: &'a MutationLookupSeed,
        txn: &T,
        fragment: &XmlFragmentRef,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> Option<Self> {
        seed.matches(
            txn,
            fragment,
            document,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
        .then_some(Self {
            document,
            block_path,
            from,
            to,
            seed,
        })
    }
}

impl<'a> LocalizedRootWindowLocator<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mint<T: ReadTxn>(
        request_id: u64,
        document: &'a Document,
        expected_preview: &'a Document,
        replacement: &super::super::StructuralReplacement,
        seed: &'a MutationLookupSeed,
        txn: &T,
        fragment: &XmlFragmentRef,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Option<Self>> {
        let (from_child, to_child) = replacement.child_window();
        let Some(root) = document.root().content() else {
            return Ok(None);
        };
        let Ok(root_len) = u32::try_from(root.child_count()) else {
            return Ok(None);
        };
        if !replacement.parent_path().is_empty()
            || from_child >= to_child
            || to_child > root_len
            || !seed.matches(
                txn,
                fragment,
                document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
        {
            return Ok(None);
        }
        let mut expected_children = Vec::new();
        expected_children
            .try_reserve_exact(replacement.content().child_count())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "localized root-window content allocation failed",
                )
            })?;
        expected_children.extend(replacement.content().iter().cloned());
        Ok(Some(Self {
            document,
            expected_preview,
            from_child,
            to_child,
            expected_content: Fragment::from(expected_children),
            seed,
        }))
    }
}

fn localized_root_structural_parent<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    document: &Document,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> OperationResult<Option<StructuralParentTarget>> {
    let Some(semantic_children) = document.root().content() else {
        return Ok(None);
    };
    let child_count = usize::try_from(fragment.len(txn)).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized root child count exceeds usize",
        )
    })?;
    if child_count != semantic_children.child_count() {
        return Ok(None);
    }
    let parent_id = AsRef::<Branch>::as_ref(fragment).id();
    let mut storage_children = Vec::new();
    storage_children.try_reserve(child_count).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized root-window allocation failed",
        )
    })?;
    let mut child_ids = Vec::new();
    child_ids.try_reserve_exact(child_count).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized root signature allocation failed",
        )
    })?;
    for (index, (wire, semantic)) in fragment
        .children(txn)
        .zip(semantic_children.iter())
        .enumerate()
    {
        child_ids.push(wire.id());
        let XmlOut::Element(element) = wire else {
            return Ok(None);
        };
        let tag = element.tag();
        let normalized_heading = (tag.as_ref() == "heading")
            .then(|| super::super::codec::normalized_wire_element_node_type(&element, txn));
        let wire_node_type = normalized_heading.as_deref().unwrap_or(tag.as_ref());
        if semantic.is_text()
            || semantic.node_type() != wire_node_type
            || semantic.is_void() != wire_element_is_semantic_void(&element, txn, schema)
        {
            return Ok(None);
        }
        let mut attrs = Vec::new();
        let mut normalized_attr_count = 0usize;
        let synthetic_heading_level = wire_node_type != tag.as_ref();
        let mut attribute_budget =
            super::super::codec::WireAttributeJsonBudget::new(resource_limits);
        for (key, value) in element.attributes(txn) {
            let yrs::Out::Any(value) = value else {
                return Ok(None);
            };
            if !(synthetic_heading_level && key == "level") {
                let Some(expected) = semantic.attrs().get(key) else {
                    return Ok(None);
                };
                let Ok(actual) = attribute_budget.convert(&value) else {
                    return Ok(None);
                };
                if &actual != expected {
                    return Ok(None);
                }
                normalized_attr_count = normalized_attr_count.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "localized root normalized attribute count overflow",
                    )
                })?;
            }
            attrs.try_reserve(1).map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "localized root attribute allocation failed",
                )
            })?;
            attrs.push((Arc::<str>::from(key), value));
        }
        if normalized_attr_count != semantic.attrs().len() {
            return Ok(None);
        }
        attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
        let child_index = u32::try_from(index).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized root child index exceeds u32",
            )
        })?;
        let mut path = Vec::new();
        path.try_reserve_exact(1).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "localized root path allocation failed",
            )
        })?;
        path.push((parent_id.clone(), child_index));
        storage_children.push(StorageChildKind::Element {
            target: element.clone(),
            signature: Arc::new(ElementSignature {
                target: AsRef::<Branch>::as_ref(&element).id(),
                path,
                tag: element.tag().clone(),
                attrs,
            }),
        });
    }
    Ok(Some(StructuralParentTarget {
        parent: XmlParentRef::Fragment(fragment.clone()),
        signature: Arc::new(StructuralParentSignature {
            parent: parent_id,
            path: Vec::new(),
            children: child_ids,
        }),
        storage_children,
    }))
}

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

impl LocalizedFormatCompiler {
    pub(crate) fn charge_format_boundary_node(
        &mut self,
        operation_index: usize,
    ) -> OperationResult<()> {
        self.compiler.charge_boundary_node(operation_index)
    }

    pub(crate) fn charge_format_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        self.compiler
            .charge_boundary_text(operation_index, text_bytes)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
        locator: LocalizedFormatLocator<'_>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Option<Self>> {
        let seed = locator.seed;
        if !seed.matches_storage(
            txn,
            fragment,
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
        }) = localized_existing_textblock_targets(
            request_id,
            txn,
            fragment,
            schema,
            LocalizedTextblockLocator::Format(locator),
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
        LOCALIZED_RANGE_FORMAT_HIT_COUNT
            .set(LOCALIZED_RANGE_FORMAT_HIT_COUNT.get().saturating_add(1));
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
            seed_pending_traversal_work: seed_payload.pending_traversal_work,
            seed_materialization_work: Arc::clone(&seed_payload.target_materialization_work),
        }))
    }

    pub(crate) fn format(
        mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
        attrs: Attrs,
    ) -> OperationResult<(YrsMutationPlan, MutationLookupPromotion)> {
        let base_pending_traversal_work = self.seed_pending_traversal_work;
        self.compiler
            .format(operation_index, from, to, boundaries, attrs)?;

        let mut materialization_work_updates = Vec::new();
        materialization_work_updates
            .try_reserve(self.compiler.actions.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion allocation failed",
                )
            })?;
        let mut promoted_targets = HashSet::new();
        promoted_targets
            .try_reserve(self.compiler.actions.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion allocation failed",
                )
            })?;
        let mut current_materialization_work = HashMap::new();
        current_materialization_work
            .try_reserve(self.compiler.targets.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion allocation failed",
                )
            })?;
        for target in &self.compiler.targets {
            #[cfg(test)]
            LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT.set(
                LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT
                    .get()
                    .saturating_add(1),
            );
            let ResolvedTargetKind::Existing { signature, .. } = &target.kind else {
                return Err(OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion contains a non-existing target",
                ));
            };
            let next_work =
                prepared_materialization_work(self.compiler.request_id, &target.current_runs)?;
            if current_materialization_work
                .insert(signature.target.clone(), next_work)
                .is_some()
            {
                return Err(OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion contains a duplicate target",
                ));
            }
        }
        for slot in &self.compiler.actions {
            let ActionSlot::Concrete(action) = slot else {
                continue;
            };
            let YrsMutationAction::FormatText { signature, .. } = action.as_ref() else {
                continue;
            };
            if !promoted_targets.insert(signature.target.clone()) {
                continue;
            }
            if self
                .seed_materialization_work
                .get(&signature.target)
                .copied()
                != Some(signature.capture_work)
            {
                return Err(OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion does not match its seed",
                ));
            }
            let next_work = current_materialization_work
                .get(&signature.target)
                .copied()
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        self.compiler.request_id,
                        Some(operation_index),
                        "localized format promotion target is absent from the sealed compiler",
                    )
                })?;
            materialization_work_updates.push((
                signature.target.clone(),
                signature.capture_work,
                next_work,
            ));
        }
        if materialization_work_updates.is_empty() {
            return Err(OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized format produced no existing-text action",
            ));
        }
        let (old_work, new_work) = materialization_work_updates
            .iter()
            .try_fold((0usize, 0usize), |(old_total, new_total), (_, old, new)| {
                Some((old_total.checked_add(*old)?, new_total.checked_add(*new)?))
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion work overflow",
                )
            })?;
        let next_pending_traversal_work = base_pending_traversal_work
            .checked_sub(old_work)
            .and_then(|work| work.checked_add(new_work))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion work overflow",
                )
            })?;
        let promotion = MutationLookupPromotion {
            request_id: self.compiler.request_id,
            source: MutationLookupPromotionSource::ExistingFormat,
            materialization_work_updates,
            next_pending_traversal_work,
        };
        self.compiler
            .finish(Some(operation_index))
            .map(|plan| (plan, promotion))
    }
}

fn prepared_materialization_work(
    request_id: u64,
    runs: &[PreparedTextRun],
) -> OperationResult<usize> {
    let overflow = || {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "Yrs XML text materialization work overflow",
        )
    };
    runs.iter()
        .filter(|run| !run.text.is_empty())
        .try_fold(0usize, |work, run| {
            let attrs = &run.attrs;
            let attr_work = attrs.iter().try_fold(attrs.len(), |work, (key, value)| {
                work.checked_add(key.len())
                    .and_then(|work| work.checked_add(super::plan::any_preflight_work(value)?))
            });
            let sort_partitions = binary_partition_work(attrs.len());
            let sort_key_work = attrs.iter().try_fold(0usize, |work, (key, _)| {
                work.checked_add(key.len().checked_mul(sort_partitions)?)
            });
            work.checked_add(attr_work.ok_or_else(overflow)?)
                .and_then(|work| {
                    work.checked_add(
                        attrs
                            .len()
                            .checked_mul(sort_partitions)?
                            .checked_add(sort_key_work?)?,
                    )
                })
                .and_then(|work| work.checked_add(run.text.len()))
                .and_then(|work| work.checked_add(1))
                .ok_or_else(overflow)
        })
}

struct LocalizedTextblockTargets {
    targets: Vec<ResolvedText>,
    path_parent_widths: HashMap<BranchID, usize>,
}

#[derive(Clone, Copy)]
enum LocalizedTextblockLocator<'a> {
    Insert(LocalizedInsertLocator<'a>),
    Format(LocalizedFormatLocator<'a>),
}

impl<'a> LocalizedTextblockLocator<'a> {
    fn document(self) -> &'a Document {
        match self {
            Self::Insert(locator) => locator.document,
            Self::Format(locator) => locator.document,
        }
    }

    fn block_path(self) -> &'a [u32] {
        match self {
            Self::Insert(locator) => locator.block_path,
            Self::Format(locator) => locator.block_path,
        }
    }
}

fn localized_existing_textblock_targets<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
    locator: LocalizedTextblockLocator<'_>,
) -> OperationResult<Option<LocalizedTextblockTargets>> {
    let Some((semantic_block, block_start, block_end)) =
        semantic_node_bounds(locator.document(), locator.block_path())
    else {
        return Ok(None);
    };
    let valid_range = match locator {
        LocalizedTextblockLocator::Insert(locator) => {
            locator.position >= block_start && locator.position <= block_end
        }
        LocalizedTextblockLocator::Format(locator) => {
            locator.from < locator.to && locator.from >= block_start && locator.to <= block_end
        }
    };
    if !valid_range {
        return Ok(None);
    }
    if semantic_block.is_void()
        || !schema
            .node(semantic_block.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || semantic_block.content().is_none_or(|content| {
            content.child_count() == 0 || content.iter().any(|child| !child.is_text())
        })
    {
        return Ok(None);
    }

    let mut parent = XmlParentRef::Fragment(fragment.clone());
    let mut branch_path = Vec::<(BranchID, u32)>::new();
    let mut path_parent_widths = HashMap::<BranchID, usize>::new();
    let mut semantic_path = Vec::<u32>::new();
    for &child_index in locator.block_path() {
        let children = match &parent {
            XmlParentRef::Fragment(parent) => parent.children(txn).collect::<Vec<_>>(),
            XmlParentRef::Element(parent) => parent.children(txn).collect::<Vec<_>>(),
        };
        path_parent_widths.insert(parent.id(), children.len());
        let Some(child) = usize::try_from(child_index)
            .ok()
            .and_then(|index| children.get(index))
        else {
            return Ok(None);
        };
        let XmlOut::Element(element) = child else {
            return Ok(None);
        };
        semantic_path.push(child_index);
        let Some(semantic_node) = locator.document().node_at(&semantic_path) else {
            return Ok(None);
        };
        if semantic_node.node_type() != element.tag().as_ref()
            || wire_element_is_semantic_void(element, txn, schema)
        {
            return Ok(None);
        }
        branch_path.insert(0, (parent.id(), child_index));
        parent = XmlParentRef::Element(element.clone());
    }
    let XmlParentRef::Element(textblock) = parent else {
        return Ok(None);
    };
    let children = textblock.children(txn).collect::<Vec<_>>();
    path_parent_widths.insert(AsRef::<Branch>::as_ref(&textblock).id(), children.len());
    if children.is_empty()
        || children
            .iter()
            .any(|child| !matches!(child, XmlOut::Text(_)))
    {
        return Ok(None);
    }

    let mut located = Vec::new();
    let mut materialized_texts = HashMap::new();
    let mut traversal_work = 0usize;
    let context = TextTargetContext {
        request_id,
        txn,
        schema,
    };
    collect_text_targets(
        &context,
        (0u32..).zip(children),
        TextTargetParent {
            id: AsRef::<Branch>::as_ref(&textblock).id(),
            ancestors: &branch_path,
        },
        block_start,
        &mut traversal_work,
        &mut materialized_texts,
        &mut located,
    )?;
    if located.is_empty()
        || located
            .iter()
            .any(|target| !matches!(target, LocatedTarget::Existing { .. }))
    {
        return Ok(None);
    }
    if let LocalizedTextblockLocator::Insert(locator) = locator {
        let matching_targets = located
            .iter()
            .filter(|target| {
                let LocatedTarget::Existing {
                    start, scalar_len, ..
                } = target
                else {
                    return false;
                };
                start
                    .checked_add(*scalar_len)
                    .is_some_and(|end| locator.position >= *start && locator.position <= end)
            })
            .count();
        if matching_targets != 1 {
            return Ok(None);
        }
    }
    let combined_text = located
        .iter()
        .try_fold(String::new(), |mut combined, target| {
            let LocatedTarget::Existing { text, .. } = target else {
                return None;
            };
            combined.push_str(text);
            Some(combined)
        });
    if combined_text.as_deref() != Some(semantic_block.text_content().as_str()) {
        return Ok(None);
    }
    let mut previous_end = 0u32;
    let mut targets = Vec::with_capacity(located.len());
    for located in located {
        let LocatedTarget::Existing {
            start,
            target,
            text,
            scalar_len,
            signature,
        } = located
        else {
            return Ok(None);
        };
        let Some(materialized) = materialized_texts.get(&AsRef::<Branch>::as_ref(&target).id())
        else {
            return Ok(None);
        };
        let Some(gap_before) = start.checked_sub(previous_end) else {
            return Ok(None);
        };
        let Some(end) = start.checked_add(scalar_len) else {
            return Ok(None);
        };
        previous_end = end;
        targets.push(ResolvedText {
            kind: ResolvedTargetKind::Existing { target, signature },
            gap_before,
            text,
            scalar_len,
            base_runs: materialized.prepared_runs.clone(),
            current_runs: materialized.prepared_runs.clone(),
            action_slots: Vec::new(),
        });
    }
    Ok(Some(LocalizedTextblockTargets {
        targets,
        path_parent_widths,
    }))
}

fn semantic_node_bounds<'a>(
    document: &'a Document,
    node_path: &[u32],
) -> Option<(&'a Node, u32, u32)> {
    let mut parent = document.root();
    let mut parent_content_start = 0u32;
    for (depth, &raw_index) in node_path.iter().enumerate() {
        let index = usize::try_from(raw_index).ok()?;
        let content = parent.content()?;
        let child_start = content
            .iter()
            .take(index)
            .try_fold(parent_content_start, |position, sibling| {
                position.checked_add(sibling.node_size())
            })?;
        let child = content.child(index)?;
        let child_content_start = child_start.checked_add(u32::from(child.is_element()))?;
        if depth + 1 == node_path.len() {
            let child_content_end = child_content_start.checked_add(child.content()?.size())?;
            return Some((child, child_content_start, child_content_end));
        }
        parent = child;
        parent_content_start = child_content_start;
    }
    None
}
