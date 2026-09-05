use crate::boundary::ResourceLimits;
use std::collections::HashMap;
use std::sync::Arc;

use proptest::prelude::*;
use serde_json::{json, Value};
use yrs::branch::{Branch, BranchPtr};
use yrs::types::text::Text;
use yrs::types::xml::{XmlElementPrelim, XmlFragment, XmlOut, XmlTextPrelim, XmlTextRef};
use yrs::types::Attrs;
use yrs::undo::UndoManager;
use yrs::updates::decoder::Decode;
use yrs::Any;
use yrs::{
    ArrayPrelim, Assoc, Doc, MapPrelim, OffsetKind, Options, ReadTxn, StateVector, StickyIndex,
    Transact, Update, WriteTxn, Xml,
};

use crate::model::{Document, Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::schema::presets::tiptap_schema;
use crate::schema::Schema;
use crate::serialize::{
    from_html, from_prosemirror_json, rehydrate_reserved_html_opaque, to_html, to_prosemirror_json,
    FromHtmlOptions, UnknownTypeMode,
};
use crate::transform::DocumentValidator;

use super::codec::{PreparedXmlChild, PreparedXmlNode};
use super::compiler::{compile_transaction_with_yrs, CompilationContext, CompiledTransaction};
use super::mutation::{
    crdt_clock_scan_reservation, crdt_envelope, direct_root_wrap_metrics,
    direct_xml_replacement_growth, execute_mutation_plan, preflight_mutation_plan,
    preflight_mutation_work_for_test, TextRangeDisposition, YrsMutationAction,
};
use super::YrsDocumentCodec;
use super::{
    Affinity, EditingLimits, EditorOffsetKind, HistoryPolicy, InitializationMode,
    RevisionedPosition, SelectionIntent, TransactionOrigin, TypedOperation, TypedTransaction,
    YrsDocumentEngine, YrsEngineConfig,
};

const TWO_PARAGRAPHS: &str = r#"{
  "type":"doc",
  "content":[
    {"type":"paragraph","content":[{"type":"text","text":"alpha"}]},
    {"type":"paragraph","content":[{"type":"text","text":"omega"}]}
  ]
}"#;

use super::{
    codec, doc_pos_to_relative_point, mutation, relative_point_to_doc_pos, sticky_index_to_doc_pos,
};

fn utf16_doc() -> Doc {
    Doc::with_options(Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    })
}

fn compile_and_execute(
    source: Value,
    operations: Vec<TypedOperation>,
) -> (Value, Value, String, usize, usize) {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let before_update = {
        let txn = doc.transact();
        txn.encode_state_as_update_v1(&StateVector::default())
    };
    let before_full_len = before_update.len();
    let compiled = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits: &limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 71,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let expected_json = to_prosemirror_json(&compiled.preview, &schema);
    let expected_html = to_html(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    let undo_bound = compiled.undo_units_bound;
    let has_actions = !compiled.mutation_plan.actions.is_empty();
    let fragment = doc.transact().get_xml_fragment("prosemirror").unwrap();
    let mut undo = UndoManager::<()>::new();
    undo.expand_scope(&doc, &fragment);
    let update = if has_actions {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
        txn.commit();
        txn.encode_update_v1()
    } else {
        Vec::new()
    };
    if has_actions {
        let item = undo
            .undo_stack()
            .last()
            .expect("a changed helper transaction must be captured by UndoManager");
        let units = |set: &yrs::IdSet| {
            set.iter()
                .flat_map(|(_, ranges)| ranges.into_iter())
                .map(|range| u64::from(range.end - range.start))
                .sum::<u64>()
        };
        let actual_undo_units = units(item.insertions()) + units(item.deletions());
        assert!(
            actual_undo_units <= undo_bound,
            "actual undo units {actual_undo_units} > {undo_bound}"
        );
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual_json = codec.read_json(&fragment, &txn).unwrap();
    let update_len = update.len();
    let after_full_len = txn.encode_state_as_update_v1(&StateVector::default()).len();
    assert_eq!(actual_json, expected_json);
    let actual_document = rehydrate_reserved_html_opaque(
        &from_prosemirror_json(&actual_json, &schema, UnknownTypeMode::Preserve).unwrap(),
    );
    assert_eq!(actual_document, compiled.preview);
    assert_eq!(to_html(&actual_document, &schema), expected_html);
    assert!(update_len <= estimate, "{update_len} > {estimate}");
    assert!(
        after_full_len <= before_full_len + estimate,
        "full state grew {} > {estimate}",
        after_full_len.saturating_sub(before_full_len)
    );

    let replica = utf16_doc();
    {
        let mut replica_txn = replica.transact_mut();
        replica_txn
            .apply_update(Update::decode_v1(&before_update).unwrap())
            .unwrap();
        if has_actions {
            replica_txn
                .apply_update(Update::decode_v1(&update).unwrap())
                .unwrap();
        }
    }
    let replica_txn = replica.transact();
    let replica_fragment = replica_txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(
        codec.read_json(&replica_fragment, &replica_txn).unwrap(),
        actual_json
    );
    assert_eq!(replica_txn.state_vector(), txn.state_vector());
    (
        actual_json,
        expected_json,
        expected_html,
        update_len,
        estimate,
    )
}

fn diagnostic_doc(
    source: &Value,
) -> (
    Doc,
    crate::schema::Schema,
    ResourceLimits,
    EditingLimits,
    crate::model::Document,
) {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let document = from_prosemirror_json(source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), source)
            .unwrap();
    }
    (doc, schema, limits, editing_limits, document)
}

fn compile_operations_with_schema(
    source: &Value,
    operations: Vec<TypedOperation>,
    schema: crate::schema::Schema,
) -> (
    Doc,
    crate::schema::Schema,
    ResourceLimits,
    CompiledTransaction,
) {
    let (doc, schema, limits, editing_limits, document) =
        diagnostic_doc_with_schema(source, schema);
    let compiled = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits: &limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 122,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    (doc, schema, limits, compiled)
}

fn compile_operations_with_undo_limit(
    source: &Value,
    operations: Vec<TypedOperation>,
    schema: crate::schema::Schema,
    max_undo_retained_units: u64,
) -> super::OperationResult<CompiledTransaction> {
    let (doc, schema, limits, mut editing_limits, document) =
        diagnostic_doc_with_schema(source, schema);
    editing_limits.max_undo_retained_units = max_undo_retained_units;
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    compile_transaction_with_yrs(
        CompilationContext {
            document: &document,
            selection: None,
            schema: &schema,
            resource_limits: &limits,
            editing_limits: &editing_limits,
            document_revision: 0,
            max_length: None,
        },
        TypedTransaction {
            request_id: 123,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalCommand,
            operations,
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
}

fn attribute_schema() -> crate::schema::Schema {
    crate::schema::Schema::from_json(&json!({
        "nodes": [
            { "name": "doc", "content": "block+", "role": "doc" },
            { "name": "paragraph", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "p" },
            { "name": "blockquote", "content": "block+", "group": "block", "role": "block", "htmlTag": "blockquote" },
            { "name": "h2", "content": "inline*", "group": "block", "role": "textBlock", "htmlTag": "h2", "attrs": { "id": { "default": null } } },
            { "name": "codeBlock", "content": "text*", "group": "block", "role": "textBlock", "htmlTag": "pre", "attrs": { "language": { "default": null } } },
            { "name": "hardBreak", "content": "", "group": "inline", "role": "hardBreak", "htmlTag": "br", "isVoid": true },
            { "name": "inlineWidget", "content": "", "group": "inline", "role": "inline", "htmlTag": "span", "isVoid": true, "attrs": { "id": { "default": null }, "meta": { "default": null } } },
            { "name": "taskList", "content": "taskItem+", "group": "block", "role": "list", "htmlTag": "ul", "attrs": { "listMeta": { "default": null } } },
            { "name": "taskItem", "content": "paragraph block*", "role": "listItem", "htmlTag": "li", "attrs": { "checked": { "default": null }, "itemMeta": { "default": null } } },
            { "name": "image", "content": "", "group": "block", "role": "block", "htmlTag": "img", "isVoid": true, "attrs": { "src": {}, "alt": { "default": null } } },
            { "name": "customBlock", "content": "", "group": "block", "role": "block", "htmlTag": "aside", "isVoid": true, "allowUndeclaredAttrs": true },
            { "name": "text", "group": "inline", "role": "text" }
        ],
        "marks": [{ "name": "bold", "htmlTag": "strong" }]
    }))
    .unwrap()
}

fn diagnostic_doc_with_schema(
    source: &Value,
    schema: crate::schema::Schema,
) -> (
    Doc,
    crate::schema::Schema,
    ResourceLimits,
    EditingLimits,
    crate::model::Document,
) {
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let document = from_prosemirror_json(source, &schema, UnknownTypeMode::Preserve).unwrap();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), source)
            .unwrap();
    }
    (doc, schema, limits, editing_limits, document)
}

fn point_for_test(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
}

fn rendered_scalar_offset(source: &Value, schema: &crate::schema::Schema, needle: &str) -> u32 {
    let document = from_prosemirror_json(source, schema, UnknownTypeMode::Preserve).unwrap();
    let rendered = crate::render::rendered_text(&document, schema);
    u32::try_from(rendered[..rendered.find(needle).unwrap()].chars().count()).unwrap()
}

fn prepared_text_for_test(runs: &[super::codec::PreparedTextRun]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

fn range_for_test(from: u32, to: u32) -> super::RevisionedRange {
    super::RevisionedRange {
        from: point_for_test(from),
        to: point_for_test(to),
    }
}

fn paragraph_text<T: ReadTxn>(
    fragment: &yrs::types::xml::XmlFragmentRef,
    txn: &T,
    index: u32,
) -> XmlTextRef {
    let XmlOut::Element(paragraph) = fragment.get(txn, index).unwrap() else {
        panic!("paragraph expected")
    };
    let XmlOut::Text(text) = paragraph.get(txn, 0).unwrap() else {
        panic!("text expected")
    };
    text
}

#[path = "mutation_tests/block_structure.rs"]
mod block_structure;
#[path = "mutation_tests/direct_operations.rs"]
mod direct_operations;
#[path = "mutation_tests/list_structure.rs"]
mod list_structure;
#[path = "mutation_tests/preflight_and_properties.rs"]
mod preflight_and_properties;
