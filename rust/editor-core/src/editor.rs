//! The main Editor API surface.
//!
//! Owns a schema, backend, selection, and interceptor pipeline. Provides
//! high-level methods for editing, querying state, and serialization.

use std::collections::HashMap;

use crate::backend::{DocumentBackend, StandaloneBackend};
use crate::intercept::{InterceptError, InterceptorPipeline};
use crate::model::resolved_pos::ResolvedPos;
use crate::model::{Document, Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::render::RenderElement;
use crate::schema::{NodeRole, Schema};
use crate::selection::Selection;
use crate::serialize;
use crate::transform::steps::rebuild_element;
use crate::transform::{Source, Step, Transaction, TransformError};

// ---------------------------------------------------------------------------
// EditorError
// ---------------------------------------------------------------------------

/// Unified error type for editor operations.
#[derive(Debug)]
pub enum EditorError {
    Transform(TransformError),
    Intercept(InterceptError),
    Parse(String),
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditorError::Transform(e) => write!(f, "transform error: {e}"),
            EditorError::Intercept(e) => write!(f, "intercept error: {e}"),
            EditorError::Parse(e) => write!(f, "parse error: {e}"),
        }
    }
}

impl From<TransformError> for EditorError {
    fn from(e: TransformError) -> Self {
        EditorError::Transform(e)
    }
}

impl From<InterceptError> for EditorError {
    fn from(e: InterceptError) -> Self {
        EditorError::Intercept(e)
    }
}

// ---------------------------------------------------------------------------
// EditorUpdate
// ---------------------------------------------------------------------------

/// The result of an editing operation, returned to the host platform.
pub struct EditorUpdate {
    pub render_elements: Vec<RenderElement>,
    pub selection: Selection,
    pub active_state: ActiveState,
    pub history_state: HistoryState,
}

/// Which marks and node types are active at the current selection.
pub struct ActiveState {
    pub marks: HashMap<String, bool>,
    pub mark_attrs: HashMap<String, serde_json::Value>,
    pub nodes: HashMap<String, bool>,
    pub commands: HashMap<String, bool>,
    /// Mark names that can be toggled at the current cursor position.
    pub allowed_marks: Vec<String>,
    /// Node type names that can be inserted at the current selection context.
    pub insertable_nodes: Vec<String>,
}

/// Whether undo/redo are available.
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone)]
struct SelectionPathRemap {
    target_item_path: Vec<u16>,
    selection: SelectionOffset,
}

#[derive(Clone)]
enum SelectionOffset {
    Text { anchor: u32, head: u32 },
    Node { pos: u32 },
}

// ---------------------------------------------------------------------------
// SplitAction
// ---------------------------------------------------------------------------

/// Action to take when split_block is called on an empty structured block.
enum SplitAction {
    /// Unwrap the list item to a paragraph (top-level list).
    UnwrapList(u32),
    /// Outdent the list item one level (nested list).
    OutdentList(u32),
    /// Exit the current empty paragraph out of its surrounding blockquote.
    ExitBlockquote(u32),
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

/// The main editor API. Owns schema, backend, selection, and interceptor pipeline.
pub struct Editor {
    schema: Schema,
    backend: StandaloneBackend,
    selection: Selection,
    stored_marks: Option<Vec<Mark>>,
    interceptors: InterceptorPipeline,
    allow_base64_images: bool,
}

impl Editor {
    /// Create a new editor with the given schema and interceptor pipeline.
    ///
    /// Starts with an empty document (single empty paragraph).
    pub fn new(schema: Schema, interceptors: InterceptorPipeline, allow_base64_images: bool) -> Self {
        let doc = make_empty_doc(&schema);
        let backend = StandaloneBackend::new(doc);
        Self {
            schema,
            backend,
            selection: Selection::cursor(1), // inside the empty paragraph
            stored_marks: None,
            interceptors,
            allow_base64_images,
        }
    }

    // -----------------------------------------------------------------------
    // Content: set/get
    // -----------------------------------------------------------------------

    /// Replace the document content from an HTML string.
    pub fn set_html(&mut self, html: &str) -> Result<Vec<RenderElement>, EditorError> {
        let doc = serialize::from_html(
            html,
            &self.schema,
            &serialize::FromHtmlOptions {
                strict: false,
                allow_base64_images: self.allow_base64_images,
            },
        )
        .map_err(|e| EditorError::Parse(e.to_string()))?;
        self.backend = StandaloneBackend::new(doc);
        self.selection = Selection::cursor(1);
        self.stored_marks = None;
        let elements = self.backend.to_render_elements(&self.schema);
        Ok(elements)
    }

    /// Replace the document content from a ProseMirror JSON value.
    pub fn set_json(
        &mut self,
        json: &serde_json::Value,
    ) -> Result<Vec<RenderElement>, EditorError> {
        let doc = serialize::from_prosemirror_json(
            json,
            &self.schema,
            serialize::UnknownTypeMode::Preserve,
        )
        .map_err(|e| EditorError::Parse(e.to_string()))?;
        self.backend = StandaloneBackend::new(doc);
        self.selection = Selection::cursor(1);
        self.stored_marks = None;
        let elements = self.backend.to_render_elements(&self.schema);
        Ok(elements)
    }

    /// Serialize the current document to HTML.
    pub fn get_html(&self) -> String {
        serialize::to_html(self.backend.document(), &self.schema)
    }

    /// Serialize the current document to ProseMirror JSON.
    pub fn get_json(&self) -> serde_json::Value {
        serialize::to_prosemirror_json(self.backend.document(), &self.schema)
    }

    // -----------------------------------------------------------------------
    // Editing operations
    // -----------------------------------------------------------------------

    /// Insert text at a document position.
    pub fn insert_text(&mut self, pos: u32, text: &str) -> Result<EditorUpdate, EditorError> {
        let marks = self.effective_marks_for_insert(pos);
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::InsertText {
            pos,
            text: text.to_string(),
            marks,
        });
        self.apply_transaction(tx)
    }

    /// Delete content between two document positions.
    pub fn delete_range(&mut self, from: u32, to: u32) -> Result<EditorUpdate, EditorError> {
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::DeleteRange { from, to });
        self.apply_transaction(tx)
    }

    /// Toggle a mark (bold, italic, etc.) on the current selection.
    ///
    /// If the selection is collapsed, toggle stored marks for subsequent
    /// insertions. If the selection is a range, add or remove the mark.
    pub fn toggle_mark(&mut self, mark_name: &str) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let from = self.selection.from(doc);
        let to = self.selection.to(doc);

        if from == to {
            let mut stored = self
                .stored_marks
                .clone()
                .unwrap_or_else(|| self.marks_at_pos(from));
            if let Some(idx) = stored.iter().position(|mark| mark.mark_type() == mark_name) {
                stored.remove(idx);
            } else {
                stored.push(Mark::new(mark_name.to_string(), HashMap::new()));
            }
            self.stored_marks = Some(stored);
            return Ok(self.build_update_from_current());
        }

        let mark = Mark::new(mark_name.to_string(), HashMap::new());
        let has_mark = self.range_has_mark(from, to, mark_name);

        let mut tx = Transaction::new(Source::Format);
        if has_mark {
            tx.add_step(Step::RemoveMark {
                from,
                to,
                mark_type: mark_name.to_string(),
            });
        } else {
            tx.add_step(Step::AddMark { from, to, mark });
        }
        match self.apply_transaction(tx) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Set a mark with explicit attrs on the current selection.
    pub fn set_mark(
        &mut self,
        mark_name: &str,
        attrs: HashMap<String, serde_json::Value>,
    ) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let from = self.selection.from(doc);
        let to = self.selection.to(doc);

        if from == to {
            if let Some((range_from, range_to)) = self.mark_range_at_pos(from, mark_name) {
                if range_from < range_to {
                    let mut tx = Transaction::new(Source::Format);
                    tx.add_step(Step::RemoveMark {
                        from: range_from,
                        to: range_to,
                        mark_type: mark_name.to_string(),
                    });
                    tx.add_step(Step::AddMark {
                        from: range_from,
                        to: range_to,
                        mark: Mark::new(mark_name.to_string(), attrs),
                    });
                    return match self.apply_transaction(tx) {
                        Ok(update) => Ok(update),
                        Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
                        Err(e) => Err(e),
                    };
                }
            }

            let mut stored = self
                .stored_marks
                .clone()
                .unwrap_or_else(|| self.marks_at_pos(from));
            stored.retain(|mark| mark.mark_type() != mark_name);
            stored.push(Mark::new(mark_name.to_string(), attrs));
            self.stored_marks = Some(stored);
            return Ok(self.build_update_from_current());
        }

        let mut tx = Transaction::new(Source::Format);
        tx.add_step(Step::RemoveMark {
            from,
            to,
            mark_type: mark_name.to_string(),
        });
        tx.add_step(Step::AddMark {
            from,
            to,
            mark: Mark::new(mark_name.to_string(), attrs),
        });
        match self.apply_transaction(tx) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Remove a mark from the current selection.
    pub fn unset_mark(&mut self, mark_name: &str) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let from = self.selection.from(doc);
        let to = self.selection.to(doc);

        if from == to {
            if let Some((range_from, range_to)) = self.mark_range_at_pos(from, mark_name) {
                if range_from < range_to {
                    let mut tx = Transaction::new(Source::Format);
                    tx.add_step(Step::RemoveMark {
                        from: range_from,
                        to: range_to,
                        mark_type: mark_name.to_string(),
                    });
                    return match self.apply_transaction(tx) {
                        Ok(update) => Ok(update),
                        Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
                        Err(e) => Err(e),
                    };
                }
            }

            let mut stored = self
                .stored_marks
                .clone()
                .unwrap_or_else(|| self.marks_at_pos(from));
            stored.retain(|mark| mark.mark_type() != mark_name);
            self.stored_marks = Some(stored);
            return Ok(self.build_update_from_current());
        }

        let mut tx = Transaction::new(Source::Format);
        tx.add_step(Step::RemoveMark {
            from,
            to,
            mark_type: mark_name.to_string(),
        });
        match self.apply_transaction(tx) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Split a block at the given position (e.g. pressing Enter).
    pub fn split_block(&mut self, pos: u32) -> Result<EditorUpdate, EditorError> {
        if let Some(action) = self.empty_split_action(pos) {
            return self.apply_empty_split_action(action);
        }

        // Normal split: create a new paragraph block.
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::SplitBlock {
            pos,
            node_type: "paragraph".to_string(),
            attrs: HashMap::new(),
        });
        self.apply_transaction(tx)
    }

    /// Join two adjacent blocks at the given boundary position.
    pub fn join_blocks(&mut self, pos: u32) -> Result<EditorUpdate, EditorError> {
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::JoinBlocks { pos });
        self.apply_transaction(tx)
    }

    /// Wrap a range of blocks in a list.
    pub fn wrap_in_list(
        &mut self,
        from: u32,
        to: u32,
        list_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        // Determine the list item type from the schema.
        let item_type = self
            .schema
            .all_nodes()
            .find(|n| matches!(n.role, crate::schema::NodeRole::ListItem))
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "listItem".to_string());

        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::WrapInList {
            from,
            to,
            list_type: list_type.to_string(),
            item_type,
            attrs: HashMap::new(),
        });
        match self.apply_transaction(tx) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Apply a list type to the current selection.
    ///
    /// When the selection is already inside a different list type, this
    /// converts the containing list node in-place so the whole list changes
    /// type atomically.
    pub fn apply_list_type(&mut self, list_type: &str) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);

        if let Some((list_start, list_node)) = self.containing_list_node_at(pos) {
            if list_node.node_type() != list_type {
                let replacement = Node::element(
                    list_type.to_string(),
                    list_attrs_for_type(list_type, list_node.attrs()),
                    list_node.content().cloned().unwrap_or_else(Fragment::empty),
                );
                let mut tx = Transaction::new(Source::Format);
                tx.add_step(Step::ReplaceRange {
                    from: list_start,
                    to: list_start + list_node.node_size(),
                    content: Fragment::from(vec![replacement]),
                });
                return match self.apply_transaction(tx) {
                    Ok(update) => Ok(update),
                    Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
                    Err(e) => Err(e),
                };
            }
        }

        let from = self.selection.from(doc);
        let to = self.selection.to(doc);
        if let Some(update) = self.wrap_selected_blocks_in_list(from, to, list_type)? {
            return Ok(update);
        }
        self.wrap_in_list(from, to, list_type)
    }

    /// Toggle a blockquote around the current block selection.
    ///
    /// If the current selection is inside a blockquote, unwraps the nearest
    /// containing blockquote. Otherwise wraps the selected sibling block range
    /// in a new blockquote container.
    pub fn toggle_blockquote(&mut self) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);

        if self.containing_blockquote_node_at(pos).is_some() {
            return self.unwrap_from_blockquote(pos);
        }

        let Some(blockquote_type) = self.blockquote_node_name() else {
            return Ok(self.build_update_from_current());
        };
        let from = self.selection.from(doc);
        let to = self.selection.to(doc);
        self.wrap_in_blockquote(from, to, &blockquote_type)
    }

    /// Toggle a blockquote at an explicit scalar selection supplied by the caller.
    pub fn toggle_blockquote_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.toggle_blockquote()
    }

    /// Wrap the selected sibling block range in a blockquote container.
    pub fn wrap_in_blockquote(
        &mut self,
        from: u32,
        to: u32,
        blockquote_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        let Some(range) = self.selected_block_range(from, to) else {
            return Ok(self.build_update_from_current());
        };

        let parent = if range.parent_path.is_empty() {
            self.backend.document().root()
        } else {
            self.backend.document().node_at(&range.parent_path)
                .unwrap_or_else(|| self.backend.document().root())
        };
        let Some(parent_spec) = self.schema.node(parent.node_type()) else {
            return Ok(self.build_update_from_current());
        };
        let insertable = self.schema.insertable_nodes_at(parent_spec, parent.child_count());
        if !insertable.iter().any(|name| name == blockquote_type) {
            return Ok(self.build_update_from_current());
        }

        let quote = Node::element(
            blockquote_type.to_string(),
            HashMap::new(),
            Fragment::from(range.selected_blocks),
        );
        let mut tx = Transaction::new(Source::Format);
        tx.add_step(Step::ReplaceRange {
            from: range.replace_from,
            to: range.replace_to,
            content: Fragment::from(vec![quote]),
        });

        let selection_after = self.shifted_selection(1);
        match self.apply_transaction_with_selection_adjustments(tx, None, selection_after) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Unwrap the nearest containing blockquote at the given position.
    pub fn unwrap_from_blockquote(&mut self, pos: u32) -> Result<EditorUpdate, EditorError> {
        let Some((quote_start, _quote_path, quote_node)) = self.containing_blockquote_node_at(pos)
        else {
            return Ok(self.build_update_from_current());
        };
        let Some(content) = quote_node.content() else {
            return Ok(self.build_update_from_current());
        };

        let mut tx = Transaction::new(Source::Format);
        tx.add_step(Step::ReplaceRange {
            from: quote_start,
            to: quote_start + quote_node.node_size(),
            content: Fragment::from(content.iter().cloned().collect::<Vec<_>>()),
        });

        let selection_after = self.shifted_selection(-1);
        match self.apply_transaction_with_selection_adjustments(tx, None, selection_after) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Unwrap a list item at the given position.
    pub fn unwrap_from_list(&mut self, pos: u32) -> Result<EditorUpdate, EditorError> {
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::UnwrapFromList { pos });
        match self.apply_transaction(tx) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Increase the nesting level of the current list item.
    pub fn indent_list_item(&mut self) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::IndentListItem { pos });
        self.apply_transaction(tx)
    }

    /// Decrease the nesting level of the current list item.
    pub fn outdent_list_item(&mut self) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);
        let selection_remap = self.selection_remap_for_outdent(pos);
        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::OutdentListItem { pos });
        self.apply_transaction_with_selection_remap(tx, selection_remap)
    }

    /// Insert a node at a position.
    ///
    /// Block nodes (for example horizontal rules) are resolved to the nearest
    /// block-level insertion point. Inline void nodes (for example hard
    /// breaks) are inserted directly at `pos`.
    pub fn insert_node(&mut self, pos: u32, node_type: &str) -> Result<EditorUpdate, EditorError> {
        if self.is_inline_insertable_node(node_type) {
            return self.insert_inline_node(pos, node_type);
        }

        let insert_pos = self.resolve_block_insert_pos(pos);
        if !self.can_insert_block_node_at_pos(pos, node_type) {
            return Ok(self.build_update_from_current());
        }
        let node = Node::void(node_type.to_string(), HashMap::new());
        let mut tx = Transaction::new(Source::Input);
        let selection_after = if Self::is_horizontal_rule_node(node_type) {
            let replace_range = self.empty_text_block_replace_range_at(pos);
            let replace_from = replace_range.map(|(from, _)| from).unwrap_or(insert_pos);
            let replace_to = replace_range.map(|(_, to)| to).unwrap_or(insert_pos);
            tx.add_step(Step::ReplaceRange {
                from: replace_from,
                to: replace_to,
                content: Fragment::from(vec![node, Self::empty_paragraph_node()]),
            });
            Some(Selection::cursor(replace_from.saturating_add(2)))
        } else {
            tx.add_step(Step::InsertNode {
                pos: insert_pos,
                node,
            });
            Some(Selection::cursor(insert_pos.saturating_add(1)))
        };
        match self.apply_transaction_with_selection_adjustments(tx, None, selection_after) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    /// Insert a node at the current selection.
    ///
    /// Inline nodes replace the current selection atomically. Block nodes keep
    /// the existing insertion semantics and use the selection anchor.
    pub fn insert_node_at_selection(
        &mut self,
        node_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let from = self.selection.from(doc);
        let to = self.selection.to(doc);

        if self.is_inline_insertable_node(node_type) {
            return self.replace_selection_with_inline_node(from, to, node_type);
        }

        self.insert_node(from, node_type)
    }

    pub fn resize_image_at_doc_pos(
        &mut self,
        doc_pos: u32,
        width: u32,
        height: u32,
    ) -> Result<EditorUpdate, EditorError> {
        if width == 0 || height == 0 {
            return Ok(self.build_update_from_current());
        }

        let Some((image_pos, mut attrs)) = self.image_node_at_doc_pos(doc_pos) else {
            return Ok(self.build_update_from_current());
        };

        let width_value = serde_json::Value::Number(serde_json::Number::from(width));
        let height_value = serde_json::Value::Number(serde_json::Number::from(height));
        if attrs.get("width") == Some(&width_value) && attrs.get("height") == Some(&height_value) {
            return Ok(self.build_update_from_current());
        }

        attrs.insert("width".to_string(), width_value);
        attrs.insert("height".to_string(), height_value);

        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::UpdateNodeAttrs {
            pos: image_pos,
            attrs,
        });
        self.apply_transaction_with_selection_adjustments(
            tx,
            None,
            Some(Selection::node(image_pos)),
        )
    }

    /// Insert HTML content at the current selection position.
    pub fn insert_content_html(&mut self, html: &str) -> Result<EditorUpdate, EditorError> {
        let parsed_doc =
            serialize::from_html(
                html,
                &self.schema,
                &serialize::FromHtmlOptions {
                    strict: false,
                    allow_base64_images: self.allow_base64_images,
                },
            )
                .map_err(|e| EditorError::Parse(e.to_string()))?;

        let content = match parsed_doc.root().content() {
            Some(c) => c.clone(),
            None => return Ok(self.build_update_from_current()),
        };

        let doc = self.backend.document();
        let from = self.selection.from(doc);

        if self.is_block_void_fragment(&content) {
            return self.insert_block_void_fragment_at_selection(from, &content, Source::Paste);
        }

        let to = self.selection.to(doc);

        let mut tx = Transaction::new(Source::Paste);
        tx.add_step(Step::ReplaceRange { from, to, content });
        self.apply_transaction(tx)
    }

    /// Insert JSON content at the current selection position.
    pub fn insert_content_json(
        &mut self,
        json: &serde_json::Value,
    ) -> Result<EditorUpdate, EditorError> {
        let parsed_doc = serialize::from_prosemirror_json(
            json,
            &self.schema,
            serialize::UnknownTypeMode::Preserve,
        )
        .map_err(|e| EditorError::Parse(e.to_string()))?;

        let content = match parsed_doc.root().content() {
            Some(c) => c.clone(),
            None => return Ok(self.build_update_from_current()),
        };

        let doc = self.backend.document();
        let from = self.selection.from(doc);

        if self.is_block_void_fragment(&content) {
            return self.insert_block_void_fragment_at_selection(from, &content, Source::Api);
        }

        let to = self.selection.to(doc);

        let mut tx = Transaction::new(Source::Api);
        tx.add_step(Step::ReplaceRange { from, to, content });
        self.apply_transaction(tx)
    }

    /// Replace the entire document content with HTML via a transaction.
    ///
    /// Unlike `set_html()` which resets the backend (dropping undo history),
    /// this goes through the transaction pipeline so it can be undone and
    /// preserves the selection where possible.
    pub fn replace_html(&mut self, html: &str) -> Result<EditorUpdate, EditorError> {
        let parsed_doc =
            serialize::from_html(
                html,
                &self.schema,
                &serialize::FromHtmlOptions {
                    strict: false,
                    allow_base64_images: self.allow_base64_images,
                },
            )
                .map_err(|e| EditorError::Parse(e.to_string()))?;

        let content = match parsed_doc.root().content() {
            Some(c) => c.clone(),
            None => return Ok(self.build_update_from_current()),
        };

        let doc = self.backend.document();
        let doc_content_size = doc.content_size();
        self.stored_marks = None;

        let mut tx = Transaction::new(Source::Api);
        tx.add_step(Step::ReplaceRange {
            from: 0,
            to: doc_content_size,
            content,
        });
        self.apply_transaction(tx)
    }

    /// Replace the entire document content with JSON.
    pub fn replace_json(&mut self, json: &serde_json::Value) -> Result<EditorUpdate, EditorError> {
        // This is equivalent to set_json but goes through the transaction
        // pipeline so it can be undone.
        let parsed_doc = serialize::from_prosemirror_json(
            json,
            &self.schema,
            serialize::UnknownTypeMode::Preserve,
        )
        .map_err(|e| EditorError::Parse(e.to_string()))?;

        let content = match parsed_doc.root().content() {
            Some(c) => c.clone(),
            None => return Ok(self.build_update_from_current()),
        };

        let doc = self.backend.document();
        let doc_content_size = doc.content_size();
        self.stored_marks = None;

        let mut tx = Transaction::new(Source::Api);
        tx.add_step(Step::ReplaceRange {
            from: 0,
            to: doc_content_size,
            content,
        });
        self.apply_transaction(tx)
    }

    // -----------------------------------------------------------------------
    // Selection
    // -----------------------------------------------------------------------

    /// Set the current selection, normalizing positions to cursorable locations.
    pub fn set_selection(&mut self, selection: Selection) {
        let doc = self.backend.document();
        let pos_map = self.backend.position_map();
        self.selection = selection.normalized(doc, pos_map);
        self.stored_marks = None;
    }

    /// Get a reference to the current selection.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Get the names of marks active at the current selection.
    pub fn active_marks(&self) -> Vec<String> {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);
        let marks = self.effective_marks_for_selection(pos, self.selection.is_empty(doc));
        marks.iter().map(|m| m.mark_type().to_string()).collect()
    }

    /// Get the names of node types in the current selection's path.
    pub fn active_nodes(&self) -> Vec<String> {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);
        self.nodes_at_pos(pos)
    }

    // -----------------------------------------------------------------------
    // History
    // -----------------------------------------------------------------------

    /// Undo the last edit.
    pub fn undo(&mut self) -> Option<EditorUpdate> {
        let state = self.backend.undo(&self.schema)?;
        let render_elements = state.render_elements;
        if let Some(sel) = state.selection_update {
            self.selection = sel;
        }
        self.stored_marks = None;
        Some(self.build_update(render_elements))
    }

    /// Redo the last undone edit.
    pub fn redo(&mut self) -> Option<EditorUpdate> {
        let state = self.backend.redo(&self.schema)?;
        let render_elements = state.render_elements;
        if let Some(sel) = state.selection_update {
            self.selection = sel;
        }
        self.stored_marks = None;
        Some(self.build_update(render_elements))
    }

    /// Whether undo is available.
    pub fn can_undo(&self) -> bool {
        self.backend.can_undo()
    }

    /// Whether redo is available.
    pub fn can_redo(&self) -> bool {
        self.backend.can_redo()
    }

    /// Get the current full state as an EditorUpdate (render elements, selection,
    /// active state, history state). Used by native views to pull initial state
    /// when binding to an editor that already has content loaded via the bridge.
    pub fn get_current_state(&self) -> EditorUpdate {
        self.build_update_from_current()
    }

    // -----------------------------------------------------------------------
    // Scalar-position APIs (for native views)
    // -----------------------------------------------------------------------
    //
    // Native text views (UITextView, EditText) work in scalar offsets — the
    // offset into the rendered text measured in Unicode scalars. The Rust
    // document model uses *document positions* which include structural tokens
    // (node open/close). These methods accept scalar offsets and convert to
    // document positions internally before delegating to the doc-position APIs.

    /// Insert text at a scalar offset.
    pub fn insert_text_scalar(
        &mut self,
        scalar_pos: u32,
        text: &str,
    ) -> Result<EditorUpdate, EditorError> {
        let doc_pos = self.scalar_to_doc(scalar_pos);
        self.insert_text(doc_pos, text)
    }

    /// Delete content between two scalar offsets.
    pub fn delete_scalar_range(
        &mut self,
        scalar_from: u32,
        scalar_to: u32,
    ) -> Result<EditorUpdate, EditorError> {
        let doc_from = self.scalar_to_doc(scalar_from);
        let doc_to = self.scalar_to_doc(scalar_to);
        if let Some(list_pos) =
            self.empty_list_unwrap_pos_for_scalar_delete(scalar_from, scalar_to, doc_from, doc_to)
        {
            return self.unwrap_from_list(list_pos);
        }
        if let Some(quote_pos) =
            self.empty_blockquote_exit_pos_for_scalar_delete(scalar_from, scalar_to, doc_from, doc_to)
        {
            return self.exit_empty_blockquote(quote_pos);
        }
        if let Some((replace_from, replace_to, content, selection_after)) = self
            .lift_empty_text_block_out_of_list_for_scalar_delete(
                scalar_from,
                scalar_to,
                doc_from,
                doc_to,
            )
        {
            let mut tx = Transaction::new(Source::Input);
            tx.add_step(Step::ReplaceRange {
                from: replace_from,
                to: replace_to,
                content: Fragment::from(content),
            });
            return self.apply_transaction_with_selection_adjustments(
                tx,
                None,
                Some(selection_after),
            );
        }
        if let Some((replace_from, replace_to, selection_after)) = self
            .block_void_replacement_for_scalar_delete(
                scalar_from,
                scalar_to,
                doc_from,
                doc_to,
            )
        {
            let mut tx = Transaction::new(Source::Input);
            tx.add_step(Step::ReplaceRange {
                from: replace_from,
                to: replace_to,
                content: Fragment::from(vec![Self::empty_paragraph_node()]),
            });
            return self.apply_transaction_with_selection_adjustments(
                tx,
                None,
                Some(selection_after),
            );
        }
        if let Some((block_from, block_to)) = self.empty_text_block_delete_range_for_scalar_delete(
            scalar_from,
            scalar_to,
            doc_from,
            doc_to,
        ) {
            return self.delete_range(block_from, block_to);
        }
        self.delete_range(doc_from, doc_to)
    }

    /// Replace a scalar range with text in a single transaction.
    ///
    /// Used when the user types with a range selection — the selection is
    /// deleted and the new text is inserted atomically so undo reverses both.
    pub fn replace_text_scalar(
        &mut self,
        scalar_from: u32,
        scalar_to: u32,
        text: &str,
    ) -> Result<EditorUpdate, EditorError> {
        let doc_from = self.scalar_to_doc(scalar_from);
        let doc_to = self.scalar_to_doc(scalar_to);
        let marks = self.effective_marks_for_insert(doc_from);
        let mut tx = Transaction::new(Source::Input);
        if doc_from < doc_to {
            tx.add_step(Step::DeleteRange {
                from: doc_from,
                to: doc_to,
            });
        }
        if !text.is_empty() {
            tx.add_step(Step::InsertText {
                pos: doc_from,
                text: text.to_string(),
                marks,
            });
        }
        self.apply_transaction(tx)
    }

    /// Split a block at a scalar offset.
    pub fn split_block_scalar(&mut self, scalar_pos: u32) -> Result<EditorUpdate, EditorError> {
        let doc_pos = self.scalar_to_doc(scalar_pos);
        self.split_block(doc_pos)
    }

    /// Delete a scalar range then split the block, in a single transaction.
    ///
    /// Used when the user presses Enter with a range selection.
    /// If the delete leaves an empty list item, unwraps/outdents instead of splitting.
    pub fn delete_and_split_scalar(
        &mut self,
        scalar_from: u32,
        scalar_to: u32,
    ) -> Result<EditorUpdate, EditorError> {
        let doc_from = self.scalar_to_doc(scalar_from);
        let doc_to = self.scalar_to_doc(scalar_to);

        // Apply the delete as a separate transaction first.
        if doc_from < doc_to {
            let mut delete_tx = Transaction::new(Source::Input);
            delete_tx.add_step(Step::DeleteRange {
                from: doc_from,
                to: doc_to,
            });
            self.apply_transaction(delete_tx)?;
        }

        // After the delete, check if we're now in an empty structured block.
        // doc_from is still valid because DeleteRange remaps positions before
        // the range to themselves.
        if let Some(action) = self.empty_split_action(doc_from) {
            return self.apply_empty_split_action(action);
        }

        // Normal split.
        self.split_block(doc_from)
    }

    /// Set the selection from scalar offsets, converting to document positions.
    pub fn set_selection_scalar(&mut self, scalar_anchor: u32, scalar_head: u32) {
        let doc_anchor = self.scalar_to_doc(scalar_anchor);
        let doc_head = self.scalar_to_doc(scalar_head);
        let sel = if doc_anchor == doc_head {
            Selection::cursor(doc_anchor)
        } else {
            Selection::text(doc_anchor, doc_head)
        };
        self.set_selection(sel);
    }

    /// Toggle a mark at an explicit scalar selection supplied by the caller.
    pub fn toggle_mark_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
        mark_name: &str,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.toggle_mark(mark_name)
    }

    /// Set a mark with attrs at an explicit scalar selection supplied by the caller.
    pub fn set_mark_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
        mark_name: &str,
        attrs: HashMap<String, serde_json::Value>,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.set_mark(mark_name, attrs)
    }

    /// Remove a mark at an explicit scalar selection supplied by the caller.
    pub fn unset_mark_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
        mark_name: &str,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.unset_mark(mark_name)
    }

    /// Apply a list type at an explicit scalar selection supplied by the caller.
    pub fn apply_list_type_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
        list_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.apply_list_type(list_type)
    }

    /// Unwrap the list item at an explicit scalar selection supplied by the caller.
    pub fn unwrap_from_list_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        let doc = self.backend.document();
        let pos = self.selection.from(doc);
        self.unwrap_from_list(pos)
    }

    /// Indent the list item at an explicit scalar selection supplied by the caller.
    pub fn indent_list_item_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.indent_list_item()
    }

    /// Outdent the list item at an explicit scalar selection supplied by the caller.
    pub fn outdent_list_item_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.outdent_list_item()
    }

    /// Insert a node at an explicit scalar selection supplied by the caller.
    pub fn insert_node_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
        node_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.insert_node_at_selection(node_type)
    }

    /// Insert JSON content at an explicit scalar selection supplied by the caller.
    pub fn insert_content_json_at_selection_scalar(
        &mut self,
        scalar_anchor: u32,
        scalar_head: u32,
        json: &serde_json::Value,
    ) -> Result<EditorUpdate, EditorError> {
        self.set_selection_scalar(scalar_anchor, scalar_head);
        self.insert_content_json(json)
    }

    /// Replace the current selection with plain text in a single transaction.
    pub fn replace_selection_text(&mut self, text: &str) -> Result<EditorUpdate, EditorError> {
        let doc = self.backend.document();
        let from = self.selection.from(doc);
        let to = self.selection.to(doc);
        let marks = self.effective_marks_for_insert(from);
        let mut tx = Transaction::new(Source::Input);
        if from < to {
            tx.add_step(Step::DeleteRange { from, to });
        }
        if !text.is_empty() {
            tx.add_step(Step::InsertText {
                pos: from,
                text: text.to_string(),
                marks,
            });
        }
        self.apply_transaction(tx)
    }

    // -----------------------------------------------------------------------
    // Position mapping
    // -----------------------------------------------------------------------

    /// Convert a rendered-text scalar offset to a document position.
    pub fn scalar_to_doc(&self, scalar: u32) -> u32 {
        self.backend
            .position_map()
            .scalar_to_doc(scalar, self.backend.document())
    }

    /// Convert a document position to a rendered-text scalar offset.
    pub fn doc_to_scalar(&self, pos: u32) -> u32 {
        self.backend
            .position_map()
            .doc_to_scalar(pos, self.backend.document())
    }

    /// Normalize a document position to the nearest cursorable position.
    pub fn normalize_pos(&self, pos: u32) -> u32 {
        self.backend
            .position_map()
            .normalize_cursor_pos(pos, self.backend.document())
    }

    // -----------------------------------------------------------------------
    // Access to internals (for testing and advanced use)
    // -----------------------------------------------------------------------

    /// Reference to the schema.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Reference to the current document.
    pub fn document(&self) -> &Document {
        self.backend.document()
    }

    /// Reference to the current position map.
    pub fn position_map(&self) -> &PositionMap {
        self.backend.position_map()
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Apply a transaction through the interceptor pipeline and backend.
    fn apply_transaction(&mut self, tx: Transaction) -> Result<EditorUpdate, EditorError> {
        self.apply_transaction_with_selection_adjustments(tx, None, None)
    }

    fn apply_transaction_with_selection_remap(
        &mut self,
        tx: Transaction,
        selection_remap: Option<SelectionPathRemap>,
    ) -> Result<EditorUpdate, EditorError> {
        self.apply_transaction_with_selection_adjustments(tx, selection_remap, None)
    }

    fn apply_transaction_with_selection_adjustments(
        &mut self,
        tx: Transaction,
        selection_remap: Option<SelectionPathRemap>,
        selection_override: Option<Selection>,
    ) -> Result<EditorUpdate, EditorError> {
        // Clone the document for interceptor checks and step map computation.
        // This avoids holding an immutable borrow of self.backend across a
        // mutable call.
        let doc_before = self.backend.document().clone();

        // Run interceptors.
        let tx = self.interceptors.run(tx, &doc_before)?;

        // Compute the step map for selection mapping before mutating backend.
        let (preview_doc, step_map) = tx.apply(&doc_before, &self.schema)?;

        // Save selection before for history.
        let selection_before = self.selection.clone();
        let preserve_stored_marks =
            selection_before.is_empty(&doc_before) && self.stored_marks.is_some();

        // Map the selection through the step map.
        let new_selection = self.selection.map(&step_map);
        let pos_map_temp = self.backend.position_map();
        let selection_after = selection_override
            .clone()
            .map(|selection| selection.normalized(&preview_doc, pos_map_temp))
            .or_else(|| {
                selection_remap
                    .as_ref()
                    .and_then(|remap| remap.resolve(&preview_doc))
            })
            .unwrap_or_else(|| new_selection.normalized(&doc_before, pos_map_temp));

        // Apply to backend (mutates document, position map, history).
        let state = self.backend.apply_transaction(
            &tx,
            &self.schema,
            &selection_before,
            &selection_after,
        )?;

        // Set the selection to the mapped/normalized version using the
        // post-apply position map and document.
        let new_selection = self.selection.map(&step_map);
        let pos_map = self.backend.position_map();
        self.selection = selection_override
            .map(|selection| selection.normalized(self.backend.document(), pos_map))
            .or_else(|| selection_remap.and_then(|remap| remap.resolve(self.backend.document())))
            .unwrap_or_else(|| new_selection.normalized(self.backend.document(), pos_map));
        if !(preserve_stored_marks && self.selection.is_empty(self.backend.document())) {
            self.stored_marks = None;
        }

        Ok(self.build_update(state.render_elements))
    }

    /// Build an EditorUpdate from render elements and current state.
    fn build_update(&self, render_elements: Vec<RenderElement>) -> EditorUpdate {
        EditorUpdate {
            render_elements,
            selection: self.selection.clone(),
            active_state: self.compute_active_state(),
            history_state: HistoryState {
                can_undo: self.backend.can_undo(),
                can_redo: self.backend.can_redo(),
            },
        }
    }

    /// Build an EditorUpdate from the current document state (no changes).
    fn build_update_from_current(&self) -> EditorUpdate {
        let render_elements = self.backend.to_render_elements(&self.schema);
        self.build_update(render_elements)
    }

    /// Compute which marks and nodes are active at the current selection.
    fn compute_active_state(&self) -> ActiveState {
        let doc = self.backend.document();
        let pos = self.selection.from(doc);
        let marks_at = self.effective_marks_for_selection(pos, self.selection.is_empty(doc));
        let nodes_at = self.nodes_at_pos(pos);

        let mut marks = HashMap::new();
        let mut mark_attrs = HashMap::new();
        for mark_spec in self.schema.all_marks() {
            let active_mark = marks_at.iter().find(|m| m.mark_type() == mark_spec.name);
            let is_active = active_mark.is_some();
            marks.insert(mark_spec.name.clone(), is_active);
            if let Some(mark) = active_mark {
                if !mark.attrs().is_empty() {
                    mark_attrs.insert(
                        mark_spec.name.clone(),
                        serde_json::Value::Object(
                            mark.attrs().clone().into_iter().collect()
                        ),
                    );
                }
            }
        }

        let active_list_type = self
            .containing_list_node_at(pos)
            .map(|(_, node)| node.node_type().to_string());
        let mut nodes = HashMap::new();
        for node_name in &nodes_at {
            let is_list_node = self
                .schema
                .node(node_name)
                .map(|spec| matches!(spec.role, NodeRole::List { .. }))
                .unwrap_or(false);
            if is_list_node {
                if active_list_type.as_deref() == Some(node_name.as_str()) {
                    nodes.insert(node_name.clone(), true);
                }
                continue;
            }
            nodes.insert(node_name.clone(), true);
        }

        let mut commands = HashMap::new();
        commands.insert("indentList".to_string(), self.can_indent_list_item(pos));
        commands.insert("outdentList".to_string(), self.can_outdent_list_item(pos));
        commands.insert(
            "toggleBlockquote".to_string(),
            self.can_toggle_blockquote_at(pos),
        );

        // Compute allowed_marks and insertable_nodes based on selection type.
        let (allowed_marks, insertable_nodes) = match &self.selection {
            Selection::All => {
                // All-selection: no meaningful cursor context for marks or insertion.
                (Vec::new(), Vec::new())
            }
            Selection::Node { .. } => {
                // Node selection (e.g. on a void node): no text cursor for marks,
                // but we can still compute insertable nodes at this position.
                let resolved = doc.resolve(pos).ok();
                let insertable = resolved
                    .map(|r| self.insertable_nodes_from_resolved(doc, &r))
                    .unwrap_or_default();
                (Vec::new(), insertable)
            }
            Selection::Text { .. } => {
                // Text selection: compute both fields.
                let active_mark_names: Vec<&str> = marks_at.iter().map(|m| m.mark_type()).collect();

                let resolved = doc.resolve(pos).ok();

                let allowed = resolved
                    .as_ref()
                    .and_then(|r| {
                        let parent = r.parent(doc);
                        self.schema
                            .node(parent.node_type())
                            .map(|spec| self.schema.allowed_marks_at(spec, &active_mark_names))
                    })
                    .unwrap_or_default();

                let insertable = resolved
                    .map(|r| self.insertable_nodes_from_resolved(doc, &r))
                    .unwrap_or_default();

                (allowed, insertable)
            }
        };

        // List wrap commands: true if already in a list (toggle/switch) or if
        // the parent context allows list nodes.
        commands.insert("wrapBulletList".to_string(), self.can_wrap_in_list_at(pos));
        commands.insert("wrapOrderedList".to_string(), self.can_wrap_in_list_at(pos));

        ActiveState {
            marks,
            mark_attrs,
            nodes,
            commands,
            allowed_marks,
            insertable_nodes,
        }
    }

    /// Compute insertable nodes for the current selection context.
    ///
    /// Block nodes are derived from the nearest non-text-block ancestor.
    /// Inline void nodes are derived from the current text block.
    fn insertable_nodes_from_resolved(
        &self,
        doc: &Document,
        resolved: &ResolvedPos,
    ) -> Vec<String> {
        let mut insertable = self.block_insertable_nodes_from_resolved(doc, resolved);
        for node_type in self.inline_insertable_nodes_from_resolved(doc, resolved) {
            if !insertable.contains(&node_type) {
                insertable.push(node_type);
            }
        }
        insertable
    }

    /// Walk up from the resolved position past TextBlock nodes to find the
    /// block-level parent, then compute insertable block nodes at that level.
    fn block_insertable_nodes_from_resolved(
        &self,
        doc: &Document,
        resolved: &ResolvedPos,
    ) -> Vec<String> {
        // Walk the node path to find the deepest non-TextBlock ancestor.
        // The resolved position's parent is the innermost node; if it's a
        // TextBlock (e.g. paragraph), we go one level up.
        let path = &resolved.node_path;

        // Build the chain of nodes from root to the parent at resolved pos.
        let mut nodes_in_path: Vec<&Node> = vec![doc.root()];
        let mut current = doc.root();
        for &idx in path.iter() {
            if let Some(child) = current.child(idx as usize) {
                nodes_in_path.push(child);
                current = child;
            }
        }

        // Walk backwards to find the first non-TextBlock node.
        // That's the block-level parent where we'd insert new block nodes.
        for i in (0..nodes_in_path.len()).rev() {
            let node = nodes_in_path[i];
            if let Some(spec) = self.schema.node(node.node_type()) {
                if matches!(spec.role, NodeRole::TextBlock) {
                    continue;
                }
                // Found the block-level parent. Count its existing children
                // to determine the content rule slot.
                let insertable = self.schema.insertable_nodes_at(spec, node.child_count());
                return self.filter_insertable_nodes_for_parent(node, insertable);
            }
        }

        Vec::new()
    }

    fn inline_insertable_nodes_from_resolved(
        &self,
        doc: &Document,
        resolved: &ResolvedPos,
    ) -> Vec<String> {
        let parent = resolved.parent(doc);
        let Some(parent_spec) = self.schema.node(parent.node_type()) else {
            return Vec::new();
        };
        if !matches!(parent_spec.role, NodeRole::TextBlock) {
            return Vec::new();
        }

        self.schema
            .all_nodes()
            .filter(|spec| {
                matches!(spec.role, NodeRole::HardBreak | NodeRole::Inline) && spec.is_void
            })
            .map(|spec| spec.name.clone())
            .collect()
    }

    fn insertable_nodes_at_pos(&self, pos: u32) -> Vec<String> {
        let doc = self.backend.document();
        doc.resolve(pos)
            .ok()
            .map(|resolved| self.insertable_nodes_from_resolved(doc, &resolved))
            .unwrap_or_default()
    }

    fn filter_insertable_nodes_for_parent(
        &self,
        parent: &Node,
        insertable: Vec<String>,
    ) -> Vec<String> {
        let mut filtered: Vec<String> = insertable
            .into_iter()
            .filter(|node_type| {
                self.schema
                    .node(node_type)
                    .map(|spec| matches!(spec.role, NodeRole::Block) && spec.is_void)
                    .unwrap_or(false)
            })
            .collect();

        if parent.node_type() == "listItem" || parent.node_type() == "list_item" {
            filtered.retain(|node_type| !Self::is_horizontal_rule_node(node_type));
        }

        filtered
    }

    fn can_insert_block_node_at_pos(&self, pos: u32, node_type: &str) -> bool {
        self.insertable_nodes_at_pos(pos)
            .iter()
            .any(|insertable| insertable == node_type)
    }

    fn is_block_void_fragment(&self, content: &Fragment) -> bool {
        content.size() > 0
            && content.iter().all(|node| {
                self.schema
                    .node(node.node_type())
                    .map(|spec| matches!(spec.role, NodeRole::Block) && spec.is_void)
                    .unwrap_or(false)
            })
    }

    fn image_node_at_doc_pos(
        &self,
        doc_pos: u32,
    ) -> Option<(u32, HashMap<String, serde_json::Value>)> {
        let block_index = self.position_map().find_block_for_doc_pos(doc_pos)?;
        let block = self.position_map().block(block_index)?;
        if !block.is_void_block {
            return None;
        }

        let node = self.document().node_at(&block.node_path)?;
        if node.node_type() != "image" {
            return None;
        }

        Some((block.doc_start, node.attrs().clone()))
    }

    fn insert_block_void_fragment_at_selection(
        &mut self,
        pos: u32,
        content: &Fragment,
        source: Source,
    ) -> Result<EditorUpdate, EditorError> {
        if !content
            .iter()
            .all(|node| self.can_insert_block_node_at_pos(pos, node.node_type()))
        {
            return Ok(self.build_update_from_current());
        }

        let insert_pos = self.resolve_block_insert_pos(pos);
        let replace_range = self.empty_text_block_replace_range_at(pos);
        let replace_from = replace_range.map(|(from, _)| from).unwrap_or(insert_pos);
        let replace_to = replace_range.map(|(_, to)| to).unwrap_or(insert_pos);
        let inserted_size = content.size();

        let mut replacement_nodes: Vec<Node> = content.iter().cloned().collect();
        replacement_nodes.push(Self::empty_paragraph_node());

        let mut tx = Transaction::new(source);
        tx.add_step(Step::ReplaceRange {
            from: replace_from,
            to: replace_to,
            content: Fragment::from(replacement_nodes),
        });

        self.apply_transaction_with_selection_adjustments(
            tx,
            None,
            Some(Selection::cursor(
                replace_from
                    .saturating_add(inserted_size)
                    .saturating_add(1),
            )),
        )
    }

    fn is_inline_insertable_node(&self, node_type: &str) -> bool {
        self.schema
            .node(node_type)
            .map(|spec| matches!(spec.role, NodeRole::HardBreak | NodeRole::Inline) && spec.is_void)
            .unwrap_or(false)
    }

    fn can_insert_inline_node_at_pos(&self, pos: u32, node_type: &str) -> bool {
        if !self.is_inline_insertable_node(node_type) {
            return false;
        }

        let doc = self.backend.document();
        let resolved = match doc.resolve(pos) {
            Ok(resolved) => resolved,
            Err(_) => return false,
        };
        let parent = resolved.parent(doc);
        let Some(parent_spec) = self.schema.node(parent.node_type()) else {
            return false;
        };

        matches!(parent_spec.role, NodeRole::TextBlock)
    }

    fn is_horizontal_rule_node(node_type: &str) -> bool {
        matches!(node_type, "horizontalRule" | "horizontal_rule")
    }

    fn insert_inline_node(
        &mut self,
        pos: u32,
        node_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        self.replace_selection_with_inline_node(pos, pos, node_type)
    }

    fn replace_selection_with_inline_node(
        &mut self,
        from: u32,
        to: u32,
        node_type: &str,
    ) -> Result<EditorUpdate, EditorError> {
        if !self.can_insert_inline_node_at_pos(from, node_type) {
            return Ok(self.build_update_from_current());
        }

        let node = Node::void(node_type.to_string(), HashMap::new());
        let mut tx = Transaction::new(Source::Input);
        if from < to {
            tx.add_step(Step::ReplaceRange {
                from,
                to,
                content: Fragment::from(vec![node]),
            });
        } else {
            tx.add_step(Step::InsertNode { pos: from, node });
        }

        match self.apply_transaction_with_selection_adjustments(
            tx,
            None,
            Some(Selection::cursor(from.saturating_add(1))),
        ) {
            Ok(update) => Ok(update),
            Err(EditorError::Transform(_)) => Ok(self.build_update_from_current()),
            Err(e) => Err(e),
        }
    }

    fn empty_paragraph_node() -> Node {
        Node::element("paragraph".to_string(), HashMap::new(), Fragment::empty())
    }

    fn apply_empty_split_action(
        &mut self,
        action: SplitAction,
    ) -> Result<EditorUpdate, EditorError> {
        match action {
            SplitAction::UnwrapList(p) => self.unwrap_from_list(p),
            SplitAction::OutdentList(p) => {
                let remap = self.selection_remap_for_outdent(p);
                let mut tx = Transaction::new(Source::Input);
                tx.add_step(Step::OutdentListItem { pos: p });
                self.apply_transaction_with_selection_remap(tx, remap)
            }
            SplitAction::ExitBlockquote(p) => self.exit_empty_blockquote(p),
        }
    }

    fn empty_split_action(&self, pos: u32) -> Option<SplitAction> {
        self.empty_list_item_split_action(pos)
            .or_else(|| self.empty_blockquote_split_action(pos))
    }

    fn exit_empty_blockquote(&mut self, pos: u32) -> Result<EditorUpdate, EditorError> {
        let context = self.empty_blockquote_exit_context(pos).ok_or_else(|| {
            EditorError::Transform(TransformError::InvalidTarget(
                "cannot exit blockquote outside an empty direct blockquote paragraph".to_string(),
            ))
        })?;

        let quote_content = context.quote_node.content().ok_or_else(|| {
            EditorError::Transform(TransformError::InvalidTarget(
                "blockquote content missing while exiting blockquote".to_string(),
            ))
        })?;

        let before_children = quote_content
            .iter()
            .take(context.block_idx)
            .cloned()
            .collect::<Vec<_>>();
        let after_children = quote_content
            .iter()
            .skip(context.block_idx + 1)
            .cloned()
            .collect::<Vec<_>>();

        let mut replacement = Vec::new();
        let mut target_cursor_pos = context.replace_from;

        if !before_children.is_empty() {
            let before_quote = Node::element(
                context.quote_node.node_type().to_string(),
                context.quote_node.attrs().clone(),
                Fragment::from(before_children),
            );
            target_cursor_pos += before_quote.node_size();
            replacement.push(before_quote);
        }

        replacement.push(Self::empty_paragraph_node());

        if !after_children.is_empty() {
            replacement.push(Node::element(
                context.quote_node.node_type().to_string(),
                context.quote_node.attrs().clone(),
                Fragment::from(after_children),
            ));
        }

        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::ReplaceRange {
            from: context.replace_from,
            to: context.replace_to,
            content: Fragment::from(replacement),
        });

        self.apply_transaction_with_selection_adjustments(
            tx,
            None,
            Some(Selection::cursor(target_cursor_pos.saturating_add(1))),
        )
    }

    fn empty_text_block_replace_range_at(&self, pos: u32) -> Option<(u32, u32)> {
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;
        let block = resolved.parent(doc);
        let block_spec = self.schema.node(block.node_type())?;
        if !matches!(block_spec.role, NodeRole::TextBlock) {
            return None;
        }
        if block.content_size() != 0 {
            return None;
        }

        let block_start = Self::node_delete_start_pos(doc, &resolved.node_path)?;
        Some((block_start, block_start + block.node_size()))
    }

    /// Resolve a document position to a block-level insertion point.
    ///
    /// If `pos` is inside a TextBlock (e.g. a paragraph), returns the position
    /// immediately after that TextBlock's close tag. Otherwise returns `pos`
    /// unchanged.
    fn resolve_block_insert_pos(&self, pos: u32) -> u32 {
        let doc = self.backend.document();
        let resolved = match doc.resolve(pos) {
            Ok(r) => r,
            Err(_) => return pos,
        };

        // Check if the innermost parent is a TextBlock
        let parent = resolved.parent(doc);
        let is_text_block = self
            .schema
            .node(parent.node_type())
            .map(|s| matches!(s.role, NodeRole::TextBlock))
            .unwrap_or(false);

        if !is_text_block {
            return pos; // already at block level
        }

        // Walk from root to the TextBlock, summing positions to find where
        // the TextBlock ends in document coordinates.
        // resolved.node_path gives the child indices from root to the parent.
        let mut abs_pos: u32 = 0; // tracks the content-start of the current node
        let mut node = doc.root();
        for &child_idx in &resolved.node_path {
            // Sum node_sizes of all children before child_idx
            if let Some(content) = node.content() {
                for i in 0..child_idx as usize {
                    if let Some(sibling) = content.child(i) {
                        abs_pos += sibling.node_size();
                    }
                }
            }
            abs_pos += 1; // open tag of the child we descend into
            if let Some(child) = node.child(child_idx as usize) {
                node = child;
            } else {
                return pos;
            }
        }

        // `node` is now the TextBlock, `abs_pos` is the start of its content.
        // Position after the TextBlock = content_start - 1 (back to open tag)
        // + node_size.
        let text_block_start = abs_pos - 1; // position of the open tag
        text_block_start + node.node_size()
    }

    /// Check if the current position can be wrapped in a list.
    ///
    /// Returns `true` if:
    /// - The cursor is already inside a list (toggle off / switch type), or
    /// - The parent context at doc level allows list nodes.
    fn can_wrap_in_list_at(&self, pos: u32) -> bool {
        // If already in a list, wrapping is possible (toggle/switch).
        if self.containing_list_node_at(pos).is_some() {
            return true;
        }

        // Otherwise, check if the doc-level context accepts list nodes.
        let doc = self.backend.document();
        let resolved = match doc.resolve(pos) {
            Ok(r) => r,
            Err(_) => return false,
        };

        // Walk up past TextBlock to find the block-level parent.
        let path = &resolved.node_path;
        let mut nodes_in_path: Vec<&Node> = vec![doc.root()];
        let mut current = doc.root();
        for &idx in path.iter() {
            if let Some(child) = current.child(idx as usize) {
                nodes_in_path.push(child);
                current = child;
            }
        }

        for i in (0..nodes_in_path.len()).rev() {
            let node = nodes_in_path[i];
            if let Some(spec) = self.schema.node(node.node_type()) {
                if matches!(spec.role, NodeRole::TextBlock) {
                    continue;
                }
                // Check if this parent's content rule accepts list nodes.
                let insertable = self.schema.insertable_nodes_at(spec, node.child_count());
                return insertable.iter().any(|name| {
                    self.schema
                        .node(name)
                        .map(|s| matches!(s.role, NodeRole::List { .. }))
                        .unwrap_or(false)
                });
            }
        }

        false
    }

    fn can_toggle_blockquote_at(&self, pos: u32) -> bool {
        let Some(blockquote_type) = self.blockquote_node_name() else {
            return false;
        };
        if self.containing_blockquote_node_at(pos).is_some() {
            return true;
        }

        let doc = self.backend.document();
        let resolved = match doc.resolve(pos) {
            Ok(r) => r,
            Err(_) => return false,
        };

        let path = &resolved.node_path;
        let mut nodes_in_path: Vec<&Node> = vec![doc.root()];
        let mut current = doc.root();
        for &idx in path.iter() {
            if let Some(child) = current.child(idx as usize) {
                nodes_in_path.push(child);
                current = child;
            }
        }

        for i in (0..nodes_in_path.len()).rev() {
            let node = nodes_in_path[i];
            let Some(spec) = self.schema.node(node.node_type()) else {
                continue;
            };
            if matches!(spec.role, NodeRole::TextBlock) {
                continue;
            }
            let insertable = self.schema.insertable_nodes_at(spec, node.child_count());
            return insertable.iter().any(|name| name == &blockquote_type);
        }

        false
    }

    fn wrap_selected_blocks_in_list(
        &mut self,
        from: u32,
        to: u32,
        list_type: &str,
    ) -> Result<Option<EditorUpdate>, EditorError> {
        let Some(range) = self.selected_block_range(from, to) else {
            return Ok(None);
        };
        if range.parent_path.is_empty() {
            return Ok(None);
        }

        let Some(parent) = self.backend.document().node_at(&range.parent_path) else {
            return Ok(None);
        };
        if parent.node_type() != "blockquote" {
            return Ok(None);
        }

        let Some(parent_spec) = self.schema.node(parent.node_type()) else {
            return Ok(None);
        };
        let insertable = self.schema.insertable_nodes_at(parent_spec, parent.child_count());
        if !insertable.iter().any(|name| name == list_type) {
            return Ok(Some(self.build_update_from_current()));
        }

        let item_type = self
            .schema
            .all_nodes()
            .find(|n| matches!(n.role, crate::schema::NodeRole::ListItem))
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "listItem".to_string());

        let list_items = range
            .selected_blocks
            .into_iter()
            .map(|block| {
                Node::element(
                    item_type.clone(),
                    HashMap::new(),
                    Fragment::from(vec![block]),
                )
            })
            .collect::<Vec<_>>();
        let list_node = Node::element(
            list_type.to_string(),
            list_attrs_for_type(list_type, &HashMap::new()),
            Fragment::from(list_items),
        );

        let mut tx = Transaction::new(Source::Input);
        tx.add_step(Step::ReplaceRange {
            from: range.replace_from,
            to: range.replace_to,
            content: Fragment::from(vec![list_node]),
        });
        match self.apply_transaction(tx) {
            Ok(update) => Ok(Some(update)),
            Err(EditorError::Transform(_)) => Ok(Some(self.build_update_from_current())),
            Err(e) => Err(e),
        }
    }

    /// Get marks at a document position.
    fn marks_at_pos(&self, pos: u32) -> Vec<Mark> {
        let doc = self.backend.document();
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
            if child.is_text() {
                if offset <= parent_offset && parent_offset <= offset + child_size {
                    return child.marks().to_vec();
                }
            }
            offset += child_size;
        }

        // If at end of content, check the last text node.
        if parent_offset == offset {
            // Check if there's a preceding text node.
            for child in content.iter().rev() {
                if child.is_text() {
                    return child.marks().to_vec();
                }
            }
        }

        Vec::new()
    }

    /// Get the node type names in the path at a document position.
    fn nodes_at_pos(&self, pos: u32) -> Vec<String> {
        let doc = self.backend.document();
        let resolved = match doc.resolve(pos) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let mut result = vec![doc.root().node_type().to_string()];
        let mut node = doc.root();
        for &idx in &resolved.node_path {
            if let Some(child) = node.child(idx as usize) {
                result.push(child.node_type().to_string());
                node = child;
            }
        }

        result
    }

    /// Check if all text in a range has a given mark.
    fn range_has_mark(&self, from: u32, to: u32, mark_name: &str) -> bool {
        let doc = self.backend.document();
        let resolved_from = match doc.resolve(from) {
            Ok(r) => r,
            Err(_) => return false,
        };

        let parent = resolved_from.parent(doc);
        let content = match parent.content() {
            Some(c) => c,
            None => return false,
        };

        let from_offset = resolved_from.parent_offset;
        let to_offset = from_offset + (to - from);

        let mut offset: u32 = 0;
        let mut found_text = false;

        for child in content.iter() {
            let child_size = child.node_size();
            let child_start = offset;
            let child_end = offset + child_size;

            if child.is_text() && child_end > from_offset && child_start < to_offset {
                found_text = true;
                if !child.marks().iter().any(|m| m.mark_type() == mark_name) {
                    return false;
                }
            }

            offset = child_end;
        }

        found_text
    }

    /// Get the contiguous range of a mark at a collapsed position within the current parent.
    fn mark_range_at_pos(&self, pos: u32, mark_name: &str) -> Option<(u32, u32)> {
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;
        let parent = resolved.parent(doc);
        let content = parent.content()?;
        let parent_offset = resolved.parent_offset;
        let parent_content_start = self.content_start_for_path(&resolved.node_path)?;

        struct MarkedTextChild {
            index: usize,
            start: u32,
            end: u32,
            mark: Option<Mark>,
        }

        let mut children = Vec::new();
        let mut offset: u32 = 0;
        for (index, child) in content.iter().enumerate() {
            let child_size = child.node_size();
            if child.is_text() {
                let mark = child
                    .marks()
                    .iter()
                    .find(|m| m.mark_type() == mark_name)
                    .cloned();
                children.push(MarkedTextChild {
                    index,
                    start: offset,
                    end: offset + child_size,
                    mark,
                });
            }
            offset += child_size;
        }

        let target = children
            .iter()
            .find(|child| {
                child.mark.is_some()
                    && child.start <= parent_offset
                    && parent_offset <= child.end
            })
            .or_else(|| {
                if parent_offset == offset {
                    children.iter().rev().find(|child| child.mark.is_some())
                } else {
                    None
                }
            })?;

        let target_mark = target.mark.as_ref()?;
        let mut range_start = target.start;
        let mut range_end = target.end;

        let mut left_index = target.index;
        while left_index > 0 {
            let sibling = content.child(left_index - 1)?;
            if !sibling.is_text() {
                break;
            }
            let sibling_mark = sibling.marks().iter().find(|m| *m == target_mark);
            if sibling_mark.is_none() {
                break;
            }
            range_start -= sibling.node_size();
            left_index -= 1;
        }

        let mut right_index = target.index;
        while right_index + 1 < content.child_count() {
            let sibling = content.child(right_index + 1)?;
            if !sibling.is_text() {
                break;
            }
            let sibling_mark = sibling.marks().iter().find(|m| *m == target_mark);
            if sibling_mark.is_none() {
                break;
            }
            range_end += sibling.node_size();
            right_index += 1;
        }

        Some((parent_content_start + range_start, parent_content_start + range_end))
    }

    fn content_start_for_path(&self, path: &[u16]) -> Option<u32> {
        let doc = self.backend.document();
        let mut content_start: u32 = 0;
        let mut node = doc.root();

        for &child_idx in path {
            if let Some(content) = node.content() {
                for i in 0..child_idx as usize {
                    content_start += content.child(i)?.node_size();
                }
            }
            content_start += 1;
            node = node.child(child_idx as usize)?;
        }

        Some(content_start)
    }

    fn containing_list_node_at(&self, pos: u32) -> Option<(u32, Node)> {
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;
        let mut node = doc.root();
        let mut content_start: u32 = 0;
        let mut nearest_list: Option<(u32, Node)> = None;

        for &idx in &resolved.node_path {
            let content = node.content()?;
            let mut child_open = content_start;
            for i in 0..(idx as usize) {
                child_open += content.child(i)?.node_size();
            }

            let child = content.child(idx as usize)?;
            let spec = self.schema.node(child.node_type())?;
            if matches!(spec.role, NodeRole::List { .. }) {
                nearest_list = Some((child_open, child.clone()));
            }

            if !child.is_element() {
                break;
            }

            node = child;
            content_start = child_open + 1;
        }

        nearest_list
    }

    fn blockquote_node_name(&self) -> Option<String> {
        self.schema
            .node_by_html_tag("blockquote")
            .map(|spec| spec.name.clone())
    }

    fn block_path_for_pos(&self, pos: u32) -> Option<Vec<u16>> {
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;
        let mut node = doc.root();
        let mut path = Vec::new();
        let mut block_path = None;

        for &idx in &resolved.node_path {
            let child = node.child(idx as usize)?;
            path.push(idx);
            let spec = self.schema.node(child.node_type())?;
            if matches!(
                spec.role,
                NodeRole::TextBlock | NodeRole::Block | NodeRole::List { .. }
            ) {
                block_path = Some(path.clone());
            }
            node = child;
        }

        block_path
    }

    fn selected_block_range(&self, from: u32, to: u32) -> Option<BlockSelectionRange> {
        let start_block_path = self.block_path_for_pos(from)?;
        let end_pos = if to > from { to.saturating_sub(1) } else { from };
        let end_block_path = self.block_path_for_pos(end_pos)?;

        let start_parent = &start_block_path[..start_block_path.len().saturating_sub(1)];
        let end_parent = &end_block_path[..end_block_path.len().saturating_sub(1)];
        if start_parent != end_parent {
            return None;
        }

        let parent_path = start_parent.to_vec();
        let doc = self.backend.document();
        let parent = if parent_path.is_empty() {
            doc.root()
        } else {
            doc.node_at(&parent_path)?
        };

        let first_idx = *start_block_path.last()? as usize;
        let last_idx = *end_block_path.last()? as usize;
        if first_idx > last_idx {
            return None;
        }

        let selected_blocks = (first_idx..=last_idx)
            .map(|idx| parent.child(idx).cloned())
            .collect::<Option<Vec<_>>>()?;
        let replace_from = Self::node_delete_start_pos(doc, &start_block_path)?;
        let replace_to = Self::node_delete_start_pos(doc, &end_block_path)?
            + parent.child(last_idx)?.node_size();

        Some(BlockSelectionRange {
            parent_path,
            replace_from,
            replace_to,
            selected_blocks,
        })
    }

    fn containing_blockquote_node_at(&self, pos: u32) -> Option<(u32, Vec<u16>, Node)> {
        let blockquote_type = self.blockquote_node_name()?;
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;
        let mut node = doc.root();
        let mut content_start: u32 = 0;
        let mut nearest: Option<(u32, Vec<u16>, Node)> = None;
        let mut path = Vec::new();

        for &idx in &resolved.node_path {
            let content = node.content()?;
            let mut child_open = content_start;
            for i in 0..(idx as usize) {
                child_open += content.child(i)?.node_size();
            }

            let child = content.child(idx as usize)?;
            path.push(idx);
            if child.node_type() == blockquote_type {
                nearest = Some((child_open, path.clone(), child.clone()));
            }

            if !child.is_element() {
                break;
            }
            node = child;
            content_start = child_open + 1;
        }

        nearest
    }

    fn shifted_selection(&self, delta: i32) -> Option<Selection> {
        match self.selection.clone() {
            Selection::Text { anchor, head } => {
                let anchor = if delta >= 0 {
                    anchor.checked_add(delta as u32)?
                } else {
                    anchor.checked_sub(delta.unsigned_abs())?
                };
                let head = if delta >= 0 {
                    head.checked_add(delta as u32)?
                } else {
                    head.checked_sub(delta.unsigned_abs())?
                };
                Some(Selection::text(anchor, head))
            }
            Selection::Node { pos } => {
                let pos = if delta >= 0 {
                    pos.checked_add(delta as u32)?
                } else {
                    pos.checked_sub(delta.unsigned_abs())?
                };
                Some(Selection::Node { pos })
            }
            Selection::All => None,
        }
    }

    fn can_indent_list_item(&self, pos: u32) -> bool {
        self.list_item_context_at(pos)
            .map(|ctx| ctx.list_item_idx > 0)
            .unwrap_or(false)
    }

    fn can_outdent_list_item(&self, pos: u32) -> bool {
        self.list_item_context_at(pos)
            .map(|ctx| !ctx.list_path.is_empty() && ctx.parent_is_list_item)
            .unwrap_or(false)
    }

    fn selection_remap_for_outdent(&self, pos: u32) -> Option<SelectionPathRemap> {
        let context = self.list_item_context_at(pos)?;
        if context.list_path.is_empty() || !context.parent_is_list_item {
            return None;
        }

        let current_item_path = {
            let mut path = context.list_path.clone();
            path.push(context.list_item_idx as u16);
            path
        };
        let parent_list_item_path = &context.list_path[..context.list_path.len() - 1];
        if parent_list_item_path.is_empty() {
            return None;
        }
        let parent_list_path = &parent_list_item_path[..parent_list_item_path.len() - 1];
        let parent_list_item_idx = *parent_list_item_path.last()? as usize;
        let mut target_item_path = parent_list_path.to_vec();
        target_item_path.push((parent_list_item_idx + 1) as u16);

        let item_open = Self::node_open_pos(self.backend.document(), &current_item_path)?;
        let selection = SelectionOffset::from_selection(&self.selection, item_open)?;

        Some(SelectionPathRemap {
            target_item_path,
            selection,
        })
    }

    fn list_item_context_at(&self, pos: u32) -> Option<ListItemContext> {
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;
        let path = &resolved.node_path;

        let mut current_node = doc.root();
        let mut list_item_depth = None;

        for (depth_idx, &child_idx) in path.iter().enumerate() {
            let content = current_node.content()?;
            let child = content.child(child_idx as usize)?;

            if let Some(spec) = self.schema.node(child.node_type()) {
                if matches!(spec.role, NodeRole::ListItem) {
                    list_item_depth = Some(depth_idx);
                }
            }

            current_node = child;
        }

        let li_depth = list_item_depth?;
        let list_path = path[..li_depth].to_vec();
        let parent_is_list_item = li_depth > 0
            && doc
                .node_at(&path[..li_depth - 1])
                .map(|node| node.node_type() == "listItem")
                .unwrap_or(false);

        Some(ListItemContext {
            list_path,
            list_item_idx: path[li_depth] as usize,
            parent_is_list_item,
        })
    }

    fn node_open_pos(doc: &Document, path: &[u16]) -> Option<u32> {
        let mut current = doc.root();
        let mut open_pos = 0;

        for &idx in path {
            let content = current.content()?;
            let mut child_open = open_pos + 1;
            for i in 0..idx {
                child_open += content.child(i as usize)?.node_size();
            }
            current = content.child(idx as usize)?;
            open_pos = child_open;
        }

        Some(open_pos)
    }

    fn node_delete_start_pos(doc: &Document, path: &[u16]) -> Option<u32> {
        let open_pos = Self::node_open_pos(doc, path)?;
        open_pos.checked_sub(1)
    }

    fn empty_list_unwrap_pos_for_scalar_delete(
        &self,
        scalar_from: u32,
        scalar_to: u32,
        doc_from: u32,
        doc_to: u32,
    ) -> Option<u32> {
        if scalar_from >= scalar_to || doc_from != doc_to {
            return None;
        }

        let doc = self.backend.document();
        let resolved = doc.resolve(doc_to).ok()?;
        let parent = resolved.parent(doc);
        let parent_spec = self.schema.node(parent.node_type())?;
        if !matches!(parent_spec.role, NodeRole::TextBlock) {
            return None;
        }
        if resolved.parent_offset != 0 || parent.content_size() != 0 {
            return None;
        }
        if self.doc_to_scalar(doc_to) != scalar_to {
            return None;
        }

        let mut node = doc.root();
        let mut list_item_depth = None;
        for (depth, &idx) in resolved.node_path.iter().enumerate() {
            let child = node.child(idx as usize)?;
            let spec = self.schema.node(child.node_type())?;
            if matches!(spec.role, NodeRole::ListItem) {
                list_item_depth = Some(depth);
            }
            node = child;
        }

        let list_item_depth = list_item_depth?;
        let block_idx_in_list_item = *resolved.node_path.get(list_item_depth + 1)?;
        if block_idx_in_list_item != 0 {
            return None;
        }

        Some(doc_to)
    }

    fn empty_blockquote_exit_pos_for_scalar_delete(
        &self,
        scalar_from: u32,
        scalar_to: u32,
        doc_from: u32,
        doc_to: u32,
    ) -> Option<u32> {
        if scalar_from >= scalar_to || doc_from != doc_to {
            return None;
        }
        if self.doc_to_scalar(doc_to) != scalar_to {
            return None;
        }

        self.empty_blockquote_exit_context(doc_to)
            .map(|context| context.cursor_pos)
    }

    fn lift_empty_text_block_out_of_list_for_scalar_delete(
        &self,
        scalar_from: u32,
        scalar_to: u32,
        doc_from: u32,
        doc_to: u32,
    ) -> Option<(u32, u32, Vec<Node>, Selection)> {
        if scalar_from >= scalar_to || doc_from != doc_to {
            return None;
        }

        let doc = self.backend.document();
        let resolved = doc.resolve(doc_to).ok()?;
        let block = resolved.parent(doc);
        let block_spec = self.schema.node(block.node_type())?;
        if !matches!(block_spec.role, NodeRole::TextBlock) {
            return None;
        }
        if resolved.parent_offset != 0 || block.content_size() != 0 {
            return None;
        }
        if self.doc_to_scalar(doc_to) != scalar_to {
            return None;
        }

        let mut current = doc.root();
        let mut list_item_depth = None;
        for (depth, &idx) in resolved.node_path.iter().enumerate() {
            let child = current.child(idx as usize)?;
            let spec = self.schema.node(child.node_type())?;
            if matches!(spec.role, NodeRole::ListItem) {
                list_item_depth = Some(depth);
            }
            current = child;
        }

        let list_item_depth = list_item_depth?;
        let block_idx_in_list_item = *resolved.node_path.get(list_item_depth + 1)? as usize;
        if block_idx_in_list_item == 0 {
            return None;
        }

        let list_path = resolved.node_path[..list_item_depth].to_vec();
        let list_node = doc.node_at(&list_path)?;
        let list_spec = self.schema.node(list_node.node_type())?;
        if !matches!(list_spec.role, NodeRole::List { .. }) {
            return None;
        }

        let list_content = list_node.content()?;
        let list_item_idx = resolved.node_path[list_item_depth] as usize;
        let list_item_node = list_content.child(list_item_idx)?;
        let list_item_content = list_item_node.content()?;
        if block_idx_in_list_item != list_item_content.child_count().checked_sub(1)? {
            return None;
        }

        let prefix_children: Vec<Node> = (0..block_idx_in_list_item)
            .map(|idx| list_item_content.child(idx).cloned())
            .collect::<Option<Vec<_>>>()?;
        if prefix_children.is_empty() {
            return None;
        }
        let lifted_block = list_item_content.child(block_idx_in_list_item)?.clone();

        let mut before_items: Vec<Node> = (0..list_item_idx)
            .map(|idx| list_content.child(idx).cloned())
            .collect::<Option<Vec<_>>>()?;
        before_items.push(rebuild_element(list_item_node, prefix_children));
        let after_items: Vec<Node> = ((list_item_idx + 1)..list_content.child_count())
            .map(|idx| list_content.child(idx).cloned())
            .collect::<Option<Vec<_>>>()?;

        let list_start = Self::node_delete_start_pos(doc, &list_path)?;
        let mut replacement_nodes = Vec::new();

        let selection_anchor = if !before_items.is_empty() {
            let before_list = Node::element(
                list_node.node_type().to_string(),
                list_node.attrs().clone(),
                Fragment::from(before_items),
            );
            let anchor = list_start + before_list.node_size() + 1;
            replacement_nodes.push(before_list);
            anchor
        } else {
            list_start + 1
        };

        replacement_nodes.push(lifted_block);

        if !after_items.is_empty() {
            replacement_nodes.push(Node::element(
                list_node.node_type().to_string(),
                list_node.attrs().clone(),
                Fragment::from(after_items),
            ));
        }

        Some((
            list_start,
            list_start + list_node.node_size(),
            replacement_nodes,
            Selection::cursor(selection_anchor),
        ))
    }

    /// Detect whether `split_block(pos)` should unwrap or outdent instead of splitting.
    ///
    /// Returns `Some(SplitAction)` when `pos` is inside an empty list item
    /// (a list item whose only child is an empty text block). Returns `None`
    /// for non-empty items or positions not in a list — callers fall through
    /// to the normal split_block path.
    fn empty_list_item_split_action(&self, pos: u32) -> Option<SplitAction> {
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;

        // 1. Check we're in an empty text block
        let parent = resolved.parent(doc);
        let parent_spec = self.schema.node(parent.node_type())?;
        if !matches!(parent_spec.role, NodeRole::TextBlock) {
            return None;
        }
        if resolved.parent_offset != 0 || parent.content_size() != 0 {
            return None;
        }

        // 2. Walk the node path to find the innermost list item
        let mut node = doc.root();
        let mut list_item_depth: Option<usize> = None;
        for (depth, &idx) in resolved.node_path.iter().enumerate() {
            let child = node.child(idx as usize)?;
            let spec = self.schema.node(child.node_type())?;
            if matches!(spec.role, NodeRole::ListItem) {
                list_item_depth = Some(depth);
            }
            node = child;
        }
        let list_item_depth = list_item_depth?;

        // 3. Verify the text block is the first child of the list item
        let block_idx_in_list_item = *resolved.node_path.get(list_item_depth + 1)?;
        if block_idx_in_list_item != 0 {
            return None;
        }

        // 4. Verify the list item has exactly one child (the empty text block).
        //    A list item with <p></p><ul>...</ul> has child_count > 1 and should
        //    NOT be treated as "empty" — normal split_block applies.
        let mut list_item_node = doc.root();
        for &idx in &resolved.node_path[..=list_item_depth] {
            list_item_node = list_item_node.child(idx as usize)?;
        }
        if list_item_node.child_count() != 1 {
            return None;
        }

        // 5. Nesting check: is the list node's parent also a list item?
        if list_item_depth >= 2 {
            let mut ancestor = doc.root();
            for &idx in &resolved.node_path[..list_item_depth - 1] {
                ancestor = ancestor.child(idx as usize)?;
            }
            let ancestor_spec = self.schema.node(ancestor.node_type())?;
            if matches!(ancestor_spec.role, NodeRole::ListItem) {
                return Some(SplitAction::OutdentList(pos));
            }
        }

        Some(SplitAction::UnwrapList(pos))
    }

    fn empty_blockquote_split_action(&self, pos: u32) -> Option<SplitAction> {
        self.empty_blockquote_exit_context(pos)
            .map(|context| SplitAction::ExitBlockquote(context.cursor_pos))
    }

    fn empty_blockquote_exit_context(&self, pos: u32) -> Option<EmptyBlockquoteExitContext> {
        let blockquote_type = self.blockquote_node_name()?;
        let doc = self.backend.document();
        let resolved = doc.resolve(pos).ok()?;

        let parent = resolved.parent(doc);
        let parent_spec = self.schema.node(parent.node_type())?;
        if !matches!(parent_spec.role, NodeRole::TextBlock) {
            return None;
        }
        if resolved.parent_offset != 0 || parent.content_size() != 0 {
            return None;
        }

        let mut node = doc.root();
        let mut blockquote_depth: Option<usize> = None;
        for (depth, &idx) in resolved.node_path.iter().enumerate() {
            let child = node.child(idx as usize)?;
            if child.node_type() == blockquote_type {
                blockquote_depth = Some(depth);
            }
            node = child;
        }

        let blockquote_depth = blockquote_depth?;
        if resolved.node_path.len() != blockquote_depth + 2 {
            return None;
        }

        let quote_path = resolved.node_path[..=blockquote_depth].to_vec();
        let block_idx = *resolved.node_path.get(blockquote_depth + 1)? as usize;
        let quote_node = doc.node_at(&quote_path)?.clone();
        let quote_content = quote_node.content()?;
        if block_idx >= quote_content.child_count() {
            return None;
        }

        let replace_from = Self::node_delete_start_pos(doc, &quote_path)?;
        let replace_to = replace_from.checked_add(quote_node.node_size())?;

        Some(EmptyBlockquoteExitContext {
            cursor_pos: pos,
            quote_node,
            block_idx,
            replace_from,
            replace_to,
        })
    }

    fn block_void_replacement_for_scalar_delete(
        &self,
        scalar_from: u32,
        scalar_to: u32,
        _doc_from: u32,
        doc_to: u32,
    ) -> Option<(u32, u32, Selection)> {
        if scalar_from >= scalar_to {
            return None;
        }

        let doc = self.backend.document();
        let (resolved, block_open) = if doc_to < doc.content_size() {
            let candidate = doc.resolve(doc_to + 1).ok()?;
            let block = candidate.parent(doc);
            let block_spec = self.schema.node(block.node_type())?;
            if matches!(block_spec.role, NodeRole::TextBlock)
                && candidate.parent_offset == 0
                && block.content_size() == 0
                && self.doc_to_scalar(doc_to) == scalar_to
            {
                (candidate, doc_to)
            } else {
                let fallback = doc.resolve(doc_to).ok()?;
                let fallback_block = fallback.parent(doc);
                let fallback_spec = self.schema.node(fallback_block.node_type())?;
                if !matches!(fallback_spec.role, NodeRole::TextBlock) {
                    return None;
                }
                if fallback.parent_offset != 0 || fallback_block.content_size() != 0 {
                    return None;
                }
                if self.doc_to_scalar(doc_to) != scalar_to {
                    return None;
                }
                (fallback, doc_to.checked_sub(1)?)
            }
        } else {
            let fallback = doc.resolve(doc_to).ok()?;
            let fallback_block = fallback.parent(doc);
            let fallback_spec = self.schema.node(fallback_block.node_type())?;
            if !matches!(fallback_spec.role, NodeRole::TextBlock) {
                return None;
            }
            if fallback.parent_offset != 0 || fallback_block.content_size() != 0 {
                return None;
            }
            if self.doc_to_scalar(doc_to) != scalar_to {
                return None;
            }
            (fallback, doc_to.checked_sub(1)?)
        };

        let block = resolved.parent(doc);
        let block_path = &resolved.node_path;
        let &block_index = block_path.last()?;
        if block_index == 0 {
            return None;
        }

        let parent_path = &block_path[..block_path.len() - 1];
        let parent = doc.node_at(parent_path)?;
        let previous_sibling = parent.child(block_index as usize - 1)?;
        if !self.is_block_void_node(previous_sibling) {
            return None;
        }

        let mut previous_path = parent_path.to_vec();
        previous_path.push(block_index - 1);
        let previous_start = Self::node_delete_start_pos(doc, &previous_path)?;
        let replace_to = block_open + block.node_size();
        Some((
            previous_start,
            replace_to,
            Selection::cursor(previous_start.saturating_add(1)),
        ))
    }

    fn empty_text_block_delete_range_for_scalar_delete(
        &self,
        scalar_from: u32,
        scalar_to: u32,
        doc_from: u32,
        doc_to: u32,
    ) -> Option<(u32, u32)> {
        if scalar_from >= scalar_to {
            return None;
        }

        let doc = self.backend.document();
        let (resolved, block_open) = if doc_to < doc.content_size() {
            let candidate = doc.resolve(doc_to + 1).ok()?;
            let block = candidate.parent(doc);
            let block_spec = self.schema.node(block.node_type())?;
            if matches!(block_spec.role, NodeRole::TextBlock)
                && candidate.parent_offset == 0
                && block.content_size() == 0
                && self.doc_to_scalar(doc_to) == scalar_to
            {
                (candidate, doc_to)
            } else {
                let fallback = doc.resolve(doc_to).ok()?;
                let fallback_block = fallback.parent(doc);
                let fallback_spec = self.schema.node(fallback_block.node_type())?;
                if !matches!(fallback_spec.role, NodeRole::TextBlock) {
                    return None;
                }
                if fallback.parent_offset != 0 || fallback_block.content_size() != 0 {
                    return None;
                }
                if self.doc_to_scalar(doc_to) != scalar_to {
                    return None;
                }
                (fallback, doc_to.checked_sub(1)?)
            }
        } else {
            let fallback = doc.resolve(doc_to).ok()?;
            let fallback_block = fallback.parent(doc);
            let fallback_spec = self.schema.node(fallback_block.node_type())?;
            if !matches!(fallback_spec.role, NodeRole::TextBlock) {
                return None;
            }
            if fallback.parent_offset != 0 || fallback_block.content_size() != 0 {
                return None;
            }
            if self.doc_to_scalar(doc_to) != scalar_to {
                return None;
            }
            (fallback, doc_to.checked_sub(1)?)
        };

        let block = resolved.parent(doc);
        let block_path = &resolved.node_path;
        let &block_index = block_path.last()?;
        if block_index == 0 {
            return None;
        }

        let parent_path = &block_path[..block_path.len() - 1];
        let parent = doc.node_at(parent_path)?;
        let previous_sibling = parent.child(block_index as usize - 1)?;
        if !previous_sibling.is_element() && !previous_sibling.is_void() {
            return None;
        }
        if doc_from != doc_to {
            return None;
        }
        Some((block_open, block_open + block.node_size()))
    }

    fn is_block_void_node(&self, node: &Node) -> bool {
        node.is_void()
            && self
                .schema
                .node(node.node_type())
                .map(|spec| matches!(spec.role, NodeRole::Block))
                .unwrap_or(false)
    }

    fn effective_marks_for_insert(&self, pos: u32) -> Vec<Mark> {
        self.stored_marks
            .clone()
            .unwrap_or_else(|| self.marks_at_pos(pos))
    }

    fn effective_marks_for_selection(&self, pos: u32, collapsed: bool) -> Vec<Mark> {
        if collapsed {
            self.stored_marks
                .clone()
                .unwrap_or_else(|| self.marks_at_pos(pos))
        } else {
            self.common_marks_for_text_selection()
        }
    }

    fn common_marks_for_text_selection(&self) -> Vec<Mark> {
        let Selection::Text { anchor, head } = &self.selection else {
            return Vec::new();
        };

        let from = (*anchor).min(*head);
        let to = (*anchor).max(*head);
        if from == to {
            return self.marks_at_pos(from);
        }

        let mut overlapping_marks = Vec::new();
        Self::collect_text_marks_in_range(self.backend.document().root(), 0, from, to, &mut overlapping_marks);

        let mut iter = overlapping_marks.into_iter();
        let Some(first_marks) = iter.next() else {
            return self.marks_at_pos(from);
        };

        let mut intersection = first_marks;
        for marks in iter {
            intersection.retain(|mark| marks.iter().any(|candidate| candidate == mark));
            if intersection.is_empty() {
                break;
            }
        }

        intersection
    }

    fn collect_text_marks_in_range(
        node: &Node,
        start: u32,
        from: u32,
        to: u32,
        out: &mut Vec<Vec<Mark>>,
    ) {
        if from >= to {
            return;
        }

        if node.is_text() {
            let end = start + node.node_size();
            if start < to && end > from {
                out.push(node.marks().to_vec());
            }
            return;
        }

        let Some(content) = node.content() else {
            return;
        };

        let mut child_start = if node.node_type() == "doc" { start } else { start + 1 };
        for child in content.iter() {
            let child_end = child_start + child.node_size();
            if child_end > from && child_start < to {
                Self::collect_text_marks_in_range(child, child_start, from, to, out);
            }
            child_start = child_end;
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an empty document with a single empty paragraph.
fn make_empty_doc(schema: &Schema) -> Document {
    use crate::schema::NodeRole;

    let para_name = schema
        .all_nodes()
        .find(|n| matches!(n.role, NodeRole::TextBlock))
        .map(|n| n.name.as_str())
        .unwrap_or("paragraph");

    let para = Node::element(para_name.to_string(), HashMap::new(), Fragment::empty());
    let doc_node = Node::element(
        "doc".to_string(),
        HashMap::new(),
        Fragment::from(vec![para]),
    );
    Document::new(doc_node)
}

fn list_attrs_for_type(
    list_type: &str,
    current_attrs: &HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    if matches!(list_type, "orderedList" | "ordered_list") {
        let mut attrs = HashMap::new();
        if let Some(start) = current_attrs.get("start") {
            attrs.insert("start".to_string(), start.clone());
        }
        attrs
    } else {
        HashMap::new()
    }
}

struct ListItemContext {
    list_path: Vec<u16>,
    list_item_idx: usize,
    parent_is_list_item: bool,
}

struct BlockSelectionRange {
    parent_path: Vec<u16>,
    replace_from: u32,
    replace_to: u32,
    selected_blocks: Vec<Node>,
}

struct EmptyBlockquoteExitContext {
    cursor_pos: u32,
    quote_node: Node,
    block_idx: usize,
    replace_from: u32,
    replace_to: u32,
}

impl SelectionPathRemap {
    fn resolve(&self, doc: &Document) -> Option<Selection> {
        let item_open = Editor::node_open_pos(doc, &self.target_item_path)?;
        Some(self.selection.to_selection(item_open))
    }
}

impl SelectionOffset {
    fn from_selection(selection: &Selection, item_open: u32) -> Option<Self> {
        match selection {
            Selection::Text { anchor, head } => Some(Self::Text {
                anchor: anchor.checked_sub(item_open)?,
                head: head.checked_sub(item_open)?,
            }),
            Selection::Node { pos } => Some(Self::Node {
                pos: pos.checked_sub(item_open)?,
            }),
            Selection::All => None,
        }
    }

    fn to_selection(&self, item_open: u32) -> Selection {
        match self {
            Self::Text { anchor, head } => Selection::text(item_open + anchor, item_open + head),
            Self::Node { pos } => Selection::Node {
                pos: item_open + pos,
            },
        }
    }
}
