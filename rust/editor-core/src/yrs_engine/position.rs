use yrs::branch::{Branch, BranchPtr};
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef};
use yrs::{Any, Assoc, ReadTxn, StickyIndex};

use crate::model::Document;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;

use super::{Affinity, EditorOffsetKind, RevisionedPosition};

#[cfg(test)]
std::thread_local! {
    static RELATIVE_FULL_SIZE_PREPASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RELATIVE_FORWARD_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RELATIVE_REVERSE_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_relative_position_traversal_counts_for_test() {
    RELATIVE_FULL_SIZE_PREPASSES.set(0);
    RELATIVE_FORWARD_TRAVERSALS.set(0);
    RELATIVE_REVERSE_TRAVERSALS.set(0);
}

#[cfg(test)]
pub(crate) fn take_relative_position_traversal_counts_for_test() -> (usize, usize, usize) {
    (
        RELATIVE_FULL_SIZE_PREPASSES.replace(0),
        RELATIVE_FORWARD_TRAVERSALS.replace(0),
        RELATIVE_REVERSE_TRAVERSALS.replace(0),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativePoint {
    pub sticky: StickyIndex,
    pub affinity: Affinity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelativeSelection {
    Text {
        anchor: RelativePoint,
        head: RelativePoint,
    },
    Node {
        point: RelativePoint,
    },
    All,
}

pub fn doc_pos_to_relative_point<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    affinity: Affinity,
    schema: &Schema,
) -> Option<RelativePoint> {
    let sticky = doc_pos_to_sticky_index(txn, fragment, doc_pos, affinity.into(), schema)?;
    Some(RelativePoint { sticky, affinity })
}

/// Materialize a relative point after the caller has admitted the complete
/// ready-document scan budget and resolved `doc_pos` against the certified
/// derived document. The forward traversal itself proves the position exists,
/// so it deliberately avoids the general helper's full-fragment size prepass.
pub(crate) fn admitted_doc_pos_to_relative_point<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    affinity: Affinity,
    schema: &Schema,
) -> Option<RelativePoint> {
    let assoc = affinity.into();
    let sticky = forward_doc_pos_to_sticky_index(txn, fragment, doc_pos, assoc, schema)?;
    sticky.get_offset(txn)?;
    (sticky.assoc == assoc).then_some(RelativePoint { sticky, affinity })
}

pub fn relative_point_to_doc_pos<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    point: &RelativePoint,
    schema: &Schema,
) -> Option<u32> {
    sticky_index_to_doc_pos(txn, fragment, &point.sticky, schema)
}

pub fn relative_selection_to_selection<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    relative: &RelativeSelection,
    schema: &Schema,
    document: &Document,
    position_map: &PositionMap,
) -> Option<Selection> {
    let selection = match relative {
        RelativeSelection::Text { anchor, head } => Selection::text(
            relative_point_to_doc_pos(txn, fragment, anchor, schema)?,
            relative_point_to_doc_pos(txn, fragment, head, schema)?,
        ),
        RelativeSelection::Node { point } => {
            Selection::node(relative_point_to_doc_pos(txn, fragment, point, schema)?)
        }
        RelativeSelection::All => Selection::all(),
    };
    Some(selection.normalized(document, position_map))
}

pub fn revisioned_position_to_relative_point<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    position: RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
    schema: &Schema,
) -> Option<RelativePoint> {
    let doc_pos = editor_offset_to_doc_pos(
        position.offset,
        position.kind,
        rendered_text,
        position_map,
        document,
    )?;
    doc_pos_to_relative_point(txn, fragment, doc_pos, position.affinity, schema)
}

pub(crate) fn editor_offset_to_doc_pos(
    offset: u32,
    kind: EditorOffsetKind,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
) -> Option<u32> {
    let scalar_offset = editor_offset_to_scalar(offset, kind, rendered_text, position_map)?;
    Some(position_map.scalar_to_doc(scalar_offset, document))
}

pub(crate) fn editor_offset_to_scalar(
    offset: u32,
    kind: EditorOffsetKind,
    rendered_text: &str,
    position_map: &PositionMap,
) -> Option<u32> {
    let scalar_offset = match kind {
        EditorOffsetKind::Scalar => offset,
        EditorOffsetKind::Utf16 => utf16_offset_to_scalar(rendered_text, offset)?,
    };
    (scalar_offset <= position_map.total_scalars()).then_some(scalar_offset)
}

pub(crate) fn sticky_index_to_doc_pos<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    sticky_index: &StickyIndex,
    schema: &Schema,
) -> Option<u32> {
    #[cfg(test)]
    RELATIVE_REVERSE_TRAVERSALS.set(RELATIVE_REVERSE_TRAVERSALS.get().saturating_add(1));
    let offset = sticky_index.get_offset(txn)?;
    let root_branch = BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(fragment));
    if offset.branch == root_branch {
        return sequence_branch_index_to_doc_pos(txn, fragment.children(txn), offset.index, schema);
    }
    let mut child_start = 0u32;
    for child in fragment.children(txn) {
        if let Some(position) = sticky_index_to_doc_pos_in_node(
            txn,
            &child,
            offset.branch,
            offset.index,
            child_start,
            schema,
        ) {
            return Some(position);
        }
        child_start = child_start.checked_add(xml_out_pm_size(txn, &child, schema)?)?;
    }
    None
}

fn sticky_index_to_doc_pos_in_node<T: ReadTxn>(
    txn: &T,
    node: &XmlOut,
    target_branch: BranchPtr,
    target_index: u32,
    node_start: u32,
    schema: &Schema,
) -> Option<u32> {
    match node {
        XmlOut::Text(text) => {
            let text_branch = BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(text));
            if text_branch == target_branch {
                let text_value = xml_text_plain_string(text, txn)?;
                let scalar_offset = utf16_offset_to_scalar(&text_value, target_index)?;
                return Some(node_start + scalar_offset);
            }
            None
        }
        XmlOut::Element(element) => {
            if is_void_element(element, txn, schema) {
                return None;
            }
            let element_branch = BranchPtr::from(<XmlElementRef as AsRef<Branch>>::as_ref(element));
            if element_branch == target_branch {
                let content_start = node_start + 1;
                return sequence_branch_index_to_doc_pos(
                    txn,
                    element.children(txn),
                    target_index,
                    schema,
                )
                .map(|value| content_start + value);
            }

            let mut child_start = node_start + 1;
            for child in element.children(txn) {
                if let Some(position) = sticky_index_to_doc_pos_in_node(
                    txn,
                    &child,
                    target_branch,
                    target_index,
                    child_start,
                    schema,
                ) {
                    return Some(position);
                }
                child_start = child_start.checked_add(xml_out_pm_size(txn, &child, schema)?)?;
            }
            None
        }
        XmlOut::Fragment(fragment) => {
            let fragment_branch =
                BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(fragment));
            if fragment_branch == target_branch {
                return sequence_branch_index_to_doc_pos(
                    txn,
                    fragment.children(txn),
                    target_index,
                    schema,
                )
                .map(|value| node_start + value);
            }

            let mut child_start = node_start;
            for child in fragment.children(txn) {
                if let Some(position) = sticky_index_to_doc_pos_in_node(
                    txn,
                    &child,
                    target_branch,
                    target_index,
                    child_start,
                    schema,
                ) {
                    return Some(position);
                }
                child_start = child_start.checked_add(xml_out_pm_size(txn, &child, schema)?)?;
            }
            None
        }
    }
}

fn sequence_branch_index_to_doc_pos<'a, T: ReadTxn>(
    txn: &T,
    children: impl Iterator<Item = XmlOut> + 'a,
    target_index: u32,
    schema: &Schema,
) -> Option<u32> {
    let mut branch_index = 0u32;
    let mut doc_pos = 0u32;

    for child in children {
        match &child {
            XmlOut::Text(text) => {
                let text_value = xml_text_plain_string(text, txn)?;
                let text_scalar_len = scalar_len(&text_value);
                if target_index == branch_index {
                    return Some(doc_pos);
                }
                branch_index += 1;
                doc_pos += text_scalar_len;
                if target_index == branch_index {
                    return Some(doc_pos);
                }
            }
            XmlOut::Element(_) | XmlOut::Fragment(_) => {
                if target_index == branch_index {
                    return Some(doc_pos);
                }
                branch_index += 1;
                doc_pos = doc_pos.checked_add(xml_out_pm_size(txn, &child, schema)?)?;
                if target_index == branch_index {
                    return Some(doc_pos);
                }
            }
        }
    }

    if target_index == branch_index {
        Some(doc_pos)
    } else {
        None
    }
}

pub(crate) fn doc_pos_to_sticky_index<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    assoc: Assoc,
    schema: &Schema,
) -> Option<StickyIndex> {
    #[cfg(test)]
    RELATIVE_FULL_SIZE_PREPASSES.set(RELATIVE_FULL_SIZE_PREPASSES.get().saturating_add(1));
    let content_size = xml_fragment_pm_content_size(txn, fragment, schema)?;
    if doc_pos > content_size {
        return None;
    }
    forward_doc_pos_to_sticky_index(txn, fragment, doc_pos, assoc, schema)
}

fn forward_doc_pos_to_sticky_index<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    assoc: Assoc,
    schema: &Schema,
) -> Option<StickyIndex> {
    #[cfg(test)]
    RELATIVE_FORWARD_TRAVERSALS.set(RELATIVE_FORWARD_TRAVERSALS.get().saturating_add(1));
    doc_pos_to_sticky_index_in_sequence(
        txn,
        fragment.children(txn),
        doc_pos,
        assoc,
        BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(fragment)),
        schema,
    )
}

pub(crate) fn cursor_sticky_index_from_doc_pos<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    doc_pos: u32,
    collapsed: bool,
    schema: &Schema,
) -> Option<StickyIndex> {
    if !collapsed {
        return doc_pos_to_sticky_index(txn, fragment, doc_pos, Assoc::Before, schema);
    }

    doc_pos_to_sticky_index(txn, fragment, doc_pos, Assoc::After, schema)
        .or_else(|| doc_pos_to_sticky_index(txn, fragment, doc_pos, Assoc::Before, schema))
}

fn doc_pos_to_sticky_index_in_sequence<'a, T: ReadTxn>(
    txn: &T,
    children: impl Iterator<Item = XmlOut> + 'a,
    doc_pos: u32,
    assoc: Assoc,
    branch: BranchPtr,
    schema: &Schema,
) -> Option<StickyIndex> {
    let mut branch_index = 0u32;
    let mut consumed_pm = 0u32;
    let mut children = children.peekable();

    while let Some(child) = children.next() {
        match &child {
            XmlOut::Text(text) => {
                let text_value = xml_text_plain_string(text, txn)?;
                let text_scalar_len = scalar_len(&text_value);
                let mut retry_adjacent_text = false;
                if doc_pos <= consumed_pm + text_scalar_len {
                    let utf16_offset = scalar_offset_to_utf16(&text_value, doc_pos - consumed_pm)?;
                    if let Some(sticky) = StickyIndex::at(
                        txn,
                        BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(text)),
                        utf16_offset,
                        assoc,
                    ) {
                        return Some(sticky);
                    }
                    if doc_pos < consumed_pm + text_scalar_len {
                        return None;
                    }
                    retry_adjacent_text = true;
                }
                branch_index += 1;
                consumed_pm += text_scalar_len;
                if retry_adjacent_text && !matches!(children.peek(), Some(XmlOut::Text(_))) {
                    return None;
                }
            }
            XmlOut::Element(element) => {
                let child_size = xml_out_pm_size(txn, &child, schema)?;
                if doc_pos == consumed_pm {
                    return StickyIndex::at(txn, branch, branch_index, assoc);
                }
                if doc_pos < consumed_pm + child_size {
                    return doc_pos_to_sticky_index_in_sequence(
                        txn,
                        element.children(txn),
                        doc_pos - consumed_pm - 1,
                        assoc,
                        BranchPtr::from(<XmlElementRef as AsRef<Branch>>::as_ref(element)),
                        schema,
                    );
                }
                branch_index += 1;
                consumed_pm += child_size;
            }
            XmlOut::Fragment(nested) => {
                let child_size = xml_out_pm_size(txn, &child, schema)?;
                if doc_pos == consumed_pm {
                    return StickyIndex::at(txn, branch, branch_index, assoc);
                }
                if doc_pos < consumed_pm + child_size {
                    return doc_pos_to_sticky_index_in_sequence(
                        txn,
                        nested.children(txn),
                        doc_pos - consumed_pm,
                        assoc,
                        BranchPtr::from(<XmlFragmentRef as AsRef<Branch>>::as_ref(nested)),
                        schema,
                    );
                }
                branch_index += 1;
                consumed_pm += child_size;
            }
        }
    }

    if doc_pos == consumed_pm {
        StickyIndex::at(txn, branch, branch_index, assoc)
    } else {
        None
    }
}

fn xml_fragment_pm_content_size<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
) -> Option<u32> {
    fragment.children(txn).try_fold(0u32, |size, child| {
        size.checked_add(xml_out_pm_size(txn, &child, schema)?)
    })
}

fn is_void_element<T: ReadTxn>(element: &XmlElementRef, txn: &T, schema: &Schema) -> bool {
    let node_type = super::codec::normalized_wire_element_node_type(element, txn);
    if matches!(node_type.as_str(), "__opaque" | "__opaque_json" | "__skip") {
        return true;
    }
    if let Some(spec) = schema.node(&node_type) {
        return spec.is_void;
    }
    true
}

fn xml_out_pm_size<T: ReadTxn>(txn: &T, node: &XmlOut, schema: &Schema) -> Option<u32> {
    match node {
        XmlOut::Text(text) => Some(scalar_len(&xml_text_plain_string(text, txn)?)),
        XmlOut::Element(element) => {
            if is_void_element(element, txn, schema) {
                Some(1)
            } else {
                element.children(txn).try_fold(2u32, |size, child| {
                    size.checked_add(xml_out_pm_size(txn, &child, schema)?)
                })
            }
        }
        XmlOut::Fragment(fragment) => fragment.children(txn).try_fold(0u32, |size, child| {
            size.checked_add(xml_out_pm_size(txn, &child, schema)?)
        }),
    }
}

fn xml_text_plain_string<T: ReadTxn>(text: &XmlTextRef, txn: &T) -> Option<String> {
    let mut value = String::new();
    for diff in text.diff(txn, YChange::identity) {
        let yrs::Out::Any(Any::String(run)) = diff.insert else {
            return None;
        };
        value.push_str(&run);
    }
    Some(value)
}

fn scalar_len(value: &str) -> u32 {
    value.chars().count() as u32
}

pub fn scalar_offset_to_utf16(value: &str, scalar_offset: u32) -> Option<u32> {
    let mut scalar_count = 0u32;
    let mut utf16_count = 0u32;
    if scalar_offset == 0 {
        return Some(0);
    }
    for character in value.chars() {
        scalar_count += 1;
        utf16_count += character.len_utf16() as u32;
        if scalar_count == scalar_offset {
            return Some(utf16_count);
        }
    }
    None
}

pub fn utf16_offset_to_scalar(value: &str, utf16_offset: u32) -> Option<u32> {
    let mut scalar_count = 0u32;
    let mut utf16_count = 0u32;
    if utf16_offset == 0 {
        return Some(0);
    }
    for character in value.chars() {
        scalar_count += 1;
        utf16_count += character.len_utf16() as u32;
        if utf16_count == utf16_offset {
            return Some(scalar_count);
        }
        if utf16_count > utf16_offset {
            return None;
        }
    }
    None
}

impl From<Affinity> for Assoc {
    fn from(value: Affinity) -> Self {
        match value {
            Affinity::Before => Self::Before,
            Affinity::After => Self::After,
        }
    }
}
