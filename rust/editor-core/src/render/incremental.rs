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
#[path = "incremental_tests.rs"]
mod tests;
