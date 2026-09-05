use super::insert_admission::LocalizedInsertAdmission;
#[cfg(test)]
use super::observability::{
    forced_localized_index_allocation_stage, LocalizedIndexAllocationStage,
    FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE, FORCE_LOCALIZED_INDEX_BUDGET,
    LOCALIZED_INDEX_BUILD_COUNT, LOCALIZED_INDEX_BUILD_VISITS, LOCALIZED_INDEX_LOOKUP_COMPARISONS,
    LOCALIZED_INDEX_PATH_HOPS,
};
use super::validation::DocumentValidationCertificate;
use crate::boundary::ResourceLimits;
use crate::model::{Document, Mark};
use crate::position::build::{classify_position_block, PositionBlockKind};
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::yrs_engine::canonical::CanonicalArtifact;
use sha2::Digest;
use std::sync::Arc;

/// A rendered text leaf identity and its exact document/scalar/UTF-16 ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalizedTextLeafCertificate {
    pub(super) block_index: usize,
    pub(super) child_ordinal: u32,
    pub(super) doc_start: u32,
    pub(super) doc_end: u32,
    pub(super) scalar_start: u32,
    pub(super) scalar_end: u32,
    pub(super) utf16_start: u32,
    pub(super) utf16_end: u32,
    pub(super) text_sha256: [u8; 32],
    pub(super) text_scalars: u32,
    pub(super) text_utf16: u32,
    pub(super) text_utf8_bytes: usize,
    pub(super) marks_sha256: [u8; 32],
}

#[allow(dead_code)] // E1 evidence API is consumed by E2 and admission-oracle tests.
impl LocalizedTextLeafCertificate {
    pub(crate) fn doc_start(&self) -> u32 {
        self.doc_start
    }

    pub(crate) fn doc_end(&self) -> u32 {
        self.doc_end
    }

    pub(super) fn resolve<'a>(
        &self,
        document: &'a Document,
        position_map: &PositionMap,
    ) -> Option<&'a crate::model::Node> {
        let block = position_map.block(self.block_index)?;
        document
            .node_at(&block.node_path)?
            .content()?
            .child(usize::try_from(self.child_ordinal).ok()?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LocalizedTextLeafIndex {
    pub(super) leaves: Vec<LocalizedTextLeafCertificate>,
    pub(super) schema_fingerprint: Arc<str>,
    pub(super) canonical_artifact: CanonicalArtifact,
    pub(super) canonical_fingerprint: [u8; 32],
    pub(super) canonical_fingerprint_materialized: bool,
    pub(super) document_revision: u64,
    pub(super) retained_bytes: usize,
}

#[allow(dead_code)] // E1 evidence API is consumed by E2 and admission-oracle tests.
impl LocalizedTextLeafIndex {
    pub(super) fn build(
        document: &Document,
        position_map: &PositionMap,
        rendered_text: &str,
        validation: &DocumentValidationCertificate,
        resource_limits: &ResourceLimits,
        schema: &Schema,
    ) -> Option<Self> {
        #[cfg(test)]
        LOCALIZED_INDEX_BUILD_COUNT.set(LOCALIZED_INDEX_BUILD_COUNT.get().saturating_add(1));
        #[cfg(test)]
        if FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE.get() {
            return None;
        }
        if !position_map.has_effective_stored_bounds() {
            return None;
        }
        let cache_budget = resource_limits.max_input_bytes;
        #[cfg(test)]
        let cache_budget = FORCE_LOCALIZED_INDEX_BUDGET.get().unwrap_or(cache_budget);
        let path_bytes = validation
            .stats
            .max_depth
            .checked_mul(std::mem::size_of::<u32>())?;
        let leaf_budget = cache_budget.checked_sub(path_bytes)?;
        let leaf_size = std::mem::size_of::<LocalizedTextLeafCertificate>();
        let max_leaf_capacity = leaf_budget.checked_div(leaf_size)?;
        let initial_leaf_capacity = position_map
            .block_count()
            .min(resource_limits.max_document_nodes)
            .min(max_leaf_capacity);
        let mut leaves = Vec::new();
        #[cfg(test)]
        if forced_localized_index_allocation_stage(
            LocalizedIndexAllocationStage::InitialLeafCapacity,
        ) {
            return None;
        }
        leaves.try_reserve_exact(initial_leaf_capacity).ok()?;
        let initial_leaf_capacity_bytes = leaves.capacity().checked_mul(leaf_size)?;
        if initial_leaf_capacity_bytes > leaf_budget {
            return None;
        }
        let mut rendered_cursor = RenderedCursor::new(rendered_text);
        let mut retained_bytes = initial_leaf_capacity_bytes;
        let mut path = Vec::new();
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::TraversalPath) {
            return None;
        }
        path.try_reserve_exact(validation.stats.max_depth).ok()?;
        let path_capacity_bytes = path.capacity().checked_mul(std::mem::size_of::<u32>())?;
        if path_capacity_bytes.checked_add(retained_bytes)? > cache_budget {
            return None;
        }
        let mut next_block_index = 0usize;
        collect_localized_index_streamed(
            document.root(),
            &mut path,
            position_map,
            schema,
            0,
            &mut next_block_index,
            &mut leaves,
            &mut rendered_cursor,
            &mut retained_bytes,
            cache_budget,
            path_capacity_bytes,
        )?;
        if next_block_index != position_map.block_count() {
            return None;
        }
        Some(Self {
            leaves,
            schema_fingerprint: Arc::clone(&validation.schema_fingerprint),
            canonical_artifact: validation.canonical_artifact.clone(),
            canonical_fingerprint: validation.canonical_fingerprint,
            canonical_fingerprint_materialized: validation.canonical_fingerprint_materialized,
            document_revision: validation.document_revision,
            retained_bytes,
        })
    }

    pub(crate) fn leaves(&self) -> &[LocalizedTextLeafCertificate] {
        &self.leaves
    }

    pub(super) fn strict_inside(
        &self,
        document_position: u32,
    ) -> Option<&LocalizedTextLeafCertificate> {
        let mut low = 0usize;
        let mut high = self.leaves.len();
        while low < high {
            #[cfg(test)]
            LOCALIZED_INDEX_LOOKUP_COMPARISONS
                .set(LOCALIZED_INDEX_LOOKUP_COMPARISONS.get().saturating_add(1));
            let middle = low + (high - low) / 2;
            if self.leaves[middle].doc_end <= document_position {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        self.leaves
            .get(low)
            .filter(|leaf| leaf.doc_start < document_position && document_position < leaf.doc_end)
    }

    pub(super) fn matches(&self, validation: &DocumentValidationCertificate) -> bool {
        self.schema_fingerprint == validation.schema_fingerprint
            && if self.canonical_fingerprint_materialized
                && validation.canonical_fingerprint_materialized
            {
                self.canonical_fingerprint == validation.canonical_fingerprint
            } else {
                !self.canonical_fingerprint_materialized
                    && !validation.canonical_fingerprint_materialized
                    && self
                        .canonical_artifact
                        .ptr_eq(&validation.canonical_artifact)
            }
            && self.document_revision == validation.document_revision
    }

    // Keep the certificate and every sealed identity dimension explicit at this proof boundary.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn matches_materialized_identity(
        &self,
        validation: &DocumentValidationCertificate,
        canonical_artifact: &CanonicalArtifact,
        canonical_fingerprint: [u8; 32],
        canonical_serialized_len: usize,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> bool {
        self.schema_fingerprint == validation.schema_fingerprint
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.document_revision == document_revision
            && self.document_revision == validation.document_revision
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && validation.matches_materialized_identity(
                canonical_artifact,
                canonical_fingerprint,
                canonical_serialized_len,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
            && if self.canonical_fingerprint_materialized {
                self.canonical_fingerprint == canonical_fingerprint
            } else {
                canonical_artifact.sha256() == canonical_fingerprint
            }
    }

    pub(super) fn materialize_canonical_fingerprint(
        &mut self,
        validation: &DocumentValidationCertificate,
    ) {
        self.canonical_fingerprint = validation.canonical_fingerprint;
        self.canonical_fingerprint_materialized = true;
    }

    #[cfg(test)]
    pub(crate) fn canonical_fingerprint_materialized_for_test(&self) -> bool {
        self.canonical_fingerprint_materialized
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    #[cfg(test)]
    pub(crate) fn promotion_transient_budget_for_test(&self) -> Option<usize> {
        let mut promoted = Vec::<LocalizedTextLeafCertificate>::new();
        promoted.try_reserve_exact(self.leaves.len()).ok()?;
        let promoted_bytes = promoted
            .capacity()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        self.retained_bytes.checked_add(promoted_bytes)
    }

    pub(super) fn try_clone(&self, cache_budget: usize) -> Option<Self> {
        if self.retained_bytes > cache_budget {
            return None;
        }
        let required_bytes = self
            .leaves
            .len()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        let available_bytes = cache_budget.checked_sub(self.retained_bytes)?;
        if required_bytes > available_bytes {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(
            LocalizedIndexAllocationStage::InitialLeafCapacity,
        ) {
            return None;
        }
        let mut leaves = Vec::new();
        leaves.try_reserve_exact(self.leaves.len()).ok()?;
        let retained_bytes = leaves
            .capacity()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        if retained_bytes > available_bytes {
            return None;
        }
        leaves.extend_from_slice(&self.leaves);
        Some(Self {
            leaves,
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
            canonical_artifact: self.canonical_artifact.clone(),
            canonical_fingerprint: self.canonical_fingerprint,
            canonical_fingerprint_materialized: self.canonical_fingerprint_materialized,
            document_revision: self.document_revision,
            retained_bytes,
        })
    }

    pub(super) fn promote_existing_insert(
        &self,
        validation: &DocumentValidationCertificate,
        admission: &LocalizedInsertAdmission,
        block_path: &[u32],
        preview: &Document,
        canonical_artifact: &CanonicalArtifact,
        cache_budget: usize,
    ) -> Option<Self> {
        #[cfg(test)]
        if FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE.get() {
            return None;
        }
        if !self.matches(validation) || self.retained_bytes > cache_budget {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::PromotionClone) {
            return None;
        }
        let required_bytes = self
            .leaves
            .len()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        let available_bytes = cache_budget.checked_sub(self.retained_bytes)?;
        if required_bytes > available_bytes {
            return None;
        }
        let mut leaves = Vec::new();
        leaves.try_reserve_exact(self.leaves.len()).ok()?;
        let retained_bytes = leaves
            .capacity()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        if retained_bytes > available_bytes {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::PromotionGrowth) {
            return None;
        }
        leaves.extend_from_slice(&self.leaves);
        let target = self.strict_inside_index(admission.inserted_document_position)?;
        if leaves.get(target)? != &admission.leaf {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::PromotionUpdate) {
            return None;
        }
        let block = preview.node_at(block_path)?;
        let next_leaf = block
            .content()?
            .child(usize::try_from(admission.leaf.child_ordinal).ok()?)?;
        let next_text = next_leaf.text_str()?;
        let inserted_scalars = admission.inserted_scalars;
        let inserted_utf16 = admission.inserted_utf16;
        let inserted_utf8 = admission.inserted_utf8_bytes;
        let target_leaf = leaves.get_mut(target)?;
        target_leaf.doc_end = target_leaf.doc_end.checked_add(inserted_scalars)?;
        target_leaf.scalar_end = target_leaf.scalar_end.checked_add(inserted_scalars)?;
        target_leaf.utf16_end = target_leaf.utf16_end.checked_add(inserted_utf16)?;
        target_leaf.text_scalars = target_leaf.text_scalars.checked_add(inserted_scalars)?;
        target_leaf.text_utf16 = target_leaf.text_utf16.checked_add(inserted_utf16)?;
        target_leaf.text_utf8_bytes = target_leaf.text_utf8_bytes.checked_add(inserted_utf8)?;
        target_leaf.text_sha256 = sha2::Sha256::digest(next_text.as_bytes()).into();
        target_leaf.marks_sha256 = canonical_marks_sha256(next_leaf.marks())?;
        for leaf in leaves.iter_mut().skip(target + 1) {
            leaf.doc_start = leaf.doc_start.checked_add(inserted_scalars)?;
            leaf.doc_end = leaf.doc_end.checked_add(inserted_scalars)?;
            leaf.scalar_start = leaf.scalar_start.checked_add(inserted_scalars)?;
            leaf.scalar_end = leaf.scalar_end.checked_add(inserted_scalars)?;
            leaf.utf16_start = leaf.utf16_start.checked_add(inserted_utf16)?;
            leaf.utf16_end = leaf.utf16_end.checked_add(inserted_utf16)?;
        }
        Some(Self {
            leaves,
            schema_fingerprint: Arc::clone(&validation.schema_fingerprint),
            canonical_artifact: canonical_artifact.clone(),
            canonical_fingerprint: canonical_artifact.sha256(),
            canonical_fingerprint_materialized: true,
            document_revision: validation.document_revision,
            retained_bytes,
        })
    }

    pub(super) fn strict_inside_index(&self, document_position: u32) -> Option<usize> {
        let mut low = 0usize;
        let mut high = self.leaves.len();
        while low < high {
            let middle = low + (high - low) / 2;
            if self.leaves[middle].doc_end <= document_position {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        self.leaves.get(low).and_then(|leaf| {
            (leaf.doc_start < document_position && document_position < leaf.doc_end).then_some(low)
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_localized_index_streamed(
    node: &crate::model::Node,
    path: &mut Vec<u32>,
    position_map: &PositionMap,
    schema: &Schema,
    doc_offset: u32,
    next_block_index: &mut usize,
    leaves: &mut Vec<LocalizedTextLeafCertificate>,
    rendered_cursor: &mut RenderedCursor<'_>,
    retained_bytes: &mut usize,
    cache_budget: usize,
    path_capacity_bytes: usize,
) -> Option<()> {
    #[cfg(test)]
    LOCALIZED_INDEX_BUILD_VISITS.set(LOCALIZED_INDEX_BUILD_VISITS.get().saturating_add(1));
    if let Some(kind) = classify_position_block(node, schema) {
        let block = position_map.block(*next_block_index)?;
        let is_void = kind == PositionBlockKind::Void;
        let expected_doc_end = if is_void {
            doc_offset
        } else {
            doc_offset.checked_add(node.content()?.size())?
        };
        if block.is_void_block != is_void
            || block.node_path.len() != path.len()
            || block.doc_start != doc_offset
            || block.doc_end != expected_doc_end
        {
            return None;
        }
        let block_index = *next_block_index;
        *next_block_index = next_block_index.checked_add(1)?;
        if !is_void {
            collect_localized_text_leaves_streamed(
                position_map,
                node,
                block.doc_start,
                block.scalar_start.checked_add(block.scalar_prefix_len)?,
                block_index,
                leaves,
                rendered_cursor,
                retained_bytes,
                cache_budget,
                path_capacity_bytes,
            )?;
        }
        return Some(());
    }
    let Some(content) = node.content() else {
        return Some(());
    };
    let mut child_doc_offset = doc_offset;
    for (child_index, child) in content.iter().enumerate() {
        #[cfg(test)]
        LOCALIZED_INDEX_PATH_HOPS.set(LOCALIZED_INDEX_PATH_HOPS.get().saturating_add(1));
        path.push(u32::try_from(child_index).ok()?);
        collect_localized_index_streamed(
            child,
            path,
            position_map,
            schema,
            if child.is_element() {
                child_doc_offset.checked_add(1)?
            } else {
                child_doc_offset
            },
            next_block_index,
            leaves,
            rendered_cursor,
            retained_bytes,
            cache_budget,
            path_capacity_bytes,
        )?;
        path.pop()?;
        child_doc_offset = child_doc_offset.checked_add(child.node_size())?;
    }
    Some(())
}

pub(super) struct RenderedCursor<'a> {
    pub(super) characters: std::str::Chars<'a>,
    pub(super) scalar: u32,
    pub(super) utf16: u32,
}

impl<'a> RenderedCursor<'a> {
    pub(super) fn new(rendered: &'a str) -> Self {
        Self {
            characters: rendered.chars(),
            scalar: 0,
            utf16: 0,
        }
    }

    pub(super) fn advance_to(&mut self, target: u32) -> Option<()> {
        while self.scalar < target {
            let character = self.characters.next()?;
            self.scalar = self.scalar.checked_add(1)?;
            self.utf16 = self
                .utf16
                .checked_add(u32::try_from(character.len_utf16()).ok()?)?;
        }
        (self.scalar == target).then_some(())
    }

    pub(super) fn match_text(&mut self, text: &str) -> Option<(u32, u32)> {
        let start = self.utf16;
        for expected in text.chars() {
            if self.characters.next()? != expected {
                return None;
            }
            self.scalar = self.scalar.checked_add(1)?;
            self.utf16 = self
                .utf16
                .checked_add(u32::try_from(expected.len_utf16()).ok()?)?;
        }
        Some((start, self.utf16))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn collect_localized_text_leaves_streamed(
    position_map: &PositionMap,
    node: &crate::model::Node,
    content_start: u32,
    scalar_content_start: u32,
    block_index: usize,
    leaves: &mut Vec<LocalizedTextLeafCertificate>,
    rendered_cursor: &mut RenderedCursor<'_>,
    retained_bytes: &mut usize,
    cache_budget: usize,
    path_capacity_bytes: usize,
) -> Option<()> {
    let content = node.content()?;
    let mut child_start = content_start;
    let mut child_scalar_start = scalar_content_start;
    for (child_index, child) in content.iter().enumerate() {
        #[cfg(test)]
        {
            LOCALIZED_INDEX_BUILD_VISITS.set(LOCALIZED_INDEX_BUILD_VISITS.get().saturating_add(1));
            LOCALIZED_INDEX_PATH_HOPS.set(LOCALIZED_INDEX_PATH_HOPS.get().saturating_add(1));
        }
        if let Some(text) = child.text_str() {
            let doc_end = child_start.checked_add(child.node_size())?;
            let scalar_end = child_scalar_start.checked_add(child.node_size())?;
            rendered_cursor.advance_to(child_scalar_start)?;
            let (utf16_start, utf16_end) = rendered_cursor.match_text(text)?;
            let next_len = leaves.len().checked_add(1)?;
            let logical_leaf_bytes =
                next_len.checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
            if path_capacity_bytes.checked_add(logical_leaf_bytes)? > cache_budget {
                return None;
            }
            if leaves.len() == leaves.capacity() {
                #[cfg(test)]
                if forced_localized_index_allocation_stage(
                    LocalizedIndexAllocationStage::LeafGrowth,
                ) {
                    return None;
                }
                let old_capacity_bytes = leaves
                    .capacity()
                    .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
                let maximum_capacity = cache_budget
                    .checked_sub(path_capacity_bytes)?
                    .checked_sub(old_capacity_bytes)?
                    .checked_div(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
                let doubled_capacity = leaves.capacity().checked_mul(2).unwrap_or(maximum_capacity);
                let target_capacity = doubled_capacity.max(next_len).min(maximum_capacity);
                let additional = target_capacity.checked_sub(leaves.capacity())?;
                if additional == 0 {
                    return None;
                }
                leaves.try_reserve_exact(additional).ok()?;
                let new_capacity_bytes = leaves
                    .capacity()
                    .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
                if path_capacity_bytes
                    .checked_add(old_capacity_bytes)?
                    .checked_add(new_capacity_bytes)?
                    > cache_budget
                {
                    return None;
                }
            }
            *retained_bytes = leaves
                .capacity()
                .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
            if path_capacity_bytes.checked_add(*retained_bytes)? > cache_budget {
                return None;
            }
            leaves.push(LocalizedTextLeafCertificate {
                block_index,
                child_ordinal: u32::try_from(child_index).ok()?,
                doc_start: child_start,
                doc_end,
                scalar_start: child_scalar_start,
                scalar_end,
                utf16_start,
                utf16_end,
                text_sha256: sha2::Sha256::digest(text.as_bytes()).into(),
                text_scalars: child.node_size(),
                text_utf16: utf16_end.checked_sub(utf16_start)?,
                text_utf8_bytes: text.len(),
                marks_sha256: canonical_marks_sha256(child.marks())?,
            });
            child_scalar_start = scalar_end;
        } else if child.is_void() {
            child_scalar_start =
                child_scalar_start.checked_add(position_map.inline_void_scalar_len(child)?)?;
        }
        child_start = child_start.checked_add(child.node_size())?;
    }
    Some(())
}

pub(super) struct Sha256Writer(sha2::Sha256);

impl std::io::Write for Sha256Writer {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn canonical_marks_sha256(marks: &[Mark]) -> Option<[u8; 32]> {
    let mut writer = Sha256Writer(sha2::Sha256::new());
    std::io::Write::write_all(&mut writer, &u64::try_from(marks.len()).ok()?.to_le_bytes()).ok()?;
    for mark in marks {
        std::io::Write::write_all(&mut writer, b"{\"type\":").ok()?;
        serde_json::to_writer(&mut writer, mark.mark_type()).ok()?;
        std::io::Write::write_all(&mut writer, b",\"attrs\":{").ok()?;
        for (index, (key, value)) in mark.attrs().iter().enumerate() {
            if index != 0 {
                std::io::Write::write_all(&mut writer, b",").ok()?;
            }
            serde_json::to_writer(&mut writer, key).ok()?;
            std::io::Write::write_all(&mut writer, b":").ok()?;
            std::io::Write::write_all(
                &mut writer,
                &crate::boundary::serialize_json_value_stack_safe(value, 0),
            )
            .ok()?;
        }
        std::io::Write::write_all(&mut writer, b"}}").ok()?;
    }
    Some(writer.0.finalize().into())
}

pub(super) fn node_path_sha256(path: &[u32]) -> [u8; 32] {
    let mut digest = sha2::Sha256::new();
    digest.update(u64::try_from(path.len()).unwrap_or(u64::MAX).to_le_bytes());
    for index in path {
        digest.update(index.to_le_bytes());
    }
    digest.finalize().into()
}
