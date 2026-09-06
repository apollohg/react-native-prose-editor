use crate::boundary::ResourceLimits;
use crate::position::PositionMap;
use crate::schema::presets::tiptap_schema;
use crate::selection::Selection;
use crate::serialize::{from_prosemirror_json, UnknownTypeMode};
use crate::yrs_engine::canonical::CanonicalSchemaContext;
use crate::yrs_engine::commands::PlanningContext;
use crate::yrs_engine::{EditingLimits, ResolvedPoint, ResolvedSelection, TransactionOrigin};

const BEFORE: &str =
    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]}]}"#;
const AFTER: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]},{"type":"paragraph","content":[{"type":"text","text":"World"}]}]}"#;

// Task 16B: a bounded command-planning work budget is a deterministic
// operation limit, not allocation-class resource exhaustion.
#[test]
fn structural_diff_work_budget_excess_is_an_operation_limit_not_resource_exhaustion() {
    let schema = tiptap_schema();
    let before = from_prosemirror_json(
        &serde_json::from_str(BEFORE).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    let after = from_prosemirror_json(
        &serde_json::from_str(AFTER).unwrap(),
        &schema,
        UnknownTypeMode::Preserve,
    )
    .unwrap();
    // The structural diff work budget is max_document_nodes * 4; the two
    // fixture trees exceed a budget of 4 without any allocation failure.
    let limits = ResourceLimits {
        max_document_nodes: 1,
        ..ResourceLimits::default()
    };
    let editing_limits = EditingLimits::default();
    let canonical_schema = CanonicalSchemaContext::new(&schema);
    let canonical_artifact = canonical_schema.derive(&before).unwrap();
    let position_map = PositionMap::build(&before, &schema);
    let rendered_text = crate::render::rendered_text(&before, &schema);
    let point = ResolvedPoint {
        document: 0,
        scalar: 0,
        utf16: 0,
    };
    let selection = ResolvedSelection::Text {
        anchor: point,
        head: point,
    };
    let context = PlanningContext {
        request_id: 7,
        revision: 0,
        state_revision: 0,
        document: &before,
        position_map: &position_map,
        rendered_text: &rendered_text,
        selection: &selection,
        initial_selection: None,
        origin: TransactionOrigin::LocalCommand,
        stored_marks: None,
        schema: &schema,
        resource_limits: &limits,
        editing_limits: &editing_limits,
        max_length: None,
        yrs_state_epoch: 0,
        canonical_schema: &canonical_schema,
        canonical_artifact: &canonical_artifact,
        allow_deferred_admission: false,
        preparation: None,
    };
    let error = super::structural_fallback_transaction(
        &context,
        crate::command_planner::SemanticCommandHistory::InputBoundary,
        &after,
        &Selection::cursor(0),
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        error.details,
        Some(serde_json::json!({ "field": "commandPlanningWork" }))
    );
}
