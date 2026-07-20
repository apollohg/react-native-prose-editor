//! Test-only observability for document-wide work in the Yrs editing path.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct FullPassCounts {
    pub import_model_parses: usize,
    pub validated_evidence_constructions: usize,
    pub validation_certificate_constructions: usize,
    pub planner_simulations: usize,
    pub document_validations: usize,
    pub canonical_mark_tree_scans: usize,
    pub canonical_mark_validation_attempts: usize,
    pub canonical_mark_validation_completions: usize,
    pub canonical_mark_nodes_visited: usize,
    pub canonical_identity_predicate_nodes_visited: usize,
    pub canonical_projections: usize,
    pub canonical_serializations: usize,
    pub canonical_hashes: usize,
    pub affected_top_level_scans: usize,
    pub position_map_clones: usize,
    pub position_map_compactions: usize,
    pub rendered_text_derivations: usize,
    pub raw_document_text_scans: usize,
    pub document_node_count_scans: usize,
    pub render_limit_tree_scans: usize,
    pub render_identity_scans: usize,
    pub render_top_level_start_scans: usize,
    pub active_applicability_passes: usize,
    pub ordinary_step_applications: usize,
}

#[allow(dead_code)] // Later deferred-admission tasks install the lifecycle transition sites.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparedAdmissionCounts {
    pub staged_seed_preparations: usize,
    pub installed_base_seed_publications: usize,
    pub staged_identity_materializations: usize,
    pub deferred_capsules_created: usize,
    pub deferred_capsules_finalized: usize,
    pub eager_fallbacks: usize,
}

macro_rules! recorder {
    ($name:ident, $field:ident) => {
        #[inline]
        pub(crate) fn $name() {
            FULL_PASS_COUNTS.with(|counts| {
                let mut next = counts.get();
                next.$field = next.$field.saturating_add(1);
                counts.set(next);
            });
        }
    };
}

std::thread_local! {
    static FULL_PASS_COUNTS: std::cell::Cell<FullPassCounts> = const {
        std::cell::Cell::new(FullPassCounts {
            import_model_parses: 0,
            validated_evidence_constructions: 0,
            validation_certificate_constructions: 0,
            planner_simulations: 0,
            document_validations: 0,
            canonical_mark_tree_scans: 0,
            canonical_mark_validation_attempts: 0,
            canonical_mark_validation_completions: 0,
            canonical_mark_nodes_visited: 0,
            canonical_identity_predicate_nodes_visited: 0,
            canonical_projections: 0,
            canonical_serializations: 0,
            canonical_hashes: 0,
            affected_top_level_scans: 0,
            position_map_clones: 0,
            position_map_compactions: 0,
            rendered_text_derivations: 0,
            raw_document_text_scans: 0,
            document_node_count_scans: 0,
            render_limit_tree_scans: 0,
            render_identity_scans: 0,
            render_top_level_start_scans: 0,
            active_applicability_passes: 0,
            ordinary_step_applications: 0,
        })
    };
    static PREPARED_ADMISSION_COUNTS: std::cell::Cell<PreparedAdmissionCounts> = const {
        std::cell::Cell::new(PreparedAdmissionCounts {
            staged_seed_preparations: 0,
            installed_base_seed_publications: 0,
            staged_identity_materializations: 0,
            deferred_capsules_created: 0,
            deferred_capsules_finalized: 0,
            eager_fallbacks: 0,
        })
    };
}

recorder!(record_planner_simulation, planner_simulations);
recorder!(record_import_model_parse, import_model_parses);
recorder!(
    record_validated_evidence_construction,
    validated_evidence_constructions
);
recorder!(
    record_validation_certificate_construction,
    validation_certificate_constructions
);
recorder!(record_document_validation, document_validations);
pub(crate) fn record_canonical_mark_validation_attempt() {
    FULL_PASS_COUNTS.with(|counts| {
        let mut next = counts.get();
        next.canonical_mark_tree_scans = next.canonical_mark_tree_scans.saturating_add(1);
        next.canonical_mark_validation_attempts =
            next.canonical_mark_validation_attempts.saturating_add(1);
        counts.set(next);
    });
}
recorder!(
    record_canonical_mark_validation_completion,
    canonical_mark_validation_completions
);
recorder!(
    record_canonical_mark_node_visited,
    canonical_mark_nodes_visited
);
recorder!(
    record_canonical_identity_predicate_node_visited,
    canonical_identity_predicate_nodes_visited
);
recorder!(record_canonical_projection, canonical_projections);
recorder!(record_canonical_serialization, canonical_serializations);
recorder!(record_canonical_hash, canonical_hashes);
recorder!(record_affected_top_level_scan, affected_top_level_scans);
recorder!(record_position_map_clone, position_map_clones);
recorder!(record_position_map_compaction, position_map_compactions);
recorder!(record_rendered_text_derivation, rendered_text_derivations);
recorder!(record_raw_document_text_scan, raw_document_text_scans);
recorder!(record_document_node_count_scan, document_node_count_scans);
recorder!(record_render_limit_tree_scan, render_limit_tree_scans);
recorder!(
    record_render_top_level_start_scan,
    render_top_level_start_scans
);
recorder!(
    record_active_applicability_pass,
    active_applicability_passes
);
recorder!(record_ordinary_step_application, ordinary_step_applications);

pub(crate) fn reset_full_pass_counts_for_test() {
    FULL_PASS_COUNTS.set(FullPassCounts::default());
}

pub(crate) fn take_full_pass_counts_for_test() -> FullPassCounts {
    FULL_PASS_COUNTS.replace(FullPassCounts::default())
}

#[allow(dead_code)] // Exercised when the deferred-admission lifecycle is wired in later tasks.
pub(crate) fn reset_prepared_admission_counts_for_test() {
    PREPARED_ADMISSION_COUNTS.set(PreparedAdmissionCounts::default());
}

#[allow(dead_code)] // Exercised when the deferred-admission lifecycle is wired in later tasks.
pub(crate) fn take_prepared_admission_counts_for_test() -> PreparedAdmissionCounts {
    PREPARED_ADMISSION_COUNTS.replace(PreparedAdmissionCounts::default())
}

#[allow(dead_code)] // Exercised as deferred-admission lifecycle stages are wired in.
pub(crate) fn record_staged_seed_preparation() {
    PREPARED_ADMISSION_COUNTS.with(|counts| {
        let mut next = counts.get();
        next.staged_seed_preparations = next.staged_seed_preparations.saturating_add(1);
        counts.set(next);
    });
}

#[allow(dead_code)] // Exercised as deferred-admission lifecycle stages are wired in.
pub(crate) fn record_installed_base_seed_publication() {
    PREPARED_ADMISSION_COUNTS.with(|counts| {
        let mut next = counts.get();
        next.installed_base_seed_publications =
            next.installed_base_seed_publications.saturating_add(1);
        counts.set(next);
    });
}

#[allow(dead_code)] // Exercised as deferred-admission lifecycle stages are wired in.
pub(crate) fn record_staged_identity_materialization() {
    PREPARED_ADMISSION_COUNTS.with(|counts| {
        let mut next = counts.get();
        next.staged_identity_materializations =
            next.staged_identity_materializations.saturating_add(1);
        counts.set(next);
    });
}

pub(crate) fn record_deferred_capsule_created() {
    PREPARED_ADMISSION_COUNTS.with(|counts| {
        let mut next = counts.get();
        next.deferred_capsules_created = next.deferred_capsules_created.saturating_add(1);
        counts.set(next);
    });
}

pub(crate) fn record_deferred_capsule_finalized() {
    PREPARED_ADMISSION_COUNTS.with(|counts| {
        let mut next = counts.get();
        next.deferred_capsules_finalized = next.deferred_capsules_finalized.saturating_add(1);
        counts.set(next);
    });
}
