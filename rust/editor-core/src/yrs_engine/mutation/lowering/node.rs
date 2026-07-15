impl MutationCompiler {
    pub(crate) fn insert_structural_node(
        &mut self,
        operation_index: usize,
        context: MutationDocumentContext<'_>,
        position: u32,
        node: &Node,
    ) -> OperationResult<()> {
        let MutationDocumentContext {
            before: document,
            after,
            schema,
            limits,
        } = context;
        let resolved = document.resolve(position).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "at", message)
        })?;
        let parent = resolved.parent(document);
        let content = parent.content().ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "structural insertion parent has no content",
            )
        })?;
        let path = resolved.node_path.iter().copied().collect::<Vec<_>>();
        if let Some(handle) = self.prepared_elements.get(&path).cloned() {
            return self.insert_into_prepared_structural_parent(
                operation_index,
                handle,
                content,
                resolved.parent_offset,
                node,
                schema,
                limits,
                after,
            );
        }
        let target = self.structural_parents.get(&path).cloned().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "semantic structural parent has no tracked Yrs branch",
            )
        })?;
        let insertion = self
            .current_storage_insertion(
                content.iter(),
                &target.storage_children,
                resolved.parent_offset,
            )
            .ok_or_else(|| {
                OperationError::operation_invalid(
                    self.request_id,
                    operation_index,
                    "at",
                    "structural insertion must resolve to an XML child boundary",
                )
            })?;
        let json = crate::serialize::node_to_prosemirror_json(node, schema);
        let mut batch = prepare_xml_nodes(
            std::slice::from_ref(&json),
            limits,
            path.len().saturating_add(2),
        )
        .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let materialized_empty_targets =
            materialize_empty_prepared_textblocks(&mut batch.nodes, schema);
        let (child_index, split_action, split_virtual) = match insertion {
            StorageInsertion::Boundary(child_index) => (child_index, None, None),
            StorageInsertion::InsideText {
                child_index,
                local_scalar,
                target: text_target,
                signature,
                runs,
            } => {
                let (delete_index_utf16, delete_len_utf16, suffix) = split_prepared_text_runs(
                    self.request_id,
                    operation_index,
                    &runs,
                    local_scalar,
                )?;
                let suffix_index = child_index
                    .checked_add(2)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                batch.nodes.push(PreparedXmlChild {
                    index: suffix_index,
                    node: PreparedXmlNode::Text { runs: suffix },
                });
                let target_id = AsRef::<Branch>::as_ref(&text_target).id();
                (
                    child_index
                        .checked_add(1)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?,
                    Some(YrsMutationAction::DeleteText {
                        target: text_target,
                        index_utf16: delete_index_utf16,
                        len_utf16: delete_len_utf16,
                        signature,
                        operation_index,
                    }),
                    Some((
                        target_id,
                        child_index,
                        local_scalar,
                        delete_index_utf16,
                        delete_len_utf16,
                    )),
                )
            }
        };
        for child in &mut batch.nodes {
            if child.index == child_index.saturating_add(1)
                && matches!(child.node, PreparedXmlNode::Text { .. })
            {
                continue;
            }
            child.index = child_index.checked_add(child.index).ok_or_else(|| {
                OperationError::operation_limit_exceeded(
                    self.request_id,
                    Some(operation_index),
                    "maxActionsPerTransaction",
                    u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                    u64::MAX,
                )
            })?;
        }
        let work = batch
            .work
            .checked_add(materialized_empty_targets)
            .and_then(|work| work.checked_add(target.signature.children.len()))
            .and_then(|work| work.checked_add(batch.nodes.len()))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, work)?;
        let split_storage = split_virtual.is_some();
        if let Some(action) = split_action {
            let slot = self.push_action(action);
            let (target_id, storage_index, local_scalar, delete_index_utf16, delete_len_utf16) =
                split_virtual
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let target_index = self
                .targets
                .iter()
                .position(|candidate| {
                    matches!(
                        &candidate.kind,
                        ResolvedTargetKind::Existing { target, .. }
                            if AsRef::<Branch>::as_ref(target).id() == target_id
                    )
                })
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let old_scalar_len = self.targets[target_index].scalar_len;
            remove_scalar_range(
                self.request_id,
                operation_index,
                &mut self.targets[target_index].text,
                local_scalar,
                old_scalar_len,
            )?;
            delete_prepared_run_range(
                &mut self.targets[target_index].current_runs,
                delete_index_utf16,
                delete_len_utf16,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets[target_index].scalar_len = local_scalar;
            self.targets[target_index].action_slots.push(slot);
            let parent = self
                .structural_parents
                .get_mut(&path)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let StorageChildKind::Text {
                scalar_len, runs, ..
            } = parent
                .storage_children
                .get_mut(
                    usize::try_from(storage_index)
                        .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
                )
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
            else {
                return Err(invalid_action_range(self.request_id, operation_index));
            };
            *scalar_len = local_scalar;
            delete_prepared_run_range(runs, delete_index_utf16, delete_len_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let semantic_index = semantic_insertion_index(content, resolved.parent_offset)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if !split_storage {
            let existing_insert_id =
                self.prepared_inserts
                    .iter()
                    .enumerate()
                    .find_map(|(insert_id, pending)| {
                        pending.as_ref().and_then(|pending| {
                            (pending.parent.id() == target.parent.id()
                                && pending.child_index == child_index
                                && pending.semantic_parent_path == path
                                && pending.first_semantic_index == semantic_index)
                                .then_some(insert_id)
                        })
                    });
            if let Some(insert_id) = existing_insert_id {
                let inserted_count = u32::try_from(batch.nodes.len())
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
                let pending = self
                    .prepared_inserts
                    .get_mut(insert_id)
                    .and_then(Option::as_mut)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                for child in &mut pending.nodes {
                    child.index = child
                        .index
                        .checked_add(inserted_count)
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                }
                let mut nodes = batch.nodes;
                nodes.append(&mut pending.nodes);
                pending.nodes = nodes;
                self.prepared_elements
                    .retain(|_, handle| handle.insert_id != insert_id);
                self.targets.retain(|target| {
                    !matches!(
                        &target.kind,
                        ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id
                    )
                });
                self.register_inserted_structural_children(
                    operation_index,
                    insert_id,
                    &path,
                    content,
                    resolved.parent_offset,
                    position,
                    child_index,
                    node.node_size(),
                    after,
                )?;
                return Ok(());
            }
        }
        let nodes = batch.nodes;
        let insert_id = self.queue_prepared_insert(PendingPreparedInsert {
            parent: target.parent.clone(),
            child_index,
            nodes,
            signature: target.signature.clone(),
            operation_index,
            semantic_parent_path: path.clone(),
            first_semantic_index: semantic_index,
        });
        self.register_inserted_structural_children(
            operation_index,
            insert_id,
            &path,
            content,
            resolved.parent_offset,
            position,
            child_index,
            node.node_size(),
            after,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_into_prepared_structural_parent(
        &mut self,
        operation_index: usize,
        parent_handle: PreparedHandle,
        semantic_children: &Fragment,
        parent_offset: u32,
        node: &Node,
        schema: &Schema,
        limits: &ResourceLimits,
        after: &Document,
    ) -> OperationResult<()> {
        let json = crate::serialize::node_to_prosemirror_json(node, schema);
        let mut batch = prepare_xml_nodes(
            std::slice::from_ref(&json),
            limits,
            parent_handle.ordinal_path.len().saturating_add(1),
        )
        .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        let inserted = batch
            .nodes
            .pop()
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
            .node;
        let insertion = {
            let parent = self.prepared_node_mut(&parent_handle, operation_index)?;
            let PreparedXmlNode::Element { children, .. } = parent else {
                return Err(invalid_action_range(self.request_id, operation_index));
            };
            prepared_structural_insertion(semantic_children, children, parent_offset)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
        };
        let request_id = self.request_id;
        let parent = self.prepared_node_mut(&parent_handle, operation_index)?;
        let PreparedXmlNode::Element { children, .. } = parent else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        match insertion {
            PreparedStructuralInsertion::Boundary(index) => {
                children.insert(
                    index,
                    PreparedXmlChild {
                        index: 0,
                        node: inserted,
                    },
                );
            }
            PreparedStructuralInsertion::InsideText {
                child_index,
                local_scalar,
            } => {
                let PreparedXmlNode::Text { runs } = &children
                    .get(child_index)
                    .ok_or_else(|| invalid_action_range(request_id, operation_index))?
                    .node
                else {
                    return Err(invalid_action_range(self.request_id, operation_index));
                };
                let cut_utf16 =
                    prepared_runs_utf16_at_scalar(request_id, operation_index, runs, local_scalar)?;
                let (left, right) = split_runs_utf16(runs, cut_utf16)
                    .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
                children[child_index].node = PreparedXmlNode::Text { runs: left };
                children.insert(
                    child_index + 1,
                    PreparedXmlChild {
                        index: 0,
                        node: inserted,
                    },
                );
                children.insert(
                    child_index + 2,
                    PreparedXmlChild {
                        index: 0,
                        node: PreparedXmlNode::Text { runs: right },
                    },
                );
            }
        }
        for (index, child) in children.iter_mut().enumerate() {
            child.index = u32::try_from(index)
                .map_err(|_| invalid_action_range(request_id, operation_index))?;
        }
        let insert_id = parent_handle.insert_id;
        self.prepared_elements
            .retain(|_, handle| handle.insert_id != insert_id);
        self.targets.retain(
            |target| !matches!(&target.kind, ResolvedTargetKind::Prepared { handle } if handle.insert_id == insert_id),
        );
        self.charge_operation_work(
            operation_index,
            batch
                .work
                .checked_add(semantic_children.child_count())
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        self.register_prepared_insert_state(operation_index, insert_id, after)
    }

    #[allow(clippy::too_many_arguments)]
    fn register_inserted_structural_children(
        &mut self,
        operation_index: usize,
        insert_id: usize,
        parent_path: &[u32],
        before_children: &Fragment,
        parent_offset: u32,
        absolute_position: u32,
        storage_child_index: u32,
        inserted_node_size: u32,
        after: &Document,
    ) -> OperationResult<()> {
        let semantic_index = semantic_insertion_index(before_children, parent_offset)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;

        let remap_path = |path: &mut Vec<u32>| -> OperationResult<()> {
            if path.len() > parent_path.len()
                && path.starts_with(parent_path)
                && path[parent_path.len()] >= semantic_index
            {
                path[parent_path.len()] = path[parent_path.len()]
                    .checked_add(1)
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            }
            Ok(())
        };
        let mut parents = HashMap::with_capacity(self.structural_parents.len());
        for (mut path, parent) in std::mem::take(&mut self.structural_parents) {
            remap_path(&mut path)?;
            parents.insert(path, parent);
        }
        self.structural_parents = parents;
        let mut prepared_elements = HashMap::with_capacity(self.prepared_elements.len());
        for (mut path, handle) in std::mem::take(&mut self.prepared_elements) {
            remap_path(&mut path)?;
            prepared_elements.insert(path, handle);
        }
        self.prepared_elements = prepared_elements;

        let parent = self
            .structural_parents
            .get_mut(parent_path)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let storage_index = usize::try_from(storage_child_index)
            .map_err(|_| invalid_action_range(self.request_id, operation_index))?;
        if storage_index > parent.storage_children.len() {
            return Err(invalid_action_range(self.request_id, operation_index));
        }
        parent
            .storage_children
            .insert(storage_index, StorageChildKind::PreparedElement);

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
            parent_path,
            semantic_index,
            Some(after),
            &mut elements,
            &mut texts,
        )?;
        for (path, handle) in elements {
            if self.prepared_elements.insert(path, handle).is_some() {
                return Err(OperationError::engine_invariant_failed(
                    self.request_id,
                    Some(operation_index),
                    "duplicate inserted prepared element semantic path",
                ));
            }
        }

        let positions = self.positions()?;
        let first_start = texts
            .first()
            .and_then(|(path, _, _)| first_text_doc_position(after.root(), path));
        let insertion = first_start.map_or_else(
            || positions.partition_point(|(start, _)| *start < absolute_position),
            |start| positions.partition_point(|(existing, _)| *existing < start),
        );
        let old_next_start = positions.get(insertion).map(|(start, _)| *start);
        let mut target_index = insertion;
        let mut current_end = if insertion == 0 {
            0
        } else {
            positions[insertion - 1].1
        };
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
            let shifted = old_next_start
                .checked_add(inserted_node_size)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            self.targets[target_index].gap_before = shifted
                .checked_sub(current_end)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        Ok(())
    }

}

fn wire_element_is_semantic_void<T: ReadTxn>(
    element: &XmlElementRef,
    txn: &T,
    schema: &Schema,
) -> bool {
    let node_type = super::super::codec::normalized_wire_element_node_type(element, txn);
    if let Some(spec) = schema.node(&node_type) {
        return spec.is_void;
    }
    true
}

fn materialize_text<T: ReadTxn>(
    request_id: u64,
    target: &XmlTextRef,
    txn: &T,
) -> OperationResult<MaterializedText> {
    let mut text = String::new();
    let mut signature_runs = Vec::<TextSignatureRun>::new();
    let mut prepared_runs = Vec::new();
    let mut scalar_len = 0u32;
    let mut utf16_len = 0u32;
    let mut work = 0usize;
    for diff in target.diff(txn, YChange::identity) {
        let yrs::Out::Any(Any::String(value)) = diff.insert else {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs XML text materialization contains a non-string value",
            ));
        };
        let value = value.to_string();
        if value.is_empty() {
            continue;
        }
        let value_scalar_len = u32::try_from(value.chars().count()).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs XML text scalar length exceeds u32",
            )
        })?;
        let value_utf16_len = u32::try_from(value.encode_utf16().count()).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs XML text UTF-16 length exceeds u32",
            )
        })?;
        let attrs = diff.attributes.as_deref().cloned().unwrap_or_default();
        work = work.checked_add(attrs.len()).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs XML text materialization work overflow",
            )
        })?;
        for (key, attr_value) in attrs.iter() {
            work = work
                .checked_add(key.len())
                .and_then(|work| work.checked_add(super::plan::any_preflight_work(attr_value)?))
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "Yrs XML text materialization work overflow",
                    )
                })?;
        }
        let mut signature_attrs = attrs
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        let sort_partitions = binary_partition_work(signature_attrs.len());
        let sort_key_work = signature_attrs
            .iter()
            .try_fold(0usize, |work, (key, _)| {
                work.checked_add(key.len().checked_mul(sort_partitions)?)
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs XML text materialization work overflow",
                )
            })?;
        work = work
            .checked_add(
                signature_attrs
                    .len()
                    .checked_mul(sort_partitions)
                    .and_then(|work| work.checked_add(sort_key_work))
                    .ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs XML text materialization work overflow",
                        )
                    })?,
            )
            .and_then(|work| work.checked_add(value.len()))
            .and_then(|work| work.checked_add(1))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs XML text materialization work overflow",
                )
            })?;
        signature_attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
        prepared_runs.push(PreparedTextRun {
            index_utf16: utf16_len,
            text: value.clone(),
            attrs,
        });
        if let Some(previous) = signature_runs
            .last_mut()
            .filter(|run| run.attrs == signature_attrs)
        {
            previous.text.push_str(&value);
        } else {
            signature_runs.push(TextSignatureRun {
                text: value.clone(),
                attrs: signature_attrs,
            });
        }
        text.push_str(&value);
        scalar_len = scalar_len.checked_add(value_scalar_len).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs XML text scalar length overflow",
            )
        })?;
        utf16_len = utf16_len.checked_add(value_utf16_len).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs XML text UTF-16 length overflow",
            )
        })?;
    }
    Ok(MaterializedText {
        text,
        scalar_len,
        utf16_len,
        signature_runs,
        prepared_runs,
        work,
    })
}

fn parent_signature_from_children(
    parent: &XmlElementRef,
    path: &[(BranchID, u32)],
    children: &[XmlOut],
    child_count: u32,
    child_index: u32,
) -> ParentSignature {
    ParentSignature {
        parent: <XmlElementRef as AsRef<Branch>>::as_ref(parent).id(),
        tag: parent.tag().clone(),
        path: path.to_vec(),
        child_count,
        initial_child_index: child_index,
        left_neighbor: child_index
            .checked_sub(1)
            .and_then(|index| children.get(index as usize))
            .map(XmlOut::id),
        right_neighbor: children.get(child_index as usize).map(XmlOut::id),
    }
}

fn checked_scalar_len(
    request_id: u64,
    operation_index: Option<usize>,
    text: &str,
) -> OperationResult<u32> {
    u32::try_from(text.chars().count()).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            operation_index,
            "text scalar length exceeds u32",
        )
    })
}

fn checked_text_lengths(
    request_id: u64,
    operation_index: Option<usize>,
    text: &str,
) -> OperationResult<(u32, u32)> {
    text.chars().try_fold((0u32, 0u32), |(scalars, utf16), ch| {
        let scalars = scalars.checked_add(1).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                operation_index,
                "text scalar length exceeds u32",
            )
        })?;
        let utf16 = utf16
            .checked_add(u32::from(ch.len_utf16() as u16))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    operation_index,
                    "text UTF-16 length exceeds u32",
                )
            })?;
        Ok((scalars, utf16))
    })
}

fn scalar_to_utf16(
    request_id: u64,
    operation_index: usize,
    text: &str,
    scalar: u32,
) -> OperationResult<u32> {
    super::super::scalar_offset_to_utf16(text, scalar).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved scalar offset is outside its Yrs XML text target",
        )
    })
}

fn scalar_byte_index(text: &str, scalar: u32) -> Option<usize> {
    if scalar == 0 {
        return Some(0);
    }
    text.char_indices()
        .nth(usize::try_from(scalar).ok()?)
        .map(|(index, _)| index)
        .or_else(|| (text.chars().count() == scalar as usize).then_some(text.len()))
}
