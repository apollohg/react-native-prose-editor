use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Fragment, Mark, Node};
use crate::schema::content_rule::ContentRule;
use crate::schema::presets::tiptap_schema;
use crate::schema::{AttrSpec, NodeRole, NodeSpec, Schema};
use crate::selection::Selection;
use crate::transform::{Source, Step, Transaction};
use crate::yrs_engine::{
    Affinity, EditingLimitOverrides, EditingLimits, EditorOffsetKind, HistoryPolicy,
    InitializationMode, RevisionedPosition, RevisionedRange, SelectionInput, SelectionIntent,
    TransactionOrigin, TypedOperation, TypedTransaction, YrsDocumentEngine, YrsEngineConfig,
};

use super::{HistoryClass, SelectionPlan};

const PLAIN: &str =
    r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]}]}"#;
const BOLD: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"Hello"}]}]}"#;
const LINK: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"link","attrs":{"href":"old"}}],"text":"Hello"}]}]}"#;

#[derive(Debug, PartialEq)]
struct Audit {
    revision: u64,
    origin: Option<TransactionOrigin>,
    json: serde_json::Value,
    html: String,
    encoded: Vec<u8>,
}

fn audit(engine: &YrsDocumentEngine) -> Audit {
    Audit {
        revision: engine.revision(),
        origin: engine.last_committed_origin(),
        json: engine.document_json().unwrap(),
        html: engine.document_html().unwrap(),
        encoded: engine.encoded_state().unwrap(),
    }
}

fn engine(json: &str) -> YrsDocumentEngine {
    engine_with(
        json,
        ResourceLimits::default(),
        EditingLimits::default(),
        None,
    )
}

fn engine_with(
    json: &str,
    resource_limits: ResourceLimits,
    editing_limits: EditingLimits,
    max_length: Option<u32>,
) -> YrsDocumentEngine {
    let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
        schema: tiptap_schema(),
        fragment_name: "prosemirror".into(),
        initialization_mode: InitializationMode::LocalEmpty,
        resource_limits,
        editing_limits,
        max_length,
        scope: None,
    })
    .unwrap();
    if json != PLAIN || max_length != Some(0) {
        engine
            .import_json(json, TransactionOrigin::DocumentImport)
            .unwrap();
    }
    engine
}

fn point(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn range(from: u32, to: u32) -> RevisionedRange {
    RevisionedRange {
        from: point(from),
        to: point(to),
    }
}

fn transaction(engine: &YrsDocumentEngine, operations: Vec<TypedOperation>) -> TypedTransaction {
    TypedTransaction {
        request_id: 7,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalInput,
        operations,
        selection_intent: SelectionIntent::UseOperationResult,
        history_policy: HistoryPolicy::Auto,
    }
}

fn legacy(engine: &YrsDocumentEngine, steps: Vec<Step>) -> crate::model::Document {
    let mut transaction = Transaction::new(Source::Input);
    for step in steps {
        transaction.add_step(step);
    }
    transaction
        .apply(
            engine.document().unwrap(),
            &crate::schema::presets::tiptap_schema(),
        )
        .unwrap()
        .0
}

fn render_parity_schema() -> Schema {
    let base = tiptap_schema();
    let mut nodes: Vec<NodeSpec> = base.all_nodes().cloned().collect();
    let label_attrs = HashMap::from([(
        "label".to_string(),
        AttrSpec {
            default: Some(serde_json::Value::Null),
            has_default: true,
            ..AttrSpec::default()
        },
    )]);
    nodes.extend([
        NodeSpec {
            name: "mention".into(),
            content: ContentRule::parse("").unwrap(),
            group: Some("inline".into()),
            attrs: label_attrs.clone(),
            role: NodeRole::Inline,
            html_tag: None,
            html_rules: None,
            json_projection: None,
            is_void: true,
            deletable_on_backspace: None,
            allow_undeclared_attrs: true,
        },
        NodeSpec {
            name: "chip".into(),
            content: ContentRule::parse("").unwrap(),
            group: Some("inline".into()),
            attrs: label_attrs,
            role: NodeRole::Inline,
            html_tag: None,
            html_rules: None,
            json_projection: None,
            is_void: true,
            deletable_on_backspace: None,
            allow_undeclared_attrs: false,
        },
    ]);
    Schema::new(nodes, base.all_marks().cloned().collect())
}

fn assert_preview(
    engine: &YrsDocumentEngine,
    operation: TypedOperation,
    legacy_steps: Vec<Step>,
    history_class: HistoryClass,
) {
    let before = audit(engine);
    let expected = legacy(engine, legacy_steps);
    let compiled = engine
        .compile_typed_transaction(transaction(engine, vec![operation]))
        .unwrap();
    assert_eq!(compiled.preview, expected);
    assert_eq!(compiled.history_class, history_class);
    assert!(!compiled.mutation_plan.actions.is_empty());
    assert!(compiled.encoded_growth_bound > 0);
    assert!(!compiled.affected_top_level_blocks.is_empty());
    assert_eq!(audit(engine), before);
}

#[test]
fn semantic_preview_insert_matches_the_legacy_result_without_mutating_yrs() {
    let engine = engine_with(
        PLAIN,
        ResourceLimits::default(),
        EditingLimits::default(),
        None,
    );
    assert_preview(
        &engine,
        TypedOperation::InsertText {
            at: point(5),
            text: "!".into(),
            marks: vec![],
        },
        vec![Step::InsertText {
            pos: 6,
            text: "!".into(),
            marks: vec![],
        }],
        HistoryClass::Insert,
    );
}

#[test]
fn semantic_previews_for_delete_replace_and_marks_match_legacy_without_mutation() {
    let plain = engine(PLAIN);
    assert_preview(
        &plain,
        TypedOperation::DeleteRange { range: range(1, 4) },
        vec![Step::DeleteRange { from: 2, to: 5 }],
        HistoryClass::Delete,
    );
    let replacement = Fragment::from(vec![Node::text("i".into(), vec![])]);
    assert_preview(
        &plain,
        TypedOperation::ReplaceRange {
            range: range(1, 4),
            content: replacement.clone(),
        },
        vec![Step::ReplaceRange {
            from: 2,
            to: 5,
            content: replacement,
        }],
        HistoryClass::Structural,
    );
    let insertion = Fragment::from(vec![Node::text("!".into(), vec![])]);
    assert_preview(
        &plain,
        TypedOperation::ReplaceRange {
            range: range(5, 5),
            content: insertion.clone(),
        },
        vec![Step::ReplaceRange {
            from: 6,
            to: 6,
            content: insertion,
        }],
        HistoryClass::Insert,
    );
    assert_preview(
        &plain,
        TypedOperation::ReplaceRange {
            range: range(1, 4),
            content: Fragment::empty(),
        },
        vec![Step::ReplaceRange {
            from: 2,
            to: 5,
            content: Fragment::empty(),
        }],
        HistoryClass::Delete,
    );
    let bold = Mark::new("bold".into(), HashMap::new());
    assert_preview(
        &plain,
        TypedOperation::AddMark {
            range: range(1, 4),
            mark: bold.clone(),
        },
        vec![Step::AddMark {
            from: 2,
            to: 5,
            mark: bold,
        }],
        HistoryClass::Format,
    );

    let bold_engine = engine(BOLD);
    assert_preview(
        &bold_engine,
        TypedOperation::RemoveMark {
            range: range(1, 4),
            mark_type: "bold".into(),
        },
        vec![Step::RemoveMark {
            from: 2,
            to: 5,
            mark_type: "bold".into(),
        }],
        HistoryClass::Format,
    );

    let link_engine = engine(LINK);
    let replacement_link = Mark::new(
        "link".into(),
        HashMap::from([("href".into(), serde_json::json!("new"))]),
    );
    assert_preview(
        &link_engine,
        TypedOperation::ReplaceMark {
            range: range(1, 4),
            mark: replacement_link.clone(),
        },
        vec![
            Step::RemoveMark {
                from: 2,
                to: 5,
                mark_type: "link".into(),
            },
            Step::AddMark {
                from: 2,
                to: 5,
                mark: replacement_link,
            },
        ],
        HistoryClass::Format,
    );
}

#[test]
fn sequential_operations_resolve_against_base_and_map_selection() {
    let engine = engine(PLAIN);
    let current = Selection::cursor(6);
    let context = super::CompilationContext {
        document: engine.document().unwrap(),
        selection: Some(&current),
        schema: &tiptap_schema(),
        resource_limits: engine.resource_limits(),
        editing_limits: engine.editing_limits(),
        document_revision: engine.revision(),
        max_length: None,
    };
    let mut tx = transaction(
        &engine,
        vec![
            TypedOperation::InsertText {
                at: point(0),
                text: "!".into(),
                marks: vec![],
            },
            TypedOperation::InsertText {
                at: point(5),
                text: "?".into(),
                marks: vec![],
            },
        ],
    );
    tx.selection_intent = SelectionIntent::Preserve;
    let compiled = super::compile_transaction(context, tx).unwrap();
    assert_eq!(compiled.preview.root().text_content(), "!Hello?");
    assert_eq!(
        compiled.selection_plan,
        SelectionPlan::Mapped(Selection::cursor(8))
    );

    let context = super::CompilationContext {
        document: engine.document().unwrap(),
        selection: None,
        schema: &tiptap_schema(),
        resource_limits: engine.resource_limits(),
        editing_limits: engine.editing_limits(),
        document_revision: engine.revision(),
        max_length: None,
    };
    let mut tx = transaction(&engine, vec![]);
    tx.selection_intent = SelectionIntent::Set(SelectionInput::Text {
        anchor: point(1),
        head: point(4),
    });
    let compiled = super::compile_transaction(context, tx).unwrap();
    assert_eq!(
        compiled.selection_plan,
        SelectionPlan::Explicit(Selection::text(2, 5))
    );
}

#[test]
fn admission_errors_have_exact_codes_and_operation_indices() {
    let engine = engine(PLAIN);
    let mut stale = transaction(&engine, vec![]);
    stale.base_document_revision -= 1;
    let error = engine.compile_typed_transaction(stale).unwrap_err();
    assert_eq!(error.code, "REVISION_MISMATCH");
    assert_eq!(error.operation_index, None);

    let invalid = [
        TypedOperation::DeleteRange { range: range(4, 1) },
        TypedOperation::AddMark {
            range: range(1, 4),
            mark: Mark::new("unknown".into(), HashMap::new()),
        },
        TypedOperation::InsertText {
            at: point(1),
            text: "x".into(),
            marks: vec![
                Mark::new("bold".into(), HashMap::new()),
                Mark::new("bold".into(), HashMap::new()),
            ],
        },
        TypedOperation::InsertText {
            at: point(1),
            text: String::new(),
            marks: vec![],
        },
    ];
    for operation in invalid {
        let error = engine
            .compile_typed_transaction(transaction(&engine, vec![operation]))
            .unwrap_err();
        assert_eq!(error.operation_index, Some(0), "{error:?}");
    }
}

#[test]
fn unsorted_valid_input_marks_compile_to_canonical_legacy_previews() {
    let engine = engine(PLAIN);
    let bold = Mark::new("bold".into(), HashMap::new());
    let italic = Mark::new("italic".into(), HashMap::new());
    let unsorted = vec![italic.clone(), bold.clone()];
    let canonical = vec![bold, italic];

    assert_preview(
        &engine,
        TypedOperation::InsertText {
            at: point(5),
            text: "!".into(),
            marks: unsorted.clone(),
        },
        vec![Step::InsertText {
            pos: 6,
            text: "!".into(),
            marks: canonical.clone(),
        }],
        HistoryClass::Insert,
    );

    let unsorted_content = Fragment::from(vec![Node::text("!".into(), unsorted)]);
    let canonical_content = Fragment::from(vec![Node::text("!".into(), canonical)]);
    assert_preview(
        &engine,
        TypedOperation::ReplaceRange {
            range: range(4, 5),
            content: unsorted_content,
        },
        vec![Step::ReplaceRange {
            from: 5,
            to: 6,
            content: canonical_content,
        }],
        HistoryClass::Structural,
    );
}

#[test]
fn invalid_input_marks_reject_at_their_exact_operation_index() {
    let engine = engine(PLAIN);
    let invalid_mark_sets = [
        vec![
            Mark::new("bold".into(), HashMap::new()),
            Mark::new("bold".into(), HashMap::new()),
        ],
        vec![Mark::new("unknown".into(), HashMap::new())],
        vec![Mark::new(
            "bold".into(),
            HashMap::from([("undeclared".into(), serde_json::json!(true))]),
        )],
    ];

    for marks in invalid_mark_sets {
        let invalid_operations = [
            TypedOperation::InsertText {
                at: point(1),
                text: "x".into(),
                marks: marks.clone(),
            },
            TypedOperation::ReplaceRange {
                range: range(1, 2),
                content: Fragment::from(vec![Node::text("x".into(), marks)]),
            },
        ];
        for invalid_operation in invalid_operations {
            let error = engine
                .compile_typed_transaction(transaction(
                    &engine,
                    vec![
                        TypedOperation::DeleteRange { range: range(0, 0) },
                        invalid_operation,
                    ],
                ))
                .unwrap_err();
            assert_eq!(error.code, "OPERATION_INVALID");
            assert_eq!(error.operation_index, Some(1));
            assert_eq!(error.details, Some(serde_json::json!({ "field": "marks" })));
        }
    }
}

#[test]
fn no_ops_and_empty_ranges_compile_without_history_or_actions() {
    let engine = engine(PLAIN);
    for operations in [
        vec![],
        vec![TypedOperation::DeleteRange { range: range(2, 2) }],
    ] {
        let empty_envelope = operations.is_empty();
        let compiled = engine
            .compile_typed_transaction(transaction(&engine, operations))
            .unwrap();
        assert_eq!(compiled.preview, *engine.document().unwrap());
        assert_eq!(compiled.history_class, HistoryClass::Skip);
        assert!(compiled.affected_top_level_blocks.is_empty());
        assert!(compiled.mutation_plan.actions.is_empty());
        assert_eq!(compiled.canonical_artifact.is_none(), empty_envelope);
    }
}

include!("compiler_tests/resource_admission.rs");
