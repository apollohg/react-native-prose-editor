//! Document backend: owns the document, position map, and undo history.
//!
//! The `DocumentBackend` trait defines the interface for applying transactions
//! and querying state. `StandaloneBackend` is the single-user implementation.

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::history::UndoHistory;
use crate::model::{Document, Fragment, Node};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::render::incremental::{
    contiguous_render_blocks_patch, flatten_render_blocks, render_blocks, RenderBlocksPatch,
};
use crate::render::RenderElement;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::{Source, Step, Transaction, TransformError};

// ---------------------------------------------------------------------------
// DocState
// ---------------------------------------------------------------------------

/// The result of applying a transaction or undo/redo operation.
pub struct DocState {
    pub doc: Document,
    pub render_elements: Vec<RenderElement>,
    pub render_blocks: Vec<Vec<RenderElement>>,
    pub render_patch: Option<RenderBlocksPatch>,
    pub selection_update: Option<Selection>,
}

// ---------------------------------------------------------------------------
// DocumentBackend trait
// ---------------------------------------------------------------------------

/// Trait for document backends (standalone, CRDT, etc.).
pub trait DocumentBackend {
    /// Apply a transaction, returning the new state.
    ///
    /// `selection_before` is the current selection before the transaction is
    /// applied (used for undo history).
    fn apply_transaction(
        &mut self,
        tx: &Transaction,
        schema: &Schema,
        prepared_doc: &Document,
        prepared_map: &crate::transform::StepMap,
        selection_before: &Selection,
        selection_after: &Selection,
    ) -> Result<DocState, TransformError>;

    /// Reference to the current document.
    fn document(&self) -> &Document;

    /// Generate render elements for the current document.
    fn to_render_elements(&self, schema: &Schema) -> Vec<RenderElement>;

    /// Generate segmented top-level render blocks for the current document.
    fn to_render_blocks(&self, schema: &Schema) -> Vec<Vec<RenderElement>>;

    /// Reference to the current position map.
    fn position_map(&self) -> &PositionMap;

    /// Undo the last history entry, returning the resulting state.
    fn undo(&mut self, schema: &Schema) -> Option<DocState>;

    /// Redo the last undone entry, returning the resulting state.
    fn redo(&mut self, schema: &Schema) -> Option<DocState>;

    /// Whether there are entries on the undo stack.
    fn can_undo(&self) -> bool;

    /// Whether there are entries on the redo stack.
    fn can_redo(&self) -> bool;
}

// ---------------------------------------------------------------------------
// StandaloneBackend
// ---------------------------------------------------------------------------

/// Single-user backend: owns document, position map, and undo history.
pub struct StandaloneBackend {
    doc: Document,
    pos_map: PositionMap,
    history: UndoHistory,
    render_blocks: Vec<Vec<RenderElement>>,
    resource_limits: ResourceLimits,
}

impl StandaloneBackend {
    /// Create a new backend from an initial document.
    pub fn new(doc: Document, schema: &Schema, resource_limits: ResourceLimits) -> Self {
        let pos_map = PositionMap::build(&doc, schema);
        let render_blocks = render_blocks(&doc, schema);
        Self {
            doc,
            pos_map,
            history: UndoHistory::with_default_depth(),
            render_blocks,
            resource_limits,
        }
    }

    fn apply_render_blocks_patch(
        current_blocks: &[Vec<RenderElement>],
        patch: &RenderBlocksPatch,
    ) -> Option<Vec<Vec<RenderElement>>> {
        if patch.start_index > current_blocks.len()
            || patch.start_index + patch.delete_count > current_blocks.len()
        {
            return None;
        }

        let mut next_blocks = Vec::with_capacity(
            current_blocks.len() + patch.blocks.len().saturating_sub(patch.delete_count),
        );
        next_blocks.extend_from_slice(&current_blocks[..patch.start_index]);
        next_blocks.extend(patch.blocks.iter().cloned());
        next_blocks.extend_from_slice(&current_blocks[(patch.start_index + patch.delete_count)..]);
        Some(next_blocks)
    }

    fn classify_position_map_update(steps: &[Step]) -> UpdateMode {
        if steps
            .iter()
            .all(|step| matches!(step, Step::AddMark { .. } | Step::RemoveMark { .. }))
        {
            return UpdateMode::MarksOnly;
        }

        if steps
            .iter()
            .all(|step| matches!(step, Step::InsertText { .. } | Step::DeleteRange { .. }))
        {
            return UpdateMode::InlineTextOnly;
        }

        UpdateMode::Rebuild
    }

    /// Compute inverse steps for a transaction before applying it.
    ///
    /// For each step, we compute the inverse that would undo it:
    /// - InsertText at pos with N chars -> DeleteRange { from: pos, to: pos + N }
    /// - DeleteRange { from, to } -> InsertText at from with the deleted text (extracted from current doc)
    /// - AddMark { from, to, mark } -> RemoveMark { from, to, mark_type }
    /// - RemoveMark { from, to, mark_type } -> we can't easily re-add marks without knowing which were there,
    ///   so we record the original text nodes' marks. For now, use ReplaceRange to restore.
    /// - SplitBlock at pos -> JoinBlocks at pos (the two blocks rejoin)
    /// - JoinBlocks at pos -> SplitBlock at pos (but we need the original node type)
    ///
    /// For steps that are hard to invert precisely, we fall back to computing
    /// the inverse from the document state before and after each step.
    fn compute_inverse_steps(&self, tx: &Transaction, schema: &Schema) -> Vec<Step> {
        let mut inverse_steps = Vec::new();
        let mut current_doc = self.doc.clone();

        for step in &tx.steps {
            let inv = self.invert_step(step, &current_doc, schema);
            inverse_steps.push(inv);

            // Apply the step to advance current_doc for the next inverse computation.
            if let Ok((new_doc, _)) =
                crate::transform::apply::apply_step(&current_doc, step, schema)
            {
                current_doc = new_doc;
            }
        }

        // Inverse steps are applied in reverse order during undo.
        inverse_steps.reverse();
        inverse_steps
    }

    /// Compute the inverse of a single step given the current document state.
    fn invert_step(&self, step: &Step, doc: &Document, schema: &Schema) -> Step {
        match step {
            Step::InsertText { pos, text, .. } => {
                let len = text.chars().count() as u32;
                Step::DeleteRange {
                    from: *pos,
                    to: pos + len,
                }
            }
            step @ Step::DeleteRange { from, to } => {
                if extract_exact_sibling_fragment(doc, *from, *to).is_some() {
                    if let Some(inverse) = localized_structural_inverse(schema, doc, step) {
                        return inverse;
                    }
                }
                if let Some(inverse) = structural_delete_inverse(schema, doc, *from, *to) {
                    return inverse;
                }
                // Extract the text being deleted from the current document.
                let deleted_text = extract_text_in_range(doc, *from, *to);
                // Reconstruct marks from the current document at the deletion point.
                let marks = extract_marks_at(doc, *from);
                Step::InsertText {
                    pos: *from,
                    text: deleted_text,
                    marks,
                }
            }
            Step::AddMark { from, to, mark } => Step::RemoveMark {
                from: *from,
                to: *to,
                mark_type: mark.mark_type().to_string(),
            },
            Step::RemoveMark {
                from,
                to,
                mark_type,
            } => {
                if let Some(content) = extract_inline_fragment_in_range(doc, *from, *to) {
                    return Step::ReplaceRange {
                        from: *from,
                        to: *to,
                        content,
                    };
                }
                // Defensive fallback for a range that cannot be represented
                // as one inline fragment.
                let mark = extract_mark_in_range(doc, *from, *to, mark_type);
                Step::AddMark {
                    from: *from,
                    to: *to,
                    mark,
                }
            }
            step @ Step::SplitBlock { pos, .. } => {
                if let Some(inverse) = localized_structural_inverse(schema, doc, step) {
                    return inverse;
                }
                // Splitting at pos inserts 2 tokens (close tag + open tag).
                // In the post-split document, the block boundary where we need
                // to join is at pos + 1 (the open tag of the new block).
                Step::JoinBlocks { pos: pos + 1 }
            }
            Step::JoinBlocks { pos } => {
                if let Some(inverse) = list_item_join_inverse(schema, doc, *pos) {
                    // `SplitBlock` only splits text blocks. Joining list items
                    // merges their block children, so the ordinary inverse
                    // below cannot reconstruct the second list-item wrapper.
                    // Restore only the merged item, keeping unrelated siblings
                    // out of the retained history payload.
                    return inverse;
                }
                // To undo a join, we need to split the merged block.
                // The join removed 2 tokens (close tag + open tag) at the
                // boundary. In the post-join doc, the split position is
                // pos - 1 (one less because the close tag before the
                // boundary was removed).
                // Resolve the second block's type and attrs from the pre-step doc.
                let (node_type, attrs) = resolve_second_block_at(doc, *pos);
                Step::SplitBlock {
                    pos: pos - 1,
                    node_type,
                    attrs,
                }
            }
            step @ Step::WrapInList { from, .. } => {
                if let Some(inverse) = localized_structural_inverse(schema, doc, step) {
                    return inverse;
                }
                // Inverse of wrapping is unwrapping. We need a position inside
                // the first list item. The first list item's content starts at
                // from + 2 (after list_open + li_open).
                Step::UnwrapFromList { pos: from + 2 }
            }
            step @ Step::UnwrapFromList { pos } => {
                if let Some(inverse) = localized_structural_inverse(schema, doc, step) {
                    return inverse;
                }
                // Resolve the containing list node from the pre-step document
                // to get its type, attrs, and the range of content being unwrapped.
                let list_context = resolve_list_context_at(schema, doc, *pos);
                Step::WrapInList {
                    from: list_context.wrap_from,
                    to: list_context.wrap_to,
                    list_type: list_context.list_type,
                    item_type: list_context.item_type,
                    attrs: list_context.list_attrs,
                    item_attrs: list_context.item_attrs,
                }
            }
            step @ (Step::IndentListItem { .. } | Step::OutdentListItem { .. }) => {
                localized_structural_inverse(schema, doc, step).unwrap_or_else(|| {
                    // The transform was already previewed successfully before
                    // inverse computation, so failing to derive a local exact
                    // inverse is an internal invariant violation.
                    panic!("validated list transform must have a localized exact inverse")
                })
            }
            Step::InsertNode { pos, node } => {
                let node_size = node.node_size();
                Step::DeleteRange {
                    from: *pos,
                    to: pos + node_size,
                }
            }
            Step::UpdateNodeAttrs { pos, .. } => {
                let original_attrs = resolve_node_attrs_at(doc, *pos);
                Step::UpdateNodeAttrs {
                    pos: *pos,
                    attrs: original_attrs,
                }
            }
            Step::ReplaceRange { from, to, content } => {
                // Inverse: replace the inserted content with the original content.
                // Preserve complete nodes when the replaced range is an exact
                // sibling slice. Inline/partial ranges retain the established
                // text-and-marks extraction path below.
                let original_content = extract_exact_sibling_fragment(doc, *from, *to)
                    .unwrap_or_else(|| extract_fragment_in_range(doc, *from, *to));
                // The editor computes and validates the post-step document
                // before the backend asks for inverses, so this inserted end
                // is already proven representable in the u32 position model.
                let inverse_to = from
                    .checked_add(content.size())
                    .expect("validated ReplaceRange end must fit the position model");
                Step::ReplaceRange {
                    from: *from,
                    to: inverse_to,
                    content: original_content,
                }
            }
        }
    }
}

fn extract_exact_sibling_fragment(doc: &Document, from: u32, to: u32) -> Option<Fragment> {
    if from >= to {
        return None;
    }
    let resolved_from = doc.resolve(from).ok()?;
    let resolved_to = doc.resolve(to).ok()?;
    if resolved_from.node_path != resolved_to.node_path {
        return None;
    }
    let content = resolved_from.parent(doc).content()?;
    let mut offset = 0u32;
    let mut selected = Vec::new();
    let mut started = false;
    for child in content.iter() {
        if !started {
            if offset == resolved_from.parent_offset {
                started = true;
            } else if offset > resolved_from.parent_offset {
                return None;
            }
        }
        let next = offset.checked_add(child.node_size())?;
        if started {
            if next > resolved_to.parent_offset {
                return None;
            }
            selected.push(child.clone());
            if next == resolved_to.parent_offset {
                return Some(Fragment::from(selected));
            }
        }
        offset = next;
    }
    None
}

fn extract_inline_fragment_in_range(doc: &Document, from: u32, to: u32) -> Option<Fragment> {
    if from >= to {
        return None;
    }
    let resolved_from = doc.resolve(from).ok()?;
    let resolved_to = doc.resolve(to).ok()?;
    if resolved_from.node_path != resolved_to.node_path {
        return None;
    }
    let content = resolved_from.parent(doc).content()?;
    let from_offset = resolved_from.parent_offset;
    let to_offset = resolved_to.parent_offset;
    let mut offset = 0u32;
    let mut selected = Vec::new();
    for child in content.iter() {
        let end = offset.checked_add(child.node_size())?;
        let overlap_from = from_offset.max(offset);
        let overlap_to = to_offset.min(end);
        if overlap_from < overlap_to {
            if let Some(text) = child.text_str() {
                let chars = text.chars().collect::<Vec<_>>();
                let local_from = usize::try_from(overlap_from.checked_sub(offset)?).ok()?;
                let local_to = usize::try_from(overlap_to.checked_sub(offset)?).ok()?;
                selected.push(Node::text(
                    chars.get(local_from..local_to)?.iter().collect(),
                    child.marks().to_vec(),
                ));
            } else {
                if overlap_from != offset || overlap_to != end {
                    return None;
                }
                selected.push(child.clone());
            }
        }
        offset = end;
    }
    (!selected.is_empty()).then(|| Fragment::from(selected))
}

fn structural_delete_inverse(schema: &Schema, doc: &Document, from: u32, to: u32) -> Option<Step> {
    if from >= to {
        return None;
    }
    let resolved_from = doc.resolve(from).ok()?;
    let resolved_to = doc.resolve(to).ok()?;
    let common_depth = resolved_from
        .node_path
        .iter()
        .zip(resolved_to.node_path.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if common_depth == resolved_from.node_path.len() && common_depth == resolved_to.node_path.len()
    {
        return None;
    }
    let parent_path = &resolved_from.node_path[..common_depth];
    let parent = doc.node_at(parent_path)?;
    let content = parent.content()?;
    let start = endpoint_child_index(
        content,
        resolved_from.node_path.get(common_depth).copied(),
        resolved_from.parent_offset,
    )?;
    let end = match resolved_to.node_path.get(common_depth).copied() {
        Some(index) => usize::try_from(index).ok()?.checked_add(1)?,
        None => endpoint_child_index(content, None, resolved_to.parent_offset)?,
    };
    if start >= end || end > content.child_count() {
        return None;
    }
    let original = Fragment::from(
        content
            .iter()
            .skip(start)
            .take(end - start)
            .cloned()
            .collect::<Vec<_>>(),
    );
    let range_start = absolute_child_start(doc, parent_path, start)?;
    let delete = Step::DeleteRange { from, to };
    let (post, _) = crate::transform::apply::apply_step(doc, &delete, schema).ok()?;
    let removed = doc.content_size().checked_sub(post.content_size())?;
    let post_size = original.size().checked_sub(removed)?;
    let inverse = Step::ReplaceRange {
        from: range_start,
        to: range_start.checked_add(post_size)?,
        content: original,
    };
    let (restored, _) = crate::transform::apply::apply_step(&post, &inverse, schema).ok()?;
    (&restored == doc).then_some(inverse)
}

fn localized_structural_inverse(schema: &Schema, doc: &Document, step: &Step) -> Option<Step> {
    let (post, _) = crate::transform::apply::apply_step(doc, step, schema).ok()?;
    let diff = crate::command_planner::structural_diff(&post, doc)?;
    let from_index = usize::try_from(diff.from_child).ok()?;
    let to_index = usize::try_from(diff.to_child).ok()?;
    let from = absolute_child_start(&post, &diff.parent_path, from_index)?;
    let to = absolute_child_start(&post, &diff.parent_path, to_index)?;
    let inverse = Step::ReplaceRange {
        from,
        to,
        content: diff.content,
    };
    let (restored, _) = crate::transform::apply::apply_step(&post, &inverse, schema).ok()?;
    (&restored == doc).then_some(inverse)
}

fn endpoint_child_index(
    content: &Fragment,
    path_index: Option<u32>,
    parent_offset: u32,
) -> Option<usize> {
    if let Some(index) = path_index {
        return usize::try_from(index).ok();
    }
    let mut offset = 0u32;
    for (index, child) in content.iter().enumerate() {
        if offset == parent_offset {
            return Some(index);
        }
        offset = offset.checked_add(child.node_size())?;
    }
    (offset == parent_offset).then_some(content.child_count())
}

fn absolute_child_start(doc: &Document, parent_path: &[u32], index: usize) -> Option<u32> {
    let mut node = doc.root();
    let mut content_start = 0u32;
    for path_index in parent_path.iter().copied() {
        let content = node.content()?;
        let index = usize::try_from(path_index).ok()?;
        for sibling in content.iter().take(index) {
            content_start = content_start.checked_add(sibling.node_size())?;
        }
        content_start = content_start.checked_add(1)?;
        node = content.child(index)?;
    }
    for sibling in node.content()?.iter().take(index) {
        content_start = content_start.checked_add(sibling.node_size())?;
    }
    Some(content_start)
}

fn list_item_join_inverse(schema: &Schema, doc: &Document, pos: u32) -> Option<Step> {
    let Ok(resolved) = doc.resolve(pos) else {
        return None;
    };
    let content = resolved.parent(doc).content()?;
    let mut offset = 0;
    for (index, child) in content.iter().enumerate() {
        if offset == resolved.parent_offset && index > 0 {
            let previous = content.child(index - 1)?;
            if !schema.is_list_item(previous.node_type()) || !schema.is_list_item(child.node_type())
            {
                return None;
            }
            let from = pos.checked_sub(previous.node_size())?;
            let merged_size = previous
                .node_size()
                .checked_add(child.node_size())?
                .checked_sub(2)?;
            return Some(Step::ReplaceRange {
                from,
                to: from.checked_add(merged_size)?,
                content: Fragment::from(vec![previous.clone(), child.clone()]),
            });
        }
        offset = offset.checked_add(child.node_size())?;
    }
    None
}

fn resolve_node_attrs_at(
    doc: &Document,
    pos: u32,
) -> std::collections::HashMap<String, serde_json::Value> {
    let resolved = match doc.resolve(pos) {
        Ok(resolved) => resolved,
        Err(_) => return std::collections::HashMap::new(),
    };
    let parent = resolved.parent(doc);
    let content = match parent.content() {
        Some(content) => content,
        None => return std::collections::HashMap::new(),
    };

    let mut offset = 0;
    for child in content.iter() {
        let child_size = child.node_size();
        if !child.is_text() && resolved.parent_offset == offset {
            return child.attrs().clone();
        }
        offset += child_size;
    }

    std::collections::HashMap::new()
}

impl DocumentBackend for StandaloneBackend {
    fn apply_transaction(
        &mut self,
        tx: &Transaction,
        schema: &Schema,
        prepared_doc: &Document,
        prepared_map: &crate::transform::StepMap,
        selection_before: &Selection,
        selection_after: &Selection,
    ) -> Result<DocState, TransformError> {
        // 1. Compute inverse steps before applying (uses current doc state).
        let inverse_steps = self.compute_inverse_steps(tx, schema);

        // 2. The editor supplies the candidate and map after one authoritative
        // validation with its resolved resource limits.
        let new_doc = prepared_doc.clone();
        let step_map = prepared_map;

        // 3. Update position map.
        self.pos_map.update(
            step_map,
            &self.doc,
            &new_doc,
            Self::classify_position_map_update(&tx.steps),
            schema,
        );

        // 4. Generate render elements and a contiguous top-level patch.
        let render_patch = contiguous_render_blocks_patch(&self.doc, &new_doc, schema);
        let render_blocks = render_patch
            .as_ref()
            .and_then(|patch| Self::apply_render_blocks_patch(&self.render_blocks, patch))
            .unwrap_or_else(|| render_blocks(&new_doc, schema));
        let render_elements = flatten_render_blocks(&render_blocks);

        // 5. Map the selection through the step map for a suggested update.
        let selection_update = Some(Selection::cursor(step_map.map_pos(0)));

        // 6. Push to history (unless this is a History-sourced transaction).
        if tx.source != Source::History {
            self.history.push(
                tx.steps.clone(),
                inverse_steps,
                tx.source.clone(),
                selection_before.clone(),
                selection_after.clone(),
            );
        }

        // 7. Update document.
        self.doc = new_doc.clone();
        self.render_blocks = render_blocks.clone();

        Ok(DocState {
            doc: new_doc,
            render_elements,
            render_blocks,
            render_patch,
            selection_update,
        })
    }

    fn document(&self) -> &Document {
        &self.doc
    }

    fn to_render_elements(&self, schema: &Schema) -> Vec<RenderElement> {
        let _ = schema;
        flatten_render_blocks(&self.render_blocks)
    }

    fn to_render_blocks(&self, schema: &Schema) -> Vec<Vec<RenderElement>> {
        let _ = schema;
        self.render_blocks.clone()
    }

    fn position_map(&self) -> &PositionMap {
        &self.pos_map
    }

    fn undo(&mut self, schema: &Schema) -> Option<DocState> {
        let (inverse_steps, saved_selection) = self.history.undo()?;

        let mut tx = Transaction::new(Source::History);
        for step in inverse_steps {
            tx.add_step(step);
        }

        match tx.apply_with_limits(&self.doc, schema, &self.resource_limits) {
            Ok((new_doc, step_map)) => {
                self.pos_map.update(
                    &step_map,
                    &self.doc,
                    &new_doc,
                    Self::classify_position_map_update(&tx.steps),
                    schema,
                );
                let render_patch = contiguous_render_blocks_patch(&self.doc, &new_doc, schema);
                let render_blocks = render_patch
                    .as_ref()
                    .and_then(|patch| Self::apply_render_blocks_patch(&self.render_blocks, patch))
                    .unwrap_or_else(|| render_blocks(&new_doc, schema));
                let render_elements = flatten_render_blocks(&render_blocks);
                self.doc = new_doc.clone();
                self.render_blocks = render_blocks.clone();
                Some(DocState {
                    doc: new_doc,
                    render_elements,
                    render_blocks,
                    render_patch,
                    selection_update: Some(saved_selection),
                })
            }
            Err(_) => None,
        }
    }

    fn redo(&mut self, schema: &Schema) -> Option<DocState> {
        let (redo_steps, saved_selection) = self.history.redo()?;

        let mut tx = Transaction::new(Source::History);
        for step in redo_steps {
            tx.add_step(step);
        }

        match tx.apply_with_limits(&self.doc, schema, &self.resource_limits) {
            Ok((new_doc, step_map)) => {
                self.pos_map.update(
                    &step_map,
                    &self.doc,
                    &new_doc,
                    Self::classify_position_map_update(&tx.steps),
                    schema,
                );
                let render_patch = contiguous_render_blocks_patch(&self.doc, &new_doc, schema);
                let render_blocks = render_patch
                    .as_ref()
                    .and_then(|patch| Self::apply_render_blocks_patch(&self.render_blocks, patch))
                    .unwrap_or_else(|| render_blocks(&new_doc, schema));
                let render_elements = flatten_render_blocks(&render_blocks);
                self.doc = new_doc.clone();
                self.render_blocks = render_blocks.clone();
                Some(DocState {
                    doc: new_doc,
                    render_elements,
                    render_blocks,
                    render_patch,
                    selection_update: Some(saved_selection),
                })
            }
            Err(_) => None,
        }
    }

    fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
}

// ---------------------------------------------------------------------------
// Helpers for inverse step computation
// ---------------------------------------------------------------------------

/// Extract plain text from a document range [from, to).
fn extract_text_in_range(doc: &Document, from: u32, to: u32) -> String {
    if from >= to {
        return String::new();
    }

    let resolved_from = match doc.resolve(from) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };

    let parent = resolved_from.parent(doc);
    let content = match parent.content() {
        Some(c) => c,
        None => return String::new(),
    };

    let from_offset = resolved_from.parent_offset;
    let len = to - from;
    let to_offset = from_offset + len;

    let mut result = String::new();
    let mut offset: u32 = 0;

    for child in content.iter() {
        let child_size = child.node_size();
        let child_start = offset;
        let child_end = offset + child_size;

        if child_end <= from_offset || child_start >= to_offset {
            offset = child_end;
            continue;
        }

        if child.is_text() {
            let text = child.text_str().unwrap();
            let chars: Vec<char> = text.chars().collect();

            let start = if from_offset > child_start {
                (from_offset - child_start) as usize
            } else {
                0
            };
            let end = if to_offset < child_end {
                (to_offset - child_start) as usize
            } else {
                chars.len()
            };

            if start < end && end <= chars.len() {
                let extracted: String = chars[start..end].iter().collect();
                result.push_str(&extracted);
            }
        }

        offset = child_end;
    }

    result
}

/// Extract marks at a given position in the document.
fn extract_marks_at(doc: &Document, pos: u32) -> Vec<crate::model::Mark> {
    let resolved = match doc.resolve(pos) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let parent = resolved.parent(doc);
    let content = match parent.content() {
        Some(c) => c,
        None => return Vec::new(),
    };

    let parent_offset = resolved.parent_offset;
    let mut offset: u32 = 0;

    for child in content.iter() {
        let child_size = child.node_size();
        if child.is_text() && offset <= parent_offset && parent_offset < offset + child_size {
            return child.marks().to_vec();
        }
        offset += child_size;
    }

    Vec::new()
}

/// Extract a Fragment from a document range [from, to).
fn extract_fragment_in_range(doc: &Document, from: u32, to: u32) -> crate::model::Fragment {
    if from >= to {
        return crate::model::Fragment::empty();
    }

    let text = extract_text_in_range(doc, from, to);
    if text.is_empty() {
        return crate::model::Fragment::empty();
    }

    let marks = extract_marks_at(doc, from);
    crate::model::Fragment::from(vec![crate::model::Node::text(text, marks)])
}

/// Extract a mark (with its full attrs) of the given type from the document
/// range [from, to). Scans text nodes in the range to find one that carries
/// the mark, preserving the original attrs (e.g. link href).
fn extract_mark_in_range(
    doc: &Document,
    from: u32,
    to: u32,
    mark_type: &str,
) -> crate::model::Mark {
    if let Ok(resolved_from) = doc.resolve(from) {
        let parent = resolved_from.parent(doc);
        if let Some(content) = parent.content() {
            let from_offset = resolved_from.parent_offset;
            let len = to - from;
            let to_offset = from_offset + len;
            let mut offset: u32 = 0;

            for child in content.iter() {
                let child_size = child.node_size();
                let child_start = offset;
                let child_end = offset + child_size;

                if child.is_text() && child_end > from_offset && child_start < to_offset {
                    // This text node overlaps with the range. Check for the mark.
                    if let Some(m) = child.marks().iter().find(|m| m.mark_type() == mark_type) {
                        return m.clone();
                    }
                }
                offset = child_end;
            }
        }
    }
    // Fallback: construct a mark with no attrs.
    crate::model::Mark::new(mark_type.to_string(), HashMap::new())
}

/// Resolve the second block's type and attrs at a join position from the
/// pre-step document. Returns (node_type, attrs).
fn resolve_second_block_at(
    doc: &Document,
    pos: u32,
) -> (String, HashMap<String, serde_json::Value>) {
    if let Ok(resolved) = doc.resolve(pos) {
        let parent = resolved.parent(doc);
        if let Some(content) = parent.content() {
            let mut offset: u32 = 0;
            for child in content.iter() {
                let child_size = child.node_size();
                if offset == resolved.parent_offset && child.is_element() {
                    return (child.node_type().to_string(), child.attrs().clone());
                }
                offset += child_size;
            }
        }
    }
    ("paragraph".to_string(), HashMap::new())
}

/// The list context resolved at a position inside a list item, used to build
/// the inverse of UnwrapFromList. `wrap_from..wrap_to` covers the content
/// that was inside the list item being unwrapped.
struct ListContext {
    list_type: String,
    item_type: String,
    list_attrs: HashMap<String, serde_json::Value>,
    item_attrs: HashMap<String, serde_json::Value>,
    wrap_from: u32,
    wrap_to: u32,
}

/// Resolve the list context at a position inside a list item for building
/// the inverse of UnwrapFromList. List and item nodes are recognized by
/// their schema role (`Schema::is_list` / `Schema::is_list_item`), never by
/// node-type name.
fn resolve_list_context_at(schema: &Schema, doc: &Document, pos: u32) -> ListContext {
    // Walk the resolved path to find the list item and its parent list.
    if let Ok(resolved) = doc.resolve(pos) {
        let path = &resolved.node_path;

        // Find the list item node in the path.
        let mut current_node = doc.root();
        let mut abs_pos: u32 = 0;

        for (depth_idx, &child_idx) in path.iter().enumerate() {
            let content = match current_node.content() {
                Some(c) => c,
                None => break,
            };

            // Compute absolute position of this child's open tag.
            let mut child_abs_pos = abs_pos + 1; // after parent's open tag
            for i in 0..(child_idx as usize) {
                child_abs_pos += content.child(i).unwrap().node_size();
            }

            let child = match content.child(child_idx as usize) {
                Some(c) => c,
                None => break,
            };

            // Check if this child is a list item and its parent is a list.
            if schema.is_list_item(child.node_type()) {
                let parent_is_list = schema.is_list(current_node.node_type());

                if parent_is_list {
                    let li_content_size = child.content_size();
                    let list_type = current_node.node_type().to_string();
                    let item_type = child.node_type().to_string();
                    let list_attrs = current_node.attrs().clone();
                    let item_attrs = child.attrs().clone();

                    // Compute the absolute position of the list node in doc
                    // content. The list is `current_node`, and its content
                    // starts at `abs_pos + 1`. The list's position in the
                    // doc content is `abs_pos` (just before its open tag)
                    // if abs_pos > 0, or 0 if it's the first child.
                    // Actually, abs_pos tracks the position after the
                    // parent's open tag plus preceding siblings. For a
                    // root-level list, abs_pos will be 0 only if there are
                    // no preceding siblings.

                    // Compute the list's start position in doc content:
                    // walk from the root to find where the list starts.
                    let list_path = &path[..depth_idx];
                    let mut list_start: u32 = 0;
                    let mut walk_node = doc.root();
                    for &idx in list_path {
                        let walk_content = walk_node.content().unwrap();
                        for i in 0..(idx as usize) {
                            list_start += walk_content.child(i).unwrap().node_size();
                        }
                        list_start += 1; // open tag of this node
                        walk_node = walk_content.child(idx as usize).unwrap();
                    }
                    // Now list_start is the position in doc content where the
                    // list node starts (before its open tag). After unwrap,
                    // the extracted content appears at this position.
                    let wrap_from = list_start;
                    let wrap_to = list_start + li_content_size;

                    return ListContext {
                        list_type,
                        item_type,
                        list_attrs,
                        item_attrs,
                        wrap_from,
                        wrap_to,
                    };
                }
            }

            abs_pos = child_abs_pos;
            current_node = child;
        }
    }

    // Fallback for unresolvable positions (no list-role node found on the
    // path): an empty wrap range with preset names. Not list detection —
    // detection above is role-driven; this is a last-resort default.
    ListContext {
        list_type: "bulletList".to_string(),
        item_type: "listItem".to_string(),
        list_attrs: HashMap::new(),
        item_attrs: HashMap::new(),
        wrap_from: pos,
        wrap_to: pos,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serialize::{from_prosemirror_json, UnknownTypeMode};

    #[test]
    fn list_item_join_inverse_retains_only_the_local_items() {
        let schema = crate::tiptap_schema();
        let prefix = "x".repeat(4096);
        let source = serde_json::json!({
            "type":"doc",
            "content":[
                {"type":"paragraph","content":[{"type":"text","text":prefix}]},
                {"type":"bulletList","content":[
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},
                    {"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}
                ]},
                {"type":"paragraph","content":[{"type":"text","text":"untouched"}]}
            ]
        });
        let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Error).unwrap();
        let prefix_size = document.root().child(0).unwrap().node_size();
        let list = document.root().child(1).unwrap();
        let first = list.child(0).unwrap();
        let join = prefix_size + 1 + first.node_size();

        let Step::ReplaceRange { from, to, content } =
            list_item_join_inverse(&schema, &document, join).unwrap()
        else {
            panic!("list-item join inverse must be a localized replacement")
        };
        assert_eq!(from, prefix_size + 1);
        assert_eq!(
            to - from,
            first.node_size() + list.child(1).unwrap().node_size() - 2
        );
        assert_eq!(content.child_count(), 2);
        assert!(from > 0);
        assert!(to < document.content_size());
    }

    #[test]
    fn structural_delete_inverse_is_local_at_root_and_nested_parent() {
        let schema = crate::tiptap_schema();
        let root_source = serde_json::json!({
            "type":"doc",
            "content":[
                {"type":"paragraph","content":[{"type":"text","text":"ab"}]},
                {"type":"paragraph","content":[{"type":"text","text":"cd"}]},
                {"type":"paragraph","content":[{"type":"text","text":"untouched"}]}
            ]
        });
        let root = from_prosemirror_json(&root_source, &schema, UnknownTypeMode::Error).unwrap();
        let Step::ReplaceRange { from, to, content } =
            structural_delete_inverse(&schema, &root, 2, 6).unwrap()
        else {
            panic!("cross-block root delete must have a localized inverse")
        };
        assert_eq!((from, to, content.child_count()), (0, 4, 2));
        assert!(to < root.content_size());

        let nested_source = serde_json::json!({
            "type":"doc",
            "content":[
                {"type":"paragraph","content":[{"type":"text","text":"z"}]},
                {"type":"blockquote","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"ab"}]},
                    {"type":"paragraph","content":[{"type":"text","text":"cd"}]}
                ]},
                {"type":"paragraph","content":[{"type":"text","text":"untouched"}]}
            ]
        });
        let nested =
            from_prosemirror_json(&nested_source, &schema, UnknownTypeMode::Error).unwrap();
        let Step::ReplaceRange { from, to, content } =
            structural_delete_inverse(&schema, &nested, 6, 10).unwrap()
        else {
            panic!("nested cross-block delete must have a localized inverse")
        };
        assert_eq!((from, to, content.child_count()), (4, 8, 2));
        assert!(from > 0);
        assert!(to < nested.content_size());

        assert!(structural_delete_inverse(&schema, &nested, 5, 6).is_none());
    }
}
