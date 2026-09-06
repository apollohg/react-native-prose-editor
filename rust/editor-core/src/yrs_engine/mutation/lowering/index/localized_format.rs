impl LocalizedFormatCompiler {
    pub(crate) fn charge_format_boundary_node(
        &mut self,
        operation_index: usize,
    ) -> OperationResult<()> {
        self.compiler.charge_boundary_node(operation_index)
    }

    pub(crate) fn charge_format_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        self.compiler
            .charge_boundary_text(operation_index, text_bytes)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn try_new<T: ReadTxn>(
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        action_limit: usize,
        scan_limit: usize,
        scan_work: usize,
        locator: LocalizedFormatLocator<'_>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Option<Self>> {
        let seed = locator.seed;
        if !seed.matches_storage(
            txn,
            fragment,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        ) {
            return Ok(None);
        }
        let Some(seed_payload) = seed.ready_payload() else {
            return Ok(None);
        };
        let document_guard = capture_document_guard(request_id, txn)?;
        let Some(LocalizedTextblockTargets {
            targets,
            path_parent_widths,
        }) = localized_existing_textblock_targets(
            request_id,
            txn,
            fragment,
            schema,
            LocalizedTextblockLocator::Format(locator),
        )?
        else {
            return Ok(None);
        };
        if path_parent_widths.iter().any(|(parent, width)| {
            seed_payload.path_parent_widths.get(parent).copied() != Some(*width)
        }) || targets.iter().any(|target| match &target.kind {
            ResolvedTargetKind::Existing { signature, .. } => {
                seed_payload
                    .target_materialization_work
                    .get(&signature.target)
                    .copied()
                    != Some(signature.capture_work)
            }
            ResolvedTargetKind::Missing { .. } | ResolvedTargetKind::Prepared { .. } => true,
        }) {
            return Ok(None);
        }
        #[cfg(test)]
        LOCALIZED_RANGE_FORMAT_HIT_COUNT
            .set(LOCALIZED_RANGE_FORMAT_HIT_COUNT.get().saturating_add(1));
        Ok(Some(Self {
            compiler: MutationCompiler {
                request_id,
                document_guard,
                targets,
                structural_parents: HashMap::new(),
                actions: Vec::new(),
                prepared_inserts: Vec::new(),
                prepared_elements: HashMap::new(),
                charged_work: 0,
                pending_traversal_work: seed_payload.pending_traversal_work,
                localized_position_target_count: Some(seed_payload.target_count),
                explicit_path_parent_widths: Some(path_parent_widths),
                action_limit,
                scan_work,
                scan_limit,
                #[cfg(test)]
                position_resolver_work: 0,
                created_gap_shifts: HashMap::new(),
                pending_element_attrs: HashMap::new(),
                wrap_checkpoints: HashMap::new(),
                #[cfg(test)]
                virtual_delete_visits: 0,
            },
            seed_pending_traversal_work: seed_payload.pending_traversal_work,
            seed_materialization_work: Arc::clone(&seed_payload.target_materialization_work),
        }))
    }

    pub(crate) fn format(
        mut self,
        operation_index: usize,
        from: u32,
        to: u32,
        boundaries: &[u32],
        attrs: Attrs,
    ) -> OperationResult<(YrsMutationPlan, MutationLookupPromotion)> {
        let base_pending_traversal_work = self.seed_pending_traversal_work;
        self.compiler
            .format(operation_index, from, to, boundaries, attrs)?;

        let mut materialization_work_updates = Vec::new();
        materialization_work_updates
            .try_reserve(self.compiler.actions.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion allocation failed",
                )
            })?;
        let mut promoted_targets = HashSet::new();
        promoted_targets
            .try_reserve(self.compiler.actions.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion allocation failed",
                )
            })?;
        let mut current_materialization_work = HashMap::new();
        current_materialization_work
            .try_reserve(self.compiler.targets.len())
            .map_err(|_| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion allocation failed",
                )
            })?;
        for target in &self.compiler.targets {
            #[cfg(test)]
            LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT.set(
                LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT
                    .get()
                    .saturating_add(1),
            );
            let ResolvedTargetKind::Existing { signature, .. } = &target.kind else {
                return Err(OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion contains a non-existing target",
                ));
            };
            let next_work =
                prepared_materialization_work(self.compiler.request_id, &target.current_runs)?;
            if current_materialization_work
                .insert(signature.target.clone(), next_work)
                .is_some()
            {
                return Err(OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion contains a duplicate target",
                ));
            }
        }
        for slot in &self.compiler.actions {
            let ActionSlot::Concrete(action) = slot else {
                continue;
            };
            let YrsMutationAction::FormatText { signature, .. } = action.as_ref() else {
                continue;
            };
            if !promoted_targets.insert(signature.target.clone()) {
                continue;
            }
            if self
                .seed_materialization_work
                .get(&signature.target)
                .copied()
                != Some(signature.capture_work)
            {
                return Err(OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion does not match its seed",
                ));
            }
            let next_work = current_materialization_work
                .get(&signature.target)
                .copied()
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        self.compiler.request_id,
                        Some(operation_index),
                        "localized format promotion target is absent from the sealed compiler",
                    )
                })?;
            materialization_work_updates.push((
                signature.target.clone(),
                signature.capture_work,
                next_work,
            ));
        }
        if materialization_work_updates.is_empty() {
            return Err(OperationError::engine_invariant_failed(
                self.compiler.request_id,
                Some(operation_index),
                "localized format produced no existing-text action",
            ));
        }
        let (old_work, new_work) = materialization_work_updates
            .iter()
            .try_fold((0usize, 0usize), |(old_total, new_total), (_, old, new)| {
                Some((old_total.checked_add(*old)?, new_total.checked_add(*new)?))
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion work overflow",
                )
            })?;
        let next_pending_traversal_work = base_pending_traversal_work
            .checked_sub(old_work)
            .and_then(|work| work.checked_add(new_work))
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    self.compiler.request_id,
                    Some(operation_index),
                    "localized format promotion work overflow",
                )
            })?;
        let promotion = MutationLookupPromotion {
            request_id: self.compiler.request_id,
            source: MutationLookupPromotionSource::ExistingFormat,
            materialization_work_updates,
            next_pending_traversal_work,
        };
        self.compiler
            .finish(Some(operation_index))
            .map(|plan| (plan, promotion))
    }
}

fn prepared_materialization_work(
    request_id: u64,
    runs: &[PreparedTextRun],
) -> OperationResult<usize> {
    let overflow = || {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "Yrs XML text materialization work overflow",
        )
    };
    runs.iter()
        .filter(|run| !run.text.is_empty())
        .try_fold(0usize, |work, run| {
            let attrs = &run.attrs;
            let attr_work = attrs.iter().try_fold(attrs.len(), |work, (key, value)| {
                work.checked_add(key.len())
                    .and_then(|work| work.checked_add(super::plan::any_preflight_work(value)?))
            });
            let sort_partitions = binary_partition_work(attrs.len());
            let sort_key_work = attrs.iter().try_fold(0usize, |work, (key, _)| {
                work.checked_add(key.len().checked_mul(sort_partitions)?)
            });
            work.checked_add(attr_work.ok_or_else(overflow)?)
                .and_then(|work| {
                    work.checked_add(
                        attrs
                            .len()
                            .checked_mul(sort_partitions)?
                            .checked_add(sort_key_work?)?,
                    )
                })
                .and_then(|work| work.checked_add(run.text.len()))
                .and_then(|work| work.checked_add(1))
                .ok_or_else(overflow)
        })
}

struct LocalizedTextblockTargets {
    targets: Vec<ResolvedText>,
    path_parent_widths: HashMap<BranchID, usize>,
}

#[derive(Clone, Copy)]
enum LocalizedTextblockLocator<'a> {
    Insert(LocalizedInsertLocator<'a>),
    Format(LocalizedFormatLocator<'a>),
}

impl<'a> LocalizedTextblockLocator<'a> {
    fn document(self) -> &'a Document {
        match self {
            Self::Insert(locator) => locator.document,
            Self::Format(locator) => locator.document,
        }
    }

    fn block_path(self) -> &'a [u32] {
        match self {
            Self::Insert(locator) => locator.block_path,
            Self::Format(locator) => locator.block_path,
        }
    }
}

fn localized_existing_textblock_targets<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
    locator: LocalizedTextblockLocator<'_>,
) -> OperationResult<Option<LocalizedTextblockTargets>> {
    let Some((semantic_block, block_start, block_end)) =
        semantic_node_bounds(locator.document(), locator.block_path())
    else {
        return Ok(None);
    };
    let valid_range = match locator {
        LocalizedTextblockLocator::Insert(locator) => {
            locator.position >= block_start && locator.position <= block_end
        }
        LocalizedTextblockLocator::Format(locator) => {
            locator.from < locator.to && locator.from >= block_start && locator.to <= block_end
        }
    };
    if !valid_range {
        return Ok(None);
    }
    if semantic_block.is_void()
        || !schema
            .node(semantic_block.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
        || semantic_block.content().is_none_or(|content| {
            content.child_count() == 0 || content.iter().any(|child| !child.is_text())
        })
    {
        return Ok(None);
    }

    let mut parent = XmlParentRef::Fragment(fragment.clone());
    let mut branch_path = Vec::<(BranchID, u32)>::new();
    let mut path_parent_widths = HashMap::<BranchID, usize>::new();
    let mut semantic_path = Vec::<u32>::new();
    for &child_index in locator.block_path() {
        let children = match &parent {
            XmlParentRef::Fragment(parent) => parent.children(txn).collect::<Vec<_>>(),
            XmlParentRef::Element(parent) => parent.children(txn).collect::<Vec<_>>(),
        };
        path_parent_widths.insert(parent.id(), children.len());
        let Some(child) = usize::try_from(child_index)
            .ok()
            .and_then(|index| children.get(index))
        else {
            return Ok(None);
        };
        let XmlOut::Element(element) = child else {
            return Ok(None);
        };
        semantic_path.push(child_index);
        let Some(semantic_node) = locator.document().node_at(&semantic_path) else {
            return Ok(None);
        };
        if super::super::codec::wire_element_node_spec(element, txn, schema)
            .is_none_or(|spec| semantic_node.node_type() != spec.name)
            || wire_element_is_semantic_void(element, txn, schema)
        {
            return Ok(None);
        }
        branch_path.insert(0, (parent.id(), child_index));
        parent = XmlParentRef::Element(element.clone());
    }
    let XmlParentRef::Element(textblock) = parent else {
        return Ok(None);
    };
    let children = textblock.children(txn).collect::<Vec<_>>();
    path_parent_widths.insert(AsRef::<Branch>::as_ref(&textblock).id(), children.len());
    if children.is_empty()
        || children
            .iter()
            .any(|child| !matches!(child, XmlOut::Text(_)))
    {
        return Ok(None);
    }

    let mut located = Vec::new();
    let mut materialized_texts = HashMap::new();
    let mut traversal_work = 0usize;
    let context = TextTargetContext {
        request_id,
        txn,
        schema,
    };
    collect_text_targets(
        &context,
        (0u32..).zip(children),
        TextTargetParent {
            id: AsRef::<Branch>::as_ref(&textblock).id(),
            ancestors: &branch_path,
        },
        block_start,
        &mut traversal_work,
        &mut materialized_texts,
        &mut located,
    )?;
    if located.is_empty()
        || located
            .iter()
            .any(|target| !matches!(target, LocatedTarget::Existing { .. }))
    {
        return Ok(None);
    }
    if let LocalizedTextblockLocator::Insert(locator) = locator {
        let matching_targets = located
            .iter()
            .filter(|target| {
                let LocatedTarget::Existing {
                    start, scalar_len, ..
                } = target
                else {
                    return false;
                };
                start
                    .checked_add(*scalar_len)
                    .is_some_and(|end| locator.position >= *start && locator.position <= end)
            })
            .count();
        if matching_targets != 1 {
            return Ok(None);
        }
    }
    let combined_text = located
        .iter()
        .try_fold(String::new(), |mut combined, target| {
            let LocatedTarget::Existing { text, .. } = target else {
                return None;
            };
            combined.push_str(text);
            Some(combined)
        });
    if combined_text.as_deref() != Some(semantic_block.text_content().as_str()) {
        return Ok(None);
    }
    let mut previous_end = 0u32;
    let mut targets = Vec::with_capacity(located.len());
    for located in located {
        let LocatedTarget::Existing {
            start,
            target,
            text,
            scalar_len,
            signature,
        } = located
        else {
            return Ok(None);
        };
        let Some(materialized) = materialized_texts.get(&AsRef::<Branch>::as_ref(&target).id())
        else {
            return Ok(None);
        };
        let Some(gap_before) = start.checked_sub(previous_end) else {
            return Ok(None);
        };
        let Some(end) = start.checked_add(scalar_len) else {
            return Ok(None);
        };
        previous_end = end;
        targets.push(ResolvedText {
            kind: ResolvedTargetKind::Existing { target, signature },
            gap_before,
            text,
            scalar_len,
            base_runs: materialized.prepared_runs.clone(),
            current_runs: materialized.prepared_runs.clone(),
            action_slots: Vec::new(),
        });
    }
    Ok(Some(LocalizedTextblockTargets {
        targets,
        path_parent_widths,
    }))
}

fn semantic_node_bounds<'a>(
    document: &'a Document,
    node_path: &[u32],
) -> Option<(&'a Node, u32, u32)> {
    let mut parent = document.root();
    let mut parent_content_start = 0u32;
    for (depth, &raw_index) in node_path.iter().enumerate() {
        let index = usize::try_from(raw_index).ok()?;
        let content = parent.content()?;
        let child_start = content
            .iter()
            .take(index)
            .try_fold(parent_content_start, |position, sibling| {
                position.checked_add(sibling.node_size())
            })?;
        let child = content.child(index)?;
        let child_content_start = child_start.checked_add(u32::from(child.is_element()))?;
        if depth + 1 == node_path.len() {
            let child_content_end = child_content_start.checked_add(child.content()?.size())?;
            return Some((child, child_content_start, child_content_end));
        }
        parent = child;
        parent_content_start = child_content_start;
    }
    None
}
