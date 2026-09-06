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

include!("incremental/cache.rs");

include!("incremental/cache_helpers.rs");

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
                language: node
                    .attrs()
                    .get("language")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
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
                language: node
                    .attrs()
                    .get("language")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
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
                language: node
                    .attrs()
                    .get("language")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
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
