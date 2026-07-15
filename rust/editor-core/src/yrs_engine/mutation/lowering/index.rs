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
        let mut located = Vec::new();
        let mut materialized_texts = HashMap::new();
        let mut traversal_work = 0usize;
        let text_target_context = TextTargetContext {
            request_id,
            txn,
            schema,
        };
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
                        if !child.is_text() {
                            return None;
                        }
                        let width = child.node_size();
                        if width == 0 || width > remaining {
                            return None;
                        }
                        offset = offset.checked_add(width)?;
                        remaining -= width;
                        semantic_index += 1;
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
