#[cfg(test)]
std::thread_local! {
    pub(super) static PREVIEW_POSITION_MAP_DERIVATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static PREVIEW_RENDERED_TEXT_DERIVATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static FORCE_INITIALIZE_SCALAR_MISMATCH: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    pub(super) static LOCALIZED_INDEX_BUILD_VISITS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_PATH_HOPS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_LOOKUP_COMPARISONS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_PATH_COPY_ELEMENTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_PATH_COMPARISON_ELEMENTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_PROMOTION_ATTEMPTS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_PROMOTION_SUCCESSES: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static LOCALIZED_INDEX_PROMOTION_DROPS: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static FORCE_LOCALIZED_INDEX_ALLOCATION_FAILURE: std::cell::Cell<bool> = const {
        std::cell::Cell::new(false)
    };
    pub(super) static FORCE_LOCALIZED_INDEX_ALLOCATION_STAGE: std::cell::Cell<Option<LocalizedIndexAllocationStage>> = const {
        std::cell::Cell::new(None)
    };
    pub(super) static FORCE_LOCALIZED_INDEX_BUDGET: std::cell::Cell<Option<usize>> = const {
        std::cell::Cell::new(None)
    };
    pub(super) static LOCALIZED_INSERT_ADMISSION_WORK: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static ACTIVE_STATE_CACHE_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_CACHE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_CACHE_FALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_GENERIC_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_CANDIDATE_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_CACHE_INSTALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_CACHE_DROPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_PUBLIC_RESULT_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ACTIVE_STATE_FULL_ASSEMBLIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static FORCE_ACTIVE_STATE_CACHE_ALLOCATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static FORCE_ACTIVE_STATE_CACHE_BUDGET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    pub(super) static FORCE_ACTIVE_STATE_CACHE_HIT_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static FORCE_ACTIVE_STATE_PUBLIC_MATERIALIZATION_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static OPERATION_RESULT_RELATIVE_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static RELATIVE_SELECTION_RESOLUTION_TRAVERSALS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PREWRITE_SELECTION_PROOF_ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PREWRITE_SELECTION_PROOF_FINALIZATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PREWRITE_SELECTION_PROOF_FALLBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PREWRITE_SELECTION_PROOF_INSTALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static FORCE_HISTORY_DOCUMENT_SNAPSHOT_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static FORCE_HISTORY_SNAPSHOT_SEMANTIC_FALLBACK: std::cell::Cell<Option<HistorySnapshotSemanticFallbackForTest>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) struct ForcedHistoryDocumentSnapshotFallback {
    pub(super) previous: bool,
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
    pub(super) previous: Option<HistorySnapshotSemanticFallbackForTest>,
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
pub(super) fn history_snapshot_semantic_fallback_forced(
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
pub(super) fn forced_localized_index_allocation_stage(
    stage: LocalizedIndexAllocationStage,
) -> bool {
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
