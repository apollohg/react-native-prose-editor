impl MutationCompiler {
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
}
