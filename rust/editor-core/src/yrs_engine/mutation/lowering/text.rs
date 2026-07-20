impl MutationCompiler {
    pub(crate) fn insert(
        &mut self,
        operation_index: usize,
        position: u32,
        text: &str,
        marks: &[Mark],
    ) -> OperationResult<()> {
        self.charge_operation_work(
            operation_index,
            self.localized_position_target_count
                .unwrap_or(self.targets.len()),
        )?;
        let attrs = marks_to_attrs(marks);
        self.charge_scan_work(operation_index, text.len())?;
        let (text_scalar_len, text_utf16) =
            checked_text_lengths(self.request_id, Some(operation_index), text)?;
        let action_work = 1usize
            .checked_add(text.len())
            .and_then(|work| work.checked_add(usize::try_from(text_utf16).ok()?))
            .and_then(|work| work.checked_add(attrs_work(&attrs)))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, action_work)?;
        let ResolvedInsertion {
            target_index,
            scalar_index,
        } = self.resolve_insertion(operation_index, position)?;
        self.charge_scan_work(operation_index, self.targets[target_index].text.len())?;
        let index_utf16 = scalar_to_utf16(
            self.request_id,
            operation_index,
            &self.targets[target_index].text,
            scalar_index,
        )?;
        let missing_gap_work = match &self.targets[target_index].kind {
            ResolvedTargetKind::Missing {
                signature,
                create_action: None,
                ..
            } => {
                let fenwick_len = usize::try_from(signature.child_count)
                    .ok()
                    .and_then(|len| len.checked_add(2))
                    .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                binary_partition_work(fenwick_len)
                    .checked_mul(2)
                    .and_then(|work| {
                        work.checked_add(
                            if self.created_gap_shifts.contains_key(&signature.parent) {
                                0
                            } else {
                                fenwick_len
                            },
                        )
                    })
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?
            }
            _ => 0,
        };
        self.charge_operation_work(operation_index, missing_gap_work)?;

        let run_attrs = attrs.clone();
        let mut prepared_handle = None;
        match &mut self.targets[target_index].kind {
            ResolvedTargetKind::Existing { target, signature } => {
                let slot = self.actions.len();
                self.actions
                    .push(ActionSlot::concrete(YrsMutationAction::InsertText {
                        target: target.clone(),
                        index_utf16,
                        text: text.to_owned(),
                        len_utf16: text_utf16,
                        attrs,
                        signature: signature.clone(),
                        operation_index,
                    }));
                self.targets[target_index].action_slots.push(slot);
            }
            ResolvedTargetKind::Missing {
                parent,
                child_index,
                signature,
                create_action,
            } => {
                if let Some(action_index) = *create_action {
                    let Some(YrsMutationAction::CreateText { follow_up, .. }) =
                        self.actions[action_index].concrete_mut()
                    else {
                        return Err(OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "created Yrs text target action index is invalid",
                        ));
                    };
                    follow_up.push(CreatedTextAction::Insert {
                        index_utf16,
                        text: text.to_owned(),
                        len_utf16: text_utf16,
                        attrs,
                        operation_index,
                    });
                } else {
                    let fenwick_len = usize::try_from(signature.child_count)
                        .ok()
                        .and_then(|len| len.checked_add(2))
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let gap = usize::try_from(signature.initial_child_index)
                        .ok()
                        .and_then(|index| index.checked_add(1))
                        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
                    let prior_creates_before_gap = {
                        let shifts = self
                            .created_gap_shifts
                            .entry(signature.parent.clone())
                            .or_insert_with(|| vec![0; fenwick_len]);
                        let prior = fenwick_prefix(shifts, gap).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                        fenwick_add(shifts, gap).ok_or_else(|| {
                            invalid_action_range(self.request_id, operation_index)
                        })?;
                        prior
                    };
                    let execution_child_index = child_index
                        .checked_add(prior_creates_before_gap)
                        .ok_or_else(|| {
                        invalid_action_range(self.request_id, operation_index)
                    })?;
                    *create_action = Some(self.actions.len());
                    self.actions
                        .push(ActionSlot::concrete(YrsMutationAction::CreateText {
                            parent: parent.clone(),
                            child_index: execution_child_index,
                            text: text.to_owned(),
                            scalar_len: text_scalar_len,
                            len_utf16: text_utf16,
                            attrs,
                            follow_up: Vec::new(),
                            signature: signature.clone(),
                            operation_index,
                        }));
                }
            }
            ResolvedTargetKind::Prepared { handle } => {
                prepared_handle = Some(handle.clone());
            }
        }
        let mutation_work = self.targets[target_index]
            .text
            .len()
            .checked_mul(2)
            .and_then(|work| work.checked_add(text.len()))
            .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
        self.charge_scan_work(operation_index, mutation_work)?;
        insert_scalar(
            self.request_id,
            operation_index,
            &mut self.targets[target_index].text,
            scalar_index,
            text,
        )?;
        self.targets[target_index].scalar_len = self.targets[target_index]
            .scalar_len
            .checked_add(text_scalar_len)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        insert_prepared_run(
            &mut self.targets[target_index].current_runs,
            index_utf16,
            text,
            run_attrs,
        )
        .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if let Some(handle) = prepared_handle {
            let runs = self.targets[target_index].current_runs.clone();
            let PreparedXmlNode::Text { runs: blueprint } =
                self.prepared_node_mut(&handle, operation_index)?
            else {
                return Err(invalid_action_range(self.request_id, operation_index));
            };
            *blueprint = runs;
        }
        Ok(())
    }

    pub(crate) fn delete(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
    ) -> OperationResult<TextRangeDisposition> {
        if from == to {
            return Ok(TextRangeDisposition::Applied);
        }
        self.charge_operation_work(
            operation_index,
            self.localized_position_target_count
                .unwrap_or(self.targets.len())
                .checked_add(boundaries.len())
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        let Some(spans) = self.covered_spans(operation_index, from, to, boundaries)? else {
            return Ok(TextRangeDisposition::Structural);
        };
        for span in spans.iter().rev() {
            #[cfg(test)]
            {
                self.virtual_delete_visits += 1;
            }
            self.charge_operation_work(
                operation_index,
                1usize
                    .checked_add(usize::try_from(span.len_utf16).unwrap_or(usize::MAX))
                    .ok_or_else(|| {
                        work_overflow(self.request_id, operation_index, self.action_limit)
                    })?,
            )?;
            match &self.targets[span.target].kind {
                ResolvedTargetKind::Existing { target, signature } => {
                    let slot = self.actions.len();
                    self.actions
                        .push(ActionSlot::concrete(YrsMutationAction::DeleteText {
                            target: target.clone(),
                            index_utf16: span.index_utf16,
                            len_utf16: span.len_utf16,
                            signature: signature.clone(),
                            operation_index,
                        }));
                    self.targets[span.target].action_slots.push(slot);
                }
                ResolvedTargetKind::Missing {
                    create_action: Some(action_index),
                    ..
                } => {
                    let Some(YrsMutationAction::CreateText { follow_up, .. }) =
                        self.actions[*action_index].concrete_mut()
                    else {
                        return Err(OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "created Yrs text target action index is invalid",
                        ));
                    };
                    follow_up.push(CreatedTextAction::Delete {
                        index_utf16: span.index_utf16,
                        len_utf16: span.len_utf16,
                        operation_index,
                    });
                }
                ResolvedTargetKind::Missing { .. } => {
                    return Err(OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "delete resolved to an uncreated Yrs text target",
                    ));
                }
                ResolvedTargetKind::Prepared { .. } => {}
            }
        }
        for span in spans.iter().rev() {
            let mutation_work = self.targets[span.target]
                .text
                .len()
                .checked_mul(3)
                .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
            self.charge_scan_work(operation_index, mutation_work)?;
            remove_scalar_range(
                self.request_id,
                operation_index,
                &mut self.targets[span.target].text,
                span.from_scalar,
                span.to_scalar,
            )?;
            self.targets[span.target].scalar_len -= span.to_scalar - span.from_scalar;
            delete_prepared_run_range(
                &mut self.targets[span.target].current_runs,
                span.index_utf16,
                span.len_utf16,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let handle = match &self.targets[span.target].kind {
                ResolvedTargetKind::Prepared { handle } => Some(handle.clone()),
                _ => None,
            };
            if let Some(handle) = handle {
                let runs = self.targets[span.target].current_runs.clone();
                let PreparedXmlNode::Text { runs: blueprint } =
                    self.prepared_node_mut(&handle, operation_index)?
                else {
                    return Err(invalid_action_range(self.request_id, operation_index));
                };
                *blueprint = runs;
            }
        }
        Ok(TextRangeDisposition::Applied)
    }

    pub(crate) fn format(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
        attrs: Attrs,
    ) -> OperationResult<()> {
        if from == to {
            return Ok(());
        }
        self.charge_operation_work(
            operation_index,
            self.localized_position_target_count
                .unwrap_or(self.targets.len())
                .checked_add(boundaries.len())
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        let Some(spans) = self.covered_spans(operation_index, from, to, boundaries)? else {
            return Err(OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "mark range crosses structural XML content",
            ));
        };
        for span in spans {
            let action_work = 1usize
                .checked_add(usize::try_from(span.len_utf16).unwrap_or(usize::MAX))
                .and_then(|work| work.checked_add(attrs_work(&attrs)))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?;
            self.charge_operation_work(operation_index, action_work)?;
            match &self.targets[span.target].kind {
                ResolvedTargetKind::Existing { target, signature } => {
                    let slot = self.actions.len();
                    self.actions
                        .push(ActionSlot::concrete(YrsMutationAction::FormatText {
                            target: target.clone(),
                            index_utf16: span.index_utf16,
                            len_utf16: span.len_utf16,
                            attrs: attrs.clone(),
                            signature: signature.clone(),
                            operation_index,
                        }));
                    self.targets[span.target].action_slots.push(slot);
                }
                ResolvedTargetKind::Missing {
                    create_action: Some(action_index),
                    ..
                } => {
                    let Some(YrsMutationAction::CreateText { follow_up, .. }) =
                        self.actions[*action_index].concrete_mut()
                    else {
                        return Err(OperationError::engine_invariant_failed(
                            self.request_id,
                            Some(operation_index),
                            "created Yrs text target action index is invalid",
                        ));
                    };
                    follow_up.push(CreatedTextAction::Format {
                        index_utf16: span.index_utf16,
                        len_utf16: span.len_utf16,
                        attrs: attrs.clone(),
                        operation_index,
                    });
                }
                ResolvedTargetKind::Missing { .. } => {
                    return Err(OperationError::engine_invariant_failed(
                        self.request_id,
                        Some(operation_index),
                        "format resolved to an uncreated Yrs text target",
                    ));
                }
                ResolvedTargetKind::Prepared { .. } => {}
            }
            format_prepared_run_range(
                &mut self.targets[span.target].current_runs,
                span.index_utf16,
                span.len_utf16,
                &attrs,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            let handle = match &self.targets[span.target].kind {
                ResolvedTargetKind::Prepared { handle } => Some(handle.clone()),
                _ => None,
            };
            if let Some(handle) = handle {
                let runs = self.targets[span.target].current_runs.clone();
                let PreparedXmlNode::Text { runs: blueprint } =
                    self.prepared_node_mut(&handle, operation_index)?
                else {
                    return Err(invalid_action_range(self.request_id, operation_index));
                };
                *blueprint = runs;
            }
        }
        Ok(())
    }

    pub(crate) fn replace(
        &mut self,
        operation_index: usize,
        context: MutationDocumentContext<'_>,
        replacement: ReplacementInput<'_>,
    ) -> OperationResult<()> {
        let ReplacementInput {
            from,
            to,
            boundaries,
            content,
        } = replacement;
        if self.delete(operation_index, from, to, boundaries)? == TextRangeDisposition::Structural {
            return self.replace_structural_range(operation_index, context, replacement);
        }
        let pieces = inline_text_pieces(self.request_id, operation_index, content)?;
        let piece_materialization_work = pieces
            .iter()
            .try_fold(0usize, |total, (text, _)| total.checked_add(text.len()))
            .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
        self.charge_scan_work(operation_index, piece_materialization_work)?;
        let mut position = from;
        for (text, marks) in pieces {
            if text.is_empty() {
                continue;
            }
            self.insert(operation_index, position, &text, &marks)?;
            self.charge_scan_work(operation_index, text.len())?;
            position = position
                .checked_add(checked_scalar_len(
                    self.request_id,
                    Some(operation_index),
                    &text,
                )?)
                .ok_or_else(|| {
                    OperationError::operation_invalid(
                        self.request_id,
                        operation_index,
                        "content",
                        "replacement position overflow",
                    )
                })?;
        }
        Ok(())
    }

    pub(crate) fn replace_structural_range(
        &mut self,
        operation_index: usize,
        context: MutationDocumentContext<'_>,
        replacement: ReplacementInput<'_>,
    ) -> OperationResult<()> {
        let MutationDocumentContext {
            before: document,
            after,
            schema,
            limits,
        } = context;
        let ReplacementInput {
            from, to, content, ..
        } = replacement;
        let from_resolved = document.resolve(from).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "range", message)
        })?;
        let to_resolved = document.resolve(to).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "range", message)
        })?;
        if from_resolved.node_path != to_resolved.node_path {
            self.delete_cross_parent_structural_range(
                operation_index,
                document,
                &from_resolved,
                &to_resolved,
            )?;
            return self.insert_cross_parent_replacement(
                operation_index,
                document,
                after,
                from,
                &from_resolved,
                content,
                schema,
                limits,
            );
        }
        let parent = from_resolved.parent(document);
        let semantic_children = parent.content().ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "range",
                "structural replacement parent has no content",
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
        let start = self
            .current_storage_insertion(
                semantic_children.iter(),
                &target.storage_children,
                from_resolved.parent_offset,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        let end = self
            .current_storage_insertion(
                semantic_children.iter(),
                &target.storage_children,
                to_resolved.parent_offset,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;

        if let StorageInsertion::InsideText {
            local_scalar,
            target,
            signature,
            runs,
            ..
        } = &end
        {
            let end_utf16 = prepared_runs_utf16_at_scalar(
                self.request_id,
                operation_index,
                runs,
                *local_scalar,
            )?;
            if end_utf16 > 0 {
                self.push_action(YrsMutationAction::DeleteText {
                    target: target.clone(),
                    index_utf16: 0,
                    len_utf16: end_utf16,
                    signature: signature.clone(),
                    operation_index,
                });
            }
        }
        if let StorageInsertion::InsideText {
            local_scalar,
            target,
            signature,
            runs,
            ..
        } = &start
        {
            let start_utf16 = prepared_runs_utf16_at_scalar(
                self.request_id,
                operation_index,
                runs,
                *local_scalar,
            )?;
            let total_utf16 = prepared_runs_utf16_len(self.request_id, operation_index, runs)?;
            let len_utf16 = total_utf16
                .checked_sub(start_utf16)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
            if len_utf16 > 0 {
                self.push_action(YrsMutationAction::DeleteText {
                    target: target.clone(),
                    index_utf16: start_utf16,
                    len_utf16,
                    signature: signature.clone(),
                    operation_index,
                });
            }
        }

        let delete_start = match &start {
            StorageInsertion::Boundary(index) => *index,
            StorageInsertion::InsideText { child_index, .. } => child_index
                .checked_add(1)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?,
        };
        let delete_end = match &end {
            StorageInsertion::Boundary(index) => *index,
            StorageInsertion::InsideText { child_index, .. } => *child_index,
        };
        let child_count = delete_end
            .checked_sub(delete_start)
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        if child_count > 0 {
            self.push_action(YrsMutationAction::DeleteXmlChildren {
                parent: target.parent.clone(),
                child_index: delete_start,
                child_count,
                signature: target.signature.clone(),
                operation_index,
            });
        }

        let json = content
            .iter()
            .map(|node| crate::serialize::node_to_prosemirror_json(node, schema))
            .collect::<Vec<_>>();
        let mut batch = prepare_xml_nodes(&json, limits, path.len().saturating_add(2))
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
        for child in &mut batch.nodes {
            child.index = delete_start
                .checked_add(child.index)
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?;
        }
        let endpoint_work = match (&start, &end) {
            (StorageInsertion::InsideText { .. }, StorageInsertion::InsideText { .. }) => 2,
            (StorageInsertion::InsideText { .. }, _) | (_, StorageInsertion::InsideText { .. }) => {
                1
            }
            _ => 0,
        };
        let work = target
            .signature
            .children
            .len()
            .checked_add(usize::try_from(child_count).unwrap_or(usize::MAX))
            .and_then(|work| work.checked_add(batch.work))
            .and_then(|work| work.checked_add(endpoint_work))
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.charge_operation_work(operation_index, work)?;
        if !batch.nodes.is_empty() {
            self.push_action(YrsMutationAction::InsertXmlChildren {
                parent: target.parent,
                child_index: delete_start,
                nodes: batch.nodes,
                signature: target.signature,
                operation_index,
            });
        }
        Ok(())
    }
}

fn insert_scalar(
    request_id: u64,
    operation_index: usize,
    target: &mut String,
    scalar: u32,
    text: &str,
) -> OperationResult<()> {
    let byte = scalar_byte_index(target, scalar).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "resolved insertion scalar offset is outside virtual target",
        )
    })?;
    target.insert_str(byte, text);
    Ok(())
}

fn remove_scalar_range(
    request_id: u64,
    operation_index: usize,
    target: &mut String,
    from: u32,
    to: u32,
) -> OperationResult<()> {
    let from_byte = scalar_byte_index(target, from)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    let to_byte = scalar_byte_index(target, to)
        .ok_or_else(|| invalid_action_range(request_id, operation_index))?;
    target.replace_range(from_byte..to_byte, "");
    Ok(())
}

fn marks_to_attrs(marks: &[Mark]) -> Attrs {
    marks
        .iter()
        .map(|mark| {
            let value = if mark.attrs().is_empty() {
                Any::Bool(true)
            } else {
                json_to_any(&Value::Object(mark.attrs().clone().into_iter().collect()))
            };
            (Arc::<str>::from(mark.mark_type()), value)
        })
        .collect()
}

pub(crate) fn mark_attr(mark: &Mark) -> Attrs {
    marks_to_attrs(std::slice::from_ref(mark))
}

pub(crate) fn removed_mark_attr(mark_type: &str) -> Attrs {
    Attrs::from([(Arc::<str>::from(mark_type), Any::Null)])
}

fn json_to_any(value: &Value) -> Any {
    match value {
        Value::Null => Any::Null,
        Value::Bool(value) => Any::Bool(*value),
        Value::Number(number) => number
            .as_i64()
            .map(Any::BigInt)
            .or_else(|| number.as_f64().map(Any::Number))
            .unwrap_or(Any::Null),
        Value::String(value) => Any::String(value.clone().into()),
        Value::Array(values) => Any::Array(values.iter().map(json_to_any).collect()),
        Value::Object(values) => Any::Map(Arc::new(
            values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_any(value)))
                .collect(),
        )),
    }
}

fn inline_text_pieces(
    request_id: u64,
    operation_index: usize,
    fragment: &Fragment,
) -> OperationResult<Vec<(String, Vec<Mark>)>> {
    let mut output = Vec::new();
    for node in fragment.iter() {
        collect_inline_piece(request_id, operation_index, node, &mut output)?;
    }
    Ok(output)
}

fn collect_inline_piece(
    request_id: u64,
    operation_index: usize,
    node: &Node,
    output: &mut Vec<(String, Vec<Mark>)>,
) -> OperationResult<()> {
    if let Some(text) = node.text_str() {
        output.push((text.to_owned(), node.marks().to_vec()));
        return Ok(());
    }
    Err(OperationError::operation_invalid(
        request_id,
        operation_index,
        "content",
        "replacement content must resolve to existing XML text targets",
    ))
}
