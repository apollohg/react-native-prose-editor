//! Document backend: owns the document, position map, and undo history.
//!
//! The `DocumentBackend` trait defines the interface for applying transactions
//! and querying state. `StandaloneBackend` is the single-user implementation.

use std::collections::HashMap;

use crate::history::UndoHistory;
use crate::model::{Document, Fragment};
use crate::position::PositionMap;
use crate::render::generate::generate;
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
        selection_before: &Selection,
        selection_after: &Selection,
    ) -> Result<DocState, TransformError>;

    /// Reference to the current document.
    fn document(&self) -> &Document;

    /// Generate render elements for the current document.
    fn to_render_elements(&self, schema: &Schema) -> Vec<RenderElement>;

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
}

impl StandaloneBackend {
    /// Create a new backend from an initial document.
    pub fn new(doc: Document) -> Self {
        let pos_map = PositionMap::build(&doc);
        Self {
            doc,
            pos_map,
            history: UndoHistory::with_default_depth(),
        }
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
            let inv = self.invert_step(step, &current_doc);
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
    fn invert_step(&self, step: &Step, doc: &Document) -> Step {
        match step {
            Step::InsertText { pos, text, .. } => {
                let len = text.chars().count() as u32;
                Step::DeleteRange {
                    from: *pos,
                    to: pos + len,
                }
            }
            Step::DeleteRange { from, to } => {
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
                // Scan the pre-step document to find which text segments
                // actually had this mark, and extract the full mark (with
                // attrs) from each. We build a single AddMark using the
                // mark found in the document. If the mark carried attrs
                // (e.g. link href), this preserves them.
                let mark = extract_mark_in_range(doc, *from, *to, mark_type);
                Step::AddMark {
                    from: *from,
                    to: *to,
                    mark,
                }
            }
            Step::SplitBlock { pos, .. } => {
                // Splitting at pos inserts 2 tokens (close tag + open tag).
                // In the post-split document, the block boundary where we need
                // to join is at pos + 1 (the open tag of the new block).
                Step::JoinBlocks { pos: pos + 1 }
            }
            Step::JoinBlocks { pos } => {
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
            Step::WrapInList {
                from,
                to: _,
                list_type: _,
                item_type: _,
                attrs: _,
            } => {
                // Inverse of wrapping is unwrapping. We need a position inside
                // the first list item. The first list item's content starts at
                // from + 2 (after list_open + li_open).
                Step::UnwrapFromList { pos: from + 2 }
            }
            Step::UnwrapFromList { pos } => {
                // Resolve the containing list node from the pre-step document
                // to get its type, attrs, and the range of content being unwrapped.
                let (list_type, item_type, list_attrs, wrap_from, wrap_to) =
                    resolve_list_context_at(doc, *pos);
                Step::WrapInList {
                    from: wrap_from,
                    to: wrap_to,
                    list_type,
                    item_type,
                    attrs: list_attrs,
                }
            }
            Step::IndentListItem { .. } | Step::OutdentListItem { .. } => {
                let original_content = doc
                    .root()
                    .content()
                    .cloned()
                    .unwrap_or_else(Fragment::empty);
                Step::ReplaceRange {
                    from: 0,
                    to: doc.content_size(),
                    content: original_content,
                }
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
                let original_content = extract_fragment_in_range(doc, *from, *to);
                Step::ReplaceRange {
                    from: *from,
                    to: from + content.size(),
                    content: original_content,
                }
            }
        }
    }
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
        selection_before: &Selection,
        selection_after: &Selection,
    ) -> Result<DocState, TransformError> {
        // 1. Compute inverse steps before applying (uses current doc state).
        let inverse_steps = self.compute_inverse_steps(tx, schema);

        // 2. Apply the transaction to get the new document.
        let (new_doc, step_map) = tx.apply(&self.doc, schema)?;

        // 3. Update position map.
        self.pos_map.update(&step_map, &new_doc);

        // 4. Generate render elements.
        let render_elements = generate(&new_doc, schema);

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

        Ok(DocState {
            doc: new_doc,
            render_elements,
            selection_update,
        })
    }

    fn document(&self) -> &Document {
        &self.doc
    }

    fn to_render_elements(&self, schema: &Schema) -> Vec<RenderElement> {
        generate(&self.doc, schema)
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

        match tx.apply(&self.doc, schema) {
            Ok((new_doc, step_map)) => {
                self.pos_map.update(&step_map, &new_doc);
                let render_elements = generate(&new_doc, schema);
                self.doc = new_doc.clone();
                Some(DocState {
                    doc: new_doc,
                    render_elements,
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

        match tx.apply(&self.doc, schema) {
            Ok((new_doc, step_map)) => {
                self.pos_map.update(&step_map, &new_doc);
                let render_elements = generate(&new_doc, schema);
                self.doc = new_doc.clone();
                Some(DocState {
                    doc: new_doc,
                    render_elements,
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

/// Resolve the list context at a position inside a list item for building
/// the inverse of UnwrapFromList. Returns (list_type, item_type, list_attrs,
/// wrap_from, wrap_to) where wrap_from..wrap_to covers the content that was
/// inside the list item being unwrapped.
fn resolve_list_context_at(
    doc: &Document,
    pos: u32,
) -> (String, String, HashMap<String, serde_json::Value>, u32, u32) {
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
            if child.node_type() == "listItem" {
                let parent_is_list = current_node.node_type() == "bulletList"
                    || current_node.node_type() == "orderedList"
                    || current_node.node_type().ends_with("List");

                if parent_is_list {
                    let li_content_size = child.content_size();
                    let list_type = current_node.node_type().to_string();
                    let item_type = child.node_type().to_string();
                    let list_attrs = current_node.attrs().clone();

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

                    return (list_type, item_type, list_attrs, wrap_from, wrap_to);
                }
            }

            abs_pos = child_abs_pos;
            current_node = child;
        }
    }

    // Fallback
    (
        "bulletList".to_string(),
        "listItem".to_string(),
        HashMap::new(),
        pos,
        pos,
    )
}
