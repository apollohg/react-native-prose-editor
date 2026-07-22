use std::cell::RefCell;
use std::collections::HashMap;

use crate::boundary::ResourceLimits;
use crate::model::{Document, Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::render::RenderElement;
use crate::schema::{NodeRole, Schema};
use crate::serialize::{from_prosemirror_json, to_html, to_prosemirror_json, UnknownTypeMode};
use crate::tiptap_schema;
use crate::transform::{DocumentValidator, Source, Step, Transaction, TransformError};
use crate::yrs_engine::{
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
    let blocks = crate::render::incremental::render_blocks(document, schema);
    let elements = crate::render::incremental::flatten_render_blocks(&blocks);
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
                        crate::render::task_list_marker_string(context.checked.unwrap_or(false))
                    } else {
                        crate::render::list_marker_string(context.ordered, context.index)
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
            } => text.push_str(&crate::render::opaque_atom_visible_string(
                &node_type, &label,
            )),
            RenderElement::OpaqueBlockAtom {
                node_type, label, ..
            } => {
                begin_block(&mut text, &mut started_block);
                text.push_str(&crate::render::opaque_atom_visible_string(
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

fn assert_installed_position_map_matches_full_build(
    engine: &YrsDocumentEngine,
    schema: &Schema,
    context: &str,
) {
    let document = engine.document().unwrap();
    let installed = engine.position_map().unwrap();
    let full = PositionMap::build(document, schema);

    assert_eq!(
        installed.block_count(),
        full.block_count(),
        "{context}: block count"
    );
    assert_eq!(
        installed.total_scalars(),
        full.total_scalars(),
        "{context}: total scalars"
    );
    for index in 0..full.block_count() {
        let installed_block = installed.block(index).unwrap();
        let full_block = full.block(index).unwrap();
        assert_eq!(
            installed_block.doc_start, full_block.doc_start,
            "{context}: block {index} doc_start"
        );
        assert_eq!(
            installed_block.doc_end, full_block.doc_end,
            "{context}: block {index} doc_end"
        );
        assert_eq!(
            installed_block.scalar_start, full_block.scalar_start,
            "{context}: block {index} scalar_start"
        );
        assert_eq!(
            installed_block.scalar_len, full_block.scalar_len,
            "{context}: block {index} scalar_len"
        );
        assert_eq!(
            installed_block.scalar_prefix_len, full_block.scalar_prefix_len,
            "{context}: block {index} scalar_prefix_len"
        );
        assert_eq!(
            installed_block.rendered_break_after, full_block.rendered_break_after,
            "{context}: block {index} rendered_break_after"
        );
        assert_eq!(
            installed_block.node_path, full_block.node_path,
            "{context}: block {index} node_path"
        );
        assert_eq!(
            installed_block.is_void_block, full_block.is_void_block,
            "{context}: block {index} is_void_block"
        );
    }
    for scalar in 0..=full.total_scalars() {
        assert_eq!(
            installed.scalar_to_doc(scalar, document),
            full.scalar_to_doc(scalar, document),
            "{context}: scalar offset {scalar}"
        );
    }
    for position in 0..=document.content_size() {
        assert_eq!(
            installed.doc_to_scalar(position, document),
            full.doc_to_scalar(position, document),
            "{context}: document position {position}"
        );
    }
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
    assert_installed_position_map_matches_full_build(
        &yrs,
        &schema,
        &format!("operation kind {}", spec.kind),
    );
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
        let local_state_before = (
            yrs.relative_selection().cloned(),
            yrs.resolved_selection().cloned(),
            yrs.stored_marks().map(<[Mark]>::to_vec),
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
        let operation_debug = format!("{operation:?}");
        let commit = yrs
            .apply_typed_transaction(transaction(
                &yrs,
                10_000 + u64::try_from(index).unwrap(),
                operation,
            ))
            .unwrap_or_else(|error| panic!("Yrs trace step {index} ({spec:?}) failed: {error:?}"));
        assert_installed_position_map_matches_full_build(
            &yrs,
            &schema,
            &format!("trace step {index} ({spec:?})"),
        );
        let local_state_after = (
            yrs.relative_selection().cloned(),
            yrs.resolved_selection().cloned(),
            yrs.stored_marks().map(<[Mark]>::to_vec),
        );
        let local_state_changed = local_state_after != local_state_before;
        let expected_commit_changed = expected_changed || local_state_changed;
        assert_eq!(
            commit.changed, expected_commit_changed,
            "trace step {index}: {spec:?}; operation={operation_debug}; before_revisions=({}, {}); after=({}, {}, {:?}, {:?})",
            engine_before.1,
            engine_before.2,
            yrs.revision(),
            yrs.state_revision(),
            yrs.stored_marks(),
            yrs.resolved_selection(),
        );
        assert_eq!(
            yrs.revision(),
            engine_before.1 + u64::from(expected_changed),
            "document revision at trace step {index}: {spec:?}"
        );
        assert_eq!(
            yrs.state_revision(),
            engine_before.2 + u64::from(expected_commit_changed),
            "state revision at trace step {index}: {spec:?}"
        );
        if !expected_commit_changed {
            assert_eq!(
                (
                    yrs.encoded_state().unwrap(),
                    yrs.revision(),
                    yrs.state_revision()
                ),
                engine_before,
                "no-op trace step {index}: {spec:?}"
            );
        } else if !expected_changed {
            assert_eq!(
                yrs.encoded_state().unwrap(),
                engine_before.0,
                "state-only trace step wrote Yrs content at {index}: {spec:?}"
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
    assert_installed_position_map_matches_full_build(&yrs, &schema, "custom root insert");
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
        assert_installed_position_map_matches_full_build(
            &yrs,
            &schema,
            &format!("evolving list operation {operation_index}"),
        );
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

mod whole_root_replacement {
    use std::collections::HashMap;

    use crate::boundary::ResourceLimits;
    use crate::tiptap_schema;
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, ReplacementHistory, TransactionOrigin,
        YrsDocumentEngine, YrsEngineConfig,
    };
    use proptest::prelude::*;
    use yrs::updates::decoder::Decode;
    use yrs::{Doc, GetString, ReadTxn, Transact, Update};

    fn engine_with_document(json: &serde_json::Value) -> YrsDocumentEngine {
        let mut engine = YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::LocalEmpty,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: None,
        })
        .unwrap();
        engine
            .import_json(&json.to_string(), TransactionOrigin::DocumentImport)
            .unwrap();
        engine
    }

    fn paragraphs(texts: &[String]) -> serde_json::Value {
        serde_json::json!({
            "type": "doc",
            "content": texts
                .iter()
                .map(|text| {
                    if text.is_empty() {
                        serde_json::json!({ "type": "paragraph" })
                    } else {
                        serde_json::json!({
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": text }],
                        })
                    }
                })
                .collect::<Vec<_>>(),
        })
    }

    fn state_vector_entries(update: &[u8]) -> HashMap<u64, u32> {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut();
            txn.apply_update(Update::decode_v1(update).unwrap())
                .unwrap();
        }
        let txn = doc.transact();
        let vector = txn.state_vector();
        vector
            .iter()
            .map(|(client, clock)| (client.get(), *clock))
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn root_replacement_is_same_store_import_equivalent_and_convergent(
            before in proptest::collection::vec("[a-z]{0,6}", 1..4usize),
            after in proptest::collection::vec("[a-z]{0,6}", 1..4usize),
            reset in proptest::bool::ANY,
        ) {
            let before_doc = paragraphs(&before);
            let after_doc = paragraphs(&after);
            let mut engine = engine_with_document(&before_doc);
            let reference = engine_with_document(&after_doc);
            let expected_changed = engine.document_json() != reference.document_json();

            let base_state = engine.encoded_state().unwrap();
            let writer = engine.client_id();
            let history = if reset {
                ReplacementHistory::ResetAndClear
            } else {
                ReplacementHistory::UndoableBoundary
            };

            let commit = engine
                .prepare_root_replacement_json(7, &after_doc.to_string(), history)
                .unwrap();

            // Replacement is canonically equivalent to a fresh import of the
            // same target document.
            prop_assert_eq!(commit.changed, expected_changed);
            prop_assert_eq!(engine.document_json(), reference.document_json());
            prop_assert_eq!(engine.client_id(), writer);

            // Same-store: the writer's clock strictly advances on change and
            // no other state-vector entry moves.
            let after_state = engine.encoded_state().unwrap();
            let before_entries = state_vector_entries(&base_state);
            let after_entries = state_vector_entries(&after_state);
            if expected_changed {
                prop_assert!(
                    after_entries.get(&writer).copied().unwrap_or(0)
                        > before_entries.get(&writer).copied().unwrap_or(0)
                );
            } else {
                prop_assert_eq!(&after_entries, &before_entries);
            }
            for (client, clock) in &before_entries {
                if *client != writer {
                    prop_assert_eq!(after_entries.get(client), Some(clock));
                }
            }

            // Standard incremental convergence: a peer holding the prior
            // state converges through the base->after Update-v1 alone.
            let peer = Doc::new();
            let peer_fragment = peer.get_or_insert_xml_fragment("prosemirror");
            {
                let mut txn = peer.transact_mut();
                txn.apply_update(Update::decode_v1(&base_state).unwrap()).unwrap();
            }
            let base_vector = peer.transact().state_vector();
            let replica = Doc::new();
            let replica_fragment = replica.get_or_insert_xml_fragment("prosemirror");
            {
                let mut txn = replica.transact_mut();
                txn.apply_update(Update::decode_v1(&after_state).unwrap()).unwrap();
            }
            let incremental = replica.transact().encode_state_as_update_v1(&base_vector);
            {
                let mut txn = peer.transact_mut();
                txn.apply_update(Update::decode_v1(&incremental).unwrap()).unwrap();
            }
            {
                let peer_txn = peer.transact();
                let replica_txn = replica.transact();
                prop_assert_eq!(peer_txn.state_vector(), replica_txn.state_vector());
                prop_assert_eq!(
                    peer_fragment.get_string(&peer_txn),
                    replica_fragment.get_string(&replica_txn)
                );
            }

            // Exact history policy per mode.
            if reset {
                prop_assert!(!engine.can_undo());
                prop_assert!(!engine.can_redo());
            } else {
                prop_assert_eq!(engine.can_undo(), expected_changed);
                if expected_changed {
                    prop_assert!(engine.undo(8).unwrap().is_some());
                    let restored = engine_with_document(&before_doc);
                    prop_assert_eq!(engine.document_json(), restored.document_json());
                }
            }
        }
    }
}

/// Task 7 property extension: every durable local path — typed input
/// transaction, command, undo, redo, replace (`UndoableBoundary`), and reset
/// (`ResetAndClear`) — reserves a conservative outbound bound before the
/// irreversible Yrs write and captures an incremental Update-v1 whose length
/// never exceeds that admitted bound, while a twin replica fed only by the
/// captured outbox entries converges exactly. Selection requests reserve and
/// enqueue nothing.
mod outbound_update_bounds {
    use crate::boundary::ResourceLimits;
    use crate::native_bridge_test_support::{self as bridge, BridgeTestOutcome, SessionOptions};
    use crate::tiptap_schema;
    use crate::yrs_engine::{
        EditingLimits, InitializationMode, YrsDocumentEngine, YrsEngineConfig,
    };
    use proptest::prelude::*;

    fn paragraphs_json(texts: &[String]) -> String {
        serde_json::json!({
            "type": "doc",
            "content": texts
                .iter()
                .map(|text| {
                    if text.is_empty() {
                        serde_json::json!({ "type": "paragraph" })
                    } else {
                        serde_json::json!({
                            "type": "paragraph",
                            "content": [{ "type": "text", "text": text }],
                        })
                    }
                })
                .collect::<Vec<_>>(),
        })
        .to_string()
    }

    fn twin_replica() -> YrsDocumentEngine {
        // Content-free AwaitRemote replica: converges exclusively from the
        // captured outbox entries.
        YrsDocumentEngine::new(YrsEngineConfig {
            schema: tiptap_schema(),
            fragment_name: "prosemirror".into(),
            initialization_mode: InitializationMode::AwaitRemote,
            resource_limits: ResourceLimits::default(),
            editing_limits: EditingLimits::default(),
            max_length: None,
            scope: Some(crate::yrs_engine::DocumentScope {
                document_id: "bounds-twin".into(),
                lineage_id: "bounds-twin-lineage".into(),
            }),
        })
        .unwrap()
    }

    fn revision(id: u64) -> u64 {
        bridge::session_audit(id).unwrap().document_revision
    }

    fn text_fragment() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z]{1,6}",
            "[\u{1F600}-\u{1F604}]{1,3}",
            "[\u{e9}-\u{ef}]{1,4}",
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        #[test]
        fn captured_updates_stay_within_admitted_bounds_and_converge(
            seed_texts in proptest::collection::vec(text_fragment(), 1..3usize),
            typed in text_fragment(),
            typed_second in text_fragment(),
            replace_texts in proptest::collection::vec(text_fragment(), 1..3usize),
            reset_texts in proptest::collection::vec(text_fragment(), 1..3usize),
        ) {
            let id = bridge::create_session(SessionOptions {
                initial_json: Some(paragraphs_json(&seed_texts)),
                attach_runtime: true,
                ..SessionOptions::default()
            })
            .unwrap();

            let mut twin = twin_replica();
            let mut replay = 50_000u64;
            let initial = bridge::session_audit(id).unwrap().encoded_state.unwrap();
            twin.apply_remote_update_v1(replay, &initial).unwrap();

            let mut replay_one = |twin: &mut YrsDocumentEngine,
                                  label: &str|
             -> Result<(), TestCaseError> {
                let bound = bridge::last_reserved_upper_bound(id).unwrap();
                let bound = bound.unwrap_or_else(|| panic!("{label}: missing reservation"));
                let (_, update) = bridge::take_next_update(id).unwrap()
                    .unwrap_or_else(|| panic!("{label}: missing outbox entry"));
                prop_assert!(
                    update.len() <= bound,
                    "{} captured {} bytes above admitted bound {}",
                    label,
                    update.len(),
                    bound,
                );
                replay += 1;
                twin.apply_remote_update_v1(replay, &update).unwrap();
                prop_assert_eq!(
                    twin.document_json(),
                    bridge::session_audit(id).unwrap().document_json,
                    "{} twin replica must converge from the captured update",
                    label,
                );
                prop_assert!(
                    bridge::take_next_update(id).unwrap().is_none(),
                    "{} must enqueue exactly one entry",
                    label,
                );
                Ok(())
            };

            // Typed local-input transaction.
            let outcome = bridge::submit_input(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "1",
                    "baseDocumentRevision": revision(id).to_string(),
                    "text": typed,
                })
                .to_string(),
            )
            .unwrap();
            let changed_transaction = matches!(
                outcome,
                BridgeTestOutcome::Transaction { changed: true, .. }
            );
            prop_assert!(changed_transaction);
            replay_one(&mut twin, "input")?;

            // Selection/state-only request: reserves nothing, enqueues nothing.
            bridge::submit_selection(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "2",
                    "baseDocumentRevision": revision(id).to_string(),
                    "selection": {
                        "type": "text",
                        "anchor": { "offset": 0, "kind": "scalar" },
                        "head": { "offset": 0, "kind": "scalar" },
                    },
                })
                .to_string(),
            )
            .unwrap();
            prop_assert_eq!(bridge::outbox_pending(id).unwrap(), Some((0, 0)));

            // Command.
            let outcome = bridge::submit_command(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "3",
                    "baseDocumentRevision": revision(id).to_string(),
                    "command": { "type": "toggleBlockquote" },
                })
                .to_string(),
            )
            .unwrap();
            let changed_transaction = matches!(
                outcome,
                BridgeTestOutcome::Transaction { changed: true, .. }
            );
            prop_assert!(changed_transaction);
            replay_one(&mut twin, "command")?;

            // Second input so undo has a mixed-history group to pop.
            bridge::submit_input(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "4",
                    "baseDocumentRevision": revision(id).to_string(),
                    "text": typed_second,
                })
                .to_string(),
            )
            .unwrap();
            replay_one(&mut twin, "input-second")?;

            // Undo and redo.
            prop_assert!(bridge::undo(id, 5).unwrap());
            replay_one(&mut twin, "undo")?;
            prop_assert!(bridge::redo(id, 6).unwrap());
            replay_one(&mut twin, "redo")?;

            // Whole-document replace: one undoable local-API boundary.
            bridge::submit_local_api(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "7",
                    "baseDocumentRevision": revision(id).to_string(),
                    "setJson": serde_json::from_str::<serde_json::Value>(
                        &paragraphs_json(&replace_texts),
                    )
                    .unwrap(),
                    "history": "undoableBoundary",
                })
                .to_string(),
            )
            .unwrap();
            if bridge::outbox_pending(id).unwrap() == Some((0, 0)) {
                // Identical replacement content is an unchanged commit and
                // must not enqueue an update.
                prop_assert_eq!(
                    twin.document_json(),
                    bridge::session_audit(id).unwrap().document_json,
                );
            } else {
                replay_one(&mut twin, "replace")?;
            }

            // Reset: non-undoable, clears history, still one bounded entry.
            bridge::submit_local_api(
                id,
                &serde_json::json!({
                    "version": 1,
                    "requestId": "8",
                    "baseDocumentRevision": revision(id).to_string(),
                    "setJson": serde_json::from_str::<serde_json::Value>(
                        &paragraphs_json(&reset_texts),
                    )
                    .unwrap(),
                    "history": "resetAndClear",
                })
                .to_string(),
            )
            .unwrap();
            let audit = bridge::session_audit(id).unwrap();
            prop_assert!(!audit.can_undo);
            if bridge::outbox_pending(id).unwrap() == Some((0, 0)) {
                prop_assert_eq!(twin.document_json(), audit.document_json);
            } else {
                replay_one(&mut twin, "reset")?;
            }

            bridge::destroy_session(id);
        }
    }
}
