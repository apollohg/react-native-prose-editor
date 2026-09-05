use crate::yrs_engine;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompiledCommitPreparationStage {
    AllocationProbe,
    OperationPreparation,
    DocumentValidation,
    LookupTransition,
    HistoryReservation,
    HistoryUpdateEncoding,
    SelectionFinalization,
    DerivedStateBuild,
    HistorySnapshotConstruction,
}

#[cfg(test)]
impl CompiledCommitPreparationStage {
    const fn field_name(self) -> &'static str {
        match self {
            Self::AllocationProbe => "allocationProbe",
            Self::OperationPreparation => "operationPreparation",
            Self::DocumentValidation => "documentValidation",
            Self::LookupTransition => "lookupTransition",
            Self::HistoryReservation => "historyReservation",
            Self::HistoryUpdateEncoding => "historyUpdateEncoding",
            Self::SelectionFinalization => "selectionFinalization",
            Self::DerivedStateBuild => "derivedStateBuild",
            Self::HistorySnapshotConstruction => "historySnapshotConstruction",
        }
    }
}

#[cfg(test)]
std::thread_local! {
    pub(super) static COMPILED_COMMIT_STAGE_FAILPOINT: std::cell::Cell<Option<CompiledCommitPreparationStage>> = const { std::cell::Cell::new(None) };
    pub(super) static COMPILED_COMMIT_DURABLE_WRITE_OPENED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static COMPILED_COMMIT_AUTHORITY_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static COMPILED_COMMIT_LIVE_VIEWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PREPARED_CANDIDATE_CACHE_HITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static PREPARED_CANDIDATE_FULL_BOOTSTRAPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static CANDIDATE_BOUNDED_STATE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static IMPORT_CANDIDATE_STATE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static IMPORT_RECEIPT_STATE_DECODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static IMPORT_RECEIPT_SHA256_MINTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static IMPORT_RECEIPT_SHA256_MATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static COMMIT_CURRENT_STATE_ENCODINGS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static COMMIT_SEALED_STATE_REUSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static FAIL_QUARANTINED_UPDATE_RESERVATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(super) static FAIL_OUTBOUND_STAGING_COPY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(super) fn set_quarantined_update_reservation_failure_for_test(enabled: bool) {
    FAIL_QUARANTINED_UPDATE_RESERVATION.set(enabled);
}

#[cfg(test)]
pub(super) fn set_outbound_staging_copy_failure_for_test(enabled: bool) {
    FAIL_OUTBOUND_STAGING_COPY.set(enabled);
}

#[cfg(test)]
pub(super) fn reset_prepared_candidate_cache_counts_for_test() {
    PREPARED_CANDIDATE_CACHE_HITS.set(0);
    PREPARED_CANDIDATE_FULL_BOOTSTRAPS.set(0);
}

#[cfg(test)]
pub(super) fn take_prepared_candidate_cache_counts_for_test() -> (usize, usize) {
    let hits = PREPARED_CANDIDATE_CACHE_HITS.replace(0);
    let bootstraps = PREPARED_CANDIDATE_FULL_BOOTSTRAPS.replace(0);
    (hits, bootstraps)
}

#[cfg(test)]
pub(super) fn reset_encoded_state_reuse_counts_for_test() {
    IMPORT_CANDIDATE_STATE_ENCODINGS.set(0);
    COMMIT_CURRENT_STATE_ENCODINGS.set(0);
    COMMIT_SEALED_STATE_REUSES.set(0);
}

#[cfg(test)]
pub(super) fn reset_import_state_encoding_counts_for_test() {
    CANDIDATE_BOUNDED_STATE_ENCODINGS.set(0);
    IMPORT_CANDIDATE_STATE_ENCODINGS.set(0);
}

#[cfg(test)]
pub(super) fn take_import_state_encoding_counts_for_test() -> (usize, usize) {
    (
        CANDIDATE_BOUNDED_STATE_ENCODINGS.replace(0),
        IMPORT_CANDIDATE_STATE_ENCODINGS.replace(0),
    )
}

#[cfg(test)]
pub(super) fn reset_import_receipt_state_decodings_for_test() {
    IMPORT_RECEIPT_STATE_DECODINGS.set(0);
}

#[cfg(test)]
pub(super) fn take_import_receipt_state_decodings_for_test() -> usize {
    IMPORT_RECEIPT_STATE_DECODINGS.replace(0)
}

#[cfg(test)]
pub(super) fn reset_import_receipt_sha256_counts_for_test() {
    IMPORT_RECEIPT_SHA256_MINTS.set(0);
    IMPORT_RECEIPT_SHA256_MATCHES.set(0);
}

#[cfg(test)]
pub(super) fn take_import_receipt_sha256_counts_for_test() -> (usize, usize) {
    (
        IMPORT_RECEIPT_SHA256_MINTS.replace(0),
        IMPORT_RECEIPT_SHA256_MATCHES.replace(0),
    )
}

#[cfg(test)]
pub(super) fn take_encoded_state_reuse_counts_for_test() -> (usize, usize, usize) {
    (
        IMPORT_CANDIDATE_STATE_ENCODINGS.replace(0),
        COMMIT_CURRENT_STATE_ENCODINGS.replace(0),
        COMMIT_SEALED_STATE_REUSES.replace(0),
    )
}

#[cfg(test)]
pub(super) fn set_compiled_commit_stage_failpoint_for_test(
    stage: Option<CompiledCommitPreparationStage>,
) {
    COMPILED_COMMIT_STAGE_FAILPOINT.set(stage);
    COMPILED_COMMIT_DURABLE_WRITE_OPENED.set(false);
}

#[cfg(test)]
pub(super) fn begin_compiled_commit_preparation_for_test() {
    COMPILED_COMMIT_DURABLE_WRITE_OPENED.set(false);
    COMPILED_COMMIT_AUTHORITY_VALIDATIONS.set(0);
    COMPILED_COMMIT_LIVE_VIEWS.set(0);
}

#[cfg(test)]
pub(super) fn record_compiled_commit_authority_validation_for_test() {
    COMPILED_COMMIT_AUTHORITY_VALIDATIONS.set(
        COMPILED_COMMIT_AUTHORITY_VALIDATIONS
            .get()
            .saturating_add(1),
    );
}

#[cfg(test)]
pub(super) fn record_compiled_commit_live_view_for_test() {
    COMPILED_COMMIT_LIVE_VIEWS.set(COMPILED_COMMIT_LIVE_VIEWS.get().saturating_add(1));
}

#[cfg(test)]
pub(super) fn take_compiled_commit_authority_counts_for_test() -> (usize, usize) {
    (
        COMPILED_COMMIT_AUTHORITY_VALIDATIONS.replace(0),
        COMPILED_COMMIT_LIVE_VIEWS.replace(0),
    )
}

#[cfg(test)]
pub(super) fn mark_compiled_commit_durable_write_for_test() {
    COMPILED_COMMIT_DURABLE_WRITE_OPENED.set(true);
}

#[cfg(test)]
pub(super) fn check_compiled_commit_preparation_stage_for_test(
    request_id: u64,
    stage: CompiledCommitPreparationStage,
) -> yrs_engine::OperationResult<()> {
    let durable_write_opened = COMPILED_COMMIT_DURABLE_WRITE_OPENED.get();
    if durable_write_opened || COMPILED_COMMIT_STAGE_FAILPOINT.get() == Some(stage) {
        let phase = if durable_write_opened {
            "postwrite"
        } else {
            "prewrite"
        };
        return Err(yrs_engine::OperationError::engine_invariant_failed(
            request_id,
            None,
            format!(
                "compiled commit {} preparation failpoint ran {phase}",
                stage.field_name()
            ),
        ));
    }
    Ok(())
}
