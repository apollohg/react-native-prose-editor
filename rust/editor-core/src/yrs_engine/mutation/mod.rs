mod lowering;
mod plan;

pub(crate) use lowering::{
    mark_attr, removed_mark_attr, MutationCompiler, MutationDocumentContext, ReplacementInput,
    TextRangeDisposition,
};
pub(crate) use plan::{
    crdt_clock_scan_reservation, crdt_envelope, estimate_undo_units, estimate_update_v1_growth,
    planned_insertion_units, preflight_mutation_plan, CrdtEnvelope, YrsMutationPlan,
};

#[allow(unused_imports)] // Production execution is consumed by the Task 7 engine boundary.
pub(crate) use plan::execute_mutation_plan;

#[cfg(test)]
pub(crate) use plan::{preflight_mutation_work_for_test, YrsMutationAction};
