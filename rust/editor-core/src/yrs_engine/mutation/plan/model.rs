use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yrs::any::Any;
use yrs::branch::{Branch, BranchID, BranchPtr};
use yrs::types::text::Text;
use yrs::types::xml::{
    Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef,
};
use yrs::types::xml::XmlTextPrelim;
use yrs::types::Attrs;
use yrs::ReadTxn;
use yrs::Snapshot;
use yrs::StickyIndex;
use yrs::TransactionMut;

use super::super::codec::insert_prepared_node;
use super::super::codec::{PreparedXmlChild, PreparedXmlNode};
use super::super::{OperationError, OperationResult};

#[derive(Debug, Clone)]
pub(crate) enum XmlParentRef {
    Fragment(XmlFragmentRef),
    Element(XmlElementRef),
}

impl XmlParentRef {
    pub(super) fn id(&self) -> BranchID {
        match self {
            Self::Fragment(parent) => AsRef::<Branch>::as_ref(parent).id(),
            Self::Element(parent) => AsRef::<Branch>::as_ref(parent).id(),
        }
    }

    fn children<T: ReadTxn>(&self, txn: &T) -> Vec<BranchID> {
        match self {
            Self::Fragment(parent) => parent.children(txn).map(|child| child.id()).collect(),
            Self::Element(parent) => parent.children(txn).map(|child| child.id()).collect(),
        }
    }

    #[allow(dead_code)] // Production execution is consumed by the Task 7 engine boundary.
    fn remove_range(&self, txn: &mut TransactionMut<'_>, index: u32, len: u32) {
        match self {
            Self::Fragment(parent) => parent.remove_range(txn, index, len),
            Self::Element(parent) => parent.remove_range(txn, index, len),
        }
    }

    #[allow(dead_code)] // Production execution is consumed by the Task 7 engine boundary.
    fn insert_prepared(&self, txn: &mut TransactionMut<'_>, index: u32, node: PreparedXmlNode) {
        match self {
            Self::Fragment(parent) => insert_prepared_node(parent, txn, index, node),
            Self::Element(parent) => insert_prepared_node(parent, txn, index, node),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructuralParentSignature {
    pub(super) parent: BranchID,
    pub(super) path: Vec<(BranchID, u32)>,
    pub(super) children: Vec<BranchID>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextSignatureRun {
    pub(super) text: String,
    pub(super) attrs: Vec<(Arc<str>, Any)>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TargetSignature {
    pub(super) target: BranchID,
    pub(super) path: Vec<(BranchID, u32)>,
    pub(super) initial_len_utf16: u32,
    pub(super) runs: Vec<TextSignatureRun>,
    pub(super) capture_work: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ElementSignature {
    pub(super) target: BranchID,
    pub(super) path: Vec<(BranchID, u32)>,
    pub(super) tag: Arc<str>,
    pub(super) attrs: Vec<(Arc<str>, Any)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParentSignature {
    pub(super) parent: BranchID,
    pub(super) tag: Arc<str>,
    pub(super) path: Vec<(BranchID, u32)>,
    pub(super) child_count: u32,
    pub(super) initial_child_index: u32,
    pub(super) left_neighbor: Option<BranchID>,
    pub(super) right_neighbor: Option<BranchID>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct YrsMutationPlan {
    pub actions: Vec<YrsMutationAction>,
    pub(super) compilation_work: usize,
    pub(super) expected_preflight_work: usize,
    pub(super) work_limit: usize,
    pub(super) document_guard: Option<DocumentGuard>,
    pub(super) prepared_metrics: Vec<Option<PreparedActionMetrics>>,
    pub(crate) scan_work: usize,
    #[cfg(test)]
    pub(crate) position_resolver_work: usize,
}

#[derive(Debug, Clone)]
pub(super) struct DocumentGuard {
    store_token: usize,
    snapshot: Snapshot,
    state_clock_work: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PreparedActionMetrics {
    growth_bytes: usize,
    insertion_units: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CrdtEnvelope {
    pub(crate) live_clock_units: u64,
    pub(crate) client_count: usize,
    pub(crate) scan_work: usize,
}

impl YrsMutationPlan {
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub(crate) fn requires_crdt_envelope(&self) -> bool {
        plan_may_delete_live(self)
    }

    /// Returns whether this point is contained by a base-document XML branch
    /// that the plan removes. Text tombstones stay attached to their live XML
    /// branch, so they do not need a freshly materialized affinity fallback.
    pub(crate) fn removes_sticky_branch<T: ReadTxn>(
        &self,
        txn: &T,
        fragment: &XmlFragmentRef,
        sticky: &StickyIndex,
    ) -> bool {
        let Some(offset) = sticky.get_offset(txn) else {
            return true;
        };
        let Some(deleted_roots) = self.deleted_base_branch_roots(txn) else {
            return true;
        };
        if deleted_roots.is_empty() {
            return false;
        }
        fragment.children(txn).any(|child| {
            branch_is_deleted_or_descendant(
                txn,
                child,
                offset.branch,
                &deleted_roots,
                false,
            )
        })
    }

    fn deleted_base_branch_roots<T: ReadTxn>(&self, txn: &T) -> Option<HashSet<BranchID>> {
        let mut virtual_children = HashMap::<BranchID, Vec<Option<BranchID>>>::new();
        let mut deleted = HashSet::new();
        for action in &self.actions {
            match action {
                YrsMutationAction::DeleteXmlChildren {
                    parent,
                    child_index,
                    child_count,
                    ..
                } => {
                    let children = virtual_children.entry(parent.id()).or_insert_with(|| {
                        parent.children(txn).into_iter().map(Some).collect()
                    });
                    let start = usize::try_from(*child_index).ok()?;
                    let count = usize::try_from(*child_count).ok()?;
                    let end = start.checked_add(count)?;
                    if end > children.len() || count == 0 {
                        return None;
                    }
                    for branch in children.drain(start..end).flatten() {
                        deleted.insert(branch);
                    }
                }
                YrsMutationAction::InsertXmlChildren {
                    parent,
                    child_index,
                    nodes,
                    ..
                } => {
                    let children = virtual_children.entry(parent.id()).or_insert_with(|| {
                        parent.children(txn).into_iter().map(Some).collect()
                    });
                    let index = usize::try_from(*child_index).ok()?;
                    if index > children.len() || nodes.is_empty() {
                        return None;
                    }
                    children.splice(index..index, std::iter::repeat_n(None, nodes.len()));
                }
                YrsMutationAction::CreateText {
                    parent,
                    child_index,
                    ..
                } => {
                    let parent_id = AsRef::<Branch>::as_ref(parent).id();
                    let children = virtual_children.entry(parent_id).or_insert_with(|| {
                        parent.children(txn).map(|child| Some(child.id())).collect()
                    });
                    let index = usize::try_from(*child_index).ok()?;
                    if index > children.len() {
                        return None;
                    }
                    children.insert(index, None);
                }
                _ => {}
            }
        }
        Some(deleted)
    }
}

fn branch_is_deleted_or_descendant<T: ReadTxn>(
    txn: &T,
    node: XmlOut,
    target: BranchPtr,
    deleted_roots: &HashSet<BranchID>,
    ancestor_deleted: bool,
) -> bool {
    let (branch, children): (&Branch, Option<Box<dyn Iterator<Item = XmlOut> + '_>>) = match &node {
        XmlOut::Text(text) => (AsRef::<Branch>::as_ref(text), None),
        XmlOut::Element(element) => (
            AsRef::<Branch>::as_ref(element),
            Some(Box::new(element.children(txn))),
        ),
        XmlOut::Fragment(fragment) => (
            AsRef::<Branch>::as_ref(fragment),
            Some(Box::new(fragment.children(txn))),
        ),
    };
    let deleted = ancestor_deleted || deleted_roots.contains(&branch.id());
    if BranchPtr::from(branch) == target {
        return deleted;
    }
    children.is_some_and(|children| {
        children.into_iter().any(|child| {
            branch_is_deleted_or_descendant(txn, child, target, deleted_roots, deleted)
        })
    })
}

#[cfg(test)]
impl YrsMutationPlan {
    pub(crate) fn compilation_work_for_test(&self) -> usize {
        self.compilation_work
    }

    pub(crate) fn expected_preflight_work_for_test(&self) -> usize {
        self.expected_preflight_work
    }

    pub(crate) fn position_resolver_work_for_test(&self) -> usize {
        self.position_resolver_work
    }

    pub(crate) fn set_work_limit_for_test(&mut self, limit: usize) {
        self.work_limit = limit;
    }

    pub(crate) fn single_action_for_test(action: YrsMutationAction) -> Self {
        Self {
            actions: vec![action],
            compilation_work: 0,
            expected_preflight_work: 0,
            work_limit: usize::MAX,
            document_guard: None,
            prepared_metrics: Vec::new(),
            scan_work: 0,
            position_resolver_work: 0,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)] // Names mirror the admitted typed operation vocabulary.
pub(crate) enum YrsMutationAction {
    DeleteXmlChildren {
        parent: XmlParentRef,
        child_index: u32,
        child_count: u32,
        signature: Arc<StructuralParentSignature>,
        operation_index: usize,
    },
    InsertXmlChildren {
        parent: XmlParentRef,
        child_index: u32,
        nodes: Vec<PreparedXmlChild>,
        signature: Arc<StructuralParentSignature>,
        operation_index: usize,
    },
    SetXmlAttribute {
        target: XmlElementRef,
        key: Arc<str>,
        value: Any,
        signature: Arc<ElementSignature>,
        operation_index: usize,
    },
    RemoveXmlAttribute {
        target: XmlElementRef,
        key: Arc<str>,
        signature: Arc<ElementSignature>,
        operation_index: usize,
    },
    CreateText {
        parent: XmlElementRef,
        child_index: u32,
        text: String,
        scalar_len: u32,
        len_utf16: u32,
        attrs: Attrs,
        follow_up: Vec<CreatedTextAction>,
        signature: ParentSignature,
        operation_index: usize,
    },
    InsertText {
        target: XmlTextRef,
        index_utf16: u32,
        text: String,
        len_utf16: u32,
        attrs: Attrs,
        signature: TargetSignature,
        operation_index: usize,
    },
    DeleteText {
        target: XmlTextRef,
        index_utf16: u32,
        len_utf16: u32,
        signature: TargetSignature,
        operation_index: usize,
    },
    FormatText {
        target: XmlTextRef,
        index_utf16: u32,
        len_utf16: u32,
        attrs: Attrs,
        signature: TargetSignature,
        operation_index: usize,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum CreatedTextAction {
    Insert {
        index_utf16: u32,
        text: String,
        len_utf16: u32,
        attrs: Attrs,
        operation_index: usize,
    },
    Delete {
        index_utf16: u32,
        len_utf16: u32,
        operation_index: usize,
    },
    Format {
        index_utf16: u32,
        len_utf16: u32,
        attrs: Attrs,
        operation_index: usize,
    },
}

impl CreatedTextAction {
    fn operation_index(&self) -> usize {
        match self {
            Self::Insert {
                operation_index, ..
            }
            | Self::Delete {
                operation_index, ..
            }
            | Self::Format {
                operation_index, ..
            } => *operation_index,
        }
    }
}

impl YrsMutationAction {
    fn target(&self) -> &XmlTextRef {
        match self {
            Self::CreateText { .. }
            | Self::DeleteXmlChildren { .. }
            | Self::InsertXmlChildren { .. }
            | Self::SetXmlAttribute { .. }
            | Self::RemoveXmlAttribute { .. } => {
                unreachable!("non-text actions do not have a text target")
            }
            Self::InsertText { target, .. }
            | Self::DeleteText { target, .. }
            | Self::FormatText { target, .. } => target,
        }
    }

    fn signature(&self) -> &TargetSignature {
        match self {
            Self::CreateText { .. }
            | Self::DeleteXmlChildren { .. }
            | Self::InsertXmlChildren { .. }
            | Self::SetXmlAttribute { .. }
            | Self::RemoveXmlAttribute { .. } => {
                unreachable!("non-text actions do not have a text signature")
            }
            Self::InsertText { signature, .. }
            | Self::DeleteText { signature, .. }
            | Self::FormatText { signature, .. } => signature,
        }
    }

    fn operation_index(&self) -> usize {
        match self {
            Self::CreateText {
                operation_index, ..
            }
            | Self::DeleteXmlChildren {
                operation_index, ..
            }
            | Self::InsertXmlChildren {
                operation_index, ..
            }
            | Self::SetXmlAttribute {
                operation_index, ..
            }
            | Self::RemoveXmlAttribute {
                operation_index, ..
            }
            | Self::InsertText {
                operation_index, ..
            }
            | Self::DeleteText {
                operation_index, ..
            }
            | Self::FormatText {
                operation_index, ..
            } => *operation_index,
        }
    }
}
