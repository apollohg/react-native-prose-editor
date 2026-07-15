#[derive(Debug, Clone)]
struct ResolvedText {
    kind: ResolvedTargetKind,
    gap_before: u32,
    text: String,
    scalar_len: u32,
    base_runs: Vec<PreparedTextRun>,
    current_runs: Vec<PreparedTextRun>,
    action_slots: Vec<usize>,
}

#[derive(Debug, Clone)]
enum ResolvedTargetKind {
    Existing {
        target: XmlTextRef,
        signature: TargetSignature,
    },
    Missing {
        parent: XmlElementRef,
        child_index: u32,
        signature: ParentSignature,
        create_action: Option<usize>,
    },
    Prepared {
        handle: PreparedHandle,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PreparedHandle {
    insert_id: usize,
    ordinal_path: Box<[usize]>,
}

#[derive(Debug, Clone)]
struct PendingPreparedInsert {
    parent: XmlParentRef,
    child_index: u32,
    nodes: Vec<PreparedXmlChild>,
    signature: Arc<StructuralParentSignature>,
    operation_index: usize,
    semantic_parent_path: Vec<u32>,
    first_semantic_index: u32,
}

#[derive(Debug, Clone)]
enum ActionSlot {
    Concrete(Box<YrsMutationAction>),
    PreparedInsert(usize),
    Tombstone,
}

impl ActionSlot {
    fn concrete(action: YrsMutationAction) -> Self {
        Self::Concrete(Box::new(action))
    }

    fn concrete_mut(&mut self) -> Option<&mut YrsMutationAction> {
        match self {
            Self::Concrete(action) => Some(action.as_mut()),
            Self::PreparedInsert(_) | Self::Tombstone => None,
        }
    }
}

enum LocatedTarget {
    Existing {
        start: u32,
        target: XmlTextRef,
        text: String,
        scalar_len: u32,
        signature: TargetSignature,
    },
    Missing {
        start: u32,
        parent: XmlElementRef,
        child_index: u32,
        signature: ParentSignature,
    },
}

#[derive(Debug, Clone)]
struct MaterializedText {
    text: String,
    scalar_len: u32,
    utf16_len: u32,
    signature_runs: Vec<TextSignatureRun>,
    prepared_runs: Vec<PreparedTextRun>,
    work: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct MutationDocumentContext<'a> {
    pub(crate) before: &'a Document,
    pub(crate) after: &'a Document,
    pub(crate) schema: &'a Schema,
    pub(crate) limits: &'a ResourceLimits,
}

#[derive(Clone, Copy)]
pub(crate) struct ReplacementInput<'a> {
    pub(crate) from: u32,
    pub(crate) to: u32,
    pub(crate) boundaries: &'a [u32],
    pub(crate) content: &'a Fragment,
}

#[derive(Debug, Clone)]
struct StructuralParentTarget {
    parent: XmlParentRef,
    signature: Arc<StructuralParentSignature>,
    storage_children: Vec<StorageChildKind>,
}

#[derive(Debug, Clone)]
struct PendingElementAttrs {
    target: XmlElementRef,
    signature: Arc<ElementSignature>,
    desired: Vec<(Arc<str>, Any)>,
    operation_index: usize,
    first_order: usize,
}

#[derive(Debug, Clone)]
struct VirtualStateCheckpoint {
    document: Document,
    targets: Vec<ResolvedText>,
    structural_parents: HashMap<Vec<u32>, StructuralParentTarget>,
    actions: Vec<ActionSlot>,
    prepared_inserts: Vec<Option<PendingPreparedInsert>>,
    prepared_elements: HashMap<Vec<u32>, PreparedHandle>,
    created_gap_shifts: HashMap<BranchID, Vec<u32>>,
    pending_element_attrs: HashMap<BranchID, PendingElementAttrs>,
}

#[derive(Debug, Clone)]
enum StorageChildKind {
    Text {
        scalar_len: u32,
        target: XmlTextRef,
        signature: TargetSignature,
        runs: Vec<PreparedTextRun>,
    },
    Element {
        target: XmlElementRef,
        signature: Arc<ElementSignature>,
    },
    PreparedElement,
}

#[derive(Debug, Clone)]
enum StorageInsertion {
    Boundary(u32),
    InsideText {
        child_index: u32,
        local_scalar: u32,
        target: XmlTextRef,
        signature: TargetSignature,
        runs: Vec<PreparedTextRun>,
    },
}

#[derive(Debug, Clone)]
struct VirtualStructuralSplice {
    parent_path: Vec<u32>,
    semantic_index: u32,
    semantic_delete: u32,
    semantic_insert: u32,
    storage_index: u32,
    storage_delete: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextRangeDisposition {
    Applied,
    Structural,
}

#[derive(Debug)]
pub(crate) struct MutationCompiler {
    request_id: u64,
    document_guard: DocumentGuard,
    targets: Vec<ResolvedText>,
    structural_parents: HashMap<Vec<u32>, StructuralParentTarget>,
    actions: Vec<ActionSlot>,
    prepared_inserts: Vec<Option<PendingPreparedInsert>>,
    prepared_elements: HashMap<Vec<u32>, PreparedHandle>,
    charged_work: usize,
    pending_traversal_work: usize,
    action_limit: usize,
    scan_work: usize,
    scan_limit: usize,
    #[cfg(test)]
    position_resolver_work: usize,
    created_gap_shifts: HashMap<BranchID, Vec<u32>>,
    pending_element_attrs: HashMap<BranchID, PendingElementAttrs>,
    wrap_checkpoints: HashMap<usize, VirtualStateCheckpoint>,
    #[cfg(test)]
    virtual_delete_visits: usize,
}
