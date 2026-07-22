use std::collections::HashMap;

/// A mark represents inline formatting (bold, italic, link, etc.) applied to
/// a text node. Marks don't occupy positions in the document's token stream —
/// they're metadata attached to text nodes.
#[derive(Debug)]
pub struct Mark {
    mark_type: String,
    attrs: HashMap<String, serde_json::Value>,
}

impl Clone for Mark {
    fn clone(&self) -> Self {
        Self {
            mark_type: self.mark_type.clone(),
            attrs: crate::boundary::clone_json_object_stack_safe(&self.attrs),
        }
    }
}

impl Drop for Mark {
    fn drop(&mut self) {
        for value in self.attrs.values_mut() {
            crate::boundary::drop_json_value_stack_safe(std::mem::take(value));
        }
    }
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

    pub(crate) fn history_snapshot_clone_retained_bytes(&self) -> Option<usize> {
        let table = crate::model::hash_table_retained_bytes::<String, serde_json::Value>(
            self.attrs.capacity(),
        )?;
        self.attrs.iter().try_fold(
            self.mark_type.capacity().checked_add(table)?,
            |total, (key, value)| {
                total
                    .checked_add(key.capacity())?
                    .checked_add(crate::model::json_value_retained_bytes(value)?)
            },
        )
    }
}

impl PartialEq for Mark {
    fn eq(&self, other: &Self) -> bool {
        self.mark_type == other.mark_type
            && crate::boundary::json_objects_equal_stack_safe(&self.attrs, &other.attrs)
    }
}

impl Eq for Mark {}
