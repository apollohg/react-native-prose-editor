#[cfg(test)]
use positions::resolve_structural_window;
mod admission;
mod input_limits;
mod insert_position;
mod observability;
mod operations;
mod positions;
mod preview;
mod selection;
mod semantic;
mod text_boundaries;
mod yrs_compilation;

use super::canonical::CanonicalArtifact;
use super::derived_state::{LocalizedInsertAdmission, PreparedDerivedEvidence};
use super::mutation::{
    LocalizedFormatCompiler, LocalizedInsertCompiler, LocalizedRootWindowCompiler,
    MutationCompiler, MutationLookupPromotion, YrsMutationAction, YrsMutationPlan,
};
use super::prepared_admission::DerivedStateAuthority;
use super::{
    EditingLimits, HistoryPolicy, OperationError, OperationResult, TransactionOrigin,
    TypedTransaction,
};
use crate::boundary::ResourceLimits;
use crate::model::{Document, Mark, Node};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::StepMap;
#[allow(unused_imports)]
pub(crate) use admission::{
    finalize_deferred_admission, PreparedCommandContractKind, PreparedSemanticAdmission,
    PreparedSemanticLiveContext,
};
#[allow(unused_imports)]
pub(super) use admission::{
    CandidateValidationAuthority, DeferredAdmissionAuthority, PreparedCandidateSeed,
};
#[allow(unused_imports)]
pub(crate) use input_limits::{admit_transaction_envelope, admit_yrs_scan_work};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use observability::{
    check_atomic_failpoint, force_localized_semantic_allocation_failure_for_test,
    reset_base_compilation_build_counts_for_test, reset_semantic_compilation_count_for_test,
    set_atomic_failpoint_for_test, take_base_compilation_build_counts_for_test,
    take_semantic_compilation_count_for_test, AtomicFailpoint,
};
#[cfg(test)]
use observability::{
    BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT, BASE_POSITION_MAP_BUILD_COUNT,
    BASE_RENDERED_TEXT_BUILD_COUNT,
};
#[allow(unused_imports)]
pub(crate) use positions::map_position;
use preview::LocalizedSemanticCompilation;
#[allow(unused_imports)]
pub(crate) use selection::selectable_void_at;
use semantic::compile_transaction_impl;
use std::sync::Arc;
use yrs::branch::{Branch, BranchPtr};
use yrs::types::Attrs;
use yrs_compilation::compile_transaction_with_yrs_impl;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) struct CompilationContext<'a> {
    pub document: &'a Document,
    pub selection: Option<&'a Selection>,
    pub schema: &'a Schema,
    pub resource_limits: &'a ResourceLimits,
    pub editing_limits: &'a EditingLimits,
    pub document_revision: u64,
    pub max_length: Option<u32>,
}

#[derive(Clone, Copy)]
pub(crate) struct CachedCompilationView<'a> {
    pub document: &'a Document,
    pub position_map: &'a PositionMap,
    pub rendered_text: &'a str,
    pub rendered_scalars: u32,
    pub document_text_bytes: usize,
    pub document_node_count: usize,
    pub selection: &'a Selection,
    pub document_revision: u64,
    pub state_revision: u64,
    pub schema_fingerprint: &'a str,
    pub canonical_artifact: &'a CanonicalArtifact,
}

#[derive(Clone, Copy)]
pub(crate) struct EngineCompilationView<'a> {
    pub cached: CachedCompilationView<'a>,
    pub authority: &'a dyn DerivedStateAuthority,
    pub state_revision: u64,
    pub schema_fingerprint: &'a str,
    pub yrs_state_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum HistoryClass {
    Insert,
    Delete,
    Format,
    Structural,
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum SelectionPlan {
    Preserve,
    Mapped(Selection),
    Explicit(Selection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RelativeSelectionPlan {
    Unsealed,
    Preserve,
    PreserveWithFallback(Selection),
    Precomputed {
        relative: super::RelativeSelection,
        fallback: Selection,
    },
    OperationResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoredMarksPlan {
    Unsealed,
    Set(Option<Vec<Mark>>),
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedSelectionMutationSeal {
    request_id: u64,
    base_state_revision: u64,
    origin: TransactionOrigin,
    history_policy: HistoryPolicy,
    history_class: HistoryClass,
    selection_plan: SelectionPlan,
    relative_selection_plan: RelativeSelectionPlan,
    stored_marks_plan: StoredMarksPlan,
    localized_semantic_used: bool,
    yrs_state_epoch: u64,
    preview: Document,
    target: BranchPtr,
    index_utf16: u32,
    len_utf16: u32,
    initial_len_utf16: u32,
    text: String,
    attrs: Attrs,
    inserted_document_position: u32,
    inserted_scalars: u32,
    inserted_utf16: u32,
    operation_result: super::ResolvedSelection,
    admission: LocalizedInsertAdmission,
}

impl PreparedSelectionMutationSeal {
    pub(crate) fn capture(compiled: &CompiledTransaction) -> Option<Self> {
        let admission = compiled.localized_insert_admission.as_ref()?;
        let [YrsMutationAction::InsertText {
            target,
            index_utf16,
            len_utf16,
            text,
            attrs,
            signature,
            ..
        }] = compiled.mutation_plan.actions.as_slice()
        else {
            return None;
        };
        Some(Self {
            request_id: compiled.request_id,
            base_state_revision: compiled.base_state_revision,
            origin: compiled.origin,
            history_policy: compiled.history_policy,
            history_class: compiled.history_class,
            selection_plan: compiled.selection_plan.clone(),
            relative_selection_plan: compiled.relative_selection_plan.clone(),
            stored_marks_plan: compiled.stored_marks_plan.clone(),
            localized_semantic_used: compiled.localized_semantic_used,
            yrs_state_epoch: compiled.yrs_state_epoch,
            preview: compiled.preview.clone(),
            target: BranchPtr::from(<yrs::types::xml::XmlTextRef as AsRef<Branch>>::as_ref(
                target,
            )),
            index_utf16: *index_utf16,
            len_utf16: *len_utf16,
            initial_len_utf16: signature.initial_len_utf16(),
            text: text.clone(),
            attrs: attrs.clone(),
            inserted_document_position: admission.inserted_document_position(),
            inserted_scalars: admission.inserted_scalars(),
            inserted_utf16: admission.inserted_utf16(),
            operation_result: admission.operation_result_selection().clone(),
            admission: admission.clone(),
        })
    }

    pub(crate) fn matches(
        &self,
        compiled: &CompiledTransaction,
        authority: &dyn DerivedStateAuthority,
    ) -> bool {
        let Some(admission) = compiled.localized_insert_admission.as_ref() else {
            return false;
        };
        let Ok(authority_seed) = authority.lookup_seed(self.request_id) else {
            return false;
        };
        let [YrsMutationAction::InsertText {
            target,
            index_utf16,
            len_utf16,
            text,
            attrs,
            signature,
            ..
        }] = compiled.mutation_plan.actions.as_slice()
        else {
            return false;
        };
        self.request_id == compiled.request_id
            && self.base_state_revision == compiled.base_state_revision
            && self.origin == compiled.origin
            && self.history_policy == compiled.history_policy
            && self.history_class == compiled.history_class
            && self.selection_plan == compiled.selection_plan
            && self.relative_selection_plan == compiled.relative_selection_plan
            && self.stored_marks_plan == compiled.stored_marks_plan
            && self.localized_semantic_used == compiled.localized_semantic_used
            && self.yrs_state_epoch == compiled.yrs_state_epoch
            && self.preview == compiled.preview
            && self.target
                == BranchPtr::from(<yrs::types::xml::XmlTextRef as AsRef<Branch>>::as_ref(
                    target,
                ))
            && self.index_utf16 == *index_utf16
            && self.len_utf16 == *len_utf16
            && self.initial_len_utf16 == signature.initial_len_utf16()
            && self.text.as_str() == text.as_str()
            && &self.attrs == attrs
            && self.inserted_document_position == admission.inserted_document_position()
            && self.inserted_scalars == admission.inserted_scalars()
            && self.inserted_utf16 == admission.inserted_utf16()
            && self.operation_result == *admission.operation_result_selection()
            && self.admission.same_prewrite_selection_claims(admission)
            && self.admission.lookup_seal_matches(authority_seed)
    }
}

pub(crate) struct StoredMarksCompilationContext<'a> {
    pub stored_marks: Option<&'a [Mark]>,
    pub resolved_selection: &'a super::ResolvedSelection,
    pub relative_selection: &'a super::RelativeSelection,
}

#[derive(Debug, Clone)]
pub(crate) enum MutationLookupTransition {
    Promote(MutationLookupPromotion),
    Invalidate { request_id: u64 },
}

impl MutationLookupTransition {
    pub(crate) fn request_id(&self) -> u64 {
        match self {
            Self::Promote(promotion) => promotion.request_id(),
            Self::Invalidate { request_id } => *request_id,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct CompiledTransaction {
    pub request_id: u64,
    pub base_state_revision: u64,
    pub origin: TransactionOrigin,
    pub history_policy: HistoryPolicy,
    pub history_class: HistoryClass,
    pub preview: Document,
    pub canonical_artifact: Option<CanonicalArtifact>,
    pub preview_derivations: Option<CompiledDocumentDerivations>,
    pub selection_plan: SelectionPlan,
    pub affected_top_level_blocks: Vec<usize>,
    pub composed_map: StepMap,
    pub position_update_mode: UpdateMode,
    pub relative_selection_plan: RelativeSelectionPlan,
    pub stored_marks_plan: StoredMarksPlan,
    pub mutation_plan: YrsMutationPlan,
    pub mutation_lookup_transition: Option<MutationLookupTransition>,
    pub encoded_growth_bound: usize,
    pub undo_units_bound: u64,
    pub replay_work_units_bound: u64,
    pub authored_clock_units: u64,
    pub yrs_state_epoch: u64,
    /// Stage E2 admission evidence. The semantic shortcut revalidates it during
    /// compilation; Stage E3 uses the resulting prepared evidence to install
    /// post-commit derived state without rebuilding it.
    pub localized_insert_admission: Option<LocalizedInsertAdmission>,
    pub prepared_derived_evidence: Option<PreparedDerivedEvidence>,
    pub prepared_candidate_validation: Option<super::derived_state::PreparedCandidateValidation>,
    pub prepared_active_state_transition:
        Option<super::derived_state::PreparedActiveStateTransition>,
    pub prepared_selection_state: Option<super::derived_state::FinalizedSelectionState>,
    pub prepared_selection_mutation_seal: Option<PreparedSelectionMutationSeal>,
    pub(crate) localized_semantic_used: bool,
}

impl CompiledTransaction {
    /// Conservative upper bound on the outbound incremental Update-v1 this
    /// plan can produce. This reuses the admitted encoded-growth bound: the
    /// commit pipeline captures the exact update on its private candidate and
    /// invariant-checks `len() <= encoded_growth_bound` before the durable
    /// write, so the collaboration outbox reservation shares the same seam
    /// instead of a parallel bound pipeline.
    pub(crate) fn outbound_update_upper_bound(&self) -> usize {
        self.encoded_growth_bound
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledDocumentDerivations {
    pub identity_seal: Arc<()>,
    pub position_map: PositionMap,
    pub rendered_text: String,
    pub rendered_scalars: u32,
    pub document_text_bytes: usize,
    pub document_node_count: usize,
}

enum TransactionMutationLowering {
    Eager(Box<MutationCompiler>),
    LocalizedInsert(Box<LocalizedInsertCompiler>),
    LocalizedFormat(Box<LocalizedFormatCompiler>),
    LocalizedRootWindow(Box<LocalizedRootWindowCompiler>),
}

#[derive(Clone, Copy)]
pub(crate) struct PreparedSemanticContext<'a> {
    pub admission: &'a PreparedSemanticAdmission,
    pub expected_preview: &'a Document,
    pub yrs_state_epoch: u64,
    pub state_revision: u64,
    pub schema_fingerprint: &'a str,
}

struct SemanticCompilationShortcuts<'a> {
    prepared: Option<PreparedSemanticContext<'a>>,
    localized: Option<LocalizedSemanticCompilation>,
}

pub(super) fn compile_transaction(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
) -> OperationResult<CompiledTransaction> {
    admit_transaction_envelope(context, &transaction)?;
    compile_transaction_impl(
        context,
        &transaction,
        None,
        None,
        None,
        None,
        SemanticCompilationShortcuts {
            prepared: None,
            localized: None,
        },
    )
}

pub(super) fn compile_transaction_with_yrs<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
) -> OperationResult<CompiledTransaction> {
    compile_transaction_with_yrs_impl(context, transaction, txn, fragment, None, None, None)
}

pub(super) fn compile_transaction_with_yrs_and_stored_marks<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    stored_marks: StoredMarksCompilationContext<'_>,
    engine_view: EngineCompilationView<'_>,
) -> OperationResult<CompiledTransaction> {
    compile_transaction_with_yrs_impl(
        context,
        transaction,
        txn,
        fragment,
        Some(stored_marks),
        None,
        Some(engine_view),
    )
}

pub(super) fn compile_prepared_transaction_with_yrs_and_stored_marks<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    stored_marks: StoredMarksCompilationContext<'_>,
    prepared: PreparedSemanticContext<'_>,
    engine_view: EngineCompilationView<'_>,
) -> OperationResult<CompiledTransaction> {
    compile_transaction_with_yrs_impl(
        context,
        transaction,
        txn,
        fragment,
        Some(stored_marks),
        Some(prepared),
        Some(engine_view),
    )
}

pub(crate) fn document_text_bytes(document: &Document) -> Option<usize> {
    #[cfg(test)]
    super::observability::record_raw_document_text_scan();
    fn node_bytes(node: &Node) -> Option<usize> {
        let mut total = node.text_str().map_or(0, str::len);
        if let Some(content) = node.content() {
            for child in content.iter() {
                total = total.checked_add(node_bytes(child)?)?;
            }
        }
        Some(total)
    }
    node_bytes(document.root())
}

fn base_document_text_bytes(document: &Document) -> Option<usize> {
    #[cfg(test)]
    BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT
        .set(BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT.get().saturating_add(1));
    document_text_bytes(document)
}

fn base_position_map(document: &Document, schema: &Schema) -> PositionMap {
    #[cfg(test)]
    BASE_POSITION_MAP_BUILD_COUNT.set(BASE_POSITION_MAP_BUILD_COUNT.get().saturating_add(1));
    PositionMap::build(document, schema)
}

fn base_rendered_text(document: &Document, schema: &Schema) -> String {
    #[cfg(test)]
    BASE_RENDERED_TEXT_BUILD_COUNT.set(BASE_RENDERED_TEXT_BUILD_COUNT.get().saturating_add(1));
    crate::render::rendered_text(document, schema)
}

fn merge_history_class(current: HistoryClass, next: HistoryClass) -> HistoryClass {
    match (current, next) {
        (HistoryClass::Skip, next) => next,
        (current, next) if current == next => current,
        (HistoryClass::Structural, _) | (_, HistoryClass::Structural) => HistoryClass::Structural,
        _ => HistoryClass::Structural,
    }
}

fn map_transform_error(
    request_id: u64,
    operation_index: usize,
    field: &'static str,
    error: crate::transform::TransformError,
) -> OperationError {
    match error {
        crate::transform::TransformError::OutOfBounds(message)
        | crate::transform::TransformError::InvalidTarget(message) => {
            OperationError::position_invalid(request_id, operation_index, field, message)
        }
        crate::transform::TransformError::InvalidRange(message) => {
            OperationError::operation_invalid(request_id, operation_index, "range", message)
        }
        crate::transform::TransformError::ContentViolation(message) => {
            OperationError::document_invalid(request_id, Some(operation_index), "content", message)
        }
        crate::transform::TransformError::NotImplemented(message) => {
            OperationError::operation_invalid(request_id, operation_index, "operation", message)
        }
    }
}

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;
