#[allow(dead_code)]
impl CachedRenderBlocks {
    pub(crate) fn build(
        document: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> Result<Self, CachedRenderError> {
        record_cached_build();
        check_forced_cached_render_error()?;
        ensure_document_render_limits(document, schema, limits)?;
        let schema_fingerprint = Arc::<str>::from(schema_fingerprint(schema));
        Self::build_after_validation(document, schema, limits, schema_fingerprint)
    }

    /// Builds a cache from an exact document whose node/depth bounds and
    /// schema fingerprint have already been admitted by sealed validation
    /// evidence. Render-specific integer arithmetic, allocation, position,
    /// and element limits remain independently checked here.
    pub(crate) fn build_validated(
        document: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
        sealed_schema_fingerprint: &str,
        validated_node_count: usize,
        validated_max_depth: usize,
    ) -> Result<Self, CachedRenderError> {
        record_cached_build();
        check_forced_cached_render_error()?;
        if validated_node_count > limits.max_document_nodes
            || validated_max_depth > limits.max_document_depth
            || validated_max_depth > usize::from(u16::MAX)
        {
            return Err(CachedRenderError::ResourceLimitExceeded);
        }
        ensure_document_render_arithmetic(document, schema)?;
        Self::build_after_validation(
            document,
            schema,
            limits,
            Arc::<str>::from(sealed_schema_fingerprint),
        )
    }

    fn build_after_validation(
        document: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
        schema_fingerprint: Arc<str>,
    ) -> Result<Self, CachedRenderError> {
        let root = document.root();
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(root.child_count())
            .map_err(|_| CachedRenderError::AllocationFailed)?;

        let mut start_pos = 0u32;
        let mut element_count = 0usize;
        for index in 0..root.child_count() {
            let node = root
                .child(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let block = render_cached_block(node, schema, start_pos)?;
            element_count = element_count
                .checked_add(block.elements.len())
                .ok_or(CachedRenderError::ResourceLimitExceeded)?;
            if element_count > max_cached_elements(limits)? {
                return Err(CachedRenderError::ResourceLimitExceeded);
            }
            start_pos = start_pos
                .checked_add(node.node_size())
                .ok_or(CachedRenderError::PositionOverflow)?;
            blocks.push(block);
        }

        let cache = Self {
            blocks,
            document_root_seal: root.clone(),
            schema_fingerprint,
        };
        #[cfg(any(test, debug_assertions))]
        cache.assert_slow_invariant(document, schema);
        Ok(cache)
    }

    /// Reconstructs visible text directly from retained render elements,
    /// avoiding a deep clone of every cached block and element.
    pub(crate) fn rendered_text(&self, schema: &Schema) -> String {
        #[cfg(test)]
        crate::yrs_engine::observability::record_rendered_text_derivation();
        let mut text = String::new();
        let mut pending_prefix = String::new();
        let mut started_block = false;
        for element in self.blocks.iter().flat_map(|block| block.elements.iter()) {
            match element {
                RenderElement::BlockStart {
                    node_type,
                    list_context,
                    ..
                } => {
                    if let Some(context) = list_context {
                        pending_prefix = if context.kind.as_deref() == Some("task") {
                            crate::render::task_list_marker_string(context.checked.unwrap_or(false))
                        } else {
                            crate::render::list_marker_string(context.ordered, context.index)
                        };
                    }
                    if schema
                        .node(node_type)
                        .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
                    {
                        if started_block {
                            text.push('\n');
                        }
                        started_block = true;
                        text.push_str(&pending_prefix);
                        pending_prefix.clear();
                    }
                }
                RenderElement::TextRun { text: value, .. } => text.push_str(value),
                RenderElement::VoidInline { .. } => text.push('\n'),
                RenderElement::VoidBlock { .. } => {
                    if started_block {
                        text.push('\n');
                    }
                    started_block = true;
                    text.push('\u{fffc}');
                }
                RenderElement::OpaqueInlineAtom {
                    node_type, label, ..
                } => text.push_str(&crate::render::opaque_atom_visible_string(node_type, label)),
                RenderElement::OpaqueBlockAtom {
                    node_type, label, ..
                } => {
                    if started_block {
                        text.push('\n');
                    }
                    started_block = true;
                    text.push_str(&crate::render::opaque_atom_visible_string(node_type, label));
                }
                RenderElement::BlockEnd => {}
            }
        }
        text
    }

    pub(crate) fn materialize(&self) -> Vec<Vec<RenderElement>> {
        self.blocks
            .iter()
            .map(|block| block.elements.as_ref().clone())
            .collect()
    }

    pub(crate) fn history_snapshot_retained_bytes(&self) -> Option<usize> {
        fn json_map_bytes(
            values: &std::collections::HashMap<String, serde_json::Value>,
        ) -> Option<usize> {
            let table = crate::model::hash_table_retained_bytes::<String, serde_json::Value>(
                values.capacity(),
            )?;
            values.iter().try_fold(table, |total, (key, value)| {
                total
                    .checked_add(key.capacity())?
                    .checked_add(crate::model::json_value_retained_bytes(value)?)
            })
        }

        fn element_bytes(element: &RenderElement) -> Option<usize> {
            match element {
                RenderElement::TextRun { text, marks } => {
                    let slots = marks
                        .capacity()
                        .checked_mul(std::mem::size_of::<RenderMark>())?;
                    marks
                        .iter()
                        .try_fold(text.capacity().checked_add(slots)?, |total, mark| {
                            total
                                .checked_add(mark.mark_type.capacity())?
                                .checked_add(json_map_bytes(&mark.attrs)?)
                        })
                }
                RenderElement::VoidInline {
                    node_type, attrs, ..
                }
                | RenderElement::VoidBlock {
                    node_type, attrs, ..
                } => node_type.capacity().checked_add(json_map_bytes(attrs)?),
                RenderElement::OpaqueInlineAtom {
                    node_type,
                    label,
                    attrs,
                    mention_theme,
                    ..
                } => node_type
                    .capacity()
                    .checked_add(label.capacity())?
                    .checked_add(json_map_bytes(attrs)?)?
                    .checked_add(mention_theme.as_ref().map_or(Some(0), json_map_bytes)?),
                RenderElement::OpaqueBlockAtom {
                    node_type,
                    label,
                    attrs,
                    ..
                } => node_type
                    .capacity()
                    .checked_add(label.capacity())?
                    .checked_add(json_map_bytes(attrs)?),
                RenderElement::BlockStart {
                    node_type,
                    list_context,
                    ..
                } => node_type.capacity().checked_add(
                    list_context
                        .as_ref()
                        .and_then(|context| context.kind.as_ref())
                        .map_or(0, String::capacity),
                ),
                RenderElement::BlockEnd => Some(0),
            }
        }

        let block_slots = self
            .blocks
            .capacity()
            .checked_mul(std::mem::size_of::<CachedRenderBlock>())?;
        let blocks = self.blocks.iter().try_fold(block_slots, |total, block| {
            let node = crate::model::arc_allocation_retained_bytes(std::mem::size_of::<Node>())?
                .checked_add(block.node.history_snapshot_retained_bytes()?)?;
            let element_slots = block
                .elements
                .capacity()
                .checked_mul(std::mem::size_of::<RenderElement>())?;
            let elements = block
                .elements
                .iter()
                .try_fold(element_slots, |bytes, element| {
                    bytes.checked_add(element_bytes(element)?)
                })?;
            let elements = crate::model::arc_allocation_retained_bytes(std::mem::size_of::<
                Vec<RenderElement>,
            >())?
            .checked_add(elements)?;
            let position_indices =
                crate::model::arc_allocation_retained_bytes(std::mem::size_of::<Vec<usize>>())?
                    .checked_add(
                        block
                            .position_element_indices
                            .capacity()
                            .checked_mul(std::mem::size_of::<usize>())?,
                    )?;
            total
                .checked_add(node)?
                .checked_add(elements)?
                .checked_add(position_indices)
        })?;
        crate::model::arc_allocation_retained_bytes(std::mem::size_of::<Self>())?
            .checked_add(blocks)?
            .checked_add(self.document_root_seal.history_snapshot_retained_bytes()?)?
            .checked_add(crate::model::arc_allocation_retained_bytes(
                self.schema_fingerprint.len(),
            )?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn transition_localized_insert(
        &self,
        old_document: &Document,
        new_document: &Document,
        schema: &Schema,
        target_index: usize,
        inserted_scalars: u32,
        limits: &ResourceLimits,
    ) -> Result<CachedRenderTransition, CachedRenderError> {
        check_forced_cached_render_error()?;
        if self.schema_fingerprint.as_ref() != schema_fingerprint(schema)
            || !self.matches_document(old_document)
            || old_document.root().child_count() != new_document.root().child_count()
            || target_index >= self.blocks.len()
            || self.blocks.len() > limits.max_document_nodes
            || inserted_scalars == 0
        {
            return Err(CachedRenderError::CacheInvariantViolation);
        }

        let old_root = old_document.root();
        let new_root = new_document.root();
        let old_target_node = old_root
            .child(target_index)
            .ok_or(CachedRenderError::CacheInvariantViolation)?;
        let new_target_node = new_root
            .child(target_index)
            .ok_or(CachedRenderError::CacheInvariantViolation)?;
        let old_target_block = self
            .blocks
            .get(target_index)
            .ok_or(CachedRenderError::CacheInvariantViolation)?;
        let expected_target_size = old_target_node
            .node_size()
            .checked_add(inserted_scalars)
            .ok_or(CachedRenderError::PositionOverflow)?;
        if !old_target_block.node.shares_storage_with(old_target_node)
            || old_target_block.node_size != old_target_node.node_size()
            || new_target_node.node_size() != expected_target_size
        {
            return Err(CachedRenderError::CacheInvariantViolation);
        }

        check_forced_localized_render_allocation_failure()?;
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(self.blocks.len())
            .map_err(|_| CachedRenderError::AllocationFailed)?;
        let max_elements = max_cached_elements(limits)?;
        let mut element_count = 0usize;

        for index in 0..target_index {
            let old_node = old_root
                .child(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let new_node = new_root
                .child(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let block = self
                .blocks
                .get(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            if !old_node.shares_storage_with(new_node)
                || !block.node.shares_storage_with(old_node)
                || block.node.as_ref() != old_node
                || block.node_size != old_node.node_size()
                || block.node_size != new_node.node_size()
            {
                return Err(CachedRenderError::CacheInvariantViolation);
            }
            element_count = element_count
                .checked_add(block.elements.len())
                .ok_or(CachedRenderError::ResourceLimitExceeded)?;
            if element_count > max_elements {
                return Err(CachedRenderError::ResourceLimitExceeded);
            }
            blocks.push(block.clone());
        }

        let target_block =
            render_cached_block(new_target_node, schema, old_target_block.start_pos)?;
        element_count = element_count
            .checked_add(target_block.elements.len())
            .ok_or(CachedRenderError::ResourceLimitExceeded)?;
        check_forced_localized_render_resource_failure()?;
        if element_count > max_elements {
            return Err(CachedRenderError::ResourceLimitExceeded);
        }
        blocks.push(target_block);

        for index in target_index + 1..self.blocks.len() {
            let old_node = old_root
                .child(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let new_node = new_root
                .child(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let old_block = self
                .blocks
                .get(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            if !old_node.shares_storage_with(new_node)
                || !old_block.node.shares_storage_with(old_node)
                || old_block.node.as_ref() != old_node
                || old_block.node_size != old_node.node_size()
                || old_block.node_size != new_node.node_size()
            {
                return Err(CachedRenderError::CacheInvariantViolation);
            }
            check_forced_localized_render_position_failure()?;
            let new_start = old_block
                .start_pos
                .checked_add(inserted_scalars)
                .ok_or(CachedRenderError::PositionOverflow)?;
            let block = rebase_cached_block(old_block, new_node, new_start)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            element_count = element_count
                .checked_add(block.elements.len())
                .ok_or(CachedRenderError::ResourceLimitExceeded)?;
            if element_count > max_elements {
                return Err(CachedRenderError::ResourceLimitExceeded);
            }
            blocks.push(block);
        }

        check_forced_localized_render_invariant_failure()?;
        if blocks.len() != new_root.child_count() {
            return Err(CachedRenderError::CacheInvariantViolation);
        }
        let cache = Self {
            blocks,
            document_root_seal: new_root.clone(),
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
        };
        #[cfg(any(test, debug_assertions))]
        cache.assert_slow_invariant(new_document, schema);
        let update = classify_cached_transition(self, &cache, &[], true);
        record_cached_transition();
        record_cached_rerendered_blocks(1);
        Ok(CachedRenderTransition {
            cache,
            update,
            rerendered_new_blocks: 1,
        })
    }

    pub(crate) fn transition(
        &self,
        old_document: &Document,
        new_document: &Document,
        schema: &Schema,
        affected_indices: &[usize],
        limits: &ResourceLimits,
    ) -> Result<CachedRenderTransition, CachedRenderError> {
        record_cached_transition();
        check_forced_cached_render_error()?;
        ensure_document_render_limits(new_document, schema, limits)?;
        if self.schema_fingerprint.as_ref() != schema_fingerprint(schema) {
            return Self::full_transition(new_document, schema, limits);
        }
        if !self.matches_document(old_document) {
            return Self::full_transition(new_document, schema, limits);
        }
        if old_document == new_document {
            let cache = Self {
                blocks: self.blocks.clone(),
                document_root_seal: new_document.root().clone(),
                schema_fingerprint: Arc::clone(&self.schema_fingerprint),
            };
            #[cfg(any(test, debug_assertions))]
            cache.assert_slow_invariant(new_document, schema);
            return Ok(CachedRenderTransition {
                cache,
                update: CachedRenderTransitionUpdate::None,
                rerendered_new_blocks: 0,
            });
        }

        let old_root = old_document.root();
        let new_root = new_document.root();
        let old_len = old_root.child_count();
        let new_len = new_root.child_count();
        let mut prefix = 0usize;
        while prefix < old_len
            && prefix < new_len
            && old_root.child(prefix) == new_root.child(prefix)
        {
            prefix += 1;
        }

        let mut old_suffix = old_len;
        let mut new_suffix = new_len;
        while old_suffix > prefix
            && new_suffix > prefix
            && old_root.child(old_suffix - 1) == new_root.child(new_suffix - 1)
        {
            old_suffix -= 1;
            new_suffix -= 1;
        }

        let starts = match checked_top_level_starts(new_document, limits) {
            Ok(starts) => starts,
            Err(_) => return Self::full_transition(new_document, schema, limits),
        };
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(new_len)
            .map_err(|_| CachedRenderError::AllocationFailed)?;
        blocks.extend(self.blocks[..prefix].iter().cloned());

        let mut element_count = blocks.iter().try_fold(0usize, |total, block| {
            total
                .checked_add(block.elements.len())
                .ok_or(CachedRenderError::ResourceLimitExceeded)
        })?;
        for (index, start_pos) in starts.iter().enumerate().take(new_suffix).skip(prefix) {
            let node = new_root
                .child(index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let block = render_cached_block(node, schema, *start_pos)?;
            element_count = element_count
                .checked_add(block.elements.len())
                .ok_or(CachedRenderError::ResourceLimitExceeded)?;
            if element_count > max_cached_elements(limits)? {
                return Err(CachedRenderError::ResourceLimitExceeded);
            }
            blocks.push(block);
        }

        for (new_index, new_start) in starts.iter().enumerate().skip(new_suffix) {
            let suffix_offset = new_index - new_suffix;
            let old_index = old_suffix
                .checked_add(suffix_offset)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let Some(old_block) = self.blocks.get(old_index) else {
                return Self::full_transition(new_document, schema, limits);
            };
            let node = new_root
                .child(new_index)
                .ok_or(CachedRenderError::CacheInvariantViolation)?;
            let Some(block) = rebase_cached_block(old_block, node, *new_start) else {
                return Self::full_transition(new_document, schema, limits);
            };
            element_count = element_count
                .checked_add(block.elements.len())
                .ok_or(CachedRenderError::ResourceLimitExceeded)?;
            if element_count > max_cached_elements(limits)? {
                return Err(CachedRenderError::ResourceLimitExceeded);
            }
            blocks.push(block);
        }
        if blocks.len() != new_len {
            return Self::full_transition(new_document, schema, limits);
        }

        let cache = Self {
            blocks,
            document_root_seal: new_root.clone(),
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
        };
        #[cfg(any(test, debug_assertions))]
        cache.assert_slow_invariant(new_document, schema);
        let update = classify_cached_transition(
            self,
            &cache,
            affected_indices,
            old_document != new_document,
        );
        let rerendered_new_blocks = new_suffix.saturating_sub(prefix);
        record_cached_rerendered_blocks(rerendered_new_blocks);
        Ok(CachedRenderTransition {
            cache,
            update,
            rerendered_new_blocks,
        })
    }

    pub(crate) fn classify_transition_to(
        &self,
        old_document: &Document,
        new_document: &Document,
        new_cache: &Self,
        affected_indices: &[usize],
    ) -> CachedRenderTransitionUpdate {
        if self.schema_fingerprint != new_cache.schema_fingerprint
            || !self.matches_document(old_document)
            || !new_cache.matches_document(new_document)
        {
            return CachedRenderTransitionUpdate::Full(new_cache.materialize());
        }
        classify_cached_transition(
            self,
            new_cache,
            affected_indices,
            old_document != new_document,
        )
    }

    pub(crate) fn classify_cached_transition_to(
        &self,
        new_cache: &Self,
    ) -> CachedRenderTransitionUpdate {
        if self.schema_fingerprint != new_cache.schema_fingerprint {
            return CachedRenderTransitionUpdate::Full(new_cache.materialize());
        }
        classify_cached_transition(self, new_cache, &[], true)
    }

    pub(crate) fn matches_identity(&self, document: &Document, schema_fingerprint: &str) -> bool {
        self.schema_fingerprint.as_ref() == schema_fingerprint && self.matches_document(document)
    }

    #[cfg(any(test, debug_assertions))]
    fn verify_slow_invariant(&self, document: &Document, schema: &Schema) -> bool {
        if self.schema_fingerprint.as_ref() != schema_fingerprint(schema)
            || !self.document_root_seal.shares_storage_with(document.root())
        {
            return false;
        }
        let root = document.root();
        if self.blocks.len() != root.child_count() {
            return false;
        }
        let mut expected_start = 0u32;
        for (index, block) in self.blocks.iter().enumerate() {
            let Some(node) = root.child(index) else {
                return false;
            };
            if block.node.as_ref() != node
                || block.node_size != node.node_size()
                || block.start_pos != expected_start
            {
                return false;
            }
            let Some(next_start) = expected_start.checked_add(node.node_size()) else {
                return false;
            };
            expected_start = next_start;
        }
        true
    }

    #[cfg(any(test, debug_assertions))]
    fn assert_slow_invariant(&self, document: &Document, schema: &Schema) {
        #[cfg(test)]
        {
            SLOW_INVARIANT_CHECK_COUNT.set(SLOW_INVARIANT_CHECK_COUNT.get().saturating_add(1));
            assert!(self.verify_slow_invariant(document, schema));
        }
        #[cfg(all(not(test), debug_assertions))]
        debug_assert!(self.verify_slow_invariant(document, schema));
    }

    fn matches_document(&self, document: &Document) -> bool {
        self.document_root_seal.shares_storage_with(document.root())
    }

    fn full_transition(
        document: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> Result<CachedRenderTransition, CachedRenderError> {
        record_cached_full_transition();
        let cache = Self::build(document, schema, limits)?;
        #[cfg(any(test, debug_assertions))]
        cache.assert_slow_invariant(document, schema);
        let update = CachedRenderTransitionUpdate::Full(cache.materialize());
        let rerendered_new_blocks = cache.blocks.len();
        record_cached_rerendered_blocks(rerendered_new_blocks);
        Ok(CachedRenderTransition {
            cache,
            update,
            rerendered_new_blocks,
        })
    }
}
