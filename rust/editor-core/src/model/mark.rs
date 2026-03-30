use std::collections::HashMap;

/// A mark represents inline formatting (bold, italic, link, etc.) applied to
/// a text node. Marks don't occupy positions in the document's token stream —
/// they're metadata attached to text nodes.
#[derive(Debug, Clone)]
pub struct Mark {
    mark_type: String,
    attrs: HashMap<String, serde_json::Value>,
}

impl Mark {
    /// Create a new mark with the given type name and attributes.
    pub fn new(mark_type: String, attrs: HashMap<String, serde_json::Value>) -> Self {
        Self { mark_type, attrs }
    }

    /// The mark type name (e.g. "bold", "italic", "link").
    pub fn mark_type(&self) -> &str {
        &self.mark_type
    }

    /// The mark's attributes (e.g. `{"href": "https://..."}` for a link mark).
    pub fn attrs(&self) -> &HashMap<String, serde_json::Value> {
        &self.attrs
    }
}

impl PartialEq for Mark {
    fn eq(&self, other: &Self) -> bool {
        self.mark_type == other.mark_type && self.attrs == other.attrs
    }
}

impl Eq for Mark {}
