//! Utilities for step application: node rebuilding, text splitting, and
//! mark manipulation.

use crate::model::{Fragment, Mark, Node};

/// Split a text node at `offset` (in Unicode scalars) into two text nodes.
/// Returns `(left, right)`. Either side may be empty.
pub(crate) fn split_text_node(node: &Node, offset: u32) -> (Option<Node>, Option<Node>) {
    let text_str = node
        .text_str()
        .expect("split_text_node called on non-text node");
    let chars: Vec<char> = text_str.chars().collect();
    let offset = offset as usize;

    let left = if offset > 0 {
        let left_str: String = chars[..offset].iter().collect();
        Some(Node::text(left_str, node.marks().to_vec()))
    } else {
        None
    };

    let right = if offset < chars.len() {
        let right_str: String = chars[offset..].iter().collect();
        Some(Node::text(right_str, node.marks().to_vec()))
    } else {
        None
    };

    (left, right)
}

/// Check if two sets of marks are equal (same types and attrs, order-insensitive).
pub(crate) fn marks_eq(a: &[Mark], b: &[Mark]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // Since marks are a small set, O(n^2) is fine.
    a.iter().all(|ma| b.iter().any(|mb| ma == mb))
}

/// Merge adjacent text nodes that share the same marks. This produces a
/// minimal set of text nodes.
pub(crate) fn merge_adjacent_text_nodes(nodes: Vec<Node>) -> Vec<Node> {
    let mut result: Vec<Node> = Vec::with_capacity(nodes.len());

    for node in nodes {
        if !node.is_text() {
            result.push(node);
            continue;
        }

        // Skip empty text nodes.
        if node.text_str().map_or(false, |s| s.is_empty()) {
            continue;
        }

        if let Some(last) = result.last() {
            if last.is_text() && marks_eq(last.marks(), node.marks()) {
                // Merge with the previous text node.
                let merged_text =
                    format!("{}{}", last.text_str().unwrap(), node.text_str().unwrap());
                let merged = Node::text(merged_text, last.marks().to_vec());
                let len = result.len();
                result[len - 1] = merged;
                continue;
            }
        }

        result.push(node);
    }

    result
}

/// Rebuild a parent element node with new children (creating a new Fragment).
pub(crate) fn rebuild_element(parent: &Node, new_children: Vec<Node>) -> Node {
    Node::element(
        parent.node_type().to_string(),
        parent.attrs().clone(),
        Fragment::from(new_children),
    )
}

/// Add a mark to a mark set if not already present.
pub(crate) fn add_mark_to_set(marks: &[Mark], mark: &Mark) -> Vec<Mark> {
    let mut result: Vec<Mark> = marks.to_vec();
    if !result.iter().any(|m| m == mark) {
        result.push(mark.clone());
    }
    result
}

/// Remove all marks of a given type from a mark set.
pub(crate) fn remove_mark_from_set(marks: &[Mark], mark_type: &str) -> Vec<Mark> {
    marks
        .iter()
        .filter(|m| m.mark_type() != mark_type)
        .cloned()
        .collect()
}
