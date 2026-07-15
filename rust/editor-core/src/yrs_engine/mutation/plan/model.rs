use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yrs::any::Any;
use yrs::branch::{Branch, BranchID};
use yrs::types::text::Text;
use yrs::types::xml::{
    Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef,
};
use yrs::types::xml::XmlTextPrelim;
use yrs::types::Attrs;
use yrs::ReadTxn;
use yrs::Snapshot;
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
