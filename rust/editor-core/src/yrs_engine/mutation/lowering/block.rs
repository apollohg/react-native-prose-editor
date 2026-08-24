impl MutationCompiler {
    fn normalize_existing_target_for_split(
        &mut self,
        operation_index: usize,
        target_index: usize,
        final_cut_utf16: u32,
        desired_left: Vec<PreparedTextRun>,
    ) -> OperationResult<(u32, u32)> {
        let (target, signature) = match &self.targets[target_index].kind {
            ResolvedTargetKind::Existing { target, signature } => {
                (target.clone(), signature.clone())
            }
            _ => return Err(invalid_action_range(self.request_id, operation_index)),
        };
        let old_slots = std::mem::take(&mut self.targets[target_index].action_slots);
        let mut cut = final_cut_utf16;
        let mut kept_reversed = Vec::new();
        for &slot in old_slots.iter().rev() {
            let action = match self.actions.get(slot) {
                Some(ActionSlot::Concrete(action)) => (**action).clone(),
                _ => return Err(invalid_action_range(self.request_id, operation_index)),
            };
            let kept = match action {
                YrsMutationAction::InsertText {
                    target,
                    index_utf16,
                    text,
                    len_utf16,
                    attrs,
                    signature,
                    operation_index,
                } if index_utf16 < cut => {
                    let retained = len_utf16.min(cut - index_utf16);
                    let byte = utf16_byte_index(&text, retained)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    cut = cut
                        .checked_sub(retained)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    Some(YrsMutationAction::InsertText {
                        target,
                        index_utf16,
                        text: text[..byte].to_owned(),
                        len_utf16: retained,
                        attrs,
                        signature,
                        operation_index,
                    })
                }
                YrsMutationAction::InsertText { .. } => None,
                YrsMutationAction::DeleteText {
                    target,
                    index_utf16,
                    len_utf16,
                    signature,
                    operation_index,
                } if index_utf16 < cut => {
                    cut = cut
                        .checked_add(len_utf16)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    Some(YrsMutationAction::DeleteText {
                        target,
                        index_utf16,
                        len_utf16,
                        signature,
                        operation_index,
                    })
                }
                YrsMutationAction::DeleteText { .. } => None,
                YrsMutationAction::FormatText {
                    target,
                    index_utf16,
                    len_utf16,
                    attrs,
                    signature,
                    operation_index,
                } if index_utf16 < cut => Some(YrsMutationAction::FormatText {
                    target,
                    index_utf16,
                    len_utf16: len_utf16.min(cut - index_utf16),
                    attrs,
                    signature,
                    operation_index,
                }),
                YrsMutationAction::FormatText { .. } => None,
                _ => {
                    return Err(OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "split target action history contains a non-text action",
                    ))
                }
            };
            self.actions[slot] = ActionSlot::Tombstone;
            if let Some(action) = kept {
                kept_reversed.push(action);
            }
        }
        kept_reversed.reverse();
        let mut replayed = self.targets[target_index].base_runs.clone();
        for action in &kept_reversed {
            match action {
                YrsMutationAction::InsertText {
                    index_utf16,
                    text,
                    attrs,
                    ..
                } => insert_prepared_run(&mut replayed, *index_utf16, text, attrs.clone()),
                YrsMutationAction::DeleteText {
                    index_utf16,
                    len_utf16,
                    ..
                } => delete_prepared_run_range(&mut replayed, *index_utf16, *len_utf16),
                YrsMutationAction::FormatText {
                    index_utf16,
                    len_utf16,
                    attrs,
                    ..
                } => format_prepared_run_range(&mut replayed, *index_utf16, *len_utf16, attrs),
                _ => None,
            }
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let left_len_utf16 =
            prepared_runs_utf16_len(self.request_id, operation_index, &desired_left)?;
        let (physical_left, _) = split_runs_utf16(&replayed, left_len_utf16)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if physical_left != desired_left {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "split action normalization did not reproduce the retained text runs",
            ));
        }
        for action in kept_reversed {
            let slot = self.push_action(action);
            self.targets[target_index].action_slots.push(slot);
        }
        let physical_len_utf16 =
            prepared_runs_utf16_len(self.request_id, operation_index, &replayed)?;
        let delete_len_utf16 = physical_len_utf16
            .checked_sub(left_len_utf16)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if delete_len_utf16 > 0 {
            let slot = self.push_action(YrsMutationAction::DeleteText {
                target,
                index_utf16: left_len_utf16,
                len_utf16: delete_len_utf16,
                signature,
                operation_index,
            });
            self.targets[target_index].action_slots.push(slot);
        }
        Ok((left_len_utf16, delete_len_utf16))
    }

    fn register_prepared_block(
        &mut self,
        operation_index: usize,
        insert_id: usize,
        semantic_path: Vec<u32>,
        after_target: usize,
        first_gap_before: u32,
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
        collect_prepared_handles(
            insert_id,
            &nodes,
            &semantic_path,
            after,
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
        for (insertion, (ordinal, (handle, runs))) in
            (after_target + 1..).zip(texts.into_iter().enumerate())
        {
            let text = prepared_runs_text(&runs);
            let scalar_len = u32::try_from(text.chars().count())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            self.targets.insert(
                insertion,
                ResolvedText {
                    kind: ResolvedTargetKind::Prepared { handle },
                    gap_before: if ordinal == 0 { first_gap_before } else { 0 },
                    text,
                    scalar_len,
                    base_runs: Vec::new(),
                    current_runs: runs,
                    action_slots: Vec::new(),
                },
            );
        }
        Ok(())
    }

    pub(crate) fn split_block(
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
        let list_item_path = (resolved.node_path.len() >= 2)
            .then(|| resolved.node_path[..resolved.node_path.len() - 1].to_vec())
            .filter(|path| {
                before.node_at(path).is_some_and(|node| {
                    schema
                        .node(node.node_type())
                        .is_some_and(|spec| matches!(spec.role, NodeRole::ListItem))
                })
            });
        let block_path = resolved.node_path.iter().copied().collect::<Vec<_>>();
        let block_target = self
            .structural_parents
            .get(&block_path)
            .cloned()
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "split block has no tracked Yrs element",
                )
            })?;
        let block = resolved.parent(before);
        let block_content = block.content().ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "split block has no content",
            )
        })?;
        if let Some(StorageInsertion::Boundary(move_index)) = self.current_storage_insertion(
            block_content.iter(),
            &block_target.storage_children,
            resolved.parent_offset,
        ) {
            if move_index == 0 {
                if let Some(item_path) = list_item_path.as_ref() {
                    let list_path = item_path[..item_path.len() - 1].to_vec();
                    let item_index = *item_path
                        .last()
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let inserted_node = after.node_at(item_path).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "list-start split preview has no inserted empty item",
                        )
                    })?;
                    let json = crate::serialize::node_to_prosemirror_json(inserted_node, schema);
                    let mut batch = prepare_xml_nodes(
                        std::slice::from_ref(&json),
                        limits,
                        item_path.len().saturating_add(1),
                    )
                    .map_err(|error| {
                        map_prepared_node_error(self.request_id, operation_index, error)
                    })?;
                    for child in &mut batch.nodes {
                        child.index = item_index.checked_add(child.index).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                    }
                    let insertion_parent = self
                        .structural_parents
                        .get(&list_path)
                        .cloned()
                        .ok_or_else(|| {
                            OperationError::engine_invariant_failed(
                                self.request_id,
                                Some(operation_index),
                                "list-start split has no tracked list parent",
                            )
                        })?;
                    self.apply_virtual_structural_splices(
                        operation_index,
                        after,
                        &[VirtualStructuralSplice {
                            parent_path: list_path.clone(),
                            semantic_index: item_index,
                            semantic_delete: 0,
                            semantic_insert: 1,
                            storage_index: item_index,
                            storage_delete: 0,
                        }],
                        None,
                    )?;
                    let prepared_id = self.queue_prepared_insert(PendingPreparedInsert {
                        parent: insertion_parent.parent,
                        child_index: item_index,
                        nodes: batch.nodes,
                        signature: insertion_parent.signature,
                        operation_index,
                        semantic_parent_path: list_path,
                        first_semantic_index: item_index,
                    });
                    self.register_prepared_insert_state(operation_index, prepared_id, after)?;
                    self.charge_operation_work(
                        operation_index,
                        batch
                            .work
                            .checked_add(block_target.signature.children.len())
                            .ok_or_else(|| {
                                work_overflow(self.request_id, operation_index, self.action_limit)
                            })?,
                    )?;
                    return Ok(());
                }
            }
            let moved_count = u32::try_from(block_target.storage_children.len())
                .ok()
                .and_then(|len| len.checked_sub(move_index))
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let moved_start = usize::try_from(move_index)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            let suffix_is_empty = block_target.storage_children[moved_start..]
                .iter()
                .all(|child| matches!(child, StorageChildKind::Text { scalar_len: 0, .. }));
            if moved_count == 0 || suffix_is_empty {
                // Zero-length Yrs text children do not make a semantic suffix.
                return self.finish_split_with_created_right(
                    operation_index,
                    after,
                    list_item_path,
                    &block_path,
                    block_target.signature.children.len(),
                    0,
                    0,
                    schema,
                    limits,
                );
            }
            self.push_action(YrsMutationAction::DeleteXmlChildren {
                parent: block_target.parent.clone(),
                child_index: move_index,
                child_count: moved_count,
                signature: block_target.signature.clone(),
                operation_index,
            });
            let moved_ids = block_target.storage_children[moved_start..]
                .iter()
                .filter_map(|child| match child {
                    StorageChildKind::Text { target, .. } => {
                        Some(AsRef::<Branch>::as_ref(target).id())
                    }
                    StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => None,
                })
                .collect::<HashSet<_>>();
            let retained_text_id = block_target.storage_children[..moved_start]
                .iter()
                .rev()
                .find_map(|child| match child {
                    StorageChildKind::Text { target, .. } => {
                        Some(AsRef::<Branch>::as_ref(target).id())
                    }
                    StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => None,
                });
            if retained_text_id.is_none() {
                let first_moved_text_id =
                    block_target
                        .storage_children
                        .get(moved_start)
                        .and_then(|child| match child {
                            StorageChildKind::Text { target, .. } => {
                                Some(AsRef::<Branch>::as_ref(target).id())
                            }
                            StorageChildKind::Element { .. }
                            | StorageChildKind::PreparedElement => None,
                        });
                if let (Some(first_moved_text_id), XmlParentRef::Element(parent)) =
                    (first_moved_text_id, &block_target.parent)
                {
                    if let Some(candidate) = self.targets.iter_mut().find(|candidate| {
                        matches!(
                            &candidate.kind,
                            ResolvedTargetKind::Existing { target, .. }
                                if AsRef::<Branch>::as_ref(target).id() == first_moved_text_id
                        )
                    }) {
                        for slot in std::mem::take(&mut candidate.action_slots) {
                            self.actions[slot] = ActionSlot::Tombstone;
                        }
                        candidate.kind = ResolvedTargetKind::Missing {
                            parent: parent.clone(),
                            child_index: move_index,
                            signature: ParentSignature {
                                parent: block_target.signature.parent.clone(),
                                tag: parent.tag().clone(),
                                path: block_target.signature.path.clone(),
                                child_count: u32::try_from(block_target.signature.children.len())
                                    .map_err(|_| {
                                    invalid_action_range(self.request_id, operation_index)
                                })?,
                                initial_child_index: move_index,
                                left_neighbor: move_index
                                    .checked_sub(1)
                                    .and_then(|index| {
                                        block_target.signature.children.get(index as usize)
                                    })
                                    .cloned(),
                                right_neighbor: block_target
                                    .signature
                                    .children
                                    .get(move_index as usize)
                                    .cloned(),
                            },
                            create_action: None,
                        };
                        candidate.text.clear();
                        candidate.scalar_len = 0;
                        candidate.base_runs.clear();
                        candidate.current_runs.clear();
                    }
                }
            }
            self.targets.retain(|candidate| {
                !matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if moved_ids.contains(&AsRef::<Branch>::as_ref(target).id())
                )
            });
            let after_target = if let Some(retained_text_id) = retained_text_id {
                self.targets.iter().position(|candidate| {
                    matches!(
                        &candidate.kind,
                        ResolvedTargetKind::Existing { target, .. }
                            if AsRef::<Branch>::as_ref(target).id() == retained_text_id
                    )
                })
            } else {
                let block_parent_id = match &block_target.parent {
                    XmlParentRef::Element(element) => AsRef::<Branch>::as_ref(element).id(),
                    XmlParentRef::Fragment(_) => {
                        return Err(invalid_action_range(self.request_id, operation_index));
                    }
                };
                self.targets.iter().position(|candidate| {
                    matches!(
                        &candidate.kind,
                        ResolvedTargetKind::Missing {
                            parent,
                            child_index,
                            ..
                        }
                            if AsRef::<Branch>::as_ref(parent).id() == block_parent_id
                                && *child_index == move_index
                    )
                })
            }
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let block_index = *block_path
                .last()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let insertion_index = block_index
                .checked_add(1)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let insertion_parent_path = block_path[..block_path.len() - 1].to_vec();
            let mut inserted_path = insertion_parent_path.clone();
            inserted_path.push(insertion_index);
            let right_block = after
                .node_at(&inserted_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let json = crate::serialize::node_to_prosemirror_json(right_block, schema);
            let mut batch = prepare_xml_nodes(
                std::slice::from_ref(&json),
                limits,
                inserted_path.len().saturating_add(1),
            )
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
            for child in &mut batch.nodes {
                child.index = insertion_index
                    .checked_add(child.index)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            let insertion_parent = self
                .structural_parents
                .get(&insertion_parent_path)
                .cloned()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let prepared_id = self.queue_prepared_insert(PendingPreparedInsert {
                parent: insertion_parent.parent,
                child_index: insertion_index,
                nodes: batch.nodes,
                signature: insertion_parent.signature,
                operation_index,
                semantic_parent_path: insertion_parent_path,
                first_semantic_index: insertion_index,
            });
            let previous_end = self
                .positions()?
                .get(after_target)
                .map(|(_, end)| *end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let created_start = first_text_doc_position(after.root(), &inserted_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let first_gap_before = created_start
                .checked_sub(previous_end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.register_prepared_block(
                operation_index,
                prepared_id,
                inserted_path,
                after_target,
                first_gap_before,
                after,
            )?;
            self.charge_operation_work(
                operation_index,
                block_target
                    .signature
                    .children
                    .len()
                    .checked_add(batch.work)
                    .and_then(|work| work.checked_add(usize::try_from(moved_count).ok()?))
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?,
            )?;
            return Ok(());
        }
        let ResolvedInsertion {
            target_index,
            scalar_index,
        } = self.resolve_insertion(operation_index, position)?;
        let ResolvedTargetKind::Existing {
            target: virtual_target,
            signature: virtual_signature,
        } = &self.targets[target_index].kind
        else {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "split block must retain an existing XML text target",
            ));
        };
        let virtual_target_id = AsRef::<Branch>::as_ref(virtual_target).id();
        if scalar_index == 0 || scalar_index >= self.targets[target_index].scalar_len {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "split block currently requires a position inside one XML text child",
            ));
        }
        let tracked_in_block = block_target.storage_children.iter().any(|child| {
            matches!(
                child,
                StorageChildKind::Text { target, signature, .. }
                    if AsRef::<Branch>::as_ref(target).id()
                        == AsRef::<Branch>::as_ref(virtual_target).id()
                        && signature.target == virtual_signature.target
            )
        });
        if !tracked_in_block {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "split block virtual target does not match its structural parent",
            ));
        }
        let final_cut_utf16 = scalar_to_utf16(
            self.request_id,
            operation_index,
            &self.targets[target_index].text,
            scalar_index,
        )?;
        let (left_runs, _) =
            split_runs_utf16(&self.targets[target_index].current_runs, final_cut_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let (_, delete_len_utf16) = self.normalize_existing_target_for_split(
            operation_index,
            target_index,
            final_cut_utf16,
            left_runs.clone(),
        )?;
        let structural_block = self
            .structural_parents
            .get_mut(&block_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let StorageChildKind::Text {
            scalar_len, runs, ..
        } = structural_block
            .storage_children
            .iter_mut()
            .find(|child| {
                matches!(
                    child,
                    StorageChildKind::Text { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == virtual_target_id
                )
            })
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
        else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        *scalar_len = scalar_index;
        *runs = left_runs.clone();
        self.targets[target_index].text = prepared_runs_text(&left_runs);
        self.targets[target_index].scalar_len = scalar_index;
        self.targets[target_index].current_runs = left_runs.clone();

        let split_storage_index = block_target
            .storage_children
            .iter()
            .position(|child| {
                matches!(
                    child,
                    StorageChildKind::Text { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == virtual_target_id
                )
            })
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let suffix_storage_start = split_storage_index
            .checked_add(1)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let suffix_storage_count = block_target
            .storage_children
            .len()
            .checked_sub(suffix_storage_start)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if suffix_storage_count > 0 {
            self.push_action(YrsMutationAction::DeleteXmlChildren {
                parent: block_target.parent.clone(),
                child_index: u32::try_from(suffix_storage_start)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                child_count: u32::try_from(suffix_storage_count)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                signature: block_target.signature.clone(),
                operation_index,
            });
            let deleted_text_ids = block_target.storage_children[suffix_storage_start..]
                .iter()
                .filter_map(|child| match child {
                    StorageChildKind::Text { target, .. } => {
                        Some(AsRef::<Branch>::as_ref(target).id())
                    }
                    StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => None,
                })
                .collect::<HashSet<_>>();
            for candidate in &mut self.targets {
                let deleted = matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if deleted_text_ids.contains(&AsRef::<Branch>::as_ref(target).id())
                );
                if deleted {
                    for slot in std::mem::take(&mut candidate.action_slots) {
                        self.actions[slot] = ActionSlot::Tombstone;
                    }
                }
            }
            self.targets.retain(|candidate| {
                !matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if deleted_text_ids.contains(&AsRef::<Branch>::as_ref(target).id())
                )
            });
            self.structural_parents
                .get_mut(&block_path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
                .storage_children
                .truncate(suffix_storage_start);
        }

        self.finish_split_with_created_right(
            operation_index,
            after,
            list_item_path,
            &block_path,
            block_target.signature.children.len(),
            delete_len_utf16,
            suffix_storage_count,
            schema,
            limits,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_split_with_created_right(
        &mut self,
        operation_index: usize,
        after: &Document,
        list_item_path: Option<Vec<u32>>,
        block_path: &[u32],
        block_children_len: usize,
        delete_len_utf16: u32,
        suffix_storage_count: usize,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let (
            insertion_parent_path,
            insertion_index,
            inserted_path,
            moved_wrapper_work,
            moved_suffix,
        ) = if let Some(item_path) = list_item_path {
            let list_path = item_path[..item_path.len() - 1].to_vec();
            let item_index = *item_path
                .last()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let insertion_index = item_index
                .checked_add(1)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let block_index = *block_path
                .last()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let item_target = self
                .structural_parents
                .get(&item_path)
                .cloned()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let moved_start = block_index
                .checked_add(1)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let moved_count = u32::try_from(item_target.storage_children.len())
                .ok()
                .and_then(|len| len.checked_sub(moved_start))
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if moved_count > 0 {
                self.push_action(YrsMutationAction::DeleteXmlChildren {
                    parent: item_target.parent,
                    child_index: moved_start,
                    child_count: moved_count,
                    signature: item_target.signature,
                    operation_index,
                });
            }
            let mut inserted_path = list_path.clone();
            inserted_path.push(insertion_index);
            (
                list_path,
                insertion_index,
                inserted_path,
                usize::try_from(moved_count).unwrap_or(usize::MAX),
                Some((item_path, moved_start, moved_count)),
            )
        } else {
            let insertion_parent_path = block_path[..block_path.len() - 1].to_vec();
            let block_index = *block_path
                .last()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let insertion_index = block_index
                .checked_add(1)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let mut inserted_path = insertion_parent_path.clone();
            inserted_path.push(insertion_index);
            (
                insertion_parent_path,
                insertion_index,
                inserted_path,
                0,
                None,
            )
        };
        let inserted_node = after.node_at(&inserted_path).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "split preview has no created right wrapper",
            )
        })?;
        let json = crate::serialize::node_to_prosemirror_json(inserted_node, schema);
        let mut batch = prepare_xml_nodes(
            std::slice::from_ref(&json),
            limits,
            inserted_path.len().saturating_add(1),
        )
        .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        for child in &mut batch.nodes {
            child.index = insertion_index
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let insertion_parent = self
            .structural_parents
            .get(&insertion_parent_path)
            .cloned()
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "split insertion parent has no tracked Yrs branch",
                )
            })?;
        let mut splices = Vec::with_capacity(2);
        if let Some((item_path, moved_start, moved_count)) = moved_suffix {
            splices.push(VirtualStructuralSplice {
                parent_path: item_path,
                semantic_index: moved_start,
                semantic_delete: moved_count,
                semantic_insert: 0,
                storage_index: moved_start,
                storage_delete: moved_count,
            });
        }
        splices.push(VirtualStructuralSplice {
            parent_path: insertion_parent_path.clone(),
            semantic_index: insertion_index,
            semantic_delete: 0,
            semantic_insert: 1,
            storage_index: insertion_index,
            storage_delete: 0,
        });
        self.apply_virtual_structural_splices(operation_index, after, &splices, None)?;
        let prepared_id = self.queue_prepared_insert(PendingPreparedInsert {
            parent: insertion_parent.parent,
            child_index: insertion_index,
            nodes: batch.nodes,
            signature: insertion_parent.signature,
            operation_index,
            semantic_parent_path: insertion_parent_path.clone(),
            first_semantic_index: insertion_index,
        });
        self.register_prepared_insert_state(operation_index, prepared_id, after)?;
        let work = block_children_len
            .checked_add(batch.work)
            .and_then(|work| work.checked_add(usize::try_from(delete_len_utf16).ok()?))
            .and_then(|work| work.checked_add(suffix_storage_count))
            .and_then(|work| work.checked_add(moved_wrapper_work))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, work)?;
        Ok(())
    }

    pub(crate) fn join_blocks(
        &mut self,
        operation_index: usize,
        document: &Document,
        position: u32,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let resolved = document.resolve(position).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "at", message)
        })?;
        let parent_path = resolved.node_path.iter().copied().collect::<Vec<_>>();
        let parent = resolved.parent(document);
        let content = parent.content().ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "join parent has no content",
            )
        })?;
        let mut offset = 0u32;
        let mut boundary = None;
        for (index, child) in content.iter().enumerate() {
            if offset == resolved.parent_offset && index > 0 {
                boundary = Some(index);
                break;
            }
            offset = offset
                .checked_add(child.node_size())
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let boundary = boundary.ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "join position is not between adjacent blocks",
            )
        })?;
        let left_node = content
            .child(boundary - 1)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let right_node = content
            .child(boundary)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let mut left_path = parent_path.clone();
        left_path.push(
            u32::try_from(boundary - 1)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
        );
        let mut right_path = parent_path.clone();
        right_path.push(
            u32::try_from(boundary)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
        );
        let parent_target = self
            .structural_parents
            .get(&parent_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let left_target = self
            .structural_parents
            .get(&left_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let right_target = self
            .structural_parents
            .get(&right_path)
            .cloned()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let XmlParentRef::Element(left_element) = left_target.parent.clone() else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };

        let mut consumed_semantic = 0usize;
        let mut appended_right_target = None;
        if let (
            Some(StorageChildKind::Text {
                target: left_text,
                signature: left_signature,
                ..
            }),
            Some(StorageChildKind::Text {
                target: right_text, ..
            }),
        ) = (
            left_target.storage_children.last(),
            right_target.storage_children.first(),
        ) {
            let left_id = AsRef::<Branch>::as_ref(left_text).id();
            let right_id = AsRef::<Branch>::as_ref(right_text).id();
            let left_virtual = self
                .targets
                .iter()
                .position(|candidate| {
                    matches!(
                        &candidate.kind,
                        ResolvedTargetKind::Existing { target, .. }
                            if AsRef::<Branch>::as_ref(target).id() == left_id
                    )
                })
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let right_virtual = self
                .targets
                .iter()
                .position(|candidate| {
                    matches!(
                        &candidate.kind,
                        ResolvedTargetKind::Existing { target, .. }
                            if AsRef::<Branch>::as_ref(target).id() == right_id
                    )
                })
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let mut index_utf16 = prepared_runs_utf16_len(
                self.request_id,
                operation_index,
                &self.targets[left_virtual].current_runs,
            )?;
            let effective_right_runs = self.targets[right_virtual].current_runs.clone();
            let effective_right_scalars = self.targets[right_virtual].scalar_len;
            for slot in std::mem::take(&mut self.targets[right_virtual].action_slots) {
                self.actions[slot] = ActionSlot::Tombstone;
            }
            for run in &effective_right_runs {
                let len_utf16 = u32::try_from(run.text.encode_utf16().count())
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
                let slot = self.push_action(YrsMutationAction::InsertText {
                    target: left_text.clone(),
                    index_utf16,
                    text: run.text.clone(),
                    len_utf16,
                    attrs: run.attrs.clone(),
                    signature: left_signature.clone(),
                    operation_index,
                });
                self.targets[left_virtual].action_slots.push(slot);
                index_utf16 = index_utf16
                    .checked_add(len_utf16)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            let mut consumed = 0u32;
            let right_content = right_node
                .content()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            for child in right_content.iter() {
                if consumed >= effective_right_scalars || !child.is_text() {
                    break;
                }
                consumed = consumed
                    .checked_add(child.node_size())
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                consumed_semantic += 1;
            }
            if consumed != effective_right_scalars {
                return Err(OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "joined right semantic text does not match its tracked XML text",
                ));
            }
            appended_right_target = Some(right_id);
        }

        let remaining = right_node
            .content()
            .map(|fragment| {
                fragment
                    .iter()
                    .skip(consumed_semantic)
                    .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut prepared_work = 0usize;
        if !remaining.is_empty() {
            let mut batch =
                prepare_xml_nodes(&remaining, limits, left_path.len().saturating_add(2)).map_err(
                    |error| map_prepared_node_error(self.request_id, operation_index, error),
                )?;
            prepared_work = batch.work;
            let child_index = u32::try_from(left_target.storage_children.len())
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
            for child in &mut batch.nodes {
                child.index = child_index
                    .checked_add(child.index)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            self.push_action(YrsMutationAction::InsertXmlChildren {
                parent: XmlParentRef::Element(left_element),
                child_index,
                nodes: batch.nodes,
                signature: left_target.signature.clone(),
                operation_index,
            });
        }
        let delete_index = u32::try_from(boundary)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        self.push_action(YrsMutationAction::DeleteXmlChildren {
            parent: parent_target.parent,
            child_index: delete_index,
            child_count: 1,
            signature: parent_target.signature,
            operation_index,
        });
        self.charge_operation_work(
            operation_index,
            left_target
                .signature
                .children
                .len()
                .checked_add(right_target.signature.children.len())
                .and_then(|work| work.checked_add(prepared_work))
                .and_then(|work| work.checked_add(1))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;

        if let Some(right_id) = appended_right_target {
            let left_id = left_target
                .storage_children
                .last()
                .and_then(|child| match child {
                    StorageChildKind::Text { target, .. } => {
                        Some(AsRef::<Branch>::as_ref(target).id())
                    }
                    StorageChildKind::Element { .. } | StorageChildKind::PreparedElement => None,
                });
            let right_index = self.targets.iter().position(|candidate| {
                matches!(
                    &candidate.kind,
                    ResolvedTargetKind::Existing { target, .. }
                        if AsRef::<Branch>::as_ref(target).id() == right_id
                )
            });
            let left_index = left_id.and_then(|left_id| {
                self.targets.iter().position(|candidate| {
                    matches!(
                        &candidate.kind,
                        ResolvedTargetKind::Existing { target, .. }
                            if AsRef::<Branch>::as_ref(target).id() == left_id
                    )
                })
            });
            if let (Some(left_index), Some(right_index)) = (left_index, right_index) {
                let right = self.targets.remove(right_index);
                let left_index = if right_index < left_index {
                    left_index - 1
                } else {
                    left_index
                };
                self.targets[left_index].text.push_str(&right.text);
                self.targets[left_index].scalar_len = self.targets[left_index]
                    .scalar_len
                    .checked_add(right.scalar_len)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                let mut runs = self.targets[left_index].current_runs.clone();
                runs.extend(right.current_runs);
                self.targets[left_index].current_runs = normalize_prepared_runs(runs)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
        }
        let _ = left_node;
        Ok(())
    }

    pub(crate) fn finish(
        mut self,
        operation_index: Option<usize>,
    ) -> OperationResult<YrsMutationPlan> {
        let mut pending = std::mem::take(&mut self.pending_element_attrs)
            .into_values()
            .collect::<Vec<_>>();
        pending.sort_by_key(|entry| entry.first_order);
        for entry in pending {
            let mut concrete = Vec::new();
            append_attribute_diff(&mut concrete, entry);
            self.actions
                .extend(concrete.into_iter().map(ActionSlot::concrete));
        }
        self.charged_work = self
            .charged_work
            .checked_add(self.pending_traversal_work)
            .ok_or_else(|| {
                OperationError::operation_limit_exceeded(
                    self.request_id,
                    operation_index,
                    "maxActionsPerTransaction",
                    u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                    u64::MAX,
                )
            })?;
        self.pending_traversal_work = 0;
        if self.charged_work > self.action_limit {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                operation_index,
                "maxActionsPerTransaction",
                u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                u64::try_from(self.charged_work).unwrap_or(u64::MAX),
            ));
        }
        let path_parent_widths = self.explicit_path_parent_widths.take().unwrap_or_else(|| {
            self.structural_parents
                .values()
                .map(|parent| {
                    (
                        parent.signature.parent.clone(),
                        parent.signature.children.len(),
                    )
                })
                .collect::<HashMap<_, _>>()
        });
        let mut actions = Vec::new();
        for slot in self.actions {
            match slot {
                ActionSlot::Concrete(action) => actions.push(*action),
                ActionSlot::PreparedInsert(insert_id) => {
                    let insert = self
                        .prepared_inserts
                        .get_mut(insert_id)
                        .and_then(Option::take)
                        .ok_or_else(|| {
                            OperationError::engine_invariant_failed(
                                self.request_id,
                                operation_index,
                                "prepared insertion action has no owned blueprint",
                            )
                        })?;
                    actions.push(YrsMutationAction::InsertXmlChildren {
                        parent: insert.parent,
                        child_index: insert.child_index,
                        nodes: insert.nodes,
                        signature: insert.signature,
                        operation_index: insert.operation_index,
                    });
                }
                ActionSlot::Tombstone => {}
            }
        }
        let expected_preflight_work =
            expected_preflight_work(self.request_id, &actions, &path_parent_widths)?;
        Ok(YrsMutationPlan {
            actions,
            compilation_work: self.charged_work,
            expected_preflight_work,
            work_limit: self.action_limit,
            document_guard: Some(self.document_guard),
            prepared_metrics: Vec::new(),
            scan_work: self.scan_work,
            #[cfg(test)]
            position_resolver_work: self.position_resolver_work,
        })
    }
}
