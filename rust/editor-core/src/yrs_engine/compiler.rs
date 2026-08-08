use std::borrow::Cow;
use std::sync::Arc;

use crate::boundary::{JsonMeterDimension, JsonMeterError, JsonValueMeter, ResourceLimits};
use crate::model::{Document, Fragment, Mark, Node};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::content_rule::WorkBudget;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::{DocumentValidator, Step, StepMap};
use smallvec::SmallVec;
use yrs::branch::{Branch, BranchPtr};
use yrs::types::Attrs;

use super::canonical::{CanonicalArtifact, CanonicalSchemaContext};
use super::derived_state::{
    LocalizedInsertAdmission, PreparedDerivedEvidence, ValidatedLocalizedInsertAdmission,
};
use super::editing_limits::CheckedWork;
use super::mutation::{
    crdt_clock_scan_reservation, crdt_envelope, estimate_undo_units, estimate_update_v1_growth,
    mark_attr, planned_insertion_units, preflight_mutation_plan, removed_mark_attr, CrdtEnvelope,
    LocalizedFormatCompiler, LocalizedFormatLocator, LocalizedInsertCompiler,
    LocalizedInsertLocator, LocalizedRootWindowCompiler, LocalizedRootWindowLocator,
    MutationCompiler, MutationDocumentContext, MutationLookupPromotion, ReplacementInput,
    TextRangeDisposition, YrsMutationAction, YrsMutationPlan,
};
use super::prepared_admission::DerivedStateAuthority;
use super::{
    editor_offset_to_doc_pos, Affinity, EditingLimits, HistoryPolicy, OperationError,
    OperationResult, RevisionedRange, SelectionIntent, TransactionOrigin, TypedOperation,
    TypedTransaction,
};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicFailpoint {
    EnvelopeAdmission,
    SemanticCompilation,
    MutationPreflight,
    FinalPreflight,
    EncodedAdmission,
    CanonicalOutputAdmission,
    RevisionAdmission,
    DurableMetadataAdmission,
    RemoteHistoryAdmission,
}

#[cfg(test)]
impl AtomicFailpoint {
    pub(crate) const fn field_name(self) -> &'static str {
        match self {
            Self::EnvelopeAdmission => "envelopeAdmission",
            Self::SemanticCompilation => "semanticCompilation",
            Self::MutationPreflight => "mutationPreflight",
            Self::FinalPreflight => "finalPreflight",
            Self::EncodedAdmission => "encodedAdmission",
            Self::CanonicalOutputAdmission => "canonicalOutputAdmission",
            Self::RevisionAdmission => "revisionAdmission",
            Self::DurableMetadataAdmission => "durableMetadataAdmission",
            Self::RemoteHistoryAdmission => "remoteHistoryAdmission",
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static ATOMIC_FAILPOINT: std::cell::Cell<Option<AtomicFailpoint>> = const {
        std::cell::Cell::new(None)
    };
    static SEMANTIC_COMPILATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static BASE_POSITION_MAP_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static BASE_RENDERED_TEXT_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FORCE_LOCALIZED_SEMANTIC_ALLOCATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
}

#[cfg(test)]
pub(crate) fn set_atomic_failpoint_for_test(failpoint: Option<AtomicFailpoint>) {
    ATOMIC_FAILPOINT.set(failpoint);
}

#[cfg(test)]
pub(crate) fn reset_semantic_compilation_count_for_test() {
    SEMANTIC_COMPILATION_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_semantic_compilation_count_for_test() -> usize {
    SEMANTIC_COMPILATION_COUNT.replace(0)
}

#[cfg(test)]
pub(crate) fn reset_base_compilation_build_counts_for_test() {
    BASE_POSITION_MAP_BUILD_COUNT.set(0);
    BASE_RENDERED_TEXT_BUILD_COUNT.set(0);
    BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_base_compilation_build_counts_for_test() -> (usize, usize, usize) {
    (
        BASE_POSITION_MAP_BUILD_COUNT.replace(0),
        BASE_RENDERED_TEXT_BUILD_COUNT.replace(0),
        BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn force_localized_semantic_allocation_failure_for_test(force: bool) {
    FORCE_LOCALIZED_SEMANTIC_ALLOCATION_FAILURE.set(force);
}

#[cfg(test)]
pub(crate) fn check_atomic_failpoint(
    request_id: u64,
    stage: AtomicFailpoint,
) -> OperationResult<()> {
    if ATOMIC_FAILPOINT.get() == Some(stage) {
        Err(OperationError::atomic_failpoint(
            request_id,
            stage.field_name(),
        ))
    } else {
        Ok(())
    }
}

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

struct LocalizedSemanticCompilation {
    position: u32,
    preview: Document,
    step_map: StepMap,
    derivations: LocalizedSemanticDerivations,
}

struct LocalizedSemanticDerivations {
    affected_top_level_blocks: Vec<usize>,
    rendered_text: String,
    rendered_scalars: u32,
    document_text_bytes: usize,
    document_node_count: usize,
    raw_text_scalars: u64,
    raw_text_utf8_bytes: usize,
}

enum TransactionMutationLowering {
    Eager(Box<MutationCompiler>),
    LocalizedInsert(Box<LocalizedInsertCompiler>),
    LocalizedFormat(Box<LocalizedFormatCompiler>),
    LocalizedRootWindow(Box<LocalizedRootWindowCompiler>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreparedCommandContractKind {
    None,
    RootWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreparedCommandContractOracle {
    None,
    RootWrap {
        direct_insertion_units: u64,
        direct_growth_bytes: usize,
        replaced_children: u32,
    },
}

impl PreparedCommandContractOracle {
    fn admits_transaction(
        self,
        transaction: &TypedTransaction,
        has_candidate_validation: bool,
    ) -> bool {
        match self {
            Self::None => true,
            Self::RootWrap {
                replaced_children, ..
            } => {
                let [TypedOperation::ReplaceStructure(replacement)] =
                    transaction.operations.as_slice()
                else {
                    return false;
                };
                let (from_child, to_child) = replacement.child_window();
                has_candidate_validation
                    && replacement.parent_path().is_empty()
                    && replacement.content().child_count() == 1
                    && replaced_children > 0
                    && to_child.checked_sub(from_child) == Some(replaced_children)
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedSemanticAdmission {
    request_id: u64,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    schema_fingerprint: Box<str>,
    transaction: TypedTransaction,
    expected_document: Document,
    canonical_artifact: CanonicalArtifact,
    candidate_validation: Option<super::derived_state::PreparedCandidateValidation>,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
    undo_units: u64,
    command_contract_oracle: PreparedCommandContractOracle,
}

struct PreparedSemanticConstruction {
    request_id: u64,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    schema_fingerprint: Box<str>,
    transaction: TypedTransaction,
    expected_document: Document,
    canonical_artifact: CanonicalArtifact,
    candidate_validation: Option<super::derived_state::PreparedCandidateValidation>,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
    undo_units: u64,
    command_contract_oracle: PreparedCommandContractOracle,
}

/// Unforgeable authority for the raw candidate-evidence constructor. The type
/// is visible to `derived_state` so it can require the capability, while its
/// private field keeps construction inside this compiler module.
pub(super) struct CandidateValidationAuthority(());

/// One-shot authority for consuming a deferred semantic admission. Only this
/// compiler module can mint the capability, after staged/live seals pass.
pub(super) struct DeferredAdmissionAuthority(());

#[derive(Clone, Copy)]
pub(crate) struct PreparedSemanticLiveContext<'a> {
    pub(crate) transaction: &'a super::TypedTransaction,
    pub(crate) expected_preview: &'a crate::model::Document,
    pub(crate) canonical_schema: &'a super::canonical::CanonicalSchemaContext,
}

/// Opaque, compiler-minted candidate input. Callers can request a seed and use
/// it to normalize a simulated selection, but cannot supply or inspect the
/// position map that will become validation evidence.
pub(super) struct PreparedCandidateSeed {
    document: Document,
    position_map: PositionMap,
    schema_fingerprint: Box<str>,
    canonical_schema: CanonicalSchemaContext,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
}

impl PreparedCandidateSeed {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn mint(
        request_id: u64,
        document: &Document,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
    ) -> OperationResult<Self> {
        let schema_fingerprint = crate::schema::schema_fingerprint(schema);
        if schema_fingerprint != canonical_schema.schema_fingerprint() {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "candidate seed schema does not match its canonical context",
            ));
        }
        Ok(Self {
            document: document.clone(),
            position_map: PositionMap::build(document, schema),
            schema_fingerprint: schema_fingerprint.into(),
            canonical_schema: canonical_schema.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
        })
    }

    pub(super) fn normalize_selection(&self, selection: &Selection) -> Selection {
        selection
            .clone()
            .normalized(&self.document, &self.position_map)
    }

    #[allow(clippy::too_many_arguments)]
    fn consume(
        self,
        request_id: u64,
        document: &Document,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
    ) -> OperationResult<PositionMap> {
        if !self.document.shares_root_storage_with(document)
            || self.schema_fingerprint.as_ref() != crate::schema::schema_fingerprint(schema)
            || !self.canonical_schema.ptr_eq(canonical_schema)
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(0),
                "prepared candidate seed does not match the admitted document context",
            ));
        }
        Ok(self.position_map)
    }
}

fn prepare_command_contract_oracle(
    request_id: u64,
    transaction: &TypedTransaction,
    command_contract_kind: PreparedCommandContractKind,
    schema: &Schema,
    resource_limits: &ResourceLimits,
) -> OperationResult<PreparedCommandContractOracle> {
    match command_contract_kind {
        PreparedCommandContractKind::None => Ok(PreparedCommandContractOracle::None),
        PreparedCommandContractKind::RootWrap => {
            let [TypedOperation::ReplaceStructure(replacement)] = transaction.operations.as_slice()
            else {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "prepared root wrap has no exact structural replacement",
                ));
            };
            let (from_child, to_child) = replacement.child_window();
            let replaced_children = to_child.checked_sub(from_child).filter(|count| *count > 0);
            let list_node = replacement.content().child(0);
            if !replacement.parent_path().is_empty()
                || replacement.content().child_count() != 1
                || replaced_children.is_none()
                || list_node.is_none()
            {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "prepared root wrap replacement is not one nonempty root window",
                ));
            }
            let metrics = super::mutation::direct_root_wrap_metrics(
                request_id,
                0,
                list_node.expect("checked prepared root-wrap child"),
                schema,
                resource_limits,
            )?;
            Ok(PreparedCommandContractOracle::RootWrap {
                direct_insertion_units: metrics.insertion_units,
                direct_growth_bytes: metrics.growth_bytes,
                replaced_children: replaced_children
                    .expect("checked prepared root-wrap child window"),
            })
        }
    }
}

pub(crate) fn finalize_deferred_admission(
    staged: &super::prepared_admission::StagedDerivedStateAuthority<'_>,
    deferred: super::prepared_admission::DeferredCommandAdmission,
    live: PreparedSemanticLiveContext<'_>,
) -> OperationResult<PreparedSemanticAdmission> {
    deferred.validate_finalization_context(staged, live)?;
    let authority = DeferredAdmissionAuthority(());
    PreparedSemanticAdmission::finalize_from_validated_evidence(&authority, staged, deferred, live)
}

impl PreparedSemanticAdmission {
    fn from_post_validation_construction(construction: PreparedSemanticConstruction) -> Self {
        Self {
            request_id: construction.request_id,
            document_revision: construction.document_revision,
            state_revision: construction.state_revision,
            yrs_state_epoch: construction.yrs_state_epoch,
            schema_fingerprint: construction.schema_fingerprint,
            transaction: construction.transaction,
            expected_document: construction.expected_document,
            canonical_artifact: construction.canonical_artifact,
            candidate_validation: construction.candidate_validation,
            resource_limits: construction.resource_limits,
            editing_limits: construction.editing_limits,
            max_length: construction.max_length,
            undo_units: construction.undo_units,
            command_contract_oracle: construction.command_contract_oracle,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_deferred_insert(
        request_id: u64,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema_fingerprint: Box<str>,
        transaction: TypedTransaction,
        expected_document: Document,
        canonical_artifact: CanonicalArtifact,
        canonical_schema: CanonicalSchemaContext,
        validation: crate::transform::DocumentValidationReport,
        candidate_evidence: super::derived_state::PreparedCandidateEvidence,
        resource_limits: ResourceLimits,
        editing_limits: EditingLimits,
        max_length: Option<u32>,
        undo_units: u64,
    ) -> OperationResult<Self> {
        if !canonical_artifact.matches_exact_source_document(&expected_document)
            || canonical_artifact.schema_fingerprint() != schema_fingerprint.as_ref()
            || validation.stats.node_count == 0
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                Some(0),
                "deferred insert candidate evidence is not internally consistent",
            ));
        }
        let authority = CandidateValidationAuthority(());
        let candidate_validation = candidate_evidence
            .finalize_deferred(
                &authority,
                &expected_document,
                &canonical_artifact,
                validation,
                &resource_limits,
                &editing_limits,
                max_length,
                schema_fingerprint.as_ref(),
                &canonical_schema,
            )
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    "deferred insert validation evidence could not be sealed",
                )
            })?;
        Ok(Self::from_post_validation_construction(
            PreparedSemanticConstruction {
                request_id,
                document_revision,
                state_revision,
                yrs_state_epoch,
                schema_fingerprint,
                transaction,
                expected_document,
                canonical_artifact,
                candidate_validation: Some(candidate_validation),
                resource_limits,
                editing_limits,
                max_length,
                undo_units,
                command_contract_oracle: PreparedCommandContractOracle::None,
            },
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_single_operation(
        request_id: u64,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        transaction: &TypedTransaction,
        expected_document: &Document,
        candidate_seed: Option<PreparedCandidateSeed>,
        known_canonical_serialized_len: Option<usize>,
        undo_units: u64,
        command_contract_kind: PreparedCommandContractKind,
    ) -> OperationResult<Self> {
        let validation =
            DocumentValidator::validate_report(expected_document, schema, resource_limits)
                .map_err(|error| {
                    OperationError::document_invalid(request_id, None, "command", error.to_string())
                })?;
        if let Some(limit) = max_length {
            let actual = expected_document.root().text_content().chars().count();
            if actual > limit as usize {
                return Err(OperationError::document_limit_exceeded(
                    request_id,
                    None,
                    "maxLength",
                    u64::from(limit),
                    actual as u64,
                ));
            }
        }
        let command_contract_oracle = prepare_command_contract_oracle(
            request_id,
            transaction,
            command_contract_kind,
            schema,
            resource_limits,
        )?;
        Self::prepare_single_operation_from_validation(
            request_id,
            document_revision,
            state_revision,
            yrs_state_epoch,
            schema,
            canonical_schema,
            resource_limits,
            editing_limits,
            max_length,
            transaction,
            expected_document,
            validation,
            candidate_seed,
            known_canonical_serialized_len,
            undo_units,
            command_contract_oracle,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_single_operation_from_validation(
        request_id: u64,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema: &Schema,
        canonical_schema: &CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &EditingLimits,
        max_length: Option<u32>,
        transaction: &TypedTransaction,
        expected_document: &Document,
        validation: crate::transform::DocumentValidationReport,
        candidate_seed: Option<PreparedCandidateSeed>,
        known_canonical_serialized_len: Option<usize>,
        undo_units: u64,
        command_contract_oracle: PreparedCommandContractOracle,
    ) -> OperationResult<Self> {
        let candidate_position_map = candidate_seed
            .map(|seed| {
                seed.consume(
                    request_id,
                    expected_document,
                    schema,
                    canonical_schema,
                    resource_limits,
                    editing_limits,
                    max_length,
                )
            })
            .transpose()?;
        let canonical_artifact = known_canonical_serialized_len
            .map_or_else(
                || canonical_schema.derive(expected_document),
                |serialized_len| {
                    canonical_schema
                        .derive_with_known_serialized_len(expected_document, serialized_len)
                },
            )
            .map_err(|error| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(0),
                    format!("preview serialization failed: {error}"),
                )
            })?;
        let candidate_validation = candidate_position_map
            .map(|position_map| {
                let authority = CandidateValidationAuthority(());
                super::derived_state::PreparedCandidateValidation::prepare(
                    &authority,
                    expected_document,
                    &canonical_artifact,
                    validation,
                    schema,
                    resource_limits,
                    editing_limits,
                    max_length,
                    canonical_schema.schema_fingerprint(),
                    position_map,
                )
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(0),
                        "prepared command candidate validation could not be sealed",
                    )
                })
            })
            .transpose()?;
        Ok(Self::from_post_validation_construction(
            PreparedSemanticConstruction {
                request_id,
                document_revision,
                state_revision,
                yrs_state_epoch,
                schema_fingerprint: canonical_schema.schema_fingerprint().into(),
                transaction: transaction.clone(),
                expected_document: expected_document.clone(),
                canonical_artifact,
                candidate_validation,
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                undo_units,
                command_contract_oracle,
            },
        ))
    }

    pub(crate) fn finalize_from_validated_evidence(
        authority: &DeferredAdmissionAuthority,
        staged: &super::prepared_admission::StagedDerivedStateAuthority<'_>,
        deferred: super::prepared_admission::DeferredCommandAdmission,
        live: PreparedSemanticLiveContext<'_>,
    ) -> OperationResult<PreparedSemanticAdmission> {
        let parts = deferred.into_finalization_parts(authority, staged, live)?;
        let exact_candidate_len = parts
            .base_canonical_serialized_len
            .checked_add(parts.escaped_body_bytes)
            .filter(|exact| {
                *exact <= parts.candidate_output_upper_bound
                    && *exact <= parts.editing_limits.max_derived_output_bytes
            })
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    parts.request_id,
                    Some(0),
                    "deferred insert exact output exceeded its conservative proof",
                )
            })?;
        let canonical_artifact = parts
            .candidate_canonical
            .seal_with_known_serialized_len(exact_candidate_len)
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    parts.request_id,
                    Some(0),
                    "deferred insert scalar caches diverged from its canonical projection",
                )
            })?;
        if canonical_artifact.serialized_len() != exact_candidate_len
            || !canonical_artifact.matches_exact_source_document(&parts.expected_document)
            || canonical_artifact.schema_fingerprint() != parts.schema_fingerprint.as_ref()
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !canonical_artifact
                .schema_context()
                .ptr_eq(&parts.canonical_schema)
        {
            return Err(OperationError::engine_invariant_failed(
                parts.request_id,
                Some(0),
                "deferred insert canonical projection evidence diverged",
            ));
        }
        let candidate_authority = CandidateValidationAuthority(());
        let candidate_validation = parts
            .candidate_evidence
            .finalize_deferred(
                &candidate_authority,
                &parts.expected_document,
                &canonical_artifact,
                parts.validation_report,
                &parts.resource_limits,
                &parts.editing_limits,
                parts.max_length,
                parts.schema_fingerprint.as_ref(),
                &parts.canonical_schema,
            )
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    parts.request_id,
                    Some(0),
                    "deferred insert saved validation evidence diverged",
                )
            })?;
        let prepared = Self::from_post_validation_construction(PreparedSemanticConstruction {
            request_id: parts.request_id,
            document_revision: parts.document_revision,
            state_revision: parts.state_revision,
            yrs_state_epoch: parts.yrs_state_epoch,
            schema_fingerprint: parts.schema_fingerprint,
            transaction: parts.transaction,
            expected_document: parts.expected_document,
            canonical_artifact,
            candidate_validation: Some(candidate_validation),
            resource_limits: parts.resource_limits,
            editing_limits: parts.editing_limits,
            max_length: parts.max_length,
            undo_units: parts.undo_units,
            command_contract_oracle: PreparedCommandContractOracle::None,
        });
        prepared.pre_admit_seed_independent(
            live.transaction,
            live.expected_preview,
            &prepared.editing_limits,
        )?;
        #[cfg(test)]
        super::observability::record_deferred_capsule_finalized();
        Ok(prepared)
    }

    #[allow(clippy::too_many_arguments)]
    fn admit(
        &self,
        transaction: &TypedTransaction,
        expected_preview: &Document,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        limits: &super::EditingLimits,
        max_length: Option<u32>,
        canonical_schema: &CanonicalSchemaContext,
    ) -> OperationResult<()> {
        self.pre_admit_seed_independent(transaction, expected_preview, limits)?;
        if self.request_id != transaction.request_id
            || self.document_revision != transaction.base_document_revision
            || self.document_revision != document_revision
            || self.state_revision != state_revision
            || self.yrs_state_epoch != yrs_state_epoch
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || !self
                .canonical_artifact
                .matches_exact_source_document(expected_preview)
            || self.canonical_artifact.schema_fingerprint() != schema_fingerprint
            || self.canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !self
                .canonical_artifact
                .schema_context()
                .ptr_eq(canonical_schema)
            || self.resource_limits != *resource_limits
            || self.max_length != max_length
            || self
                .candidate_validation
                .as_ref()
                .is_some_and(|validation| {
                    !validation.admits_context(
                        expected_preview,
                        &self.canonical_artifact,
                        resource_limits,
                        limits,
                        max_length,
                        schema_fingerprint,
                        canonical_schema,
                    )
                })
        {
            return Err(OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "prepared semantic admission does not match the live command context",
            ));
        }
        Ok(())
    }

    pub(crate) fn pre_admit_seed_independent(
        &self,
        transaction: &TypedTransaction,
        expected_preview: &Document,
        editing_limits: &EditingLimits,
    ) -> OperationResult<()> {
        if self.request_id != transaction.request_id
            || self.transaction != *transaction
            || !self.admits_expected_document(expected_preview)
            || self.editing_limits != *editing_limits
            || !self
                .command_contract_oracle
                .admits_transaction(transaction, self.candidate_validation.is_some())
        {
            return Err(OperationError::engine_invariant_failed(
                transaction.request_id,
                None,
                "prepared semantic admission does not match the live command context",
            ));
        }
        if self.canonical_artifact.serialized_len() > editing_limits.max_derived_output_bytes {
            return Err(OperationError::document_limit_exceeded(
                transaction.request_id,
                Some(0),
                "maxDerivedOutputBytes",
                u64::try_from(editing_limits.max_derived_output_bytes).unwrap_or(u64::MAX),
                u64::try_from(self.canonical_artifact.serialized_len()).unwrap_or(u64::MAX),
            ));
        }
        if self.undo_units > editing_limits.max_undo_retained_units {
            return Err(OperationError::operation_limit_exceeded(
                transaction.request_id,
                Some(0),
                "maxUndoRetainedUnits",
                editing_limits.max_undo_retained_units,
                self.undo_units,
            ));
        }
        Ok(())
    }

    pub(crate) fn is_identity_dependent_insert(&self) -> bool {
        matches!(
            self.transaction.operations.as_slice(),
            [TypedOperation::InsertText { .. }]
        )
    }

    pub(crate) fn transaction(&self) -> &TypedTransaction {
        &self.transaction
    }

    pub(crate) fn expected_document(&self) -> &Document {
        &self.expected_document
    }

    pub(crate) fn admits_expected_document(&self, document: &Document) -> bool {
        self.expected_document.shares_root_storage_with(document)
    }

    pub(crate) fn canonical_artifact(&self) -> &CanonicalArtifact {
        &self.canonical_artifact
    }

    pub(crate) fn undo_units(&self) -> u64 {
        self.undo_units
    }

    pub(crate) fn candidate_derivations(&self) -> Option<CompiledDocumentDerivations> {
        self.candidate_validation.as_ref()?.compiled_derivations(
            &self.expected_document,
            &self.canonical_artifact,
            &self.resource_limits,
            &self.editing_limits,
            self.max_length,
            &self.schema_fingerprint,
            self.canonical_artifact.schema_context(),
        )
    }

    fn candidate_validation(&self) -> Option<super::derived_state::PreparedCandidateValidation> {
        self.candidate_validation.clone()
    }

    fn candidate_validation_ref(
        &self,
    ) -> Option<&super::derived_state::PreparedCandidateValidation> {
        self.candidate_validation.as_ref()
    }

    fn command_contract_oracle(&self) -> PreparedCommandContractOracle {
        self.command_contract_oracle
    }

    #[cfg(test)]
    pub(crate) fn replace_candidate_artifact_for_test(
        &mut self,
        canonical_artifact: CanonicalArtifact,
    ) {
        self.candidate_validation
            .as_mut()
            .expect("test tamper requires prepared candidate validation")
            .replace_canonical_artifact_for_test(canonical_artifact);
    }

    #[cfg(test)]
    pub(crate) fn replace_canonical_artifact_for_test(
        &mut self,
        canonical_artifact: CanonicalArtifact,
    ) {
        self.canonical_artifact = canonical_artifact;
    }
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

fn compile_transaction_with_yrs_impl<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    stored_marks: Option<StoredMarksCompilationContext<'_>>,
    prepared_semantics: Option<PreparedSemanticContext<'_>>,
    engine_view: Option<EngineCompilationView<'_>>,
) -> OperationResult<CompiledTransaction> {
    let request_id = transaction.request_id;
    #[cfg(test)]
    check_atomic_failpoint(request_id, AtomicFailpoint::EnvelopeAdmission)?;
    let admitted_input_bytes = admit_transaction_envelope(context, &transaction)?;
    #[cfg(test)]
    check_atomic_failpoint(request_id, AtomicFailpoint::SemanticCompilation)?;
    let action_multiplier = context
        .editing_limits
        .max_operations_per_transaction
        .checked_add(context.resource_limits.max_document_depth)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::MAX,
                u64::MAX,
            )
        })?;
    let action_limit = action_multiplier
        .checked_mul(context.resource_limits.max_document_nodes)
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxActionsPerTransaction",
                u64::MAX,
                u64::MAX,
            )
        })?;
    if let Some(prepared) = prepared_semantics {
        let canonical_schema = engine_view
            .map(|view| view.cached.canonical_artifact.schema_context())
            .ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "prepared semantic compilation has no live canonical schema context",
                )
            })?;
        prepared.admission.admit(
            &transaction,
            prepared.expected_preview,
            context.document_revision,
            prepared.state_revision,
            prepared.yrs_state_epoch,
            prepared.schema_fingerprint,
            context.resource_limits,
            context.editing_limits,
            context.max_length,
            canonical_schema,
        )?;
    }
    let cached_view = engine_view
        .map(|view| validate_cached_compilation_view(request_id, context, view))
        .transpose()?;
    let document_text_bytes = cached_view
        .map(|view| view.document_text_bytes)
        .or_else(|| base_document_text_bytes(context.document))
        .ok_or_else(|| {
            OperationError::operation_limit_exceeded(
                request_id,
                None,
                "maxInputBytes",
                u64::try_from(context.resource_limits.max_input_bytes).unwrap_or(u64::MAX),
                u64::MAX,
            )
        })?;
    let charged_scan_work = admit_yrs_scan_work(
        request_id,
        admitted_input_bytes,
        document_text_bytes,
        txn,
        context.resource_limits,
    )?;
    let localized_insert_admission = engine_view.and_then(|view| {
        let [TypedOperation::InsertText { at, .. }] = transaction.operations.as_slice() else {
            return None;
        };
        resolve_position(
            request_id,
            Some(0),
            "at",
            *at,
            view.cached.rendered_text,
            view.cached.position_map,
            view.cached.document,
        )
        .ok()
        .and_then(|document_position| {
            view.authority
                .installed()
                .admit_existing_text_insert_with_authority(
                    &transaction,
                    prepared_semantics.is_some(),
                    document_position,
                    txn,
                    fragment,
                    view.authority.lookup_seed(request_id).ok()?,
                    view.authority.materialized_identity(),
                    view.schema_fingerprint,
                    context.resource_limits,
                    context.editing_limits,
                    context.max_length,
                    view.yrs_state_epoch,
                )
        })
    });
    let mut localized_compiler = None;
    if let (Some(view), [TypedOperation::InsertText { at, text, marks: _ }]) =
        (engine_view, transaction.operations.as_slice())
    {
        if !text.is_empty() && !matches!(transaction.selection_intent, SelectionIntent::Set(_)) {
            if let Ok(position) = resolve_position(
                request_id,
                Some(0),
                "at",
                *at,
                view.cached.rendered_text,
                view.cached.position_map,
                view.cached.document,
            ) {
                if let Some(block) = view
                    .cached
                    .position_map
                    .find_block_for_doc_pos(position)
                    .and_then(|index| view.cached.position_map.block(index))
                {
                    if let Ok(authority_lookup_seed) = view.authority.lookup_seed(request_id) {
                        if let Some(localized) = LocalizedInsertCompiler::try_new(
                            request_id,
                            txn,
                            fragment,
                            context.schema,
                            action_limit,
                            context.resource_limits.max_input_bytes,
                            charged_scan_work,
                            LocalizedInsertLocator {
                                document: context.document,
                                block_path: block.node_path.as_slice(),
                                position,
                            },
                            authority_lookup_seed.as_ref(),
                            context.resource_limits,
                            context.editing_limits,
                            context.max_length,
                            view.schema_fingerprint,
                            view.yrs_state_epoch,
                            context.document_revision,
                        )? {
                            localized_compiler = Some(localized);
                        }
                    }
                }
            }
        }
    }
    let mut localized_format_compiler = None;
    let mut localized_root_window_compiler = None;
    #[cfg(test)]
    let localized_format_candidate = prepared_semantics.is_some()
        && matches!(
            transaction.operations.as_slice(),
            [TypedOperation::AddMark { .. } | TypedOperation::RemoveMark { .. }]
        );
    #[cfg(test)]
    let localized_root_window_candidate = prepared_semantics.is_some()
        && matches!(
            transaction.operations.as_slice(),
            [TypedOperation::ReplaceStructure(replacement)] if replacement.parent_path().is_empty()
        );
    if prepared_semantics.is_some() {
        if let (Some(view), [operation]) = (engine_view, transaction.operations.as_slice()) {
            let range = match operation {
                TypedOperation::AddMark { range, .. }
                | TypedOperation::RemoveMark { range, .. } => Some(*range),
                _ => None,
            };
            if let Some(range) = range {
                if let Ok((from, to)) = resolve_range(
                    request_id,
                    0,
                    range,
                    view.cached.rendered_text,
                    view.cached.position_map,
                    view.cached.document,
                    &StepMap::empty(),
                ) {
                    if let Some(last) = to.checked_sub(1) {
                        let from_block = view.cached.position_map.find_block_for_doc_pos(from);
                        let to_block = view.cached.position_map.find_block_for_doc_pos(last);
                        if from < to && from_block == to_block {
                            if let Some(block) =
                                from_block.and_then(|index| view.cached.position_map.block(index))
                            {
                                if let Ok(authority_lookup_seed) =
                                    view.authority.lookup_seed(request_id)
                                {
                                    if let Some(locator) = LocalizedFormatLocator::mint(
                                        context.document,
                                        block.node_path.as_slice(),
                                        from,
                                        to,
                                        authority_lookup_seed.as_ref(),
                                        txn,
                                        fragment,
                                        context.resource_limits,
                                        context.editing_limits,
                                        context.max_length,
                                        view.schema_fingerprint,
                                        view.yrs_state_epoch,
                                        context.document_revision,
                                    ) {
                                        localized_format_compiler =
                                            LocalizedFormatCompiler::try_new(
                                                request_id,
                                                txn,
                                                fragment,
                                                context.schema,
                                                action_limit,
                                                context.resource_limits.max_input_bytes,
                                                charged_scan_work,
                                                locator,
                                                view.schema_fingerprint,
                                                view.yrs_state_epoch,
                                                context.document_revision,
                                            )?;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let (Some(prepared), Some(view), [TypedOperation::ReplaceStructure(replacement)]) = (
        prepared_semantics,
        engine_view,
        transaction.operations.as_slice(),
    ) {
        if let Ok(authority_lookup_seed) = view.authority.lookup_seed(request_id) {
            if let Some(locator) = LocalizedRootWindowLocator::mint(
                request_id,
                context.document,
                prepared.expected_preview,
                replacement,
                authority_lookup_seed.as_ref(),
                txn,
                fragment,
                context.resource_limits,
                context.editing_limits,
                context.max_length,
                view.schema_fingerprint,
                view.yrs_state_epoch,
                context.document_revision,
            )? {
                localized_root_window_compiler = LocalizedRootWindowCompiler::try_new(
                    request_id,
                    txn,
                    fragment,
                    context.schema,
                    action_limit,
                    context.resource_limits.max_input_bytes,
                    charged_scan_work,
                    locator,
                )?;
            }
        }
    }
    let localized_semantic = if localized_compiler.is_some() {
        engine_view.and_then(|view| {
            let admission = localized_insert_admission.as_ref()?;
            let [TypedOperation::InsertText { at, .. }] = transaction.operations.as_slice() else {
                return None;
            };
            let document_position = resolve_position(
                request_id,
                Some(0),
                "at",
                *at,
                view.cached.rendered_text,
                view.cached.position_map,
                view.cached.document,
            )
            .ok()?;
            let validated = admission.validate_current_with_authority(
                view.authority.installed(),
                &transaction,
                document_position,
                txn,
                fragment,
                view.authority.lookup_seed(request_id).ok()?,
                view.authority.materialized_identity(),
                context.resource_limits,
                context.editing_limits,
                context.max_length,
                view.yrs_state_epoch,
            )?;
            let localized = try_localized_semantic_compilation(context, &transaction, &validated)?;
            if let Some(prepared) = prepared_semantics {
                if localized.preview != *prepared.expected_preview {
                    return None;
                }
            }
            Some(localized)
        })
    } else {
        None
    };
    let mutation_lowering = if let Some(localized) = localized_compiler {
        TransactionMutationLowering::LocalizedInsert(Box::new(localized))
    } else if let Some(localized) = localized_format_compiler {
        TransactionMutationLowering::LocalizedFormat(Box::new(localized))
    } else if let Some(localized) = localized_root_window_compiler {
        TransactionMutationLowering::LocalizedRootWindow(Box::new(localized))
    } else {
        #[cfg(test)]
        if localized_format_candidate {
            super::mutation::record_range_format_eager_fallback_for_test();
        }
        #[cfg(test)]
        if localized_root_window_candidate {
            super::mutation::record_root_window_eager_fallback_for_test();
        }
        TransactionMutationLowering::Eager(Box::new(MutationCompiler::new(
            request_id,
            txn,
            fragment,
            context.schema,
            action_limit,
            context.resource_limits.max_input_bytes,
            charged_scan_work,
        )?))
    };
    let mut load_crdt_envelope = |mutation_scan_work: usize| {
        let envelope = crdt_envelope(
            request_id,
            txn,
            context.resource_limits.max_encoded_state_bytes,
        )?;
        let reconciled = mutation_scan_work
            .checked_add(envelope.scan_work)
            .ok_or_else(|| {
                input_limit_error(
                    request_id,
                    None,
                    context.resource_limits.max_input_bytes,
                    usize::MAX,
                )
            })?;
        if reconciled > context.resource_limits.max_input_bytes {
            return Err(input_limit_error(
                request_id,
                None,
                context.resource_limits.max_input_bytes,
                reconciled,
            ));
        }
        Ok(envelope)
    };
    let mut compiled = compile_transaction_impl(
        context,
        &transaction,
        Some(mutation_lowering),
        Some(&mut load_crdt_envelope),
        stored_marks,
        cached_view,
        SemanticCompilationShortcuts {
            prepared: prepared_semantics,
            localized: localized_semantic,
        },
    )?;
    compiled.localized_insert_admission = localized_insert_admission;
    compiled.relative_selection_plan =
        match (&compiled.selection_plan, &transaction.selection_intent) {
            (SelectionPlan::Preserve, _) => RelativeSelectionPlan::Preserve,
            (SelectionPlan::Mapped(selection), _) => {
                RelativeSelectionPlan::PreserveWithFallback(selection.clone())
            }
            (SelectionPlan::Explicit(selection), SelectionIntent::Set(_)) => {
                RelativeSelectionPlan::Precomputed {
                    relative: planned_relative_selection(
                        context,
                        &transaction,
                        txn,
                        fragment,
                        cached_view,
                    )?
                    .ok_or_else(|| {
                        OperationError::engine_invariant_failed(
                            request_id,
                            None,
                            "explicit Set selection has no relative plan",
                        )
                    })?,
                    fallback: selection.clone(),
                }
            }
            (SelectionPlan::Explicit(_), SelectionIntent::UseOperationResult) => {
                RelativeSelectionPlan::OperationResult
            }
            (SelectionPlan::Explicit(_), SelectionIntent::Preserve) => {
                return Err(OperationError::engine_invariant_failed(
                    request_id,
                    None,
                    "Preserve selection unexpectedly compiled as explicit",
                ));
            }
        };
    // The server owns this read view through compilation and preflight. The
    // plan's document guard was captured only after the CRDT clock scan and
    // its input-work reservation above admitted full snapshot construction.
    // Preflight checks that sealed snapshot before any eager Yrs target reads.
    #[cfg(test)]
    check_atomic_failpoint(request_id, AtomicFailpoint::MutationPreflight)?;
    preflight_mutation_plan(request_id, &compiled.mutation_plan, txn)?;
    if compiled.localized_semantic_used {
        compiled.prepared_derived_evidence = engine_view.and_then(|view| {
            let admission = compiled.localized_insert_admission.as_ref()?;
            let [TypedOperation::InsertText { at, .. }] = transaction.operations.as_slice() else {
                return None;
            };
            let document_position = resolve_position(
                request_id,
                Some(0),
                "at",
                *at,
                view.cached.rendered_text,
                view.cached.position_map,
                view.cached.document,
            )
            .ok()?;
            let validated = admission.validate_current_with_authority(
                view.authority.installed(),
                &transaction,
                document_position,
                txn,
                fragment,
                view.authority.lookup_seed(request_id).ok()?,
                view.authority.materialized_identity(),
                context.resource_limits,
                context.editing_limits,
                context.max_length,
                view.yrs_state_epoch,
            )?;
            validated.prepare_derived_evidence(
                &compiled.preview,
                compiled.canonical_artifact.as_ref()?,
                compiled.preview_derivations.as_ref()?,
            )
        });
    }
    Ok(compiled)
}

fn validate_cached_compilation_view<'a>(
    request_id: u64,
    context: CompilationContext<'_>,
    engine_view: EngineCompilationView<'a>,
) -> OperationResult<CachedCompilationView<'a>> {
    let cached = engine_view.cached;
    let selection_matches = context
        .selection
        .is_some_and(|selection| std::ptr::eq(selection, cached.selection));
    if !std::ptr::eq(context.document, cached.document)
        || !selection_matches
        || context.document_revision != cached.document_revision
        || engine_view.state_revision != cached.state_revision
        || engine_view.schema_fingerprint != cached.schema_fingerprint
        || cached.rendered_scalars != cached.position_map.total_scalars()
        || cached.document_node_count == 0
    {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "cached semantic compilation view does not match the live engine state",
        ));
    }
    Ok(cached)
}

pub(crate) fn admit_transaction_envelope(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
) -> OperationResult<usize> {
    let request_id = transaction.request_id;
    if transaction.base_document_revision != context.document_revision {
        return Err(OperationError::revision_mismatch(
            request_id,
            transaction.base_document_revision,
            context.document_revision,
        ));
    }
    if !matches!(
        transaction.origin,
        TransactionOrigin::LocalInput
            | TransactionOrigin::LocalCommand
            | TransactionOrigin::LocalApi
    ) {
        return Err(OperationError::transaction_invalid(
            request_id,
            "origin",
            "typed editing transactions require a local input, command, or API origin",
        ));
    }
    let mut work = CheckedWork::default();
    work.charge_operations(
        request_id,
        transaction.operations.len(),
        context.editing_limits.max_operations_per_transaction,
    )?;

    let mut input_bytes = 0usize;
    for (operation_index, operation) in transaction.operations.iter().enumerate() {
        let amount = match operation {
            TypedOperation::ReplaceStructure(replacement) => Some(checked_fragment_input_bytes(
                request_id,
                operation_index,
                replacement.content(),
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::InsertText { text, marks, .. } => {
                if text.is_empty() {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "text",
                        "insert text must not be empty",
                    ));
                }
                text.len().checked_add(checked_mark_set_input_bytes(
                    request_id,
                    operation_index,
                    marks,
                    context.resource_limits,
                    input_bytes.saturating_add(text.len()),
                )?)
            }
            TypedOperation::DeleteRange { .. } => Some(0),
            TypedOperation::ReplaceRange { content, .. } => Some(checked_fragment_input_bytes(
                request_id,
                operation_index,
                content,
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::AddMark { mark, .. } | TypedOperation::ReplaceMark { mark, .. } => {
                Some(checked_mark_input_bytes(
                    request_id,
                    operation_index,
                    mark,
                    context.resource_limits,
                    input_bytes,
                )?)
            }
            TypedOperation::RemoveMark { mark_type, .. } => Some(mark_type.len()),
            TypedOperation::InsertNode { node, .. } => Some(checked_node_input_bytes(
                request_id,
                operation_index,
                node,
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::UpdateNodeAttrs { attrs, .. } => Some(checked_attrs_input_bytes(
                request_id,
                operation_index,
                attrs,
                context.resource_limits,
                input_bytes,
            )?),
            TypedOperation::SplitBlock {
                node_type, attrs, ..
            } => node_type.len().checked_add(checked_attrs_input_bytes(
                request_id,
                operation_index,
                attrs,
                context.resource_limits,
                input_bytes.saturating_add(node_type.len()),
            )?),
            TypedOperation::JoinBlocks { .. } => Some(0),
            TypedOperation::WrapInList {
                list_type,
                item_type,
                attrs,
                item_attrs,
                ..
            } => {
                let attrs_bytes = checked_attrs_input_bytes(
                    request_id,
                    operation_index,
                    attrs,
                    context.resource_limits,
                    input_bytes
                        .saturating_add(list_type.len())
                        .saturating_add(item_type.len()),
                )?;
                let item_attrs_bytes = checked_attrs_input_bytes(
                    request_id,
                    operation_index,
                    item_attrs,
                    context.resource_limits,
                    input_bytes
                        .saturating_add(list_type.len())
                        .saturating_add(item_type.len())
                        .saturating_add(attrs_bytes),
                )?;
                list_type
                    .len()
                    .checked_add(item_type.len())
                    .and_then(|amount| amount.checked_add(attrs_bytes))
                    .and_then(|amount| amount.checked_add(item_attrs_bytes))
            }
            TypedOperation::UnwrapFromList { .. }
            | TypedOperation::IndentListItem { .. }
            | TypedOperation::OutdentListItem { .. } => Some(0),
        }
        .ok_or_else(|| input_work_overflow(request_id, operation_index, context))?;
        charge_input(
            &mut input_bytes,
            amount,
            request_id,
            operation_index,
            context.resource_limits.max_input_bytes,
        )?;
    }
    Ok(input_bytes)
}

fn input_limit_error(
    request_id: u64,
    operation_index: Option<usize>,
    limit: usize,
    actual: usize,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        operation_index,
        "maxInputBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::try_from(actual).unwrap_or(u64::MAX),
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

pub(crate) fn admit_yrs_scan_work<T: yrs::ReadTxn>(
    request_id: u64,
    admitted_input_bytes: usize,
    document_text_bytes: usize,
    txn: &T,
    resource_limits: &ResourceLimits,
) -> OperationResult<usize> {
    let crdt_clock_work =
        crdt_clock_scan_reservation(request_id, txn, resource_limits.max_encoded_state_bytes)?;
    // One pass materializes each Yrs text and one pass builds its scalar/UTF-16 index.
    // Reserve both before any selection or mutation traversal. Selection-only callers
    // may supply the exact document-text metric cached at the last document change.
    let initial_scan_work = document_text_bytes
        .checked_mul(2)
        .and_then(|work| work.checked_add(crdt_clock_work.checked_mul(2)?))
        .ok_or_else(|| {
            input_limit_error(
                request_id,
                None,
                resource_limits.max_input_bytes,
                usize::MAX,
            )
        })?;
    let charged_scan_work = admitted_input_bytes
        .checked_add(initial_scan_work)
        .ok_or_else(|| {
            input_limit_error(
                request_id,
                None,
                resource_limits.max_input_bytes,
                usize::MAX,
            )
        })?;
    if charged_scan_work > resource_limits.max_input_bytes {
        return Err(input_limit_error(
            request_id,
            None,
            resource_limits.max_input_bytes,
            charged_scan_work,
        ));
    }
    Ok(charged_scan_work)
}

fn compile_transaction_impl(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
    mutation_lowering: Option<TransactionMutationLowering>,
    mut crdt_envelope_loader: Option<&mut dyn FnMut(usize) -> OperationResult<CrdtEnvelope>>,
    stored_marks_context: Option<StoredMarksCompilationContext<'_>>,
    cached_view: Option<CachedCompilationView<'_>>,
    semantic_shortcuts: SemanticCompilationShortcuts<'_>,
) -> OperationResult<CompiledTransaction> {
    let SemanticCompilationShortcuts {
        prepared: prepared_semantics,
        localized: mut localized_semantic,
    } = semantic_shortcuts;
    let (
        mut lowering,
        mut localized_insert,
        mut localized_format,
        mut localized_root_window,
        mut prelowered_plan,
        mut prelowered_lookup_transition,
    ) = match mutation_lowering {
        Some(TransactionMutationLowering::Eager(compiler)) => {
            (Some(*compiler), None, None, None, None, None)
        }
        Some(TransactionMutationLowering::LocalizedInsert(localized)) => {
            (None, Some(*localized), None, None, None, None)
        }
        Some(TransactionMutationLowering::LocalizedFormat(localized)) => {
            (None, None, Some(*localized), None, None, None)
        }
        Some(TransactionMutationLowering::LocalizedRootWindow(localized)) => {
            (None, None, None, Some(*localized), None, None)
        }
        None => (None, None, None, None, None, None),
    };
    #[cfg(test)]
    SEMANTIC_COMPILATION_COUNT.set(SEMANTIC_COMPILATION_COUNT.get().saturating_add(1));
    let request_id = transaction.request_id;
    let mut work = CheckedWork::default();
    // Revision, origin, operation count and aggregate input were admitted by the
    // single shared envelope path before any optional Yrs target traversal.
    debug_assert_eq!(
        transaction.base_document_revision,
        context.document_revision
    );
    debug_assert!(matches!(
        transaction.origin,
        TransactionOrigin::LocalInput
            | TransactionOrigin::LocalCommand
            | TransactionOrigin::LocalApi
    ));
    debug_assert!(
        transaction.operations.len() <= context.editing_limits.max_operations_per_transaction
    );

    let owned_base_position_map;
    let owned_rendered_text;
    let (base_position_map, rendered_text, rendered_scalars) = if let Some(cached) = cached_view {
        (
            cached.position_map,
            cached.rendered_text,
            cached.rendered_scalars,
        )
    } else {
        owned_base_position_map = base_position_map(context.document, context.schema);
        owned_rendered_text = base_rendered_text(context.document, context.schema);
        let rendered_scalars = u32::try_from(owned_rendered_text.chars().count()).ok();
        (
            &owned_base_position_map,
            owned_rendered_text.as_str(),
            rendered_scalars.unwrap_or(u32::MAX),
        )
    };
    if rendered_scalars != base_position_map.total_scalars() {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "rendered text and base position map have different scalar lengths",
        ));
    }
    let yrs_lowering_requested = lowering.is_some()
        || localized_insert.is_some()
        || localized_format.is_some()
        || localized_root_window.is_some();
    let mut preview = Cow::Borrowed(context.document);
    let mut composed_map = StepMap::empty();
    let mut operation_result = None;
    let mut undo_units_bound = 0u64;
    let mut undo_limit_error = None;
    let mut history_class = HistoryClass::Skip;
    let records_history = transaction.history_policy != HistoryPolicy::Skip;
    let mut canonical_artifact = (!transaction.operations.is_empty())
        .then(|| cached_view.map(|cached| cached.canonical_artifact.clone()))
        .flatten();
    let owned_canonical_schema;
    let canonical_schema = if let Some(cached) = cached_view {
        cached.canonical_artifact.schema_context()
    } else {
        owned_canonical_schema = CanonicalSchemaContext::new(context.schema);
        &owned_canonical_schema
    };
    let mut stored_marks_state = stored_marks_context
        .as_ref()
        .map(|state| state.stored_marks.map(<[Mark]>::to_vec));
    // Set when a caret-anchored SplitBlock carried the stored marks through.
    // Return moves the caret across the new block boundary, so the generic
    // "caret mapped to the same place" compatibility check cannot see it.
    let mut split_at_caret_kept_stored_marks = false;
    let tracked_caret = stored_marks_context.as_ref().and_then(|state| {
        let super::ResolvedSelection::Text { anchor, head } = state.resolved_selection else {
            return None;
        };
        let super::RelativeSelection::Text {
            head: relative_head,
            ..
        } = state.relative_selection
        else {
            return None;
        };
        (anchor.document == head.document).then_some((head.document, relative_head.affinity))
    });
    let mut localized_derivations = None;
    for (operation_index, operation) in transaction.operations.iter().enumerate() {
        let tracked_caret_for_operation = tracked_caret
            .map(|(position, affinity)| map_position(&composed_map, position, affinity));
        let mut stored_marks_input = None;
        let mut inherited_marks = None;
        let mut operation_changed;
        let mut compatible_text_delete = false;
        match operation {
            TypedOperation::ReplaceStructure(replacement) => {
                if transaction.operations.len() != 1 {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "structure",
                        "sealed structural replacement must be the transaction's only operation",
                    ));
                }
                validate_fragment_marks(
                    request_id,
                    operation_index,
                    replacement.content(),
                    context.schema,
                )?;
                let (from, to) = resolve_structural_window(
                    request_id,
                    operation_index,
                    &preview,
                    replacement,
                    context.resource_limits,
                )?;
                stored_marks_input = Some((from, to));
                let step = Step::ReplaceRange {
                    from,
                    to,
                    content: replacement.content().clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "structure", error)
                    })?;
                if lowering.is_some()
                    && !prepared_candidate_matches(
                        prepared_semantics,
                        transaction.operations.len(),
                        operation_index,
                        &next,
                        context,
                        canonical_schema,
                    )
                {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            lowering,
                        )?;
                        lowering.replace_structural_range(
                            operation_index,
                            MutationDocumentContext {
                                before: &preview,
                                after: &next,
                                schema: context.schema,
                                limits: context.resource_limits,
                            },
                            ReplacementInput {
                                from,
                                to,
                                boundaries: &boundaries,
                                content: replacement.content(),
                            },
                        )?;
                    } else if localized_root_window.is_some() {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            localized_root_window
                                .as_mut()
                                .expect("localized root-window compiler was checked"),
                        )?;
                        let plan = localized_root_window
                            .take()
                            .expect("localized root-window compiler was checked")
                            .replace_structural_range(
                                operation_index,
                                MutationDocumentContext {
                                    before: &preview,
                                    after: &next,
                                    schema: context.schema,
                                    limits: context.resource_limits,
                                },
                                ReplacementInput {
                                    from,
                                    to,
                                    boundaries: &boundaries,
                                    content: replacement.content(),
                                },
                            )?;
                        prelowered_plan = Some(plan);
                        prelowered_lookup_transition =
                            Some(MutationLookupTransition::Invalidate { request_id });
                    }
                }
                operation_result = Some(replacement.selection_after().clone());
                composed_map = composed_map.compose(&step_map);
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from)
                            .saturating_add(u64::from(replacement.content().size())),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            TypedOperation::InsertText { at, text, marks } => {
                let localized = localized_semantic.take();
                let (pos, next, step_map) = if let Some(localized) = localized {
                    let LocalizedSemanticCompilation {
                        position,
                        preview,
                        step_map,
                        derivations,
                    } = localized;
                    localized_derivations = Some(derivations);
                    (position, preview, step_map)
                } else {
                    validate_operation_marks(request_id, operation_index, marks, context.schema)?;
                    let base_pos = resolve_position(
                        request_id,
                        Some(operation_index),
                        "at",
                        *at,
                        rendered_text,
                        base_position_map,
                        context.document,
                    )?;
                    let pos = map_position(&composed_map, base_pos, at.affinity);
                    let step = Step::InsertText {
                        pos,
                        text: text.clone(),
                        marks: marks.clone(),
                    };
                    let (next, step_map) = crate::transform::apply_step_canonical_marks(
                        &preview,
                        &step,
                        context.schema,
                    )
                    .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                    if lowering.is_some() || localized_insert.is_some() {
                        validate_preview(request_id, Some(operation_index), &next, context)?;
                    }
                    (pos, next, step_map)
                };
                stored_marks_input = Some((pos, pos));
                if let Some(lowering) = &mut lowering {
                    lowering.insert(operation_index, pos, text, marks)?;
                } else if let Some(localized) = localized_insert.take() {
                    let (plan, promotion) =
                        localized.compile_with_promotion(operation_index, pos, text, marks)?;
                    prelowered_plan = Some(plan);
                    prelowered_lookup_transition =
                        Some(MutationLookupTransition::Promote(promotion));
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        text.chars().count() as u64,
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Insert);
            }
            TypedOperation::DeleteRange { range } => {
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                let step = Step::DeleteRange { from, to };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    let boundaries = text_boundaries(
                        request_id,
                        operation_index,
                        &preview,
                        context.schema,
                        lowering,
                    )?;
                    match lowering.delete(operation_index, from, to, &boundaries)? {
                        TextRangeDisposition::Applied => compatible_text_delete = true,
                        TextRangeDisposition::Structural => {
                            lowering.delete_structural_range(
                                operation_index,
                                &preview,
                                from,
                                to,
                            )?;
                        }
                    }
                }
                operation_result = Some(Selection::cursor(from));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Delete);
            }
            TypedOperation::ReplaceRange { range, content } => {
                validate_fragment_marks(request_id, operation_index, content, context.schema)?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                let step = Step::ReplaceRange {
                    from,
                    to,
                    content: content.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            lowering,
                        )?;
                        lowering.replace(
                            operation_index,
                            MutationDocumentContext {
                                before: &preview,
                                after: &next,
                                schema: context.schema,
                                limits: context.resource_limits,
                            },
                            ReplacementInput {
                                from,
                                to,
                                boundaries: &boundaries,
                                content,
                            },
                        )?;
                    }
                }
                operation_result = Some(Selection::cursor(from.saturating_add(content.size())));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from).saturating_add(u64::from(content.size())),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                let class = if content.size() == 0 {
                    HistoryClass::Delete
                } else if from == to {
                    HistoryClass::Insert
                } else {
                    HistoryClass::Structural
                };
                if operation_changed {
                    history_class = merge_history_class(history_class, class);
                }
            }
            TypedOperation::AddMark { range, mark } => {
                validate_operation_marks(
                    request_id,
                    operation_index,
                    std::slice::from_ref(mark),
                    context.schema,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                if from == to {
                    inherited_marks = Some(super::derived_state::marks_at_position(&preview, from));
                }
                if add_mark_conflicts_with_existing_attrs(&preview, from, to, mark) {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "mark",
                        "AddMark conflicts with an existing same-type mark; use ReplaceMark",
                    ));
                }
                let step = Step::AddMark {
                    from,
                    to,
                    mark: mark.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some()
                    && !prepared_candidate_matches(
                        prepared_semantics,
                        transaction.operations.len(),
                        operation_index,
                        &next,
                        context,
                        canonical_schema,
                    )
                {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            lowering,
                        )?;
                        lowering.format(operation_index, from, to, &boundaries, mark_attr(mark))?;
                    } else if localized_format.is_some() {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            localized_format
                                .as_mut()
                                .expect("localized format capability is present"),
                        )?;
                        let (plan, promotion) = localized_format
                            .take()
                            .expect("localized format capability is present")
                            .format(operation_index, from, to, &boundaries, mark_attr(mark))?;
                        prelowered_plan = Some(plan);
                        prelowered_lookup_transition =
                            Some(MutationLookupTransition::Promote(promotion));
                    }
                }
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Format);
            }
            TypedOperation::RemoveMark { range, mark_type } => {
                if context.schema.mark(mark_type).is_none() {
                    return Err(OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "markType",
                        format!("unknown mark '{mark_type}'"),
                    ));
                }
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                if from == to {
                    inherited_marks = Some(super::derived_state::marks_at_position(&preview, from));
                }
                let step = Step::RemoveMark {
                    from,
                    to,
                    mark_type: mark_type.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some()
                    && !prepared_candidate_matches(
                        prepared_semantics,
                        transaction.operations.len(),
                        operation_index,
                        &next,
                        context,
                        canonical_schema,
                    )
                {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            lowering,
                        )?;
                        lowering.format(
                            operation_index,
                            from,
                            to,
                            &boundaries,
                            removed_mark_attr(mark_type),
                        )?;
                    } else if localized_format.is_some() {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            localized_format
                                .as_mut()
                                .expect("localized format capability is present"),
                        )?;
                        let (plan, promotion) = localized_format
                            .take()
                            .expect("localized format capability is present")
                            .format(
                                operation_index,
                                from,
                                to,
                                &boundaries,
                                removed_mark_attr(mark_type),
                            )?;
                        prelowered_plan = Some(plan);
                        prelowered_lookup_transition =
                            Some(MutationLookupTransition::Promote(promotion));
                    }
                }
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(to - from),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Format);
            }
            TypedOperation::ReplaceMark { range, mark } => {
                validate_operation_marks(
                    request_id,
                    operation_index,
                    std::slice::from_ref(mark),
                    context.schema,
                )?;
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                stored_marks_input = Some((from, to));
                if from == to {
                    inherited_marks = Some(super::derived_state::marks_at_position(&preview, from));
                }
                let remove = Step::RemoveMark {
                    from,
                    to,
                    mark_type: mark.mark_type().to_string(),
                };
                let (without, remove_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &remove, context.schema)
                        .map_err(|error| {
                            map_transform_error(request_id, operation_index, "range", error)
                        })?;
                let add = Step::AddMark {
                    from,
                    to,
                    mark: mark.clone(),
                };
                let (next, add_map) =
                    crate::transform::apply_step_canonical_marks(&without, &add, context.schema)
                        .map_err(|error| {
                            map_transform_error(request_id, operation_index, "range", error)
                        })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        let boundaries = text_boundaries(
                            request_id,
                            operation_index,
                            &preview,
                            context.schema,
                            lowering,
                        )?;
                        lowering.format(operation_index, from, to, &boundaries, mark_attr(mark))?;
                    }
                }
                let step_map = remove_map.compose(&add_map);
                operation_result = Some(Selection::text(from, to));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    for _ in 0..2 {
                        charge_undo_bound(
                            &mut undo_units_bound,
                            &mut undo_limit_error,
                            u64::from(to - from),
                            request_id,
                            operation_index,
                            context.editing_limits.max_undo_retained_units,
                        );
                    }
                }
                history_class = merge_history_class(history_class, HistoryClass::Format);
            }
            TypedOperation::InsertNode { at, node } => {
                let resolver_limit = lowering.as_ref().map_or(
                    context.resource_limits.max_input_bytes,
                    MutationCompiler::remaining_scan_work,
                );
                let resolver_budget = WorkBudget::new(resolver_limit);
                let base_pos = resolve_insert_node_position(
                    InsertNodeResolverContext {
                        request_id,
                        operation_index,
                        document: context.document,
                        node,
                        schema: context.schema,
                        budget: &resolver_budget,
                        limit: context.resource_limits.max_input_bytes,
                    },
                    *at,
                    rendered_text,
                    base_position_map,
                )?;
                let mapped_pos = map_position(&composed_map, base_pos, at.affinity);
                let pos = normalize_current_insert_node_position(
                    InsertNodeResolverContext {
                        request_id,
                        operation_index,
                        document: &preview,
                        node,
                        schema: context.schema,
                        budget: &resolver_budget,
                        limit: context.resource_limits.max_input_bytes,
                    },
                    mapped_pos,
                    at.affinity,
                )?;
                if let Some(lowering) = &mut lowering {
                    lowering.charge_position_resolver_work(
                        operation_index,
                        resolver_budget.consumed(resolver_limit),
                    )?;
                }
                let step = Step::InsertNode {
                    pos,
                    node: node.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.insert_structural_node(
                        operation_index,
                        MutationDocumentContext {
                            before: &preview,
                            after: &next,
                            schema: context.schema,
                            limits: context.resource_limits,
                        },
                        pos,
                        node,
                    )?;
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::from(node.node_size()),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::SplitBlock {
                at,
                node_type,
                attrs,
            } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                // Captured against the pre-split document: these are the marks
                // the next typed character would have inherited had Return not
                // been pressed.
                inherited_marks = Some(super::derived_state::marks_at_position(&preview, pos));
                let step = Step::SplitBlock {
                    pos,
                    node_type: node_type.clone(),
                    attrs: attrs.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.split_block(
                        operation_index,
                        &preview,
                        &next,
                        pos,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                stored_marks_input = Some((pos, pos));
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        2,
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::JoinBlocks { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let pos = resolve_join_target_position(request_id, operation_index, &preview, pos)?;
                let step = Step::JoinBlocks { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.join_blocks(
                        operation_index,
                        &preview,
                        pos,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::WrapInList {
                range,
                list_type,
                item_type,
                attrs,
                item_attrs,
            } => {
                let (from, to) = resolve_range(
                    request_id,
                    operation_index,
                    *range,
                    rendered_text,
                    base_position_map,
                    context.document,
                    &composed_map,
                )?;
                let step = Step::WrapInList {
                    from,
                    to,
                    list_type: list_type.clone(),
                    item_type: item_type.clone(),
                    attrs: attrs.clone(),
                    item_attrs: item_attrs.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "range", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.wrap_in_list(
                        operation_index,
                        &preview,
                        &next,
                        from,
                        to,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                operation_result = Some(Selection::text(from, step_map.map_pos(to)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::UnwrapFromList { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::UnwrapFromList { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                if let Some(lowering) = &mut lowering {
                    lowering.unwrap_from_list(
                        operation_index,
                        &preview,
                        &next,
                        pos,
                        context.schema,
                        context.resource_limits,
                    )?;
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                history_class = merge_history_class(history_class, HistoryClass::Structural);
            }
            TypedOperation::IndentListItem { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::IndentListItem { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        lowering.indent_list_item(
                            operation_index,
                            &preview,
                            &next,
                            pos,
                            context.schema,
                            context.resource_limits,
                        )?;
                    }
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            TypedOperation::OutdentListItem { at } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let step = Step::OutdentListItem { pos };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        lowering.outdent_list_item(
                            operation_index,
                            &preview,
                            &next,
                            pos,
                            context.schema,
                            context.resource_limits,
                        )?;
                    }
                }
                operation_result = Some(Selection::cursor(step_map.map_pos(pos)));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
            TypedOperation::UpdateNodeAttrs { at, attrs } => {
                let base_pos = resolve_position(
                    request_id,
                    Some(operation_index),
                    "at",
                    *at,
                    rendered_text,
                    base_position_map,
                    context.document,
                )?;
                let pos = map_position(&composed_map, base_pos, at.affinity);
                let pos = resolve_attribute_target_position(
                    request_id,
                    operation_index,
                    &preview,
                    pos,
                    attrs,
                    context.schema,
                )?;
                let step = Step::UpdateNodeAttrs {
                    pos,
                    attrs: attrs.clone(),
                };
                let (next, step_map) =
                    crate::transform::apply_step_canonical_marks(&preview, &step, context.schema)
                        .map_err(|error| {
                        map_transform_error(request_id, operation_index, "at", error)
                    })?;
                if lowering.is_some() {
                    validate_preview(request_id, Some(operation_index), &next, context)?;
                }
                operation_changed = next != *preview;
                if operation_changed {
                    if let Some(lowering) = &mut lowering {
                        lowering.update_node_attrs(
                            operation_index,
                            &preview,
                            pos,
                            attrs,
                            context.schema,
                            context.resource_limits,
                        )?;
                    }
                }
                operation_result = selectable_void_at(preview.root(), pos, 0, context.schema)
                    .then(|| Selection::node(pos));
                composed_map = composed_map.compose(&step_map);
                operation_changed = next != *preview;
                preview = Cow::Owned(next);
                if records_history && operation_changed {
                    charge_undo_bound(
                        &mut undo_units_bound,
                        &mut undo_limit_error,
                        u64::try_from(checked_attrs_input_bytes(
                            request_id,
                            operation_index,
                            attrs,
                            context.resource_limits,
                            0,
                        )?)
                        .unwrap_or(u64::MAX),
                        request_id,
                        operation_index,
                        context.editing_limits.max_undo_retained_units,
                    );
                }
                if operation_changed {
                    history_class = merge_history_class(history_class, HistoryClass::Structural);
                }
            }
        }
        if let Some(current) = stored_marks_state.as_mut() {
            let operation_at_caret = tracked_caret_for_operation
                .zip(stored_marks_input)
                .is_some_and(|(caret, (from, to))| from == caret && to == caret);
            let deletion_touches_caret = tracked_caret_for_operation
                .zip(stored_marks_input)
                .is_some_and(|(caret, (from, to))| from == caret || to == caret);
            match operation {
                TypedOperation::AddMark { .. }
                | TypedOperation::RemoveMark { .. }
                | TypedOperation::ReplaceMark { .. }
                    if operation_at_caret =>
                {
                    if let Some(marks) = current.as_mut() {
                        super::derived_state::apply_stored_mark_operation(
                            marks,
                            operation,
                            context.schema,
                        )
                        .map_err(|mut error| {
                            error.request_id = request_id;
                            error.operation_index = Some(operation_index);
                            error
                        })?;
                    } else {
                        let mut marks = inherited_marks.take().unwrap_or_default();
                        let changed = super::derived_state::apply_stored_mark_operation(
                            &mut marks,
                            operation,
                            context.schema,
                        )
                        .map_err(|mut error| {
                            error.request_id = request_id;
                            error.operation_index = Some(operation_index);
                            error
                        })?;
                        let materializes = match operation {
                            TypedOperation::AddMark { .. } | TypedOperation::ReplaceMark { .. } => {
                                changed
                            }
                            TypedOperation::RemoveMark { .. } => true,
                            _ => unreachable!(),
                        };
                        if materializes {
                            *current = Some(marks);
                        }
                    }
                }
                TypedOperation::InsertText { text, marks, .. }
                    if operation_changed && !text.is_empty() && operation_at_caret =>
                {
                    if let Some(effective) = current.as_ref() {
                        if super::derived_state::canonical_marks(marks, context.schema)
                            != *effective
                        {
                            *current = None;
                        }
                    }
                }
                TypedOperation::DeleteRange { .. }
                    if operation_changed && deletion_touches_caret && compatible_text_delete =>
                {
                    // A compatible text deletion carries the current stored set
                    // through to the mapped caret without changing it.
                }
                TypedOperation::SplitBlock { .. } if operation_changed && operation_at_caret => {
                    // Pressing Return carries the active formatting onto the new
                    // block, as every comparable editor does: the marks the next
                    // character would have taken before the split are the marks
                    // it takes after it. A split away from the caret is still a
                    // structural edit and falls through to the clearing arm.
                    //
                    // With no explicit stored set the active formatting is
                    // whatever the text before the caret carries, so it has to be
                    // materialised here — the new block is empty and has nothing
                    // for the next character to inherit from.
                    if current.is_none() {
                        let inherited = inherited_marks.take().unwrap_or_default();
                        if !inherited.is_empty() {
                            *current = Some(super::derived_state::canonical_marks(
                                &inherited,
                                context.schema,
                            ));
                        }
                    }
                    split_at_caret_kept_stored_marks = true;
                }
                _ if operation_changed => *current = None,
                _ => {}
            }
        }
        let prepared_candidate_matches_preview = prepared_candidate_matches(
            prepared_semantics,
            transaction.operations.len(),
            operation_index,
            &preview,
            context,
            canonical_schema,
        );
        if localized_derivations.is_none() && !prepared_candidate_matches_preview {
            validate_preview_marks(request_id, operation_index, &preview, context.schema)?;
        }
        let next_artifact = if operation_changed {
            if let Some(prepared) = prepared_semantics.filter(|prepared| {
                transaction.operations.len() == 1
                    && operation_index == 0
                    && *preview == *prepared.expected_preview
            }) {
                charge_prepared_preview_output(
                    &mut work,
                    request_id,
                    operation_index,
                    prepared.admission,
                    context,
                )?
            } else if let Some(localized) = localized_derivations.as_ref() {
                charge_localized_preview_output(
                    &mut work,
                    request_id,
                    operation_index,
                    &preview,
                    canonical_schema,
                    context,
                    localized.raw_text_scalars,
                    localized.raw_text_utf8_bytes,
                )?
            } else {
                charge_preview_output(
                    &mut work,
                    request_id,
                    operation_index,
                    &preview,
                    canonical_schema,
                    context,
                )?
            }
        } else if let Some(existing) = canonical_artifact.as_ref() {
            charge_canonical_output(&mut work, request_id, operation_index, existing, context)?;
            existing.clone()
        } else {
            charge_preview_output(
                &mut work,
                request_id,
                operation_index,
                &preview,
                canonical_schema,
                context,
            )?
        };
        canonical_artifact = Some(next_artifact);
    }

    let prepared_candidate_matches_preview = transaction
        .operations
        .len()
        .checked_sub(1)
        .is_some_and(|operation_index| {
            prepared_candidate_matches(
                prepared_semantics,
                transaction.operations.len(),
                operation_index,
                &preview,
                context,
                canonical_schema,
            )
        });
    if localized_derivations.is_none() && !prepared_candidate_matches_preview {
        validate_preview(
            request_id,
            transaction.operations.len().checked_sub(1),
            &preview,
            context,
        )?;
    }
    let prepared_candidate_validation = prepared_semantics
        .filter(|_| prepared_candidate_matches_preview)
        .and_then(|prepared| {
            preview = Cow::Borrowed(prepared.expected_preview);
            prepared.admission.candidate_validation()
        });
    let localized_semantic_used = localized_derivations.is_some();
    let affected_top_level_blocks = if let Some(localized) = localized_derivations.as_mut() {
        std::mem::take(&mut localized.affected_top_level_blocks)
    } else {
        affected_top_level_blocks(context.document, &preview)
    };
    let position_update_mode = position_update_mode(&transaction.operations);
    let preview_derivations = if let Some(prepared) = prepared_candidate_validation.as_ref() {
        let canonical_artifact = canonical_artifact.as_ref().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "prepared candidate has no canonical artifact",
            )
        })?;
        Some(
            prepared
                .compiled_derivations(
                    &preview,
                    canonical_artifact,
                    context.resource_limits,
                    context.editing_limits,
                    context.max_length,
                    prepared_semantics
                        .expect("prepared candidate retains semantic context")
                        .schema_fingerprint,
                    canonical_schema,
                )
                .ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "prepared candidate derivations do not match the live command context",
                    )
                })?,
        )
    } else if yrs_lowering_requested && *preview != *context.document {
        Some(if let Some(localized) = localized_derivations.take() {
            derive_localized_preview_document(
                request_id,
                context,
                base_position_map,
                &preview,
                &composed_map,
                position_update_mode,
                &affected_top_level_blocks,
                localized,
            )?
        } else {
            derive_preview_document(
                request_id,
                context,
                base_position_map,
                &preview,
                &composed_map,
                position_update_mode,
                &affected_top_level_blocks,
            )?
        })
    } else {
        None
    };
    let use_operation_result_falls_back_to_preserve = operation_result.is_none();
    let selection_plan = selection_plan(
        context,
        &transaction.selection_intent,
        rendered_text,
        base_position_map,
        &composed_map,
        operation_result,
        request_id,
        &preview,
        preview_derivations
            .as_ref()
            .map(|derivations| &derivations.position_map),
    )?;
    let stored_marks_plan = if let (Some(mut stored), Some(initial)) =
        (stored_marks_state, stored_marks_context.as_ref())
    {
        let after = match &selection_plan {
            SelectionPlan::Preserve => initial.resolved_selection.clone(),
            SelectionPlan::Mapped(selection) | SelectionPlan::Explicit(selection) => {
                let resolved = if let Some(derivations) = preview_derivations.as_ref() {
                    super::derived_state::resolved_from_legacy_with_view(
                        &preview,
                        selection,
                        context.schema,
                        &derivations.position_map,
                        &derivations.rendered_text,
                    )
                } else {
                    super::derived_state::resolved_from_legacy(&preview, selection, context.schema)
                };
                resolved.ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "compiled selection cannot produce resolved stored-mark state",
                    )
                })?
            }
        };
        let mapped_tracked_caret = tracked_caret
            .map(|(position, affinity)| map_position(&composed_map, position, affinity));
        let after_is_mapped_tracked_caret = mapped_tracked_caret.is_some_and(|mapped| {
            matches!(
                &after,
                super::ResolvedSelection::Text { anchor, head }
                    if anchor.document == head.document && head.document == mapped
            )
        });
        let compatible_moved_selection = match transaction.selection_intent {
            SelectionIntent::Preserve => tracked_caret.is_some(),
            SelectionIntent::UseOperationResult if use_operation_result_falls_back_to_preserve => {
                tracked_caret.is_some()
            }
            SelectionIntent::UseOperationResult => {
                after_is_mapped_tracked_caret || split_at_caret_kept_stored_marks
            }
            SelectionIntent::Set(_) => false,
        };
        let after_is_collapsed_text = matches!(
            &after,
            super::ResolvedSelection::Text { anchor, head }
                if anchor.document == head.document
        );
        if !after_is_collapsed_text
            || (initial.resolved_selection != &after && !compatible_moved_selection)
        {
            stored = None;
        } else if matches!(transaction.selection_intent, SelectionIntent::Set(_))
            || transaction.operations.is_empty()
        {
            stored = super::derived_state::stored_marks_after_selection_change(
                stored.as_deref(),
                initial.resolved_selection,
                &after,
                &preview,
                context.schema,
            );
        }
        StoredMarksPlan::Set(stored)
    } else {
        StoredMarksPlan::Unsealed
    };
    if *preview == *context.document || transaction.history_policy == HistoryPolicy::Skip {
        history_class = HistoryClass::Skip;
        undo_units_bound = 0;
        undo_limit_error = None;
    }

    let yrs_lowered = yrs_lowering_requested;
    let lowered_plan = lowering
        .map(|compiler| compiler.finish(transaction.operations.len().checked_sub(1)))
        .transpose()?
        .or(prelowered_plan)
        .unwrap_or_default();
    let mut mutation_plan = if *preview == *context.document {
        YrsMutationPlan::default()
    } else {
        lowered_plan
    };
    mutation_plan.cache_prepared_metrics(request_id)?;
    let authored_clock_units = planned_insertion_units(request_id, &mutation_plan)?;
    let crdt_envelope = if mutation_plan.requires_crdt_envelope() {
        Some(crdt_envelope_loader.as_mut().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                None,
                "Yrs live deletion plan has no snapshot envelope loader",
            )
        })?(mutation_plan.scan_work)?)
    } else {
        None
    };
    let replay_work_units_bound = if yrs_lowered {
        estimate_undo_units(request_id, &mutation_plan, crdt_envelope.as_ref())?
    } else {
        undo_units_bound
    };
    let admitted_undo_units_bound = match prepared_semantics
        .map(|prepared| prepared.admission.command_contract_oracle())
        .unwrap_or(PreparedCommandContractOracle::None)
    {
        PreparedCommandContractOracle::None => replay_work_units_bound,
        PreparedCommandContractOracle::RootWrap {
            direct_insertion_units,
            direct_growth_bytes: _,
            replaced_children: _,
        } => {
            let envelope = crdt_envelope.as_ref().ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    transaction.operations.len().checked_sub(1),
                    "prepared root-wrap undo oracle has no live deletion envelope",
                )
            })?;
            replay_work_units_bound.max(super::mutation::deleting_plan_undo_units(
                request_id,
                transaction.operations.len().saturating_sub(1),
                direct_insertion_units,
                envelope,
            )?)
        }
    };
    if yrs_lowered && history_class != HistoryClass::Skip {
        undo_units_bound = replay_work_units_bound;
        if admitted_undo_units_bound > context.editing_limits.max_undo_retained_units {
            // The semantic pass records the first operation that crosses the
            // aggregate limit. Preserve only that attribution when the exact
            // Yrs estimator confirms the failure: the reported `actual` must
            // always be the exact plan-derived bound.
            let operation_index = undo_limit_error
                .as_ref()
                .and_then(|error| error.operation_index)
                .or_else(|| transaction.operations.len().checked_sub(1));
            undo_limit_error = Some(OperationError::operation_limit_exceeded(
                request_id,
                operation_index,
                "maxUndoRetainedUnits",
                context.editing_limits.max_undo_retained_units,
                admitted_undo_units_bound,
            ));
        } else {
            undo_limit_error = None;
        }
    }
    if let Some(error) = undo_limit_error {
        return Err(error);
    }
    let encoded_growth_bound =
        estimate_update_v1_growth(request_id, &mutation_plan, crdt_envelope.as_ref())?;
    let encoded_growth_bound = match prepared_semantics
        .map(|prepared| prepared.admission.command_contract_oracle())
        .unwrap_or(PreparedCommandContractOracle::None)
    {
        PreparedCommandContractOracle::None => encoded_growth_bound,
        PreparedCommandContractOracle::RootWrap {
            direct_insertion_units,
            direct_growth_bytes,
            replaced_children,
        } => {
            let envelope = crdt_envelope.as_ref().ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    transaction.operations.len().checked_sub(1),
                    "prepared root-wrap growth oracle has no live deletion envelope",
                )
            })?;
            encoded_growth_bound.max(super::mutation::direct_xml_replacement_growth(
                request_id,
                transaction.operations.len().saturating_sub(1),
                replaced_children,
                direct_growth_bytes,
                direct_insertion_units,
                envelope,
            )?)
        }
    };
    let mutation_lookup_transition = (!mutation_plan.is_empty())
        .then_some(prelowered_lookup_transition)
        .flatten();

    Ok(CompiledTransaction {
        request_id,
        base_state_revision: cached_view.map_or(0, |cached| cached.state_revision),
        origin: transaction.origin,
        history_policy: transaction.history_policy,
        history_class,
        preview: preview.into_owned(),
        canonical_artifact,
        preview_derivations,
        selection_plan,
        affected_top_level_blocks,
        composed_map,
        position_update_mode,
        relative_selection_plan: RelativeSelectionPlan::Unsealed,
        stored_marks_plan,
        mutation_plan,
        mutation_lookup_transition,
        encoded_growth_bound,
        undo_units_bound,
        replay_work_units_bound,
        authored_clock_units,
        // Standalone compiler tests do not own an engine epoch. The engine
        // seals its current epoch onto the compiled plan before it can leave
        // the stable read view.
        yrs_state_epoch: 0,
        localized_insert_admission: None,
        prepared_derived_evidence: None,
        prepared_candidate_validation,
        prepared_active_state_transition: None,
        prepared_selection_state: None,
        prepared_selection_mutation_seal: None,
        localized_semantic_used,
    })
}

fn planned_relative_selection<T: yrs::ReadTxn>(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
    txn: &T,
    fragment: &yrs::types::xml::XmlFragmentRef,
    cached_view: Option<CachedCompilationView<'_>>,
) -> OperationResult<Option<super::RelativeSelection>> {
    let owned_rendered;
    let owned_map;
    let (rendered, map) = if let Some(cached) = cached_view {
        (cached.rendered_text, cached.position_map)
    } else {
        owned_rendered = base_rendered_text(context.document, context.schema);
        owned_map = base_position_map(context.document, context.schema);
        (owned_rendered.as_str(), &owned_map)
    };
    let relative_point = |field: &'static str, point: super::RevisionedPosition| {
        super::revisioned_position_to_relative_point(
            txn,
            fragment,
            point,
            rendered,
            map,
            context.document,
            context.schema,
        )
        .ok_or_else(|| {
            OperationError::selection_position_invalid(
                transaction.request_id,
                field,
                "selection cannot be represented with the requested Yrs affinity",
            )
        })
    };
    let text = |anchor, head| {
        Ok(super::RelativeSelection::Text {
            anchor: relative_point("selection.anchor", anchor)?,
            head: relative_point("selection.head", head)?,
        })
    };
    let relative = match &transaction.selection_intent {
        SelectionIntent::Preserve => return Ok(None),
        SelectionIntent::Set(super::SelectionInput::Text { anchor, head }) => text(*anchor, *head)?,
        SelectionIntent::Set(super::SelectionInput::Node { at }) => {
            super::RelativeSelection::Node {
                point: relative_point("selection.at", *at)?,
            }
        }
        SelectionIntent::Set(super::SelectionInput::All) => super::RelativeSelection::All,
        SelectionIntent::UseOperationResult => return Ok(None),
    };
    Ok(Some(relative))
}

fn position_update_mode(operations: &[TypedOperation]) -> UpdateMode {
    if operations.iter().all(|operation| {
        matches!(
            operation,
            TypedOperation::AddMark { .. }
                | TypedOperation::RemoveMark { .. }
                | TypedOperation::ReplaceMark { .. }
        )
    }) {
        UpdateMode::MarksOnly
    } else if operations.iter().all(|operation| {
        matches!(
            operation,
            TypedOperation::InsertText { .. } | TypedOperation::DeleteRange { .. }
        )
    }) {
        UpdateMode::InlineTextOnly
    } else {
        UpdateMode::Rebuild
    }
}

trait TextBoundaryWorkCharger {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()>;
    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()>;
}

impl TextBoundaryWorkCharger for MutationCompiler {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        MutationCompiler::charge_boundary_node(self, operation_index)
    }

    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        MutationCompiler::charge_boundary_text(self, operation_index, text_bytes)
    }
}

impl TextBoundaryWorkCharger for LocalizedFormatCompiler {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        self.charge_format_boundary_node(operation_index)
    }

    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        self.charge_format_boundary_text(operation_index, text_bytes)
    }
}

impl TextBoundaryWorkCharger for LocalizedRootWindowCompiler {
    fn charge_boundary_node(&mut self, operation_index: usize) -> OperationResult<()> {
        LocalizedRootWindowCompiler::charge_boundary_node(self, operation_index)
    }

    fn charge_boundary_text(
        &mut self,
        operation_index: usize,
        text_bytes: usize,
    ) -> OperationResult<()> {
        LocalizedRootWindowCompiler::charge_boundary_text(self, operation_index, text_bytes)
    }
}

fn text_boundaries<C: TextBoundaryWorkCharger>(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    schema: &Schema,
    lowering: &mut C,
) -> OperationResult<Vec<u32>> {
    fn visit<C: TextBoundaryWorkCharger>(
        request_id: u64,
        operation_index: usize,
        node: &Node,
        schema: &Schema,
        lowering: &mut C,
        position: &mut u32,
        output: &mut Vec<u32>,
    ) -> OperationResult<()> {
        lowering.charge_boundary_node(operation_index)?;
        if let Some(text) = node.text_str() {
            output.push(*position);
            lowering.charge_boundary_text(operation_index, text.len())?;
            let len = u32::try_from(text.chars().count()).map_err(|_| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview text scalar length exceeds u32",
                )
            })?;
            *position = position.checked_add(len).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview text boundary overflow",
                )
            })?;
            output.push(*position);
            return Ok(());
        }
        if let Some(content) = node.content() {
            let is_document = node.node_type() == schema.doc_node_type();
            if !is_document {
                *position = position.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "preview node boundary overflow",
                    )
                })?;
            }
            for child in content.iter() {
                visit(
                    request_id,
                    operation_index,
                    child,
                    schema,
                    lowering,
                    position,
                    output,
                )?;
            }
            if !is_document {
                *position = position.checked_add(1).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "preview node boundary overflow",
                    )
                })?;
            }
        } else {
            *position = position.checked_add(1).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "preview leaf boundary overflow",
                )
            })?;
        }
        Ok(())
    }

    let mut output = Vec::new();
    let mut position = 0u32;
    visit(
        request_id,
        operation_index,
        document.root(),
        schema,
        lowering,
        &mut position,
        &mut output,
    )?;
    output.sort_unstable();
    output.dedup();
    Ok(output)
}

fn resolve_position(
    request_id: u64,
    operation_index: Option<usize>,
    field: &'static str,
    position: super::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
) -> OperationResult<u32> {
    editor_offset_to_doc_pos(
        position.offset,
        position.kind,
        rendered_text,
        position_map,
        document,
    )
    .ok_or_else(|| {
        let message = format!("{field} is outside the base document");
        match operation_index {
            Some(operation_index) => {
                OperationError::position_invalid(request_id, operation_index, field, message)
            }
            None => OperationError::selection_position_invalid(request_id, field, message),
        }
    })
}

fn resolve_structural_window(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    replacement: &super::StructuralReplacement,
    limits: &ResourceLimits,
) -> OperationResult<(u32, u32)> {
    if replacement.parent_path().len() > limits.max_document_depth {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            Some(operation_index),
            "maxDocumentDepth",
            u64::try_from(limits.max_document_depth).unwrap_or(u64::MAX),
            u64::try_from(replacement.parent_path().len()).unwrap_or(u64::MAX),
        ));
    }
    let mut node = document.root();
    let mut content_start = 0u32;
    let mut work = 0usize;
    for path_index in replacement.parent_path().iter().copied() {
        work = work.saturating_add(1);
        if work > limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "maxDocumentNodes",
                u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(work).unwrap_or(u64::MAX),
            ));
        }
        let content = node.content().ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target parent has no child content",
            )
        })?;
        let index = usize::try_from(path_index).map_err(|_| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target path index is not representable",
            )
        })?;
        for sibling in content.iter().take(index) {
            work = work.saturating_add(1);
            if work > limits.max_document_nodes {
                return Err(OperationError::operation_limit_exceeded(
                    request_id,
                    Some(operation_index),
                    "maxDocumentNodes",
                    u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                    u64::try_from(work).unwrap_or(u64::MAX),
                ));
            }
            content_start = content_start
                .checked_add(sibling.node_size())
                .ok_or_else(|| {
                    OperationError::operation_invalid(
                        request_id,
                        operation_index,
                        "structure",
                        "structural target position overflowed",
                    )
                })?;
        }
        content_start = content_start.checked_add(1).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target position overflowed",
            )
        })?;
        node = content.child(index).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target path is outside the document",
            )
        })?;
    }
    let content = node.content().ok_or_else(|| {
        OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural target parent has no child content",
        )
    })?;
    let (from_child, to_child) = replacement.child_window();
    let from_child = usize::try_from(from_child).map_err(|_| {
        OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural child window is not representable",
        )
    })?;
    let to_child = usize::try_from(to_child).map_err(|_| {
        OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural child window is not representable",
        )
    })?;
    if from_child > to_child || to_child > content.child_count() {
        return Err(OperationError::operation_invalid(
            request_id,
            operation_index,
            "structure",
            "structural child window is outside its parent",
        ));
    }
    let mut from = content_start;
    for sibling in content.iter().take(from_child) {
        work = work.saturating_add(1);
        if work > limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "maxDocumentNodes",
                u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(work).unwrap_or(u64::MAX),
            ));
        }
        from = from.checked_add(sibling.node_size()).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target position overflowed",
            )
        })?;
    }
    let mut to = from;
    for sibling in content.iter().skip(from_child).take(to_child - from_child) {
        work = work.saturating_add(1);
        if work > limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                request_id,
                Some(operation_index),
                "maxDocumentNodes",
                u64::try_from(limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(work).unwrap_or(u64::MAX),
            ));
        }
        to = to.checked_add(sibling.node_size()).ok_or_else(|| {
            OperationError::operation_invalid(
                request_id,
                operation_index,
                "structure",
                "structural target position overflowed",
            )
        })?;
    }
    Ok((from, to))
}

#[derive(Clone, Copy)]
struct InsertNodeResolverContext<'a> {
    request_id: u64,
    operation_index: usize,
    document: &'a Document,
    node: &'a Node,
    schema: &'a Schema,
    budget: &'a WorkBudget,
    limit: usize,
}

fn resolve_insert_node_position(
    context: InsertNodeResolverContext<'_>,
    position: super::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
) -> OperationResult<u32> {
    let InsertNodeResolverContext {
        request_id,
        operation_index,
        document,
        node,
        schema,
        budget: resolver_budget,
        limit: resolver_limit,
    } = context;
    let (fallback, scalar_offset, block_index) = resolve_insert_node_base_position(
        request_id,
        operation_index,
        position,
        rendered_text,
        position_map,
        document,
        resolver_budget,
        resolver_limit,
    )?;
    if !insert_node_is_block(node, schema) {
        return Ok(fallback);
    }
    consume_resolver_work(
        resolver_budget,
        1,
        request_id,
        operation_index,
        resolver_limit,
    )?;
    let Some(index) = block_index else {
        return Ok(fallback);
    };
    if let Some(block) = position_map.block(index) {
        let break_start = block
            .scalar_start
            .checked_add(block.scalar_prefix_len)
            .and_then(|start| start.checked_add(block.scalar_len));
        let break_end = break_start.and_then(|start| start.checked_add(block.rendered_break_after));
        if break_start
            .zip(break_end)
            .is_some_and(|(start, end)| scalar_offset >= start && scalar_offset < end)
        {
            let Some(next) = position_map.block(index + 1) else {
                return Ok(fallback);
            };
            let mut common_depth = 0usize;
            for (left, right) in block.node_path.iter().zip(next.node_path.iter()) {
                consume_resolver_work(
                    resolver_budget,
                    1,
                    request_id,
                    operation_index,
                    resolver_limit,
                )?;
                if left != right {
                    break;
                }
                common_depth += 1;
            }
            let direct_path = common_depth
                .checked_add(1)
                .filter(|depth| *depth <= block.node_path.len())
                .map(|depth| &block.node_path[..depth]);
            let mut candidates = Vec::with_capacity(4);
            if let Some(path) = direct_path {
                candidates.push((path, true));
            }
            match position.affinity {
                super::Affinity::Before => {
                    candidates.push((block.node_path.as_slice(), true));
                    candidates.push((next.node_path.as_slice(), false));
                    candidates.push((next.node_path.as_slice(), true));
                }
                super::Affinity::After => {
                    candidates.push((next.node_path.as_slice(), false));
                    candidates.push((next.node_path.as_slice(), true));
                    candidates.push((block.node_path.as_slice(), true));
                }
            }
            for (path, after_child) in candidates {
                let candidate = document_position_at_path_budgeted(
                    document,
                    path,
                    after_child,
                    resolver_budget,
                    request_id,
                    operation_index,
                    resolver_limit,
                )?;
                if let Some(candidate) = candidate {
                    let valid = insertion_is_schema_valid(context, candidate)?;
                    if valid {
                        return Ok(candidate);
                    }
                }
            }
            return Ok(fallback);
        }
        let Some(content_start) = block.scalar_start.checked_add(block.scalar_prefix_len) else {
            return Ok(fallback);
        };
        let Some(content_end) = content_start.checked_add(block.scalar_len) else {
            return Ok(fallback);
        };
        if scalar_offset == content_start && position.affinity == super::Affinity::Before {
            if let Some(candidate) = document_position_at_path_budgeted(
                document,
                &block.node_path,
                false,
                resolver_budget,
                request_id,
                operation_index,
                resolver_limit,
            )? {
                let valid = insertion_is_schema_valid(context, candidate)?;
                if valid {
                    return Ok(candidate);
                }
            }
        }
        if scalar_offset == content_end && position.affinity == super::Affinity::After {
            if let Some(candidate) = document_position_at_path_budgeted(
                document,
                &block.node_path,
                true,
                resolver_budget,
                request_id,
                operation_index,
                resolver_limit,
            )? {
                let valid = insertion_is_schema_valid(context, candidate)?;
                if valid {
                    return Ok(candidate);
                }
            }
        }
    }
    Ok(fallback)
}

#[allow(clippy::too_many_arguments)]
fn resolve_insert_node_base_position(
    request_id: u64,
    operation_index: usize,
    position: super::RevisionedPosition,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
    resolver_budget: &WorkBudget,
    resolver_limit: usize,
) -> OperationResult<(u32, u32, Option<usize>)> {
    let scalar_offset = match position.kind {
        super::EditorOffsetKind::Scalar => Some(position.offset),
        super::EditorOffsetKind::Utf16 => utf16_offset_to_scalar_budgeted(
            rendered_text,
            position.offset,
            resolver_budget,
            request_id,
            operation_index,
            resolver_limit,
        )?,
    }
    .filter(|offset| *offset <= position_map.total_scalars())
    .ok_or_else(|| {
        OperationError::position_invalid(
            request_id,
            operation_index,
            "at",
            "at is outside the base document",
        )
    })?;

    let mut exhausted = false;
    let fallback = position_map.scalar_to_doc_metered(scalar_offset, document, |amount| {
        let admitted = resolver_budget.consume_n(amount);
        exhausted |= !admitted;
        admitted
    });
    if exhausted {
        return Err(resolver_budget_exhausted(
            request_id,
            operation_index,
            resolver_limit,
        ));
    }
    let (fallback, block_index) = fallback.ok_or_else(|| {
        OperationError::position_invalid(
            request_id,
            operation_index,
            "at",
            "at is outside the base document",
        )
    })?;
    Ok((
        fallback,
        scalar_offset,
        position_map.block(block_index).map(|_| block_index),
    ))
}

fn utf16_offset_to_scalar_budgeted(
    value: &str,
    utf16_offset: u32,
    resolver_budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    resolver_limit: usize,
) -> OperationResult<Option<u32>> {
    if utf16_offset == 0 {
        return Ok(Some(0));
    }
    let mut utf16_seen = 0u32;
    let mut scalars_seen = 0u32;
    for character in value.chars() {
        consume_resolver_work(
            resolver_budget,
            1,
            request_id,
            operation_index,
            resolver_limit,
        )?;
        let next_utf16 = utf16_seen.saturating_add(character.len_utf16() as u32);
        if utf16_offset < next_utf16 {
            return Ok(None);
        }
        scalars_seen = scalars_seen.saturating_add(1);
        if utf16_offset == next_utf16 {
            return Ok(Some(scalars_seen));
        }
        utf16_seen = next_utf16;
    }
    Ok(None)
}

fn normalize_current_insert_node_position(
    context: InsertNodeResolverContext<'_>,
    position: u32,
    affinity: super::Affinity,
) -> OperationResult<u32> {
    let InsertNodeResolverContext {
        request_id,
        operation_index,
        document,
        node,
        schema,
        budget: resolver_budget,
        limit: resolver_limit,
    } = context;
    let valid = insertion_is_schema_valid(context, position)?;
    if valid || !insert_node_is_block(node, schema) {
        return Ok(position);
    }
    let Some(resolved) = resolve_insert_position_budgeted(
        document,
        position,
        resolver_budget,
        request_id,
        operation_index,
        resolver_limit,
    )?
    else {
        return Ok(position);
    };
    let Some(content) = resolved.parent.content() else {
        return Ok(position);
    };
    let at_start = resolved.parent_offset == 0;
    let at_end = resolved.parent_offset == content.size();
    if !at_start && !at_end {
        return Ok(position);
    }
    for depth in (1..=resolved.node_path.len()).rev() {
        let path = &resolved.node_path[..depth];
        let boundary_order = match (at_start, at_end, affinity) {
            (true, true, super::Affinity::After) => [true, false],
            (true, _, _) => [false, true],
            _ => [true, false],
        };
        for after_child in boundary_order {
            let Some(candidate) = document_position_at_path_budgeted(
                document,
                path,
                after_child,
                resolver_budget,
                request_id,
                operation_index,
                resolver_limit,
            )?
            else {
                continue;
            };
            let valid = insertion_is_schema_valid(context, candidate)?;
            if valid {
                return Ok(candidate);
            }
        }
    }
    Ok(position)
}

fn insert_node_is_block(node: &Node, schema: &Schema) -> bool {
    schema.node(node.node_type()).map_or_else(
        || {
            matches!(node.node_type(), "__opaque" | "__opaque_json")
                && node
                    .attrs()
                    .get("opaque_placement")
                    .and_then(serde_json::Value::as_str)
                    == Some("block")
        },
        |spec| {
            matches!(
                spec.role,
                crate::schema::NodeRole::TextBlock
                    | crate::schema::NodeRole::List { .. }
                    | crate::schema::NodeRole::ListItem
                    | crate::schema::NodeRole::Block
            )
        },
    )
}

fn insertion_is_schema_valid(
    context: InsertNodeResolverContext<'_>,
    position: u32,
) -> OperationResult<bool> {
    let InsertNodeResolverContext {
        request_id,
        operation_index,
        document,
        node: inserted,
        schema,
        budget: resolver_budget,
        limit: resolver_limit,
    } = context;
    enum Child<'a> {
        Existing(&'a Node),
        Inserted(&'a Node),
    }
    let Some(resolved) = resolve_insert_position_budgeted(
        document,
        position,
        resolver_budget,
        request_id,
        operation_index,
        resolver_limit,
    )?
    else {
        return Ok(false);
    };
    let parent = resolved.parent;
    let Some(content) = parent.content() else {
        return Ok(false);
    };
    let mut offset = 0u32;
    let mut insertion_index = None;
    for (index, child) in content.iter().enumerate() {
        consume_resolver_work(
            resolver_budget,
            1,
            request_id,
            operation_index,
            resolver_limit,
        )?;
        if offset == resolved.parent_offset {
            insertion_index = Some(index);
            break;
        }
        let child_size = resolver_node_size(
            child,
            resolver_budget,
            request_id,
            operation_index,
            resolver_limit,
        )?;
        let Some(end) = offset.checked_add(child_size) else {
            return Ok(false);
        };
        if resolved.parent_offset < end {
            return Ok(false);
        }
        offset = end;
    }
    let insertion_index = insertion_index
        .or_else(|| (offset == resolved.parent_offset).then_some(content.child_count()));
    let Some(insertion_index) = insertion_index else {
        return Ok(false);
    };
    let Some(parent_spec) = schema.node(parent.node_type()) else {
        return Ok(false);
    };
    let assembly_work = content
        .child_count()
        .checked_add(1)
        .ok_or_else(|| resolver_budget_exhausted(request_id, operation_index, resolver_limit))?;
    consume_resolver_work(
        resolver_budget,
        assembly_work,
        request_id,
        operation_index,
        resolver_limit,
    )?;
    let children = content
        .iter()
        .enumerate()
        .flat_map(|(index, child)| {
            (index == insertion_index)
                .then_some(Child::Inserted(inserted))
                .into_iter()
                .chain(std::iter::once(Child::Existing(child)))
        })
        .chain((insertion_index == content.child_count()).then_some(Child::Inserted(inserted)))
        .collect::<Vec<_>>();
    let valid = parent_spec
        .content
        .matches_with_budget(
            &children,
            |child, symbol| {
                let node = match child {
                    Child::Existing(node) | Child::Inserted(node) => node,
                };
                if matches!(node.node_type(), "__opaque" | "__opaque_json") {
                    return node
                        .attrs()
                        .get("opaque_placement")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|placement| {
                            schema.symbol_accepts_opaque_placement(symbol, placement)
                        });
                }
                schema.node_matches_symbol(node.node_type(), symbol)
            },
            resolver_budget,
        )
        .map_err(|_| resolver_budget_exhausted(request_id, operation_index, resolver_limit))?;
    Ok(valid)
}

fn resolver_budget_exhausted(
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxInputBytes",
        u64::try_from(limit).unwrap_or(u64::MAX),
        u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1),
    )
}

fn consume_resolver_work(
    budget: &WorkBudget,
    amount: usize,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<()> {
    if budget.consume_n(amount) {
        Ok(())
    } else {
        Err(resolver_budget_exhausted(
            request_id,
            operation_index,
            limit,
        ))
    }
}

struct InsertResolvedPosition<'a> {
    node_path: SmallVec<[u32; 8]>,
    parent_offset: u32,
    parent: &'a Node,
}

fn resolve_insert_position_budgeted<'a>(
    document: &'a Document,
    position: u32,
    budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<Option<InsertResolvedPosition<'a>>> {
    consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
    if position > document.content_size() {
        return Ok(None);
    }

    let mut parent = document.root();
    let mut relative_position = position;
    let mut path = SmallVec::<[u32; 8]>::new();
    loop {
        let Some(content) = parent.content() else {
            return Ok(None);
        };
        if relative_position > content.size() {
            return Ok(None);
        }

        let mut offset = 0u32;
        let mut descended = false;
        for (child_index, child) in content.iter().enumerate() {
            consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
            let child_size = resolver_node_size(child, budget, request_id, operation_index, limit)?;
            if child.is_text() || child.is_void() {
                if relative_position < offset.saturating_add(child_size) {
                    return Ok(Some(InsertResolvedPosition {
                        node_path: path,
                        parent_offset: relative_position,
                        parent,
                    }));
                }
                let Some(next_offset) = offset.checked_add(child_size) else {
                    return Ok(None);
                };
                offset = next_offset;
                continue;
            }

            if relative_position == offset {
                return Ok(Some(InsertResolvedPosition {
                    node_path: path,
                    parent_offset: relative_position,
                    parent,
                }));
            }
            let Some(inner_start) = offset.checked_add(1) else {
                return Ok(None);
            };
            let Some(inner_end) = offset
                .checked_add(child_size)
                .and_then(|end| end.checked_sub(1))
            else {
                return Ok(None);
            };
            if relative_position >= inner_start && relative_position <= inner_end {
                let Ok(child_index) = u32::try_from(child_index) else {
                    return Ok(None);
                };
                path.push(child_index);
                relative_position -= inner_start;
                parent = child;
                descended = true;
                break;
            }
            let Some(next_offset) = offset.checked_add(child_size) else {
                return Ok(None);
            };
            offset = next_offset;
        }
        if descended {
            continue;
        }
        return Ok(
            (relative_position == offset).then_some(InsertResolvedPosition {
                node_path: path,
                parent_offset: relative_position,
                parent,
            }),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn document_position_at_path_budgeted(
    document: &Document,
    path: &[u32],
    after_child: bool,
    budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<Option<u32>> {
    let mut parent = document.root();
    let mut position = 0u32;
    for (depth, &child_index) in path.iter().enumerate() {
        consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
        let Some(content) = parent.content() else {
            return Ok(None);
        };
        let Ok(child_index) = usize::try_from(child_index) else {
            return Ok(None);
        };
        for child in content.iter().take(child_index) {
            consume_resolver_work(budget, 1, request_id, operation_index, limit)?;
            let child_size = resolver_node_size(child, budget, request_id, operation_index, limit)?;
            let Some(next_position) = position.checked_add(child_size) else {
                return Ok(None);
            };
            position = next_position;
        }
        let Some(child) = content.child(child_index) else {
            return Ok(None);
        };
        if depth + 1 == path.len() {
            return Ok(if after_child {
                position.checked_add(resolver_node_size(
                    child,
                    budget,
                    request_id,
                    operation_index,
                    limit,
                )?)
            } else {
                Some(position)
            });
        }
        let Some(next_position) = position.checked_add(1) else {
            return Ok(None);
        };
        position = next_position;
        parent = child;
    }
    Ok(Some(position))
}

fn resolver_node_size(
    node: &Node,
    budget: &WorkBudget,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<u32> {
    if let Some(text) = node.text_str() {
        consume_resolver_work(budget, text.len(), request_id, operation_index, limit)?;
    }
    Ok(node.node_size())
}

#[allow(clippy::too_many_arguments)]
fn resolve_range(
    request_id: u64,
    operation_index: usize,
    range: RevisionedRange,
    rendered_text: &str,
    position_map: &PositionMap,
    document: &Document,
    composed_map: &StepMap,
) -> OperationResult<(u32, u32)> {
    let base_from = resolve_position(
        request_id,
        Some(operation_index),
        "range.from",
        range.from,
        rendered_text,
        position_map,
        document,
    )?;
    let base_to = resolve_position(
        request_id,
        Some(operation_index),
        "range.to",
        range.to,
        rendered_text,
        position_map,
        document,
    )?;
    if base_from > base_to {
        return Err(OperationError::operation_invalid(
            request_id,
            operation_index,
            "range",
            "range.from must not be greater than range.to",
        ));
    }
    Ok((
        map_position(composed_map, base_from, range.from.affinity),
        map_position(composed_map, base_to, range.to.affinity),
    ))
}

pub(crate) fn map_position(map: &StepMap, mut position: u32, affinity: Affinity) -> u32 {
    for &(range_position, deleted, inserted) in map.ranges() {
        if position < range_position {
            continue;
        }
        if position == range_position {
            if inserted > 0 && affinity == Affinity::After {
                position = position.saturating_add(inserted);
            }
        } else if position <= range_position.saturating_add(deleted) {
            position = range_position.saturating_add(inserted);
        } else {
            position = position.saturating_sub(deleted).saturating_add(inserted);
        }
    }
    position
}

fn resolve_attribute_target_position(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    position: u32,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
    schema: &Schema,
) -> OperationResult<u32> {
    let accepts = |node: &Node| {
        schema.node(node.node_type()).is_some_and(|spec| {
            (spec.allow_undeclared_attrs || attrs.keys().all(|key| spec.attrs.contains_key(key)))
                && (!attrs.is_empty() || !spec.attrs.is_empty() || !node.attrs().is_empty())
        })
    };
    if direct_attribute_target(document, position).is_some_and(&accepts) {
        return Ok(position);
    }
    let resolved = document.resolve(position).map_err(|message| {
        OperationError::position_invalid(request_id, operation_index, "at", message)
    })?;
    for depth in (1..=resolved.node_path.len()).rev() {
        let path = &resolved.node_path[..depth];
        let Some(node) = document.node_at(path) else {
            continue;
        };
        if accepts(node) {
            return node_boundary_position(document.root(), path).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "attribute target path has no document boundary",
                )
            });
        }
    }
    Err(OperationError::position_invalid(
        request_id,
        operation_index,
        "at",
        "position does not resolve to a node accepting the supplied attributes",
    ))
}

fn resolve_join_target_position(
    request_id: u64,
    operation_index: usize,
    document: &Document,
    position: u32,
) -> OperationResult<u32> {
    let resolved = document.resolve(position).map_err(|message| {
        OperationError::position_invalid(request_id, operation_index, "at", message)
    })?;
    if resolved.node_path.is_empty() {
        return Ok(position);
    }
    let deepest = document.node_at(&resolved.node_path).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            "join position path has no containing node",
        )
    })?;
    if resolved.parent_offset != deepest.content().map(Fragment::size).unwrap_or(0) {
        return Ok(position);
    }
    for depth in (1..=resolved.node_path.len()).rev() {
        let path = &resolved.node_path[..depth];
        let parent_path = &path[..depth - 1];
        let child_index = usize::try_from(path[depth - 1]).map_err(|_| {
            OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "join child index exceeds usize",
            )
        })?;
        let parent = if parent_path.is_empty() {
            document.root()
        } else {
            document.node_at(parent_path).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "join ancestor path has no parent node",
                )
            })?
        };
        let content = parent.content().ok_or_else(|| {
            OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                "join ancestor parent has no content",
            )
        })?;
        if let Some(next) = content.child(child_index.saturating_add(1)) {
            let candidate = document.node_at(path).ok_or_else(|| {
                OperationError::engine_invariant_failed(
                    request_id,
                    Some(operation_index),
                    "join candidate path has no node",
                )
            })?;
            if candidate.is_element() && next.is_element() {
                let start = node_boundary_position(document.root(), path).ok_or_else(|| {
                    OperationError::engine_invariant_failed(
                        request_id,
                        Some(operation_index),
                        "join candidate path has no document boundary",
                    )
                })?;
                return start.checked_add(candidate.node_size()).ok_or_else(|| {
                    OperationError::position_invalid(
                        request_id,
                        operation_index,
                        "at",
                        "join boundary position overflow",
                    )
                });
            }
            break;
        }
    }
    Ok(position)
}

fn direct_attribute_target(document: &Document, position: u32) -> Option<&Node> {
    let resolved = document.resolve(position).ok()?;
    let parent = resolved.parent(document);
    let mut offset = 0u32;
    parent.content()?.iter().find(|child| {
        let matches = !child.is_text() && resolved.parent_offset == offset;
        if !matches {
            offset = offset.saturating_add(child.node_size());
        }
        matches
    })
}

fn node_boundary_position(root: &Node, path: &[u32]) -> Option<u32> {
    let mut node = root;
    let mut content_start = 0u32;
    for (depth, child_index) in path.iter().copied().enumerate() {
        let content = node.content()?;
        let index = usize::try_from(child_index).ok()?;
        let preceding = content
            .iter()
            .take(index)
            .try_fold(0u32, |total, child| total.checked_add(child.node_size()))?;
        let boundary = content_start.checked_add(preceding)?;
        node = content.child(index)?;
        if depth + 1 == path.len() {
            return Some(boundary);
        }
        content_start = boundary.checked_add(1)?;
    }
    None
}

fn charge_input(
    charged: &mut usize,
    amount: usize,
    request_id: u64,
    operation_index: usize,
    limit: usize,
) -> OperationResult<()> {
    let actual = charged.checked_add(amount);
    let overflowed = actual.is_none();
    let actual = actual.unwrap_or(usize::MAX);
    if overflowed || actual > limit {
        return Err(OperationError::operation_limit_exceeded(
            request_id,
            Some(operation_index),
            "maxInputBytes",
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        ));
    }
    *charged = actual;
    Ok(())
}

fn charge_undo_bound(
    charged: &mut u64,
    pending_error: &mut Option<OperationError>,
    amount: u64,
    request_id: u64,
    operation_index: usize,
    limit: u64,
) {
    let actual = charged.checked_add(amount);
    let overflowed = actual.is_none();
    let actual = actual.unwrap_or(u64::MAX);
    *charged = actual;
    if pending_error.is_none() && (overflowed || actual > limit) {
        *pending_error = Some(OperationError::operation_limit_exceeded(
            request_id,
            Some(operation_index),
            "maxUndoRetainedUnits",
            limit,
            actual,
        ));
    }
}

fn validate_operation_marks(
    request_id: u64,
    operation_index: usize,
    marks: &[Mark],
    schema: &Schema,
) -> OperationResult<()> {
    crate::transform::validate_input_mark_set(marks, schema).map_err(|error| {
        OperationError::operation_invalid(request_id, operation_index, "marks", error.to_string())
    })
}

fn add_mark_conflicts_with_existing_attrs(
    document: &Document,
    from: u32,
    to: u32,
    mark: &Mark,
) -> bool {
    if from >= to {
        return false;
    }
    let (Ok(resolved_from), Ok(resolved_to)) = (document.resolve(from), document.resolve(to))
    else {
        return false;
    };
    if resolved_from.node_path != resolved_to.node_path {
        return false;
    }
    let parent = resolved_from.parent(document);
    let Some(content) = parent.content() else {
        return false;
    };
    let mut offset = 0u32;
    for child in content.iter() {
        let child_end = offset.saturating_add(child.node_size());
        if child.is_text()
            && child_end > resolved_from.parent_offset
            && offset < resolved_to.parent_offset
            && child
                .marks()
                .iter()
                .any(|existing| existing.mark_type() == mark.mark_type() && existing != mark)
        {
            return true;
        }
        offset = child_end;
    }
    false
}

fn validate_fragment_marks(
    request_id: u64,
    operation_index: usize,
    fragment: &Fragment,
    schema: &Schema,
) -> OperationResult<()> {
    fn visit(
        request_id: u64,
        operation_index: usize,
        node: &Node,
        schema: &Schema,
    ) -> OperationResult<()> {
        validate_operation_marks(request_id, operation_index, node.marks(), schema)?;
        if let Some(content) = node.content() {
            for child in content.iter() {
                visit(request_id, operation_index, child, schema)?;
            }
        }
        Ok(())
    }

    for node in fragment.iter() {
        visit(request_id, operation_index, node, schema)?;
    }
    Ok(())
}

fn validate_preview_marks(
    request_id: u64,
    operation_index: usize,
    preview: &Document,
    schema: &Schema,
) -> OperationResult<()> {
    crate::transform::validate_canonical_marks(preview, schema).map_err(|error| {
        OperationError::document_invalid(
            request_id,
            Some(operation_index),
            "marks",
            error.to_string(),
        )
    })
}

fn input_work_overflow(
    request_id: u64,
    operation_index: usize,
    context: CompilationContext<'_>,
) -> OperationError {
    OperationError::operation_limit_exceeded(
        request_id,
        Some(operation_index),
        "maxInputBytes",
        u64::try_from(context.resource_limits.max_input_bytes).unwrap_or(u64::MAX),
        u64::MAX,
    )
}

fn checked_mark_input_bytes(
    request_id: u64,
    operation_index: usize,
    mark: &Mark,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    counter.charge_bytes(mark.mark_type().len())?;
    counter.count_attrs(mark.attrs())?;
    Ok(counter.delta())
}

fn checked_mark_set_input_bytes(
    request_id: u64,
    operation_index: usize,
    marks: &[Mark],
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    for mark in marks {
        counter.charge_bytes(mark.mark_type().len())?;
        counter.count_attrs(mark.attrs())?;
    }
    Ok(counter.delta())
}

fn checked_fragment_input_bytes(
    request_id: u64,
    operation_index: usize,
    fragment: &Fragment,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    count_node_forest(&mut counter, fragment.children())?;
    Ok(counter.delta())
}

fn checked_node_input_bytes(
    request_id: u64,
    operation_index: usize,
    node: &Node,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    count_node_forest(&mut counter, std::slice::from_ref(node))?;
    Ok(counter.delta())
}

fn checked_attrs_input_bytes(
    request_id: u64,
    operation_index: usize,
    attrs: &std::collections::HashMap<String, serde_json::Value>,
    limits: &ResourceLimits,
    base_bytes: usize,
) -> OperationResult<usize> {
    let mut counter = StructuredInputCounter::new(request_id, operation_index, limits, base_bytes);
    counter.count_attrs(attrs)?;
    Ok(counter.delta())
}

fn count_node_forest(
    counter: &mut StructuredInputCounter<'_>,
    roots: &[Node],
) -> OperationResult<()> {
    enum Frame<'a> {
        Node(&'a Node, usize),
        Children(std::slice::Iter<'a, Node>, usize),
    }
    let mut stack = vec![Frame::Children(roots.iter(), 1)];
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Node(node, depth) => {
                counter.charge_bytes(node.node_type().len())?;
                counter.count_attrs(node.attrs())?;
                for mark in node.marks() {
                    counter.charge_bytes(mark.mark_type().len())?;
                    counter.count_attrs(mark.attrs())?;
                }
                if let Some(text) = node.text_str() {
                    counter.charge_bytes(text.len())?;
                }
                if let Some(content) = node.content() {
                    let child_depth = depth.checked_add(1).ok_or_else(|| counter.depth_error())?;
                    stack.push(Frame::Children(content.children().iter(), child_depth));
                }
            }
            Frame::Children(mut children, depth) => {
                if let Some(child) = children.next() {
                    counter.admit_item(depth)?;
                    stack.push(Frame::Children(children, depth));
                    stack.push(Frame::Node(child, depth));
                }
            }
        }
    }
    Ok(())
}

struct StructuredInputCounter<'a> {
    request_id: u64,
    operation_index: usize,
    limits: &'a ResourceLimits,
    base_bytes: usize,
    json_meter: JsonValueMeter,
    items: usize,
}

impl<'a> StructuredInputCounter<'a> {
    fn new(
        request_id: u64,
        operation_index: usize,
        limits: &'a ResourceLimits,
        base_bytes: usize,
    ) -> Self {
        Self {
            request_id,
            operation_index,
            limits,
            base_bytes,
            json_meter: JsonValueMeter::new(
                limits.max_input_bytes,
                limits.max_document_nodes,
                limits.max_document_depth,
                base_bytes,
            ),
            items: 0,
        }
    }

    fn delta(&self) -> usize {
        self.json_meter.bytes() - self.base_bytes
    }

    fn charge_bytes(&mut self, amount: usize) -> OperationResult<()> {
        self.json_meter
            .charge_bytes(amount)
            .map_err(|error| self.map_json_meter_error(error))
    }

    fn admit_item(&mut self, depth: usize) -> OperationResult<()> {
        if depth > self.limits.max_document_depth {
            return Err(self.depth_error());
        }
        let actual = self.items.saturating_add(1);
        if actual > self.limits.max_document_nodes {
            return Err(OperationError::operation_limit_exceeded(
                self.request_id,
                Some(self.operation_index),
                "maxDocumentNodes",
                u64::try_from(self.limits.max_document_nodes).unwrap_or(u64::MAX),
                u64::try_from(actual).unwrap_or(u64::MAX),
            ));
        }
        self.items = actual;
        Ok(())
    }

    fn depth_error(&self) -> OperationError {
        OperationError::operation_limit_exceeded(
            self.request_id,
            Some(self.operation_index),
            "maxDocumentDepth",
            u64::try_from(self.limits.max_document_depth).unwrap_or(u64::MAX),
            u64::try_from(self.limits.max_document_depth)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
    }

    fn count_attrs(
        &mut self,
        attrs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> OperationResult<()> {
        self.json_meter
            .admit_object(attrs)
            .map_err(|error| self.map_json_meter_error(error))
    }

    fn map_json_meter_error(&self, error: JsonMeterError) -> OperationError {
        match error.dimension {
            JsonMeterDimension::Bytes => input_limit_error(
                self.request_id,
                Some(self.operation_index),
                error.limit,
                error.actual,
            ),
            JsonMeterDimension::Work => OperationError::operation_limit_exceeded(
                self.request_id,
                Some(self.operation_index),
                "maxDocumentNodes",
                u64::try_from(error.limit).unwrap_or(u64::MAX),
                u64::try_from(error.actual).unwrap_or(u64::MAX),
            ),
            JsonMeterDimension::Depth => OperationError::operation_limit_exceeded(
                self.request_id,
                Some(self.operation_index),
                "maxDocumentDepth",
                u64::try_from(error.limit).unwrap_or(u64::MAX),
                u64::try_from(error.actual).unwrap_or(u64::MAX),
            ),
        }
    }
}

fn charge_preview_output(
    work: &mut CheckedWork,
    request_id: u64,
    operation_index: usize,
    preview: &Document,
    canonical_schema: &CanonicalSchemaContext,
    context: CompilationContext<'_>,
) -> OperationResult<CanonicalArtifact> {
    let artifact = canonical_schema.derive(preview).map_err(|error| {
        OperationError::engine_invariant_failed(
            request_id,
            Some(operation_index),
            format!("preview serialization failed: {error}"),
        )
    })?;
    charge_canonical_output(work, request_id, operation_index, &artifact, context)?;
    Ok(artifact)
}

#[allow(clippy::too_many_arguments)]
fn charge_localized_preview_output(
    work: &mut CheckedWork,
    request_id: u64,
    operation_index: usize,
    preview: &Document,
    canonical_schema: &CanonicalSchemaContext,
    context: CompilationContext<'_>,
    raw_text_scalars: u64,
    raw_text_utf8_bytes: usize,
) -> OperationResult<CanonicalArtifact> {
    let artifact = canonical_schema
        .derive_with_known_text_metrics(preview, raw_text_scalars, raw_text_utf8_bytes)
        .map_err(|error| {
            OperationError::engine_invariant_failed(
                request_id,
                Some(operation_index),
                format!("preview serialization failed: {error}"),
            )
        })?;
    charge_canonical_output(work, request_id, operation_index, &artifact, context)?;
    Ok(artifact)
}

fn charge_canonical_output(
    work: &mut CheckedWork,
    request_id: u64,
    operation_index: usize,
    artifact: &CanonicalArtifact,
    context: CompilationContext<'_>,
) -> OperationResult<()> {
    work.charge_output_bytes(
        request_id,
        operation_index,
        artifact.serialized_len(),
        context.editing_limits.max_derived_output_bytes,
    )
}

fn charge_prepared_preview_output(
    work: &mut CheckedWork,
    request_id: u64,
    operation_index: usize,
    admission: &PreparedSemanticAdmission,
    context: CompilationContext<'_>,
) -> OperationResult<CanonicalArtifact> {
    let artifact = admission.canonical_artifact();
    charge_canonical_output(work, request_id, operation_index, artifact, context)?;
    Ok(artifact.clone())
}

fn prepared_candidate_matches(
    prepared: Option<PreparedSemanticContext<'_>>,
    operation_count: usize,
    operation_index: usize,
    candidate: &Document,
    context: CompilationContext<'_>,
    canonical_schema: &CanonicalSchemaContext,
) -> bool {
    operation_count == 1
        && operation_index == 0
        && prepared.is_some_and(|prepared| {
            *candidate == *prepared.expected_preview
                && prepared
                    .admission
                    .candidate_validation_ref()
                    .is_some_and(|validation| {
                        validation.admits_context(
                            prepared.expected_preview,
                            prepared.admission.canonical_artifact(),
                            context.resource_limits,
                            context.editing_limits,
                            context.max_length,
                            prepared.schema_fingerprint,
                            canonical_schema,
                        )
                    })
        })
}

fn validate_preview(
    request_id: u64,
    operation_index: Option<usize>,
    preview: &Document,
    context: CompilationContext<'_>,
) -> OperationResult<()> {
    DocumentValidator::validate(preview, context.schema, context.resource_limits).map_err(
        |error| {
            let field = if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                "document"
            } else {
                "content"
            };
            if error.code() == "DOCUMENT_LIMIT_EXCEEDED" {
                OperationError::document_limit_exceeded(
                    request_id,
                    operation_index,
                    field,
                    error.limit.unwrap_or(0) as u64,
                    error.actual.unwrap_or(0) as u64,
                )
            } else {
                OperationError::document_invalid(
                    request_id,
                    operation_index,
                    field,
                    error.to_string(),
                )
            }
        },
    )?;
    if let Some(limit) = context.max_length {
        let actual = preview.root().text_content().chars().count() as u64;
        if actual > limit as u64 {
            return Err(OperationError::document_limit_exceeded(
                request_id,
                operation_index,
                "maxLength",
                limit as u64,
                actual,
            ));
        }
    }
    Ok(())
}

fn scalar_byte_offset(text: &str, scalar_offset: u32) -> Option<usize> {
    let mut scalars = 0u32;
    for (byte, _) in text.char_indices() {
        if scalars == scalar_offset {
            return Some(byte);
        }
        scalars = scalars.checked_add(1)?;
    }
    (scalars == scalar_offset).then_some(text.len())
}

fn try_rebuild_element(parent: &Node, children: Vec<Node>) -> Node {
    Node::element(
        parent.node_type().to_owned(),
        parent.attrs().clone(),
        Fragment::from(children),
    )
}

fn try_replace_node_at_path(current: &Node, path: &[u32], replacement: Node) -> Option<Node> {
    if path.is_empty() {
        return Some(replacement);
    }
    let replace_index = usize::try_from(path[0]).ok()?;
    let content = current.content()?;
    if replace_index >= content.child_count() {
        return None;
    }
    let mut children = Vec::new();
    children.try_reserve_exact(content.child_count()).ok()?;
    let mut replacement = Some(replacement);
    for (index, child) in content.iter().enumerate() {
        if index == replace_index {
            children.push(try_replace_node_at_path(
                child,
                &path[1..],
                replacement.take()?,
            )?);
        } else {
            children.push(child.clone());
        }
    }
    Some(try_rebuild_element(current, children))
}

fn try_localized_semantic_compilation(
    context: CompilationContext<'_>,
    transaction: &TypedTransaction,
    validated: &ValidatedLocalizedInsertAdmission<'_>,
) -> Option<LocalizedSemanticCompilation> {
    #[cfg(test)]
    if FORCE_LOCALIZED_SEMANTIC_ALLOCATION_FAILURE.get() {
        return None;
    }
    let [TypedOperation::InsertText { text, marks, .. }] = transaction.operations.as_slice() else {
        return None;
    };
    let position = validated.document_position();
    let block_path = validated.block_path();
    let block = context.document.node_at(block_path)?;
    let child_index = usize::try_from(validated.child_ordinal()).ok()?;
    let live_leaf = block.child(child_index)?;
    let live_text = live_leaf.text_str()?;
    if live_leaf.marks() != marks {
        return None;
    }
    let local_scalar = position.checked_sub(validated.leaf_doc_start())?;
    if local_scalar == 0 || local_scalar >= live_leaf.node_size() {
        return None;
    }
    let local_byte = scalar_byte_offset(live_text, local_scalar)?;
    let next_text_bytes = live_text.len().checked_add(text.len())?;
    let mut next_text = String::new();
    next_text.try_reserve_exact(next_text_bytes).ok()?;
    next_text.push_str(&live_text[..local_byte]);
    next_text.push_str(text);
    next_text.push_str(&live_text[local_byte..]);

    let mut next_marks = Vec::new();
    next_marks.try_reserve_exact(live_leaf.marks().len()).ok()?;
    next_marks.extend_from_slice(live_leaf.marks());
    let mut next_leaf = Some(Node::text(next_text, next_marks));
    let content = block.content()?;
    let mut block_children = Vec::new();
    block_children
        .try_reserve_exact(content.child_count())
        .ok()?;
    for (index, child) in content.iter().enumerate() {
        block_children.push(if index == child_index {
            next_leaf.take()?
        } else {
            child.clone()
        });
    }
    if next_leaf.is_some() {
        return None;
    }
    let next_block = try_rebuild_element(block, block_children);
    let next_root = try_replace_node_at_path(context.document.root(), block_path, next_block)?;
    let preview = Document::new(next_root);
    let step_map = StepMap::try_from_insert(position, validated.inserted_scalars())?;

    let rendered_scalar = validated.rendered_scalar_position();
    let rendered_byte = scalar_byte_offset(validated.rendered_text(), rendered_scalar)?;
    let rendered_capacity = validated.rendered_text().len().checked_add(text.len())?;
    let mut rendered_text = String::new();
    rendered_text.try_reserve_exact(rendered_capacity).ok()?;
    rendered_text.push_str(&validated.rendered_text()[..rendered_byte]);
    rendered_text.push_str(text);
    rendered_text.push_str(&validated.rendered_text()[rendered_byte..]);

    let top_level_count = context.document.root().child_count();
    let affected_start = validated.affected_top_level_index().saturating_sub(1);
    if affected_start >= top_level_count {
        return None;
    }
    let affected_len = top_level_count.checked_sub(affected_start)?;
    let mut affected_top_level_blocks = Vec::new();
    affected_top_level_blocks
        .try_reserve_exact(affected_len)
        .ok()?;
    affected_top_level_blocks.extend(affected_start..top_level_count);

    Some(LocalizedSemanticCompilation {
        position,
        preview,
        step_map,
        derivations: LocalizedSemanticDerivations {
            affected_top_level_blocks,
            rendered_text,
            rendered_scalars: validated.next_rendered_scalars(),
            document_text_bytes: validated.next_raw_text_utf8_bytes(),
            document_node_count: validated.document_node_count(),
            raw_text_scalars: validated.next_raw_text_scalars(),
            raw_text_utf8_bytes: validated.next_raw_text_utf8_bytes(),
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn derive_preview_document(
    request_id: u64,
    context: CompilationContext<'_>,
    base_position_map: &PositionMap,
    preview: &Document,
    composed_map: &StepMap,
    position_update_mode: UpdateMode,
    affected_top_level_blocks: &[usize],
) -> OperationResult<CompiledDocumentDerivations> {
    super::derived_state::record_preview_position_map_derivation();
    #[cfg(test)]
    super::observability::record_position_map_clone();
    let mut position_map = base_position_map.clone();
    let update_mode = if affected_top_level_blocks.is_empty() && preview != context.document {
        UpdateMode::Rebuild
    } else {
        position_update_mode
    };
    position_map.update(
        composed_map,
        context.document,
        preview,
        update_mode,
        context.schema,
    );
    #[cfg(test)]
    super::observability::record_position_map_compaction();
    position_map.compact();
    super::derived_state::record_preview_rendered_text_derivation();
    let rendered_text = crate::render::rendered_text(preview, context.schema);
    let rendered_scalars = u32::try_from(rendered_text.chars().count()).map_err(|_| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "preview rendered scalar count exceeds the position domain",
        )
    })?;
    if rendered_scalars != position_map.total_scalars() {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "preview rendered text and position map have different scalar lengths",
        ));
    }
    let document_text_bytes = document_text_bytes(preview).ok_or_else(|| {
        OperationError::engine_invariant_failed(
            request_id,
            None,
            "preview document text byte metric overflowed",
        )
    })?;
    #[cfg(test)]
    super::observability::record_document_node_count_scan();
    let document_node_count = crate::editor_state::document_node_count(preview.root());
    Ok(CompiledDocumentDerivations {
        identity_seal: Arc::new(()),
        position_map,
        rendered_text,
        rendered_scalars,
        document_text_bytes,
        document_node_count,
    })
}

#[allow(clippy::too_many_arguments)]
fn derive_localized_preview_document(
    request_id: u64,
    context: CompilationContext<'_>,
    base_position_map: &PositionMap,
    preview: &Document,
    composed_map: &StepMap,
    position_update_mode: UpdateMode,
    affected_top_level_blocks: &[usize],
    localized: LocalizedSemanticDerivations,
) -> OperationResult<CompiledDocumentDerivations> {
    super::derived_state::record_preview_position_map_derivation();
    #[cfg(test)]
    super::observability::record_position_map_clone();
    let mut position_map = base_position_map.clone();
    let update_mode = if affected_top_level_blocks.is_empty() && preview != context.document {
        UpdateMode::Rebuild
    } else {
        position_update_mode
    };
    position_map.update(
        composed_map,
        context.document,
        preview,
        update_mode,
        context.schema,
    );
    #[cfg(test)]
    super::observability::record_position_map_compaction();
    position_map.compact();
    if localized.rendered_scalars != position_map.total_scalars() {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "localized preview rendered text and position map have different scalar lengths",
        ));
    }
    Ok(CompiledDocumentDerivations {
        identity_seal: Arc::new(()),
        position_map,
        rendered_text: localized.rendered_text,
        rendered_scalars: localized.rendered_scalars,
        document_text_bytes: localized.document_text_bytes,
        document_node_count: localized.document_node_count,
    })
}

#[allow(clippy::too_many_arguments)]
fn selection_plan(
    context: CompilationContext<'_>,
    intent: &SelectionIntent,
    rendered_text: &str,
    base_position_map: &PositionMap,
    composed_map: &StepMap,
    operation_result: Option<Selection>,
    request_id: u64,
    preview: &Document,
    prepared_preview_map: Option<&PositionMap>,
) -> OperationResult<SelectionPlan> {
    let owned_preview_map;
    let preview_map = if let Some(prepared) = prepared_preview_map {
        prepared
    } else {
        super::derived_state::record_preview_position_map_derivation();
        owned_preview_map = PositionMap::build(preview, context.schema);
        &owned_preview_map
    };
    let uses_preserved_fallback =
        matches!(intent, SelectionIntent::UseOperationResult) && operation_result.is_none();
    let mut candidate = match intent {
        SelectionIntent::Preserve => context
            .selection
            .map(|selection| selection.map(composed_map).normalized(preview, preview_map)),
        SelectionIntent::UseOperationResult => operation_result
            .or_else(|| {
                context
                    .selection
                    .map(|selection| selection.map(composed_map))
            })
            .map(|selection| selection.normalized(preview, preview_map)),
        SelectionIntent::Set(input) => Some(
            match input {
                super::SelectionInput::Text { anchor, head } => Selection::text(
                    map_position(
                        composed_map,
                        resolve_position(
                            request_id,
                            None,
                            "selection.anchor",
                            *anchor,
                            rendered_text,
                            base_position_map,
                            context.document,
                        )?,
                        anchor.affinity,
                    ),
                    map_position(
                        composed_map,
                        resolve_position(
                            request_id,
                            None,
                            "selection.head",
                            *head,
                            rendered_text,
                            base_position_map,
                            context.document,
                        )?,
                        head.affinity,
                    ),
                ),
                super::SelectionInput::Node { at } => Selection::node(map_position(
                    composed_map,
                    resolve_position(
                        request_id,
                        None,
                        "selection.at",
                        *at,
                        rendered_text,
                        base_position_map,
                        context.document,
                    )?,
                    at.affinity,
                )),
                super::SelectionInput::All => Selection::all(),
            }
            .normalized(preview, preview_map),
        ),
    };

    if let Some(Selection::Node { pos }) = candidate.as_ref() {
        let pos = *pos;
        if !selectable_void_at(preview.root(), pos, 0, context.schema) {
            match intent {
                SelectionIntent::Set(super::SelectionInput::Node { .. }) => {
                    return Err(OperationError::selection_position_invalid(
                        request_id,
                        "selection.at",
                        "node selection must target a selectable void or atom node",
                    ));
                }
                SelectionIntent::Preserve => {
                    candidate = Some(Selection::cursor(pos).normalized(preview, preview_map));
                }
                SelectionIntent::UseOperationResult if uses_preserved_fallback => {
                    candidate = Some(Selection::cursor(pos).normalized(preview, preview_map));
                }
                SelectionIntent::UseOperationResult => {
                    return Err(OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "operation result produced a node selection for a non-selectable node",
                    ));
                }
                SelectionIntent::Set(_) => {
                    return Err(OperationError::engine_invariant_failed(
                        request_id,
                        None,
                        "non-node explicit selection compiled to an invalid node selection",
                    ));
                }
            }
        }
    } else if matches!(
        intent,
        SelectionIntent::Set(super::SelectionInput::Node { .. })
    ) {
        return Err(OperationError::engine_invariant_failed(
            request_id,
            None,
            "node selection did not compile to a node selection",
        ));
    }

    match (intent, candidate) {
        (_, None) => Ok(SelectionPlan::Preserve),
        (SelectionIntent::Preserve, Some(_)) if preview == context.document => {
            Ok(SelectionPlan::Preserve)
        }
        (SelectionIntent::Preserve, Some(candidate)) => Ok(SelectionPlan::Mapped(candidate)),
        (SelectionIntent::UseOperationResult, Some(_))
            if uses_preserved_fallback && preview == context.document =>
        {
            Ok(SelectionPlan::Preserve)
        }
        (SelectionIntent::UseOperationResult, Some(candidate)) if uses_preserved_fallback => {
            Ok(SelectionPlan::Mapped(candidate))
        }
        (_, Some(candidate)) => Ok(SelectionPlan::Explicit(candidate)),
    }
}

pub(crate) fn selectable_void_at(
    node: &Node,
    target: u32,
    content_start: u32,
    schema: &Schema,
) -> bool {
    let Some(content) = node.content() else {
        return false;
    };
    let mut offset = content_start;
    for child in content.iter() {
        let selectable = child.is_void()
            || schema
                .node(child.node_type())
                .is_some_and(|spec| spec.is_void);
        if selectable && target == offset {
            return true;
        }
        if child.content().is_some()
            && target > offset
            && target < offset.saturating_add(child.node_size())
            && selectable_void_at(child, target, offset.saturating_add(1), schema)
        {
            return true;
        }
        offset = offset.saturating_add(child.node_size());
    }
    false
}

fn affected_top_level_blocks(before: &Document, after: &Document) -> Vec<usize> {
    #[cfg(test)]
    super::observability::record_affected_top_level_scan();
    if before == after {
        return Vec::new();
    }
    let before_children = before
        .root()
        .content()
        .map(|content| content.children())
        .unwrap_or(&[]);
    let after_children = after
        .root()
        .content()
        .map(|content| content.children())
        .unwrap_or(&[]);
    let mut prefix = 0usize;
    while prefix < before_children.len()
        && prefix < after_children.len()
        && before_children[prefix] == after_children[prefix]
    {
        prefix += 1;
    }
    let start = prefix.saturating_sub(1);
    let end = before_children.len().max(after_children.len());
    (start..end).collect()
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
