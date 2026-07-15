impl MutationCompiler {
    fn charge_operation_work(
        &mut self,
        operation_index: usize,
        amount: usize,
    ) -> OperationResult<()> {
        let amount = amount
            .checked_add(self.pending_traversal_work)
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        self.pending_traversal_work = 0;
        self.charged_work = self
            .charged_work
            .checked_add(amount)
            .ok_or_else(|| work_overflow(self.request_id, operation_index, self.action_limit))?;
        if self.charged_work > self.action_limit {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                Some(operation_index),
                "maxActionsPerTransaction",
                u64::try_from(self.action_limit).unwrap_or(u64::MAX),
                u64::try_from(self.charged_work).unwrap_or(u64::MAX),
            ));
        }
        Ok(())
    }

    fn charge_scan_work(&mut self, operation_index: usize, amount: usize) -> OperationResult<()> {
        self.scan_work = self
            .scan_work
            .checked_add(amount)
            .ok_or_else(|| scan_overflow(self.request_id, operation_index, self.scan_limit))?;
        if self.scan_work > self.scan_limit {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                Some(operation_index),
                "maxInputBytes",
                u64::try_from(self.scan_limit).unwrap_or(u64::MAX),
                u64::try_from(self.scan_work).unwrap_or(u64::MAX),
            ));
        }
        Ok(())
    }

    pub(crate) fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        bytes: usize,
    ) -> OperationResult<()> {
        self.charge_scan_work(operation_index, bytes)
    }

    pub(crate) fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        self.charge_operation_work(operation_index, 1)
    }

    #[cfg(test)]
    pub(crate) fn total_mutation_work_for_test(&self) -> usize {
        self.charged_work + self.pending_traversal_work
    }

    #[cfg(test)]
    pub(crate) fn target_positions_for_test(&self) -> OperationResult<Vec<(u32, u32)>> {
        self.positions()
    }

    #[cfg(test)]
    pub(crate) fn virtual_delete_visits_for_test(&self) -> usize {
        self.virtual_delete_visits
    }

    fn positions(&self) -> OperationResult<Vec<(u32, u32)>> {
        let mut positions = Vec::with_capacity(self.targets.len());
        let mut cursor = 0u32;
        for target in &self.targets {
            cursor = cursor.checked_add(target.gap_before).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    None,
                    "mutation target position overflow",
                )
            })?;
            let start = cursor;
            cursor = cursor.checked_add(target.scalar_len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.request_id,
                    None,
                    "mutation target length overflow",
                )
            })?;
            positions.push((start, cursor));
        }
        Ok(positions)
    }

    fn resolve_insertion(
        &self,
        operation_index: usize,
        position: u32,
    ) -> OperationResult<ResolvedInsertion> {
        let positions = self.positions()?;
        for (index, &(start, end)) in positions.iter().enumerate() {
            if position >= start && position <= end {
                return Ok(ResolvedInsertion {
                    target_index: index,
                    scalar_index: position - start,
                });
            }
        }
        Err(OperationError::position_invalid(
            self.request_id,
            operation_index,
            "at",
            "text insertion does not resolve to an existing Yrs XML text target",
        ))
    }

    fn covered_spans(
        &mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
    ) -> OperationResult<Option<Vec<ResolvedSpan>>> {
        if boundaries.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(OperationError::engine_invariant_failed(
                self.request_id,
                Some(operation_index),
                "semantic text boundaries must be strictly increasing",
            ));
        }
        let positions = self.positions()?;
        let mut spans = Vec::new();
        let mut covered = 0u32;
        for (target, &(start, end)) in positions.iter().enumerate() {
            let overlap_from = from.max(start);
            let overlap_to = to.min(end);
            if overlap_from >= overlap_to {
                continue;
            }
            let lower = boundaries.partition_point(|boundary| *boundary <= overlap_from);
            let upper = boundaries.partition_point(|boundary| *boundary < overlap_to);
            let local_boundaries = &boundaries[lower..upper];
            let search_work = binary_partition_work(boundaries.len())
                .checked_mul(2)
                .and_then(|work| work.checked_add(local_boundaries.len()))
                .ok_or_else(|| {
                    work_overflow(self.request_id, operation_index, self.action_limit)
                })?;
            self.charge_operation_work(operation_index, search_work)?;
            let mut cuts = Vec::with_capacity(local_boundaries.len() + 2);
            cuts.push(overlap_from);
            cuts.extend_from_slice(local_boundaries);
            cuts.push(overlap_to);
            for pair in cuts.windows(2) {
                let local_from = pair[0] - start;
                let local_to = pair[1] - start;
                let coordinate_work =
                    self.targets[target]
                        .text
                        .len()
                        .checked_mul(2)
                        .ok_or_else(|| {
                            scan_overflow(self.request_id, operation_index, self.scan_limit)
                        })?;
                self.charge_scan_work(operation_index, coordinate_work)?;
                let index_utf16 = scalar_to_utf16(
                    self.request_id,
                    operation_index,
                    &self.targets[target].text,
                    local_from,
                )?;
                let end_utf16 = scalar_to_utf16(
                    self.request_id,
                    operation_index,
                    &self.targets[target].text,
                    local_to,
                )?;
                spans.push(ResolvedSpan {
                    target,
                    from_scalar: local_from,
                    to_scalar: local_to,
                    index_utf16,
                    len_utf16: end_utf16 - index_utf16,
                });
            }
            covered = covered
                .checked_add(overlap_to - overlap_from)
                .ok_or_else(|| {
                    OperationError::operation_invalid(
                        self.request_id,
                        operation_index,
                        "range",
                        "mutation range work overflow",
                    )
                })?;
        }
        if covered != to - from {
            return Ok(None);
        }
        Ok(Some(spans))
    }
}

fn any_traversal_work(root: &Any) -> Option<usize> {
    let mut work = 0usize;
    let mut stack = vec![root];
    while let Some(value) = stack.pop() {
        work = work.checked_add(1)?;
        match value {
            Any::String(value) => work = work.checked_add(value.len())?,
            Any::Buffer(value) => work = work.checked_add(value.len())?,
            Any::Array(values) => {
                work = work.checked_add(values.len())?;
                stack.extend(values.iter());
            }
            Any::Map(values) => {
                work = work.checked_add(values.len())?;
                for (key, value) in values.iter() {
                    work = work.checked_add(key.len())?;
                    stack.push(value);
                }
            }
            Any::Null | Any::Undefined | Any::Bool(_) | Any::Number(_) | Any::BigInt(_) => {}
        }
    }
    Some(work)
}

fn semantic_node_clone_work(root: &Node) -> Option<usize> {
    fn json_work(value: &Value) -> Option<usize> {
        match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => Some(1),
            Value::String(value) => 1usize.checked_add(value.len()),
            Value::Array(values) => values
                .iter()
                .try_fold(1usize.checked_add(values.len())?, |work, value| {
                    work.checked_add(json_work(value)?)
                }),
            Value::Object(values) => values
                .iter()
                .try_fold(1usize.checked_add(values.len())?, |work, (key, value)| {
                    work.checked_add(key.len())?.checked_add(json_work(value)?)
                }),
        }
    }

    let mut work = 0usize;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        work = work.checked_add(1)?.checked_add(node.node_type().len())?;
        for (key, value) in node.attrs() {
            work = work
                .checked_add(key.len())?
                .checked_add(json_work(value)?)?;
        }
        for mark in node.marks() {
            work = work.checked_add(1)?.checked_add(mark.mark_type().len())?;
            for (key, value) in mark.attrs() {
                work = work
                    .checked_add(key.len())?
                    .checked_add(json_work(value)?)?;
            }
        }
        if let Some(text) = node.text_str() {
            work = work.checked_add(text.len())?;
        }
        if let Some(content) = node.content() {
            work = work.checked_add(content.child_count())?;
            stack.extend(content.iter());
        }
    }
    Some(work)
}

fn remap_semantic_path(
    path: &[u32],
    splice: &VirtualStructuralSplice,
) -> OperationResult<Option<Vec<u32>>> {
    if path.len() <= splice.parent_path.len() || !path.starts_with(&splice.parent_path) {
        return Ok(Some(path.to_vec()));
    }
    let mut remapped = path.to_vec();
    let child = remapped[splice.parent_path.len()];
    let delete_end = splice
        .semantic_index
        .checked_add(splice.semantic_delete)
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(0, None, "semantic splice overflow")
        })?;
    if child < splice.semantic_index {
        return Ok(Some(remapped));
    }
    if child < delete_end {
        return Ok(None);
    }
    remapped[splice.parent_path.len()] =
        shift_semantic_index(child, splice.semantic_delete, splice.semantic_insert)?;
    Ok(Some(remapped))
}

fn shift_semantic_index(index: u32, deleted: u32, inserted: u32) -> OperationResult<u32> {
    index
        .checked_sub(deleted)
        .and_then(|index| index.checked_add(inserted))
        .ok_or_else(|| OperationError::engine_invariant_failed(0, None, "semantic index overflow"))
}

fn mutation_action_touches_branches(
    action: &YrsMutationAction,
    branches: &HashSet<BranchID>,
) -> bool {
    match action {
        YrsMutationAction::InsertText { target, .. }
        | YrsMutationAction::DeleteText { target, .. }
        | YrsMutationAction::FormatText { target, .. } => {
            branches.contains(&AsRef::<Branch>::as_ref(target).id())
        }
        YrsMutationAction::CreateText { parent, .. } => {
            branches.contains(&AsRef::<Branch>::as_ref(parent).id())
        }
        YrsMutationAction::DeleteXmlChildren { parent, .. }
        | YrsMutationAction::InsertXmlChildren { parent, .. } => match parent {
            XmlParentRef::Element(parent) => {
                branches.contains(&AsRef::<Branch>::as_ref(parent).id())
            }
            XmlParentRef::Fragment(_) => false,
        },
        YrsMutationAction::SetXmlAttribute { target, .. }
        | YrsMutationAction::RemoveXmlAttribute { target, .. } => {
            branches.contains(&AsRef::<Branch>::as_ref(target).id())
        }
    }
}

fn collect_document_content_positions(
    request_id: u64,
    root: &Node,
) -> OperationResult<(HashMap<Vec<u32>, u32>, usize)> {
    fn visit(
        request_id: u64,
        node: &Node,
        path: &mut Vec<u32>,
        content_position: u32,
        positions: &mut HashMap<Vec<u32>, u32>,
        work: &mut usize,
    ) -> OperationResult<()> {
        positions.insert(path.clone(), content_position);
        *work = work
            .checked_add(1)
            .and_then(|work| work.checked_add(path.len()))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "document content-position work overflow",
                )
            })?;
        let Some(content) = node.content() else {
            return Ok(());
        };
        let mut child_position = content_position;
        for (index, child) in content.iter().enumerate() {
            if child.is_element() {
                path.push(u32::try_from(index).map_err(|_| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "document content-position child index exceeds u32",
                    )
                })?);
                visit(
                    request_id,
                    child,
                    path,
                    child_position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "document content position overflow",
                        )
                    })?,
                    positions,
                    work,
                )?;
                path.pop();
            }
            child_position = child_position
                .checked_add(child.node_size())
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "document content position overflow",
                    )
                })?;
        }
        Ok(())
    }

    let mut positions = HashMap::new();
    let mut work = 0usize;
    visit(
        request_id,
        root,
        &mut Vec::new(),
        0,
        &mut positions,
        &mut work,
    )?;
    Ok((positions, work))
}

fn exact_storage_child_span<'a>(
    children: impl Iterator<Item = &'a Node>,
    storage_children: &[StorageChildKind],
    from: u32,
    to: u32,
) -> Option<(u32, u32)> {
    let semantic_children = children.collect::<Vec<_>>();
    let mut semantic_index = 0usize;
    let mut offset = 0u32;
    let mut start = None;
    let mut end = None;
    for (index, storage) in storage_children.iter().enumerate() {
        if offset == from {
            start = Some(u32::try_from(index).ok()?);
        }
        match storage {
            StorageChildKind::Text { scalar_len, .. } => {
                let mut remaining = *scalar_len;
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
        if offset == to {
            end = Some(u32::try_from(index + 1).ok()?);
            break;
        }
        if offset > to {
            return None;
        }
    }
    if from == to || semantic_index > semantic_children.len() {
        return None;
    }
    let start = start?;
    let end = end?;
    Some((start, end.checked_sub(start)?))
}

#[allow(clippy::too_many_arguments)]
fn collect_structural_parents<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    parent: XmlParentRef,
    semantic_path: Vec<u32>,
    branch_path: Vec<(BranchID, u32)>,
    schema: &Schema,
    traversal_work: &mut usize,
    materialized_texts: &HashMap<BranchID, MaterializedText>,
    output: &mut HashMap<Vec<u32>, StructuralParentTarget>,
) -> OperationResult<()> {
    let children = match &parent {
        XmlParentRef::Fragment(parent) => parent.children(txn).collect::<Vec<_>>(),
        XmlParentRef::Element(parent) => parent.children(txn).collect::<Vec<_>>(),
    };
    *traversal_work = traversal_work
        .checked_add(1)
        .and_then(|work| work.checked_add(branch_path.len()))
        .and_then(|work| work.checked_add(children.len()))
        .ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "structural parent traversal work overflow",
            )
        })?;
    let parent_id = parent.id();
    let signature = Arc::new(StructuralParentSignature {
        parent: parent_id.clone(),
        path: branch_path.clone(),
        children: children.iter().map(XmlOut::id).collect(),
    });
    let storage_children = children
        .iter()
        .enumerate()
        .map(|(child_index, child)| match child {
            XmlOut::Text(text) => {
                let target_id = AsRef::<Branch>::as_ref(text).id();
                let materialized = materialized_texts.get(&target_id).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "structural text has no shared materialization",
                    )
                })?;
                let child_index = u32::try_from(child_index).map_err(|_| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "structural text child index exceeds u32",
                    )
                })?;
                let mut text_path = Vec::with_capacity(branch_path.len() + 1);
                text_path.push((parent_id.clone(), child_index));
                text_path.extend_from_slice(&branch_path);
                let signature = TargetSignature {
                    target: target_id,
                    path: text_path,
                    initial_len_utf16: materialized.utf16_len,
                    runs: materialized.signature_runs.clone(),
                    capture_work: materialized.work,
                };
                Ok(StorageChildKind::Text {
                    scalar_len: materialized.scalar_len,
                    target: text.clone(),
                    signature,
                    runs: materialized.prepared_runs.clone(),
                })
            }
            XmlOut::Element(element) => {
                let child_index = u32::try_from(child_index).map_err(|_| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "structural element child index exceeds u32",
                    )
                })?;
                let mut element_path = Vec::with_capacity(branch_path.len() + 1);
                element_path.push((parent_id.clone(), child_index));
                element_path.extend_from_slice(&branch_path);
                let mut attrs = Vec::new();
                for (key, value) in element.attributes(txn) {
                    let yrs::Out::Any(value) = value else {
                        return Err(OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "XML element attribute resolved to a non-Any shared value",
                        ));
                    };
                    *traversal_work = traversal_work
                        .checked_add(key.len())
                        .and_then(|work| work.checked_add(any_traversal_work(&value)?))
                        .ok_or_else(|| {
                            OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "XML attribute traversal work overflow",
                            )
                        })?;
                    attrs.push((Arc::<str>::from(key), value));
                }
                let sort_partitions = binary_partition_work(attrs.len());
                let sort_key_work = attrs.iter().try_fold(0usize, |work, (key, _)| {
                    work.checked_add(key.len().checked_mul(sort_partitions)?)
                });
                let sort_work = attrs
                    .len()
                    .checked_mul(sort_partitions)
                    .and_then(|work| work.checked_add(sort_key_work?));
                *traversal_work = traversal_work
                    .checked_add(sort_work.ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "XML attribute sort work overflow",
                        )
                    })?)
                    .ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "XML attribute sort work overflow",
                        )
                    })?;
                attrs.sort_by(|(left, _), (right, _)| left.cmp(right));
                Ok(StorageChildKind::Element {
                    target: element.clone(),
                    signature: Arc::new(ElementSignature {
                        target: AsRef::<Branch>::as_ref(element).id(),
                        path: element_path,
                        tag: element.tag().clone(),
                        attrs,
                    }),
                })
            }
            XmlOut::Fragment(_) => Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "nested XML fragments are not valid structural document children",
            )),
        })
        .collect::<OperationResult<Vec<_>>>()?;
    if output
        .insert(
            semantic_path.clone(),
            StructuralParentTarget {
                parent: parent.clone(),
                signature,
                storage_children,
            },
        )
        .is_some()
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "duplicate semantic structural parent path",
        ));
    }
    for (child_index, child) in children.into_iter().enumerate() {
        let XmlOut::Element(element) = child else {
            continue;
        };
        if wire_element_is_semantic_void(&element, txn, schema) {
            continue;
        }
        let child_index = u32::try_from(child_index).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "structural child index exceeds u32",
            )
        })?;
        let mut child_semantic_path = semantic_path.clone();
        child_semantic_path.push(child_index);
        let mut child_branch_path = Vec::with_capacity(branch_path.len() + 1);
        child_branch_path.push((parent_id.clone(), child_index));
        child_branch_path.extend_from_slice(&branch_path);
        collect_structural_parents(
            request_id,
            txn,
            XmlParentRef::Element(element),
            child_semantic_path,
            child_branch_path,
            schema,
            traversal_work,
            materialized_texts,
            output,
        )?;
    }
    Ok(())
}

struct TextTargetContext<'a, T> {
    request_id: u64,
    txn: &'a T,
    schema: &'a Schema,
}

struct TextTargetParent<'a> {
    id: BranchID,
    ancestors: &'a [(BranchID, u32)],
}

fn collect_text_targets<'a, T: ReadTxn>(
    context: &TextTargetContext<'_, T>,
    children: impl Iterator<Item = (u32, XmlOut)> + 'a,
    parent: TextTargetParent<'_>,
    mut position: u32,
    traversal_work: &mut usize,
    materialized_texts: &mut HashMap<BranchID, MaterializedText>,
    output: &mut Vec<LocatedTarget>,
) -> OperationResult<u32> {
    let request_id = context.request_id;
    let txn = context.txn;
    let schema = context.schema;
    let parent_id = parent.id;
    let ancestor_path = parent.ancestors;
    for (child_index, child) in children {
        *traversal_work = traversal_work.checked_add(1).ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs mutation target traversal work overflow",
            )
        })?;
        let mut child_path = Vec::with_capacity(ancestor_path.len() + 1);
        child_path.push((parent_id.clone(), child_index));
        child_path.extend_from_slice(ancestor_path);
        *traversal_work = traversal_work
            .checked_add(child_path.len())
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Yrs mutation path capture work overflow",
                )
            })?;
        match child {
            XmlOut::Text(text) => {
                *traversal_work =
                    traversal_work
                        .checked_add(child_path.len())
                        .ok_or_else(|| {
                            OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "Yrs text signature preflight work overflow",
                            )
                        })?;
                let materialized = materialize_text(request_id, &text, txn)?;
                *traversal_work =
                    traversal_work
                        .checked_add(materialized.work)
                        .ok_or_else(|| {
                            OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "Yrs text materialization work overflow",
                            )
                        })?;
                let target_id = <XmlTextRef as AsRef<Branch>>::as_ref(&text).id();
                if materialized_texts
                    .insert(target_id.clone(), materialized.clone())
                    .is_some()
                {
                    return Err(OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "duplicate Yrs text materialization",
                    ));
                }
                output.push(LocatedTarget::Existing {
                    start: position,
                    signature: TargetSignature {
                        target: target_id,
                        path: child_path,
                        initial_len_utf16: materialized.utf16_len,
                        runs: materialized.signature_runs.clone(),
                        capture_work: materialized.work,
                    },
                    target: text,
                    text: materialized.text.clone(),
                    scalar_len: materialized.scalar_len,
                });
                position = position
                    .checked_add(materialized.scalar_len)
                    .ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs mutation target position overflow",
                        )
                    })?;
            }
            XmlOut::Element(element) => {
                if wire_element_is_semantic_void(&element, txn, schema) {
                    position = position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs void target position overflow",
                        )
                    })?;
                } else {
                    position = position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs element target position overflow",
                        )
                    })?;
                    let is_textblock = schema
                        .node(element.tag().as_ref())
                        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock));
                    if is_textblock {
                        let children = element.children(txn).collect::<Vec<_>>();
                        *traversal_work = traversal_work
                            .checked_add(children.len())
                            .and_then(|work| work.checked_add(children.len()))
                            .and_then(|work| work.checked_add(child_path.len()))
                            .ok_or_else(|| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs textblock materialization work overflow",
                                )
                            })?;
                        let child_count = u32::try_from(children.len()).map_err(|_| {
                            OperationError::engine_invariant_failed(
                                request_id,
                                None,
                                "Yrs textblock child count exceeds u32",
                            )
                        })?;
                        let mut previous_was_text = false;
                        for (index, child) in children.iter().cloned().enumerate() {
                            let index = u32::try_from(index).map_err(|_| {
                                OperationError::engine_invariant_failed(
                                    request_id,
                                    None,
                                    "Yrs textblock child index exceeds u32",
                                )
                            })?;
                            let child_is_text = matches!(child, XmlOut::Text(_));
                            if !previous_was_text && !child_is_text {
                                *traversal_work = traversal_work
                                    .checked_add(child_path.len())
                                    .ok_or_else(|| {
                                        OperationError::engine_invariant_failed(
                                            request_id,
                                            None,
                                            "Yrs missing-gap signature work overflow",
                                        )
                                    })?;
                                output.push(LocatedTarget::Missing {
                                    start: position,
                                    parent: element.clone(),
                                    child_index: index,
                                    signature: parent_signature_from_children(
                                        &element,
                                        &child_path,
                                        &children,
                                        child_count,
                                        index,
                                    ),
                                });
                            }
                            position = collect_text_targets(
                                context,
                                std::iter::once((index, child)),
                                TextTargetParent {
                                    id: <XmlElementRef as AsRef<Branch>>::as_ref(&element).id(),
                                    ancestors: &child_path,
                                },
                                position,
                                traversal_work,
                                materialized_texts,
                                output,
                            )?;
                            previous_was_text = child_is_text;
                        }
                        if !previous_was_text {
                            *traversal_work = traversal_work
                                .checked_add(child_path.len())
                                .ok_or_else(|| {
                                    OperationError::engine_invariant_failed(
                                        request_id,
                                        None,
                                        "Yrs missing-gap signature work overflow",
                                    )
                                })?;
                            output.push(LocatedTarget::Missing {
                                start: position,
                                parent: element.clone(),
                                child_index: child_count,
                                signature: parent_signature_from_children(
                                    &element,
                                    &child_path,
                                    &children,
                                    child_count,
                                    child_count,
                                ),
                            });
                        }
                    } else {
                        position = collect_text_targets(
                            context,
                            (0u32..).zip(element.children(txn)),
                            TextTargetParent {
                                id: <XmlElementRef as AsRef<Branch>>::as_ref(&element).id(),
                                ancestors: &child_path,
                            },
                            position,
                            traversal_work,
                            materialized_texts,
                            output,
                        )?;
                    }
                    position = position.checked_add(1).ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "Yrs element target position overflow",
                        )
                    })?;
                }
            }
            XmlOut::Fragment(fragment) => {
                position = collect_text_targets(
                    context,
                    (0u32..).zip(fragment.children(txn)),
                    TextTargetParent {
                        id: <XmlFragmentRef as AsRef<Branch>>::as_ref(&fragment).id(),
                        ancestors: &child_path,
                    },
                    position,
                    traversal_work,
                    materialized_texts,
                    output,
                )?;
            }
        }
    }
    Ok(position)
}
