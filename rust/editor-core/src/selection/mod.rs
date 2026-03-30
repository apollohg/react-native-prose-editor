pub mod normalize;

use crate::model::Document;
use crate::position::PositionMap;
use crate::transform::StepMap;

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

/// Represents a selection within the document.
///
/// Three variants matching ProseMirror's selection model:
///
/// - `Text` — A text cursor or range. When `anchor == head`, it is a cursor
///   (collapsed selection). Both positions must be cursorable doc positions.
///
/// - `Node` — Selects an entire void node (e.g. horizontalRule). The `pos` is
///   the doc position of the void node.
///
/// - `All` — Selects the entire document content (Cmd+A). Lazily resolves to
///   `0..doc.content_size()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Text cursor or range selection.
    Text {
        /// The fixed end of the selection (where the user started dragging).
        anchor: u32,
        /// The moving end (where the cursor currently is).
        head: u32,
    },
    /// Selects a single void/atom node at the given doc position.
    Node {
        /// Doc position of the void node.
        pos: u32,
    },
    /// Selects the entire document content.
    All,
}

impl Selection {
    // -----------------------------------------------------------------------
    // Constructors
    // -----------------------------------------------------------------------

    /// Create a collapsed text selection (cursor) at `pos`.
    pub fn cursor(pos: u32) -> Self {
        Self::Text {
            anchor: pos,
            head: pos,
        }
    }

    /// Create a text range selection from `anchor` to `head`.
    pub fn text(anchor: u32, head: u32) -> Self {
        Self::Text { anchor, head }
    }

    /// Create a node selection at `pos`.
    pub fn node(pos: u32) -> Self {
        Self::Node { pos }
    }

    /// Create an all-document selection.
    pub fn all() -> Self {
        Self::All
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// The anchor position. For `All`, resolves to `0`.
    pub fn anchor(&self, _doc: &Document) -> u32 {
        match self {
            Self::Text { anchor, .. } => *anchor,
            Self::Node { pos } => *pos,
            Self::All => 0,
        }
    }

    /// The head position. For `All`, resolves to `doc.content_size()`.
    pub fn head(&self, doc: &Document) -> u32 {
        match self {
            Self::Text { head, .. } => *head,
            Self::Node { pos } => *pos,
            Self::All => doc.content_size(),
        }
    }

    /// The start of the selection range: `min(anchor, head)`.
    pub fn from(&self, doc: &Document) -> u32 {
        let a = self.anchor(doc);
        let h = self.head(doc);
        a.min(h)
    }

    /// The end of the selection range: `max(anchor, head)`.
    pub fn to(&self, doc: &Document) -> u32 {
        let a = self.anchor(doc);
        let h = self.head(doc);
        a.max(h)
    }

    /// Whether the selection is collapsed (anchor == head).
    ///
    /// `Node` selections are never empty (they select one node).
    /// `All` is empty only if the document has no content.
    pub fn is_empty(&self, doc: &Document) -> bool {
        self.anchor(doc) == self.head(doc)
    }

    // -----------------------------------------------------------------------
    // Normalization
    // -----------------------------------------------------------------------

    /// Normalize the selection by snapping all positions to the nearest
    /// cursorable position using the `PositionMap`.
    pub fn normalized(self, doc: &Document, pos_map: &PositionMap) -> Self {
        normalize::normalize_selection(self, doc, pos_map)
    }

    // -----------------------------------------------------------------------
    // Mapping through StepMap
    // -----------------------------------------------------------------------

    /// Map this selection through a `StepMap` to get new positions after a
    /// document transformation.
    ///
    /// `All` stays `All` since it is document-relative and always resolves
    /// lazily.
    pub fn map(&self, step_map: &StepMap) -> Self {
        match self {
            Self::Text { anchor, head } => Self::Text {
                anchor: step_map.map_pos(*anchor),
                head: step_map.map_pos(*head),
            },
            Self::Node { pos } => Self::Node {
                pos: step_map.map_pos(*pos),
            },
            Self::All => Self::All,
        }
    }
}
