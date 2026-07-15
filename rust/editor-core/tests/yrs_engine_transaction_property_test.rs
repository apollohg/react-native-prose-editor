use std::cell::RefCell;
use std::collections::HashMap;

use editor_core::boundary::ResourceLimits;
use editor_core::model::{Document, Fragment, Mark, Node};
use editor_core::position::PositionMap;
use editor_core::render::RenderElement;
use editor_core::schema::{NodeRole, Schema};
use editor_core::serialize::{
    from_prosemirror_json, to_html, to_prosemirror_json, UnknownTypeMode,
};
use editor_core::tiptap_schema;
use editor_core::transform::{DocumentValidator, Source, Step, Transaction, TransformError};
use editor_core::yrs_engine::{
    scalar_offset_to_utf16, Affinity, DocumentScope, EditingLimits, EditorOffsetKind,
    HistoryPolicy, InitializationMode, OperationError, RevisionedPosition, RevisionedRange,
    SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction, YrsDocumentEngine,
    YrsEngineConfig,
};
use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

const PLAIN: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"abcdef"}]}]}"#;
const BOLD: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"bold"}],"text":"abcdef"}]}]}"#;
const LINK: &str = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","marks":[{"type":"link","attrs":{"href":"old"}}],"text":"abcdef"}]}]}"#;

#[derive(Clone, Debug)]
struct ActionSpec {
    kind: u8,
    salt: u64,
}

#[derive(Default)]
struct Coverage {
    operations: [bool; 14],
    randomized_operations: [bool; 14],
    scalar: bool,
    utf16: bool,
    before: bool,
    after: bool,
    custom_root: bool,
    void_node: bool,
    opaque_node: bool,
    randomized_void_node: bool,
    randomized_opaque_node: bool,
    longest_trace: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorClass {
    Position,
    InvalidRange,
    InvalidContent,
}

struct Scenario {
    schema: Schema,
    source: serde_json::Value,
    operation: TypedOperation,
    legacy_steps: Vec<Step>,
}

fn engine_with_mode(schema: Schema, initialization_mode: InitializationMode) -> YrsDocumentEngine {
    YrsDocumentEngine::new(YrsEngineConfig {
        schema,
        fragment_name: "prosemirror".into(),
        initialization_mode,
        resource_limits: ResourceLimits::default(),
        editing_limits: EditingLimits::default(),
        max_length: None,
        scope: Some(DocumentScope {
            document_id: "transaction-property".into(),
            lineage_id: "transaction-property-lineage".into(),
        }),
    })
    .unwrap()
}

fn engine(schema: Schema) -> YrsDocumentEngine {
    engine_with_mode(schema, InitializationMode::LocalEmpty)
}

fn assert_encoded_state_matches(engine: &YrsDocumentEngine, expected: &Document, schema: &Schema) {
    let snapshot = engine.export_snapshot().unwrap();
    let mut hydrated = engine_with_mode(schema.clone(), InitializationMode::AwaitRemote);
    hydrated.restore_snapshot(&snapshot).unwrap();
    assert_eq!(
        hydrated.document_json(),
        Some(to_prosemirror_json(expected, schema))
    );
    assert_eq!(hydrated.document(), Some(expected));
    assert_eq!(hydrated.document_html(), Some(to_html(expected, schema)));
    DocumentValidator::validate(
        hydrated.document().unwrap(),
        schema,
        &ResourceLimits::default(),
    )
    .unwrap();
}

fn transaction(
    engine: &YrsDocumentEngine,
    request_id: u64,
    operation: TypedOperation,
) -> TypedTransaction {
    transaction_with_operations(engine, request_id, vec![operation])
}

fn transaction_with_operations(
    engine: &YrsDocumentEngine,
    request_id: u64,
    operations: Vec<TypedOperation>,
) -> TypedTransaction {
    TypedTransaction {
        request_id,
        base_document_revision: engine.revision(),
        origin: TransactionOrigin::LocalApi,
        operations,
        selection_intent: SelectionIntent::Preserve,
        history_policy: HistoryPolicy::Skip,
    }
}

fn custom_root_schema() -> Schema {
    Schema::from_json(&serde_json::json!({
        "nodes": [
            { "name": "article", "content": "body+", "role": "doc" },
            { "name": "body", "content": "inline*", "group": "body", "role": "textBlock", "htmlTag": "section" },
            { "name": "text", "content": "", "group": "inline", "role": "text" }
        ],
        "marks": []
    }))
    .unwrap()
}

fn scalar_index(haystack: &str, needle: &str) -> u32 {
    u32::try_from(haystack[..haystack.find(needle).unwrap()].chars().count()).unwrap()
}

fn rendered_text(document: &Document, schema: &Schema) -> String {
    let blocks = editor_core::render::incremental::render_blocks(document, schema);
    let elements = editor_core::render::incremental::flatten_render_blocks(&blocks);
    let mut text = String::new();
    let mut pending_prefix = String::new();
    let mut started_block = false;

    let begin_block = |text: &mut String, started_block: &mut bool| {
        if *started_block {
            text.push('\n');
        }
        *started_block = true;
    };

    for element in elements {
        match element {
            RenderElement::BlockStart {
                node_type,
                list_context,
                ..
            } => {
                if let Some(context) = list_context {
                    pending_prefix = if context.kind.as_deref() == Some("task") {
                        editor_core::render::task_list_marker_string(
                            context.checked.unwrap_or(false),
                        )
                    } else {
                        editor_core::render::list_marker_string(context.ordered, context.index)
                    };
                }
                if schema
                    .node(&node_type)
                    .is_some_and(|spec| matches!(spec.role, NodeRole::TextBlock))
                {
                    begin_block(&mut text, &mut started_block);
                    text.push_str(&pending_prefix);
                    pending_prefix.clear();
                }
            }
            RenderElement::TextRun { text: value, .. } => text.push_str(&value),
            RenderElement::VoidInline { .. } => text.push('\n'),
            RenderElement::VoidBlock { .. } => {
                begin_block(&mut text, &mut started_block);
                text.push('\u{fffc}');
            }
            RenderElement::OpaqueInlineAtom {
                node_type, label, ..
            } => text.push_str(&editor_core::render::opaque_atom_visible_string(
                &node_type, &label,
            )),
            RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => {
                begin_block(&mut text, &mut started_block);
                text.push_str(&editor_core::render::opaque_atom_visible_string(
                    &node_type, &label,
                ));
            }
            RenderElement::BlockEnd => {}
        }
    }
    text
}

fn revisioned(
    rendered: &str,
    scalar: u32,
    salt: u64,
    coverage: &RefCell<Coverage>,
) -> RevisionedPosition {
    let affinity = if salt & 2 == 0 {
        coverage.borrow_mut().before = true;
        Affinity::Before
    } else {
        coverage.borrow_mut().after = true;
        Affinity::After
    };
    if salt & 1 == 0 {
        coverage.borrow_mut().scalar = true;
        RevisionedPosition {
            offset: scalar,
            kind: EditorOffsetKind::Scalar,
            affinity,
        }
    } else {
        coverage.borrow_mut().utf16 = true;
        RevisionedPosition {
            offset: scalar_offset_to_utf16(rendered, scalar).unwrap(),
            kind: EditorOffsetKind::Utf16,
            affinity,
        }
    }
}

fn range(
    rendered: &str,
    from: u32,
    to: u32,
    salt: u64,
    coverage: &RefCell<Coverage>,
) -> RevisionedRange {
    RevisionedRange {
        from: revisioned(rendered, from, salt, coverage),
        to: revisioned(rendered, to, salt.rotate_left(1), coverage),
    }
}

fn legacy_position(document: &Document, schema: &Schema, scalar: u32) -> u32 {
    PositionMap::build(document, schema).scalar_to_doc(scalar, document)
}

fn opaque_inline(salt: u64) -> Node {
    let exact_yjs_integer = salt % 9_007_199_254_740_991;
    let original = serde_json::json!({
        "type": "traceExtension",
        "attrs": { "seed": exact_yjs_integer },
        "content": [{ "type": "text", "text": "wire-only" }]
    });
    Node::void(
        "__opaque_json".into(),
        HashMap::from([
            ("original_type".into(), serde_json::json!("traceExtension")),
            ("original_json".into(), original),
            ("opaque_placement".into(), serde_json::json!("inline")),
        ]),
    )
}

fn scenario(spec: &ActionSpec, coverage: &RefCell<Coverage>) -> Scenario {
    coverage.borrow_mut().operations[usize::from(spec.kind)] = true;
    let schema = tiptap_schema();
    let source = match spec.kind {
        4 => serde_json::from_str(BOLD).unwrap(),
        5 => serde_json::from_str(LINK).unwrap(),
        7 => serde_json::json!({
            "type": "doc",
            "content": [
                { "type": "paragraph", "content": [{ "type": "text", "text": "ab" }] },
                { "type": "paragraph", "content": [{ "type": "text", "text": "cd" }] }
            ]
        }),
        8 => serde_json::json!({
            "type": "doc",
            "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }]
        }),
        9 => serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] }]
            }]
        }),
        10 => serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "one" }] }] },
                    { "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }] }
                ]
            }]
        }),
        11 => serde_json::json!({
            "type": "doc",
            "content": [{
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [
                        { "type": "paragraph", "content": [{ "type": "text", "text": "outer" }] },
                        { "type": "bulletList", "content": [{ "type": "listItem", "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "inner" }] }] }] }
                    ]
                }]
            }]
        }),
        13 => serde_json::json!({
            "type": "doc",
            "content": [{ "type": "image", "attrs": { "src": "old", "alt": null, "title": null, "width": null, "height": null } }]
        }),
        _ => serde_json::from_str(PLAIN).unwrap(),
    };
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = rendered_text(&document, &schema);
    let pm = |scalar| legacy_position(&document, &schema, scalar);
    let bold = Mark::new("bold".into(), HashMap::new());
    let (operation, legacy_steps) = match spec.kind {
        0 => {
            let at = 3;
            let text = char::from(b'a' + u8::try_from(spec.salt % 26).unwrap()).to_string();
            (
                TypedOperation::InsertText {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                    text: text.clone(),
                    marks: vec![],
                },
                vec![Step::InsertText {
                    pos: pm(at),
                    text,
                    marks: vec![],
                }],
            )
        }
        1 => (
            TypedOperation::DeleteRange {
                range: range(&rendered, 1, 4, spec.salt, coverage),
            },
            vec![Step::DeleteRange {
                from: pm(1),
                to: pm(4),
            }],
        ),
        2 => {
            let content = Fragment::from(vec![Node::text("XY".into(), vec![])]);
            (
                TypedOperation::ReplaceRange {
                    range: range(&rendered, 2, 4, spec.salt, coverage),
                    content: content.clone(),
                },
                vec![Step::ReplaceRange {
                    from: pm(2),
                    to: pm(4),
                    content,
                }],
            )
        }
        3 => (
            TypedOperation::AddMark {
                range: range(&rendered, 1, 5, spec.salt, coverage),
                mark: bold.clone(),
            },
            vec![Step::AddMark {
                from: pm(1),
                to: pm(5),
                mark: bold,
            }],
        ),
        4 => (
            TypedOperation::RemoveMark {
                range: range(&rendered, 1, 5, spec.salt, coverage),
                mark_type: "bold".into(),
            },
            vec![Step::RemoveMark {
                from: pm(1),
                to: pm(5),
                mark_type: "bold".into(),
            }],
        ),
        5 => {
            let mark = Mark::new(
                "link".into(),
                HashMap::from([(
                    "href".into(),
                    serde_json::json!(format!("new-{}", spec.salt % 7)),
                )]),
            );
            (
                TypedOperation::ReplaceMark {
                    range: range(&rendered, 1, 5, spec.salt, coverage),
                    mark: mark.clone(),
                },
                vec![
                    Step::RemoveMark {
                        from: pm(1),
                        to: pm(5),
                        mark_type: "link".into(),
                    },
                    Step::AddMark {
                        from: pm(1),
                        to: pm(5),
                        mark,
                    },
                ],
            )
        }
        6 => (
            TypedOperation::SplitBlock {
                at: revisioned(&rendered, 3, spec.salt, coverage),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            },
            vec![Step::SplitBlock {
                pos: pm(3),
                node_type: "paragraph".into(),
                attrs: HashMap::new(),
            }],
        ),
        7 => (
            TypedOperation::JoinBlocks {
                at: revisioned(&rendered, 2, spec.salt, coverage),
            },
            vec![Step::JoinBlocks { pos: 4 }],
        ),
        8 => (
            TypedOperation::WrapInList {
                range: range(&rendered, 0, 3, spec.salt, coverage),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            },
            vec![Step::WrapInList {
                from: pm(0),
                to: pm(3),
                list_type: "bulletList".into(),
                item_type: "listItem".into(),
                attrs: HashMap::new(),
                item_attrs: HashMap::new(),
            }],
        ),
        9 => {
            let at = scalar_index(&rendered, "one") + 1;
            (
                TypedOperation::UnwrapFromList {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                },
                vec![Step::UnwrapFromList { pos: pm(at) }],
            )
        }
        10 => {
            let at = scalar_index(&rendered, "two") + 1;
            (
                TypedOperation::IndentListItem {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                },
                vec![Step::IndentListItem { pos: pm(at) }],
            )
        }
        11 => {
            let at = scalar_index(&rendered, "inner") + 1;
            (
                TypedOperation::OutdentListItem {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                },
                vec![Step::OutdentListItem { pos: pm(at) }],
            )
        }
        12 => {
            let node = if spec.salt & 4 == 0 {
                coverage.borrow_mut().void_node = true;
                Node::void("hardBreak".into(), HashMap::new())
            } else {
                coverage.borrow_mut().opaque_node = true;
                opaque_inline(spec.salt)
            };
            (
                TypedOperation::InsertNode {
                    at: revisioned(&rendered, 3, spec.salt, coverage),
                    node: node.clone(),
                },
                vec![Step::InsertNode { pos: pm(3), node }],
            )
        }
        13 => {
            let attrs = HashMap::from([
                (
                    "src".into(),
                    serde_json::json!(format!("new-{}", spec.salt % 11)),
                ),
                ("alt".into(), serde_json::json!("trace")),
                ("title".into(), serde_json::Value::Null),
                ("width".into(), serde_json::Value::Null),
                ("height".into(), serde_json::Value::Null),
            ]);
            (
                TypedOperation::UpdateNodeAttrs {
                    at: revisioned(&rendered, 0, spec.salt, coverage),
                    attrs: attrs.clone(),
                },
                vec![Step::UpdateNodeAttrs { pos: pm(0), attrs }],
            )
        }
        _ => unreachable!(),
    };
    Scenario {
        schema,
        source,
        operation,
        legacy_steps,
    }
}

fn run_scenario(spec: &ActionSpec, coverage: &RefCell<Coverage>) {
    let Scenario {
        schema,
        source,
        operation,
        legacy_steps,
    } = scenario(spec, coverage);
    let mut legacy = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let mut legacy_transaction = Transaction::new(Source::Api);
    for step in legacy_steps {
        legacy_transaction.add_step(step);
    }
    let (expected, _) = legacy_transaction
        .apply_with_limits(&legacy, &schema, &ResourceLimits::default())
        .unwrap_or_else(|error| panic!("legacy kind {} failed: {error:?}", spec.kind));
    legacy = expected;

    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let commit = yrs
        .apply_typed_transaction(transaction(&yrs, spec.salt, operation))
        .unwrap_or_else(|error| panic!("Yrs kind {} failed: {error:?}", spec.kind));
    assert_eq!(
        commit.changed,
        yrs.document().unwrap()
            != &from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap()
    );
    assert_eq!(yrs.document().unwrap(), &legacy);
    assert_eq!(
        yrs.document_json().unwrap(),
        to_prosemirror_json(&legacy, &schema)
    );
    assert_eq!(yrs.document_html().unwrap(), to_html(&legacy, &schema));
    DocumentValidator::validate(yrs.document().unwrap(), &schema, &ResourceLimits::default())
        .unwrap();
    DocumentValidator::validate(&legacy, &schema, &ResourceLimits::default()).unwrap();
    assert_encoded_state_matches(&yrs, &legacy, &schema);
}

fn trace_span(salt: u64, scalar_len: u32) -> (u32, u32) {
    debug_assert!(scalar_len > 0);
    let from = u32::try_from(salt % u64::from(scalar_len)).unwrap();
    let remaining = scalar_len - from;
    let max_width = remaining.min(3);
    let width = 1 + u32::try_from(salt.rotate_left(13) % u64::from(max_width)).unwrap();
    (from, from + width)
}

fn stateful_inline_scenario(
    spec: &ActionSpec,
    document: &Document,
    schema: &Schema,
    coverage: &RefCell<Coverage>,
) -> (TypedOperation, Vec<Step>) {
    let rendered = rendered_text(document, schema);
    let scalar_len = u32::try_from(rendered.chars().count()).unwrap();
    let kind = if scalar_len == 0 { 0 } else { spec.kind % 6 };
    let pm = |scalar| legacy_position(document, schema, scalar);
    match kind {
        0 => {
            let at = u32::try_from(spec.salt % u64::from(scalar_len + 1)).unwrap();
            let text = if spec.salt & 4 == 0 {
                "😀".to_string()
            } else {
                char::from(b'a' + u8::try_from(spec.salt % 26).unwrap()).to_string()
            };
            (
                TypedOperation::InsertText {
                    at: revisioned(&rendered, at, spec.salt, coverage),
                    text: text.clone(),
                    marks: vec![],
                },
                vec![Step::InsertText {
                    pos: pm(at),
                    text,
                    marks: vec![],
                }],
            )
        }
        1 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            (
                TypedOperation::DeleteRange {
                    range: range(&rendered, from, to, spec.salt, coverage),
                },
                vec![Step::DeleteRange {
                    from: pm(from),
                    to: pm(to),
                }],
            )
        }
        2 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            let replacement = if spec.salt & 8 == 0 { "Ω" } else { "Q" };
            let content = Fragment::from(vec![Node::text(replacement.into(), vec![])]);
            (
                TypedOperation::ReplaceRange {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    content: content.clone(),
                },
                vec![Step::ReplaceRange {
                    from: pm(from),
                    to: pm(to),
                    content,
                }],
            )
        }
        3 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            let mark = Mark::new("bold".into(), HashMap::new());
            (
                TypedOperation::AddMark {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    mark: mark.clone(),
                },
                vec![Step::AddMark {
                    from: pm(from),
                    to: pm(to),
                    mark,
                }],
            )
        }
        4 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            (
                TypedOperation::RemoveMark {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    mark_type: "bold".into(),
                },
                vec![Step::RemoveMark {
                    from: pm(from),
                    to: pm(to),
                    mark_type: "bold".into(),
                }],
            )
        }
        5 => {
            let (from, to) = trace_span(spec.salt, scalar_len);
            let mark = Mark::new("bold".into(), HashMap::new());
            (
                TypedOperation::ReplaceMark {
                    range: range(&rendered, from, to, spec.salt, coverage),
                    mark: mark.clone(),
                },
                vec![
                    Step::RemoveMark {
                        from: pm(from),
                        to: pm(to),
                        mark_type: "bold".into(),
                    },
                    Step::AddMark {
                        from: pm(from),
                        to: pm(to),
                        mark,
                    },
                ],
            )
        }
        _ => unreachable!(),
    }
}

fn run_stateful_trace(trace: &[ActionSpec], coverage: &RefCell<Coverage>) {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a😀bcdef" }] }]
    });
    let mut legacy = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    for (index, spec) in trace.iter().enumerate() {
        let legacy_before = legacy.clone();
        let engine_before = (
            yrs.encoded_state().unwrap(),
            yrs.revision(),
            yrs.state_revision(),
        );
        let (operation, legacy_steps) = stateful_inline_scenario(spec, &legacy, &schema, coverage);
        let mut legacy_transaction = Transaction::new(Source::Api);
        for step in legacy_steps {
            legacy_transaction.add_step(step);
        }
        legacy = legacy_transaction
            .apply_with_limits(&legacy, &schema, &ResourceLimits::default())
            .unwrap_or_else(|error| {
                panic!("legacy trace step {index} ({spec:?}) failed: {error:?}")
            })
            .0;
        let expected_changed = legacy != legacy_before;
        let commit = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                10_000 + u64::try_from(index).unwrap(),
                operation,
            ))
            .unwrap_or_else(|error| panic!("Yrs trace step {index} ({spec:?}) failed: {error:?}"));
        assert_eq!(
            commit.changed, expected_changed,
            "trace step {index}: {spec:?}"
        );
        if !expected_changed {
            assert_eq!(
                (
                    yrs.encoded_state().unwrap(),
                    yrs.revision(),
                    yrs.state_revision()
                ),
                engine_before,
                "no-op trace step {index}: {spec:?}"
            );
        }

        assert_eq!(
            yrs.document(),
            Some(&legacy),
            "trace step {index}: {spec:?}"
        );
        assert_eq!(
            yrs.document_json(),
            Some(to_prosemirror_json(&legacy, &schema)),
            "trace step {index}: {spec:?}"
        );
        assert_eq!(
            yrs.document_html(),
            Some(to_html(&legacy, &schema)),
            "trace step {index}: {spec:?}"
        );
        DocumentValidator::validate(&legacy, &schema, &ResourceLimits::default()).unwrap();
        assert_encoded_state_matches(&yrs, &legacy, &schema);
    }
}

fn run_custom_root_case(coverage: &RefCell<Coverage>) {
    coverage.borrow_mut().custom_root = true;
    let schema = custom_root_schema();
    let source = serde_json::json!({
        "type": "article",
        "content": [{ "type": "body", "content": [{ "type": "text", "text": "root😀" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = rendered_text(&document, &schema);
    let at = revisioned(&rendered, 2, 3, coverage);
    let pos = legacy_position(&document, &schema, 2);
    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::InsertText {
        pos,
        text: "!".into(),
        marks: vec![],
    });
    let expected = legacy.apply(&document, &schema).unwrap().0;
    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    yrs.apply_typed_transaction(transaction(
        &yrs,
        9_001,
        TypedOperation::InsertText {
            at,
            text: "!".into(),
            marks: vec![],
        },
    ))
    .unwrap();
    assert_eq!(yrs.document(), Some(&expected));
    assert_eq!(
        yrs.document_json(),
        Some(to_prosemirror_json(&expected, &schema))
    );
    assert_eq!(yrs.document_html(), Some(to_html(&expected, &schema)));
    assert_encoded_state_matches(&yrs, &expected, &schema);
}

fn run_evolving_list_chain(salt: u64, coverage: &RefCell<Coverage>) {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [
            { "type": "paragraph", "content": [{ "type": "text", "text": "one" }] },
            { "type": "paragraph", "content": [{ "type": "text", "text": "two" }] }
        ]
    });
    let mut legacy = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();

    for operation_index in 0..4 {
        let rendered = rendered_text(&legacy, &schema);
        let pm = |scalar| legacy_position(&legacy, &schema, scalar);
        let operation_salt = salt.rotate_left(operation_index * 7);
        let (operation, step) = match operation_index {
            0 => {
                let end = u32::try_from(rendered.chars().count()).unwrap();
                (
                    TypedOperation::WrapInList {
                        range: range(&rendered, 0, end, operation_salt, coverage),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    },
                    Step::WrapInList {
                        from: pm(0),
                        to: pm(end),
                        list_type: "bulletList".into(),
                        item_type: "listItem".into(),
                        attrs: HashMap::new(),
                        item_attrs: HashMap::new(),
                    },
                )
            }
            1 => {
                let at = scalar_index(&rendered, "two") + 1;
                (
                    TypedOperation::IndentListItem {
                        at: revisioned(&rendered, at, operation_salt, coverage),
                    },
                    Step::IndentListItem { pos: pm(at) },
                )
            }
            2 => {
                let at = scalar_index(&rendered, "two") + 1;
                (
                    TypedOperation::OutdentListItem {
                        at: revisioned(&rendered, at, operation_salt, coverage),
                    },
                    Step::OutdentListItem { pos: pm(at) },
                )
            }
            3 => {
                let at = scalar_index(&rendered, "one") + 1;
                (
                    TypedOperation::UnwrapFromList {
                        at: revisioned(&rendered, at, operation_salt, coverage),
                    },
                    Step::UnwrapFromList { pos: pm(at) },
                )
            }
            _ => unreachable!(),
        };

        let mut legacy_transaction = Transaction::new(Source::Api);
        legacy_transaction.add_step(step);
        legacy = legacy_transaction
            .apply_with_limits(&legacy, &schema, &ResourceLimits::default())
            .unwrap()
            .0;
        let commit = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                30_000 + u64::from(operation_index),
                operation,
            ))
            .unwrap();
        assert!(commit.changed);
        assert_eq!(yrs.document(), Some(&legacy));
        assert_eq!(
            yrs.document_json(),
            Some(to_prosemirror_json(&legacy, &schema))
        );
        assert_eq!(yrs.document_html(), Some(to_html(&legacy, &schema)));
        DocumentValidator::validate(&legacy, &schema, &ResourceLimits::default()).unwrap();
        assert_encoded_state_matches(&yrs, &legacy, &schema);
    }
}

fn arb_trace() -> impl Strategy<Value = Vec<ActionSpec>> {
    prop::collection::vec(
        (0_u8..14, any::<u64>()).prop_map(|(kind, salt)| ActionSpec { kind, salt }),
        1..=100,
    )
}

fn operation_error_class(error: &OperationError) -> ErrorClass {
    match error.code {
        "POSITION_INVALID" => ErrorClass::Position,
        "OPERATION_INVALID"
            if error
                .details
                .as_ref()
                .and_then(|value| value["field"].as_str())
                == Some("range") =>
        {
            ErrorClass::InvalidRange
        }
        "OPERATION_INVALID" | "DOCUMENT_INVALID" => ErrorClass::InvalidContent,
        other => panic!("unclassified operation error {other}: {error:?}"),
    }
}

fn transform_error_class(error: &TransformError) -> ErrorClass {
    match error {
        TransformError::OutOfBounds(_) | TransformError::InvalidTarget(_) => ErrorClass::Position,
        TransformError::InvalidRange(_) => ErrorClass::InvalidRange,
        TransformError::ContentViolation(_) | TransformError::NotImplemented(_) => {
            ErrorClass::InvalidContent
        }
    }
}

#[test]
fn deterministic_transaction_traces_match_the_legacy_oracle_after_every_operation() {
    let coverage = RefCell::new(Coverage::default());

    for kind in 0..14 {
        run_scenario(
            &ActionSpec {
                kind,
                salt: u64::from(kind),
            },
            &coverage,
        );
    }
    run_scenario(&ActionSpec { kind: 12, salt: 0 }, &coverage);
    run_scenario(&ActionSpec { kind: 12, salt: 4 }, &coverage);
    run_custom_root_case(&coverage);
    let fixed_hundred = (0..100)
        .map(|index| ActionSpec {
            kind: u8::try_from(index % 14).unwrap(),
            salt: index as u64,
        })
        .collect::<Vec<_>>();
    run_stateful_trace(&fixed_hundred, &coverage);
    coverage.borrow_mut().longest_trace = fixed_hundred.len();

    let config = Config {
        cases: 256,
        failure_persistence: None,
        max_shrink_iters: 4_096,
        ..Config::default()
    };
    let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[0x8a; 32]);
    let mut runner = TestRunner::new_with_rng(config, rng);
    runner
        .run(&arb_trace(), |trace| {
            {
                let mut coverage = coverage.borrow_mut();
                coverage.longest_trace = coverage.longest_trace.max(trace.len());
                let randomized = &trace[0];
                coverage.randomized_operations[usize::from(randomized.kind)] = true;
                if randomized.kind == 12 {
                    if randomized.salt & 4 == 0 {
                        coverage.randomized_void_node = true;
                    } else {
                        coverage.randomized_opaque_node = true;
                    }
                }
            }
            run_stateful_trace(&trace, &coverage);
            run_scenario(&trace[0], &coverage);
            run_evolving_list_chain(trace[0].salt, &coverage);
            Ok(())
        })
        .unwrap();

    let coverage = coverage.into_inner();
    assert!(coverage.operations.into_iter().all(|seen| seen));
    assert!(coverage.randomized_operations.into_iter().all(|seen| seen));
    assert!(coverage.scalar && coverage.utf16);
    assert!(coverage.before && coverage.after);
    assert!(coverage.custom_root && coverage.void_node && coverage.opaque_node);
    assert!(coverage.randomized_void_node && coverage.randomized_opaque_node);
    assert_eq!(coverage.longest_trace, 100);
}

#[test]
fn one_transaction_maps_same_base_utf16_position_by_before_and_after_affinity() {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "A😀B" }] }]
    });
    let base = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let base_pos = legacy_position(&base, &schema, 2);

    for (request_id, second_affinity, second_pos, expected_text) in [
        (20_001, Affinity::Before, base_pos, "A😀yxB"),
        (20_002, Affinity::After, base_pos + 1, "A😀xyB"),
    ] {
        let mut legacy_transaction = Transaction::new(Source::Api);
        legacy_transaction.add_step(Step::InsertText {
            pos: base_pos,
            text: "x".into(),
            marks: vec![],
        });
        legacy_transaction.add_step(Step::InsertText {
            pos: second_pos,
            text: "y".into(),
            marks: vec![],
        });
        let expected = legacy_transaction.apply(&base, &schema).unwrap().0;
        assert_eq!(expected.root().text_content(), expected_text);

        let point = |affinity| RevisionedPosition {
            offset: 3,
            kind: EditorOffsetKind::Utf16,
            affinity,
        };
        let mut yrs = engine(schema.clone());
        yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        let commit = yrs
            .apply_typed_transaction(transaction_with_operations(
                &yrs,
                request_id,
                vec![
                    TypedOperation::InsertText {
                        at: point(Affinity::After),
                        text: "x".into(),
                        marks: vec![],
                    },
                    TypedOperation::InsertText {
                        at: point(second_affinity),
                        text: "y".into(),
                        marks: vec![],
                    },
                ],
            ))
            .unwrap();
        assert!(commit.changed);
        assert_eq!(yrs.document(), Some(&expected));
        assert_encoded_state_matches(&yrs, &expected, &schema);
    }
}

#[test]
fn no_op_and_rejection_classes_match_the_legacy_oracle_without_state_changes() {
    let schema = tiptap_schema();
    let source = serde_json::json!({
        "type": "doc",
        "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": "a😀bcdef" }] }]
    });
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = rendered_text(&document, &schema);
    let coverage = RefCell::new(Coverage::default());

    let mut yrs = engine(schema.clone());
    yrs.import_json(&source.to_string(), TransactionOrigin::DocumentImport)
        .unwrap();
    let before = (
        yrs.encoded_state().unwrap(),
        yrs.document_json(),
        yrs.document_html(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let no_op = yrs
        .apply_typed_transaction(transaction(
            &yrs,
            9_100,
            TypedOperation::DeleteRange {
                range: range(&rendered, 2, 2, 0, &coverage),
            },
        ))
        .unwrap();
    assert!(!no_op.changed);
    let mut legacy_no_op = Transaction::new(Source::Api);
    legacy_no_op.add_step(Step::DeleteRange {
        from: legacy_position(&document, &schema, 2),
        to: legacy_position(&document, &schema, 2),
    });
    assert_eq!(legacy_no_op.apply(&document, &schema).unwrap().0, document);
    assert_eq!(
        (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision()
        ),
        before
    );

    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::DeleteRange {
        from: legacy_position(&document, &schema, 4),
        to: legacy_position(&document, &schema, 1),
    });
    let transform_error = legacy.apply(&document, &schema).unwrap_err();
    for (index, (kind, affinity)) in [
        (EditorOffsetKind::Scalar, Affinity::Before),
        (EditorOffsetKind::Scalar, Affinity::After),
        (EditorOffsetKind::Utf16, Affinity::Before),
        (EditorOffsetKind::Utf16, Affinity::After),
    ]
    .into_iter()
    .enumerate()
    {
        let offset = |scalar| match kind {
            EditorOffsetKind::Scalar => scalar,
            EditorOffsetKind::Utf16 => scalar_offset_to_utf16(&rendered, scalar).unwrap(),
        };
        let reverse = TypedOperation::DeleteRange {
            range: RevisionedRange {
                from: RevisionedPosition {
                    offset: offset(4),
                    kind,
                    affinity,
                },
                to: RevisionedPosition {
                    offset: offset(1),
                    kind,
                    affinity,
                },
            },
        };
        let before = (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision(),
        );
        let operation_error = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                9_101 + u64::try_from(index).unwrap(),
                reverse,
            ))
            .unwrap_err();
        assert_eq!(
            operation_error_class(&operation_error),
            transform_error_class(&transform_error)
        );
        assert_eq!(
            (
                yrs.encoded_state().unwrap(),
                yrs.document_json(),
                yrs.document_html(),
                yrs.revision(),
                yrs.state_revision(),
            ),
            before
        );
    }

    let before = (
        yrs.encoded_state().unwrap(),
        yrs.document_json(),
        yrs.document_html(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let invalid_position = TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 99,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "x".into(),
        marks: vec![],
    };
    let operation_error = yrs
        .apply_typed_transaction(transaction(&yrs, 9_102, invalid_position))
        .unwrap_err();
    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::InsertText {
        pos: document.root().node_size() + 1,
        text: "x".into(),
        marks: vec![],
    });
    let transform_error = legacy.apply(&document, &schema).unwrap_err();
    assert_eq!(
        operation_error_class(&operation_error),
        transform_error_class(&transform_error)
    );
    assert_eq!(
        (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision()
        ),
        before
    );

    let before = (
        yrs.encoded_state().unwrap(),
        yrs.document_json(),
        yrs.document_html(),
        yrs.revision(),
        yrs.state_revision(),
    );
    let invalid_mark = Mark::new("notDeclared".into(), HashMap::new());
    let invalid_content = TypedOperation::InsertText {
        at: RevisionedPosition {
            offset: 2,
            kind: EditorOffsetKind::Scalar,
            affinity: Affinity::After,
        },
        text: "x".into(),
        marks: vec![invalid_mark.clone()],
    };
    let operation_error = yrs
        .apply_typed_transaction(transaction(&yrs, 9_200, invalid_content))
        .unwrap_err();
    let mut legacy = Transaction::new(Source::Api);
    legacy.add_step(Step::InsertText {
        pos: legacy_position(&document, &schema, 2),
        text: "x".into(),
        marks: vec![invalid_mark],
    });
    let transform_error = legacy.apply(&document, &schema).unwrap_err();
    assert_eq!(
        operation_error_class(&operation_error),
        transform_error_class(&transform_error)
    );
    assert_eq!(
        operation_error_class(&operation_error),
        ErrorClass::InvalidContent
    );
    assert_eq!(
        (
            yrs.encoded_state().unwrap(),
            yrs.document_json(),
            yrs.document_html(),
            yrs.revision(),
            yrs.state_revision()
        ),
        before
    );
}
