use crate::intercept::{InterceptError, Interceptor};
use crate::model::Document;
use crate::transform::{Source, Step, Transaction};

/// Aborts transactions that would cause the document's text content to exceed
/// a maximum character count.
///
/// Computes the actual visible text delta for each step (not doc positions,
/// which include structural tokens). Applies to sources: Input, Paste, Api,
/// Reconciliation.
pub struct MaxLength {
    max: u32,
}

/// Sources that MaxLength applies to.
const MAX_LENGTH_SOURCES: &[Source] = &[
    Source::Input,
    Source::Paste,
    Source::Api,
    Source::Reconciliation,
];

impl MaxLength {
    /// Create a new MaxLength interceptor with the given character limit.
    pub fn new(max: u32) -> Self {
        Self { max }
    }
}

impl Interceptor for MaxLength {
    fn intercept(&self, tx: Transaction, doc: &Document) -> Result<Transaction, InterceptError> {
        let current_len = doc.root().text_content().chars().count() as u32;
        let mut projected_len = current_len;

        for step in &tx.steps {
            match step {
                Step::InsertText { text, .. } => {
                    projected_len += text.chars().count() as u32;
                }
                Step::ReplaceRange { from, to, content } => {
                    // Compute the actual text characters removed by extracting
                    // text content from the document range, not using doc positions
                    // (which include structural tokens).
                    let removed_text_len = extract_text_len_in_range(doc, *from, *to);
                    let added_text_len = content_text_len(content);
                    if added_text_len > removed_text_len {
                        projected_len += added_text_len - removed_text_len;
                    } else {
                        projected_len =
                            projected_len.saturating_sub(removed_text_len - added_text_len);
                    }
                }
                Step::DeleteRange { from, to } => {
                    // Compute actual text characters removed, not doc positions.
                    let removed_text_len = extract_text_len_in_range(doc, *from, *to);
                    projected_len = projected_len.saturating_sub(removed_text_len);
                }
                // All other steps (AddMark, RemoveMark, SplitBlock, etc.)
                // don't change text content length.
                _ => {}
            }

            // Reject only if the projected length exceeds the max AND the
            // transaction is making the document longer than it currently is.
            // This allows delete-only transactions to pass even when the
            // document already exceeds the limit.
            if projected_len > self.max && projected_len > current_len {
                return Err(InterceptError::new(format!(
                    "transaction would exceed max length ({} > {})",
                    projected_len, self.max
                )));
            }
        }

        Ok(tx)
    }

    fn sources(&self) -> &[Source] {
        MAX_LENGTH_SOURCES
    }
}

/// Recursively compute the text character count of a Fragment's content.
fn content_text_len(fragment: &crate::model::Fragment) -> u32 {
    let mut len = 0u32;
    for i in 0..fragment.child_count() {
        if let Some(child) = fragment.child(i) {
            len += child.text_content().chars().count() as u32;
        }
    }
    len
}

/// Extract the visible text character count from a document range [from, to).
///
/// Unlike `to - from` (which counts doc tokens including structural tokens),
/// this counts only actual text characters within the range.
fn extract_text_len_in_range(doc: &Document, from: u32, to: u32) -> u32 {
    if from >= to {
        return 0;
    }

    // Walk the document tree from `from` to `to`, counting text characters.
    // For ranges within a single parent, this is straightforward.
    // For cross-parent ranges, walk the full range.
    let root = doc.root();
    count_text_in_range(root, 0, from, to)
}

/// Recursively count text characters within a document range [from, to),
/// where positions are relative to the document content.
fn count_text_in_range(
    node: &crate::model::Node,
    node_content_start: u32,
    from: u32,
    to: u32,
) -> u32 {
    if node.is_text() {
        // Text node: its content occupies [node_content_start, node_content_start + size).
        let node_end = node_content_start + node.node_size();
        let overlap_start = from.max(node_content_start);
        let overlap_end = to.min(node_end);
        if overlap_start < overlap_end {
            return overlap_end - overlap_start;
        }
        return 0;
    }

    if node.is_void() {
        // Void nodes have no text content.
        return 0;
    }

    // Element node: content starts at node_content_start (for the root, we
    // don't add an open tag token; for children, the caller accounts for it).
    let content = match node.content() {
        Some(c) => c,
        None => return 0,
    };

    let mut count = 0u32;
    let mut offset = node_content_start;

    for child in content.iter() {
        let child_size = child.node_size();
        let child_start = offset;
        let child_end = offset + child_size;

        // Skip children entirely outside the range.
        if child_end <= from || child_start >= to {
            offset = child_end;
            continue;
        }

        if child.is_text() {
            count += count_text_in_range(child, child_start, from, to);
        } else if child.is_void() {
            // No text content
        } else {
            // Element child: content starts after open tag
            let child_content_start = child_start + 1;
            count += count_text_in_range(child, child_content_start, from, to);
        }

        offset = child_end;
    }

    count
}
