pub mod apply;
pub mod mapping;
pub mod steps;

// Re-export apply_step for use by the backend module.
pub use apply::apply_step;
pub use apply::DocumentValidator;
pub(crate) use apply::{
    apply_step_canonical_marks, validate_canonical_mark_set, validate_canonical_marks,
};

use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::Schema;

pub use mapping::StepMap;

// ---------------------------------------------------------------------------
// TransformError
// ---------------------------------------------------------------------------

/// Errors that can occur when applying a transaction to a document.
#[derive(Debug)]
pub enum TransformError {
    /// The step references a position that is out of bounds.
    OutOfBounds(String),
    /// The step has an invalid range (e.g. from > to).
    InvalidRange(String),
    /// The resulting document violates schema content rules.
    ContentViolation(String),
    /// The step type is declared but not yet implemented.
    NotImplemented(String),
    /// The position does not resolve to a text-containing node.
    InvalidTarget(String),
}

impl std::fmt::Display for TransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransformError::OutOfBounds(msg) => write!(f, "out of bounds: {msg}"),
            TransformError::InvalidRange(msg) => write!(f, "invalid range: {msg}"),
            TransformError::ContentViolation(msg) => write!(f, "content violation: {msg}"),
            TransformError::NotImplemented(msg) => write!(f, "not implemented: {msg}"),
            TransformError::InvalidTarget(msg) => write!(f, "invalid target: {msg}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

/// The origin of a transaction, used for filtering and history bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// User keyboard/IME input.
    Input,
    /// Formatting toggles (bold, italic, etc.).
    Format,
    /// Content pasted from clipboard.
    Paste,
    /// Undo/redo operations.
    History,
    /// Programmatic API calls.
    Api,
    /// CRDT reconciliation with remote peers.
    Reconciliation,
}

// ---------------------------------------------------------------------------
// Step
// ---------------------------------------------------------------------------

/// A single atomic document transformation.
///
/// Steps are the building blocks of transactions. Each step describes one
/// discrete change to the document tree.
#[derive(Debug, Clone)]
pub enum Step {
    // ---- Implemented in 4a ----
    /// Insert text at a document position. The position must resolve inside a
    /// text block (e.g. paragraph). If `marks` are provided, the inserted text
    /// carries those marks; otherwise it inherits context marks.
    InsertText {
        pos: u32,
        text: String,
        marks: Vec<Mark>,
    },

    /// Delete content between two document positions. Both positions must be
    /// within the same parent node (for this initial implementation).
    DeleteRange { from: u32, to: u32 },

    /// Apply a mark to a range of text. May split text nodes at boundaries.
    AddMark { from: u32, to: u32, mark: Mark },

    /// Remove all instances of a mark type from a range of text.
    RemoveMark {
        from: u32,
        to: u32,
        mark_type: String,
    },

    // ---- Declared but not yet implemented (4b) ----
    /// Split a block node at a position, creating a new block of the given type.
    SplitBlock {
        pos: u32,
        node_type: String,
        attrs: HashMap<String, serde_json::Value>,
    },

    /// Join two adjacent block nodes at the given boundary position.
    JoinBlocks { pos: u32 },

    // ---- Declared but not yet implemented (4c) ----
    /// Wrap a range of blocks in a list structure.
    WrapInList {
        from: u32,
        to: u32,
        list_type: String,
        item_type: String,
        attrs: HashMap<String, serde_json::Value>,
        /// Attrs applied to each list item this step creates. User-facing
        /// wraps pass an empty map (items get their schema defaults); the
        /// inverse of UnwrapFromList carries the unwrapped item's original
        /// attrs here so undo restores them (e.g. taskItem's `checked`).
        item_attrs: HashMap<String, serde_json::Value>,
    },

    /// Unwrap a list item, lifting its content out of the list.
    UnwrapFromList { pos: u32 },

    /// Increase the nesting level of the list item at `pos`.
    IndentListItem { pos: u32 },

    /// Decrease the nesting level of the list item at `pos`.
    OutdentListItem { pos: u32 },

    // ---- Declared but not yet implemented (4d) ----
    /// Insert a complete node at a position.
    InsertNode { pos: u32, node: Node },

    /// Replace the attrs of a node without changing its content.
    UpdateNodeAttrs {
        pos: u32,
        attrs: HashMap<String, serde_json::Value>,
    },

    /// Replace a range with arbitrary content.
    ReplaceRange {
        from: u32,
        to: u32,
        content: Fragment,
    },
}

// ---------------------------------------------------------------------------
// Transaction
// ---------------------------------------------------------------------------

/// A batch of steps to apply atomically to a document.
///
/// After all steps are applied sequentially, the resulting document is
/// validated against the schema. If validation fails, the entire transaction
/// is rejected.
#[derive(Debug)]
pub struct Transaction {
    pub steps: Vec<Step>,
    pub source: Source,
    pub meta: HashMap<String, serde_json::Value>,
}

impl Transaction {
    /// Create a new empty transaction with the given source.
    pub fn new(source: Source) -> Self {
        Self {
            steps: Vec::new(),
            source,
            meta: HashMap::new(),
        }
    }

    /// Append a step to this transaction. Returns `&mut Self` for chaining.
    pub fn add_step(&mut self, step: Step) -> &mut Self {
        self.steps.push(step);
        self
    }

    /// Apply all steps sequentially to `doc`, then validate the result against
    /// `schema`. Returns the new document and a composed `StepMap` on success.
    pub fn apply(
        &self,
        doc: &Document,
        schema: &Schema,
    ) -> Result<(Document, StepMap), TransformError> {
        self.apply_with_limits(doc, schema, &ResourceLimits::default())
    }

    /// Apply and validate with explicit resource limits. Callers that own an
    /// editor configuration should use those resolved limits.
    pub fn apply_with_limits(
        &self,
        doc: &Document,
        schema: &Schema,
        limits: &ResourceLimits,
    ) -> Result<(Document, StepMap), TransformError> {
        let (current, composed_map) = self.apply_steps_unchecked(doc, schema)?;
        apply::DocumentValidator::validate(&current, schema, limits).map_err(|error| {
            TransformError::ContentViolation(format!("{}: {}", error.code(), error))
        })?;
        Ok((current, composed_map))
    }

    pub(crate) fn apply_steps_unchecked(
        &self,
        doc: &Document,
        schema: &Schema,
    ) -> Result<(Document, StepMap), TransformError> {
        let mut current = doc.clone();
        let mut composed_map = StepMap::empty();

        for step in &self.steps {
            let (new_doc, step_map) = apply::apply_step(&current, step, schema)?;
            composed_map = composed_map.compose(&step_map);
            current = new_doc;
        }

        Ok((current, composed_map))
    }
}
