fn max_cached_elements(limits: &ResourceLimits) -> Result<usize, CachedRenderError> {
    limits
        .max_document_nodes
        .checked_mul(3)
        .ok_or(CachedRenderError::ResourceLimitExceeded)
}

fn ordered_list_start(node: &Node) -> Result<u32, CachedRenderError> {
    match node.attrs().get("start") {
        None => Ok(1),
        Some(start) => start
            .as_u64()
            .ok_or(CachedRenderError::PositionOverflow)
            .and_then(|start| {
                u32::try_from(start).map_err(|_| CachedRenderError::PositionOverflow)
            }),
    }
}

fn ensure_document_render_limits(
    document: &Document,
    schema: &Schema,
    limits: &ResourceLimits,
) -> Result<(), CachedRenderError> {
    #[cfg(test)]
    crate::yrs_engine::observability::record_render_limit_tree_scan();
    let mut stack = Vec::new();
    stack
        .try_reserve(1)
        .map_err(|_| CachedRenderError::AllocationFailed)?;
    stack.push((document.root(), 1usize));
    let mut count = 0usize;
    while let Some((node, depth)) = stack.pop() {
        count = count
            .checked_add(1)
            .ok_or(CachedRenderError::ResourceLimitExceeded)?;
        if count > limits.max_document_nodes
            || depth > limits.max_document_depth
            || depth > usize::from(u16::MAX)
        {
            return Err(CachedRenderError::ResourceLimitExceeded);
        }
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::List { ordered: true }))
        {
            let start = ordered_list_start(node)?;
            let total = u32::try_from(node.child_count())
                .map_err(|_| CachedRenderError::PositionOverflow)?;
            if total > 0 {
                start
                    .checked_add(total - 1)
                    .ok_or(CachedRenderError::PositionOverflow)?;
            }
        }
        let remaining_nodes = limits
            .max_document_nodes
            .checked_sub(count)
            .ok_or(CachedRenderError::ResourceLimitExceeded)?;
        let pending_nodes = stack
            .len()
            .checked_add(node.child_count())
            .ok_or(CachedRenderError::ResourceLimitExceeded)?;
        if pending_nodes > remaining_nodes {
            return Err(CachedRenderError::ResourceLimitExceeded);
        }
        stack
            .try_reserve_exact(node.child_count())
            .map_err(|_| CachedRenderError::AllocationFailed)?;
        let child_depth = depth
            .checked_add(1)
            .ok_or(CachedRenderError::ResourceLimitExceeded)?;
        for index in 0..node.child_count() {
            stack.push((
                node.child(index)
                    .ok_or(CachedRenderError::CacheInvariantViolation)?,
                child_depth,
            ));
        }
    }
    Ok(())
}

fn ensure_document_render_arithmetic(
    document: &Document,
    schema: &Schema,
) -> Result<(), CachedRenderError> {
    let mut stack = Vec::new();
    stack
        .try_reserve(1)
        .map_err(|_| CachedRenderError::AllocationFailed)?;
    stack.push(document.root());
    while let Some(node) = stack.pop() {
        if schema
            .node(node.node_type())
            .is_some_and(|spec| matches!(spec.role, NodeRole::List { ordered: true }))
        {
            let start = ordered_list_start(node)?;
            let total = u32::try_from(node.child_count())
                .map_err(|_| CachedRenderError::PositionOverflow)?;
            if total > 0 {
                start
                    .checked_add(total - 1)
                    .ok_or(CachedRenderError::PositionOverflow)?;
            }
        }
        stack
            .try_reserve(node.child_count())
            .map_err(|_| CachedRenderError::AllocationFailed)?;
        for index in 0..node.child_count() {
            stack.push(
                node.child(index)
                    .ok_or(CachedRenderError::CacheInvariantViolation)?,
            );
        }
    }
    Ok(())
}

fn checked_top_level_starts(
    document: &Document,
    limits: &ResourceLimits,
) -> Result<Vec<u32>, CachedRenderError> {
    #[cfg(test)]
    crate::yrs_engine::observability::record_render_top_level_start_scan();
    let root = document.root();
    if root.child_count() > limits.max_document_nodes {
        return Err(CachedRenderError::ResourceLimitExceeded);
    }
    let mut starts = Vec::new();
    starts
        .try_reserve_exact(root.child_count())
        .map_err(|_| CachedRenderError::AllocationFailed)?;
    let mut start = 0u32;
    for index in 0..root.child_count() {
        starts.push(start);
        start = start
            .checked_add(
                root.child(index)
                    .ok_or(CachedRenderError::CacheInvariantViolation)?
                    .node_size(),
            )
            .ok_or(CachedRenderError::PositionOverflow)?;
    }
    Ok(starts)
}

fn render_cached_block(
    node: &Node,
    schema: &Schema,
    start_pos: u32,
) -> Result<CachedRenderBlock, CachedRenderError> {
    let expected_end = start_pos
        .checked_add(node.node_size())
        .ok_or(CachedRenderError::PositionOverflow)?;
    let mut elements = Vec::new();
    elements
        .try_reserve(3)
        .map_err(|_| CachedRenderError::AllocationFailed)?;
    let mut rendered_end = start_pos;
    generate_block(node, schema, &mut elements, &mut rendered_end, 0, None, 0)?;
    if rendered_end != expected_end {
        return Err(CachedRenderError::CacheInvariantViolation);
    }
    let mut position_element_indices = Vec::new();
    position_element_indices
        .try_reserve(elements.len())
        .map_err(|_| CachedRenderError::AllocationFailed)?;
    for (index, element) in elements.iter().enumerate() {
        if render_element_doc_pos(element).is_some() {
            position_element_indices.push(index);
        }
    }
    Ok(CachedRenderBlock {
        node: Arc::new(node.clone()),
        start_pos,
        node_size: node.node_size(),
        elements: Arc::new(elements),
        position_element_indices: Arc::new(position_element_indices),
    })
}

fn render_element_doc_pos(element: &RenderElement) -> Option<u32> {
    match element {
        RenderElement::VoidInline { doc_pos, .. }
        | RenderElement::VoidBlock { doc_pos, .. }
        | RenderElement::OpaqueInlineAtom { doc_pos, .. }
        | RenderElement::OpaqueBlockAtom { doc_pos, .. } => Some(*doc_pos),
        RenderElement::TextRun { .. }
        | RenderElement::BlockStart { .. }
        | RenderElement::BlockEnd => None,
    }
}

fn set_render_element_doc_pos(element: &mut RenderElement, doc_pos: u32) -> bool {
    match element {
        RenderElement::VoidInline {
            doc_pos: current, ..
        }
        | RenderElement::VoidBlock {
            doc_pos: current, ..
        }
        | RenderElement::OpaqueInlineAtom {
            doc_pos: current, ..
        }
        | RenderElement::OpaqueBlockAtom {
            doc_pos: current, ..
        } => {
            *current = doc_pos;
            true
        }
        RenderElement::TextRun { .. }
        | RenderElement::BlockStart { .. }
        | RenderElement::BlockEnd => false,
    }
}

fn rebase_cached_block(
    old_block: &CachedRenderBlock,
    new_node: &Node,
    new_start: u32,
) -> Option<CachedRenderBlock> {
    if old_block.node.as_ref() != new_node || old_block.node_size != new_node.node_size() {
        return None;
    }
    let delta = i64::from(new_start) - i64::from(old_block.start_pos);
    let elements = if delta == 0 || old_block.position_element_indices.is_empty() {
        Arc::clone(&old_block.elements)
    } else {
        let mut rebased = old_block.elements.as_ref().clone();
        for index in old_block.position_element_indices.iter() {
            let element = rebased.get_mut(*index)?;
            let old_pos = render_element_doc_pos(element)?;
            let new_pos = u32::try_from(i64::from(old_pos).checked_add(delta)?).ok()?;
            if !set_render_element_doc_pos(element, new_pos) {
                return None;
            }
        }
        Arc::new(rebased)
    };
    Some(CachedRenderBlock {
        node: Arc::clone(&old_block.node),
        start_pos: new_start,
        node_size: new_node.node_size(),
        elements,
        position_element_indices: Arc::clone(&old_block.position_element_indices),
    })
}

fn classify_cached_transition(
    old_cache: &CachedRenderBlocks,
    new_cache: &CachedRenderBlocks,
    affected_indices: &[usize],
    document_changed: bool,
) -> CachedRenderTransitionUpdate {
    let old_len = old_cache.blocks.len();
    let new_len = new_cache.blocks.len();
    let widest_len = old_len.max(new_len);
    if affected_indices.iter().any(|index| *index >= widest_len) {
        return CachedRenderTransitionUpdate::Full(new_cache.materialize());
    }

    let mut prefix = 0usize;
    while prefix < old_len
        && prefix < new_len
        && old_cache.blocks[prefix].elements == new_cache.blocks[prefix].elements
    {
        prefix += 1;
    }
    let mut old_end = old_len;
    let mut new_end = new_len;
    while old_end > prefix
        && new_end > prefix
        && old_cache.blocks[old_end - 1].elements == new_cache.blocks[new_end - 1].elements
    {
        old_end -= 1;
        new_end -= 1;
    }
    if prefix == old_len && prefix == new_len {
        return if document_changed {
            CachedRenderTransitionUpdate::Full(new_cache.materialize())
        } else {
            CachedRenderTransitionUpdate::None
        };
    }

    let mut start = prefix;
    for index in affected_indices {
        start = start.min(*index);
        if *index < old_len {
            old_end = old_end.max(index.saturating_add(1));
        }
        if *index < new_len {
            new_end = new_end.max(index.saturating_add(1));
        }
    }
    if !cached_patch_reconstructs(old_cache, new_cache, start, old_end, new_end) {
        return CachedRenderTransitionUpdate::Full(new_cache.materialize());
    }

    CachedRenderTransitionUpdate::Patch(RenderBlocksPatch {
        start_index: start,
        delete_count: old_end.saturating_sub(start),
        blocks: new_cache.blocks[start..new_end]
            .iter()
            .map(|block| block.elements.as_ref().clone())
            .collect(),
    })
}

fn cached_patch_reconstructs(
    old_cache: &CachedRenderBlocks,
    new_cache: &CachedRenderBlocks,
    start: usize,
    old_end: usize,
    new_end: usize,
) -> bool {
    if start > old_end
        || start > new_end
        || old_end > old_cache.blocks.len()
        || new_end > new_cache.blocks.len()
    {
        return false;
    }
    if old_cache.blocks[..start]
        .iter()
        .zip(&new_cache.blocks[..start])
        .any(|(old, new)| old.elements != new.elements)
    {
        return false;
    }
    let old_suffix = &old_cache.blocks[old_end..];
    let new_suffix = &new_cache.blocks[new_end..];
    old_suffix.len() == new_suffix.len()
        && old_suffix
            .iter()
            .zip(new_suffix)
            .all(|(old, new)| old.elements == new.elements)
}
