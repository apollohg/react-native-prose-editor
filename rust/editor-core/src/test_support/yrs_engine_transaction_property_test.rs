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

include!("yrs_engine_transaction_property_test/scenarios.rs");

include!("yrs_engine_transaction_property_test/stateful_traces.rs");

include!("yrs_engine_transaction_property_test/oracle_properties.rs");
