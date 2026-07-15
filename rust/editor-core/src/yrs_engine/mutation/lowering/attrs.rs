impl MutationCompiler {
    pub(crate) fn update_node_attrs(
        &mut self,
        operation_index: usize,
        document: &Document,
        position: u32,
        attrs: &HashMap<String, Value>,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> OperationResult<()> {
        let resolved = document.resolve(position).map_err(|message| {
            OperationError::operation_invalid(self.request_id, operation_index, "at", message)
        })?;
        let parent = resolved.parent(document);
        let content = parent.content().ok_or_else(|| {
            OperationError::operation_invalid(
                self.request_id,
                operation_index,
                "at",
                "attribute target parent has no content",
            )
        })?;
        let mut offset = 0u32;
        let (semantic_index, target_node) = content
            .iter()
            .enumerate()
            .find(|(_, child)| {
                let matches = !child.is_text() && resolved.parent_offset == offset;
                if !matches {
                    offset = offset.saturating_add(child.node_size());
                }
                matches
            })
            .ok_or_else(|| {
                OperationError::operation_invalid(
                    self.request_id,
                    operation_index,
                    "at",
                    "attribute position does not resolve to an XML element",
                )
            })?;
        let path = resolved.node_path.iter().copied().collect::<Vec<_>>();
        let mut target_path = path.clone();
        target_path.push(
            u32::try_from(semantic_index)
                .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
        );
        if let Some(handle) = self.prepared_elements.get(&target_path).cloned() {
            let replacement = if target_node.is_void() {
                Node::void(target_node.node_type().to_owned(), attrs.clone())
            } else {
                Node::element(
                    target_node.node_type().to_owned(),
                    attrs.clone(),
                    target_node
                        .content()
                        .cloned()
                        .unwrap_or_else(Fragment::empty),
                )
            };
            let json = crate::serialize::node_to_prosemirror_json(&replacement, schema);
            let mut prepared = prepare_xml_nodes(
                std::slice::from_ref(&json),
                limits,
                target_path.len().saturating_add(1),
            )
            .map_err(|error| map_prepared_node_error(self.request_id, operation_index, error))?;
            let replacement = prepared
                .nodes
                .pop()
                .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
                .node;
            self.charge_operation_work(operation_index, prepared.work)?;
            *self.prepared_node_mut(&handle, operation_index)? = replacement;
            return Ok(());
        }
        let structural_parent = self.structural_parents.get(&path).cloned().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "semantic attribute parent has no tracked Yrs branch",
            )
        })?;
        let StorageInsertion::Boundary(storage_index) = self
            .current_storage_insertion(
                content.iter(),
                &structural_parent.storage_children,
                resolved.parent_offset,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
        else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        let StorageChildKind::Element { target, signature } = structural_parent
            .storage_children
            .get(
                usize::try_from(storage_index)
                    .map_err(|_| invalid_action_range(self.request_id, operation_index))?,
            )
            .ok_or_else(|| invalid_action_range(self.request_id, operation_index))?
        else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        let target = target.clone();
        let signature = signature.clone();
        let replacement = if target_node.is_void() {
            Node::void(target_node.node_type().to_owned(), attrs.clone())
        } else {
            Node::element(
                target_node.node_type().to_owned(),
                attrs.clone(),
                target_node
                    .content()
                    .cloned()
                    .unwrap_or_else(Fragment::empty),
            )
        };
        let old_json = crate::serialize::node_to_prosemirror_json(target_node, schema);
        let new_json = crate::serialize::node_to_prosemirror_json(&replacement, schema);
        let prepared =
            prepare_xml_nodes(&[old_json, new_json], limits, path.len().saturating_add(2))
                .map_err(|error| {
                    map_prepared_node_error(self.request_id, operation_index, error)
                })?;
        let mut nodes = prepared.nodes.into_iter();
        let Some(PreparedXmlChild {
            node:
                PreparedXmlNode::Element {
                    tag: old_tag,
                    attrs: old_attrs,
                    ..
                },
            ..
        }) = nodes.next()
        else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        let old_attrs = old_attrs
            .into_iter()
            .map(|(key, value)| (Arc::<str>::from(key), value))
            .collect::<Vec<_>>();
        let Some(PreparedXmlChild {
            node:
                PreparedXmlNode::Element {
                    tag: new_tag,
                    attrs: desired,
                    ..
                },
            ..
        }) = nodes.next()
        else {
            return Err(invalid_action_range(self.request_id, operation_index));
        };
        let desired = desired
            .into_iter()
            .map(|(key, value)| (Arc::<str>::from(key), value))
            .collect::<Vec<_>>();
        let expected_old_attrs = self
            .pending_element_attrs
            .get(&signature.target)
            .map(|pending| pending.desired.as_slice())
            .unwrap_or(signature.attrs.as_slice());
        if old_tag != new_tag
            || old_tag != signature.tag.as_ref()
            || old_attrs != expected_old_attrs
        {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "canonical semantic attributes do not match the tracked Yrs element",
            ));
        }
        self.charge_operation_work(
            operation_index,
            prepared
                .work
                .checked_add(signature.path.len())
                .and_then(|work| work.checked_add(signature.attrs.len()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?,
        )?;
        let first_order = self.pending_element_attrs.len();
        self.pending_element_attrs
            .entry(signature.target.clone())
            .and_modify(|pending| {
                pending.desired = desired.clone();
                pending.operation_index = operation_index;
            })
            .or_insert_with(|| PendingElementAttrs {
                target,
                signature,
                desired,
                operation_index,
                first_order,
            });
        Ok(())
    }

}
fn append_attribute_diff(actions: &mut Vec<YrsMutationAction>, entry: PendingElementAttrs) {
    let mut old_index = 0usize;
    let mut new_index = 0usize;
    while old_index < entry.signature.attrs.len() || new_index < entry.desired.len() {
        match (
            entry.signature.attrs.get(old_index),
            entry.desired.get(new_index),
        ) {
            (Some((old_key, _)), Some((new_key, new_value))) if old_key == new_key => {
                if entry.signature.attrs[old_index].1 != *new_value {
                    actions.push(YrsMutationAction::SetXmlAttribute {
                        target: entry.target.clone(),
                        key: new_key.clone(),
                        value: new_value.clone(),
                        signature: entry.signature.clone(),
                        operation_index: entry.operation_index,
                    });
                }
                old_index += 1;
                new_index += 1;
            }
            (Some((old_key, _)), Some((new_key, _))) if old_key < new_key => {
                actions.push(YrsMutationAction::RemoveXmlAttribute {
                    target: entry.target.clone(),
                    key: old_key.clone(),
                    signature: entry.signature.clone(),
                    operation_index: entry.operation_index,
                });
                old_index += 1;
            }
            (Some(_), Some((new_key, new_value))) => {
                actions.push(YrsMutationAction::SetXmlAttribute {
                    target: entry.target.clone(),
                    key: new_key.clone(),
                    value: new_value.clone(),
                    signature: entry.signature.clone(),
                    operation_index: entry.operation_index,
                });
                new_index += 1;
            }
            (Some((old_key, _)), None) => {
                actions.push(YrsMutationAction::RemoveXmlAttribute {
                    target: entry.target.clone(),
                    key: old_key.clone(),
                    signature: entry.signature.clone(),
                    operation_index: entry.operation_index,
                });
                old_index += 1;
            }
            (None, Some((new_key, new_value))) => {
                actions.push(YrsMutationAction::SetXmlAttribute {
                    target: entry.target.clone(),
                    key: new_key.clone(),
                    value: new_value.clone(),
                    signature: entry.signature.clone(),
                    operation_index: entry.operation_index,
                });
                new_index += 1;
            }
            (None, None) => break,
        }
    }
}
