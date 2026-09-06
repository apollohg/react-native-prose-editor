impl MutationCompiler {
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
}
