use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;
use yrs::any::Any;
use yrs::branch::{Branch, BranchID};
use yrs::types::text::{Text, YChange};
use yrs::types::xml::{Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef};
use yrs::types::Attrs;
use yrs::ReadTxn;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::schema::{NodeRole, Schema};

use super::super::canonical::CanonicalArtifact;
use super::super::codec::{prepare_xml_nodes, PreparedTextRun, PreparedXmlChild, PreparedXmlNode};
use super::super::{EditingLimits, OperationError, OperationResult};
use super::plan::{
    attrs_work, binary_partition_work, capture_document_guard, crdt_clock_scan_reservation,
    expected_preflight_work, fenwick_add, fenwick_prefix, invalid_action_range, scan_overflow,
    work_overflow, CreatedTextAction, DocumentGuard, ElementSignature, ParentSignature,
    StructuralParentSignature, TargetSignature, TextSignatureRun, XmlParentRef, YrsMutationAction,
    YrsMutationPlan,
};

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LookupSeedHydrationFailpoint {
    InitialReservation,
    MapGrowth,
    MapPublication,
    BindingPublication,
    SeedPublication,
    CandidateBindingPublication,
    CandidateSeedPublication,
}

#[cfg(test)]
std::thread_local! {
    static LOOKUP_SEED_HYDRATION_FAILPOINT: std::cell::Cell<Option<LookupSeedHydrationFailpoint>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_lookup_seed_hydration_failpoint_for_test(
    failpoint: Option<LookupSeedHydrationFailpoint>,
) {
    LOOKUP_SEED_HYDRATION_FAILPOINT.set(failpoint);
}

#[inline]
fn lookup_seed_hydration_should_fail(stage: &str) -> bool {
    lookup_seed_hydration_should_fail_for_stage(stage, stage)
}

#[inline]
fn lookup_seed_hydration_should_fail_for_stage(stage: &str, error_stage: &str) -> bool {
    #[cfg(test)]
    {
        match LOOKUP_SEED_HYDRATION_FAILPOINT.get() {
            Some(LookupSeedHydrationFailpoint::CandidateBindingPublication) => {
                return error_stage == "candidateBindingPublication";
            }
            Some(LookupSeedHydrationFailpoint::CandidateSeedPublication) => {
                return error_stage == "candidateSeedPublication";
            }
            _ => {}
        }
        let expected = match stage {
            "initialReservation" => LookupSeedHydrationFailpoint::InitialReservation,
            "mapGrowth" => LookupSeedHydrationFailpoint::MapGrowth,
            "mapPublication" => LookupSeedHydrationFailpoint::MapPublication,
            "bindingPublication" => LookupSeedHydrationFailpoint::BindingPublication,
            "seedPublication" => LookupSeedHydrationFailpoint::SeedPublication,
            _ => return false,
        };
        LOOKUP_SEED_HYDRATION_FAILPOINT.get() == Some(expected)
    }
    #[cfg(not(test))]
    {
        let _ = (stage, error_stage);
        false
    }
}

fn lookup_seed_allocation_error(request_id: u64, stage: &'static str) -> OperationError {
    OperationError::operation_resource_exhausted(
        request_id,
        "mutationLookupSeed",
        format!("mutation lookup seed allocation failed during {stage}"),
    )
}

fn probe_lookup_seed_publication(
    request_id: u64,
    stage: &'static str,
    bytes: usize,
) -> OperationResult<()> {
    probe_lookup_seed_publication_for_stage(request_id, stage, stage, bytes)
}

fn probe_lookup_seed_publication_for_stage(
    request_id: u64,
    failpoint_stage: &'static str,
    error_stage: &'static str,
    bytes: usize,
) -> OperationResult<()> {
    if lookup_seed_hydration_should_fail_for_stage(failpoint_stage, error_stage) {
        return Err(lookup_seed_allocation_error(request_id, error_stage));
    }
    let mut probe = Vec::<u8>::new();
    probe
        .try_reserve_exact(bytes)
        .map_err(|_| lookup_seed_allocation_error(request_id, error_stage))
}

// Responsibility shards intentionally use `include!` so lowering remains one private scope.
include!("lowering/model.rs");
include!("lowering/index.rs");
include!("lowering/list.rs");
include!("lowering/node.rs");
include!("lowering/text.rs");
include!("lowering/attrs.rs");
include!("lowering/block.rs");
include!("lowering/import_lookup.rs");
include!("lowering/range.rs");
include!("lowering/prepared.rs");
include!("lowering/tests.rs");

#[cfg(test)]
std::thread_local! {
    static LOOKUP_SEED_BUILD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCALIZED_INSERT_HIT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOOKUP_SEED_PROMOTION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static EAGER_RANGE_TEXT_COLLECTION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static EAGER_RANGE_PARENT_COLLECTION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCALIZED_RANGE_FORMAT_HIT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static RANGE_FORMAT_EAGER_FALLBACK_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCALIZED_ROOT_WINDOW_HIT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static ROOT_WINDOW_EAGER_FALLBACK_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static UNAVAILABLE_LOOKUP_SEED_INSTALL_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCALIZED_ROOT_ATTR_MAP_BUILD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static SUPPRESS_RANGE_FORMAT_LOWERING_COUNTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static IMPORT_LOOKUP_EVENT_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_import_lookup_event_count_for_test() {
    IMPORT_LOOKUP_EVENT_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_import_lookup_event_count_for_test() -> usize {
    IMPORT_LOOKUP_EVENT_COUNT.replace(0)
}

#[cfg(test)]
pub(crate) fn reset_lookup_seed_map_growth_attempts_for_test() {
    LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_lookup_seed_map_growth_attempts_for_test() -> usize {
    LOOKUP_SEED_MAP_GROWTH_ATTEMPT_COUNT.replace(0)
}

#[cfg(test)]
fn reset_localized_root_attr_map_builds_for_test() {
    LOCALIZED_ROOT_ATTR_MAP_BUILD_COUNT.set(0);
}

#[cfg(test)]
fn take_localized_root_attr_map_builds_for_test() -> usize {
    LOCALIZED_ROOT_ATTR_MAP_BUILD_COUNT.replace(0)
}

#[cfg(test)]
fn reset_localized_format_promotion_target_visits_for_test() {
    LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT.set(0);
}

#[cfg(test)]
fn take_localized_format_promotion_target_visits_for_test() -> usize {
    LOCALIZED_FORMAT_PROMOTION_TARGET_VISIT_COUNT.replace(0)
}

#[cfg(test)]
pub(crate) fn reset_localized_lookup_counts_for_test() {
    LOOKUP_SEED_BUILD_COUNT.set(0);
    LOCALIZED_INSERT_HIT_COUNT.set(0);
    LOOKUP_SEED_PROMOTION_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_localized_lookup_counts_for_test() -> (usize, usize, usize) {
    (
        LOOKUP_SEED_BUILD_COUNT.replace(0),
        LOCALIZED_INSERT_HIT_COUNT.replace(0),
        LOOKUP_SEED_PROMOTION_COUNT.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn reset_range_format_lowering_counts_for_test() {
    EAGER_RANGE_TEXT_COLLECTION_COUNT.set(0);
    EAGER_RANGE_PARENT_COLLECTION_COUNT.set(0);
    LOCALIZED_RANGE_FORMAT_HIT_COUNT.set(0);
    RANGE_FORMAT_EAGER_FALLBACK_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_range_format_lowering_counts_for_test() -> (usize, usize, usize, usize) {
    (
        EAGER_RANGE_TEXT_COLLECTION_COUNT.replace(0),
        EAGER_RANGE_PARENT_COLLECTION_COUNT.replace(0),
        LOCALIZED_RANGE_FORMAT_HIT_COUNT.replace(0),
        RANGE_FORMAT_EAGER_FALLBACK_COUNT.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn record_range_format_eager_fallback_for_test() {
    RANGE_FORMAT_EAGER_FALLBACK_COUNT
        .set(RANGE_FORMAT_EAGER_FALLBACK_COUNT.get().saturating_add(1));
}

#[cfg(test)]
pub(crate) fn reset_root_window_lowering_counts_for_test() {
    EAGER_RANGE_TEXT_COLLECTION_COUNT.set(0);
    EAGER_RANGE_PARENT_COLLECTION_COUNT.set(0);
    LOCALIZED_ROOT_WINDOW_HIT_COUNT.set(0);
    ROOT_WINDOW_EAGER_FALLBACK_COUNT.set(0);
    LOOKUP_SEED_BUILD_COUNT.set(0);
    UNAVAILABLE_LOOKUP_SEED_INSTALL_COUNT.set(0);
}

#[cfg(test)]
pub(crate) fn take_root_window_lowering_counts_for_test(
) -> (usize, usize, usize, usize, usize, usize) {
    (
        EAGER_RANGE_TEXT_COLLECTION_COUNT.replace(0),
        EAGER_RANGE_PARENT_COLLECTION_COUNT.replace(0),
        LOCALIZED_ROOT_WINDOW_HIT_COUNT.replace(0),
        ROOT_WINDOW_EAGER_FALLBACK_COUNT.replace(0),
        LOOKUP_SEED_BUILD_COUNT.replace(0),
        UNAVAILABLE_LOOKUP_SEED_INSTALL_COUNT.replace(0),
    )
}

#[cfg(test)]
pub(crate) fn record_root_window_eager_fallback_for_test() {
    ROOT_WINDOW_EAGER_FALLBACK_COUNT.set(ROOT_WINDOW_EAGER_FALLBACK_COUNT.get().saturating_add(1));
}

#[cfg(test)]
pub(crate) fn record_unavailable_lookup_seed_install_for_test() {
    UNAVAILABLE_LOOKUP_SEED_INSTALL_COUNT.set(
        UNAVAILABLE_LOOKUP_SEED_INSTALL_COUNT
            .get()
            .saturating_add(1),
    );
}
