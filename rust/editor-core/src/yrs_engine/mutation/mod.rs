mod lowering;
mod plan;

pub(crate) use lowering::{
    direct_root_wrap_metrics, mark_attr, removed_mark_attr, HistoryStoreSnapshotEvidence,
    ImportElementAttributeWork, ImportLookupMaterialization, ImportLookupMaterializationCollector,
    ImportTextCaptureWork, LocalizedFormatCompiler, LocalizedFormatLocator,
    LocalizedInsertCompiler, LocalizedInsertLocator, LocalizedRootWindowCompiler,
    LocalizedRootWindowLocator, MutationCompiler, MutationDocumentContext, MutationLookupPromotion,
    MutationLookupSeed, ReplacementInput, TextRangeDisposition,
};

#[cfg(test)]
pub(crate) use lowering::{
    lookup_payload_legacy_parity_for_test, reset_import_lookup_event_count_for_test,
    set_lookup_seed_hydration_failpoint_for_test, take_import_lookup_event_count_for_test,
    LookupSeedHydrationFailpoint,
};
pub(crate) use plan::{
    crdt_clock_scan_reservation, crdt_envelope, deleting_plan_undo_units,
    direct_xml_replacement_growth, estimate_undo_units, estimate_update_v1_growth,
    planned_insertion_units, preflight_mutation_plan, CrdtEnvelope, YrsMutationAction,
    YrsMutationPlan,
};

#[allow(unused_imports)] // Production execution is consumed by the engine boundary.
pub(crate) use plan::execute_mutation_plan;

#[cfg(test)]
pub(crate) use plan::preflight_mutation_work_for_test;

#[cfg(test)]
pub(crate) use lowering::{
    record_range_format_eager_fallback_for_test, record_root_window_eager_fallback_for_test,
    record_unavailable_lookup_seed_install_for_test, reset_localized_lookup_counts_for_test,
    reset_lookup_seed_map_growth_attempts_for_test, reset_range_format_lowering_counts_for_test,
    reset_root_window_lowering_counts_for_test, take_localized_lookup_counts_for_test,
    take_lookup_seed_map_growth_attempts_for_test, take_range_format_lowering_counts_for_test,
    take_root_window_lowering_counts_for_test,
};
