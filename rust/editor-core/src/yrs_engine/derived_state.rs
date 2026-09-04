use sha2::Digest;
use std::sync::Arc;
use yrs::branch::{Branch, BranchID};
use yrs::types::xml::XmlFragmentRef;

use yrs::{Assoc, ReadTxn};

use crate::boundary::ResourceLimits;
use crate::editor_state::ActiveState;
use crate::model::{Document, Mark, Node};
use crate::position::build::{classify_position_block, PositionBlockKind};
use crate::position::update::UpdateMode;
use crate::position::PositionMap;
use crate::schema::Schema;
use crate::selection::Selection;
use crate::transform::{
    DocumentStats, DocumentValidationMetrics, DocumentValidationReport, DocumentValidator, StepMap,
};

use super::canonical::CanonicalArtifact;
use super::codec::YrsDocumentCodec;
use super::compiler::{selectable_void_at, CachedCompilationView, CompiledDocumentDerivations};
use super::position::{
    cursor_sticky_index_from_doc_pos, doc_pos_to_relative_point, doc_pos_to_sticky_index,
    relative_point_to_doc_pos, relative_selection_to_selection,
};
use super::prepared_admission::DerivedStateAuthority;
use super::{
    scalar_offset_to_utf16, Affinity, OperationError, OperationResult, RelativePoint,
    RelativeSelection, ResolvedPoint, ResolvedSelection, TypedOperation,
};

#[cfg(test)]
std::thread_local! {
    static PREVIEW_POSITION_MAP_DERIVATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static PREVIEW_RENDERED_TEXT_DERIVATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FORCE_INITIALIZE_SCALAR_MISMATCH: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static LOCALIZED_INDEX_BUILD_VISITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_PATH_HOPS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_LOOKUP_COMPARISONS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_PATH_COPY_ELEMENTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_PATH_COMPARISON_ELEMENTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_PROMOTION_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_PROMOTION_SUCCESSES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static LOCALIZED_INDEX_PROMOTION_DROPS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    static FORCE_LOCALIZED_INDEX_ALLOCATION_STAGE: std::cell::Cell<Option<LocalizedIndexAllocationStage>> = const {
        std::cell::Cell::new(None)
    };
    static FORCE_LOCALIZED_INDEX_BUDGET: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    static LOCALIZED_INSERT_ADMISSION_WORK: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    static ACTIVE_STATE_CACHE_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_CACHE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_CACHE_FALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_GENERIC_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_CANDIDATE_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_CACHE_INSTALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_CACHE_DROPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_PUBLIC_RESULT_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ACTIVE_STATE_FULL_ASSEMBLIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FORCE_ACTIVE_STATE_CACHE_ALLOCATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FORCE_ACTIVE_STATE_CACHE_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static FORCE_ACTIVE_STATE_CACHE_HIT_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FORCE_ACTIVE_STATE_PUBLIC_MATERIALIZATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static OPERATION_RESULT_RELATIVE_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RELATIVE_SELECTION_RESOLUTION_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREWRITE_SELECTION_PROOF_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREWRITE_SELECTION_PROOF_FINALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREWRITE_SELECTION_PROOF_FALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PREWRITE_SELECTION_PROOF_INSTALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FORCE_HISTORY_SNAPSHOT_SEMANTIC_FALLBACK: std::cell::Cell<Option<HistorySnapshotSemanticFallbackForTest>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) struct ForcedHistoryDocumentSnapshotFallback {
    previous: bool,
}

#[cfg(test)]
impl Drop for ForcedHistoryDocumentSnapshotFallback {
    fn drop(&mut self) {
        FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK.set(self.previous);
    }
}

#[cfg(test)]
pub(crate) fn force_history_document_snapshot_fallback_for_test(
) -> ForcedHistoryDocumentSnapshotFallback {
    ForcedHistoryDocumentSnapshotFallback {
        previous: FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK.replace(true),
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HistorySnapshotSemanticFallbackForTest {
    RenderIdentity,
    RelativeSelection,
    ResolvedSelection,
    ResolvedMismatch,
}

#[cfg(test)]
pub(crate) struct ForcedHistorySnapshotSemanticFallback {
    previous: Option<HistorySnapshotSemanticFallbackForTest>,
}

#[cfg(test)]
impl Drop for ForcedHistorySnapshotSemanticFallback {
    fn drop(&mut self) {
        FORCE_HISTORY_SNAPSHOT_SEMANTIC_FALLBACK.set(self.previous);
    }
}

#[cfg(test)]
pub(crate) fn force_history_snapshot_semantic_fallback_for_test(
    stage: HistorySnapshotSemanticFallbackForTest,
) -> ForcedHistorySnapshotSemanticFallback {
    ForcedHistorySnapshotSemanticFallback {
        previous: FORCE_HISTORY_SNAPSHOT_SEMANTIC_FALLBACK.replace(Some(stage)),
    }
}

#[cfg(test)]
fn history_snapshot_semantic_fallback_forced(
    stage: HistorySnapshotSemanticFallbackForTest,
) -> bool {
    FORCE_HISTORY_SNAPSHOT_SEMANTIC_FALLBACK.get() == Some(stage)
}

#[cfg(test)]
pub(crate) fn reset_preview_derivation_counts_for_test() {
    PREVIEW_POSITION_MAP_DERIVATION_COUNT.set(0);
    PREVIEW_RENDERED_TEXT_DERIVATION_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_preview_derivation_counts_for_test() -> (usize, usize) {
    (
        PREVIEW_POSITION_MAP_DERIVATION_COUNT.replace(0),
        PREVIEW_RENDERED_TEXT_DERIVATION_COUNT.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn reset_localized_index_metrics_for_test() {
    LOCALIZED_INDEX_BUILD_VISITS.set(0);
    LOCALIZED_INDEX_PATH_HOPS.set(0);
    LOCALIZED_INDEX_LOOKUP_COMPARISONS.set(0);
    LOCALIZED_INDEX_PATH_COPY_ELEMENTS.set(0);
    LOCALIZED_INDEX_PATH_COMPARISON_ELEMENTS.set(0);
}

#[cfg(test)]
pub(crate) fn take_localized_index_metrics_for_test() -> (usize, usize, usize, usize, usize) {
    (
        LOCALIZED_INDEX_PATH_HOPS.replace(0),
        LOCALIZED_INDEX_BUILD_VISITS.replace(0),
        LOCALIZED_INDEX_LOOKUP_COMPARISONS.replace(0),
        LOCALIZED_INDEX_PATH_COPY_ELEMENTS.replace(0),
        LOCALIZED_INDEX_PATH_COMPARISON_ELEMENTS.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn reset_localized_index_lifecycle_counts_for_test() {
    LOCALIZED_INDEX_BUILD_COUNT.set(0);
    LOCALIZED_INDEX_PROMOTION_ATTEMPTS.set(0);
    LOCALIZED_INDEX_PROMOTION_SUCCESSES.set(0);
    LOCALIZED_INDEX_PROMOTION_DROPS.set(0);
}

#[cfg(test)]
pub(crate) fn take_localized_index_lifecycle_counts_for_test() -> (usize, usize, usize, usize) {
    (
        LOCALIZED_INDEX_BUILD_COUNT.replace(0),
        LOCALIZED_INDEX_PROMOTION_ATTEMPTS.replace(0),
        LOCALIZED_INDEX_PROMOTION_SUCCESSES.replace(0),
        LOCALIZED_INDEX_PROMOTION_DROPS.replace(0),
    )
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalizedIndexAllocationStage {
    InitialLeafCapacity,
    TraversalPath,
    LeafGrowth,
    PromotionClone,
    PromotionGrowth,
    PromotionUpdate,
}

#[cfg(test)]
fn forced_localized_index_allocation_stage(stage: LocalizedIndexAllocationStage) -> bool {
    FORCE_LOCALIZED_INDEX_ALLOCATION_STAGE.get() == Some(stage)
}

#[cfg(test)]
pub(crate) fn force_localized_index_allocation_stage_for_test(
    stage: Option<LocalizedIndexAllocationStage>,
) {
    FORCE_LOCALIZED_INDEX_ALLOCATION_STAGE.set(stage);
}

#[cfg(test)]
pub(crate) fn force_localized_index_allocation_failure_for_test(force: bool) {
    FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE.set(force);
}

#[cfg(test)]
pub(crate) fn force_localized_index_budget_for_test(budget: Option<usize>) {
    FORCE_LOCALIZED_INDEX_BUDGET.set(budget);
}

#[cfg(test)]
pub(crate) fn reset_localized_insert_admission_work_for_test() {
    LOCALIZED_INSERT_ADMISSION_WORK.set(0);
}

#[cfg(test)]
pub(crate) fn take_localized_insert_admission_work_for_test() -> usize {
    LOCALIZED_INSERT_ADMISSION_WORK.replace(0)
}

#[cfg(test)]
pub(crate) fn reset_active_state_cache_counts_for_test() {
    ACTIVE_STATE_CACHE_ATTEMPTS.set(0);
    ACTIVE_STATE_CACHE_HITS.set(0);
    ACTIVE_STATE_CACHE_FALLBACKS.set(0);
    ACTIVE_STATE_GENERIC_BUILDS.set(0);
    ACTIVE_STATE_CANDIDATE_BUILDS.set(0);
    ACTIVE_STATE_CACHE_INSTALLS.set(0);
    ACTIVE_STATE_CACHE_DROPS.set(0);
    ACTIVE_STATE_PUBLIC_RESULT_CLONES.set(0);
    ACTIVE_STATE_FULL_ASSEMBLIES.set(0);
}

#[cfg(test)]
/// Returns `(attempts, hits, fallbacks, generic_builds, candidate_builds,
/// installs, drops, public_result_clones, full_assemblies)`.
pub(crate) fn take_active_state_cache_counts_for_test() -> (
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
) {
    (
        ACTIVE_STATE_CACHE_ATTEMPTS.replace(0),
        ACTIVE_STATE_CACHE_HITS.replace(0),
        ACTIVE_STATE_CACHE_FALLBACKS.replace(0),
        ACTIVE_STATE_GENERIC_BUILDS.replace(0),
        ACTIVE_STATE_CANDIDATE_BUILDS.replace(0),
        ACTIVE_STATE_CACHE_INSTALLS.replace(0),
        ACTIVE_STATE_CACHE_DROPS.replace(0),
        ACTIVE_STATE_PUBLIC_RESULT_CLONES.replace(0),
        ACTIVE_STATE_FULL_ASSEMBLIES.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn force_active_state_cache_allocation_failure_for_test(force: bool) {
    FORCE_ACTIVE_STATE_CACHE_ALLOCATION_FAILURE.set(force);
}

#[cfg(test)]
pub(crate) fn force_active_state_cache_budget_for_test(budget: Option<usize>) {
    FORCE_ACTIVE_STATE_CACHE_BUDGET.set(budget);
}

#[cfg(test)]
pub(crate) fn force_active_state_cache_hit_fallback_for_test(force: bool) {
    FORCE_ACTIVE_STATE_CACHE_HIT_FALLBACK.set(force);
}

#[cfg(test)]
pub(crate) fn force_active_state_public_materialization_failure_for_test(force: bool) {
    FORCE_ACTIVE_STATE_PUBLIC_MATERIALIZATION_FAILURE.set(force);
}

#[cfg(test)]
pub(crate) fn reset_relative_selection_traversal_counts_for_test() {
    OPERATION_RESULT_RELATIVE_TRAVERSALS.set(0);
    RELATIVE_SELECTION_RESOLUTION_TRAVERSALS.set(0);
}

#[cfg(test)]
pub(crate) fn take_relative_selection_traversal_counts_for_test() -> (usize, usize) {
    (
        OPERATION_RESULT_RELATIVE_TRAVERSALS.replace(0),
        RELATIVE_SELECTION_RESOLUTION_TRAVERSALS.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn reset_prewrite_selection_proof_counts_for_test() {
    PREWRITE_SELECTION_PROOF_ATTEMPTS.set(0);
    PREWRITE_SELECTION_PROOF_FINALIZATIONS.set(0);
    PREWRITE_SELECTION_PROOF_FALLBACKS.set(0);
    PREWRITE_SELECTION_PROOF_INSTALLS.set(0);
}

#[cfg(test)]
pub(crate) fn take_prewrite_selection_proof_counts_for_test() -> (usize, usize, usize, usize) {
    (
        PREWRITE_SELECTION_PROOF_ATTEMPTS.replace(0),
        PREWRITE_SELECTION_PROOF_FINALIZATIONS.replace(0),
        PREWRITE_SELECTION_PROOF_FALLBACKS.replace(0),
        PREWRITE_SELECTION_PROOF_INSTALLS.replace(0),
    )
}

macro_rules! prewrite_selection_counter {
    ($name:ident, $counter:ident) => {
        #[inline]
        pub(crate) fn $name() {
            #[cfg(test)]
            $counter.set($counter.get().saturating_add(1));
        }
    };
}

prewrite_selection_counter!(
    record_prewrite_selection_proof_attempt,
    PREWRITE_SELECTION_PROOF_ATTEMPTS
);
prewrite_selection_counter!(
    record_prewrite_selection_proof_finalization,
    PREWRITE_SELECTION_PROOF_FINALIZATIONS
);
prewrite_selection_counter!(
    record_prewrite_selection_proof_fallback,
    PREWRITE_SELECTION_PROOF_FALLBACKS
);
prewrite_selection_counter!(
    record_prewrite_selection_proof_install,
    PREWRITE_SELECTION_PROOF_INSTALLS
);

#[inline]
pub(crate) fn active_state_cache_hit_fallback_forced() -> bool {
    #[cfg(test)]
    return FORCE_ACTIVE_STATE_CACHE_HIT_FALLBACK.get();
    #[cfg(not(test))]
    false
}

macro_rules! active_state_counter {
    ($name:ident, $counter:ident) => {
        #[inline]
        pub(crate) fn $name() {
            #[cfg(test)]
            $counter.set($counter.get().saturating_add(1));
        }
    };
}

active_state_counter!(
    record_active_state_cache_attempt,
    ACTIVE_STATE_CACHE_ATTEMPTS
);
active_state_counter!(record_active_state_cache_hit, ACTIVE_STATE_CACHE_HITS);
active_state_counter!(
    record_active_state_cache_fallback,
    ACTIVE_STATE_CACHE_FALLBACKS
);
active_state_counter!(
    record_active_state_generic_build,
    ACTIVE_STATE_GENERIC_BUILDS
);
active_state_counter!(
    record_active_state_candidate_build,
    ACTIVE_STATE_CANDIDATE_BUILDS
);
active_state_counter!(
    record_active_state_cache_install,
    ACTIVE_STATE_CACHE_INSTALLS
);
active_state_counter!(record_active_state_cache_drop, ACTIVE_STATE_CACHE_DROPS);
active_state_counter!(
    record_active_state_public_result_clone,
    ACTIVE_STATE_PUBLIC_RESULT_CLONES
);
active_state_counter!(
    record_active_state_full_assembly,
    ACTIVE_STATE_FULL_ASSEMBLIES
);

#[inline]
pub(crate) fn record_preview_position_map_derivation() {
    #[cfg(test)]
    PREVIEW_POSITION_MAP_DERIVATION_COUNT.set(
        PREVIEW_POSITION_MAP_DERIVATION_COUNT
            .get()
            .saturating_add(1),
    );
}

#[inline]
pub(crate) fn record_preview_rendered_text_derivation() {
    #[cfg(test)]
    PREVIEW_RENDERED_TEXT_DERIVATION_COUNT.set(
        PREVIEW_RENDERED_TEXT_DERIVATION_COUNT
            .get()
            .saturating_add(1),
    );
}

/// Reusable document-validation evidence with a controlled state-revision
/// reseal. Document, schema, limits, canonical, and epoch facts never mutate.
#[derive(Debug, Clone)]
pub(crate) struct ValidatedDocumentEvidence {
    document_root: Node,
    canonical_artifact: CanonicalArtifact,
    canonical_format_version: u8,
    validation: DocumentValidationReport,
    validation_report_seal: [usize; 4],
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: Arc<str>,
    canonical_schema: super::canonical::CanonicalSchemaContext,
    fragment_name: Arc<str>,
    store_token: usize,
    fragment_id: BranchID,
    engine_epoch: u64,
    target_document_revision: u64,
    target_state_revision: u64,
    target_yrs_state_epoch: u64,
}

pub(crate) struct ValidatedCandidateContext<'a> {
    pub evidence: &'a ValidatedDocumentEvidence,
    pub canonical_schema: &'a super::canonical::CanonicalSchemaContext,
    pub fragment_name: &'a str,
    pub engine_epoch: u64,
}

impl ValidatedDocumentEvidence {
    fn validation_report_seal(validation: DocumentValidationReport) -> [usize; 4] {
        [
            validation.stats.node_count,
            validation.stats.max_depth,
            validation.metrics.metadata_bytes,
            validation.metrics.validation_work,
        ]
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn mint<T: ReadTxn>(
        document: &Document,
        validation_source_root: &Node,
        canonical_artifact: &CanonicalArtifact,
        validation: DocumentValidationReport,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
        fragment_name: &str,
        txn: &T,
        fragment: &XmlFragmentRef,
        engine_epoch: u64,
        target_document_revision: u64,
        target_state_revision: u64,
        target_yrs_state_epoch: u64,
    ) -> Option<Self> {
        if !validation_source_root.shares_storage_with(document.root())
            || !canonical_artifact.matches_exact_source_document(document)
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !canonical_artifact.schema_context().ptr_eq(canonical_schema)
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || canonical_schema.schema_fingerprint() != schema_fingerprint
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || validation.metrics.metadata_bytes > resource_limits.max_input_bytes
            || validation.metrics.validation_work
                > resource_limits.max_document_nodes.saturating_mul(128)
            || max_length
                .is_some_and(|limit| canonical_artifact.text_scalar_len() > u64::from(limit))
        {
            return None;
        }
        #[cfg(test)]
        super::observability::record_validated_evidence_construction();
        Some(Self {
            document_root: document.root().clone(),
            canonical_artifact: canonical_artifact.clone(),
            canonical_format_version: canonical_artifact.format_version(),
            validation,
            validation_report_seal: Self::validation_report_seal(validation),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: schema_fingerprint.into(),
            canonical_schema: canonical_schema.clone(),
            fragment_name: fragment_name.into(),
            store_token: txn.store() as *const _ as usize,
            fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
            engine_epoch,
            target_document_revision,
            target_state_revision,
            target_yrs_state_epoch,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admitted_validation_report<T: ReadTxn>(
        &self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
        fragment_name: &str,
        txn: &T,
        fragment: &XmlFragmentRef,
        engine_epoch: u64,
        target_document_revision: u64,
        target_state_revision: u64,
        target_yrs_state_epoch: u64,
    ) -> Option<DocumentValidationReport> {
        (self.document_root.shares_storage_with(document.root())
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && self
                .canonical_artifact
                .matches_exact_source_document(document)
            && self.canonical_format_version == canonical_artifact.format_version()
            && self.canonical_format_version == super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            && self.validation_report_seal == Self::validation_report_seal(self.validation)
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.canonical_schema.ptr_eq(canonical_schema)
            && self
                .canonical_artifact
                .schema_context()
                .ptr_eq(canonical_schema)
            && self.fragment_name.as_ref() == fragment_name
            && self.store_token == txn.store() as *const _ as usize
            && self.fragment_id == AsRef::<Branch>::as_ref(fragment).id()
            && self.engine_epoch == engine_epoch
            && self.target_document_revision == target_document_revision
            && self.target_state_revision == target_state_revision
            && self.target_yrs_state_epoch == target_yrs_state_epoch)
            .then_some(self.validation)
    }

    #[cfg(test)]
    fn tampered_for_test(&self, schema: &Schema) -> Vec<(&'static str, Self)> {
        let foreign_document = crate::serialize::from_prosemirror_json(
            &serde_json::json!({
                "type": schema.doc_node_type(),
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "foreign"}]}]
            }),
            schema,
            crate::serialize::UnknownTypeMode::Preserve,
        )
        .expect("foreign evidence fixture should parse");
        let foreign_artifact = self
            .canonical_schema
            .derive(&foreign_document)
            .expect("foreign evidence fixture should canonicalize");
        let mut variants = Vec::new();
        macro_rules! tamper {
            ($name:literal, $field:ident, $value:expr) => {{
                let mut value = self.clone();
                value.$field = $value;
                variants.push(($name, value));
            }};
        }
        tamper!(
            "documentRoot",
            document_root,
            foreign_document.root().clone()
        );
        tamper!("canonicalArtifact", canonical_artifact, foreign_artifact);
        tamper!(
            "canonicalFormat",
            canonical_format_version,
            self.canonical_format_version.wrapping_add(1)
        );
        tamper!(
            "schemaContext",
            canonical_schema,
            super::canonical::CanonicalSchemaContext::new(schema)
        );
        tamper!(
            "schemaFingerprint",
            schema_fingerprint,
            Arc::<str>::from("tampered")
        );
        tamper!("fragment", fragment_name, Arc::<str>::from("tampered"));
        tamper!("store", store_token, self.store_token.wrapping_add(1));
        let foreign_doc = yrs::Doc::new();
        let foreign_fragment = foreign_doc.get_or_insert_xml_fragment("foreign");
        tamper!(
            "fragmentIdentity",
            fragment_id,
            AsRef::<Branch>::as_ref(&foreign_fragment).id()
        );
        tamper!(
            "engineEpoch",
            engine_epoch,
            self.engine_epoch.wrapping_add(1)
        );
        tamper!(
            "documentRevision",
            target_document_revision,
            self.target_document_revision.wrapping_add(1)
        );
        tamper!(
            "stateRevision",
            target_state_revision,
            self.target_state_revision.wrapping_add(1)
        );
        tamper!(
            "targetEpoch",
            target_yrs_state_epoch,
            self.target_yrs_state_epoch.wrapping_add(1)
        );
        let mut resource_limits = self.resource_limits.clone();
        resource_limits.max_document_nodes = resource_limits.max_document_nodes.saturating_add(1);
        tamper!("resourceLimits", resource_limits, resource_limits);
        let mut editing_limits = self.editing_limits.clone();
        editing_limits.max_derived_output_bytes =
            editing_limits.max_derived_output_bytes.saturating_add(1);
        tamper!("editingLimits", editing_limits, editing_limits);
        tamper!(
            "maxLength",
            max_length,
            self.max_length.map(|value| value + 1)
        );
        let mut validation = self.validation;
        validation.stats.node_count = validation.stats.node_count.saturating_add(1);
        tamper!("validationReport", validation, validation);
        variants
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DocumentValidationCertificate {
    stats: DocumentStats,
    metrics: DocumentValidationMetrics,
    resource_limits: ResourceLimits,
    schema_fingerprint: Arc<str>,
    canonical_artifact: CanonicalArtifact,
    canonical_fingerprint: [u8; 32],
    canonical_serialized_len: usize,
    canonical_fingerprint_materialized: bool,
    raw_text_scalars: u64,
    raw_text_utf8_bytes: usize,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
}

impl PartialEq for DocumentValidationCertificate {
    fn eq(&self, other: &Self) -> bool {
        let canonical_identity_matches = if self.canonical_fingerprint_materialized
            && other.canonical_fingerprint_materialized
        {
            self.canonical_fingerprint == other.canonical_fingerprint
                && self.canonical_serialized_len == other.canonical_serialized_len
        } else if !self.canonical_fingerprint_materialized
            && !other.canonical_fingerprint_materialized
        {
            self.canonical_artifact.ptr_eq(&other.canonical_artifact)
        } else {
            false
        };
        self.stats == other.stats
            && self.metrics == other.metrics
            && self.resource_limits == other.resource_limits
            && self.schema_fingerprint == other.schema_fingerprint
            && canonical_identity_matches
            && self.raw_text_scalars == other.raw_text_scalars
            && self.raw_text_utf8_bytes == other.raw_text_utf8_bytes
            && self.document_revision == other.document_revision
            && self.state_revision == other.state_revision
            && self.yrs_state_epoch == other.yrs_state_epoch
    }
}

impl Eq for DocumentValidationCertificate {}

#[derive(Debug, Clone)]
pub(crate) struct PreparedCandidateValidation {
    document: Document,
    canonical_artifact: CanonicalArtifact,
    validation: DocumentValidationReport,
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: Arc<str>,
    derivations: CompiledDocumentDerivations,
}

#[derive(Debug)]
pub(crate) struct PreparedCandidateEvidence {
    document: Document,
    validation_seal: DocumentValidationReport,
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: Arc<str>,
    canonical_schema: super::canonical::CanonicalSchemaContext,
    position: PreparedPositionEvidence,
    position_identity_seal: Arc<()>,
    render: PreparedRenderEvidence,
    render_identity_seal: Arc<()>,
    document_text_bytes: usize,
    document_node_count: usize,
    raw_text_scalars: u64,
    raw_text_utf8_bytes: usize,
    history_render: Option<PreparedHistoryRenderEvidence>,
}

#[derive(Debug)]
struct PreparedPositionEvidence {
    position_map: PositionMap,
    total_scalars: u32,
    block_count: usize,
    identity: Arc<()>,
}

#[derive(Debug)]
struct PreparedRenderEvidence {
    rendered_text: String,
    rendered_scalars: u32,
    identity: Arc<()>,
}

#[derive(Debug)]
struct PreparedHistoryRenderEvidence {
    base_document_root: crate::model::Node,
    target_top_level_index: usize,
    inserted_scalar_delta: u32,
    candidate_render_identity: Arc<()>,
}

impl PreparedCandidateEvidence {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_deferred(
        base_document: &Document,
        base_position_map: &PositionMap,
        base_rendered_text: &str,
        document: &Document,
        validation: DocumentValidationReport,
        schema: &Schema,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        raw_text_scalars: u64,
        raw_text_utf8_bytes: usize,
        inserted_document_position: u32,
        inserted_text: &str,
        target_top_level_index: usize,
        inserted_scalar_delta: u32,
    ) -> Option<Self> {
        let schema_fingerprint = crate::schema::schema_fingerprint(schema);
        if schema_fingerprint != canonical_schema.schema_fingerprint()
            || validation.stats.node_count == 0
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || max_length.is_some_and(|limit| raw_text_scalars > u64::from(limit))
            || inserted_scalar_delta == 0
            || base_document.root().child_count() != document.root().child_count()
            || target_top_level_index >= base_document.root().child_count()
        {
            return None;
        }
        let old_target = base_document.root().child(target_top_level_index)?;
        let new_target = document.root().child(target_top_level_index)?;
        if old_target.node_size().checked_add(inserted_scalar_delta)? != new_target.node_size() {
            return None;
        }
        crate::transform::validate_canonical_marks(document, schema).ok()?;
        record_preview_position_map_derivation();
        #[cfg(test)]
        super::observability::record_position_map_clone();
        let mut position_map = base_position_map.clone();
        let step_map = StepMap::try_from_insert(inserted_document_position, inserted_scalar_delta)?;
        position_map.update(
            &step_map,
            base_document,
            document,
            UpdateMode::InlineTextOnly,
            schema,
        );
        #[cfg(test)]
        super::observability::record_position_map_compaction();
        position_map.compact();

        let rendered_scalar =
            base_position_map.doc_to_scalar(inserted_document_position, base_document);
        if base_position_map.scalar_to_doc(rendered_scalar, base_document)
            != inserted_document_position
            || u32::try_from(inserted_text.chars().count()).ok()? != inserted_scalar_delta
        {
            return None;
        }
        let rendered_byte = if rendered_scalar == base_position_map.total_scalars() {
            base_rendered_text.len()
        } else {
            base_rendered_text
                .char_indices()
                .nth(usize::try_from(rendered_scalar).ok()?)?
                .0
        };
        let rendered_capacity = base_rendered_text.len().checked_add(inserted_text.len())?;
        let mut rendered_text = String::new();
        rendered_text.try_reserve_exact(rendered_capacity).ok()?;
        rendered_text.push_str(base_rendered_text.get(..rendered_byte)?);
        rendered_text.push_str(inserted_text);
        rendered_text.push_str(base_rendered_text.get(rendered_byte..)?);
        let rendered_scalars = base_position_map
            .total_scalars()
            .checked_add(inserted_scalar_delta)?;
        if rendered_scalars != position_map.total_scalars() {
            return None;
        }
        let position_identity_seal = Arc::new(());
        let render_identity_seal = Arc::new(());
        Some(Self {
            document: document.clone(),
            validation_seal: validation,
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: schema_fingerprint.into(),
            canonical_schema: canonical_schema.clone(),
            position: PreparedPositionEvidence {
                total_scalars: position_map.total_scalars(),
                block_count: position_map.block_count(),
                position_map,
                identity: Arc::clone(&position_identity_seal),
            },
            position_identity_seal,
            render: PreparedRenderEvidence {
                rendered_text,
                rendered_scalars,
                identity: Arc::clone(&render_identity_seal),
            },
            render_identity_seal: Arc::clone(&render_identity_seal),
            document_text_bytes: raw_text_utf8_bytes,
            document_node_count: validation.stats.node_count,
            raw_text_scalars,
            raw_text_utf8_bytes,
            history_render: Some(PreparedHistoryRenderEvidence {
                base_document_root: base_document.root().clone(),
                target_top_level_index,
                inserted_scalar_delta,
                candidate_render_identity: Arc::clone(&render_identity_seal),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_history_render_transition(
        &self,
        state: &DerivedStateCache,
        document: &Document,
        derivations: &CompiledDocumentDerivations,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
    ) -> Option<
        Result<
            crate::render::incremental::CachedRenderTransition,
            crate::render::incremental::CachedRenderError,
        >,
    > {
        let proof = self.history_render.as_ref()?;
        let expected_rendered_scalars = state
            .rendered_scalars
            .checked_add(proof.inserted_scalar_delta)?;
        let expected_raw_scalars = state
            .validation_certificate
            .raw_text_scalars
            .checked_add(u64::from(proof.inserted_scalar_delta))?;
        if !state
            .document
            .root()
            .shares_storage_with(&proof.base_document_root)
            || !self.document.shares_root_storage_with(document)
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || self.validation_seal.stats.node_count != derivations.document_node_count
            || self.validation_seal.stats.node_count > resource_limits.max_document_nodes
            || self.validation_seal.stats.max_depth > resource_limits.max_document_depth
            || !Arc::ptr_eq(&proof.candidate_render_identity, &self.render_identity_seal)
            || !Arc::ptr_eq(&derivations.identity_seal, &self.position_identity_seal)
            || derivations.rendered_text != self.render.rendered_text
            || derivations.rendered_scalars != self.render.rendered_scalars
            || derivations.rendered_scalars != expected_rendered_scalars
            || self.raw_text_scalars != expected_raw_scalars
            || state.document.root().child_count() != document.root().child_count()
            || proof.target_top_level_index >= document.root().child_count()
            || !state
                .render_blocks
                .matches_identity(&state.document, schema_fingerprint)
        {
            return None;
        }
        Some(state.render_blocks.transition_localized_insert(
            &state.document,
            document,
            schema,
            proof.target_top_level_index,
            proof.inserted_scalar_delta,
            resource_limits,
        ))
    }

    #[cfg(test)]
    pub(super) fn history_render_tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "missing",
            "baseDocument",
            "targetIndex",
            "scalarDelta",
            "renderIdentity",
        ]
    }

    #[cfg(test)]
    pub(super) fn tamper_history_render_for_test(&mut self, case: &str) {
        if case == "missing" {
            self.history_render = None;
            return;
        }
        let proof = self
            .history_render
            .as_mut()
            .expect("history render tamper fixture retains its proof");
        match case {
            "baseDocument" => proof.base_document_root = self.document.root().clone(),
            "targetIndex" => {
                proof.target_top_level_index = proof.target_top_level_index.saturating_add(1)
            }
            "scalarDelta" => {
                proof.inserted_scalar_delta = proof.inserted_scalar_delta.saturating_add(1)
            }
            "renderIdentity" => proof.candidate_render_identity = Arc::new(()),
            _ => panic!("unknown history render evidence tamper case {case}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finalize_deferred(
        self,
        _authority: &super::compiler::CandidateValidationAuthority,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        validation: DocumentValidationReport,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
    ) -> Option<PreparedCandidateValidation> {
        if !self.document.shares_root_storage_with(document)
            || self.validation_seal != validation
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || !self.canonical_schema.ptr_eq(canonical_schema)
            || !canonical_artifact.matches_exact_source_document(document)
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !canonical_artifact.schema_context().ptr_eq(canonical_schema)
            || validation.stats.node_count == 0
            || validation.stats.node_count != self.document_node_count
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || !Arc::ptr_eq(&self.position.identity, &self.position_identity_seal)
            || !Arc::ptr_eq(&self.render.identity, &self.render_identity_seal)
            || self.position.position_map.total_scalars() != self.position.total_scalars
            || self.position.position_map.block_count() != self.position.block_count
            || self.render.rendered_scalars != self.position.total_scalars
            || canonical_artifact.text_scalar_len() != self.raw_text_scalars
            || canonical_artifact.text_utf8_bytes() != self.raw_text_utf8_bytes
            || self.document_text_bytes != self.raw_text_utf8_bytes
            || max_length.is_some_and(|limit| self.raw_text_scalars > u64::from(limit))
        {
            return None;
        }
        let derivations = CompiledDocumentDerivations {
            identity_seal: self.position.identity,
            position_map: self.position.position_map,
            rendered_text: self.render.rendered_text,
            rendered_scalars: self.render.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
        };
        Some(PreparedCandidateValidation {
            document: self.document,
            canonical_artifact: canonical_artifact.clone(),
            validation,
            resource_limits: self.resource_limits,
            editing_limits: self.editing_limits,
            max_length: self.max_length,
            schema_fingerprint: self.schema_fingerprint,
            derivations,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn derivations_for_prepared_history(
        &self,
        document: &Document,
        validation: DocumentValidationReport,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
        raw_text_scalars: u64,
        raw_text_utf8_bytes: usize,
    ) -> Option<CompiledDocumentDerivations> {
        if !self.document.shares_root_storage_with(document)
            || self.validation_seal != validation
            || self.resource_limits != *resource_limits
            || self.editing_limits != *editing_limits
            || self.max_length != max_length
            || self.schema_fingerprint.as_ref() != schema_fingerprint
            || !self.canonical_schema.ptr_eq(canonical_schema)
            || validation.stats.node_count == 0
            || validation.stats.node_count != self.document_node_count
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || !Arc::ptr_eq(&self.position.identity, &self.position_identity_seal)
            || !Arc::ptr_eq(&self.render.identity, &self.render_identity_seal)
            || self.position.position_map.total_scalars() != self.position.total_scalars
            || self.position.position_map.block_count() != self.position.block_count
            || self.render.rendered_scalars != self.position.total_scalars
            || self.raw_text_scalars != raw_text_scalars
            || self.raw_text_utf8_bytes != raw_text_utf8_bytes
            || self.document_text_bytes != raw_text_utf8_bytes
            || max_length.is_some_and(|limit| raw_text_scalars > u64::from(limit))
        {
            return None;
        }
        Some(CompiledDocumentDerivations {
            identity_seal: Arc::clone(&self.position.identity),
            position_map: self.position.position_map.clone(),
            rendered_text: self.render.rendered_text.clone(),
            rendered_scalars: self.render.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
        })
    }

    #[cfg(test)]
    pub(super) fn tamper_position_for_test(&mut self) {
        self.position.total_scalars = self.position.total_scalars.saturating_add(1);
    }

    #[cfg(test)]
    pub(super) fn tamper_render_for_test(&mut self) {
        self.render.rendered_scalars = self.render.rendered_scalars.saturating_add(1);
    }

    #[cfg(test)]
    pub(super) fn tamper_same_summary_for_test(&mut self, case: &str) {
        match case {
            "position" => {
                self.position = PreparedPositionEvidence {
                    position_map: self.position.position_map.clone(),
                    total_scalars: self.position.total_scalars,
                    block_count: self.position.block_count,
                    identity: Arc::new(()),
                };
            }
            "render" => {
                let mut rendered_text = self.render.rendered_text.clone();
                let first = rendered_text
                    .chars()
                    .next()
                    .expect("render replacement fixture is nonempty");
                assert!(
                    first.is_ascii(),
                    "render replacement fixture starts in ASCII"
                );
                rendered_text
                    .replace_range(0..first.len_utf8(), if first == 'z' { "y" } else { "z" });
                self.render = PreparedRenderEvidence {
                    rendered_text,
                    rendered_scalars: self.render.rendered_scalars,
                    identity: Arc::new(()),
                };
            }
            _ => panic!("unknown same-summary evidence tamper case {case}"),
        }
    }
}

impl PreparedCandidateValidation {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        _authority: &super::compiler::CandidateValidationAuthority,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        validation: DocumentValidationReport,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        position_map: PositionMap,
    ) -> Option<Self> {
        if crate::schema::schema_fingerprint(schema) != schema_fingerprint
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !canonical_artifact.matches_exact_source_document(document)
            || validation.stats.node_count > resource_limits.max_document_nodes
            || validation.stats.max_depth > resource_limits.max_document_depth
            || max_length
                .is_some_and(|limit| canonical_artifact.text_scalar_len() > u64::from(limit))
        {
            return None;
        }
        crate::transform::validate_canonical_marks(document, schema).ok()?;
        record_preview_position_map_derivation();
        record_preview_rendered_text_derivation();
        let rendered_text = crate::render::rendered_text(document, schema);
        let rendered_scalars = u32::try_from(rendered_text.chars().count()).ok()?;
        if rendered_scalars != position_map.total_scalars() {
            return None;
        }
        let derivations = CompiledDocumentDerivations {
            identity_seal: Arc::new(()),
            position_map,
            rendered_text,
            rendered_scalars,
            document_text_bytes: canonical_artifact.text_utf8_bytes(),
            document_node_count: validation.stats.node_count,
        };
        Some(Self {
            document: document.clone(),
            canonical_artifact: canonical_artifact.clone(),
            validation,
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: schema_fingerprint.into(),
            derivations,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admits_context(
        &self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
    ) -> bool {
        self.document.shares_root_storage_with(document)
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && self
                .canonical_artifact
                .matches_exact_source_document(document)
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self
                .canonical_artifact
                .schema_context()
                .ptr_eq(canonical_schema)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn compiled_derivations(
        &self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
    ) -> Option<CompiledDocumentDerivations> {
        self.admits_context(
            document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            canonical_schema,
        )
        .then(|| self.derivations.clone())
    }

    #[cfg(test)]
    pub(crate) fn replace_canonical_artifact_for_test(
        &mut self,
        canonical_artifact: CanonicalArtifact,
    ) {
        self.canonical_artifact = canonical_artifact;
    }
}

#[allow(dead_code)] // E1 evidence API is consumed by E2 and admission-oracle tests.
impl DocumentValidationCertificate {
    #[allow(clippy::too_many_arguments)]
    fn from_report(
        validation: DocumentValidationReport,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Self {
        #[cfg(test)]
        super::observability::record_validation_certificate_construction();
        Self {
            stats: validation.stats,
            metrics: validation.metrics,
            resource_limits: resource_limits.clone(),
            schema_fingerprint: Arc::from(schema_fingerprint),
            canonical_artifact: canonical_artifact.clone(),
            canonical_fingerprint: [0; 32],
            canonical_serialized_len: 0,
            canonical_fingerprint_materialized: false,
            raw_text_scalars: canonical_artifact.text_scalar_len(),
            raw_text_utf8_bytes: canonical_artifact.text_utf8_bytes(),
            document_revision,
            state_revision,
            yrs_state_epoch,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mint(
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        if crate::schema::schema_fingerprint(schema) != schema_fingerprint
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !canonical_artifact.matches_document(document)
        {
            return None;
        }
        let validation =
            DocumentValidator::validate_report(document, schema, resource_limits).ok()?;
        crate::transform::validate_canonical_marks(document, schema).ok()?;
        let mut certificate = Self::from_report(
            validation,
            canonical_artifact,
            resource_limits,
            schema_fingerprint,
            document_revision,
            state_revision,
            yrs_state_epoch,
        );
        certificate.canonical_fingerprint = canonical_artifact.sha256();
        certificate.canonical_serialized_len = canonical_artifact.serialized_len();
        certificate.canonical_fingerprint_materialized = true;
        Some(certificate)
    }

    pub(crate) fn stats(&self) -> DocumentStats {
        self.stats
    }

    pub(crate) fn document_revision(&self) -> u64 {
        self.document_revision
    }

    pub(crate) fn state_revision(&self) -> u64 {
        self.state_revision
    }

    pub(crate) fn yrs_state_epoch(&self) -> u64 {
        self.yrs_state_epoch
    }

    pub(crate) fn canonical_fingerprint(&self) -> [u8; 32] {
        if self.canonical_fingerprint_materialized {
            self.canonical_fingerprint
        } else {
            self.canonical_artifact.sha256()
        }
    }

    // Keep every sealed identity dimension explicit so exact certificate matching stays auditable.
    #[allow(clippy::too_many_arguments)]
    fn matches_materialized_identity(
        &self,
        canonical_artifact: &CanonicalArtifact,
        canonical_fingerprint: [u8; 32],
        canonical_serialized_len: usize,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> bool {
        self.resource_limits == *resource_limits
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.document_revision == document_revision
            && self.state_revision == state_revision
            && self.yrs_state_epoch == yrs_state_epoch
            && self.raw_text_scalars == canonical_artifact.text_scalar_len()
            && self.raw_text_utf8_bytes == canonical_artifact.text_utf8_bytes()
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && if self.canonical_fingerprint_materialized {
                self.canonical_fingerprint == canonical_fingerprint
                    && self.canonical_serialized_len == canonical_serialized_len
            } else {
                canonical_artifact.sha256() == canonical_fingerprint
                    && canonical_artifact.serialized_len() == canonical_serialized_len
            }
    }

    #[cfg(test)]
    pub(crate) fn canonical_fingerprint_materialized_for_test(&self) -> bool {
        self.canonical_fingerprint_materialized
    }

    fn materialize_canonical_artifact(&mut self) {
        if !self.canonical_fingerprint_materialized {
            self.canonical_fingerprint = self.canonical_artifact.sha256();
            self.canonical_serialized_len = self.canonical_artifact.serialized_len();
            self.canonical_fingerprint_materialized = true;
        }
    }

    fn reseal_state_revision(&mut self, state_revision: u64) {
        self.state_revision = state_revision;
    }

    fn promote_existing_insert(
        &self,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        admission: &LocalizedInsertAdmission,
    ) -> Option<Self> {
        let canonical_fingerprint = canonical_artifact.sha256();
        if canonical_artifact.schema_fingerprint() != self.schema_fingerprint.as_ref()
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || canonical_artifact.serialized_len() != admission.next_canonical_serialized_len
            || canonical_artifact.text_scalar_len() != admission.next_raw_text_scalars
            || canonical_artifact.text_utf8_bytes() != admission.next_raw_text_utf8_bytes
            || derivations.document_node_count != self.stats.node_count
            || derivations.document_text_bytes != admission.next_raw_text_utf8_bytes
            || derivations.rendered_scalars != admission.next_rendered_scalars
        {
            return None;
        }
        Some(Self {
            stats: self.stats,
            metrics: self.metrics,
            resource_limits: self.resource_limits.clone(),
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
            canonical_artifact: canonical_artifact.clone(),
            canonical_fingerprint,
            canonical_serialized_len: canonical_artifact.serialized_len(),
            canonical_fingerprint_materialized: true,
            raw_text_scalars: canonical_artifact.text_scalar_len(),
            raw_text_utf8_bytes: canonical_artifact.text_utf8_bytes(),
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            yrs_state_epoch: self.yrs_state_epoch,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn matches(
        &self,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> bool {
        self.resource_limits == *resource_limits
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && (self.canonical_artifact.ptr_eq(canonical_artifact)
                || (self.canonical_fingerprint() == canonical_artifact.sha256()
                    && self.canonical_serialized_len == canonical_artifact.serialized_len()))
            && self.raw_text_scalars == canonical_artifact.text_scalar_len()
            && self.raw_text_utf8_bytes == canonical_artifact.text_utf8_bytes()
            && self.document_revision == document_revision
            && self.state_revision == state_revision
            && self.yrs_state_epoch == yrs_state_epoch
    }
}

#[derive(Debug)]
struct LocalizedRenderTransitionProof {
    base_document_root: crate::model::Node,
    preview_root: crate::model::Node,
    base_render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    resource_limits: ResourceLimits,
    schema_fingerprint: Arc<str>,
    max_operations_per_transaction: usize,
    max_undo_groups: usize,
    max_derived_output_bytes: usize,
    max_undo_retained_units: u64,
    max_length: Option<u32>,
    derivation_identity_seal: Arc<()>,
    target_top_level_index: usize,
    inserted_scalar_delta: u32,
    top_level_cardinality: usize,
    operation_kind: LocalizedRenderOperationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalizedRenderOperationKind {
    ExistingTextInsert,
    #[cfg(test)]
    Unsupported,
}

#[derive(Debug)]
pub(crate) struct PreparedDerivedEvidence {
    request_id: u64,
    base_document_root: crate::model::Node,
    preview_root: crate::model::Node,
    base_validation: DocumentValidationCertificate,
    base_render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    base_lookup_seal: Arc<super::mutation::MutationLookupSeed>,
    max_operations_per_transaction: usize,
    max_undo_groups: usize,
    max_derived_output_bytes: usize,
    max_undo_retained_units: u64,
    max_length: Option<u32>,
    derivation_identity_seal: Arc<()>,
    preview_rendered_scalars: u32,
    preview_document_text_bytes: usize,
    preview_document_node_count: usize,
    preview_position_total_scalars: u32,
    preview_position_block_count: usize,
    canonical_fingerprint: [u8; 32],
    canonical_serialized_len: usize,
    validation_certificate: DocumentValidationCertificate,
    localized_text_index: Option<LocalizedTextLeafIndex>,
    localized_render_transition_proof: Option<LocalizedRenderTransitionProof>,
}

#[derive(Debug)]
pub(crate) struct FinalizedDerivedEvidence {
    validation_certificate: DocumentValidationCertificate,
    localized_text_index: Option<LocalizedTextLeafIndex>,
}

impl PreparedCandidateValidation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finalize(
        self,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        canonical_schema: &super::canonical::CanonicalSchemaContext,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<FinalizedDerivedEvidence> {
        if !self.admits_context(
            document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            canonical_schema,
        ) || !Arc::ptr_eq(&derivations.identity_seal, &self.derivations.identity_seal)
            || derivations.position_map.total_scalars()
                != self.derivations.position_map.total_scalars()
            || derivations.position_map.block_count() != self.derivations.position_map.block_count()
            || derivations.rendered_text != self.derivations.rendered_text
            || derivations.rendered_scalars != self.derivations.rendered_scalars
            || derivations.document_text_bytes != self.derivations.document_text_bytes
            || derivations.document_node_count != self.validation.stats.node_count
            || canonical_artifact.text_utf8_bytes() != derivations.document_text_bytes
            || u64::from(derivations.rendered_scalars) < canonical_artifact.text_scalar_len()
        {
            return None;
        }
        let validation_certificate = DocumentValidationCertificate {
            stats: self.validation.stats,
            metrics: self.validation.metrics,
            resource_limits: self.resource_limits,
            schema_fingerprint: self.schema_fingerprint,
            canonical_artifact: canonical_artifact.clone(),
            canonical_fingerprint: canonical_artifact.sha256(),
            canonical_serialized_len: canonical_artifact.serialized_len(),
            canonical_fingerprint_materialized: true,
            raw_text_scalars: canonical_artifact.text_scalar_len(),
            raw_text_utf8_bytes: canonical_artifact.text_utf8_bytes(),
            document_revision,
            state_revision,
            yrs_state_epoch,
        };
        let localized_text_index = LocalizedTextLeafIndex::build(
            document,
            &derivations.position_map,
            &derivations.rendered_text,
            &validation_certificate,
            resource_limits,
            schema,
        );
        Some(FinalizedDerivedEvidence {
            validation_certificate,
            localized_text_index,
        })
    }
}

impl PreparedDerivedEvidence {
    #[cfg(test)]
    pub(crate) fn localized_render_tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "missing",
            "baseDocument",
            "previewDocument",
            "renderSeal",
            "resourceLimits",
            "schemaFingerprint",
            "maxOperations",
            "maxUndoGroups",
            "maxDerivedOutput",
            "maxUndo",
            "maxLength",
            "derivationIdentity",
            "targetIndex",
            "scalarDelta",
            "cardinality",
            "operationKind",
        ]
    }

    #[cfg(test)]
    pub(crate) fn tamper_localized_render_for_test(&mut self, case: &str) {
        if case == "missing" {
            self.localized_render_transition_proof = None;
            return;
        }
        let proof = self
            .localized_render_transition_proof
            .as_mut()
            .expect("localized render tamper fixture retains its proof");
        match case {
            "baseDocument" => proof.base_document_root = proof.preview_root.clone(),
            "previewDocument" => proof.preview_root = proof.base_document_root.clone(),
            "renderSeal" => proof.base_render_seal = Arc::new((*proof.base_render_seal).clone()),
            "resourceLimits" => {
                proof.resource_limits.max_input_bytes =
                    proof.resource_limits.max_input_bytes.saturating_add(1)
            }
            "schemaFingerprint" => proof.schema_fingerprint = Arc::<str>::from("tampered"),
            "maxOperations" => {
                proof.max_operations_per_transaction =
                    proof.max_operations_per_transaction.saturating_add(1)
            }
            "maxUndoGroups" => proof.max_undo_groups = proof.max_undo_groups.saturating_add(1),
            "maxDerivedOutput" => {
                proof.max_derived_output_bytes = proof.max_derived_output_bytes.saturating_add(1)
            }
            "maxUndo" => {
                proof.max_undo_retained_units = proof.max_undo_retained_units.saturating_add(1)
            }
            "maxLength" => proof.max_length = Some(proof.max_length.unwrap_or(1).saturating_add(1)),
            "derivationIdentity" => proof.derivation_identity_seal = Arc::new(()),
            "targetIndex" => {
                proof.target_top_level_index = proof.target_top_level_index.saturating_add(1)
            }
            "scalarDelta" => {
                proof.inserted_scalar_delta = proof.inserted_scalar_delta.saturating_add(1)
            }
            "cardinality" => {
                proof.top_level_cardinality = proof.top_level_cardinality.saturating_add(1)
            }
            "operationKind" => proof.operation_kind = LocalizedRenderOperationKind::Unsupported,
            _ => panic!("unknown localized render proof tamper case {case}"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_localized_render_transition(
        &self,
        state: &DerivedStateCache,
        preview: &Document,
        derivations: &CompiledDocumentDerivations,
        affected_top_level_blocks: &[usize],
        schema: &Schema,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
    ) -> Option<
        Result<
            crate::render::incremental::CachedRenderTransition,
            crate::render::incremental::CachedRenderError,
        >,
    > {
        let proof = self.localized_render_transition_proof.as_ref()?;
        let base_raw_scalars = state.validation_certificate.raw_text_scalars;
        let expected_raw_scalars =
            base_raw_scalars.checked_add(u64::from(proof.inserted_scalar_delta))?;
        let expected_rendered_scalars = state
            .rendered_scalars
            .checked_add(proof.inserted_scalar_delta)?;
        let expected_affected_start = proof.target_top_level_index.saturating_sub(1);
        let expected_affected_len = proof
            .top_level_cardinality
            .checked_sub(expected_affected_start)?;
        let validation_stats = self.validation_certificate.stats;
        let affected_range_matches = affected_top_level_blocks.len() == expected_affected_len
            && affected_top_level_blocks
                .iter()
                .copied()
                .eq(expected_affected_start..proof.top_level_cardinality);
        if !state
            .document
            .root()
            .shares_storage_with(&proof.base_document_root)
            || !preview.root().shares_storage_with(&proof.preview_root)
            || !Arc::ptr_eq(&state.render_blocks, &proof.base_render_seal)
            || self.base_validation != state.validation_certificate
            || proof.resource_limits != *resource_limits
            || self.validation_certificate.resource_limits != proof.resource_limits
            || self.validation_certificate.schema_fingerprint.as_ref()
                != proof.schema_fingerprint.as_ref()
            || proof.schema_fingerprint.as_ref() != schema_fingerprint
            || proof.max_operations_per_transaction != editing_limits.max_operations_per_transaction
            || proof.max_undo_groups != editing_limits.max_undo_groups
            || proof.max_derived_output_bytes != editing_limits.max_derived_output_bytes
            || proof.max_undo_retained_units != editing_limits.max_undo_retained_units
            || proof.max_length != max_length
            || !Arc::ptr_eq(&proof.derivation_identity_seal, &derivations.identity_seal)
            || proof.operation_kind != LocalizedRenderOperationKind::ExistingTextInsert
            || proof.inserted_scalar_delta == 0
            || state.document.root().child_count() != proof.top_level_cardinality
            || preview.root().child_count() != proof.top_level_cardinality
            || !affected_range_matches
            || proof.target_top_level_index >= proof.top_level_cardinality
            || derivations.rendered_scalars != expected_rendered_scalars
            || derivations.document_node_count != validation_stats.node_count
            || validation_stats.node_count > proof.resource_limits.max_document_nodes
            || validation_stats.max_depth > proof.resource_limits.max_document_depth
            || proof.top_level_cardinality > validation_stats.node_count
            || self.validation_certificate.raw_text_scalars != expected_raw_scalars
            || proof
                .max_length
                .is_some_and(|limit| expected_raw_scalars > u64::from(limit))
            || !state
                .render_blocks
                .matches_identity(&state.document, schema_fingerprint)
        {
            return None;
        }
        Some(state.render_blocks.transition_localized_insert(
            &state.document,
            preview,
            schema,
            proof.target_top_level_index,
            proof.inserted_scalar_delta,
            resource_limits,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finalize(
        mut self,
        authority: &dyn DerivedStateAuthority,
        preview: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        next_render_blocks: &crate::render::incremental::CachedRenderBlocks,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        next_document_revision: u64,
        next_state_revision: u64,
        next_yrs_state_epoch: u64,
    ) -> Option<FinalizedDerivedEvidence> {
        let state = authority.installed();
        let authority_lookup_seed = authority.lookup_seed(self.request_id).ok()?;
        if self
            .localized_text_index
            .as_ref()
            .is_some_and(|index| !index.matches(&self.validation_certificate))
            || !state
                .document
                .root()
                .shares_storage_with(&self.base_document_root)
            || !preview.root().shares_storage_with(&self.preview_root)
            || state.validation_certificate != self.base_validation
            || !Arc::ptr_eq(&state.render_blocks, &self.base_render_seal)
            || !Arc::ptr_eq(authority_lookup_seed, &self.base_lookup_seal)
            || state.validation_certificate.resource_limits != *resource_limits
            || state.schema_fingerprint != schema_fingerprint
            || editing_limits.max_operations_per_transaction != self.max_operations_per_transaction
            || editing_limits.max_undo_groups != self.max_undo_groups
            || editing_limits.max_derived_output_bytes != self.max_derived_output_bytes
            || editing_limits.max_undo_retained_units != self.max_undo_retained_units
            || max_length != self.max_length
            || !Arc::ptr_eq(&derivations.identity_seal, &self.derivation_identity_seal)
            || canonical_artifact.sha256() != self.canonical_fingerprint
            || canonical_artifact.serialized_len() != self.canonical_serialized_len
            || canonical_artifact.schema_fingerprint() != schema_fingerprint
            || !self
                .validation_certificate
                .canonical_artifact
                .ptr_eq(canonical_artifact)
            || !self
                .validation_certificate
                .canonical_fingerprint_materialized
            || self.validation_certificate.canonical_fingerprint != canonical_artifact.sha256()
            || self.validation_certificate.canonical_serialized_len
                != canonical_artifact.serialized_len()
            || self.validation_certificate.raw_text_scalars != canonical_artifact.text_scalar_len()
            || self.validation_certificate.raw_text_utf8_bytes
                != canonical_artifact.text_utf8_bytes()
            || derivations.rendered_scalars != self.preview_rendered_scalars
            || derivations.document_text_bytes != self.preview_document_text_bytes
            || derivations.document_node_count != self.preview_document_node_count
            || derivations.position_map.total_scalars() != self.preview_position_total_scalars
            || derivations.position_map.block_count() != self.preview_position_block_count
            || !next_render_blocks.matches_identity(preview, schema_fingerprint)
            || next_document_revision != self.base_validation.document_revision.checked_add(1)?
            || next_state_revision != self.base_validation.state_revision.checked_add(1)?
            || next_yrs_state_epoch != self.base_validation.yrs_state_epoch.checked_add(1)?
        {
            return None;
        }
        self.validation_certificate.document_revision = next_document_revision;
        self.validation_certificate.state_revision = next_state_revision;
        self.validation_certificate.yrs_state_epoch = next_yrs_state_epoch;
        if let Some(index) = self.localized_text_index.as_mut() {
            index.document_revision = next_document_revision;
            index.canonical_fingerprint = canonical_artifact.sha256();
            index.schema_fingerprint = Arc::clone(&self.validation_certificate.schema_fingerprint);
        }
        Some(FinalizedDerivedEvidence {
            validation_certificate: self.validation_certificate,
            localized_text_index: self.localized_text_index,
        })
    }

    #[cfg(test)]
    pub(crate) fn tamper_cases_for_test() -> &'static [&'static str] {
        &[
            "baseDocument",
            "previewDocument",
            "baseValidation",
            "renderSeal",
            "lookupSeal",
            "maxOperations",
            "maxUndoGroups",
            "maxDerivedOutput",
            "maxUndo",
            "maxLength",
            "derivationIdentity",
            "renderedScalars",
            "documentBytes",
            "documentNodes",
            "positionScalars",
            "positionBlocks",
            "canonicalFingerprint",
            "canonicalLength",
            "promotedValidation",
            "promotedIndex",
        ]
    }

    #[cfg(test)]
    pub(crate) fn tamper_for_test(&mut self, case: &str) {
        match case {
            "baseDocument" => self.base_document_root = self.preview_root.clone(),
            "previewDocument" => self.preview_root = self.base_document_root.clone(),
            "baseValidation" => {
                self.base_validation.state_revision =
                    self.base_validation.state_revision.saturating_add(1)
            }
            "renderSeal" => self.base_render_seal = Arc::new((*self.base_render_seal).clone()),
            "lookupSeal" => self.base_lookup_seal = Arc::new((*self.base_lookup_seal).clone()),
            "maxOperations" => {
                self.max_operations_per_transaction =
                    self.max_operations_per_transaction.saturating_add(1)
            }
            "maxUndoGroups" => self.max_undo_groups = self.max_undo_groups.saturating_add(1),
            "maxDerivedOutput" => {
                self.max_derived_output_bytes = self.max_derived_output_bytes.saturating_add(1)
            }
            "maxUndo" => {
                self.max_undo_retained_units = self.max_undo_retained_units.saturating_add(1)
            }
            "maxLength" => self.max_length = Some(self.max_length.unwrap_or(1).saturating_add(1)),
            "derivationIdentity" => self.derivation_identity_seal = Arc::new(()),
            "renderedScalars" => {
                self.preview_rendered_scalars = self.preview_rendered_scalars.saturating_add(1)
            }
            "documentBytes" => {
                self.preview_document_text_bytes =
                    self.preview_document_text_bytes.saturating_add(1)
            }
            "documentNodes" => {
                self.preview_document_node_count =
                    self.preview_document_node_count.saturating_add(1)
            }
            "positionScalars" => {
                self.preview_position_total_scalars =
                    self.preview_position_total_scalars.saturating_add(1)
            }
            "positionBlocks" => {
                self.preview_position_block_count =
                    self.preview_position_block_count.saturating_add(1)
            }
            "canonicalFingerprint" => self.canonical_fingerprint[0] ^= 1,
            "canonicalLength" => {
                self.canonical_serialized_len = self.canonical_serialized_len.saturating_add(1)
            }
            "promotedValidation" => self.validation_certificate.canonical_fingerprint[0] ^= 1,
            "promotedIndex" => {
                if let Some(index) = self.localized_text_index.as_mut() {
                    index.canonical_fingerprint[0] ^= 1;
                }
            }
            _ => panic!("unknown prepared evidence tamper case {case}"),
        }
    }
}

/// A rendered text leaf identity and its exact document/scalar/UTF-16 ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalizedTextLeafCertificate {
    block_index: usize,
    child_ordinal: u32,
    doc_start: u32,
    doc_end: u32,
    scalar_start: u32,
    scalar_end: u32,
    utf16_start: u32,
    utf16_end: u32,
    text_sha256: [u8; 32],
    text_scalars: u32,
    text_utf16: u32,
    text_utf8_bytes: usize,
    marks_sha256: [u8; 32],
}

#[allow(dead_code)] // E1 evidence API is consumed by E2 and admission-oracle tests.
impl LocalizedTextLeafCertificate {
    pub(crate) fn doc_start(&self) -> u32 {
        self.doc_start
    }

    pub(crate) fn doc_end(&self) -> u32 {
        self.doc_end
    }

    fn resolve<'a>(
        &self,
        document: &'a Document,
        position_map: &PositionMap,
    ) -> Option<&'a crate::model::Node> {
        let block = position_map.block(self.block_index)?;
        document
            .node_at(&block.node_path)?
            .content()?
            .child(usize::try_from(self.child_ordinal).ok()?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LocalizedTextLeafIndex {
    leaves: Vec<LocalizedTextLeafCertificate>,
    schema_fingerprint: Arc<str>,
    canonical_artifact: CanonicalArtifact,
    canonical_fingerprint: [u8; 32],
    canonical_fingerprint_materialized: bool,
    document_revision: u64,
    retained_bytes: usize,
}

#[allow(dead_code)] // E1 evidence API is consumed by E2 and admission-oracle tests.
impl LocalizedTextLeafIndex {
    fn build(
        document: &Document,
        position_map: &PositionMap,
        rendered_text: &str,
        validation: &DocumentValidationCertificate,
        resource_limits: &ResourceLimits,
        schema: &Schema,
    ) -> Option<Self> {
        #[cfg(test)]
        LOCALIZED_INDEX_BUILD_COUNT.set(LOCALIZED_INDEX_BUILD_COUNT.get().saturating_add(1));
        #[cfg(test)]
        if FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE.get() {
            return None;
        }
        if !position_map.has_effective_stored_bounds() {
            return None;
        }
        let cache_budget = resource_limits.max_input_bytes;
        #[cfg(test)]
        let cache_budget = FORCE_LOCALIZED_INDEX_BUDGET.get().unwrap_or(cache_budget);
        let path_bytes = validation
            .stats
            .max_depth
            .checked_mul(std::mem::size_of::<u32>())?;
        let leaf_budget = cache_budget.checked_sub(path_bytes)?;
        let leaf_size = std::mem::size_of::<LocalizedTextLeafCertificate>();
        let max_leaf_capacity = leaf_budget.checked_div(leaf_size)?;
        let initial_leaf_capacity = position_map
            .block_count()
            .min(resource_limits.max_document_nodes)
            .min(max_leaf_capacity);
        let mut leaves = Vec::new();
        #[cfg(test)]
        if forced_localized_index_allocation_stage(
            LocalizedIndexAllocationStage::InitialLeafCapacity,
        ) {
            return None;
        }
        leaves.try_reserve_exact(initial_leaf_capacity).ok()?;
        let initial_leaf_capacity_bytes = leaves.capacity().checked_mul(leaf_size)?;
        if initial_leaf_capacity_bytes > leaf_budget {
            return None;
        }
        let mut rendered_cursor = RenderedCursor::new(rendered_text);
        let mut retained_bytes = initial_leaf_capacity_bytes;
        let mut path = Vec::new();
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::TraversalPath) {
            return None;
        }
        path.try_reserve_exact(validation.stats.max_depth).ok()?;
        let path_capacity_bytes = path.capacity().checked_mul(std::mem::size_of::<u32>())?;
        if path_capacity_bytes.checked_add(retained_bytes)? > cache_budget {
            return None;
        }
        let mut next_block_index = 0usize;
        collect_localized_index_streamed(
            document.root(),
            &mut path,
            position_map,
            schema,
            0,
            &mut next_block_index,
            &mut leaves,
            &mut rendered_cursor,
            &mut retained_bytes,
            cache_budget,
            path_capacity_bytes,
        )?;
        if next_block_index != position_map.block_count() {
            return None;
        }
        Some(Self {
            leaves,
            schema_fingerprint: Arc::clone(&validation.schema_fingerprint),
            canonical_artifact: validation.canonical_artifact.clone(),
            canonical_fingerprint: validation.canonical_fingerprint,
            canonical_fingerprint_materialized: validation.canonical_fingerprint_materialized,
            document_revision: validation.document_revision,
            retained_bytes,
        })
    }

    pub(crate) fn leaves(&self) -> &[LocalizedTextLeafCertificate] {
        &self.leaves
    }

    fn strict_inside(&self, document_position: u32) -> Option<&LocalizedTextLeafCertificate> {
        let mut low = 0usize;
        let mut high = self.leaves.len();
        while low < high {
            #[cfg(test)]
            LOCALIZED_INDEX_LOOKUP_COMPARISONS
                .set(LOCALIZED_INDEX_LOOKUP_COMPARISONS.get().saturating_add(1));
            let middle = low + (high - low) / 2;
            if self.leaves[middle].doc_end <= document_position {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        self.leaves
            .get(low)
            .filter(|leaf| leaf.doc_start < document_position && document_position < leaf.doc_end)
    }

    fn matches(&self, validation: &DocumentValidationCertificate) -> bool {
        self.schema_fingerprint == validation.schema_fingerprint
            && if self.canonical_fingerprint_materialized
                && validation.canonical_fingerprint_materialized
            {
                self.canonical_fingerprint == validation.canonical_fingerprint
            } else {
                !self.canonical_fingerprint_materialized
                    && !validation.canonical_fingerprint_materialized
                    && self
                        .canonical_artifact
                        .ptr_eq(&validation.canonical_artifact)
            }
            && self.document_revision == validation.document_revision
    }

    // Keep the certificate and every sealed identity dimension explicit at this proof boundary.
    #[allow(clippy::too_many_arguments)]
    fn matches_materialized_identity(
        &self,
        validation: &DocumentValidationCertificate,
        canonical_artifact: &CanonicalArtifact,
        canonical_fingerprint: [u8; 32],
        canonical_serialized_len: usize,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> bool {
        self.schema_fingerprint == validation.schema_fingerprint
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.document_revision == document_revision
            && self.document_revision == validation.document_revision
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && validation.matches_materialized_identity(
                canonical_artifact,
                canonical_fingerprint,
                canonical_serialized_len,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
            && if self.canonical_fingerprint_materialized {
                self.canonical_fingerprint == canonical_fingerprint
            } else {
                canonical_artifact.sha256() == canonical_fingerprint
            }
    }

    fn materialize_canonical_fingerprint(&mut self, validation: &DocumentValidationCertificate) {
        self.canonical_fingerprint = validation.canonical_fingerprint;
        self.canonical_fingerprint_materialized = true;
    }

    #[cfg(test)]
    pub(crate) fn canonical_fingerprint_materialized_for_test(&self) -> bool {
        self.canonical_fingerprint_materialized
    }

    fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    #[cfg(test)]
    pub(crate) fn promotion_transient_budget_for_test(&self) -> Option<usize> {
        let mut promoted = Vec::<LocalizedTextLeafCertificate>::new();
        promoted.try_reserve_exact(self.leaves.len()).ok()?;
        let promoted_bytes = promoted
            .capacity()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        self.retained_bytes.checked_add(promoted_bytes)
    }

    fn try_clone(&self, cache_budget: usize) -> Option<Self> {
        if self.retained_bytes > cache_budget {
            return None;
        }
        let required_bytes = self
            .leaves
            .len()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        let available_bytes = cache_budget.checked_sub(self.retained_bytes)?;
        if required_bytes > available_bytes {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(
            LocalizedIndexAllocationStage::InitialLeafCapacity,
        ) {
            return None;
        }
        let mut leaves = Vec::new();
        leaves.try_reserve_exact(self.leaves.len()).ok()?;
        let retained_bytes = leaves
            .capacity()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        if retained_bytes > available_bytes {
            return None;
        }
        leaves.extend_from_slice(&self.leaves);
        Some(Self {
            leaves,
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
            canonical_artifact: self.canonical_artifact.clone(),
            canonical_fingerprint: self.canonical_fingerprint,
            canonical_fingerprint_materialized: self.canonical_fingerprint_materialized,
            document_revision: self.document_revision,
            retained_bytes,
        })
    }

    fn promote_existing_insert(
        &self,
        validation: &DocumentValidationCertificate,
        admission: &LocalizedInsertAdmission,
        block_path: &[u32],
        preview: &Document,
        canonical_artifact: &CanonicalArtifact,
        cache_budget: usize,
    ) -> Option<Self> {
        #[cfg(test)]
        if FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE.get() {
            return None;
        }
        if !self.matches(validation) || self.retained_bytes > cache_budget {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::PromotionClone) {
            return None;
        }
        let required_bytes = self
            .leaves
            .len()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        let available_bytes = cache_budget.checked_sub(self.retained_bytes)?;
        if required_bytes > available_bytes {
            return None;
        }
        let mut leaves = Vec::new();
        leaves.try_reserve_exact(self.leaves.len()).ok()?;
        let retained_bytes = leaves
            .capacity()
            .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
        if retained_bytes > available_bytes {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::PromotionGrowth) {
            return None;
        }
        leaves.extend_from_slice(&self.leaves);
        let target = self.strict_inside_index(admission.inserted_document_position)?;
        if leaves.get(target)? != &admission.leaf {
            return None;
        }
        #[cfg(test)]
        if forced_localized_index_allocation_stage(LocalizedIndexAllocationStage::PromotionUpdate) {
            return None;
        }
        let block = preview.node_at(block_path)?;
        let next_leaf = block
            .content()?
            .child(usize::try_from(admission.leaf.child_ordinal).ok()?)?;
        let next_text = next_leaf.text_str()?;
        let inserted_scalars = admission.inserted_scalars;
        let inserted_utf16 = admission.inserted_utf16;
        let inserted_utf8 = admission.inserted_utf8_bytes;
        let target_leaf = leaves.get_mut(target)?;
        target_leaf.doc_end = target_leaf.doc_end.checked_add(inserted_scalars)?;
        target_leaf.scalar_end = target_leaf.scalar_end.checked_add(inserted_scalars)?;
        target_leaf.utf16_end = target_leaf.utf16_end.checked_add(inserted_utf16)?;
        target_leaf.text_scalars = target_leaf.text_scalars.checked_add(inserted_scalars)?;
        target_leaf.text_utf16 = target_leaf.text_utf16.checked_add(inserted_utf16)?;
        target_leaf.text_utf8_bytes = target_leaf.text_utf8_bytes.checked_add(inserted_utf8)?;
        target_leaf.text_sha256 = sha2::Sha256::digest(next_text.as_bytes()).into();
        target_leaf.marks_sha256 = canonical_marks_sha256(next_leaf.marks())?;
        for leaf in leaves.iter_mut().skip(target + 1) {
            leaf.doc_start = leaf.doc_start.checked_add(inserted_scalars)?;
            leaf.doc_end = leaf.doc_end.checked_add(inserted_scalars)?;
            leaf.scalar_start = leaf.scalar_start.checked_add(inserted_scalars)?;
            leaf.scalar_end = leaf.scalar_end.checked_add(inserted_scalars)?;
            leaf.utf16_start = leaf.utf16_start.checked_add(inserted_utf16)?;
            leaf.utf16_end = leaf.utf16_end.checked_add(inserted_utf16)?;
        }
        Some(Self {
            leaves,
            schema_fingerprint: Arc::clone(&validation.schema_fingerprint),
            canonical_artifact: canonical_artifact.clone(),
            canonical_fingerprint: canonical_artifact.sha256(),
            canonical_fingerprint_materialized: true,
            document_revision: validation.document_revision,
            retained_bytes,
        })
    }

    fn strict_inside_index(&self, document_position: u32) -> Option<usize> {
        let mut low = 0usize;
        let mut high = self.leaves.len();
        while low < high {
            let middle = low + (high - low) / 2;
            if self.leaves[middle].doc_end <= document_position {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        self.leaves.get(low).and_then(|leaf| {
            (leaf.doc_start < document_position && document_position < leaf.doc_end).then_some(low)
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_localized_index_streamed(
    node: &crate::model::Node,
    path: &mut Vec<u32>,
    position_map: &PositionMap,
    schema: &Schema,
    doc_offset: u32,
    next_block_index: &mut usize,
    leaves: &mut Vec<LocalizedTextLeafCertificate>,
    rendered_cursor: &mut RenderedCursor<'_>,
    retained_bytes: &mut usize,
    cache_budget: usize,
    path_capacity_bytes: usize,
) -> Option<()> {
    #[cfg(test)]
    LOCALIZED_INDEX_BUILD_VISITS.set(LOCALIZED_INDEX_BUILD_VISITS.get().saturating_add(1));
    if let Some(kind) = classify_position_block(node, schema) {
        let block = position_map.block(*next_block_index)?;
        let is_void = kind == PositionBlockKind::Void;
        let expected_doc_end = if is_void {
            doc_offset
        } else {
            doc_offset.checked_add(node.content()?.size())?
        };
        if block.is_void_block != is_void
            || block.node_path.len() != path.len()
            || block.doc_start != doc_offset
            || block.doc_end != expected_doc_end
        {
            return None;
        }
        let block_index = *next_block_index;
        *next_block_index = next_block_index.checked_add(1)?;
        if !is_void {
            collect_localized_text_leaves_streamed(
                position_map,
                node,
                block.doc_start,
                block.scalar_start.checked_add(block.scalar_prefix_len)?,
                block_index,
                leaves,
                rendered_cursor,
                retained_bytes,
                cache_budget,
                path_capacity_bytes,
            )?;
        }
        return Some(());
    }
    let Some(content) = node.content() else {
        return Some(());
    };
    let mut child_doc_offset = doc_offset;
    for (child_index, child) in content.iter().enumerate() {
        #[cfg(test)]
        LOCALIZED_INDEX_PATH_HOPS.set(LOCALIZED_INDEX_PATH_HOPS.get().saturating_add(1));
        path.push(u32::try_from(child_index).ok()?);
        collect_localized_index_streamed(
            child,
            path,
            position_map,
            schema,
            if child.is_element() {
                child_doc_offset.checked_add(1)?
            } else {
                child_doc_offset
            },
            next_block_index,
            leaves,
            rendered_cursor,
            retained_bytes,
            cache_budget,
            path_capacity_bytes,
        )?;
        path.pop()?;
        child_doc_offset = child_doc_offset.checked_add(child.node_size())?;
    }
    Some(())
}

struct RenderedCursor<'a> {
    characters: std::str::Chars<'a>,
    scalar: u32,
    utf16: u32,
}

impl<'a> RenderedCursor<'a> {
    fn new(rendered: &'a str) -> Self {
        Self {
            characters: rendered.chars(),
            scalar: 0,
            utf16: 0,
        }
    }

    fn advance_to(&mut self, target: u32) -> Option<()> {
        while self.scalar < target {
            let character = self.characters.next()?;
            self.scalar = self.scalar.checked_add(1)?;
            self.utf16 = self
                .utf16
                .checked_add(u32::try_from(character.len_utf16()).ok()?)?;
        }
        (self.scalar == target).then_some(())
    }

    fn match_text(&mut self, text: &str) -> Option<(u32, u32)> {
        let start = self.utf16;
        for expected in text.chars() {
            if self.characters.next()? != expected {
                return None;
            }
            self.scalar = self.scalar.checked_add(1)?;
            self.utf16 = self
                .utf16
                .checked_add(u32::try_from(expected.len_utf16()).ok()?)?;
        }
        Some((start, self.utf16))
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_localized_text_leaves_streamed(
    position_map: &PositionMap,
    node: &crate::model::Node,
    content_start: u32,
    scalar_content_start: u32,
    block_index: usize,
    leaves: &mut Vec<LocalizedTextLeafCertificate>,
    rendered_cursor: &mut RenderedCursor<'_>,
    retained_bytes: &mut usize,
    cache_budget: usize,
    path_capacity_bytes: usize,
) -> Option<()> {
    let content = node.content()?;
    let mut child_start = content_start;
    let mut child_scalar_start = scalar_content_start;
    for (child_index, child) in content.iter().enumerate() {
        #[cfg(test)]
        {
            LOCALIZED_INDEX_BUILD_VISITS.set(LOCALIZED_INDEX_BUILD_VISITS.get().saturating_add(1));
            LOCALIZED_INDEX_PATH_HOPS.set(LOCALIZED_INDEX_PATH_HOPS.get().saturating_add(1));
        }
        if let Some(text) = child.text_str() {
            let doc_end = child_start.checked_add(child.node_size())?;
            let scalar_end = child_scalar_start.checked_add(child.node_size())?;
            rendered_cursor.advance_to(child_scalar_start)?;
            let (utf16_start, utf16_end) = rendered_cursor.match_text(text)?;
            let next_len = leaves.len().checked_add(1)?;
            let logical_leaf_bytes =
                next_len.checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
            if path_capacity_bytes.checked_add(logical_leaf_bytes)? > cache_budget {
                return None;
            }
            if leaves.len() == leaves.capacity() {
                #[cfg(test)]
                if forced_localized_index_allocation_stage(
                    LocalizedIndexAllocationStage::LeafGrowth,
                ) {
                    return None;
                }
                let old_capacity_bytes = leaves
                    .capacity()
                    .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
                let maximum_capacity = cache_budget
                    .checked_sub(path_capacity_bytes)?
                    .checked_sub(old_capacity_bytes)?
                    .checked_div(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
                let doubled_capacity = leaves.capacity().checked_mul(2).unwrap_or(maximum_capacity);
                let target_capacity = doubled_capacity.max(next_len).min(maximum_capacity);
                let additional = target_capacity.checked_sub(leaves.capacity())?;
                if additional == 0 {
                    return None;
                }
                leaves.try_reserve_exact(additional).ok()?;
                let new_capacity_bytes = leaves
                    .capacity()
                    .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
                if path_capacity_bytes
                    .checked_add(old_capacity_bytes)?
                    .checked_add(new_capacity_bytes)?
                    > cache_budget
                {
                    return None;
                }
            }
            *retained_bytes = leaves
                .capacity()
                .checked_mul(std::mem::size_of::<LocalizedTextLeafCertificate>())?;
            if path_capacity_bytes.checked_add(*retained_bytes)? > cache_budget {
                return None;
            }
            leaves.push(LocalizedTextLeafCertificate {
                block_index,
                child_ordinal: u32::try_from(child_index).ok()?,
                doc_start: child_start,
                doc_end,
                scalar_start: child_scalar_start,
                scalar_end,
                utf16_start,
                utf16_end,
                text_sha256: sha2::Sha256::digest(text.as_bytes()).into(),
                text_scalars: child.node_size(),
                text_utf16: utf16_end.checked_sub(utf16_start)?,
                text_utf8_bytes: text.len(),
                marks_sha256: canonical_marks_sha256(child.marks())?,
            });
            child_scalar_start = scalar_end;
        } else if child.is_void() {
            child_scalar_start =
                child_scalar_start.checked_add(position_map.inline_void_scalar_len(child)?)?;
        }
        child_start = child_start.checked_add(child.node_size())?;
    }
    Some(())
}

struct Sha256Writer(sha2::Sha256);

impl std::io::Write for Sha256Writer {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.update(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn canonical_marks_sha256(marks: &[Mark]) -> Option<[u8; 32]> {
    let mut writer = Sha256Writer(sha2::Sha256::new());
    std::io::Write::write_all(&mut writer, &u64::try_from(marks.len()).ok()?.to_le_bytes()).ok()?;
    for mark in marks {
        std::io::Write::write_all(&mut writer, b"{\"type\":").ok()?;
        serde_json::to_writer(&mut writer, mark.mark_type()).ok()?;
        std::io::Write::write_all(&mut writer, b",\"attrs\":{").ok()?;
        for (index, (key, value)) in mark.attrs().iter().enumerate() {
            if index != 0 {
                std::io::Write::write_all(&mut writer, b",").ok()?;
            }
            serde_json::to_writer(&mut writer, key).ok()?;
            std::io::Write::write_all(&mut writer, b":").ok()?;
            std::io::Write::write_all(
                &mut writer,
                &crate::boundary::serialize_json_value_stack_safe(value, 0),
            )
            .ok()?;
        }
        std::io::Write::write_all(&mut writer, b"}}").ok()?;
    }
    Some(writer.0.finalize().into())
}

fn node_path_sha256(path: &[u32]) -> [u8; 32] {
    let mut digest = sha2::Sha256::new();
    digest.update(u64::try_from(path.len()).unwrap_or(u64::MAX).to_le_bytes());
    for index in path {
        digest.update(index.to_le_bytes());
    }
    digest.finalize().into()
}

/// Sealed evidence that the current read view admits the narrow existing-leaf
/// insert contract. Stage E2 revalidates it immediately before localized
/// semantic reconstruction.
#[derive(Debug, Clone)]
#[allow(dead_code)] // The seal intentionally retains claims used by later E2 stages.
pub(crate) struct LocalizedInsertAdmission {
    leaf: LocalizedTextLeafCertificate,
    block_path_len: usize,
    block_path_sha256: [u8; 32],
    affected_top_level_index: usize,
    inserted_scalars: u32,
    inserted_utf8_bytes: usize,
    inserted_utf16: u32,
    inserted_escaped_json_bytes: usize,
    next_raw_text_scalars: u64,
    next_raw_text_utf8_bytes: usize,
    next_canonical_serialized_len: usize,
    next_rendered_scalars: u32,
    operation_result: ResolvedSelection,
    history_undo_units: u64,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    selection: ResolvedSelection,
    relative_selection: RelativeSelection,
    stored_marks_sha256: Option<[u8; 32]>,
    canonical_fingerprint: [u8; 32],
    validation_certificate: DocumentValidationCertificate,
    request_id: u64,
    base_document_revision: u64,
    origin: super::TransactionOrigin,
    inserted_at: super::RevisionedPosition,
    inserted_document_position: u32,
    inserted_text_sha256: [u8; 32],
    inserted_marks_sha256: [u8; 32],
    selection_intent: super::SelectionIntent,
    history_policy: super::HistoryPolicy,
    max_length: Option<u32>,
    max_operations_per_transaction: usize,
    max_undo_groups: usize,
    max_derived_output_bytes: usize,
    max_undo_retained_units: u64,
    render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    lookup_seal: Arc<super::mutation::MutationLookupSeed>,
}

struct LocalizedInsertAdmissionRequest<'a> {
    request_id: u64,
    base_document_revision: u64,
    origin: super::TransactionOrigin,
    inserted_at: super::RevisionedPosition,
    document_position: u32,
    text: &'a str,
    marks: &'a [Mark],
    selection_intent: super::SelectionIntent,
    history_policy: super::HistoryPolicy,
}

#[allow(dead_code)]
impl LocalizedInsertAdmission {
    pub(crate) fn lookup_seal_matches(
        &self,
        seed: &Arc<super::mutation::MutationLookupSeed>,
    ) -> bool {
        Arc::ptr_eq(&self.lookup_seal, seed)
    }

    pub(crate) fn same_prewrite_selection_claims(&self, other: &Self) -> bool {
        self.leaf == other.leaf
            && self.block_path_len == other.block_path_len
            && self.block_path_sha256 == other.block_path_sha256
            && self.affected_top_level_index == other.affected_top_level_index
            && self.inserted_scalars == other.inserted_scalars
            && self.inserted_utf8_bytes == other.inserted_utf8_bytes
            && self.inserted_utf16 == other.inserted_utf16
            && self.inserted_escaped_json_bytes == other.inserted_escaped_json_bytes
            && self.next_raw_text_scalars == other.next_raw_text_scalars
            && self.next_raw_text_utf8_bytes == other.next_raw_text_utf8_bytes
            && self.next_canonical_serialized_len == other.next_canonical_serialized_len
            && self.next_rendered_scalars == other.next_rendered_scalars
            && self.operation_result == other.operation_result
            && self.history_undo_units == other.history_undo_units
            && self.document_revision == other.document_revision
            && self.state_revision == other.state_revision
            && self.yrs_state_epoch == other.yrs_state_epoch
            && self.selection == other.selection
            && self.relative_selection == other.relative_selection
            && self.stored_marks_sha256 == other.stored_marks_sha256
            && self.canonical_fingerprint == other.canonical_fingerprint
            && self.validation_certificate == other.validation_certificate
            && self.request_id == other.request_id
            && self.base_document_revision == other.base_document_revision
            && self.origin == other.origin
            && self.inserted_at == other.inserted_at
            && self.inserted_document_position == other.inserted_document_position
            && self.inserted_text_sha256 == other.inserted_text_sha256
            && self.inserted_marks_sha256 == other.inserted_marks_sha256
            && self.selection_intent == other.selection_intent
            && self.history_policy == other.history_policy
            && self.max_length == other.max_length
            && self.max_operations_per_transaction == other.max_operations_per_transaction
            && self.max_undo_groups == other.max_undo_groups
            && self.max_derived_output_bytes == other.max_derived_output_bytes
            && self.max_undo_retained_units == other.max_undo_retained_units
            && Arc::ptr_eq(&self.render_seal, &other.render_seal)
            && Arc::ptr_eq(&self.lookup_seal, &other.lookup_seal)
    }

    pub(crate) fn active_state_structural_seal(&self) -> ActiveStateStructuralSeal {
        ActiveStateStructuralSeal {
            block_index: self.leaf.block_index,
            child_ordinal: self.leaf.child_ordinal,
            leaf_doc_start: self.leaf.doc_start,
            leaf_marks_sha256: self.leaf.marks_sha256,
            block_path_len: self.block_path_len,
            block_path_sha256: self.block_path_sha256,
            affected_top_level_index: self.affected_top_level_index,
        }
    }

    pub(crate) fn inserted_document_position(&self) -> u32 {
        self.inserted_document_position
    }

    pub(crate) fn inserted_scalars(&self) -> u32 {
        self.inserted_scalars
    }

    pub(crate) fn inserted_utf16(&self) -> u32 {
        self.inserted_utf16
    }

    pub(crate) fn operation_result_selection(&self) -> &ResolvedSelection {
        &self.operation_result
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_current<'a, T: ReadTxn>(
        &'a self,
        state: &'a DerivedStateCache,
        transaction: &super::TypedTransaction,
        document_position: u32,
        txn: &T,
        fragment: &XmlFragmentRef,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<ValidatedLocalizedInsertAdmission<'a>> {
        let authority = super::prepared_admission::InstalledDerivedStateAuthority::new(state);
        let lookup_seed =
            DerivedStateAuthority::lookup_seed(&authority, transaction.request_id).ok()?;
        self.validate_current_with_authority(
            state,
            transaction,
            document_position,
            txn,
            fragment,
            lookup_seed,
            authority.materialized_identity(),
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_current_with_authority<'a, T: ReadTxn>(
        &'a self,
        state: &'a DerivedStateCache,
        transaction: &super::TypedTransaction,
        document_position: u32,
        txn: &T,
        fragment: &XmlFragmentRef,
        lookup_seed: &Arc<super::mutation::MutationLookupSeed>,
        identity: Option<&super::prepared_admission::MaterializedMutationIdentity>,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<ValidatedLocalizedInsertAdmission<'a>> {
        let [super::TypedOperation::InsertText { at, text, marks }] =
            transaction.operations.as_slice()
        else {
            return None;
        };
        let index = state.localized_text_index.as_ref()?;
        let expected_leaf = index.strict_inside(document_position)?;
        let expected_block = state.position_map.block(expected_leaf.block_index)?;
        let affected_top_level_index = usize::try_from(*expected_block.node_path.first()?).ok()?;
        let live_leaf = expected_leaf.resolve(&state.document, &state.position_map)?;
        let live_text = live_leaf.text_str()?;
        let inserted_marks_sha256 = canonical_marks_sha256(marks)?;
        let stored_marks_sha256 = match state.stored_marks.as_deref() {
            Some(stored_marks) => Some(canonical_marks_sha256(stored_marks)?),
            None => None,
        };
        let inserted_scalars = u32::try_from(text.chars().count()).ok()?;
        let inserted_utf16 = u32::try_from(text.encode_utf16().count()).ok()?;
        let canonical_serialized_len = identity.map_or(
            state.validation_certificate.canonical_serialized_len,
            |identity| identity.canonical_serialized_len,
        );
        let canonical_fingerprint = identity.map_or_else(
            || state.validation_certificate.canonical_fingerprint,
            |identity| identity.canonical_fingerprint,
        );
        let escaped_limit = editing_limits
            .max_derived_output_bytes
            .checked_sub(canonical_serialized_len)?;
        let inserted_escaped_json_bytes = checked_json_string_body_len(text, escaped_limit)?;
        let next_raw_text_scalars = state
            .validation_certificate
            .raw_text_scalars
            .checked_add(u64::from(inserted_scalars))?;
        let next_raw_text_utf8_bytes = state
            .validation_certificate
            .raw_text_utf8_bytes
            .checked_add(text.len())?;
        let next_canonical_serialized_len =
            canonical_serialized_len.checked_add(inserted_escaped_json_bytes)?;
        let scalar_at = state
            .position_map
            .doc_to_scalar(document_position, &state.document);
        let utf16_at = scalar_offset_to_utf16(&state.rendered_text, scalar_at)?;
        let next_document = document_position.checked_add(inserted_scalars)?;
        let next_scalar = scalar_at.checked_add(inserted_scalars)?;
        let next_utf16 = utf16_at.checked_add(inserted_utf16)?;
        let operation_result = ResolvedSelection::Text {
            anchor: ResolvedPoint {
                document: next_document,
                scalar: next_scalar,
                utf16: next_utf16,
            },
            head: ResolvedPoint {
                document: next_document,
                scalar: next_scalar,
                utf16: next_utf16,
            },
        };
        let history_undo_units = u64::from(inserted_utf16);
        let claims_match = self.request_id == transaction.request_id
            && self.base_document_revision == transaction.base_document_revision
            && self.origin == transaction.origin
            && self.inserted_at == *at
            && self.inserted_document_position == document_position
            && self.inserted_text_sha256 == <[u8; 32]>::from(sha2::Sha256::digest(text.as_bytes()))
            && self.inserted_utf8_bytes == text.len()
            && self.inserted_scalars == inserted_scalars
            && self.inserted_utf16 == inserted_utf16
            && self.inserted_escaped_json_bytes == inserted_escaped_json_bytes
            && self.inserted_marks_sha256 == inserted_marks_sha256
            && self.selection_intent == transaction.selection_intent
            && self.history_policy == transaction.history_policy
            && self.max_length == max_length
            && self.max_operations_per_transaction == editing_limits.max_operations_per_transaction
            && self.max_undo_groups == editing_limits.max_undo_groups
            && self.max_derived_output_bytes == editing_limits.max_derived_output_bytes
            && self.max_undo_retained_units == editing_limits.max_undo_retained_units
            && next_raw_text_scalars <= u64::from(max_length.unwrap_or(u32::MAX))
            && next_canonical_serialized_len <= editing_limits.max_derived_output_bytes
            && history_undo_units <= editing_limits.max_undo_retained_units
            && self.validation_certificate == state.validation_certificate
            && identity.is_none_or(|identity| {
                state.matches_materialized_mutation_identity(
                    &state.canonical_artifact,
                    identity.canonical_fingerprint,
                    identity.canonical_serialized_len,
                    resource_limits,
                    &state.schema_fingerprint,
                    state.document_revision,
                    state.state_revision,
                    yrs_state_epoch,
                )
            })
            && (identity.is_some()
                || self.validation_certificate.matches(
                    &state.canonical_artifact,
                    resource_limits,
                    &state.schema_fingerprint,
                    state.document_revision,
                    state.state_revision,
                    yrs_state_epoch,
                ))
            && self.selection == state.resolved_selection
            && self.relative_selection == state.relative_selection
            && self.stored_marks_sha256 == stored_marks_sha256
            && self.document_revision == state.document_revision
            && self.state_revision == state.state_revision
            && self.yrs_state_epoch == yrs_state_epoch
            && self.canonical_fingerprint == canonical_fingerprint
            && self.leaf == *expected_leaf
            && self.block_path_len == expected_block.node_path.len()
            && self.block_path_sha256 == node_path_sha256(&expected_block.node_path)
            && self.affected_top_level_index == affected_top_level_index
            && <[u8; 32]>::from(sha2::Sha256::digest(live_text.as_bytes()))
                == expected_leaf.text_sha256
            && canonical_marks_sha256(live_leaf.marks())? == expected_leaf.marks_sha256
            && live_leaf.marks() == marks
            && live_leaf.node_size() == expected_leaf.text_scalars
            && u32::try_from(live_text.encode_utf16().count()).ok()? == expected_leaf.text_utf16
            && live_text.len() == expected_leaf.text_utf8_bytes
            && self.next_raw_text_scalars == next_raw_text_scalars
            && self.next_raw_text_utf8_bytes == next_raw_text_utf8_bytes
            && self.next_canonical_serialized_len == next_canonical_serialized_len
            && self.next_rendered_scalars
                == state.rendered_scalars.checked_add(inserted_scalars)?
            && self.operation_result == operation_result
            && self.history_undo_units == history_undo_units
            && Arc::ptr_eq(&self.render_seal, &state.render_blocks)
            && self
                .render_seal
                .matches_identity(&state.document, &state.schema_fingerprint)
            && Arc::ptr_eq(&self.lookup_seal, lookup_seed)
            && self.lookup_seal.matches(
                txn,
                fragment,
                &state.document,
                resource_limits,
                editing_limits,
                max_length,
                &state.schema_fingerprint,
                yrs_state_epoch,
                state.document_revision,
            );
        claims_match.then_some(ValidatedLocalizedInsertAdmission {
            admission: self,
            state,
        })
    }

    #[cfg(test)]
    pub(crate) fn tampered_claims_for_test(&self) -> Vec<(&'static str, Self)> {
        let mut cases = Vec::new();
        macro_rules! tamper {
            ($name:literal, $body:expr) => {{
                let mut proof = self.clone();
                $body(&mut proof);
                cases.push(($name, proof));
            }};
        }
        tamper!("leaf.docStart", |proof: &mut Self| proof.leaf.doc_start =
            proof.leaf.doc_start.saturating_add(1));
        tamper!("leaf.textDigest", |proof: &mut Self| proof
            .leaf
            .text_sha256[0] ^=
            1);
        tamper!("leaf.markDigest", |proof: &mut Self| proof
            .leaf
            .marks_sha256[0] ^=
            1);
        tamper!("blockPathLength", |proof: &mut Self| proof.block_path_len =
            proof.block_path_len.saturating_add(1));
        tamper!("blockPathDigest", |proof: &mut Self| proof
            .block_path_sha256[0] ^=
            1);
        tamper!("topLevelIndex", |proof: &mut Self| proof
            .affected_top_level_index =
            proof.affected_top_level_index.saturating_add(1));
        tamper!("insertedScalars", |proof: &mut Self| proof
            .inserted_scalars =
            proof.inserted_scalars.saturating_add(1));
        tamper!("insertedUtf8", |proof: &mut Self| proof
            .inserted_utf8_bytes =
            proof.inserted_utf8_bytes.saturating_add(1));
        tamper!("insertedUtf16", |proof: &mut Self| proof.inserted_utf16 =
            proof.inserted_utf16.saturating_add(1));
        tamper!("escapedJson", |proof: &mut Self| proof
            .inserted_escaped_json_bytes =
            proof.inserted_escaped_json_bytes.saturating_add(1));
        tamper!("nextRawScalars", |proof: &mut Self| proof
            .next_raw_text_scalars =
            proof.next_raw_text_scalars.saturating_add(1));
        tamper!("nextRawUtf8", |proof: &mut Self| proof
            .next_raw_text_utf8_bytes =
            proof.next_raw_text_utf8_bytes.saturating_add(1));
        tamper!("nextCanonical", |proof: &mut Self| proof
            .next_canonical_serialized_len =
            proof.next_canonical_serialized_len.saturating_add(1));
        tamper!("nextRendered", |proof: &mut Self| proof
            .next_rendered_scalars =
            proof.next_rendered_scalars.saturating_add(1));
        tamper!("operationResult", |proof: &mut Self| proof
            .operation_result =
            ResolvedSelection::All);
        tamper!("historyUnits", |proof: &mut Self| proof
            .history_undo_units =
            proof.history_undo_units.saturating_add(1));
        tamper!("documentRevision", |proof: &mut Self| proof
            .document_revision =
            proof.document_revision.saturating_add(1));
        tamper!("stateRevision", |proof: &mut Self| proof.state_revision =
            proof.state_revision.saturating_add(1));
        tamper!("epoch", |proof: &mut Self| proof.yrs_state_epoch =
            proof.yrs_state_epoch.saturating_add(1));
        tamper!("selection", |proof: &mut Self| proof.selection =
            ResolvedSelection::All);
        tamper!("relativeSelection", |proof: &mut Self| proof
            .relative_selection =
            RelativeSelection::All);
        tamper!("storedMarks", |proof: &mut Self| proof
            .stored_marks_sha256 =
            if proof.stored_marks_sha256.is_some() {
                None
            } else {
                Some([0; 32])
            });
        tamper!("canonicalFingerprint", |proof: &mut Self| proof
            .canonical_fingerprint[0] ^=
            1);
        tamper!("validationCertificate", |proof: &mut Self| proof
            .validation_certificate
            .stats
            .node_count =
            proof
                .validation_certificate
                .stats
                .node_count
                .saturating_add(1));
        tamper!("requestId", |proof: &mut Self| proof.request_id =
            proof.request_id.saturating_add(1));
        tamper!("baseDocumentRevision", |proof: &mut Self| proof
            .base_document_revision =
            proof.base_document_revision.saturating_add(1));
        tamper!("origin", |proof: &mut Self| proof.origin =
            super::TransactionOrigin::RemoteSync);
        tamper!("insertedAt", |proof: &mut Self| proof.inserted_at.offset =
            proof.inserted_at.offset.saturating_add(1));
        tamper!("documentPosition", |proof: &mut Self| proof
            .inserted_document_position =
            proof.inserted_document_position.saturating_add(1));
        tamper!("textDigest", |proof: &mut Self| proof
            .inserted_text_sha256[0] ^=
            1);
        tamper!("marks", |proof: &mut Self| proof.inserted_marks_sha256
            [0] ^= 1);
        tamper!("selectionIntent", |proof: &mut Self| proof
            .selection_intent =
            super::SelectionIntent::Preserve);
        tamper!("historyPolicy", |proof: &mut Self| proof.history_policy =
            super::HistoryPolicy::Skip);
        tamper!("maxLength", |proof: &mut Self| proof.max_length =
            Some(proof.max_length.unwrap_or(u32::MAX).saturating_sub(1)));
        tamper!("maxOperations", |proof: &mut Self| proof
            .max_operations_per_transaction =
            proof.max_operations_per_transaction.saturating_add(1));
        tamper!("maxUndoGroups", |proof: &mut Self| proof.max_undo_groups =
            proof.max_undo_groups.saturating_add(1));
        tamper!("maxDerivedOutput", |proof: &mut Self| proof
            .max_derived_output_bytes =
            proof.max_derived_output_bytes.saturating_add(1));
        tamper!("maxUndo", |proof: &mut Self| proof
            .max_undo_retained_units =
            proof.max_undo_retained_units.saturating_add(1));
        tamper!("renderSeal", |proof: &mut Self| proof.render_seal =
            Arc::new((*proof.render_seal).clone()));
        tamper!("lookupSeal", |proof: &mut Self| proof.lookup_seal =
            Arc::new((*proof.lookup_seal).clone()));
        cases
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveStateStructuralSeal {
    block_index: usize,
    child_ordinal: u32,
    leaf_doc_start: u32,
    leaf_marks_sha256: [u8; 32],
    block_path_len: usize,
    block_path_sha256: [u8; 32],
    affected_top_level_index: usize,
}

#[derive(Debug)]
pub(crate) struct CachedActiveState {
    value: ActiveState,
    retained_bytes: usize,
}

/// Deterministic deep retained-budget measure. It counts owned container slot
/// capacity and recursive string/JSON heap payload with checked arithmetic;
/// allocator-specific HashMap control bytes and global allocator bookkeeping
/// are intentionally outside this portable configured limit.
struct ActiveStateRetainedMeter<'a> {
    limits: &'a ResourceLimits,
    bytes: usize,
    items: usize,
}

impl ActiveStateRetainedMeter<'_> {
    fn add_bytes(&mut self, amount: usize) -> Option<()> {
        self.bytes = self.bytes.checked_add(amount)?;
        (self.bytes <= self.limits.max_input_bytes).then_some(())
    }

    fn add_items(&mut self, amount: usize) -> Option<()> {
        self.items = self.items.checked_add(amount)?;
        (self.items <= self.limits.max_document_nodes).then_some(())
    }

    fn string_heap(&mut self, value: &String) -> Option<()> {
        self.add_items(1)?;
        self.add_bytes(value.capacity())
    }

    fn json_heap(&mut self, value: &serde_json::Value, depth: usize) -> Option<()> {
        let mut pending = vec![(value, depth)];
        while let Some((value, depth)) = pending.pop() {
            if depth > self.limits.max_document_depth {
                return None;
            }
            self.add_items(1)?;
            match value {
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_) => {}
                serde_json::Value::String(value) => self.add_bytes(value.capacity())?,
                serde_json::Value::Array(values) => {
                    self.add_bytes(
                        values
                            .capacity()
                            .checked_mul(std::mem::size_of::<serde_json::Value>())?,
                    )?;
                    let child_depth = depth.checked_add(1)?;
                    pending.extend(values.iter().map(|value| (value, child_depth)));
                }
                serde_json::Value::Object(values) => {
                    self.add_bytes(
                        values
                            .len()
                            .checked_mul(std::mem::size_of::<(String, serde_json::Value)>())?,
                    )?;
                    let child_depth = depth.checked_add(1)?;
                    for (key, value) in values {
                        self.string_heap(key)?;
                        pending.push((value, child_depth));
                    }
                }
            }
        }
        Some(())
    }
}

fn active_state_retained_bytes(
    state: &ActiveState,
    resource_limits: &ResourceLimits,
) -> Option<usize> {
    let mut meter = ActiveStateRetainedMeter {
        limits: resource_limits,
        bytes: 0,
        items: 0,
    };
    meter.add_bytes(std::mem::size_of::<ActiveState>())?;
    for map in [&state.marks, &state.nodes, &state.commands] {
        meter.add_items(map.len())?;
        meter.add_bytes(
            map.capacity()
                .checked_mul(std::mem::size_of::<(String, bool)>())?,
        )?;
        for key in map.keys() {
            meter.string_heap(key)?;
        }
    }
    meter.add_items(state.mark_attrs.len())?;
    meter.add_bytes(
        state
            .mark_attrs
            .capacity()
            .checked_mul(std::mem::size_of::<(String, serde_json::Value)>())?,
    )?;
    for (key, value) in &state.mark_attrs {
        meter.string_heap(key)?;
        meter.json_heap(value, 1)?;
    }
    for strings in [&state.allowed_marks, &state.insertable_nodes] {
        meter.add_items(strings.len())?;
        meter.add_bytes(
            strings
                .capacity()
                .checked_mul(std::mem::size_of::<String>())?,
        )?;
        for value in strings {
            meter.string_heap(value)?;
        }
    }
    Some(meter.bytes)
}

impl CachedActiveState {
    // Returning the original owned state makes optional-cache failure
    // allocation-free; boxing this large Err would undermine that property.
    #[allow(clippy::result_large_err)]
    pub(crate) fn try_new(
        value: ActiveState,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
    ) -> Result<Arc<Self>, ActiveState> {
        let Some(retained_bytes) = active_state_retained_bytes(&value, resource_limits) else {
            return Err(value);
        };
        let retained_budget = resource_limits
            .max_input_bytes
            .min(editing_limits.max_derived_output_bytes);
        #[cfg(test)]
        if FORCE_ACTIVE_STATE_CACHE_ALLOCATION_FAILURE.get() {
            return Err(value);
        }
        #[cfg(test)]
        let retained_budget = FORCE_ACTIVE_STATE_CACHE_BUDGET
            .get()
            .unwrap_or(retained_budget);
        if retained_bytes > retained_budget {
            return Err(value);
        }
        // This catches the optional certificate allocation under configured
        // limits. Deep `ActiveState` ownership is moved, not cloned. As across
        // the rest of this crate, an actual global allocator OOM during
        // `Arc::new` follows Rust's allocator behavior rather than becoming an
        // operation error.
        let mut allocation_probe = Vec::<u8>::new();
        if allocation_probe
            .try_reserve_exact(std::mem::size_of::<Self>())
            .is_err()
        {
            return Err(value);
        }
        Ok(Arc::new(Self {
            value,
            retained_bytes,
        }))
    }

    fn fits_limits(
        &self,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
    ) -> bool {
        self.retained_bytes <= resource_limits.max_input_bytes
            && self.retained_bytes <= editing_limits.max_derived_output_bytes
    }

    pub(crate) fn clone_public(
        &self,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
    ) -> Option<ActiveState> {
        #[cfg(test)]
        if FORCE_ACTIVE_STATE_PUBLIC_MATERIALIZATION_FAILURE.get() {
            return None;
        }
        if !self.fits_limits(resource_limits, editing_limits) {
            return None;
        }
        record_active_state_public_result_clone();
        // The complete owned capacity was admitted above. Rust's global
        // allocator OOM behavior remains unchanged; configured exhaustion is
        // handled before this deep clone.
        Some(self.value.clone())
    }

    pub(crate) fn value(&self) -> &ActiveState {
        &self.value
    }

    pub(crate) fn try_into_value(cached: Arc<Self>) -> Result<ActiveState, Arc<Self>> {
        Arc::try_unwrap(cached).map(|cached| cached.value)
    }

    #[cfg(test)]
    pub(crate) fn retained_bytes_for_test(&self) -> usize {
        self.retained_bytes
    }
}

#[derive(Debug, Clone)]
struct ActiveStateBaseSeal {
    request_id: u64,
    document: Document,
    canonical_artifact: CanonicalArtifact,
    document_revision: u64,
    state_revision: u64,
    yrs_state_epoch: u64,
    schema_fingerprint: String,
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    legacy_selection: Selection,
    relative_selection: RelativeSelection,
    resolved_selection: ResolvedSelection,
    stored_marks: Option<Vec<Mark>>,
    render_seal: Arc<crate::render::incremental::CachedRenderBlocks>,
    lookup_seal: Arc<super::mutation::MutationLookupSeed>,
    validation_certificate: DocumentValidationCertificate,
    structural: ActiveStateStructuralSeal,
}

impl ActiveStateBaseSeal {
    #[allow(clippy::too_many_arguments)]
    fn mint(
        request_id: u64,
        authority: &dyn DerivedStateAuthority,
        structural: ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> OperationResult<Self> {
        let state = authority.installed();
        let lookup_seal = Arc::clone(authority.lookup_seed(request_id)?);
        Ok(Self {
            request_id,
            document: state.document.clone(),
            canonical_artifact: state.canonical_artifact.clone(),
            document_revision: state.document_revision,
            state_revision: state.state_revision,
            yrs_state_epoch,
            schema_fingerprint: state.schema_fingerprint.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            legacy_selection: state.legacy_selection.clone(),
            relative_selection: state.relative_selection.clone(),
            resolved_selection: state.resolved_selection.clone(),
            stored_marks: state.stored_marks.clone(),
            render_seal: Arc::clone(&state.render_blocks),
            lookup_seal,
            validation_certificate: state.validation_certificate.clone(),
            structural,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn matches(
        &self,
        authority: &dyn DerivedStateAuthority,
        structural: &ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> bool {
        let state = authority.installed();
        let Ok(lookup_seed) = authority.lookup_seed(self.request_id) else {
            return false;
        };
        self.matches_with_lookup_seed(
            state,
            lookup_seed,
            structural,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn matches_installed(
        &self,
        authority: &dyn DerivedStateAuthority,
        structural: &ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> bool {
        let state = authority.installed();
        self.matches_with_lookup_seed(
            state,
            &state.mutation_lookup_seed,
            structural,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn matches_with_lookup_seed(
        &self,
        state: &DerivedStateCache,
        lookup_seed: &Arc<super::mutation::MutationLookupSeed>,
        structural: &ActiveStateStructuralSeal,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> bool {
        self.document.shares_root_storage_with(&state.document)
            && self.canonical_artifact.ptr_eq(&state.canonical_artifact)
            && self.document_revision == state.document_revision
            && self.state_revision == state.state_revision
            && self.yrs_state_epoch == yrs_state_epoch
            && self.schema_fingerprint == state.schema_fingerprint
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.legacy_selection == state.legacy_selection
            && self.relative_selection == state.relative_selection
            && self.resolved_selection == state.resolved_selection
            && self.stored_marks == state.stored_marks
            && Arc::ptr_eq(&self.render_seal, &state.render_blocks)
            && Arc::ptr_eq(&self.lookup_seal, lookup_seed)
            && self.validation_certificate == state.validation_certificate
            && self.structural == *structural
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveStateCertificate {
    base: ActiveStateBaseSeal,
    cached: Arc<CachedActiveState>,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedActiveStateTransition {
    base: ActiveStateBaseSeal,
    preview: Document,
    result_selection: ResolvedSelection,
    stored_marks: Option<Vec<Mark>>,
    certificate: Option<Arc<ActiveStateCertificate>>,
}

#[cfg(test)]
impl PreparedActiveStateTransition {
    pub(crate) fn tamper_for_test(&mut self, claim: &str) {
        match claim {
            "documentRevision" => {
                self.base.document_revision = self.base.document_revision.saturating_add(1)
            }
            "stateRevision" => {
                self.base.state_revision = self.base.state_revision.saturating_add(1)
            }
            "epoch" => self.base.yrs_state_epoch = self.base.yrs_state_epoch.saturating_add(1),
            "schema" => self.base.schema_fingerprint.push('!'),
            "resource" => {
                self.base.resource_limits.max_document_nodes = self
                    .base
                    .resource_limits
                    .max_document_nodes
                    .saturating_add(1)
            }
            "editing" => {
                self.base.editing_limits.max_derived_output_bytes = self
                    .base
                    .editing_limits
                    .max_derived_output_bytes
                    .saturating_add(1)
            }
            "maxLength" => self.base.max_length = Some(self.base.max_length.unwrap_or(0) + 1),
            "selection" => self.base.resolved_selection = ResolvedSelection::All,
            "relativeSelection" => self.base.relative_selection = RelativeSelection::All,
            "legacySelection" => self.base.legacy_selection = Selection::all(),
            "storedMarks" => self.base.stored_marks = Some(Vec::new()),
            "structural" => {
                self.base.structural.leaf_doc_start =
                    self.base.structural.leaf_doc_start.saturating_add(1)
            }
            "resultSelection" => self.result_selection = ResolvedSelection::All,
            "preview" => self.preview = self.base.document.clone(),
            "render" => self.base.render_seal = Arc::new((*self.base.render_seal).clone()),
            "lookup" => self.base.lookup_seal = Arc::new((*self.base.lookup_seal).clone()),
            "validation" => {
                self.base.validation_certificate.state_revision = self
                    .base
                    .validation_certificate
                    .state_revision
                    .saturating_add(1)
            }
            "cachedPayloadIdentity" => {
                let certificate = self
                    .certificate
                    .as_ref()
                    .expect("warm transition certificate");
                self.certificate = Some(Arc::new(ActiveStateCertificate {
                    base: certificate.base.clone(),
                    cached: Arc::new(CachedActiveState {
                        value: certificate.cached.value.clone(),
                        retained_bytes: certificate.cached.retained_bytes,
                    }),
                }));
            }
            other => panic!("unknown active-state transition claim {other}"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedActiveStateInstall {
    request_id: u64,
    preview: Document,
    result_selection: ResolvedSelection,
    stored_marks: Option<Vec<Mark>>,
    cached: Arc<CachedActiveState>,
    structural: ActiveStateStructuralSeal,
    next_document_revision: u64,
    next_state_revision: u64,
    next_yrs_state_epoch: u64,
}

impl DerivedStateCache {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_active_state_transition(
        &self,
        request_id: u64,
        authority: &dyn DerivedStateAuthority,
        admission: &LocalizedInsertAdmission,
        preview: &Document,
        result_selection: &ResolvedSelection,
        stored_marks: Option<&[Mark]>,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> OperationResult<PreparedActiveStateTransition> {
        let structural = admission.active_state_structural_seal();
        let certificate = self
            .active_state_certificate
            .as_ref()
            .filter(|certificate| {
                certificate.base.matches_installed(
                    authority,
                    &structural,
                    resource_limits,
                    editing_limits,
                    max_length,
                    yrs_state_epoch,
                )
            })
            .map(Arc::clone);
        Ok(PreparedActiveStateTransition {
            base: ActiveStateBaseSeal::mint(
                request_id,
                authority,
                structural,
                resource_limits,
                editing_limits,
                max_length,
                yrs_state_epoch,
            )?,
            preview: preview.clone(),
            result_selection: result_selection.clone(),
            stored_marks: stored_marks.map(<[Mark]>::to_vec),
            certificate,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_active_state_transition(
        &self,
        authority: &dyn DerivedStateAuthority,
        transition: &PreparedActiveStateTransition,
        structural: &ActiveStateStructuralSeal,
        preview: &Document,
        result_selection: &ResolvedSelection,
        stored_marks: Option<&[Mark]>,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<Option<Arc<CachedActiveState>>> {
        let current_cache_matches = match (
            transition.certificate.as_ref(),
            self.active_state_certificate.as_ref(),
        ) {
            (Some(transition_certificate), Some(certificate)) => {
                Arc::ptr_eq(transition_certificate, certificate)
                    && certificate
                        .cached
                        .fits_limits(resource_limits, editing_limits)
                    && certificate.base.matches_installed(
                        authority,
                        structural,
                        resource_limits,
                        editing_limits,
                        max_length,
                        yrs_state_epoch,
                    )
            }
            (None, None) => true,
            _ => false,
        };
        (current_cache_matches
            && transition.base.matches(
                authority,
                structural,
                resource_limits,
                editing_limits,
                max_length,
                yrs_state_epoch,
            )
            && transition.preview.shares_root_storage_with(preview)
            && transition.result_selection == *result_selection
            && transition.stored_marks.as_deref() == stored_marks)
            .then(|| {
                transition
                    .certificate
                    .as_ref()
                    .map(|certificate| Arc::clone(&certificate.cached))
            })
    }

    pub(crate) fn prepare_active_state_install(
        transition: &PreparedActiveStateTransition,
        cached: Arc<CachedActiveState>,
        next_document_revision: u64,
        next_state_revision: u64,
        next_yrs_state_epoch: u64,
    ) -> PreparedActiveStateInstall {
        PreparedActiveStateInstall {
            request_id: transition.base.request_id,
            preview: transition.preview.clone(),
            result_selection: transition.result_selection.clone(),
            stored_marks: transition.stored_marks.clone(),
            cached,
            structural: transition.base.structural.clone(),
            next_document_revision,
            next_state_revision,
            next_yrs_state_epoch,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_active_state_certificate(
        install: PreparedActiveStateInstall,
        authority: &dyn DerivedStateAuthority,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<Arc<ActiveStateCertificate>> {
        let state = authority.installed();
        if !install.preview.shares_root_storage_with(&state.document)
            || install.result_selection != state.resolved_selection
            || install.stored_marks != state.stored_marks
            || install.next_document_revision != state.document_revision
            || install.next_state_revision != state.state_revision
            || install.next_yrs_state_epoch != yrs_state_epoch
        {
            return None;
        }
        Some(Arc::new(ActiveStateCertificate {
            base: ActiveStateBaseSeal::mint(
                install.request_id,
                authority,
                install.structural,
                resource_limits,
                editing_limits,
                max_length,
                yrs_state_epoch,
            )
            .ok()?,
            cached: install.cached,
        }))
    }

    pub(crate) fn install_active_state_certificate(
        &mut self,
        certificate: Arc<ActiveStateCertificate>,
    ) {
        self.active_state_certificate = Some(certificate);
    }

    pub(crate) fn clear_active_state_certificate(&mut self) {
        if self.active_state_certificate.take().is_some() {
            record_active_state_cache_drop();
        }
    }

    pub(crate) fn has_active_state_certificate(&self) -> bool {
        self.active_state_certificate.is_some()
    }

    #[cfg(test)]
    pub(crate) fn active_state_cache_for_test(&self) -> Option<Arc<CachedActiveState>> {
        self.active_state_certificate
            .as_ref()
            .map(|certificate| Arc::clone(&certificate.cached))
    }

    #[cfg(test)]
    pub(crate) fn remove_active_state_certificate_for_test(&mut self) {
        self.active_state_certificate = None;
    }

    #[cfg(test)]
    pub(crate) fn replace_active_state_certificate_identity_for_test(&mut self) {
        let certificate = self
            .active_state_certificate
            .as_ref()
            .expect("test requires an active-state certificate")
            .as_ref()
            .clone();
        self.active_state_certificate = Some(Arc::new(certificate));
    }

    #[cfg(test)]
    pub(crate) fn replace_active_state_payload_identity_for_test(&mut self) {
        let certificate = Arc::clone(
            self.active_state_certificate
                .as_ref()
                .expect("test requires an active-state certificate"),
        );
        self.active_state_certificate = Some(Arc::new(ActiveStateCertificate {
            base: certificate.base.clone(),
            cached: Arc::new(CachedActiveState {
                value: certificate.cached.value.clone(),
                retained_bytes: certificate.cached.retained_bytes,
            }),
        }));
    }
}

#[allow(dead_code)] // Stage E2 consumes the semantic subset; later stages use the remainder.
pub(crate) struct ValidatedLocalizedInsertAdmission<'a> {
    admission: &'a LocalizedInsertAdmission,
    state: &'a DerivedStateCache,
}

#[allow(dead_code)]
impl ValidatedLocalizedInsertAdmission<'_> {
    pub(crate) fn prepare_derived_evidence(
        &self,
        preview: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
    ) -> Option<PreparedDerivedEvidence> {
        let validation_certificate = self.state.validation_certificate.promote_existing_insert(
            canonical_artifact,
            derivations,
            self.admission,
        )?;
        #[cfg(test)]
        LOCALIZED_INDEX_PROMOTION_ATTEMPTS
            .set(LOCALIZED_INDEX_PROMOTION_ATTEMPTS.get().saturating_add(1));
        let cache_budget = self
            .state
            .validation_certificate
            .resource_limits
            .max_input_bytes;
        #[cfg(test)]
        let cache_budget = FORCE_LOCALIZED_INDEX_BUDGET.get().unwrap_or(cache_budget);
        let localized_text_index = self.state.localized_text_index.as_ref().and_then(|index| {
            index.promote_existing_insert(
                &self.state.validation_certificate,
                self.admission,
                self.block_path(),
                preview,
                canonical_artifact,
                cache_budget,
            )
        });
        #[cfg(test)]
        if localized_text_index.is_some() {
            LOCALIZED_INDEX_PROMOTION_SUCCESSES
                .set(LOCALIZED_INDEX_PROMOTION_SUCCESSES.get().saturating_add(1));
        } else {
            LOCALIZED_INDEX_PROMOTION_DROPS
                .set(LOCALIZED_INDEX_PROMOTION_DROPS.get().saturating_add(1));
        }
        Some(PreparedDerivedEvidence {
            request_id: self.admission.request_id,
            base_document_root: self.state.document.root().clone(),
            preview_root: preview.root().clone(),
            base_validation: self.state.validation_certificate.clone(),
            base_render_seal: Arc::clone(&self.admission.render_seal),
            base_lookup_seal: Arc::clone(&self.admission.lookup_seal),
            max_operations_per_transaction: self.admission.max_operations_per_transaction,
            max_undo_groups: self.admission.max_undo_groups,
            max_derived_output_bytes: self.admission.max_derived_output_bytes,
            max_undo_retained_units: self.admission.max_undo_retained_units,
            max_length: self.admission.max_length,
            derivation_identity_seal: Arc::clone(&derivations.identity_seal),
            preview_rendered_scalars: derivations.rendered_scalars,
            preview_document_text_bytes: derivations.document_text_bytes,
            preview_document_node_count: derivations.document_node_count,
            preview_position_total_scalars: derivations.position_map.total_scalars(),
            preview_position_block_count: derivations.position_map.block_count(),
            canonical_fingerprint: canonical_artifact.sha256(),
            canonical_serialized_len: canonical_artifact.serialized_len(),
            validation_certificate,
            localized_text_index,
            localized_render_transition_proof: Some(LocalizedRenderTransitionProof {
                base_document_root: self.state.document.root().clone(),
                preview_root: preview.root().clone(),
                base_render_seal: Arc::clone(&self.admission.render_seal),
                resource_limits: self.state.validation_certificate.resource_limits.clone(),
                schema_fingerprint: Arc::clone(
                    &self.state.validation_certificate.schema_fingerprint,
                ),
                max_operations_per_transaction: self.admission.max_operations_per_transaction,
                max_undo_groups: self.admission.max_undo_groups,
                max_derived_output_bytes: self.admission.max_derived_output_bytes,
                max_undo_retained_units: self.admission.max_undo_retained_units,
                max_length: self.admission.max_length,
                derivation_identity_seal: Arc::clone(&derivations.identity_seal),
                target_top_level_index: self.admission.affected_top_level_index,
                inserted_scalar_delta: self.admission.inserted_scalars,
                top_level_cardinality: self.state.document.root().child_count(),
                operation_kind: LocalizedRenderOperationKind::ExistingTextInsert,
            }),
        })
    }

    pub(crate) fn document_position(&self) -> u32 {
        self.admission.inserted_document_position
    }

    pub(crate) fn inserted_scalars(&self) -> u32 {
        self.admission.inserted_scalars
    }

    pub(crate) fn block_path(&self) -> &[u32] {
        self.state
            .position_map
            .block(self.admission.leaf.block_index)
            .expect("validated admission retains its position block")
            .node_path
            .as_slice()
    }

    pub(crate) fn child_ordinal(&self) -> u32 {
        self.admission.leaf.child_ordinal
    }

    pub(crate) fn leaf_doc_start(&self) -> u32 {
        self.admission.leaf.doc_start
    }

    pub(crate) fn affected_top_level_index(&self) -> usize {
        self.admission.affected_top_level_index
    }

    pub(crate) fn document_node_count(&self) -> usize {
        self.state.document_node_count
    }

    pub(crate) fn rendered_scalar_position(&self) -> u32 {
        self.state.position_map.doc_to_scalar(
            self.admission.inserted_document_position,
            &self.state.document,
        )
    }

    pub(crate) fn rendered_text(&self) -> &str {
        &self.state.rendered_text
    }

    pub(crate) fn next_raw_text_scalars(&self) -> u64 {
        self.admission.next_raw_text_scalars
    }

    pub(crate) fn next_raw_text_utf8_bytes(&self) -> usize {
        self.admission.next_raw_text_utf8_bytes
    }

    pub(crate) fn next_canonical_serialized_len(&self) -> usize {
        self.admission.next_canonical_serialized_len
    }

    pub(crate) fn history_undo_units(&self) -> u64 {
        self.admission.history_undo_units
    }

    pub(crate) fn next_rendered_scalars(&self) -> u32 {
        self.admission.next_rendered_scalars
    }

    pub(crate) fn operation_result(&self) -> &ResolvedSelection {
        &self.admission.operation_result
    }

    pub(crate) fn stored_marks(&self) -> Option<&[Mark]> {
        self.state.stored_marks.as_deref()
    }
}

/// A fully materialized selection state whose three representations were
/// proven against the same prewrite view. Keeping the fields private prevents
/// later compiler stages from mixing representations from different views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinalizedSelectionState {
    relative: RelativeSelection,
    resolved: ResolvedSelection,
    legacy: Selection,
}

impl FinalizedSelectionState {
    pub(crate) fn new(
        relative: RelativeSelection,
        resolved: ResolvedSelection,
        legacy: Selection,
    ) -> Option<Self> {
        (resolved_to_legacy(&resolved) == legacy).then_some(Self {
            relative,
            resolved,
            legacy,
        })
    }

    pub(crate) fn relative(&self) -> &RelativeSelection {
        &self.relative
    }

    fn into_parts(self) -> (RelativeSelection, ResolvedSelection, Selection) {
        (self.relative, self.resolved, self.legacy)
    }

    #[cfg(test)]
    pub(crate) fn tampered_for_test(&self) -> Vec<Self> {
        let mut relative = self.clone();
        relative.relative = RelativeSelection::All;
        let mut resolved = self.clone();
        resolved.resolved = ResolvedSelection::All;
        let mut legacy = self.clone();
        legacy.legacy = Selection::all();
        vec![relative, resolved, legacy]
    }
}

#[derive(Debug)]
pub(crate) struct DerivedStateCache {
    pub document: Document,
    pub canonical_artifact: CanonicalArtifact,
    pub position_map: PositionMap,
    pub rendered_text: String,
    pub rendered_scalars: u32,
    pub document_text_bytes: usize,
    pub document_node_count: usize,
    pub legacy_selection: Selection,
    pub relative_selection: RelativeSelection,
    pub resolved_selection: ResolvedSelection,
    pub stored_marks: Option<Vec<Mark>>,
    pub document_revision: u64,
    pub state_revision: u64,
    pub schema_fingerprint: String,
    pub render_blocks: Arc<crate::render::incremental::CachedRenderBlocks>,
    pub mutation_lookup_seed: Arc<super::mutation::MutationLookupSeed>,
    pub validation_certificate: DocumentValidationCertificate,
    #[cfg_attr(not(test), allow(dead_code))]
    pub localized_text_index: Option<LocalizedTextLeafIndex>,
    active_state_certificate: Option<Arc<ActiveStateCertificate>>,
}

#[derive(Debug, Clone)]
pub(crate) struct HistoryDocumentSnapshot {
    document: Document,
    canonical_artifact: CanonicalArtifact,
    position_map: PositionMap,
    rendered_text: String,
    rendered_scalars: u32,
    document_text_bytes: usize,
    document_node_count: usize,
    render_blocks: Arc<crate::render::incremental::CachedRenderBlocks>,
    validation_certificate: DocumentValidationCertificate,
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: Arc<str>,
    fragment_name: Arc<str>,
    scope: Option<super::DocumentScope>,
    retained_bytes: usize,
}

/// One exact decoded history-candidate read plus an optional pure admission
/// proof. No CRDT snapshot or mutation-seed publication work occurs while
/// constructing this value.
pub(crate) struct PreparedHistoryCandidateRead {
    json: serde_json::Value,
    admission: Option<AdmittedHistoryCandidateRead>,
}

impl PreparedHistoryCandidateRead {
    pub(crate) fn into_parts(self) -> (serde_json::Value, Option<AdmittedHistoryCandidateRead>) {
        (self.json, self.admission)
    }
}

/// Non-Clone proof that the sole retained-history codec read matched the
/// retained document and every stable restoration seal. Snapshot allocation
/// is deliberately deferred until semantic fast-path eligibility succeeds.
pub(crate) struct AdmittedHistoryCandidateRead {
    request_id: u64,
    source_document: Document,
    canonical_artifact: CanonicalArtifact,
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    store_token: usize,
    fragment_id: BranchID,
    schema_fingerprint: Arc<str>,
    yrs_state_epoch: u64,
    document_revision: u64,
}

impl AdmittedHistoryCandidateRead {
    fn validate_request(&self, request_id: u64) -> OperationResult<()> {
        if self.request_id != request_id {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history candidate read request is stale or contradictory",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_restoration<T: ReadTxn>(
        &self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        snapshot: &HistoryDocumentSnapshot,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<()> {
        self.validate_request(request_id)?;
        let matches = self.store_token == txn.store() as *const _ as usize
            && self.fragment_id == AsRef::<Branch>::as_ref(fragment).id()
            && self
                .source_document
                .shares_root_storage_with(&snapshot.document)
            && self.canonical_artifact.ptr_eq(&snapshot.canonical_artifact)
            && self
                .canonical_artifact
                .matches_exact_source_document(&snapshot.document)
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.yrs_state_epoch == yrs_state_epoch
            && self.document_revision == document_revision;
        if !matches {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history snapshot restoration admission is stale or contradictory",
            ));
        }
        Ok(())
    }

    fn mint_capability<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> OperationResult<HistoryMutationLookupCapability> {
        self.validate_request(request_id)?;
        if self.store_token != txn.store() as *const _ as usize
            || self.fragment_id != AsRef::<Branch>::as_ref(fragment).id()
        {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history candidate read storage is stale or contradictory",
            ));
        }
        let history_store_snapshot =
            super::mutation::MutationLookupSeed::prepare_history_store_snapshot(
                request_id,
                txn,
                self.resource_limits.max_encoded_state_bytes,
            )?;
        let proof = AdmittedHistoryMutationLookupProof {
            source_document: self.source_document,
            canonical_artifact: self.canonical_artifact,
            resource_limits: self.resource_limits,
            editing_limits: self.editing_limits,
            max_length: self.max_length,
            store_token: self.store_token,
            fragment_id: self.fragment_id,
            schema_fingerprint: self.schema_fingerprint,
            yrs_state_epoch: self.yrs_state_epoch,
            document_revision: self.document_revision,
            history_store_snapshot,
        };
        Ok(HistoryMutationLookupCapability {
            request_id,
            seed: super::mutation::MutationLookupSeed::from_admitted_history_proof(proof),
        })
    }

    #[cfg(test)]
    pub(crate) fn mint_capability_for_test<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
    ) -> OperationResult<HistoryMutationLookupCapability> {
        self.mint_capability(request_id, txn, fragment)
    }
}

/// Non-Clone, one-shot ownership of the only mutation lookup seed that may
/// carry retained history store/document evidence.
#[derive(Debug)]
pub(crate) struct HistoryMutationLookupCapability {
    request_id: u64,
    seed: super::mutation::MutationLookupSeed,
}

#[derive(Debug)]
pub(crate) struct RestoredHistoryDocumentState {
    pub(crate) state: DerivedStateCache,
    pub(crate) candidate_publication: HistoryMutationLookupCapability,
}

impl HistoryMutationLookupCapability {
    fn validate_request(&self, request_id: u64) -> OperationResult<()> {
        if self.request_id != request_id {
            return Err(OperationError::engine_invariant_failed(
                request_id,
                None,
                "history mutation lookup capability request is stale or contradictory",
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_candidate_publication<T: ReadTxn>(
        self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        source_document: &Document,
        canonical_artifact: &CanonicalArtifact,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<Arc<super::mutation::MutationLookupSeed>> {
        self.validate_request(request_id)?;
        self.seed.prepare_candidate_publication(
            request_id,
            txn,
            fragment,
            schema,
            source_document,
            canonical_artifact,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )
    }

    fn prepare_unavailable_placeholder(
        self,
        request_id: u64,
    ) -> OperationResult<(Arc<super::mutation::MutationLookupSeed>, Self)> {
        self.validate_request(request_id)?;
        // MutationLookupSeed is Clone for the general lifecycle, but retained
        // history evidence may only be duplicated inside this capability
        // boundary. The clone is immediately consumed by the stripping
        // publication operation; only its proof-free Arc can escape.
        let unavailable = self
            .seed
            .clone()
            .try_publish_history_unavailable(request_id)?;
        Ok((unavailable, self))
    }

    #[cfg(test)]
    pub(crate) fn into_unavailable_seed_for_test(
        self,
        request_id: u64,
    ) -> OperationResult<Arc<super::mutation::MutationLookupSeed>> {
        self.validate_request(request_id)?;
        self.seed.try_publish_history_unavailable(request_id)
    }
}

/// Unforgeable handoff from the exact derived-state read factory into the
/// private mutation binding constructor. Fields remain private here.
pub(crate) struct AdmittedHistoryMutationLookupProof {
    source_document: Document,
    canonical_artifact: CanonicalArtifact,
    resource_limits: ResourceLimits,
    editing_limits: super::EditingLimits,
    max_length: Option<u32>,
    store_token: usize,
    fragment_id: BranchID,
    schema_fingerprint: Arc<str>,
    yrs_state_epoch: u64,
    document_revision: u64,
    history_store_snapshot: super::mutation::HistoryStoreSnapshotEvidence,
}

impl AdmittedHistoryMutationLookupProof {
    #[allow(clippy::type_complexity)]
    pub(crate) fn into_seed_parts(
        self,
    ) -> (
        Document,
        CanonicalArtifact,
        ResourceLimits,
        super::EditingLimits,
        Option<u32>,
        usize,
        BranchID,
        Arc<str>,
        u64,
        u64,
        super::mutation::HistoryStoreSnapshotEvidence,
    ) {
        (
            self.source_document,
            self.canonical_artifact,
            self.resource_limits,
            self.editing_limits,
            self.max_length,
            self.store_token,
            self.fragment_id,
            self.schema_fingerprint,
            self.yrs_state_epoch,
            self.document_revision,
            self.history_store_snapshot,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HistoryDocumentSnapshotRetainedBytes(usize);

impl HistoryDocumentSnapshotRetainedBytes {
    pub(crate) fn get(self) -> usize {
        self.0
    }
}

pub(crate) struct HistoryDocumentSnapshotRetainedInput<'a> {
    pub document: &'a Document,
    pub canonical_artifact: &'a CanonicalArtifact,
    pub position_map: &'a PositionMap,
    pub rendered_text: &'a String,
    pub render_blocks: &'a crate::render::incremental::CachedRenderBlocks,
    pub schema_fingerprint: &'a str,
    pub fragment_name: &'a str,
    pub scope: Option<&'a super::DocumentScope>,
}

fn arc_allocation_bound(payload_bytes: usize) -> Option<usize> {
    // Two strong/weak counters plus one word for allocator padding and
    // alignment conservatively bound the Arc allocation header.
    payload_bytes.checked_add(std::mem::size_of::<[usize; 3]>())
}

pub(crate) fn history_document_snapshot_retained_bytes(
    input: HistoryDocumentSnapshotRetainedInput<'_>,
) -> Option<HistoryDocumentSnapshotRetainedBytes> {
    if !input
        .canonical_artifact
        .matches_exact_source_document(input.document)
    {
        return None;
    }
    let retained_charge = input
        .canonical_artifact
        .history_snapshot_retained_charge()?;
    history_document_snapshot_retained_bytes_with_precomputed_document_charge(
        retained_charge.source_document_retained_bytes,
        retained_charge.canonical_retained_bytes,
        input.position_map,
        input.rendered_text,
        input.render_blocks,
        input.schema_fingerprint,
        input.fragment_name,
        input.scope,
    )
}

#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub(crate) fn history_document_snapshot_retained_bytes_with_canonical_charge(
    document: &Document,
    canonical_retained_bytes: usize,
    position_map: &PositionMap,
    rendered_text: &String,
    render_blocks: &crate::render::incremental::CachedRenderBlocks,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&super::DocumentScope>,
) -> Option<HistoryDocumentSnapshotRetainedBytes> {
    history_document_snapshot_retained_bytes_with_precomputed_document_charge(
        document.history_snapshot_retained_bytes()?,
        canonical_retained_bytes,
        position_map,
        rendered_text,
        render_blocks,
        schema_fingerprint,
        fragment_name,
        scope,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn history_document_snapshot_retained_bytes_with_precomputed_document_charge(
    document_retained_bytes: usize,
    canonical_retained_bytes: usize,
    position_map: &PositionMap,
    rendered_text: &String,
    render_blocks: &crate::render::incremental::CachedRenderBlocks,
    schema_fingerprint: &str,
    fragment_name: &str,
    scope: Option<&super::DocumentScope>,
) -> Option<HistoryDocumentSnapshotRetainedBytes> {
    // These immutable payloads are shallow-cloned into the snapshot and may
    // otherwise become unreachable after the next edit. Each helper walks the
    // complete owned capacity recursively with checked arithmetic. Shared node
    // roots are deliberately overcounted across the three payloads; that keeps
    // admission conservative without allocator-identity bookkeeping.
    let shared_payload_bytes = document_retained_bytes
        .checked_add(canonical_retained_bytes)?
        .checked_add(render_blocks.history_snapshot_retained_bytes()?)?;

    let snapshot_allocation_bytes =
        arc_allocation_bound(std::mem::size_of::<HistoryDocumentSnapshot>())?;
    let position_map_bytes = position_map.history_snapshot_clone_retained_bytes()?;
    let schema_arc_bytes = arc_allocation_bound(schema_fingerprint.len())?;
    let validation_schema_arc_bytes = arc_allocation_bound(schema_fingerprint.len())?;
    let fragment_arc_bytes = arc_allocation_bound(fragment_name.len())?;
    let scope_string_bytes = scope.map_or(Some(0), |scope| {
        scope
            .document_id
            .capacity()
            .checked_add(scope.lineage_id.capacity())
    })?;

    snapshot_allocation_bytes
        .checked_add(position_map_bytes)?
        .checked_add(rendered_text.capacity())?
        .checked_add(schema_arc_bytes)?
        .checked_add(validation_schema_arc_bytes)?
        .checked_add(fragment_arc_bytes)?
        .checked_add(scope_string_bytes)?
        .checked_add(shared_payload_bytes)
        .map(HistoryDocumentSnapshotRetainedBytes)
}

impl HistoryDocumentSnapshot {
    pub(crate) fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    #[allow(clippy::too_many_arguments)]
    fn admits_candidate_read(
        &self,
        candidate_json: &serde_json::Value,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        fragment_name: &str,
        scope: Option<&super::DocumentScope>,
    ) -> bool {
        #[cfg(test)]
        if FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK.get() {
            return false;
        }
        crate::boundary::json_values_equal_stack_safe(
            self.canonical_artifact.value(),
            candidate_json,
        ) && self
            .canonical_artifact
            .matches_exact_source_document(&self.document)
            && self.canonical_artifact.schema_fingerprint() == schema_fingerprint
            && self.canonical_artifact.format_version()
                == super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            && crate::schema::schema_fingerprint(schema) == schema_fingerprint
            && matches!(
                AsRef::<Branch>::as_ref(fragment).id(),
                BranchID::Root(name) if name.as_ref() == fragment_name
            )
            && self.resource_limits == *resource_limits
            && self.editing_limits == *editing_limits
            && self.max_length == max_length
            && self.schema_fingerprint.as_ref() == schema_fingerprint
            && self.fragment_name.as_ref() == fragment_name
            && self.scope.as_ref() == scope
            && self.validation_certificate.resource_limits == *resource_limits
            && self.validation_certificate.schema_fingerprint.as_ref() == schema_fingerprint
            && (self
                .validation_certificate
                .canonical_artifact
                .ptr_eq(&self.canonical_artifact)
                || (self.validation_certificate.canonical_fingerprint()
                    == self.canonical_artifact.sha256()
                    && self.validation_certificate.canonical_serialized_len
                        == self.canonical_artifact.serialized_len()))
            && self.validation_certificate.raw_text_scalars
                == self.canonical_artifact.text_scalar_len()
            && self.validation_certificate.raw_text_utf8_bytes
                == self.canonical_artifact.text_utf8_bytes()
            && self.validation_certificate.stats.node_count == self.document_node_count
            && self.validation_certificate.stats.node_count <= resource_limits.max_document_nodes
            && self.validation_certificate.stats.max_depth <= resource_limits.max_document_depth
            && self.validation_certificate.metrics.metadata_bytes <= resource_limits.max_input_bytes
            && self.validation_certificate.metrics.validation_work
                <= resource_limits.max_document_nodes.saturating_mul(128)
            && self.document_text_bytes == self.validation_certificate.raw_text_utf8_bytes
            && self.canonical_artifact.serialized_len() <= editing_limits.max_derived_output_bytes
            && max_length
                .is_none_or(|limit| self.canonical_artifact.text_scalar_len() <= u64::from(limit))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_candidate_read<T: ReadTxn>(
        &self,
        request_id: u64,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        fragment_name: &str,
        scope: Option<&super::DocumentScope>,
        yrs_state_epoch: u64,
        document_revision: u64,
    ) -> OperationResult<PreparedHistoryCandidateRead> {
        // This is the sole full fragment projection on the retained-snapshot
        // path. The same value both drives HistoryDocumentSnapshot admission
        // and becomes the generic fallback input.
        let json = YrsDocumentCodec::new(schema, resource_limits)
            .read_json(fragment, txn)
            .map_err(|error| history_candidate_read_error(request_id, error))?;
        if !self.admits_candidate_read(
            &json,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            fragment_name,
            scope,
        ) {
            return Ok(PreparedHistoryCandidateRead {
                json,
                admission: None,
            });
        }
        let admission = AdmittedHistoryCandidateRead {
            request_id,
            source_document: self.document.clone(),
            canonical_artifact: self.canonical_artifact.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            store_token: txn.store() as *const _ as usize,
            fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
            schema_fingerprint: Arc::clone(&self.schema_fingerprint),
            yrs_state_epoch,
            document_revision,
        };
        Ok(PreparedHistoryCandidateRead {
            json,
            admission: Some(admission),
        })
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_history_candidate_read_for_test<T: ReadTxn>(
    request_id: u64,
    txn: &T,
    fragment: &XmlFragmentRef,
    schema: &Schema,
    source_document: &Document,
    canonical_artifact: &CanonicalArtifact,
    resource_limits: &ResourceLimits,
    editing_limits: &super::EditingLimits,
    max_length: Option<u32>,
    schema_fingerprint: &str,
    yrs_state_epoch: u64,
    document_revision: u64,
) -> OperationResult<PreparedHistoryCandidateRead> {
    let json = YrsDocumentCodec::new(schema, resource_limits)
        .read_json(fragment, txn)
        .map_err(|error| history_candidate_read_error(request_id, error))?;
    let admission =
        if crate::boundary::json_values_equal_stack_safe(canonical_artifact.value(), &json)
            && canonical_artifact.matches_exact_source_document(source_document)
            && canonical_artifact.schema_fingerprint() == schema_fingerprint
            && canonical_artifact.format_version()
                == super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            && crate::schema::schema_fingerprint(schema) == schema_fingerprint
        {
            Some(AdmittedHistoryCandidateRead {
                request_id,
                source_document: source_document.clone(),
                canonical_artifact: canonical_artifact.clone(),
                resource_limits: resource_limits.clone(),
                editing_limits: editing_limits.clone(),
                max_length,
                store_token: txn.store() as *const _ as usize,
                fragment_id: AsRef::<Branch>::as_ref(fragment).id(),
                schema_fingerprint: Arc::from(schema_fingerprint),
                yrs_state_epoch,
                document_revision,
            })
        } else {
            None
        };
    Ok(PreparedHistoryCandidateRead { json, admission })
}

fn history_candidate_read_error(request_id: u64, error: super::YrsEngineError) -> OperationError {
    if let (Some(limit), Some(actual)) = (error.limit, error.actual) {
        let field = if error.code == "INPUT_LIMIT_EXCEEDED" {
            "maxEncodedStateBytes"
        } else {
            "document"
        };
        OperationError::document_limit_exceeded(
            request_id,
            None,
            field,
            u64::try_from(limit).unwrap_or(u64::MAX),
            u64::try_from(actual).unwrap_or(u64::MAX),
        )
    } else {
        OperationError::engine_invariant_failed(request_id, None, error.message)
    }
}

impl DerivedStateCache {
    // Keep every mutation identity dimension explicit so callers cannot hide a mismatched seal.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn matches_materialized_mutation_identity(
        &self,
        canonical_artifact: &CanonicalArtifact,
        canonical_fingerprint: [u8; 32],
        canonical_serialized_len: usize,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> bool {
        self.document_revision == document_revision
            && self.state_revision == state_revision
            && self.schema_fingerprint == schema_fingerprint
            && self.canonical_artifact.ptr_eq(canonical_artifact)
            && self.validation_certificate.matches_materialized_identity(
                canonical_artifact,
                canonical_fingerprint,
                canonical_serialized_len,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
            && self.localized_text_index.as_ref().is_none_or(|index| {
                index.matches_materialized_identity(
                    &self.validation_certificate,
                    canonical_artifact,
                    canonical_fingerprint,
                    canonical_serialized_len,
                    resource_limits,
                    schema_fingerprint,
                    document_revision,
                    state_revision,
                    yrs_state_epoch,
                )
            })
    }

    #[cfg(test)]
    pub(crate) fn materialize_mutation_identity(&mut self) {
        self.validation_certificate.materialize_canonical_artifact();
        if let Some(index) = self.localized_text_index.as_mut() {
            index.materialize_canonical_fingerprint(&self.validation_certificate);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn capture_history_document_snapshot(
        &self,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        fragment_name: &str,
        scope: Option<&super::DocumentScope>,
        retained_bytes: HistoryDocumentSnapshotRetainedBytes,
    ) -> Arc<HistoryDocumentSnapshot> {
        Arc::new(HistoryDocumentSnapshot {
            document: self.document.clone(),
            canonical_artifact: self.canonical_artifact.clone(),
            position_map: self.position_map.clone(),
            rendered_text: self.rendered_text.clone(),
            rendered_scalars: self.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
            render_blocks: Arc::clone(&self.render_blocks),
            validation_certificate: self.validation_certificate.clone(),
            resource_limits: resource_limits.clone(),
            editing_limits: editing_limits.clone(),
            max_length,
            schema_fingerprint: Arc::from(self.schema_fingerprint.as_str()),
            fragment_name: Arc::from(fragment_name),
            scope: scope.cloned(),
            retained_bytes: retained_bytes.get(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore_history_document_snapshot<T: ReadTxn>(
        request_id: u64,
        snapshot: &HistoryDocumentSnapshot,
        admission: AdmittedHistoryCandidateRead,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        expected_relative_selection: &RelativeSelection,
        expected_resolved_selection: &ResolvedSelection,
        stored_marks: Option<Vec<Mark>>,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> OperationResult<Option<RestoredHistoryDocumentState>> {
        admission.validate_restoration(
            request_id,
            txn,
            fragment,
            snapshot,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            yrs_state_epoch,
            document_revision,
        )?;
        if {
            #[cfg(test)]
            {
                history_snapshot_semantic_fallback_forced(
                    HistorySnapshotSemanticFallbackForTest::RenderIdentity,
                )
            }
            #[cfg(not(test))]
            {
                false
            }
        } || snapshot.rendered_scalars < snapshot.position_map.total_scalars()
            || !snapshot
                .render_blocks
                .matches_identity(&snapshot.document, schema_fingerprint)
        {
            return Ok(None);
        }
        #[cfg(test)]
        if history_snapshot_semantic_fallback_forced(
            HistorySnapshotSemanticFallbackForTest::RelativeSelection,
        ) {
            return Ok(None);
        }
        let Some(relative_selection) = history_selection_to_relative(
            txn,
            fragment,
            expected_relative_selection,
            expected_resolved_selection,
            schema,
        ) else {
            return Ok(None);
        };
        #[cfg(test)]
        if history_snapshot_semantic_fallback_forced(
            HistorySnapshotSemanticFallbackForTest::ResolvedSelection,
        ) {
            return Ok(None);
        }
        let Some(resolved_selection) = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &snapshot.document,
            &snapshot.position_map,
            &snapshot.rendered_text,
        ) else {
            return Ok(None);
        };
        #[cfg(test)]
        if history_snapshot_semantic_fallback_forced(
            HistorySnapshotSemanticFallbackForTest::ResolvedMismatch,
        ) {
            return Ok(None);
        }
        if &resolved_selection != expected_resolved_selection {
            return Ok(None);
        }
        let mut validation_certificate = snapshot.validation_certificate.clone();
        validation_certificate.document_revision = document_revision;
        validation_certificate.state_revision = state_revision;
        validation_certificate.yrs_state_epoch = yrs_state_epoch;
        let capability = admission.mint_capability(request_id, txn, fragment)?;
        let (mutation_lookup_seed, candidate_publication) =
            capability.prepare_unavailable_placeholder(request_id)?;
        let state = Self {
            document: snapshot.document.clone(),
            canonical_artifact: snapshot.canonical_artifact.clone(),
            position_map: snapshot.position_map.clone(),
            rendered_text: snapshot.rendered_text.clone(),
            rendered_scalars: snapshot.rendered_scalars,
            document_text_bytes: snapshot.document_text_bytes,
            document_node_count: snapshot.document_node_count,
            legacy_selection: resolved_to_legacy(&resolved_selection),
            relative_selection,
            resolved_selection,
            stored_marks,
            document_revision,
            state_revision,
            schema_fingerprint: schema_fingerprint.into(),
            render_blocks: Arc::clone(&snapshot.render_blocks),
            mutation_lookup_seed,
            validation_certificate,
            localized_text_index: None,
            active_state_certificate: None,
        };
        Ok(Some(RestoredHistoryDocumentState {
            state,
            candidate_publication,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_generic_derived_evidence(
        &self,
        request_id: u64,
        authority: &dyn DerivedStateAuthority,
        document: &Document,
        canonical_artifact: &CanonicalArtifact,
        derivations: &CompiledDocumentDerivations,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<FinalizedDerivedEvidence> {
        let installed = authority.installed();
        authority.lookup_seed(request_id).ok()?;
        if !self.document.shares_root_storage_with(&installed.document)
            || !self
                .canonical_artifact
                .ptr_eq(&installed.canonical_artifact)
        {
            return None;
        }
        let validation_certificate = DocumentValidationCertificate::mint(
            document,
            canonical_artifact,
            schema,
            resource_limits,
            schema_fingerprint,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )?;
        let localized_text_index = LocalizedTextLeafIndex::build(
            document,
            &derivations.position_map,
            &derivations.rendered_text,
            &validation_certificate,
            resource_limits,
            schema,
        );
        Some(FinalizedDerivedEvidence {
            validation_certificate,
            localized_text_index,
        })
    }

    pub(crate) fn clone_with_fallible_localized_index(&self) -> Self {
        let localized_text_index = self.localized_text_index.as_ref().and_then(|index| {
            index.try_clone(self.validation_certificate.resource_limits.max_input_bytes)
        });
        Self {
            document: self.document.clone(),
            canonical_artifact: self.canonical_artifact.clone(),
            position_map: self.position_map.clone(),
            rendered_text: self.rendered_text.clone(),
            rendered_scalars: self.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
            legacy_selection: self.legacy_selection.clone(),
            relative_selection: self.relative_selection.clone(),
            resolved_selection: self.resolved_selection.clone(),
            stored_marks: self.stored_marks.clone(),
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            schema_fingerprint: self.schema_fingerprint.clone(),
            render_blocks: Arc::clone(&self.render_blocks),
            mutation_lookup_seed: Arc::clone(&self.mutation_lookup_seed),
            validation_certificate: self.validation_certificate.clone(),
            localized_text_index,
            active_state_certificate: None,
        }
    }

    pub(crate) fn reseal_state_revision(&mut self, state_revision: u64) {
        self.state_revision = state_revision;
        self.validation_certificate
            .reseal_state_revision(state_revision);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn initialize<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        Self::initialize_with_local_state(
            document,
            canonical_artifact,
            txn,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            None,
            None,
            None,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    }

    /// Initializes a history candidate from the exact local state attached to
    /// the StackItem Yrs popped. Unlike ordinary document initialization, this
    /// does not first manufacture and resolve a default selection only to
    /// clone the complete cache and replace that selection immediately.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_history<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        relative_selection: RelativeSelection,
        stored_marks: Option<Vec<Mark>>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        Self::initialize_with_local_state(
            document,
            canonical_artifact,
            txn,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            Some(relative_selection),
            stored_marks,
            None,
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn initialize_validated_candidate<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        validated_candidate: ValidatedCandidateContext<'_>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        Self::initialize_with_local_state(
            document,
            canonical_artifact,
            txn,
            fragment,
            schema,
            resource_limits,
            editing_limits,
            max_length,
            schema_fingerprint,
            None,
            None,
            Some(validated_candidate),
            document_revision,
            state_revision,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn initialize_with_local_state<T: ReadTxn>(
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        schema_fingerprint: &str,
        initial_relative_selection: Option<RelativeSelection>,
        stored_marks: Option<Vec<Mark>>,
        validated_candidate: Option<ValidatedCandidateContext<'_>>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        if canonical_artifact.schema_fingerprint() != schema_fingerprint
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
        {
            return None;
        }
        let admitted_validation = validated_candidate.as_ref().and_then(|validated| {
            validated.evidence.admitted_validation_report(
                &document,
                &canonical_artifact,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                validated.canonical_schema,
                validated.fragment_name,
                txn,
                fragment,
                validated.engine_epoch,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
        });
        // Render-block construction admits every renderer arithmetic domain,
        // including ordered-list indices. It must happen before PositionMap,
        // whose marker-length walk assumes that admission has succeeded.
        let render_blocks = Arc::new(if let Some(validation) = admitted_validation {
            crate::render::incremental::CachedRenderBlocks::build_validated(
                &document,
                schema,
                resource_limits,
                schema_fingerprint,
                validation.stats.node_count,
                validation.stats.max_depth,
            )
            .ok()?
        } else {
            crate::render::incremental::CachedRenderBlocks::build(
                &document,
                schema,
                resource_limits,
            )
            .ok()?
        });
        if !render_blocks.matches_identity(&document, schema_fingerprint) {
            return None;
        }
        let position_map = PositionMap::build(&document, schema);
        let rendered_text = if admitted_validation.is_some() {
            render_blocks.rendered_text(schema)
        } else {
            crate::render::rendered_text(&document, schema)
        };
        let rendered_scalars = u32::try_from(rendered_text.chars().count()).ok()?;
        #[cfg(test)]
        let rendered_scalars = FORCE_INITIALIZE_SCALAR_MISMATCH.with(|force| {
            if force.get() {
                rendered_scalars.saturating_sub(1)
            } else {
                rendered_scalars
            }
        });
        // Generic non-text-block containers may render direct inline content
        // that is intentionally outside the addressable PositionMap domain.
        // Cache construction only requires that every mapped scalar fits in
        // the rendered buffer; position-dependent compilation separately
        // requires exact domain parity before using the two views together.
        if rendered_scalars < position_map.total_scalars() {
            return None;
        }
        let document_text_bytes = canonical_artifact.text_utf8_bytes();
        let document_node_count = admitted_validation.map_or_else(
            || crate::editor_state::document_node_count(document.root()),
            |validation| validation.stats.node_count,
        );
        let relative_selection = initial_relative_selection.unwrap_or_else(|| {
            let selection = (0..position_map.block_count())
                .filter_map(|index| position_map.block(index))
                .find(|block| !block.is_void_block)
                .map(|block| Selection::cursor(block.doc_start))
                .or_else(|| {
                    position_map
                        .block(0)
                        .map(|block| Selection::node(block.doc_start))
                })
                .unwrap_or_else(Selection::all);
            operation_result_to_relative(txn, fragment, &selection, schema)
        });
        let resolved_selection = resolve_selection(
            txn,
            fragment,
            &relative_selection,
            schema,
            &document,
            &position_map,
            &rendered_text,
        )?;
        let legacy_selection = resolved_to_legacy(&resolved_selection);
        let mutation_lookup_seed = if admitted_validation.is_some() {
            super::mutation::MutationLookupSeed::unavailable_for_validated_import(
                txn,
                fragment,
                &document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
        } else {
            super::mutation::MutationLookupSeed::build(
                0,
                txn,
                fragment,
                schema,
                &document,
                resource_limits,
                editing_limits,
                max_length,
                schema_fingerprint,
                yrs_state_epoch,
                document_revision,
            )
            .ok()?
        };
        let mutation_lookup_seed =
            Arc::new(mutation_lookup_seed.with_canonical_artifact(&canonical_artifact));
        let validation_certificate = if let Some(validation) = admitted_validation {
            DocumentValidationCertificate::from_report(
                validation,
                &canonical_artifact,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )
        } else {
            DocumentValidationCertificate::mint(
                &document,
                &canonical_artifact,
                schema,
                resource_limits,
                schema_fingerprint,
                document_revision,
                state_revision,
                yrs_state_epoch,
            )?
        };
        let localized_text_index = LocalizedTextLeafIndex::build(
            &document,
            &position_map,
            &rendered_text,
            &validation_certificate,
            resource_limits,
            schema,
        );
        Some(Self {
            document,
            canonical_artifact,
            position_map,
            rendered_text,
            rendered_scalars,
            document_text_bytes,
            document_node_count,
            legacy_selection,
            relative_selection,
            resolved_selection,
            stored_marks,
            document_revision,
            state_revision,
            schema_fingerprint: schema_fingerprint.into(),
            render_blocks,
            mutation_lookup_seed,
            validation_certificate,
            localized_text_index,
            active_state_certificate: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn after_document_change<T: ReadTxn>(
        &self,
        document: Document,
        canonical_artifact: CanonicalArtifact,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        render_blocks: Arc<crate::render::incremental::CachedRenderBlocks>,
        prepared_derivations: Option<CompiledDocumentDerivations>,
        step_map: &StepMap,
        update_mode: UpdateMode,
        affected_top_level_blocks: &[usize],
        explicit_selection: Option<&RelativeSelection>,
        preserved_fallback: Option<&Selection>,
        strict_fallback_affinity: bool,
        promoted_mutation_lookup_seed: Option<Arc<super::mutation::MutationLookupSeed>>,
        finalized_selection: Option<FinalizedSelectionState>,
        finalized_derived_evidence: Option<FinalizedDerivedEvidence>,
        document_revision: u64,
        state_revision: u64,
        yrs_state_epoch: u64,
    ) -> Option<Self> {
        if canonical_artifact.schema_fingerprint() != schema_fingerprint
            || canonical_artifact.format_version()
                != super::canonical::CANONICAL_ARTIFACT_FORMAT_VERSION
            || !render_blocks.matches_identity(&document, schema_fingerprint)
        {
            return None;
        }
        let canonical_document_text_bytes = canonical_artifact.text_utf8_bytes();
        let (
            position_map,
            rendered_text,
            rendered_scalars,
            prepared_document_text_bytes,
            document_node_count,
        ) = if let Some(prepared) = prepared_derivations {
            (
                prepared.position_map,
                prepared.rendered_text,
                prepared.rendered_scalars,
                prepared.document_text_bytes,
                prepared.document_node_count,
            )
        } else {
            let mut position_map = self.position_map.clone();
            record_preview_position_map_derivation();
            let update_mode = if affected_top_level_blocks.is_empty() && self.document != document {
                UpdateMode::Rebuild
            } else {
                update_mode
            };
            position_map.update(step_map, &self.document, &document, update_mode, schema);
            position_map.compact();
            record_preview_rendered_text_derivation();
            let rendered_text = crate::render::rendered_text(&document, schema);
            let rendered_scalars = u32::try_from(rendered_text.chars().count()).ok()?;
            let document_node_count = crate::editor_state::document_node_count(document.root());
            (
                position_map,
                rendered_text,
                rendered_scalars,
                canonical_document_text_bytes,
                document_node_count,
            )
        };
        debug_assert_eq!(prepared_document_text_bytes, canonical_document_text_bytes);
        let document_text_bytes = canonical_document_text_bytes;
        if rendered_scalars < position_map.total_scalars() {
            return None;
        }

        let (relative_selection, resolved_selection, legacy_selection) =
            if let Some(finalized) = finalized_selection {
                record_prewrite_selection_proof_install();
                finalized.into_parts()
            } else {
                let mut relative_selection = explicit_selection
                    .cloned()
                    .unwrap_or_else(|| self.relative_selection.clone());
                let mut resolved_selection = resolve_selection(
                    txn,
                    fragment,
                    &relative_selection,
                    schema,
                    &document,
                    &position_map,
                    &rendered_text,
                );
                if resolved_selection.is_none() {
                    let fallback = preserved_fallback?;
                    relative_selection = preserve_with_mapped_fallback(
                        txn,
                        fragment,
                        &relative_selection,
                        fallback,
                        schema,
                        strict_fallback_affinity,
                    );
                    resolved_selection = resolve_selection(
                        txn,
                        fragment,
                        &relative_selection,
                        schema,
                        &document,
                        &position_map,
                        &rendered_text,
                    );
                }
                let resolved_selection = resolved_selection?;
                let legacy_selection = resolved_to_legacy(&resolved_selection);
                (relative_selection, resolved_selection, legacy_selection)
            };
        let mutation_lookup_seed = if let Some(promoted) = promoted_mutation_lookup_seed {
            promoted
        } else {
            Arc::new(
                super::mutation::MutationLookupSeed::build(
                    0,
                    txn,
                    fragment,
                    schema,
                    &document,
                    resource_limits,
                    editing_limits,
                    max_length,
                    schema_fingerprint,
                    yrs_state_epoch,
                    document_revision,
                )
                .ok()?
                .with_canonical_artifact(&canonical_artifact),
            )
        };
        let mutation_lookup_seed =
            if mutation_lookup_seed.matches_canonical_artifact(&canonical_artifact) {
                mutation_lookup_seed
            } else {
                Arc::new(
                    mutation_lookup_seed
                        .as_ref()
                        .clone()
                        .with_canonical_artifact(&canonical_artifact),
                )
            };
        let (validation_certificate, localized_text_index) =
            if let Some(finalized) = finalized_derived_evidence {
                (
                    finalized.validation_certificate,
                    finalized.localized_text_index,
                )
            } else {
                let validation_certificate = DocumentValidationCertificate::mint(
                    &document,
                    &canonical_artifact,
                    schema,
                    resource_limits,
                    schema_fingerprint,
                    document_revision,
                    state_revision,
                    yrs_state_epoch,
                )?;
                let localized_text_index = LocalizedTextLeafIndex::build(
                    &document,
                    &position_map,
                    &rendered_text,
                    &validation_certificate,
                    resource_limits,
                    schema,
                );
                (validation_certificate, localized_text_index)
            };

        Some(Self {
            document,
            canonical_artifact,
            position_map,
            rendered_text,
            rendered_scalars,
            document_text_bytes,
            document_node_count,
            legacy_selection,
            relative_selection,
            resolved_selection,
            stored_marks: self.stored_marks.clone(),
            document_revision,
            state_revision,
            schema_fingerprint: schema_fingerprint.into(),
            render_blocks,
            mutation_lookup_seed,
            validation_certificate,
            localized_text_index,
            active_state_certificate: None,
        })
    }

    pub fn update_selection_state(
        &mut self,
        relative_selection: RelativeSelection,
        resolved_selection: ResolvedSelection,
        stored_marks: Option<Vec<Mark>>,
        state_revision: u64,
    ) {
        self.legacy_selection = resolved_to_legacy(&resolved_selection);
        self.relative_selection = relative_selection;
        self.resolved_selection = resolved_selection;
        self.stored_marks = stored_marks;
        self.reseal_state_revision(state_revision);
        self.clear_active_state_certificate();
    }

    pub fn resolve_relative_selection<T: ReadTxn>(
        &self,
        relative_selection: &RelativeSelection,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema: &Schema,
    ) -> Option<ResolvedSelection> {
        resolve_selection(
            txn,
            fragment,
            relative_selection,
            schema,
            &self.document,
            &self.position_map,
            &self.rendered_text,
        )
    }

    pub fn legacy_selection(&self) -> Selection {
        self.legacy_selection.clone()
    }

    pub fn compilation_view(&self) -> CachedCompilationView<'_> {
        CachedCompilationView {
            document: &self.document,
            position_map: &self.position_map,
            rendered_text: &self.rendered_text,
            rendered_scalars: self.rendered_scalars,
            document_text_bytes: self.document_text_bytes,
            document_node_count: self.document_node_count,
            selection: &self.legacy_selection,
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            schema_fingerprint: &self.schema_fingerprint,
            canonical_artifact: &self.canonical_artifact,
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn localized_insert_admission_for_test(
        &self,
        document_position: u32,
        text: &str,
        marks: &[Mark],
        schema: &Schema,
        resource_limits: &ResourceLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<LocalizedInsertAdmission> {
        let schema_fingerprint = crate::schema::schema_fingerprint(schema);
        self.build_localized_insert_admission(
            LocalizedInsertAdmissionRequest {
                request_id: 0,
                base_document_revision: self.document_revision,
                origin: super::TransactionOrigin::LocalInput,
                inserted_at: super::RevisionedPosition {
                    offset: document_position,
                    kind: super::EditorOffsetKind::Scalar,
                    affinity: super::Affinity::After,
                },
                document_position,
                text,
                marks,
                selection_intent: super::SelectionIntent::UseOperationResult,
                history_policy: super::HistoryPolicy::Auto,
            },
            &schema_fingerprint,
            resource_limits,
            &crate::yrs_engine::EditingLimits::default(),
            max_length,
            yrs_state_epoch,
            &self.mutation_lookup_seed,
            None,
        )
    }

    /// Callers may invoke this only after envelope admission, cached-view
    /// validation, document-byte charging, and Yrs scan admission.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_existing_text_insert<T: ReadTxn>(
        &self,
        transaction: &super::TypedTransaction,
        allow_prepared_command_boundary: bool,
        document_position: u32,
        txn: &T,
        fragment: &XmlFragmentRef,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<LocalizedInsertAdmission> {
        let authority = super::prepared_admission::InstalledDerivedStateAuthority::new(self);
        let lookup_seed =
            DerivedStateAuthority::lookup_seed(&authority, transaction.request_id).ok()?;
        self.admit_existing_text_insert_with_authority(
            transaction,
            allow_prepared_command_boundary,
            document_position,
            txn,
            fragment,
            lookup_seed,
            authority.materialized_identity(),
            schema_fingerprint,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn admit_existing_text_insert_with_authority<T: ReadTxn>(
        &self,
        transaction: &super::TypedTransaction,
        allow_prepared_command_boundary: bool,
        document_position: u32,
        txn: &T,
        fragment: &XmlFragmentRef,
        lookup_seed: &Arc<super::mutation::MutationLookupSeed>,
        identity: Option<&super::prepared_admission::MaterializedMutationIdentity>,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
    ) -> Option<LocalizedInsertAdmission> {
        if transaction.base_document_revision != self.document_revision
            || transaction.selection_intent != super::SelectionIntent::UseOperationResult
            || !(transaction.history_policy == super::HistoryPolicy::Auto
                || (allow_prepared_command_boundary
                    && transaction.origin == super::TransactionOrigin::LocalCommand
                    && transaction.history_policy == super::HistoryPolicy::Boundary))
            || !matches!(
                transaction.origin,
                super::TransactionOrigin::LocalInput
                    | super::TransactionOrigin::LocalCommand
                    | super::TransactionOrigin::LocalApi
            )
            || !self
                .render_blocks
                .matches_identity(&self.document, &self.schema_fingerprint)
            || !lookup_seed.matches(
                txn,
                fragment,
                &self.document,
                resource_limits,
                editing_limits,
                max_length,
                &self.schema_fingerprint,
                yrs_state_epoch,
                self.document_revision,
            )
        {
            return None;
        }
        let [super::TypedOperation::InsertText { at, text, marks }] =
            transaction.operations.as_slice()
        else {
            return None;
        };
        self.build_localized_insert_admission(
            LocalizedInsertAdmissionRequest {
                request_id: transaction.request_id,
                base_document_revision: transaction.base_document_revision,
                origin: transaction.origin,
                inserted_at: *at,
                document_position,
                text,
                marks,
                selection_intent: transaction.selection_intent.clone(),
                history_policy: transaction.history_policy,
            },
            schema_fingerprint,
            resource_limits,
            editing_limits,
            max_length,
            yrs_state_epoch,
            lookup_seed,
            identity,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_localized_insert_admission(
        &self,
        request: LocalizedInsertAdmissionRequest<'_>,
        schema_fingerprint: &str,
        resource_limits: &ResourceLimits,
        editing_limits: &super::EditingLimits,
        max_length: Option<u32>,
        yrs_state_epoch: u64,
        lookup_seed: &Arc<super::mutation::MutationLookupSeed>,
        identity: Option<&super::prepared_admission::MaterializedMutationIdentity>,
    ) -> Option<LocalizedInsertAdmission> {
        #[cfg(test)]
        LOCALIZED_INSERT_ADMISSION_WORK
            .set(LOCALIZED_INSERT_ADMISSION_WORK.get().saturating_add(1));
        let LocalizedInsertAdmissionRequest {
            request_id,
            base_document_revision,
            origin,
            inserted_at,
            document_position,
            text,
            marks,
            selection_intent,
            history_policy,
        } = request;
        let identity_matches = identity.is_none_or(|identity| {
            self.matches_materialized_mutation_identity(
                &self.canonical_artifact,
                identity.canonical_fingerprint,
                identity.canonical_serialized_len,
                resource_limits,
                &self.schema_fingerprint,
                self.document_revision,
                self.state_revision,
                yrs_state_epoch,
            )
        });
        if text.is_empty()
            || schema_fingerprint != self.schema_fingerprint
            || !identity_matches
            || (identity.is_none()
                && !self.validation_certificate.matches(
                    &self.canonical_artifact,
                    resource_limits,
                    &self.schema_fingerprint,
                    self.document_revision,
                    self.state_revision,
                    yrs_state_epoch,
                ))
        {
            return None;
        }
        let localized_text_index = self.localized_text_index.as_ref()?;
        if identity.is_none() && !localized_text_index.matches(&self.validation_certificate) {
            return None;
        }
        let leaf = localized_text_index
            .strict_inside(document_position)
            .copied()?;
        let block = self.position_map.block(leaf.block_index)?;
        let affected_top_level_index = usize::try_from(*block.node_path.first()?).ok()?;
        let live_leaf = leaf.resolve(&self.document, &self.position_map)?;
        let live_text = live_leaf.text_str()?;
        if <[u8; 32]>::from(sha2::Sha256::digest(live_text.as_bytes())) != leaf.text_sha256
            || canonical_marks_sha256(live_leaf.marks())? != leaf.marks_sha256
            || live_leaf.marks() != marks
            || live_leaf.node_size() != leaf.text_scalars
            || u32::try_from(live_leaf.text_str()?.encode_utf16().count()).ok()? != leaf.text_utf16
            || live_leaf.text_str()?.len() != leaf.text_utf8_bytes
        {
            return None;
        }
        let inserted_scalars = u32::try_from(text.chars().count()).ok()?;
        let inserted_utf16 = u32::try_from(text.encode_utf16().count()).ok()?;
        let next_raw_text_scalars = self
            .validation_certificate
            .raw_text_scalars
            .checked_add(u64::from(inserted_scalars))?;
        if max_length.is_some_and(|limit| next_raw_text_scalars > u64::from(limit)) {
            return None;
        }
        let next_raw_text_utf8_bytes = self
            .validation_certificate
            .raw_text_utf8_bytes
            .checked_add(text.len())?;
        let canonical_serialized_len = identity.map_or(
            self.validation_certificate.canonical_serialized_len,
            |identity| identity.canonical_serialized_len,
        );
        let canonical_fingerprint = identity.map_or_else(
            || self.validation_certificate.canonical_fingerprint,
            |identity| identity.canonical_fingerprint,
        );
        let escaped_limit = editing_limits
            .max_derived_output_bytes
            .checked_sub(canonical_serialized_len)?;
        let inserted_escaped_json_bytes = checked_json_string_body_len(text, escaped_limit)?;
        let next_canonical_serialized_len =
            canonical_serialized_len.checked_add(inserted_escaped_json_bytes)?;
        if next_canonical_serialized_len > editing_limits.max_derived_output_bytes {
            return None;
        }
        let scalar_at = self
            .position_map
            .doc_to_scalar(document_position, &self.document);
        let utf16_at = scalar_offset_to_utf16(&self.rendered_text, scalar_at)?;
        let next_document = document_position.checked_add(inserted_scalars)?;
        let next_scalar = scalar_at.checked_add(inserted_scalars)?;
        let next_utf16 = utf16_at.checked_add(inserted_utf16)?;
        let operation_result = ResolvedSelection::Text {
            anchor: ResolvedPoint {
                document: next_document,
                scalar: next_scalar,
                utf16: next_utf16,
            },
            head: ResolvedPoint {
                document: next_document,
                scalar: next_scalar,
                utf16: next_utf16,
            },
        };
        let history_undo_units = u64::from(inserted_utf16);
        if history_undo_units > editing_limits.max_undo_retained_units {
            return None;
        }
        Some(LocalizedInsertAdmission {
            leaf,
            block_path_len: block.node_path.len(),
            block_path_sha256: node_path_sha256(&block.node_path),
            affected_top_level_index,
            inserted_scalars,
            inserted_utf8_bytes: text.len(),
            inserted_utf16,
            inserted_escaped_json_bytes,
            next_raw_text_scalars,
            next_raw_text_utf8_bytes,
            next_canonical_serialized_len,
            next_rendered_scalars: self.rendered_scalars.checked_add(inserted_scalars)?,
            operation_result,
            history_undo_units,
            document_revision: self.document_revision,
            state_revision: self.state_revision,
            yrs_state_epoch,
            selection: self.resolved_selection.clone(),
            relative_selection: self.relative_selection.clone(),
            stored_marks_sha256: match self.stored_marks.as_deref() {
                Some(stored_marks) => Some(canonical_marks_sha256(stored_marks)?),
                None => None,
            },
            canonical_fingerprint,
            validation_certificate: self.validation_certificate.clone(),
            request_id,
            base_document_revision,
            origin,
            inserted_at,
            inserted_document_position: document_position,
            inserted_text_sha256: sha2::Sha256::digest(text.as_bytes()).into(),
            inserted_marks_sha256: canonical_marks_sha256(marks)?,
            selection_intent,
            history_policy,
            max_length,
            max_operations_per_transaction: editing_limits.max_operations_per_transaction,
            max_undo_groups: editing_limits.max_undo_groups,
            max_derived_output_bytes: editing_limits.max_derived_output_bytes,
            max_undo_retained_units: editing_limits.max_undo_retained_units,
            render_seal: Arc::clone(&self.render_blocks),
            lookup_seal: Arc::clone(lookup_seed),
        })
    }
}

fn checked_json_string_body_len(text: &str, limit: usize) -> Option<usize> {
    let mut bytes = 0usize;
    for character in text.chars() {
        let amount = match character {
            '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
            '\u{0000}'..='\u{001f}' => 6,
            other => other.len_utf8(),
        };
        bytes = bytes.checked_add(amount)?;
        if bytes > limit {
            return None;
        }
    }
    Some(bytes)
}

pub(crate) fn stored_marks_after_selection_change(
    current: Option<&[Mark]>,
    before: &ResolvedSelection,
    after: &ResolvedSelection,
    _document: &Document,
    schema: &Schema,
) -> Option<Vec<Mark>> {
    let current = current?;
    if before != after || !is_collapsed_text(after) {
        return None;
    }
    Some(canonical_marks(current, schema))
}

pub(crate) fn apply_stored_mark_operation(
    marks: &mut Vec<Mark>,
    operation: &TypedOperation,
    schema: &Schema,
) -> OperationResult<bool> {
    match operation {
        TypedOperation::AddMark { mark, .. } => {
            if let Some(existing) = marks
                .iter()
                .find(|candidate| candidate.mark_type() == mark.mark_type())
            {
                if existing != mark {
                    return Err(OperationError::operation_invalid(
                        0,
                        0,
                        "mark",
                        "AddMark conflicts with an existing same-type mark; use ReplaceMark",
                    ));
                }
                return Ok(false);
            }
            insert_mark_ranked(marks, mark.clone(), schema);
            Ok(true)
        }
        TypedOperation::RemoveMark { mark_type, .. } => {
            let previous_len = marks.len();
            marks.retain(|candidate| candidate.mark_type() != mark_type);
            Ok(marks.len() != previous_len)
        }
        TypedOperation::ReplaceMark { mark, .. } => {
            if marks
                .iter()
                .find(|candidate| candidate.mark_type() == mark.mark_type())
                == Some(mark)
            {
                return Ok(false);
            }
            marks.retain(|candidate| candidate.mark_type() != mark.mark_type());
            insert_mark_ranked(marks, mark.clone(), schema);
            Ok(true)
        }
        _ => Err(OperationError::engine_invariant_failed(
            0,
            None,
            "stored mark transition received a non-mark operation",
        )),
    }
}

pub(crate) fn resolved_from_legacy(
    document: &Document,
    selection: &Selection,
    schema: &Schema,
) -> Option<ResolvedSelection> {
    record_preview_position_map_derivation();
    let position_map = PositionMap::build(document, schema);
    record_preview_rendered_text_derivation();
    let rendered = crate::render::rendered_text(document, schema);
    resolved_from_legacy_with_view(document, selection, schema, &position_map, &rendered)
}

pub(crate) fn resolved_from_legacy_with_view(
    document: &Document,
    selection: &Selection,
    schema: &Schema,
    position_map: &PositionMap,
    rendered: &str,
) -> Option<ResolvedSelection> {
    let point = |document_position| {
        let scalar = position_map.doc_to_scalar(document_position, document);
        Some(ResolvedPoint {
            document: document_position,
            scalar,
            utf16: scalar_offset_to_utf16(rendered, scalar)?,
        })
    };
    match selection {
        Selection::Text { anchor, head } => Some(ResolvedSelection::Text {
            anchor: point(*anchor)?,
            head: point(*head)?,
        }),
        Selection::Node { pos } if selectable_void_at(document.root(), *pos, 0, schema) => {
            Some(ResolvedSelection::Node { at: point(*pos)? })
        }
        Selection::Node { .. } => None,
        Selection::All => Some(ResolvedSelection::All),
    }
}

fn is_collapsed_text(selection: &ResolvedSelection) -> bool {
    matches!(selection, ResolvedSelection::Text { anchor, head } if anchor.document == head.document)
}

pub(crate) fn canonical_marks(marks: &[Mark], schema: &Schema) -> Vec<Mark> {
    let mut marks = marks.to_vec();
    marks.sort_by(|left, right| {
        schema
            .mark_rank(left.mark_type())
            .unwrap_or(usize::MAX)
            .cmp(&schema.mark_rank(right.mark_type()).unwrap_or(usize::MAX))
            .then_with(|| left.mark_type().cmp(right.mark_type()))
    });
    marks
}

fn insert_mark_ranked(marks: &mut Vec<Mark>, mark: Mark, schema: &Schema) {
    let rank = schema.mark_rank(mark.mark_type()).unwrap_or(usize::MAX);
    let index = marks
        .iter()
        .position(|candidate| {
            schema
                .mark_rank(candidate.mark_type())
                .unwrap_or(usize::MAX)
                > rank
        })
        .unwrap_or(marks.len());
    marks.insert(index, mark);
}

pub(crate) fn marks_at_position(document: &Document, position: u32) -> Vec<Mark> {
    crate::editor_state::marks_at_position(document, position)
}

fn preserve_with_mapped_fallback<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    current: &RelativeSelection,
    mapped: &Selection,
    schema: &Schema,
    strict_affinity: bool,
) -> RelativeSelection {
    let point = |current: &RelativePoint, mapped_position| {
        if relative_point_to_doc_pos(txn, fragment, current, schema).is_some() {
            return current.clone();
        }
        if let Some(point) =
            doc_pos_to_relative_point(txn, fragment, mapped_position, current.affinity, schema)
        {
            return point;
        }
        assert!(
            !strict_affinity,
            "prevalidated explicit selection affinity must remain exactly representable"
        );
        let sticky = cursor_sticky_index_from_doc_pos(txn, fragment, mapped_position, true, schema)
            .expect("compiler-normalized mapped fallback has a Yrs association");
        RelativePoint {
            affinity: affinity_from_assoc(sticky.assoc),
            sticky,
        }
    };
    match (current, mapped) {
        (
            RelativeSelection::Text { anchor, head },
            Selection::Text {
                anchor: mapped_anchor,
                head: mapped_head,
            },
        ) => RelativeSelection::Text {
            anchor: point(anchor, *mapped_anchor),
            head: point(head, *mapped_head),
        },
        (RelativeSelection::Node { point: current }, Selection::Node { pos }) => {
            RelativeSelection::Node {
                point: point(current, *pos),
            }
        }
        (RelativeSelection::All, Selection::All) => RelativeSelection::All,
        _ => operation_result_to_relative(txn, fragment, mapped, schema),
    }
}

pub(crate) fn resolve_selection<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    relative_selection: &RelativeSelection,
    schema: &Schema,
    document: &Document,
    position_map: &PositionMap,
    rendered_text: &str,
) -> Option<ResolvedSelection> {
    #[cfg(test)]
    RELATIVE_SELECTION_RESOLUTION_TRAVERSALS.set(
        RELATIVE_SELECTION_RESOLUTION_TRAVERSALS
            .get()
            .saturating_add(1),
    );
    let selection = relative_selection_to_selection(
        txn,
        fragment,
        relative_selection,
        schema,
        document,
        position_map,
    )?;
    let point = |document_position| {
        let scalar = position_map.doc_to_scalar(document_position, document);
        Some(ResolvedPoint {
            document: document_position,
            scalar,
            utf16: scalar_offset_to_utf16(rendered_text, scalar)?,
        })
    };
    match selection {
        Selection::Text { anchor, head } => Some(ResolvedSelection::Text {
            anchor: point(anchor)?,
            head: point(head)?,
        }),
        Selection::Node { pos } if selectable_void_at(document.root(), pos, 0, schema) => {
            Some(ResolvedSelection::Node { at: point(pos)? })
        }
        Selection::Node { .. } => None,
        Selection::All => Some(ResolvedSelection::All),
    }
}

pub(crate) fn operation_result_to_relative<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    selection: &Selection,
    schema: &Schema,
) -> RelativeSelection {
    #[cfg(test)]
    OPERATION_RESULT_RELATIVE_TRAVERSALS
        .set(OPERATION_RESULT_RELATIVE_TRAVERSALS.get().saturating_add(1));
    let before = |position| {
        doc_pos_to_sticky_index(txn, fragment, position, Assoc::Before, schema).expect(
            "compiler-normalized operation-result position has an exact Before Yrs association",
        )
    };
    match selection {
        Selection::Text { anchor, head } if anchor == head => {
            let sticky = cursor_sticky_index_from_doc_pos(txn, fragment, *anchor, true, schema)
                .expect("compiler-normalized cursor has a Yrs association");
            let point = RelativePoint {
                affinity: affinity_from_assoc(sticky.assoc),
                sticky,
            };
            RelativeSelection::Text {
                anchor: point.clone(),
                head: point,
            }
        }
        Selection::Text { anchor, head } => RelativeSelection::Text {
            anchor: RelativePoint {
                sticky: before(*anchor),
                affinity: Affinity::Before,
            },
            head: RelativePoint {
                sticky: before(*head),
                affinity: Affinity::Before,
            },
        },
        Selection::Node { pos } => RelativeSelection::Node {
            point: RelativePoint {
                sticky: before(*pos),
                affinity: Affinity::Before,
            },
        },
        Selection::All => RelativeSelection::All,
    }
}

pub(crate) fn history_selection_to_relative<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    expected_relative: &RelativeSelection,
    expected_resolved: &ResolvedSelection,
    schema: &Schema,
) -> Option<RelativeSelection> {
    #[cfg(test)]
    OPERATION_RESULT_RELATIVE_TRAVERSALS
        .set(OPERATION_RESULT_RELATIVE_TRAVERSALS.get().saturating_add(1));
    let point = |position, captured: &RelativePoint| {
        doc_pos_to_relative_point(txn, fragment, position, captured.affinity, schema)
    };
    match (expected_relative, expected_resolved) {
        (
            RelativeSelection::Text {
                anchor: captured_anchor,
                head: captured_head,
            },
            ResolvedSelection::Text { anchor, head },
        ) => Some(RelativeSelection::Text {
            anchor: point(anchor.document, captured_anchor)?,
            head: point(head.document, captured_head)?,
        }),
        (RelativeSelection::Node { point: captured }, ResolvedSelection::Node { at }) => {
            Some(RelativeSelection::Node {
                point: point(at.document, captured)?,
            })
        }
        (RelativeSelection::All, ResolvedSelection::All) => Some(RelativeSelection::All),
        _ => None,
    }
}

pub(crate) fn exact_point_is_representable<T: ReadTxn>(
    txn: &T,
    fragment: &XmlFragmentRef,
    position: u32,
    point: &RelativePoint,
    schema: &Schema,
) -> bool {
    doc_pos_to_relative_point(txn, fragment, position, point.affinity, schema).is_some()
}

pub(crate) fn resolved_to_legacy(selection: &ResolvedSelection) -> Selection {
    match selection {
        ResolvedSelection::Text { anchor, head } => Selection::text(anchor.document, head.document),
        ResolvedSelection::Node { at } => Selection::node(at.document),
        ResolvedSelection::All => Selection::all(),
    }
}

fn affinity_from_assoc(assoc: Assoc) -> Affinity {
    match assoc {
        Assoc::Before => Affinity::Before,
        Assoc::After => Affinity::After,
    }
}

#[cfg(test)]
#[path = "derived_state_tests.rs"]
mod tests;
