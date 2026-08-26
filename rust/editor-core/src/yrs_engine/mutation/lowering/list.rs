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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn indent_list_item(
        &mut self,
        operation_index: usize,
        before: &Document,
        after: &Document,
        position: u32,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let resolved = before.resolve(position).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "at", message)
        })?;
        let mut node = before.root();
        let mut item_depth = None;
        for (depth, index) in resolved.node_path.iter().copied().enumerate() {
            node = node
                .child(
                    usize::try_from(index)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                )
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if schema
                .node(node.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
            {
                item_depth = Some(depth);
            }
        }
        let item_depth = item_depth.ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "position is not inside a list item",
            )
        })?;
        let list_path = resolved.node_path[..item_depth].to_vec();
        let item_index = resolved.node_path[item_depth];
        if item_index == 0 {
            return Ok(());
        }
        let list_node = before
            .node_at(&list_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let list_content = list_node
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let item_index_usize = usize::try_from(item_index)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        if item_index_usize >= list_content.child_count() {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        let mut previous_item_path = list_path.clone();
        previous_item_path.push(
            item_index
                .checked_sub(1)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?,
        );
        let previous_item = before
            .node_at(&previous_item_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let previous_content = previous_item
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let existing_nested_index = previous_content
            .child_count()
            .checked_sub(1)
            .filter(|index| {
                previous_content
                    .child(*index)
                    .is_some_and(|child| child.node_type() == list_node.node_type())
            });

        if let Some(handle) = self.prepared_elements.get(&list_path).cloned() {
            let final_list = after
                .node_at(&list_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let json = crate::serialize::node_to_prosemirror_json(final_list, schema);
            let mut batch = prepare_xml_nodes(
                std::slice::from_ref(&json),
                limits,
                list_path.len().saturating_add(1),
            )
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
            let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
            let insert_id = handle.insert_id;
            self.replace_prepared_element_with_children(operation_index, &handle, batch.nodes)?;
            self.prepared_elements
                .retain(|_, candidate| candidate.insert_id != insert_id);
            self.targets.retain(|target| {
                !matches!(
                    &target.kind,
                    ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id
                )
            });
            self.wrap_checkpoints.remove(&insert_id);
            self.charge_operation_work(
                operation_index,
                batch.work.checked_add(empty_work).ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
            )?;
            self.register_prepared_insert_state(operation_index, insert_id, after)?;
            return Ok(());
        }

        let source_list_target = self
            .structural_parents
            .get(&list_path)
            .cloned()
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "indent source list has no tracked Yrs branch",
                )
            })?;
        self.push_action(YrsMutationAction::DeleteXmlChildren {
            parent: source_list_target.parent.clone(),
            child_index: item_index,
            child_count: 1,
            signature: source_list_target.signature.clone(),
            operation_index,
        });

        let prepared_destination = if let Some(nested_index) = existing_nested_index {
            let mut nested_path = previous_item_path.clone();
            nested_path.push(
                u32::try_from(nested_index)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
            );
            self.prepared_elements
                .get(&nested_path)
                .cloned()
                .map(|handle| (nested_path, handle))
        } else {
            self.prepared_elements
                .get(&previous_item_path)
                .cloned()
                .map(|handle| (previous_item_path.clone(), handle))
        };
        if let Some((prepared_path, handle)) = prepared_destination {
            let final_node = after
                .node_at(&prepared_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let json = crate::serialize::node_to_prosemirror_json(final_node, schema);
            let mut batch = prepare_xml_nodes(
                std::slice::from_ref(&json),
                limits,
                prepared_path.len().saturating_add(1),
            )
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
            let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
            let insert_id = handle.insert_id;
            self.replace_prepared_element_with_children(operation_index, &handle, batch.nodes)?;
            self.prepared_elements
                .retain(|_, candidate| candidate.insert_id != insert_id);
            self.targets.retain(|target| {
                !matches!(
                    &target.kind,
                    ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id
                )
            });
            self.apply_virtual_structural_splices(
                operation_index,
                after,
                &[VirtualStructuralSplice {
                    parent_path: list_path,
                    semantic_index: item_index,
                    semantic_delete: 1,
                    semantic_insert: 0,
                    storage_index: item_index,
                    storage_delete: 1,
                }],
                Some(insert_id),
            )?;
            self.wrap_checkpoints.remove(&insert_id);
            self.charge_operation_work(
                operation_index,
                source_list_target
                    .signature
                    .children
                    .len()
                    .checked_add(batch.work)
                    .and_then(|work| work.checked_add(empty_work))
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?,
            )?;
            self.register_prepared_insert_state(operation_index, insert_id, after)?;
            return Ok(());
        }

        let (
            destination_path,
            destination_target,
            semantic_insert_index,
            storage_insert_index,
            prepared_node,
        ) = if let Some(nested_index) = existing_nested_index {
            let nested_index = u32::try_from(nested_index)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let mut nested_path = previous_item_path.clone();
            nested_path.push(nested_index);
            let nested_target = self
                .structural_parents
                .get(&nested_path)
                .cloned()
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "indent destination list has no tracked Yrs branch",
                    )
                })?;
            let nested_before = before
                .node_at(&nested_path)
                .and_then(Node::content)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let insert_index = u32::try_from(nested_before.child_count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let mut inserted_path = nested_path.clone();
            inserted_path.push(insert_index);
            let inserted = after
                .node_at(&inserted_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            (
                nested_path,
                nested_target,
                insert_index,
                insert_index,
                inserted,
            )
        } else {
            let previous_target = self
                .structural_parents
                .get(&previous_item_path)
                .cloned()
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "indent previous item has no tracked Yrs branch",
                    )
                })?;
            let insert_index = u32::try_from(previous_content.child_count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let mut inserted_path = previous_item_path.clone();
            inserted_path.push(insert_index);
            let inserted = after
                .node_at(&inserted_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let storage_index = u32::try_from(previous_target.storage_children.len())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            (
                previous_item_path,
                previous_target,
                insert_index,
                storage_index,
                inserted,
            )
        };

        let json = crate::serialize::node_to_prosemirror_json(prepared_node, schema);
        let mut batch = prepare_xml_nodes(
            std::slice::from_ref(&json),
            limits,
            destination_path.len().saturating_add(2),
        )
        .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
        for child in &mut batch.nodes {
            child.index = storage_insert_index
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let operation_work = source_list_target
            .signature
            .children
            .len()
            .checked_add(destination_target.signature.children.len())
            .and_then(|work| work.checked_add(batch.work))
            .and_then(|work| work.checked_add(empty_work))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.apply_virtual_structural_splices(
            operation_index,
            after,
            &[
                VirtualStructuralSplice {
                    parent_path: list_path,
                    semantic_index: item_index,
                    semantic_delete: 1,
                    semantic_insert: 0,
                    storage_index: item_index,
                    storage_delete: 1,
                },
                VirtualStructuralSplice {
                    parent_path: destination_path.clone(),
                    semantic_index: semantic_insert_index,
                    semantic_delete: 0,
                    semantic_insert: 1,
                    storage_index: storage_insert_index,
                    storage_delete: 0,
                },
            ],
            None,
        )?;
        let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
            parent: destination_target.parent,
            child_index: storage_insert_index,
            nodes: batch.nodes,
            signature: destination_target.signature,
            operation_index,
            semantic_parent_path: destination_path,
            first_semantic_index: semantic_insert_index,
        });
        self.register_prepared_insert_state(operation_index, insert_id, after)?;
        self.charge_operation_work(operation_index, operation_work)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn outdent_list_item(
        &mut self,
        operation_index: usize,
        before: &Document,
        after: &Document,
        position: u32,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let resolved = before.resolve(position).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "at", message)
        })?;
        let mut node = before.root();
        let mut item_depth = None;
        for (depth, index) in resolved.node_path.iter().copied().enumerate() {
            node = node
                .child(
                    usize::try_from(index)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                )
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if schema
                .node(node.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
            {
                item_depth = Some(depth);
            }
        }
        let item_depth = item_depth.ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "position is not inside a list item",
            )
        })?;
        let nested_list_path = resolved.node_path[..item_depth].to_vec();
        if nested_list_path.is_empty() {
            return Ok(());
        }
        let parent_item_path = nested_list_path[..nested_list_path.len() - 1].to_vec();
        let Some(parent_item) = before.node_at(&parent_item_path) else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        if !schema
            .node(parent_item.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
            || parent_item_path.is_empty()
        {
            return Ok(());
        }
        let parent_list_path = parent_item_path[..parent_item_path.len() - 1].to_vec();
        let parent_item_index = *parent_item_path
            .last()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let nested_child_index = *nested_list_path
            .last()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let selected_index = resolved.node_path[item_depth];
        let nested_list = before
            .node_at(&nested_list_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let nested_content = nested_list
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let selected_index_usize = usize::try_from(selected_index)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        if selected_index_usize >= nested_content.child_count() {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        let trailing_count = nested_content
            .child_count()
            .checked_sub(selected_index_usize)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let trailing_count_u32 = u32::try_from(trailing_count)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        let insert_index = parent_item_index
            .checked_add(1)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if let Some(handle) = self.prepared_elements.get(&parent_list_path).cloned() {
            let final_parent_list = after
                .node_at(&parent_list_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let json = crate::serialize::node_to_prosemirror_json(final_parent_list, schema);
            let mut batch = prepare_xml_nodes(
                std::slice::from_ref(&json),
                limits,
                parent_list_path.len().saturating_add(1),
            )
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
            let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
            let insert_id = handle.insert_id;
            self.replace_prepared_element_with_children(operation_index, &handle, batch.nodes)?;
            self.prepared_elements
                .retain(|_, candidate| candidate.insert_id != insert_id);
            self.targets.retain(|target| {
                !matches!(
                    &target.kind,
                    ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id
                )
            });
            self.wrap_checkpoints.remove(&insert_id);
            self.charge_operation_work(
                operation_index,
                batch.work.checked_add(empty_work).ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
            )?;
            self.register_prepared_insert_state(operation_index, insert_id, after)?;
            return Ok(());
        }
        let mut moved_path = parent_list_path.clone();
        moved_path.push(insert_index);
        let moved_item = after
            .node_at(&moved_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let json = crate::serialize::node_to_prosemirror_json(moved_item, schema);
        let mut batch = prepare_xml_nodes(
            std::slice::from_ref(&json),
            limits,
            parent_list_path.len().saturating_add(2),
        )
        .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
        for child in &mut batch.nodes {
            child.index = insert_index
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }

        let parent_list_target = self
            .structural_parents
            .get(&parent_list_path)
            .cloned()
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "outdent parent list has no tracked Yrs branch",
                )
            })?;
        let prepared_source = self.prepared_elements.get(&nested_list_path).cloned();
        let source_is_prepared = prepared_source.is_some();
        if selected_index > 0 {
            if let Some(handle) = prepared_source.clone() {
                let final_nested = after
                    .node_at(&nested_list_path)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                let json = crate::serialize::node_to_prosemirror_json(final_nested, schema);
                let mut retained_batch = prepare_xml_nodes(
                    std::slice::from_ref(&json),
                    limits,
                    nested_list_path.len().saturating_add(1),
                )
                .map_err(|error| {
                    map_prepared_node_error(self.request_id, operation_index, error)
                })?;
                let retained_empty_work =
                    materialize_empty_prepared_textblocks(&mut retained_batch.nodes, schema);
                let retained_work = retained_batch
                    .work
                    .checked_add(retained_empty_work)
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?;
                let source_insert_id = handle.insert_id;
                self.replace_prepared_element_with_children(
                    operation_index,
                    &handle,
                    retained_batch.nodes,
                )?;
                self.prepared_elements
                    .retain(|_, candidate| candidate.insert_id != source_insert_id);
                self.targets.retain(|target| {
                    !matches!(
                        &target.kind,
                        ResolvedTargetKind::Prepared { handle }
                            if handle.insert_id == source_insert_id
                    )
                });
                self.apply_virtual_structural_splices(
                    operation_index,
                    after,
                    &[VirtualStructuralSplice {
                        parent_path: parent_list_path.clone(),
                        semantic_index: insert_index,
                        semantic_delete: 0,
                        semantic_insert: 1,
                        storage_index: insert_index,
                        storage_delete: 0,
                    }],
                    Some(source_insert_id),
                )?;
                self.wrap_checkpoints.remove(&source_insert_id);
                let moved_insert_id = self.queue_prepared_insert(PendingPreparedInsert {
                    parent: parent_list_target.parent,
                    child_index: insert_index,
                    nodes: batch.nodes,
                    signature: parent_list_target.signature.clone(),
                    operation_index,
                    semantic_parent_path: parent_list_path,
                    first_semantic_index: insert_index,
                });
                self.charge_operation_work(
                    operation_index,
                    parent_list_target
                        .signature
                        .children
                        .len()
                        .checked_add(batch.work)
                        .and_then(|work| work.checked_add(empty_work))
                        .and_then(|work| work.checked_add(retained_work))
                        .and_then(|work| work.checked_add(trailing_count))
                        .ok_or_else(|| {
                            work_overflow(self.request_id, operation_index, self.action_limit)
                        })?,
                )?;
                self.register_prepared_insert_state(operation_index, source_insert_id, after)?;
                self.register_prepared_insert_state(operation_index, moved_insert_id, after)?;
                return Ok(());
            }
        }
        if selected_index == 0 && !self.structural_parents.contains_key(&parent_item_path) {
            if let Some(handle) = prepared_source {
                let source_insert_id = handle.insert_id;
                self.replace_prepared_element_with_children(operation_index, &handle, Vec::new())?;
                self.prepared_elements
                    .retain(|_, candidate| candidate.insert_id != source_insert_id);
                self.targets.retain(|target| {
                    !matches!(
                        &target.kind,
                        ResolvedTargetKind::Prepared { handle }
                            if handle.insert_id == source_insert_id
                    )
                });
                self.apply_virtual_structural_splices(
                    operation_index,
                    after,
                    &[VirtualStructuralSplice {
                        parent_path: parent_list_path.clone(),
                        semantic_index: insert_index,
                        semantic_delete: 0,
                        semantic_insert: 1,
                        storage_index: insert_index,
                        storage_delete: 0,
                    }],
                    Some(source_insert_id),
                )?;
                self.wrap_checkpoints.remove(&source_insert_id);
                let moved_insert_id = self.queue_prepared_insert(PendingPreparedInsert {
                    parent: parent_list_target.parent,
                    child_index: insert_index,
                    nodes: batch.nodes,
                    signature: parent_list_target.signature.clone(),
                    operation_index,
                    semantic_parent_path: parent_list_path,
                    first_semantic_index: insert_index,
                });
                self.charge_operation_work(
                    operation_index,
                    parent_list_target
                        .signature
                        .children
                        .len()
                        .checked_add(batch.work)
                        .and_then(|work| work.checked_add(empty_work))
                        .and_then(|work| work.checked_add(trailing_count))
                        .ok_or_else(|| {
                            work_overflow(self.request_id, operation_index, self.action_limit)
                        })?,
                )?;
                self.register_prepared_insert_state(operation_index, source_insert_id, after)?;
                self.register_prepared_insert_state(operation_index, moved_insert_id, after)?;
                return Ok(());
            }
        }
        let (delete_target, delete_splice) = if selected_index == 0 {
            let parent_item_target = self
                .structural_parents
                .get(&parent_item_path)
                .cloned()
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "outdent parent item has no tracked Yrs branch",
                    )
                })?;
            (
                (!source_is_prepared).then_some(parent_item_target),
                VirtualStructuralSplice {
                    parent_path: parent_item_path,
                    semantic_index: nested_child_index,
                    semantic_delete: 1,
                    semantic_insert: 0,
                    storage_index: nested_child_index,
                    storage_delete: 1,
                },
            )
        } else {
            let nested_target = self
                .structural_parents
                .get(&nested_list_path)
                .cloned()
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "outdent nested list has no tracked Yrs branch",
                    )
                })?;
            (
                Some(nested_target),
                VirtualStructuralSplice {
                    parent_path: nested_list_path,
                    semantic_index: selected_index,
                    semantic_delete: trailing_count_u32,
                    semantic_insert: 0,
                    storage_index: selected_index,
                    storage_delete: trailing_count_u32,
                },
            )
        };
        let delete_child_index = delete_splice.storage_index;
        let delete_child_count = delete_splice.storage_delete;
        if let Some(delete_target) = &delete_target {
            self.push_action(YrsMutationAction::DeleteXmlChildren {
                parent: delete_target.parent.clone(),
                child_index: delete_child_index,
                child_count: delete_child_count,
                signature: delete_target.signature.clone(),
                operation_index,
            });
        }
        let operation_work = parent_list_target
            .signature
            .children
            .len()
            .checked_add(
                delete_target
                    .as_ref()
                    .map_or(0, |target| target.signature.children.len()),
            )
            .and_then(|work| work.checked_add(batch.work))
            .and_then(|work| work.checked_add(empty_work))
            .and_then(|work| work.checked_add(trailing_count))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.apply_virtual_structural_splices(
            operation_index,
            after,
            &[
                delete_splice,
                VirtualStructuralSplice {
                    parent_path: parent_list_path.clone(),
                    semantic_index: insert_index,
                    semantic_delete: 0,
                    semantic_insert: 1,
                    storage_index: insert_index,
                    storage_delete: 0,
                },
            ],
            None,
        )?;
        let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
            parent: parent_list_target.parent,
            child_index: insert_index,
            nodes: batch.nodes,
            signature: parent_list_target.signature,
            operation_index,
            semantic_parent_path: parent_list_path,
            first_semantic_index: insert_index,
        });
        self.register_prepared_insert_state(operation_index, insert_id, after)?;
        self.charge_operation_work(operation_index, operation_work)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn unwrap_from_list(
        &mut self,
        operation_index: usize,
        before: &Document,
        after: &Document,
        position: u32,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let resolved = before.resolve(position).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "at", message)
        })?;
        let mut node = before.root();
        let mut list_item_depth = None;
        for (depth, index) in resolved.node_path.iter().copied().enumerate() {
            node = node
                .child(
                    usize::try_from(index)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                )
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if schema
                .node(node.node_type())
                .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
            {
                list_item_depth = Some(depth);
            }
        }
        let list_item_depth = list_item_depth.ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "position is not inside a list item",
            )
        })?;
        let list_path = resolved.node_path[..list_item_depth].to_vec();
        let list_index = *list_path
            .last()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let parent_path = list_path[..list_path.len() - 1].to_vec();
        let parent_before = before
            .node_at(&parent_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let parent_after = after
            .node_at(&parent_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let before_children = parent_before
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let after_children = parent_after
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let list_node = before
            .node_at(&list_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let item_index = usize::try_from(resolved.node_path[list_item_depth])
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        let list_content = list_node
            .content()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let item = list_content
            .child(item_index)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let extracted_count = item
            .content()
            .map(Fragment::child_count)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if list_content.child_count() == 1 {
            if let Some(handle) = self.prepared_elements.get(&list_path) {
                if self
                    .wrap_checkpoints
                    .get(&handle.insert_id)
                    .is_some_and(|checkpoint| checkpoint.document == *after)
                {
                    let checkpoint = self
                        .wrap_checkpoints
                        .remove(&handle.insert_id)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    self.targets = checkpoint.targets;
                    self.structural_parents = checkpoint.structural_parents;
                    self.actions = checkpoint.actions;
                    self.prepared_inserts = checkpoint.prepared_inserts;
                    self.prepared_elements = checkpoint.prepared_elements;
                    self.created_gap_shifts = checkpoint.created_gap_shifts;
                    self.pending_element_attrs = checkpoint.pending_element_attrs;
                    return Ok(());
                }
            }
        }
        let list_semantic_index = usize::try_from(list_index)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        if self.prepared_elements.contains_key(&parent_path) {
            if let Some(handle) = self.prepared_elements.get(&list_path).cloned() {
                let replacement_count = extracted_count
                    .checked_add(if list_content.child_count() == 1 {
                        0
                    } else if item_index == 0 || item_index + 1 == list_content.child_count() {
                        1
                    } else {
                        2
                    })
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                let replacement = after_children
                    .iter()
                    .skip(list_semantic_index)
                    .take(replacement_count)
                    .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
                    .collect::<Vec<_>>();
                if replacement.len() != replacement_count {
                    return Err(invalid_action_range(self.request_id, operation_index));
                }
                let mut batch =
                    prepare_xml_nodes(&replacement, limits, parent_path.len().saturating_add(2))
                        .map_err(|error| {
                            map_prepared_node_error(self.request_id, operation_index, error)
                        })?;
                let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
                let insert_id = handle.insert_id;
                self.replace_prepared_element_with_children(operation_index, &handle, batch.nodes)?;
                self.prepared_elements
                    .retain(|_, candidate| candidate.insert_id != insert_id);
                self.targets.retain(
                    |target| !matches!(&target.kind, ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id),
                );
                self.wrap_checkpoints.remove(&insert_id);
                self.charge_operation_work(
                    operation_index,
                    batch
                        .work
                        .checked_add(empty_work)
                        .and_then(|work| work.checked_add(replacement_count))
                        .ok_or_else(|| {
                            work_overflow(self.request_id, operation_index, self.action_limit)
                        })?,
                )?;
                self.register_prepared_insert_state(operation_index, insert_id, after)?;
                return Ok(());
            }
        }
        let parent_target = self
            .structural_parents
            .get(&parent_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let list_offset = before_children
            .iter()
            .take(list_semantic_index)
            .try_fold(0u32, |offset, child| offset.checked_add(child.node_size()))
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let storage_index = match self.current_storage_insertion(
            before_children.iter(),
            &parent_target.storage_children,
            list_offset,
        ) {
            Some(StorageInsertion::Boundary(index)) => index,
            _ => return Err(invalid_action_range(self.request_id, operation_index)),
        };
        if let Some(handle) = self.prepared_elements.get(&list_path).cloned() {
            let replacement_count = extracted_count
                .checked_add(if list_content.child_count() == 1 {
                    0
                } else if item_index == 0 || item_index + 1 == list_content.child_count() {
                    1
                } else {
                    2
                })
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let replacement = after_children
                .iter()
                .skip(list_semantic_index)
                .take(replacement_count)
                .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
                .collect::<Vec<_>>();
            if replacement.len() != replacement_count {
                return Err(invalid_action_range(self.request_id, operation_index));
            }
            let mut batch =
                prepare_xml_nodes(&replacement, limits, parent_path.len().saturating_add(2))
                    .map_err(|error| {
                        map_prepared_node_error(self.request_id, operation_index, error)
                    })?;
            let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
            let insert_id = handle.insert_id;
            self.replace_prepared_element_with_children(operation_index, &handle, batch.nodes)?;
            self.prepared_elements
                .retain(|_, candidate| candidate.insert_id != insert_id);
            self.targets.retain(
                |target| !matches!(&target.kind, ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id),
            );
            self.apply_virtual_structural_splices(
                operation_index,
                after,
                &[VirtualStructuralSplice {
                    parent_path: parent_path.clone(),
                    semantic_index: list_index,
                    semantic_delete: 1,
                    semantic_insert: u32::try_from(replacement_count)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                    storage_index,
                    storage_delete: 1,
                }],
                Some(insert_id),
            )?;
            self.wrap_checkpoints.remove(&insert_id);
            self.charge_operation_work(
                operation_index,
                batch
                    .work
                    .checked_add(empty_work)
                    .and_then(|work| work.checked_add(replacement_count))
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?,
            )?;
            self.register_prepared_insert_state(operation_index, insert_id, after)?;
            return Ok(());
        }
        if list_content.child_count() > 1 {
            let list_target = self
                .structural_parents
                .get(&list_path)
                .cloned()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let (delete_index, delete_count, copy_start, copy_count, insert_index) =
                if item_index == 0 {
                    (
                        0usize,
                        1usize,
                        list_semantic_index,
                        extracted_count,
                        storage_index,
                    )
                } else if item_index + 1 == list_content.child_count() {
                    (
                        item_index,
                        1,
                        list_semantic_index.checked_add(1).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?,
                        extracted_count,
                        storage_index.checked_add(1).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?,
                    )
                } else {
                    let left_count = item_index;
                    let right_count = list_content
                        .child_count()
                        .checked_sub(item_index)
                        .and_then(|count| count.checked_sub(1))
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let copied_count = extracted_count
                        .checked_add(1)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    if right_count > left_count {
                        // Keep the larger stationary right side. Ties deliberately
                        // fall through to the left side for deterministic plans.
                        (
                            0,
                            item_index.checked_add(1).ok_or_else(|| {
                                invalid_action_range(self.request_id, operation_index)
                            })?,
                            list_semantic_index,
                            copied_count,
                            storage_index,
                        )
                    } else {
                        (
                            item_index,
                            list_content
                                .child_count()
                                .checked_sub(item_index)
                                .ok_or_else(|| {
                                    invalid_action_range(self.request_id, operation_index)
                                })?,
                            list_semantic_index.checked_add(1).ok_or_else(|| {
                                invalid_action_range(self.request_id, operation_index)
                            })?,
                            copied_count,
                            storage_index.checked_add(1).ok_or_else(|| {
                                invalid_action_range(self.request_id, operation_index)
                            })?,
                        )
                    }
                };
            let copied = after_children
                .iter()
                .skip(copy_start)
                .take(copy_count)
                .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
                .collect::<Vec<_>>();
            if copied.len() != copy_count {
                return Err(invalid_action_range(self.request_id, operation_index));
            }
            let mut batch = prepare_xml_nodes(&copied, limits, parent_path.len().saturating_add(2))
                .map_err(|error| {
                    map_prepared_node_error(self.request_id, operation_index, error)
                })?;
            let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
            for child in &mut batch.nodes {
                child.index = insert_index
                    .checked_add(child.index)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            self.push_action(YrsMutationAction::DeleteXmlChildren {
                parent: list_target.parent.clone(),
                child_index: u32::try_from(delete_index)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                child_count: u32::try_from(delete_count)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                signature: list_target.signature.clone(),
                operation_index,
            });
            let semantic_delete = u32::try_from(delete_count)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let semantic_insert = u32::try_from(copy_count)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            self.apply_virtual_structural_splices(
                operation_index,
                after,
                &[
                    VirtualStructuralSplice {
                        parent_path: list_path.clone(),
                        semantic_index: u32::try_from(delete_index)
                            .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                        semantic_delete,
                        semantic_insert: 0,
                        storage_index: u32::try_from(delete_index)
                            .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                        storage_delete: semantic_delete,
                    },
                    VirtualStructuralSplice {
                        parent_path: parent_path.clone(),
                        semantic_index: u32::try_from(copy_start)
                            .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                        semantic_delete: 0,
                        semantic_insert,
                        storage_index: insert_index,
                        storage_delete: 0,
                    },
                ],
                None,
            )?;
            if !batch.nodes.is_empty() {
                let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
                    parent: parent_target.parent,
                    child_index: insert_index,
                    nodes: batch.nodes,
                    signature: parent_target.signature.clone(),
                    operation_index,
                    semantic_parent_path: parent_path.clone(),
                    first_semantic_index: u32::try_from(copy_start)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                });
                self.register_prepared_insert_state(operation_index, insert_id, after)?;
            }
            self.charge_operation_work(
                operation_index,
                list_target
                    .signature
                    .children
                    .len()
                    .checked_add(parent_target.signature.children.len())
                    .and_then(|work| work.checked_add(batch.work))
                    .and_then(|work| work.checked_add(empty_work))
                    .and_then(|work| work.checked_add(copy_count))
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?,
            )?;
            return Ok(());
        }
        let extracted = after_children
            .iter()
            .skip(list_semantic_index)
            .take(extracted_count)
            .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
            .collect::<Vec<_>>();
        if extracted.len() != extracted_count {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        let mut batch = prepare_xml_nodes(&extracted, limits, parent_path.len().saturating_add(2))
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let empty_work = materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
        for child in &mut batch.nodes {
            child.index = storage_index
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        self.push_action(YrsMutationAction::DeleteXmlChildren {
            parent: parent_target.parent.clone(),
            child_index: storage_index,
            child_count: 1,
            signature: parent_target.signature.clone(),
            operation_index,
        });
        self.apply_virtual_structural_splices(
            operation_index,
            after,
            &[VirtualStructuralSplice {
                parent_path: parent_path.clone(),
                semantic_index: list_index,
                semantic_delete: 1,
                semantic_insert: u32::try_from(extracted_count)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                storage_index,
                storage_delete: 1,
            }],
            None,
        )?;
        if !batch.nodes.is_empty() {
            let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
                parent: parent_target.parent,
                child_index: storage_index,
                nodes: batch.nodes,
                signature: parent_target.signature.clone(),
                operation_index,
                semantic_parent_path: parent_path.clone(),
                first_semantic_index: list_index,
            });
            self.register_prepared_insert_state(operation_index, insert_id, after)?;
        }
        self.charge_operation_work(
            operation_index,
            parent_target
                .signature
                .children
                .len()
                .checked_add(batch.work)
                .and_then(|work| work.checked_add(empty_work))
                .and_then(|work| work.checked_add(extracted_count))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        Ok(())
    }

    fn apply_virtual_structural_splices(
        &mut self,
        operation_index: usize,
        after: &Document,
        splices: &[VirtualStructuralSplice],
        preserved_prepared_insert: Option<usize>,
    ) -> OperationResult<()> {
        let traversal_work = splices
            .len()
            .checked_add(self.structural_parents.len())
            .and_then(|work| work.checked_add(self.prepared_elements.len()))
            .and_then(|work| work.checked_add(self.prepared_inserts.len()))
            .and_then(|work| work.checked_add(self.targets.len()))
            .and_then(|work| work.checked_add(self.actions.len()))
            .and_then(|work| work.checked_add(self.pending_element_attrs.len()))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, traversal_work)?;

        let mut deleted_branch_ids = HashSet::new();
        let mut deleted_prepared_insert_ids = HashSet::new();
        for splice in splices {
            let storage_start = usize::try_from(splice.storage_index)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let storage_delete = usize::try_from(splice.storage_delete)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let storage_end = storage_start
                .checked_add(storage_delete)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let parent = self
                .structural_parents
                .get_mut(&splice.parent_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if storage_end > parent.storage_children.len() {
                return Err(invalid_action_range(self.request_id, operation_index));
            }
            for child in &parent.storage_children[storage_start..storage_end] {
                match child {
                    StorageChildKind::Text { target, .. } => {
                        deleted_branch_ids.insert(AsRef::<Branch>::as_ref(target).id());
                    }
                    StorageChildKind::Element { target, .. } => {
                        deleted_branch_ids.insert(AsRef::<Branch>::as_ref(target).id());
                    }
                    StorageChildKind::PreparedElement => {}
                }
            }
            parent.storage_children.splice(
                storage_start..storage_end,
                (0..splice.semantic_insert).map(|_| StorageChildKind::PreparedElement),
            );

            let mut parents = HashMap::with_capacity(self.structural_parents.len());
            for (path, parent) in std::mem::take(&mut self.structural_parents) {
                match remap_semantic_path(&path, splice)? {
                    Some(path) => {
                        parents.insert(path, parent);
                    }
                    None => {
                        if let XmlParentRef::Element(element) = &parent.parent {
                            deleted_branch_ids.insert(AsRef::<Branch>::as_ref(element).id());
                        }
                        for child in &parent.storage_children {
                            match child {
                                StorageChildKind::Text { target, .. } => {
                                    deleted_branch_ids.insert(AsRef::<Branch>::as_ref(target).id());
                                }
                                StorageChildKind::Element { target, .. } => {
                                    deleted_branch_ids.insert(AsRef::<Branch>::as_ref(target).id());
                                }
                                StorageChildKind::PreparedElement => {}
                            }
                        }
                    }
                }
            }
            self.structural_parents = parents;

            let mut prepared_elements = HashMap::with_capacity(self.prepared_elements.len());
            for (path, handle) in std::mem::take(&mut self.prepared_elements) {
                match remap_semantic_path(&path, splice)? {
                    Some(path) => {
                        prepared_elements.insert(path, handle);
                    }
                    None => {
                        if Some(handle.insert_id) != preserved_prepared_insert {
                            deleted_prepared_insert_ids.insert(handle.insert_id);
                        }
                    }
                }
            }
            self.prepared_elements = prepared_elements;

            for (insert_id, pending) in self.prepared_inserts.iter_mut().enumerate() {
                let Some(pending) = pending.as_mut() else {
                    continue;
                };
                if Some(insert_id) == preserved_prepared_insert {
                    if pending.semantic_parent_path != splice.parent_path {
                        match remap_semantic_path(&pending.semantic_parent_path, splice)? {
                            Some(path) => pending.semantic_parent_path = path,
                            None => {
                                deleted_prepared_insert_ids.insert(insert_id);
                            }
                        }
                    }
                    continue;
                }
                if pending.semantic_parent_path == splice.parent_path {
                    let batch_len = u32::try_from(pending.nodes.len())
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
                    let batch_end = pending
                        .first_semantic_index
                        .checked_add(batch_len)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let delete_end = splice
                        .semantic_index
                        .checked_add(splice.semantic_delete)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    if pending.first_semantic_index < delete_end
                        && batch_end > splice.semantic_index
                    {
                        deleted_prepared_insert_ids.insert(insert_id);
                    } else if pending.first_semantic_index >= delete_end {
                        pending.first_semantic_index = shift_semantic_index(
                            pending.first_semantic_index,
                            splice.semantic_delete,
                            splice.semantic_insert,
                        )?;
                    }
                } else {
                    match remap_semantic_path(&pending.semantic_parent_path, splice)? {
                        Some(path) => pending.semantic_parent_path = path,
                        None => {
                            deleted_prepared_insert_ids.insert(insert_id);
                        }
                    }
                }
            }
        }

        self.pending_element_attrs
            .retain(|id, _| !deleted_branch_ids.contains(id));
        for target in &mut self.targets {
            let deleted = match &target.kind {
                ResolvedTargetKind::Existing { target, .. } => {
                    deleted_branch_ids.contains(&AsRef::<Branch>::as_ref(target).id())
                }
                ResolvedTargetKind::Missing { parent, .. } => {
                    deleted_branch_ids.contains(&AsRef::<Branch>::as_ref(parent).id())
                }
                ResolvedTargetKind::Prepared { handle } => {
                    deleted_prepared_insert_ids.contains(&handle.insert_id)
                }
            };
            if deleted {
                for slot in std::mem::take(&mut target.action_slots) {
                    self.actions[slot] = ActionSlot::Tombstone;
                }
            }
        }
        self.targets.retain(|target| match &target.kind {
            ResolvedTargetKind::Existing { target, .. } => {
                !deleted_branch_ids.contains(&AsRef::<Branch>::as_ref(target).id())
            }
            ResolvedTargetKind::Missing { parent, .. } => {
                !deleted_branch_ids.contains(&AsRef::<Branch>::as_ref(parent).id())
            }
            ResolvedTargetKind::Prepared { handle } => {
                !deleted_prepared_insert_ids.contains(&handle.insert_id)
            }
        });
        for slot in &mut self.actions {
            let deleted = match slot {
                ActionSlot::PreparedInsert(insert_id) => {
                    deleted_prepared_insert_ids.contains(insert_id)
                        || self
                            .prepared_inserts
                            .get(*insert_id)
                            .and_then(Option::as_ref)
                            .is_some_and(|insert| match &insert.parent {
                                XmlParentRef::Element(parent) => deleted_branch_ids
                                    .contains(&AsRef::<Branch>::as_ref(parent).id()),
                                XmlParentRef::Fragment(_) => false,
                            })
                }
                ActionSlot::Concrete(action) => {
                    mutation_action_touches_branches(action, &deleted_branch_ids)
                }
                ActionSlot::Tombstone => false,
            };
            if deleted {
                if let ActionSlot::PreparedInsert(insert_id) = slot {
                    self.prepared_inserts[*insert_id] = None;
                }
                *slot = ActionSlot::Tombstone;
            }
        }
        for insert_id in deleted_prepared_insert_ids {
            if let Some(insert) = self.prepared_inserts.get_mut(insert_id) {
                *insert = None;
            }
        }
        self.rebuild_target_layout(operation_index, after)
    }

    fn register_prepared_insert_state(
        &mut self,
        operation_index: usize,
        insert_id: usize,
        after: &Document,
    ) -> OperationResult<()> {
        let insert = self
            .prepared_inserts
            .get(insert_id)
            .and_then(Option::as_ref)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let nodes = insert.nodes.clone();
        let semantic_parent_path = insert.semantic_parent_path.clone();
        let first_semantic_index = insert.first_semantic_index;
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
            &semantic_parent_path,
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
                    "duplicate prepared insert element semantic path",
                ));
            }
        }
        for (_, handle, runs) in texts {
            let text = prepared_runs_text(&runs);
            let scalar_len = u32::try_from(text.chars().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            self.targets.push(ResolvedText {
                kind: ResolvedTargetKind::Prepared { handle },
                gap_before: 0,
                text,
                scalar_len,
                base_runs: Vec::new(),
                current_runs: runs,
                action_slots: Vec::new(),
            });
        }
        self.rebuild_target_layout(operation_index, after)
    }

    fn rebuild_target_layout(
        &mut self,
        operation_index: usize,
        after: &Document,
    ) -> OperationResult<()> {
        let (content_positions, mut layout_work) =
            collect_document_content_positions(self.request_id, after.root())?;
        let mut existing_starts = HashMap::new();
        let mut missing_starts = HashMap::<BranchID, Vec<u32>>::new();
        for (path, parent) in &self.structural_parents {
            let node = after
                .node_at(path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let content = node
                .content()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            layout_work = layout_work
                .checked_add(path.len())
                .and_then(|work| work.checked_add(parent.storage_children.len()))
                .and_then(|work| work.checked_add(content.child_count()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?;
            let mut cursor = content_positions
                .get(path)
                .copied()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let mut semantic_index = 0usize;
            let mut semantic_text_offset = 0u32;
            let mut boundaries = Vec::with_capacity(parent.storage_children.len() + 1);
            for storage in &parent.storage_children {
                boundaries.push(cursor);
                match storage {
                    StorageChildKind::Text {
                        scalar_len, target, ..
                    } => {
                        existing_starts.insert(AsRef::<Branch>::as_ref(target).id(), cursor);
                        let mut remaining = *scalar_len;
                        while remaining > 0 {
                            let child = content.child(semantic_index).ok_or_else(|| {
                                invalid_action_range(self.request_id, operation_index)
                            })?;
                            if !child.is_text() || semantic_text_offset >= child.node_size() {
                                return Err(invalid_action_range(self.request_id, operation_index));
                            }
                            let available = child.node_size() - semantic_text_offset;
                            let consumed = remaining.min(available);
                            cursor = cursor.checked_add(consumed).ok_or_else(|| {
                                invalid_action_range(self.request_id, operation_index)
                            })?;
                            remaining -= consumed;
                            semantic_text_offset += consumed;
                            if semantic_text_offset == child.node_size() {
                                semantic_index += 1;
                                semantic_text_offset = 0;
                            }
                        }
                    }
                    StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => {
                        if semantic_text_offset != 0 {
                            return Err(invalid_action_range(self.request_id, operation_index));
                        }
                        let child = content.child(semantic_index).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                        if child.is_text() {
                            return Err(invalid_action_range(self.request_id, operation_index));
                        }
                        cursor = cursor.checked_add(child.node_size()).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                        semantic_index += 1;
                    }
                }
            }
            if semantic_text_offset != 0 || semantic_index != content.child_count() {
                return Err(invalid_action_range(self.request_id, operation_index));
            }
            boundaries.push(cursor);
            missing_starts.insert(parent.parent.id(), boundaries);
        }

        let mut prepared_starts = HashMap::new();
        for (insert_id, insert) in self.prepared_inserts.iter().enumerate() {
            let Some(insert) = insert else {
                continue;
            };
            layout_work = layout_work
                .checked_add(prepared_clone_work(&insert.nodes).ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?)
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?;
            let mut elements = Vec::new();
            let mut texts = Vec::new();
            collect_prepared_child_handles(
                insert_id,
                &insert.nodes,
                &insert.semantic_parent_path,
                insert.first_semantic_index,
                Some(after),
                &mut elements,
                &mut texts,
            )?;
            for (path, handle, _) in texts {
                let start = first_text_doc_position(after.root(), &path)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                prepared_starts.insert(handle, start);
            }
        }

        let target_count = self.targets.len();
        let mut indexed = Vec::with_capacity(target_count);
        for (ordinal, target) in std::mem::take(&mut self.targets).into_iter().enumerate() {
            let start = match &target.kind {
                ResolvedTargetKind::Existing { target, .. } => existing_starts
                    .get(&AsRef::<Branch>::as_ref(target).id())
                    .copied(),
                ResolvedTargetKind::Missing {
                    parent,
                    child_index,
                    ..
                } => missing_starts
                    .get(&AsRef::<Branch>::as_ref(parent).id())
                    .and_then(|boundaries| boundaries.get(*child_index as usize))
                    .copied(),
                ResolvedTargetKind::Prepared { handle } => prepared_starts.get(handle).copied(),
            }
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            indexed.push((start, ordinal, target));
        }
        layout_work = layout_work
            .checked_add(target_count)
            .and_then(|work| {
                work.checked_add(target_count.checked_mul(binary_partition_work(target_count))?)
            })
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, layout_work)?;
        indexed.sort_by_key(|(start, ordinal, _)| (*start, *ordinal));
        let mut previous_end = 0u32;
        for (start, _, mut target) in indexed {
            target.gap_before = start
                .checked_sub(previous_end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            previous_end = start
                .checked_add(target.scalar_len)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets.push(target);
        }
        Ok(())
    }
}
