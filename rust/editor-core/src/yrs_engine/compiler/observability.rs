#[cfg(test)]
use crate::yrs_engine::{OperationError, OperationResult};

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
    pub(super) static ATOMIC_FAILPOINT: std::cell::Cell<Option<AtomicFailpoint>> = const {
        std::cell::Cell::new(None)
    };
    pub(super) static SEMANTIC_COMPILATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static BASE_POSITION_MAP_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static BASE_RENDERED_TEXT_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static BASE_DOCUMENT_TEXT_BYTES_BUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
    pub(super) static FORCE_LOCALIZED_SEMANTIC_ALLOCATION_FAILURE: std::cell::Cell<bool> = const {
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
