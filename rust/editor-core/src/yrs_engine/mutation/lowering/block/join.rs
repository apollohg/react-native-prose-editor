impl MutationCompiler {
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
