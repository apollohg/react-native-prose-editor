impl MutationCompiler {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn wrap_in_list(
        &mut self,
        operation_index: usize,
        before: &Document,
        after: &Document,
        from: u32,
        to: u32,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let root_content = before
            .root()
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let mut offset = 0u32;
        let mut first = None;
        let mut last = None;
        for (index, child) in root_content.iter().enumerate() {
            let end = offset
                .checked_add(child.node_size())
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if end > from && offset < to {
                first.get_or_insert(index);
                last = Some(index);
            }
            offset = end;
        }
        let first = first.ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let last = last.ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let first_u32 = u32::try_from(first)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        let selected_count = last
            .checked_sub(first)
            .and_then(|count| count.checked_add(1))
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if self.try_wrap_prepared_root_batch(
            operation_index,
            after,
            first,
            last,
            selected_count,
            schema,
            limits,
        )? {
            return Ok(());
        }
        let root_target = self
            .structural_parents
            .get(&Vec::new())
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let path_is_selected = |path: &[u32]| {
            path.first().is_some_and(|index| {
                usize::try_from(*index).is_ok_and(|index| index >= first && index <= last)
            })
        };
        let structural_child_work = self
            .structural_parents
            .values()
            .try_fold(0usize, |work, parent| {
                work.checked_add(parent.storage_children.len())
            })
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        let traversal_work = self
            .structural_parents
            .len()
            .checked_add(structural_child_work)
            .and_then(|work| work.checked_add(self.targets.len()))
            .and_then(|work| work.checked_add(self.actions.len()))
            .and_then(|work| work.checked_add(self.prepared_elements.len()))
            .and_then(|work| work.checked_add(self.prepared_inserts.len()))
            .and_then(|work| work.checked_add(self.pending_element_attrs.len()))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, traversal_work)?;
        let checkpoint_scan_work = self
            .targets
            .iter()
            .try_fold(0usize, |work, target| {
                target.current_runs.iter().try_fold(
                    work.checked_add(target.text.len())?,
                    |work, run| {
                        work.checked_add(run.text.len())?
                            .checked_add(attrs_work(&run.attrs))
                    },
                )
            })
            .and_then(|work| work.checked_add(semantic_node_clone_work(before.root())?));
        self.charge_scan_work(
            operation_index,
            checkpoint_scan_work
                .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?,
        )?;
        let checkpoint = VirtualStateCheckpoint {
            document: before.clone(),
            targets: self.targets.clone(),
            structural_parents: self.structural_parents.clone(),
            actions: self.actions.clone(),
            prepared_inserts: self.prepared_inserts.clone(),
            prepared_elements: self.prepared_elements.clone(),
            created_gap_shifts: self.created_gap_shifts.clone(),
            pending_element_attrs: self.pending_element_attrs.clone(),
        };
        let mut selected_branch_ids = HashSet::new();
        for (path, parent) in &self.structural_parents {
            if !path_is_selected(path) {
                continue;
            }
            if let XmlParentRef::Element(element) = &parent.parent {
                selected_branch_ids.insert(AsRef::<Branch>::as_ref(element).id());
            }
            for child in &parent.storage_children {
                let id = match child {
                    StorageChildKind::Text { target, .. } => {
                        Some(AsRef::<Branch>::as_ref(target).id())
                    }
                    StorageChildKind::Element { target, .. } => {
                        Some(AsRef::<Branch>::as_ref(target).id())
                    }
                    StorageChildKind::PreparedElement => None,
                };
                if let Some(id) = id {
                    selected_branch_ids.insert(id);
                }
            }
        }
        let selected_prepared_insert_ids = self
            .prepared_elements
            .iter()
            .filter(|(path, _)| path_is_selected(path))
            .map(|(_, handle)| handle.insert_id)
            .collect::<HashSet<_>>();
        let target_is_selected = |target: &ResolvedText| match &target.kind {
            ResolvedTargetKind::Existing { target, .. } => {
                selected_branch_ids.contains(&AsRef::<Branch>::as_ref(target).id())
            }
            ResolvedTargetKind::Missing { parent, .. } => {
                selected_branch_ids.contains(&AsRef::<Branch>::as_ref(parent).id())
            }
            ResolvedTargetKind::Prepared { handle } => {
                selected_prepared_insert_ids.contains(&handle.insert_id)
            }
        };
        let selected_target_indices = self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| target_is_selected(target).then_some(index))
            .collect::<Vec<_>>();
        let positions = self.positions()?;
        let selected_start = root_content
            .iter()
            .take(first)
            .try_fold(0u32, |position, child| {
                position.checked_add(child.node_size())
            })
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let insertion_target = selected_target_indices
            .first()
            .copied()
            .unwrap_or_else(|| positions.partition_point(|(start, _)| *start < selected_start));
        let previous_end = if insertion_target == 0 {
            0
        } else {
            positions[insertion_target - 1].1
        };
        let next_target_before_removal = selected_target_indices
            .last()
            .map_or(insertion_target, |index| index + 1);
        let old_next_start = positions
            .get(next_target_before_removal)
            .map(|(start, _)| *start);
        for index in selected_target_indices.iter().copied() {
            for slot in std::mem::take(&mut self.targets[index].action_slots) {
                self.actions[slot] = ActionSlot::Tombstone;
            }
        }
        self.pending_element_attrs
            .retain(|id, _| !selected_branch_ids.contains(id));
        for slot in &mut self.actions {
            let selected = match slot {
                ActionSlot::PreparedInsert(insert_id) => {
                    selected_prepared_insert_ids.contains(insert_id)
                        || self
                            .prepared_inserts
                            .get(*insert_id)
                            .and_then(Option::as_ref)
                            .is_some_and(|insert| match &insert.parent {
                                XmlParentRef::Element(parent) => selected_branch_ids
                                    .contains(&AsRef::<Branch>::as_ref(parent).id()),
                                XmlParentRef::Fragment(_) => false,
                            })
                }
                ActionSlot::Concrete(action) => match action.as_ref() {
                    YrsMutationAction::InsertText { target, .. }
                    | YrsMutationAction::DeleteText { target, .. }
                    | YrsMutationAction::FormatText { target, .. } => {
                        selected_branch_ids.contains(&AsRef::<Branch>::as_ref(target).id())
                    }
                    YrsMutationAction::CreateText { parent, .. } => {
                        selected_branch_ids.contains(&AsRef::<Branch>::as_ref(parent).id())
                    }
                    YrsMutationAction::DeleteXmlChildren { parent, .. }
                    | YrsMutationAction::InsertXmlChildren { parent, .. } => match parent {
                        XmlParentRef::Element(parent) => {
                            selected_branch_ids.contains(&AsRef::<Branch>::as_ref(parent).id())
                        }
                        XmlParentRef::Fragment(_) => false,
                    },
                    YrsMutationAction::SetXmlAttribute { target, .. }
                    | YrsMutationAction::RemoveXmlAttribute { target, .. } => {
                        selected_branch_ids.contains(&AsRef::<Branch>::as_ref(target).id())
                    }
                },
                ActionSlot::Tombstone => false,
            };
            if selected {
                if let ActionSlot::PreparedInsert(insert_id) = slot {
                    self.prepared_inserts[*insert_id] = None;
                }
                *slot = ActionSlot::Tombstone;
            }
        }
        self.targets.retain(|target| !target_is_selected(target));
        let root_shift = u32::try_from(last - first)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        let mut remapped_parents = HashMap::with_capacity(self.structural_parents.len());
        for (mut path, mut parent) in std::mem::take(&mut self.structural_parents) {
            if path.is_empty() {
                parent.storage_children.splice(
                    first..=last,
                    std::iter::once(StorageChildKind::PreparedElement),
                );
                remapped_parents.insert(path, parent);
                continue;
            }
            if path_is_selected(&path) {
                continue;
            }
            if usize::try_from(path[0]).is_ok_and(|index| index > last) {
                path[0] = path[0]
                    .checked_sub(root_shift)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            remapped_parents.insert(path, parent);
        }
        self.structural_parents = remapped_parents;
        let mut remapped_elements = HashMap::with_capacity(self.prepared_elements.len());
        for (mut path, handle) in std::mem::take(&mut self.prepared_elements) {
            if path_is_selected(&path) {
                continue;
            }
            if path
                .first()
                .and_then(|index| usize::try_from(*index).ok())
                .is_some_and(|index| index > last)
            {
                path[0] = path[0]
                    .checked_sub(root_shift)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            remapped_elements.insert(path, handle);
        }
        self.prepared_elements = remapped_elements;

        let list_node = after
            .root()
            .content()
            .and_then(|content| content.child(first))
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let mut batch = prepare_direct_root_wrap_batch(
            self.request_id,
            operation_index,
            list_node,
            schema,
            limits,
            false,
        )?;
        for child in &mut batch.nodes {
            child.index = first_u32
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        self.push_action(YrsMutationAction::DeleteXmlChildren {
            parent: root_target.parent.clone(),
            child_index: first_u32,
            child_count: selected_count,
            signature: root_target.signature.clone(),
            operation_index,
        });
        let prepared_work = batch
            .batch_work
            .checked_add(batch.empty_work)
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
            parent: root_target.parent,
            child_index: first_u32,
            nodes: batch.nodes,
            signature: root_target.signature.clone(),
            operation_index,
            semantic_parent_path: Vec::new(),
            first_semantic_index: first_u32,
        });
        self.wrap_checkpoints.insert(insert_id, checkpoint);
        let nodes = self
            .prepared_inserts
            .get(insert_id)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
            .nodes
            .clone();
        self.charge_operation_work(
            operation_index,
            prepared_clone_work(&nodes).ok_or_else(|| {
                work_overflow(self.request_id, operation_index, self.action_limit)
            })?,
        )?;
        let mut elements = Vec::new();
        let mut texts = Vec::new();
        collect_prepared_child_handles(
            insert_id,
            &nodes,
            &[],
            first_u32,
            Some(after),
            &mut elements,
            &mut texts,
        )?;
        for (path, handle) in elements {
            if self.prepared_elements.insert(path, handle).is_some() {
                return Err(OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "duplicate wrapped prepared element semantic path",
                ));
            }
        }
        let mut target_index = insertion_target;
        let mut current_end = previous_end;
        for (path, handle, runs) in texts {
            let start = first_text_doc_position(after.root(), &path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let text = prepared_runs_text(&runs);
            let scalar_len = u32::try_from(text.chars().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let gap_before = start
                .checked_sub(current_end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets.insert(
                target_index,
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
            current_end = start
                .checked_add(scalar_len)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            target_index += 1;
        }
        if let Some(old_next_start) = old_next_start {
            let added = selected_count
                .checked_mul(2)
                .and_then(|amount| amount.checked_add(2))
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let next_start = old_next_start
                .checked_add(added)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets[target_index].gap_before = next_start
                .checked_sub(current_end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        self.charge_operation_work(
            operation_index,
            root_target
                .signature
                .children
                .len()
                .checked_add(prepared_work)
                .and_then(|work| work.checked_add(usize::try_from(selected_count).ok()?))
                .and_then(|work| work.checked_add(selected_branch_ids.len()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn try_wrap_prepared_root_batch(
        &mut self,
        operation_index: usize,
        after: &Document,
        first: usize,
        last: usize,
        selected_count: u32,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<bool> {
        let selected = (first..=last)
            .map(|index| {
                let index = u32::try_from(index).ok()?;
                self.prepared_elements.get(&vec![index]).cloned()
            })
            .collect::<Option<Vec<_>>>();
        let Some(selected) = selected else {
            return Ok(false);
        };
        let Some(first_handle) = selected.first() else {
            return Ok(false);
        };
        let insert_id = first_handle.insert_id;
        if selected.iter().any(|handle| handle.insert_id != insert_id) {
            return Ok(false);
        }
        let roots = selected
            .iter()
            .map(|handle| handle.ordinal_path.first().copied())
            .collect::<Option<Vec<_>>>();
        let Some(roots) = roots else {
            return Ok(false);
        };
        if roots
            .windows(2)
            .any(|pair| pair[1] != pair[0].saturating_add(1))
        {
            return Ok(false);
        }
        let root_start = roots[0];
        let root_end = roots[roots.len() - 1]
            .checked_add(1)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let batch_semantic_start = self
            .prepared_elements
            .iter()
            .filter(|(_, handle)| handle.insert_id == insert_id)
            .filter_map(|(path, handle)| {
                (handle.ordinal_path.len() == 1)
                    .then(|| path.first().copied())
                    .flatten()
            })
            .min()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;

        let list_node = after
            .root()
            .content()
            .and_then(|content| content.child(first))
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let json = crate::serialize::node_to_prosemirror_json(list_node, schema);
        let mut replacement = prepare_xml_nodes(std::slice::from_ref(&json), limits, 2)
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let empty_work = materialize_empty_prepared_textblocks(&mut replacement.nodes, schema);
        let replacement_node = replacement
            .nodes
            .pop()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let insert = self
            .prepared_inserts
            .get_mut(insert_id)
            .and_then(Option::as_mut)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if root_end > insert.nodes.len() {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        insert
            .nodes
            .splice(root_start..root_end, std::iter::once(replacement_node));
        for (ordinal, child) in insert.nodes.iter_mut().enumerate() {
            child.index = insert
                .child_index
                .checked_add(
                    u32::try_from(ordinal)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                )
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let nodes = insert.nodes.clone();

        let positions = self.positions()?;
        let selected_target_indices = self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| match &target.kind {
                ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id => {
                    Some(index)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let insertion_target = selected_target_indices.first().copied().unwrap_or_else(|| {
            let start = u32::try_from(first).unwrap_or(u32::MAX);
            positions.partition_point(|(position, _)| *position < start)
        });
        let next_before_removal = selected_target_indices
            .last()
            .map_or(insertion_target, |index| index + 1);
        let old_next_start = positions.get(next_before_removal).map(|(start, _)| *start);
        let previous_end = if insertion_target == 0 {
            0
        } else {
            positions[insertion_target - 1].1
        };
        self.targets.retain(|target| {
            !matches!(
                &target.kind,
                ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id
            )
        });

        let root_shift = u32::try_from(last - first)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        if let Some(root) = self.structural_parents.get_mut(&Vec::new()) {
            root.storage_children.splice(
                first..=last,
                std::iter::once(StorageChildKind::PreparedElement),
            );
        }
        let mut parents = HashMap::with_capacity(self.structural_parents.len());
        for (mut path, parent) in std::mem::take(&mut self.structural_parents) {
            if !path.is_empty() && usize::try_from(path[0]).is_ok_and(|index| index > last) {
                path[0] = path[0]
                    .checked_sub(root_shift)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            parents.insert(path, parent);
        }
        self.structural_parents = parents;
        self.prepared_elements
            .retain(|_, handle| handle.insert_id != insert_id);
        let mut remapped = HashMap::with_capacity(self.prepared_elements.len());
        for (mut path, handle) in std::mem::take(&mut self.prepared_elements) {
            if path
                .first()
                .and_then(|index| usize::try_from(*index).ok())
                .is_some_and(|index| index > last)
            {
                path[0] = path[0]
                    .checked_sub(root_shift)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            remapped.insert(path, handle);
        }
        self.prepared_elements = remapped;

        let mut elements = Vec::new();
        let mut texts = Vec::new();
        collect_prepared_child_handles(
            insert_id,
            &nodes,
            &[],
            batch_semantic_start,
            Some(after),
            &mut elements,
            &mut texts,
        )?;
        for (path, handle) in elements {
            self.prepared_elements.insert(path, handle);
        }
        let mut target_index = insertion_target;
        let mut current_end = previous_end;
        for (path, handle, runs) in texts {
            let start = first_text_doc_position(after.root(), &path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let text = prepared_runs_text(&runs);
            let scalar_len = u32::try_from(text.chars().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            self.targets.insert(
                target_index,
                ResolvedText {
                    kind: ResolvedTargetKind::Prepared { handle },
                    gap_before: start
                        .checked_sub(current_end)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?,
                    text,
                    scalar_len,
                    base_runs: Vec::new(),
                    current_runs: runs,
                    action_slots: Vec::new(),
                },
            );
            current_end = start
                .checked_add(scalar_len)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            target_index += 1;
        }
        if let Some(old_next_start) = old_next_start {
            let added = selected_count
                .checked_mul(2)
                .and_then(|amount| amount.checked_add(2))
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets[target_index].gap_before = old_next_start
                .checked_add(added)
                .and_then(|start| start.checked_sub(current_end))
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        self.charge_operation_work(
            operation_index,
            replacement
                .work
                .checked_add(empty_work)
                .and_then(|work| work.checked_add(prepared_clone_work(&nodes)?))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        Ok(true)
    }
}
