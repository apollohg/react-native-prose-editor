use std::collections::BTreeSet;
use std::sync::Arc;

use crate::boundary::ResourceLimits;
use crate::model::Document;
use crate::model::Node;
use crate::render::empty_text_block_placeholder_string;
use crate::render::inline_atom_label;
use crate::render::inline_atom_mention_theme;
use crate::render::opaque_node_is_inline;
use crate::render::task_list_marker_metadata;
use crate::render::ListContext;
use crate::render::RenderElement;
use crate::render::RenderMark;
use crate::schema::{schema_fingerprint, NodeRole, Schema};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalizedRenderFailureStage {
    Allocation,
    Resource,
    Position,
    Invariant,
}

#[cfg(test)]
std::thread_local! {
    static CACHED_BUILD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CACHED_TRANSITION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CACHED_RERENDERED_BLOCK_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static CACHED_FULL_TRANSITION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LEGACY_SAFE_PATCH_FULL_RENDER_PASS_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static SLOW_INVARIANT_CHECK_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_RENDER_TRANSITION_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_RENDER_TRANSITION_SUCCESSES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_RENDER_TRANSITION_FALLBACKS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FORCED_CACHED_RENDER_ERROR: std::cell::Cell<Option<CachedRenderError>> = const {
        std::cell::Cell::new(None)
    };
    static FORCED_LOCALIZED_RENDER_FAILURE_STAGE: std::cell::Cell<
        Option<LocalizedRenderFailureStage>
    > = const { std::cell::Cell::new(None) };
    static LOCALIZED_RENDER_ALLOCATION_CHECKPOINTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_RENDER_RESOURCE_CHECKPOINTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_RENDER_POSITION_CHECKPOINTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_RENDER_INVARIANT_CHECKPOINTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_cached_render_counts_for_test() {
    CACHED_BUILD_COUNT.set(0);
    CACHED_TRANSITION_COUNT.set(0);
    CACHED_RERENDERED_BLOCK_COUNT.set(0);
    CACHED_FULL_TRANSITION_COUNT.set(0);
    LEGACY_SAFE_PATCH_FULL_RENDER_PASS_COUNT.set(0);
}

#[cfg(test)]
/// Returns `(builds, transitions, rerendered_blocks, full_transitions,
/// legacy_safe_patch_full_render_passes)`.
pub(crate) fn take_cached_render_counts_for_test() -> (usize, usize, usize, usize, usize) {
    (
        CACHED_BUILD_COUNT.replace(0),
        CACHED_TRANSITION_COUNT.replace(0),
        CACHED_RERENDERED_BLOCK_COUNT.replace(0),
        CACHED_FULL_TRANSITION_COUNT.replace(0),
        LEGACY_SAFE_PATCH_FULL_RENDER_PASS_COUNT.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn set_cached_render_error_for_test(error: Option<CachedRenderError>) {
    FORCED_CACHED_RENDER_ERROR.set(error);
}

#[cfg(test)]
pub(crate) fn set_localized_render_failure_stage_for_test(
    stage: Option<LocalizedRenderFailureStage>,
) {
    FORCED_LOCALIZED_RENDER_FAILURE_STAGE.set(stage);
}

#[cfg(test)]
pub(crate) fn reset_localized_render_failure_checkpoint_counts_for_test() {
    LOCALIZED_RENDER_ALLOCATION_CHECKPOINTS.set(0);
    LOCALIZED_RENDER_RESOURCE_CHECKPOINTS.set(0);
    LOCALIZED_RENDER_POSITION_CHECKPOINTS.set(0);
    LOCALIZED_RENDER_INVARIANT_CHECKPOINTS.set(0);
}

#[cfg(test)]
pub(crate) fn take_localized_render_failure_checkpoint_counts_for_test(
) -> (usize, usize, usize, usize) {
    (
        LOCALIZED_RENDER_ALLOCATION_CHECKPOINTS.replace(0),
        LOCALIZED_RENDER_RESOURCE_CHECKPOINTS.replace(0),
        LOCALIZED_RENDER_POSITION_CHECKPOINTS.replace(0),
        LOCALIZED_RENDER_INVARIANT_CHECKPOINTS.replace(0),
    )
}

#[cfg(test)]
fn check_forced_localized_render_failure(
    stage: LocalizedRenderFailureStage,
) -> Result<(), CachedRenderError> {
    match stage {
        LocalizedRenderFailureStage::Allocation => LOCALIZED_RENDER_ALLOCATION_CHECKPOINTS.set(
            LOCALIZED_RENDER_ALLOCATION_CHECKPOINTS
                .get()
                .saturating_add(1),
        ),
        LocalizedRenderFailureStage::Resource => LOCALIZED_RENDER_RESOURCE_CHECKPOINTS.set(
            LOCALIZED_RENDER_RESOURCE_CHECKPOINTS
                .get()
                .saturating_add(1),
        ),
        LocalizedRenderFailureStage::Position => LOCALIZED_RENDER_POSITION_CHECKPOINTS.set(
            LOCALIZED_RENDER_POSITION_CHECKPOINTS
                .get()
                .saturating_add(1),
        ),
        LocalizedRenderFailureStage::Invariant => LOCALIZED_RENDER_INVARIANT_CHECKPOINTS.set(
            LOCALIZED_RENDER_INVARIANT_CHECKPOINTS
                .get()
                .saturating_add(1),
        ),
    }
    if FORCED_LOCALIZED_RENDER_FAILURE_STAGE.get() != Some(stage) {
        return Ok(());
    }
    FORCED_LOCALIZED_RENDER_FAILURE_STAGE.set(None);
    let error = match stage {
        LocalizedRenderFailureStage::Allocation => CachedRenderError::AllocationFailed,
        LocalizedRenderFailureStage::Resource => CachedRenderError::ResourceLimitExceeded,
        LocalizedRenderFailureStage::Position => CachedRenderError::PositionOverflow,
        LocalizedRenderFailureStage::Invariant => CachedRenderError::CacheInvariantViolation,
    };
    Err(error)
}

#[cfg(not(test))]
#[inline]
fn check_forced_localized_render_allocation_failure() -> Result<(), CachedRenderError> {
    Ok(())
}

#[cfg(not(test))]
#[inline]
fn check_forced_localized_render_resource_failure() -> Result<(), CachedRenderError> {
    Ok(())
}

#[cfg(not(test))]
#[inline]
fn check_forced_localized_render_position_failure() -> Result<(), CachedRenderError> {
    Ok(())
}

#[cfg(not(test))]
#[inline]
fn check_forced_localized_render_invariant_failure() -> Result<(), CachedRenderError> {
    Ok(())
}

#[cfg(test)]
fn check_forced_localized_render_allocation_failure() -> Result<(), CachedRenderError> {
    check_forced_localized_render_failure(LocalizedRenderFailureStage::Allocation)
}

#[cfg(test)]
fn check_forced_localized_render_resource_failure() -> Result<(), CachedRenderError> {
    check_forced_localized_render_failure(LocalizedRenderFailureStage::Resource)
}

#[cfg(test)]
fn check_forced_localized_render_position_failure() -> Result<(), CachedRenderError> {
    check_forced_localized_render_failure(LocalizedRenderFailureStage::Position)
}

#[cfg(test)]
fn check_forced_localized_render_invariant_failure() -> Result<(), CachedRenderError> {
    check_forced_localized_render_failure(LocalizedRenderFailureStage::Invariant)
}

#[cfg(test)]
fn reset_slow_invariant_checks_for_test() {
    SLOW_INVARIANT_CHECK_COUNT.set(0);
}

#[cfg(test)]
fn take_slow_invariant_checks_for_test() -> usize {
    SLOW_INVARIANT_CHECK_COUNT.replace(0)
}

#[cfg(test)]
pub(crate) fn reset_localized_render_transition_counts_for_test() {
    LOCALIZED_RENDER_TRANSITION_ATTEMPTS.set(0);
    LOCALIZED_RENDER_TRANSITION_SUCCESSES.set(0);
    LOCALIZED_RENDER_TRANSITION_FALLBACKS.set(0);
}

#[cfg(test)]
pub(crate) fn take_localized_render_transition_counts_for_test() -> (usize, usize, usize) {
    (
        LOCALIZED_RENDER_TRANSITION_ATTEMPTS.replace(0),
        LOCALIZED_RENDER_TRANSITION_SUCCESSES.replace(0),
        LOCALIZED_RENDER_TRANSITION_FALLBACKS.replace(0),
    )
}

#[inline]
pub(crate) fn record_localized_render_transition_attempt() {
    #[cfg(test)]
    LOCALIZED_RENDER_TRANSITION_ATTEMPTS
        .set(LOCALIZED_RENDER_TRANSITION_ATTEMPTS.get().saturating_add(1));
}

#[inline]
pub(crate) fn record_localized_render_transition_success() {
    #[cfg(test)]
    LOCALIZED_RENDER_TRANSITION_SUCCESSES.set(
        LOCALIZED_RENDER_TRANSITION_SUCCESSES
            .get()
            .saturating_add(1),
    );
}

#[inline]
pub(crate) fn record_localized_render_transition_fallback() {
    #[cfg(test)]
    LOCALIZED_RENDER_TRANSITION_FALLBACKS.set(
        LOCALIZED_RENDER_TRANSITION_FALLBACKS
            .get()
            .saturating_add(1),
    );
}

#[inline]
fn check_forced_cached_render_error() -> Result<(), CachedRenderError> {
    #[cfg(test)]
    if let Some(error) = FORCED_CACHED_RENDER_ERROR.get() {
        return Err(error);
    }
    Ok(())
}

#[inline]
fn record_cached_build() {
    #[cfg(test)]
    CACHED_BUILD_COUNT.set(CACHED_BUILD_COUNT.get().saturating_add(1));
}

#[inline]
fn record_cached_transition() {
    #[cfg(test)]
    CACHED_TRANSITION_COUNT.set(CACHED_TRANSITION_COUNT.get().saturating_add(1));
}

#[inline]
fn record_cached_rerendered_blocks(count: usize) {
    #[cfg(test)]
    CACHED_RERENDERED_BLOCK_COUNT.set(CACHED_RERENDERED_BLOCK_COUNT.get().saturating_add(count));
    #[cfg(not(test))]
    let _ = count;
}

#[inline]
fn record_cached_full_transition() {
    #[cfg(test)]
    CACHED_FULL_TRANSITION_COUNT.set(CACHED_FULL_TRANSITION_COUNT.get().saturating_add(1));
}

#[inline]
// Not reachable from production call paths after the legacy runtime removal;
// exercised by crate tests.
#[allow(dead_code)]
fn record_legacy_safe_patch_full_render_pass() {
    #[cfg(test)]
    LEGACY_SAFE_PATCH_FULL_RENDER_PASS_COUNT.set(
        LEGACY_SAFE_PATCH_FULL_RENDER_PASS_COUNT
            .get()
            .saturating_add(1),
    );
}

fn render_marks(node: &crate::model::Node) -> Vec<RenderMark> {
    node.marks()
        .iter()
        .map(|mark| RenderMark {
            mark_type: mark.mark_type().to_string(),
            attrs: mark.attrs().clone(),
        })
        .collect()
}

/// Result of an incremental re-render: a block index and its regenerated elements.
pub type BlockPatch = (usize, Vec<RenderElement>);

#[derive(Debug, Clone, PartialEq)]
pub struct RenderBlocksPatch {
    pub start_index: usize,
    pub delete_count: usize,
    pub blocks: Vec<Vec<RenderElement>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum CachedRenderError {
    ResourceLimitExceeded,
    AllocationFailed,
    PositionOverflow,
    CacheInvariantViolation,
}

#[derive(Debug, Clone)]
struct CachedRenderBlock {
    node: Arc<Node>,
    start_pos: u32,
    node_size: u32,
    elements: Arc<Vec<RenderElement>>,
    position_element_indices: Arc<Vec<usize>>,
}

impl Drop for CachedRenderBlock {
    fn drop(&mut self) {
        if let Some(elements) = Arc::get_mut(&mut self.elements) {
            for element in elements {
                element.drain_json_payloads();
            }
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct CachedRenderBlocks {
    blocks: Vec<CachedRenderBlock>,
    document_root_seal: Node,
    schema_fingerprint: Arc<str>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) enum CachedRenderTransitionUpdate {
    None,
    Patch(RenderBlocksPatch),
    Full(Vec<Vec<RenderElement>>),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct CachedRenderTransition {
    pub(crate) cache: CachedRenderBlocks,
    pub(crate) update: CachedRenderTransitionUpdate,
    pub(crate) rerendered_new_blocks: usize,
}

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
                    node_type, label, attrs, ..
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

/// Re-generate render elements for only the affected top-level blocks.
///
/// `affected_indices` are 0-based indices into the document root's children
/// (i.e. the top-level block nodes). Only those blocks' RenderElement
/// subsequences are regenerated.
///
/// Returns a vec of `(block_index, elements)` pairs, sorted by block index.
pub(crate) fn try_incremental(
    doc: &Document,
    schema: &Schema,
    affected_indices: &[usize],
) -> Result<Vec<BlockPatch>, CachedRenderError> {
    let affected: BTreeSet<usize> = affected_indices.iter().copied().collect();
    let root = doc.root();
    let mut results = Vec::new();

    // Walk top-level children to compute positions, but only generate elements
    // for affected blocks.
    let mut pos: u32 = 0;
    for i in 0..root.child_count() {
        let child = root.child(i).expect("child index in bounds");

        if affected.contains(&i) {
            let mut elements = Vec::new();
            let mut block_pos = pos;
            generate_block(child, schema, &mut elements, &mut block_pos, 0, None, i)?;
            results.push((i, elements));
        }

        // Advance position past this child regardless
        pos = pos
            .checked_add(child.node_size())
            .ok_or(CachedRenderError::PositionOverflow)?;
    }

    Ok(results)
}

// Retained for direct render parity tests; production v2 rendering uses the
// fallible entry point above so render-preparation errors stay structured.
#[cfg_attr(not(test), allow(dead_code))]
pub fn incremental(doc: &Document, schema: &Schema, affected_indices: &[usize]) -> Vec<BlockPatch> {
    try_incremental(doc, schema, affected_indices)
        .expect("incremental render requires a document with admitted arithmetic")
}

pub(crate) fn try_render_blocks(
    doc: &Document,
    schema: &Schema,
) -> Result<Vec<Vec<RenderElement>>, CachedRenderError> {
    let root = doc.root();
    if root.child_count() == 0 {
        return Ok(Vec::new());
    }
    let indices = (0..root.child_count()).collect::<Vec<_>>();
    try_incremental(doc, schema, &indices)
        .map(|patches| patches.into_iter().map(|(_, elements)| elements).collect())
}

pub fn render_blocks(doc: &Document, schema: &Schema) -> Vec<Vec<RenderElement>> {
    try_render_blocks(doc, schema)
        .expect("render blocks requires a document with admitted arithmetic")
}

pub fn flatten_render_blocks(blocks: &[Vec<RenderElement>]) -> Vec<RenderElement> {
    let mut elements = Vec::new();
    for block in blocks {
        elements.extend(block.iter().cloned());
    }
    elements
}

// Not reachable from production call paths after the legacy runtime removal;
// exercised by crate tests.
#[allow(dead_code)]
pub fn contiguous_render_blocks_patch(
    old_doc: &Document,
    new_doc: &Document,
    schema: &Schema,
) -> Option<RenderBlocksPatch> {
    let old_blocks = render_blocks(old_doc, schema);
    let new_blocks = render_blocks(new_doc, schema);

    let mut prefix = 0usize;
    while prefix < old_blocks.len()
        && prefix < new_blocks.len()
        && old_blocks[prefix] == new_blocks[prefix]
    {
        prefix += 1;
    }

    if prefix == old_blocks.len() && prefix == new_blocks.len() {
        return None;
    }

    let mut old_suffix = old_blocks.len();
    let mut new_suffix = new_blocks.len();
    while old_suffix > prefix
        && new_suffix > prefix
        && old_blocks[old_suffix - 1] == new_blocks[new_suffix - 1]
    {
        old_suffix -= 1;
        new_suffix -= 1;
    }

    let start_index = if prefix > 0 { prefix - 1 } else { 0 };
    let old_end = if old_suffix < old_blocks.len() {
        old_suffix + 1
    } else {
        old_suffix
    };
    let new_end = if new_suffix < new_blocks.len() {
        new_suffix + 1
    } else {
        new_suffix
    };

    Some(RenderBlocksPatch {
        start_index,
        delete_count: old_end.saturating_sub(start_index),
        blocks: new_blocks[start_index..new_end].to_vec(),
    })
}

/// Derive a contiguous render patch and prove it reconstructs the complete
/// new render. Compiler hints may widen the exact rendered diff, never narrow
/// it. `Err(full)` is the safe fallback whenever the proof cannot be made.
// Not reachable from production call paths after the Task 16C legacy runtime
// removal; exercised by crate tests.
#[allow(dead_code)]
pub fn safe_contiguous_render_blocks_patch(
    old_doc: &Document,
    new_doc: &Document,
    schema: &Schema,
    affected_indices: &[usize],
) -> Result<Option<RenderBlocksPatch>, Vec<Vec<RenderElement>>> {
    if old_doc == new_doc {
        return Ok(None);
    }

    record_legacy_safe_patch_full_render_pass();
    record_legacy_safe_patch_full_render_pass();
    let old_blocks = render_blocks(old_doc, schema);
    let new_blocks = render_blocks(new_doc, schema);
    let widest_len = old_blocks.len().max(new_blocks.len());
    if affected_indices.iter().any(|index| *index >= widest_len) {
        return Err(new_blocks);
    }

    let mut prefix = 0usize;
    while prefix < old_blocks.len()
        && prefix < new_blocks.len()
        && old_blocks[prefix] == new_blocks[prefix]
    {
        prefix += 1;
    }
    let mut old_end = old_blocks.len();
    let mut new_end = new_blocks.len();
    while old_end > prefix && new_end > prefix && old_blocks[old_end - 1] == new_blocks[new_end - 1]
    {
        old_end -= 1;
        new_end -= 1;
    }
    if prefix == old_blocks.len() && prefix == new_blocks.len() {
        return Err(new_blocks);
    }

    let mut start = prefix;
    for index in affected_indices {
        start = start.min(*index);
        if *index < old_blocks.len() {
            old_end = old_end.max(index.saturating_add(1));
        }
        if *index < new_blocks.len() {
            new_end = new_end.max(index.saturating_add(1));
        }
    }
    let patch = RenderBlocksPatch {
        start_index: start,
        delete_count: old_end.saturating_sub(start),
        blocks: new_blocks[start..new_end].to_vec(),
    };
    let mut reconstructed = old_blocks;
    let Some(end) = patch.start_index.checked_add(patch.delete_count) else {
        return Err(new_blocks);
    };
    if end > reconstructed.len() {
        return Err(new_blocks);
    }
    reconstructed.splice(patch.start_index..end, patch.blocks.clone());
    if reconstructed == new_blocks {
        Ok(Some(patch))
    } else {
        Err(new_blocks)
    }
}

/// Generate render elements for a single top-level block and its descendants.
/// This mirrors the logic in `generate::walk_children` but for a single node.
fn generate_block(
    node: &crate::model::Node,
    schema: &Schema,
    elements: &mut Vec<RenderElement>,
    pos: &mut u32,
    depth: u16,
    list_info: Option<(String, bool, u32, u32)>,
    child_index: usize,
) -> Result<(), CachedRenderError> {
    stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
        generate_block_inner(node, schema, elements, pos, depth, list_info, child_index)
    })
}

fn generate_block_inner(
    node: &crate::model::Node,
    schema: &Schema,
    elements: &mut Vec<RenderElement>,
    pos: &mut u32,
    depth: u16,
    list_info: Option<(String, bool, u32, u32)>,
    child_index: usize,
) -> Result<(), CachedRenderError> {
    let spec = schema.node(node.node_type());
    let role = spec.map(|s| &s.role);

    match role {
        Some(NodeRole::Text) => {
            let text = node.text_str().unwrap_or("").to_string();
            let marks = render_marks(node);
            elements.push(RenderElement::TextRun { text, marks });
            *pos += node.node_size();
        }
        Some(NodeRole::HardBreak) => {
            elements.push(RenderElement::VoidInline {
                node_type: node.node_type().to_string(),
                doc_pos: *pos,
                attrs: node.attrs().clone(),
            });
            *pos += node.node_size();
        }
        Some(NodeRole::List { ordered }) => {
            let ordered = *ordered;
            let start_attr = ordered_list_start(node)?;
            let total = u32::try_from(node.child_count())
                .map_err(|_| CachedRenderError::PositionOverflow)?;

            *pos += 1; // list open tag
            for j in 0..node.child_count() {
                let item = node.child(j).expect("child index in bounds");
                generate_block(
                    item,
                    schema,
                    elements,
                    pos,
                    depth,
                    Some((node.node_type().to_string(), ordered, start_attr, total)),
                    j,
                )?;
            }
            *pos += 1; // list close tag
        }
        Some(NodeRole::ListItem) => {
            let list_context = if let Some((list_node_type, ordered, start, total)) = list_info {
                let item_offset =
                    u32::try_from(child_index).map_err(|_| CachedRenderError::PositionOverflow)?;
                let index = if ordered {
                    start
                        .checked_add(item_offset)
                        .ok_or(CachedRenderError::PositionOverflow)?
                } else {
                    item_offset
                        .checked_add(1)
                        .ok_or(CachedRenderError::PositionOverflow)?
                };
                let (kind, checked) = task_list_marker_metadata(&list_node_type, node);
                Some(ListContext {
                    ordered,
                    index,
                    total,
                    start,
                    is_first: child_index == 0,
                    is_last: item_offset
                        == total
                            .checked_sub(1)
                            .ok_or(CachedRenderError::PositionOverflow)?,
                    kind,
                    checked,
                })
            } else {
                None
            };
            elements.push(RenderElement::BlockStart {
                node_type: node.node_type().to_string(),
                depth,
                list_context,
            });
            *pos += 1;
            for j in 0..node.child_count() {
                let child = node.child(j).expect("child index in bounds");
                generate_block(child, schema, elements, pos, depth + 1, None, j)?;
            }
            *pos += 1;
            elements.push(RenderElement::BlockEnd);
        }
        Some(NodeRole::TextBlock) => {
            elements.push(RenderElement::BlockStart {
                node_type: node.node_type().to_string(),
                depth,
                list_context: None,
            });
            *pos += 1;
            if node.child_count() == 0 {
                elements.push(RenderElement::TextRun {
                    text: empty_text_block_placeholder_string(),
                    marks: vec![],
                });
            } else {
                for j in 0..node.child_count() {
                    let child = node.child(j).expect("child index in bounds");
                    generate_block(child, schema, elements, pos, depth + 1, None, j)?;
                }
            }
            *pos += 1;
            elements.push(RenderElement::BlockEnd);
        }
        Some(NodeRole::Block) if node.is_void() => {
            elements.push(RenderElement::VoidBlock {
                node_type: node.node_type().to_string(),
                doc_pos: *pos,
                attrs: node.attrs().clone(),
            });
            *pos += node.node_size();
        }
        Some(NodeRole::Block) => {
            elements.push(RenderElement::BlockStart {
                node_type: node.node_type().to_string(),
                depth,
                list_context: None,
            });
            *pos += 1;
            for j in 0..node.child_count() {
                let child = node.child(j).expect("child index in bounds");
                generate_block(child, schema, elements, pos, depth + 1, None, j)?;
            }
            *pos += 1;
            elements.push(RenderElement::BlockEnd);
        }
        Some(NodeRole::Inline) if node.is_void() => {
            elements.push(RenderElement::OpaqueInlineAtom {
                node_type: node.node_type().to_string(),
                label: inline_atom_label(node.node_type(), node.attrs()),
                doc_pos: *pos,
                attrs: node.attrs().clone(),
                mention_theme: inline_atom_mention_theme(node.node_type(), node.attrs()),
            });
            *pos += node.node_size();
        }
        Some(NodeRole::Inline) => {
            *pos += node.node_size();
        }
        Some(NodeRole::Doc) => {
            *pos += 1;
            for j in 0..node.child_count() {
                let child = node.child(j).expect("child index in bounds");
                generate_block(child, schema, elements, pos, depth, None, j)?;
            }
            *pos += 1;
        }
        None => {
            if node.is_void() {
                let is_inline = opaque_node_is_inline(node, schema);
                if is_inline {
                    elements.push(RenderElement::OpaqueInlineAtom {
                        node_type: node.node_type().to_string(),
                        label: inline_atom_label(node.node_type(), node.attrs()),
                        doc_pos: *pos,
                        attrs: node.attrs().clone(),
                        mention_theme: inline_atom_mention_theme(node.node_type(), node.attrs()),
                    });
                } else {
                    elements.push(RenderElement::OpaqueBlockAtom {
                        node_type: node.node_type().to_string(),
                        label: inline_atom_label(node.node_type(), node.attrs()),
                        doc_pos: *pos,
                        attrs: node.attrs().clone(),
                    });
                }
                *pos += node.node_size();
            } else if node.is_text() {
                let text = node.text_str().unwrap_or("").to_string();
                let marks = render_marks(node);
                elements.push(RenderElement::TextRun { text, marks });
                *pos += node.node_size();
            } else {
                *pos += node.node_size();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use proptest::prelude::*;

    use crate::boundary::ResourceLimits;
    use crate::model::{Document, Fragment, Mark, Node};
    use crate::render::incremental::{
        render_blocks, try_render_blocks, CachedRenderBlocks, CachedRenderTransitionUpdate,
    };
    use crate::render::RenderElement;
    use crate::{prosemirror_schema, tiptap_schema};

    fn text(value: &str) -> Node {
        Node::text(value.to_string(), vec![])
    }

    fn paragraph(children: Vec<Node>) -> Node {
        Node::element(
            "paragraph".to_string(),
            HashMap::new(),
            Fragment::from(children),
        )
    }

    fn doc(children: Vec<Node>) -> Document {
        Document::new(Node::element(
            "doc".to_string(),
            HashMap::new(),
            Fragment::from(children),
        ))
    }

    fn replace_top_level(document: &Document, index: usize, replacement: Node) -> Document {
        let mut children = (0..document.root().child_count())
            .map(|child_index| document.root().child(child_index).unwrap().clone())
            .collect::<Vec<_>>();
        children[index] = replacement;
        doc(children)
    }

    fn inline_atom(label: &str) -> Node {
        Node::void(
            "__opaque_json".to_string(),
            HashMap::from([
                (
                    "opaque_placement".to_string(),
                    serde_json::Value::String("inline".to_string()),
                ),
                (
                    "label".to_string(),
                    serde_json::Value::String(label.to_string()),
                ),
            ]),
        )
    }

    fn opaque_block_atom(label: &str) -> Node {
        Node::void(
            "__opaque_json".to_string(),
            HashMap::from([
                (
                    "opaque_placement".to_string(),
                    serde_json::Value::String("block".to_string()),
                ),
                (
                    "label".to_string(),
                    serde_json::Value::String(label.to_string()),
                ),
            ]),
        )
    }

    fn hard_break() -> Node {
        Node::void("hardBreak".to_string(), HashMap::new())
    }

    fn horizontal_rule() -> Node {
        Node::void("horizontalRule".to_string(), HashMap::new())
    }

    fn bullet_list(children: Vec<Node>) -> Node {
        Node::element(
            "bulletList".to_string(),
            HashMap::new(),
            Fragment::from(children),
        )
    }

    fn ordered_list(start: u32, children: Vec<Node>) -> Node {
        ordered_list_with_start(Some(serde_json::Value::Number(start.into())), children)
    }

    fn ordered_list_with_start(start: Option<serde_json::Value>, children: Vec<Node>) -> Node {
        let mut attrs = HashMap::new();
        if let Some(start) = start {
            attrs.insert("start".to_string(), start);
        }
        Node::element("orderedList".to_string(), attrs, Fragment::from(children))
    }

    fn list_item(children: Vec<Node>) -> Node {
        Node::element(
            "listItem".to_string(),
            HashMap::new(),
            Fragment::from(children),
        )
    }

    fn assert_update_reconstructs(
        old_render: Vec<Vec<RenderElement>>,
        transition: &super::CachedRenderTransition,
        expected: &[Vec<RenderElement>],
    ) {
        let reconstructed = match &transition.update {
            CachedRenderTransitionUpdate::None => old_render,
            CachedRenderTransitionUpdate::Patch(patch) => {
                let mut blocks = old_render;
                let end = patch
                    .start_index
                    .checked_add(patch.delete_count)
                    .expect("test patch range should not overflow");
                blocks.splice(patch.start_index..end, patch.blocks.clone());
                blocks
            }
            CachedRenderTransitionUpdate::Full(blocks) => blocks.clone(),
        };
        assert_eq!(reconstructed, expected);
        assert_eq!(transition.cache.materialize(), expected);
    }

    #[test]
    fn legacy_safe_patch_counter_counts_only_its_old_and_new_full_render_passes() {
        let schema = tiptap_schema();
        let old_doc = doc(vec![paragraph(vec![text("old")])]);
        let new_doc = doc(vec![paragraph(vec![text("new")])]);
        super::reset_cached_render_counts_for_test();

        super::safe_contiguous_render_blocks_patch(&old_doc, &new_doc, &schema, &[0])
            .expect("valid hint should produce a safe patch");

        assert_eq!(super::take_cached_render_counts_for_test(), (0, 0, 0, 0, 2));
    }

    #[test]
    fn cached_render_slow_invariant_detects_private_block_tampering() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let document = doc(vec![
            paragraph(vec![text("first")]),
            paragraph(vec![text("second")]),
        ]);
        let mut cache = CachedRenderBlocks::build(&document, &schema, &limits).unwrap();

        assert!(cache.verify_slow_invariant(&document, &schema));
        let sealed_schema = Arc::clone(&cache.schema_fingerprint);
        cache.schema_fingerprint = Arc::<str>::from("tampered-schema");
        assert!(!cache.verify_slow_invariant(&document, &schema));
        cache.schema_fingerprint = sealed_schema;
        let sealed_root = cache.document_root_seal.clone();
        let foreign = doc(vec![
            paragraph(vec![text("first")]),
            paragraph(vec![text("second")]),
        ]);
        cache.document_root_seal = foreign.root().clone();
        assert!(!cache.verify_slow_invariant(&document, &schema));
        cache.document_root_seal = sealed_root;
        let removed_block = cache.blocks.pop().unwrap();
        assert!(!cache.verify_slow_invariant(&document, &schema));
        cache.blocks.push(removed_block);
        let sealed_node = Arc::clone(&cache.blocks[0].node);
        cache.blocks[0].node = Arc::new(paragraph(vec![text("tampered")]));
        assert!(!cache.verify_slow_invariant(&document, &schema));
        cache.blocks[0].node = sealed_node;
        let sealed_node_size = cache.blocks[0].node_size;
        cache.blocks[0].node_size = cache.blocks[0].node_size.saturating_add(1);
        assert!(!cache.verify_slow_invariant(&document, &schema));
        cache.blocks[0].node_size = sealed_node_size;
        cache.blocks[1].start_pos = cache.blocks[1].start_pos.saturating_add(1);
        assert!(!cache.verify_slow_invariant(&document, &schema));
    }

    #[test]
    fn cached_render_identity_accepts_only_the_sealed_root_and_schema() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let document = doc(vec![paragraph(vec![text("same")])]);
        let shared = document.clone();
        let foreign = doc(vec![paragraph(vec![text("same")])]);
        let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
        let cache = CachedRenderBlocks::build(&document, &schema, &limits).unwrap();

        assert_eq!(document, foreign);
        assert!(!document.root().shares_storage_with(foreign.root()));
        assert!(cache.matches_identity(&shared, &schema_fingerprint));
        assert!(!cache.matches_identity(&foreign, &schema_fingerprint));
        assert!(!cache.matches_identity(&shared, "foreign-schema"));
    }

    #[test]
    fn cached_render_build_transition_and_full_fallback_propagate_identity_seals() {
        let schema = tiptap_schema();
        let schema_fingerprint = crate::schema::schema_fingerprint(&schema);
        let limits = ResourceLimits::default();
        let old_document = doc(vec![paragraph(vec![text("old")])]);
        let new_document = doc(vec![paragraph(vec![text("new")])]);
        let foreign_new = doc(vec![paragraph(vec![text("new")])]);
        let cache = CachedRenderBlocks::build(&old_document, &schema, &limits).unwrap();

        assert!(cache.matches_identity(&old_document, &schema_fingerprint));
        let transition = cache
            .transition(&old_document, &new_document, &schema, &[0], &limits)
            .unwrap();
        assert!(transition
            .cache
            .matches_identity(&new_document, &schema_fingerprint));
        assert!(!transition
            .cache
            .matches_identity(&foreign_new, &schema_fingerprint));

        let fallback = cache
            .transition(&old_document, &new_document, &schema, &[1], &limits)
            .unwrap();
        assert!(matches!(
            fallback.update,
            CachedRenderTransitionUpdate::Full(_)
        ));
        assert!(fallback
            .cache
            .matches_identity(&new_document, &schema_fingerprint));
        assert!(!fallback
            .cache
            .matches_identity(&foreign_new, &schema_fingerprint));

        let shared_old = old_document.clone();
        let unchanged = cache
            .transition(&old_document, &shared_old, &schema, &[], &limits)
            .unwrap();
        assert!(unchanged
            .cache
            .matches_identity(&shared_old, &schema_fingerprint));

        let deep_equal_old = doc(vec![paragraph(vec![text("old")])]);
        assert_eq!(old_document, deep_equal_old);
        assert!(!old_document
            .root()
            .shares_storage_with(deep_equal_old.root()));
        let resealed_unchanged = cache
            .transition(&old_document, &deep_equal_old, &schema, &[], &limits)
            .unwrap();
        assert!(resealed_unchanged
            .cache
            .matches_identity(&deep_equal_old, &schema_fingerprint));
        assert!(!resealed_unchanged
            .cache
            .matches_identity(&old_document, &schema_fingerprint));

        let new_schema = prosemirror_schema();
        let new_schema_fingerprint = crate::schema::schema_fingerprint(&new_schema);
        let schema_fallback = cache
            .transition(&old_document, &old_document, &new_schema, &[], &limits)
            .unwrap();
        assert!(schema_fallback
            .cache
            .matches_identity(&old_document, &new_schema_fingerprint));
        assert!(!schema_fallback
            .cache
            .matches_identity(&old_document, &schema_fingerprint));
    }

    #[test]
    fn cached_render_build_and_every_transition_run_the_slow_debug_verifier() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let old_document = doc(vec![paragraph(vec![text("old")])]);
        let new_document = doc(vec![paragraph(vec![text("new")])]);

        super::reset_slow_invariant_checks_for_test();
        let cache = CachedRenderBlocks::build(&old_document, &schema, &limits).unwrap();
        assert_eq!(super::take_slow_invariant_checks_for_test(), 1);

        super::reset_slow_invariant_checks_for_test();
        cache
            .transition(&old_document, &new_document, &schema, &[0], &limits)
            .unwrap();
        assert_eq!(super::take_slow_invariant_checks_for_test(), 1);

        super::reset_slow_invariant_checks_for_test();
        let foreign_old = doc(vec![paragraph(vec![text("old")])]);
        cache
            .transition(&foreign_old, &new_document, &schema, &[0], &limits)
            .unwrap();
        assert_eq!(
            super::take_slow_invariant_checks_for_test(),
            2,
            "full fallback verifies both its rebuilt cache and transition result"
        );

        super::reset_slow_invariant_checks_for_test();
        cache
            .transition(&old_document, &old_document, &schema, &[], &limits)
            .unwrap();
        assert_eq!(super::take_slow_invariant_checks_for_test(), 1);
    }

    #[test]
    fn localized_render_transition_matches_generic_for_supported_insert_shapes() {
        fn assert_parity(old: Document, new: Document, target: usize, inserted_scalars: u32) {
            let schema = tiptap_schema();
            let limits = ResourceLimits::default();
            let cache = CachedRenderBlocks::build(&old, &schema, &limits).unwrap();
            let affected = target.saturating_sub(1)..old.root().child_count();
            let affected = affected.collect::<Vec<_>>();
            let specialized = cache
                .transition_localized_insert(&old, &new, &schema, target, inserted_scalars, &limits)
                .unwrap();
            let generic = cache.transition(&old, &new, &schema, &[], &limits).unwrap();
            assert_eq!(specialized.update, generic.update);
            assert_eq!(specialized.cache.materialize(), generic.cache.materialize());
            assert_eq!(specialized.rerendered_new_blocks, 1);
            if old.root().child_count() == 160 {
                let CachedRenderTransitionUpdate::Patch(patch) = &specialized.update else {
                    panic!("wide localized insert must retain the generic patch contract");
                };
                assert_eq!(patch.start_index, target);
                assert_eq!(patch.delete_count, 1);
                assert_eq!(patch.blocks.len(), 1);
                let conservative =
                    super::classify_cached_transition(&cache, &specialized.cache, &affected, true);
                assert_ne!(conservative, specialized.update);
                let CachedRenderTransitionUpdate::Patch(conservative) = conservative else {
                    panic!("conservative range should widen the patch for this fixture");
                };
                assert!(conservative.delete_count > patch.delete_count);
                assert!(conservative.blocks.len() > patch.blocks.len());
            }
        }

        let three = doc(vec![
            paragraph(vec![text("first")]),
            paragraph(vec![text("middle")]),
            paragraph(vec![text("last")]),
        ]);
        for (target, replacement) in [(0, "firstx"), (1, "middlex"), (2, "lastx")] {
            assert_parity(
                three.clone(),
                replace_top_level(&three, target, paragraph(vec![text(replacement)])),
                target,
                1,
            );
        }

        let bold = Mark::new("bold".to_string(), HashMap::new());
        let fragmented = doc(vec![paragraph(vec![
            Node::text("ab".to_string(), vec![bold.clone()]),
            Node::text("cd".to_string(), vec![]),
        ])]);
        assert_parity(
            fragmented.clone(),
            replace_top_level(
                &fragmented,
                0,
                paragraph(vec![
                    Node::text("ab".to_string(), vec![bold]),
                    Node::text("c🙂\\\"\n\u{1}d".to_string(), vec![]),
                ]),
            ),
            0,
            5,
        );

        let nested = doc(vec![bullet_list(vec![
            list_item(vec![paragraph(vec![text("one")])]),
            list_item(vec![paragraph(vec![text("two")])]),
        ])]);
        assert_parity(
            nested.clone(),
            replace_top_level(
                &nested,
                0,
                bullet_list(vec![
                    list_item(vec![paragraph(vec![text("one")])]),
                    list_item(vec![paragraph(vec![text("twox")])]),
                ]),
            ),
            0,
            1,
        );

        let positioned_suffix = doc(vec![
            paragraph(vec![text("edit")]),
            paragraph(vec![text("later"), hard_break(), inline_atom("mention")]),
            horizontal_rule(),
            opaque_block_atom("trailing"),
        ]);
        assert_parity(
            positioned_suffix.clone(),
            replace_top_level(
                &positioned_suffix,
                0,
                paragraph(vec![text("edit expanded")]),
            ),
            0,
            9,
        );

        let wide = doc((0..160)
            .map(|index| paragraph(vec![text(&format!("block {index}"))]))
            .collect());
        assert_parity(
            wide.clone(),
            replace_top_level(&wide, 80, paragraph(vec![text("block 80x")])),
            80,
            1,
        );
    }

    #[test]
    fn localized_render_transition_accepts_exact_element_capacity_and_rejects_one_under() {
        let schema = tiptap_schema();
        let default_limits = ResourceLimits::default();
        let old = doc(vec![paragraph(vec![
            text("a"),
            hard_break(),
            inline_atom("mention"),
            text("b"),
            hard_break(),
            inline_atom("emoji"),
            text("c"),
            hard_break(),
            inline_atom("mention"),
        ])]);
        let new = replace_top_level(
            &old,
            0,
            paragraph(vec![
                text("ax"),
                hard_break(),
                inline_atom("mention"),
                text("b"),
                hard_break(),
                inline_atom("emoji"),
                text("c"),
                hard_break(),
                inline_atom("mention"),
            ]),
        );
        let cache = CachedRenderBlocks::build(&old, &schema, &default_limits).unwrap();
        let new_cache = CachedRenderBlocks::build(&new, &schema, &default_limits).unwrap();
        let old_materialized = cache.materialize();
        let new_materialized = new_cache.materialize();
        let required_elements = old_materialized
            .iter()
            .chain(new_materialized.iter())
            .map(Vec::len)
            .max()
            .unwrap();
        let exact_nodes = required_elements.div_ceil(3);
        assert!(
            exact_nodes > 1,
            "fixture must make one-under resource-bound"
        );
        let exact = ResourceLimits {
            max_document_nodes: exact_nodes,
            ..default_limits.clone()
        };
        let one_under = ResourceLimits {
            max_document_nodes: exact_nodes - 1,
            ..default_limits
        };

        assert!(cache
            .transition_localized_insert(&old, &new, &schema, 0, 1, &exact)
            .is_ok());
        assert!(matches!(
            cache.transition_localized_insert(&old, &new, &schema, 0, 1, &one_under),
            Err(super::CachedRenderError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn localized_render_transition_rejects_unsealed_shape_and_delta_facts() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let old = doc(vec![
            paragraph(vec![text("first")]),
            paragraph(vec![text("middle")]),
            paragraph(vec![text("last")]),
        ]);
        let new = replace_top_level(&old, 1, paragraph(vec![text("middlex")]));
        let cache = CachedRenderBlocks::build(&old, &schema, &limits).unwrap();

        assert!(matches!(
            cache.transition_localized_insert(&old, &new, &schema, 1, 2, &limits),
            Err(super::CachedRenderError::CacheInvariantViolation)
        ));
        assert!(matches!(
            cache.transition_localized_insert(&old, &new, &schema, 3, 1, &limits),
            Err(super::CachedRenderError::CacheInvariantViolation)
        ));

        let changed_cardinality = doc(vec![
            paragraph(vec![text("first")]),
            paragraph(vec![text("middlex")]),
            paragraph(vec![text("last")]),
            paragraph(vec![text("extra")]),
        ]);
        assert!(matches!(
            cache.transition_localized_insert(&old, &changed_cardinality, &schema, 1, 1, &limits,),
            Err(super::CachedRenderError::CacheInvariantViolation)
        ));

        let foreign_unchanged_blocks = doc(vec![
            paragraph(vec![text("first")]),
            paragraph(vec![text("middlex")]),
            paragraph(vec![text("last")]),
        ]);
        assert!(matches!(
            cache.transition_localized_insert(
                &old,
                &foreign_unchanged_blocks,
                &schema,
                1,
                1,
                &limits,
            ),
            Err(super::CachedRenderError::CacheInvariantViolation)
        ));
    }

    #[test]
    fn cached_transition_rerenders_early_text_and_rebases_later_atoms() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let old_doc = doc(vec![
            paragraph(vec![text("one")]),
            paragraph(vec![text("middle")]),
            paragraph(vec![text("before "), inline_atom("mention")]),
        ]);
        let new_doc = doc(vec![
            paragraph(vec![text("one expanded")]),
            paragraph(vec![text("middle")]),
            paragraph(vec![text("before "), inline_atom("mention")]),
        ]);
        let old_render = render_blocks(&old_doc, &schema);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits)
            .expect("old document should be cacheable");

        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[0], &limits)
            .expect("transition should be cacheable");
        let new_render = render_blocks(&new_doc, &schema);

        assert_eq!(cache.materialize(), old_render);
        assert_eq!(transition.cache.materialize(), new_render);
        assert_eq!(transition.rerendered_new_blocks, 1);
        let CachedRenderTransitionUpdate::Patch(patch) = transition.update else {
            panic!("expected an exact contiguous patch");
        };
        let mut reconstructed = old_render;
        reconstructed.splice(
            patch.start_index..patch.start_index + patch.delete_count,
            patch.blocks,
        );
        assert_eq!(reconstructed, new_render);
    }

    #[test]
    fn cached_transition_rebases_every_position_bearing_render_variant() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let old_doc = doc(vec![
            paragraph(vec![text("a")]),
            paragraph(vec![text("later"), hard_break(), inline_atom("inline")]),
            horizontal_rule(),
            opaque_block_atom("block"),
        ]);
        let new_doc = doc(vec![
            paragraph(vec![text("a much longer prefix")]),
            paragraph(vec![text("later"), hard_break(), inline_atom("inline")]),
            horizontal_rule(),
            opaque_block_atom("block"),
        ]);
        let old_render = render_blocks(&old_doc, &schema);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[0], &limits)
            .unwrap();
        let expected = render_blocks(&new_doc, &schema);

        assert_eq!(transition.rerendered_new_blocks, 1);
        assert_update_reconstructs(old_render.clone(), &transition, &expected);

        let reverse = transition
            .cache
            .transition(&new_doc, &old_doc, &schema, &[0], &limits)
            .unwrap();
        assert_eq!(reverse.rerendered_new_blocks, 1);
        assert_update_reconstructs(expected, &reverse, &old_render);
    }

    #[test]
    fn cached_transition_handles_mark_only_change() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let old_doc = doc(vec![paragraph(vec![text("marked")])]);
        let mark = Mark::new("bold".to_string(), HashMap::new());
        let new_doc = doc(vec![paragraph(vec![Node::text(
            "marked".to_string(),
            vec![mark],
        )])]);
        let old_render = render_blocks(&old_doc, &schema);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[0], &limits)
            .unwrap();
        let expected = render_blocks(&new_doc, &schema);

        assert_eq!(transition.rerendered_new_blocks, 1);
        assert!(matches!(
            transition.update,
            CachedRenderTransitionUpdate::Patch(_)
        ));
        assert_update_reconstructs(old_render, &transition, &expected);
    }

    #[test]
    fn cached_transition_handles_top_level_insert_and_delete() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let initial = doc(vec![
            paragraph(vec![text("one")]),
            paragraph(vec![text("three")]),
        ]);
        let inserted = doc(vec![
            paragraph(vec![text("one")]),
            paragraph(vec![text("two")]),
            paragraph(vec![text("three")]),
        ]);

        let initial_render = render_blocks(&initial, &schema);
        let initial_cache = CachedRenderBlocks::build(&initial, &schema, &limits).unwrap();
        let insertion = initial_cache
            .transition(&initial, &inserted, &schema, &[1], &limits)
            .unwrap();
        let inserted_render = render_blocks(&inserted, &schema);
        assert_eq!(insertion.rerendered_new_blocks, 1);
        assert_update_reconstructs(initial_render, &insertion, &inserted_render);

        let deletion = insertion
            .cache
            .transition(&inserted, &initial, &schema, &[1], &limits)
            .unwrap();
        assert_eq!(deletion.rerendered_new_blocks, 0);
        assert_update_reconstructs(
            inserted_render,
            &deletion,
            &render_blocks(&initial, &schema),
        );
    }

    #[test]
    fn cached_transition_handles_lists_and_rebases_later_atom() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let list = |first: &str| {
            bullet_list(vec![
                list_item(vec![paragraph(vec![text(first)])]),
                list_item(vec![paragraph(vec![text("second")])]),
            ])
        };
        let old_doc = doc(vec![list("first"), paragraph(vec![inline_atom("later")])]);
        let new_doc = doc(vec![
            list("first item expanded"),
            paragraph(vec![inline_atom("later")]),
        ]);
        let old_render = render_blocks(&old_doc, &schema);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[0], &limits)
            .unwrap();
        let expected = render_blocks(&new_doc, &schema);

        assert_eq!(transition.rerendered_new_blocks, 1);
        assert_update_reconstructs(old_render, &transition, &expected);
    }

    #[test]
    fn cached_transition_classifies_net_zero_as_none_even_with_invalid_hint() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let document = doc(vec![paragraph(vec![text("same")])]);
        let cache = CachedRenderBlocks::build(&document, &schema, &limits).unwrap();
        let transition = cache
            .transition(&document, &document, &schema, &[usize::MAX], &limits)
            .unwrap();

        assert_eq!(transition.rerendered_new_blocks, 0);
        assert_eq!(transition.update, CachedRenderTransitionUpdate::None);
    }

    #[test]
    fn cached_transition_falls_back_when_schema_fingerprint_changes() {
        let old_schema = tiptap_schema();
        let new_schema = prosemirror_schema();
        let limits = ResourceLimits::default();
        let document = doc(vec![paragraph(vec![text("same document")])]);
        let cache = CachedRenderBlocks::build(&document, &old_schema, &limits).unwrap();

        let transition = cache
            .transition(&document, &document, &new_schema, &[], &limits)
            .unwrap();

        assert_eq!(
            transition.update,
            CachedRenderTransitionUpdate::Full(render_blocks(&document, &new_schema))
        );
    }

    #[test]
    fn cached_transition_uses_full_fallback_for_invalid_hint() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let old_doc = doc(vec![paragraph(vec![text("old")])]);
        let new_doc = doc(vec![paragraph(vec![text("new")])]);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[1], &limits)
            .unwrap();

        assert_eq!(
            transition.update,
            CachedRenderTransitionUpdate::Full(render_blocks(&new_doc, &schema))
        );
    }

    #[test]
    fn cached_transition_uses_full_fallback_when_changed_document_renders_identically() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let ignored = |flag: bool| {
            Node::element(
                "unrecognisedContainer".to_string(),
                HashMap::from([("flag".to_string(), serde_json::Value::Bool(flag))]),
                Fragment::from(vec![]),
            )
        };
        let old_doc = doc(vec![ignored(false)]);
        let new_doc = doc(vec![ignored(true)]);
        let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
        let transition = cache
            .transition(&old_doc, &new_doc, &schema, &[0], &limits)
            .unwrap();

        assert_eq!(
            transition.update,
            CachedRenderTransitionUpdate::Full(render_blocks(&new_doc, &schema))
        );
    }

    #[test]
    fn cached_render_build_obeys_document_node_limit() {
        let schema = tiptap_schema();
        let limits = ResourceLimits {
            max_document_nodes: 2,
            ..ResourceLimits::default()
        };
        let document = doc(vec![paragraph(vec![text("too many nodes")])]);

        assert!(matches!(
            CachedRenderBlocks::build(&document, &schema, &limits),
            Err(super::CachedRenderError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn cached_render_build_uses_canonical_root_depth_one() {
        let schema = tiptap_schema();
        let exact_limits = ResourceLimits {
            max_document_depth: 3,
            ..ResourceLimits::default()
        };
        let over_limits = ResourceLimits {
            max_document_depth: 2,
            ..ResourceLimits::default()
        };
        let document = doc(vec![paragraph(vec![text("depth three")])]);

        assert!(CachedRenderBlocks::build(&document, &schema, &exact_limits).is_ok());
        assert!(matches!(
            CachedRenderBlocks::build(&document, &schema, &over_limits),
            Err(super::CachedRenderError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn cached_render_build_rejects_root_width_over_remaining_node_budget() {
        let schema = tiptap_schema();
        let limits = ResourceLimits {
            max_document_nodes: 3,
            ..ResourceLimits::default()
        };
        let document = doc(vec![
            paragraph(vec![]),
            paragraph(vec![]),
            paragraph(vec![]),
        ]);

        assert!(matches!(
            CachedRenderBlocks::build(&document, &schema, &limits),
            Err(super::CachedRenderError::ResourceLimitExceeded)
        ));
    }

    #[test]
    fn cached_render_build_rejects_ordered_list_number_overflow() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let document = doc(vec![ordered_list(
            u32::MAX,
            vec![
                list_item(vec![paragraph(vec![text("one")])]),
                list_item(vec![paragraph(vec![text("two")])]),
            ],
        )]);

        assert!(matches!(
            CachedRenderBlocks::build(&document, &schema, &limits),
            Err(super::CachedRenderError::PositionOverflow)
        ));
    }

    #[test]
    fn incremental_ordered_list_indices_are_exact_or_structured_overflow() {
        let schema = tiptap_schema();
        let exact = doc(vec![ordered_list(
            u32::MAX,
            vec![list_item(vec![paragraph(vec![text("last")])])],
        )]);

        let exact_blocks = try_render_blocks(&exact, &schema).expect("u32::MAX must render");
        let RenderElement::BlockStart {
            list_context: Some(context),
            ..
        } = &exact_blocks[0][0]
        else {
            panic!("ordered-list item must carry a list context");
        };
        assert_eq!(context.index, u32::MAX);

        let overflow = doc(vec![ordered_list(
            u32::MAX,
            vec![
                list_item(vec![paragraph(vec![text("last")])]),
                list_item(vec![paragraph(vec![text("overflow")])]),
            ],
        )]);
        assert!(matches!(
            try_render_blocks(&overflow, &schema),
            Err(super::CachedRenderError::PositionOverflow)
        ));
    }

    #[test]
    fn ordered_list_start_defaults_only_when_absent_and_rejects_present_malformed_values() {
        let schema = tiptap_schema();
        let limits = ResourceLimits::default();
        let missing = doc(vec![ordered_list_with_start(
            None,
            vec![list_item(vec![paragraph(vec![text("first")])])],
        )]);

        let blocks = try_render_blocks(&missing, &schema).expect("missing start defaults to one");
        let RenderElement::BlockStart {
            list_context: Some(context),
            ..
        } = &blocks[0][0]
        else {
            panic!("ordered-list item must carry a list context");
        };
        assert_eq!(context.index, 1);

        for start in [
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::Value::Null,
            serde_json::json!("1"),
            serde_json::json!(u64::from(u32::MAX) + 1),
        ] {
            let malformed = doc(vec![ordered_list_with_start(
                Some(start),
                vec![list_item(vec![paragraph(vec![text("bad")])])],
            )]);
            assert!(matches!(
                CachedRenderBlocks::build(&malformed, &schema, &limits),
                Err(super::CachedRenderError::PositionOverflow)
            ));
            assert!(matches!(
                try_render_blocks(&malformed, &schema),
                Err(super::CachedRenderError::PositionOverflow)
            ));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn cached_transition_always_reconstructs_and_matches_full_render(
            values in prop::collection::vec("[a-z]{0,12}", 1..8),
            replacement in "[a-z]{0,12}",
            raw_index in any::<usize>(),
        ) {
            let schema = tiptap_schema();
            let limits = ResourceLimits::default();
            let index = raw_index % values.len();
            let old_doc = doc(values.iter().map(|value| paragraph(vec![text(value)])).collect());
            let mut new_values = values;
            new_values[index] = replacement;
            let new_doc = doc(
                new_values
                    .iter()
                    .map(|value| paragraph(vec![text(value)]))
                    .collect(),
            );
            let old_render = render_blocks(&old_doc, &schema);
            let cache = CachedRenderBlocks::build(&old_doc, &schema, &limits).unwrap();
            let transition = cache
                .transition(&old_doc, &new_doc, &schema, &[index], &limits)
                .unwrap();
            let expected = render_blocks(&new_doc, &schema);

            assert_update_reconstructs(old_render, &transition, &expected);
        }
    }
}
