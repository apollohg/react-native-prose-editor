use crate::boundary::ResourceLimits;
use std::collections::HashMap;

use proptest::prelude::*;
use serde_json::{json, Value};
use yrs::branch::{Branch, BranchPtr};
use yrs::types::text::Text;
use yrs::types::xml::{XmlElementPrelim, XmlFragment, XmlOut, XmlTextPrelim, XmlTextRef};
use yrs::types::Attrs;
use yrs::updates::decoder::Decode;
use yrs::Any;
use yrs::{
    Assoc, Doc, OffsetKind, Options, ReadTxn, StateVector, StickyIndex, Transact, Update, WriteTxn,
};

use crate::model::{Fragment, Mark, Node};
use crate::position::PositionMap;
use crate::schema::presets::tiptap_schema;
use crate::serialize::{from_prosemirror_json, to_html, to_prosemirror_json, UnknownTypeMode};

use super::compiler::{compile_transaction_with_yrs, CompilationContext};
use super::mutation::{
    execute_mutation_plan, preflight_mutation_plan, preflight_mutation_work_for_test,
    YrsMutationAction,
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

fn engine() -> YrsDocumentEngine {
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
        .import_json(TWO_PARAGRAPHS, TransactionOrigin::DocumentImport)
        .unwrap();
    engine
}

#[test]
fn one_character_insert_compiles_a_direct_mutation_action() {
    let engine = engine();
    let compiled = engine
        .compile_typed_transaction(TypedTransaction {
            request_id: 51,
            base_document_revision: engine.revision(),
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::InsertText {
                at: RevisionedPosition {
                    offset: 1,
                    kind: EditorOffsetKind::Scalar,
                    affinity: Affinity::After,
                },
                text: "!".into(),
                marks: vec![],
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        })
        .unwrap();

    assert_eq!(compiled.preview.root().text_content(), "a!lphaomega");
    assert_eq!(compiled.mutation_plan.actions.len(), 1);
    assert!(compiled.encoded_growth_bound > 0);
}

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
    let (before_vector, before_update) = {
        let txn = doc.transact();
        (
            txn.state_vector(),
            txn.encode_state_as_update_v1(&StateVector::default()),
        )
    };
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
    let has_actions = !compiled.mutation_plan.actions.is_empty();
    if has_actions {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual_json = codec.read_json(&fragment, &txn).unwrap();
    let update = if has_actions {
        txn.encode_state_as_update_v1(&before_vector)
    } else {
        Vec::new()
    };
    let update_len = update.len();
    assert_eq!(actual_json, expected_json);
    let actual_document =
        from_prosemirror_json(&actual_json, &schema, UnknownTypeMode::Preserve).unwrap();
    assert_eq!(actual_document, compiled.preview);
    assert_eq!(to_html(&actual_document, &schema), expected_html);
    assert!(update_len <= estimate, "{update_len} > {estimate}");

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

#[test]
fn every_text_and_mark_operation_executes_to_its_exact_preview() {
    let plain = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
    let bold = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Hello",
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let link = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "Hello",
                "marks": [{ "type": "link", "attrs": { "href": "old" } }]
            }]
        }]
    });
    let cases = vec![
        (
            plain.clone(),
            TypedOperation::InsertText {
                at: point_for_test(2),
                text: "🙂".into(),
                marks: vec![Mark::new("bold".into(), HashMap::new())],
            },
        ),
        (
            plain.clone(),
            TypedOperation::DeleteRange {
                range: range_for_test(1, 4),
            },
        ),
        (
            plain.clone(),
            TypedOperation::ReplaceRange {
                range: range_for_test(1, 4),
                content: Fragment::from(vec![Node::text(
                    "XY".into(),
                    vec![Mark::new("italic".into(), HashMap::new())],
                )]),
            },
        ),
        (
            plain.clone(),
            TypedOperation::AddMark {
                range: range_for_test(1, 4),
                mark: Mark::new("bold".into(), HashMap::new()),
            },
        ),
        (
            bold,
            TypedOperation::RemoveMark {
                range: range_for_test(1, 4),
                mark_type: "bold".into(),
            },
        ),
        (
            link,
            TypedOperation::ReplaceMark {
                range: range_for_test(1, 4),
                mark: Mark::new(
                    "link".into(),
                    HashMap::from([("href".into(), Value::String("new".into()))]),
                ),
            },
        ),
    ];

    for (source, operation) in cases {
        compile_and_execute(source, vec![operation]);
    }
}

#[test]
fn empty_textblocks_create_direct_text_targets_for_insert_and_collapsed_replace() {
    let empty = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let inserted = compile_and_execute(
        empty.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "hello".into(),
            marks: vec![Mark::new("bold".into(), HashMap::new())],
        }],
    );
    assert_eq!(inserted.0["content"][0]["content"][0]["text"], "hello");

    let replaced = compile_and_execute(
        empty,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(1, 1),
            content: Fragment::from(vec![Node::text("world".into(), vec![])]),
        }],
    );
    assert_eq!(replaced.0["content"][0]["content"][0]["text"], "world");
}

#[test]
fn created_text_target_executes_multi_piece_replacement_and_follow_up_edits() {
    let empty = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let pieces = Fragment::from(vec![
        Node::text("ab".into(), vec![Mark::new("bold".into(), HashMap::new())]),
        Node::text(
            "cd".into(),
            vec![Mark::new("italic".into(), HashMap::new())],
        ),
    ]);
    let actual = compile_and_execute(
        empty,
        vec![TypedOperation::ReplaceRange {
            range: range_for_test(1, 1),
            content: pieces,
        }],
    );
    assert_eq!(actual.0["content"][0]["content"][0]["text"], "ab");
    assert_eq!(actual.0["content"][0]["content"][1]["text"], "cd");

    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let (doc, schema, limits, _editing_limits, document) = diagnostic_doc(&source);
    let plan = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            83,
            &txn,
            &fragment,
            &schema,
            1_000,
            limits.max_input_bytes,
            0,
        )
        .unwrap();
        compiler
            .insert(0, 1, "abcd", &[])
            .and_then(|_| compiler.insert(1, 3, "X", &[]))
            .and_then(|_| compiler.delete(2, 2, 3, &[]))
            .and_then(|_| {
                compiler.format(
                    3,
                    1,
                    5,
                    &[1, 5],
                    super::mutation::mark_attr(&Mark::new("bold".into(), HashMap::new())),
                )
            })
            .unwrap();
        let plan = compiler.finish(Some(3)).unwrap();
        preflight_mutation_plan(83, &plan, &txn).unwrap();
        plan
    };
    assert!(matches!(
        plan.actions.first(),
        Some(YrsMutationAction::CreateText { follow_up, .. }) if follow_up.len() == 3
    ));
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let actual = YrsDocumentCodec::new(&schema, &limits)
        .read_json(&fragment, &txn)
        .unwrap();
    let actual = from_prosemirror_json(&actual, &schema, UnknownTypeMode::Preserve).unwrap();
    assert_eq!(actual.root().text_content(), "aXcd");
    assert_eq!(document.root().text_content(), "");
    let marks = actual
        .root()
        .content()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .content()
        .unwrap();
    assert!(marks
        .iter()
        .all(|node| node.marks().iter().any(|mark| mark.mark_type() == "bold")));
}

#[test]
fn atom_only_textblock_supports_both_gaps_and_rejects_atom_crossing() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let before = compile_and_execute(
        source.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(0),
            text: "before".into(),
            marks: vec![],
        }],
    );
    assert_eq!(before.0["content"][0]["content"][0]["text"], "before");
    assert_eq!(before.0["content"][0]["content"][1]["type"], "hardBreak");

    let after = compile_and_execute(
        source.clone(),
        vec![TypedOperation::InsertText {
            at: point_for_test(1),
            text: "after".into(),
            marks: vec![],
        }],
    );
    assert_eq!(after.0["content"][0]["content"][0]["type"], "hardBreak");
    assert_eq!(after.0["content"][0]["content"][1]["text"], "after");

    let both = compile_and_execute(
        source.clone(),
        vec![
            TypedOperation::InsertText {
                at: point_for_test(0),
                text: "L".into(),
                marks: vec![],
            },
            TypedOperation::InsertText {
                at: point_for_test(1),
                text: "R".into(),
                marks: vec![],
            },
        ],
    );
    assert_eq!(both.0["content"][0]["content"][0]["text"], "L");
    assert_eq!(both.0["content"][0]["content"][1]["type"], "hardBreak");
    assert_eq!(both.0["content"][0]["content"][2]["text"], "R");

    let mention_source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "mention",
                "attrs": { "id": "user-1", "label": "Alice" }
            }]
        }]
    });
    let (mention_doc, mention_schema, mention_limits, _, _) = diagnostic_doc(&mention_source);
    let mention_plan = {
        let txn = mention_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            87,
            &txn,
            &fragment,
            &mention_schema,
            1_000,
            mention_limits.max_input_bytes,
            0,
        )
        .unwrap();
        let after_mention = compiler
            .target_positions_for_test()
            .unwrap()
            .last()
            .unwrap()
            .0;
        compiler.insert(0, after_mention, "tail", &[]).unwrap();
        let plan = compiler.finish(Some(0)).unwrap();
        preflight_mutation_plan(87, &plan, &txn).unwrap();
        plan
    };
    {
        let mut txn = mention_doc.transact_mut();
        execute_mutation_plan(mention_plan, &mut txn);
    }
    let txn = mention_doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mention = YrsDocumentCodec::new(&mention_schema, &mention_limits)
        .read_json(&fragment, &txn)
        .unwrap();
    assert_eq!(mention["content"][0]["content"][0]["type"], "mention");
    assert_eq!(mention["content"][0]["content"][1]["text"], "tail");

    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let error = compile_transaction_with_yrs(
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
            request_id: 84,
            base_document_revision: 0,
            origin: TransactionOrigin::LocalInput,
            operations: vec![TypedOperation::DeleteRange {
                range: range_for_test(0, 1),
            }],
            selection_intent: SelectionIntent::UseOperationResult,
            history_policy: HistoryPolicy::Auto,
        },
        &txn,
        &fragment,
    )
    .unwrap_err();
    assert_eq!(error.code, "OPERATION_INVALID");
    assert_eq!(txn.state_vector(), doc.transact().state_vector());
}

#[test]
fn create_text_preflight_rejects_same_count_neighbor_replacement() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "hardBreak" }]
        }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
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
                request_id: 86,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(0),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(parent) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph expected")
        };
        parent.remove_range(&mut txn, 0, 1);
        parent.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let txn = doc.transact();
    let before = txn.state_vector();
    let error = preflight_mutation_plan(86, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), before);
}

#[test]
fn nested_empty_textblock_create_preserves_unaffected_sibling_identity_and_sticky() {
    let source = json!({
        "type": "doc",
        "content": [
            {
                "type": "bulletList",
                "content": [{
                    "type": "listItem",
                    "content": [{ "type": "paragraph" }]
                }]
            },
            {
                "type": "paragraph",
                "content": [{ "type": "text", "text": "omega" }]
            }
        ]
    });
    let schema = tiptap_schema();
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
    let map = PositionMap::build(&document, &schema);
    let insertion_offset = map.doc_to_scalar(map.block(0).unwrap().doc_start, &document);
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let (sibling_id, sticky, before_vector) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let sibling = paragraph_text(&fragment, &txn, 1);
        let id = <XmlTextRef as AsRef<Branch>>::as_ref(&sibling).id();
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&sibling)),
            2,
            Assoc::After,
        )
        .unwrap();
        (id, sticky, txn.state_vector())
    };
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
                request_id: 78,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(insertion_offset),
                    text: "item".into(),
                    marks: vec![Mark::new("italic".into(), HashMap::new())],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(matches!(
        compiled.mutation_plan.actions.first(),
        Some(YrsMutationAction::CreateText { .. })
    ));
    let expected = to_prosemirror_json(&compiled.preview, &schema);
    let estimate = compiled.encoded_growth_bound;
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    assert_eq!(codec.read_json(&fragment, &txn).unwrap(), expected);
    let sibling = paragraph_text(&fragment, &txn, 1);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&sibling).id(),
        sibling_id
    );
    let resolved = sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved.branch.id(), sibling_id);
    assert_eq!(resolved.index, 2);
    assert!(txn.encode_state_as_update_v1(&before_vector).len() <= estimate);
}

#[test]
fn create_text_preflight_rejects_stale_parent_atomically() {
    let source = json!({
        "type": "doc",
        "content": [{ "type": "paragraph" }]
    });
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
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
                request_id: 79,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalApi,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let before = doc.transact().state_vector();
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let XmlOut::Element(parent) = fragment.get(&txn, 0).unwrap() else {
            panic!("paragraph expected")
        };
        parent.insert(&mut txn, 0, XmlElementPrelim::empty("hardBreak"));
    }
    let after_external = doc.transact().state_vector();
    assert_ne!(before, after_external);
    let txn = doc.transact();
    let error = preflight_mutation_plan(79, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(txn.state_vector(), after_external);
}

#[test]
fn yrs_scan_accounting_accepts_exact_limit_rejects_one_over_and_amplification() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcdef" }]
        }]
    });
    let compile = |max_input_bytes: usize, source: &Value, operations: Vec<TypedOperation>| {
        let (doc, schema, mut limits, editing_limits, document) = diagnostic_doc(source);
        limits.max_input_bytes = max_input_bytes;
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
                request_id: 80,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let insertion = || TypedOperation::InsertText {
        at: point_for_test(1),
        text: "x".into(),
        marks: vec![],
    };
    compile(33, &source, vec![insertion()]).unwrap();
    let one_over = compile(32, &source, vec![insertion()]).unwrap_err();
    assert_eq!(one_over.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(one_over.limit, Some(32));

    let large = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "z".repeat(4_096) }]
        }]
    });
    let amplified = compile(20_000, &large, vec![insertion(); 8]).unwrap_err();
    assert_eq!(amplified.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(amplified.details, Some(json!({ "field": "maxInputBytes" })));

    let large_bold = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "z".repeat(4_096),
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let noop = || TypedOperation::AddMark {
        range: range_for_test(0, 4_096),
        mark: Mark::new("bold".into(), HashMap::new()),
    };
    let noops = vec![noop(); 8];
    assert!(compile(8_240, &large_bold, noops.clone())
        .unwrap()
        .mutation_plan
        .actions
        .is_empty());
    assert_eq!(
        compile(8_239, &large_bold, noops).unwrap_err().limit,
        Some(8_239)
    );
}

#[test]
fn invalid_envelopes_reject_before_yrs_scan_and_semantic_noop_charges_initial_scan() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{
                "type": "text",
                "text": "abcdef",
                "marks": [{ "type": "bold" }]
            }]
        }]
    });
    let (doc, schema, mut limits, mut editing_limits, document) = diagnostic_doc(&source);
    limits.max_input_bytes = 1;
    editing_limits.max_operations_per_transaction = 1;
    let compile = |base_document_revision, origin, operations| {
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
                request_id: 81,
                base_document_revision,
                origin,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    let noop = || TypedOperation::AddMark {
        range: range_for_test(0, 6),
        mark: Mark::new("bold".into(), HashMap::new()),
    };
    assert_eq!(
        compile(1, TransactionOrigin::LocalInput, vec![])
            .unwrap_err()
            .code,
        "REVISION_MISMATCH"
    );
    assert_eq!(
        compile(0, TransactionOrigin::RemoteSync, vec![])
            .unwrap_err()
            .code,
        "TRANSACTION_INVALID"
    );
    assert_eq!(
        compile(0, TransactionOrigin::LocalInput, vec![noop(), noop()])
            .unwrap_err()
            .details,
        Some(json!({ "field": "maxOperationsPerTransaction" }))
    );

    let (doc, schema, mut exact_limits, editing_limits, document) = diagnostic_doc(&source);
    // 6 admitted mark bytes + 12 bytes for materialization and coordinate scan.
    exact_limits.max_input_bytes = 18;
    let compile_noop = |limits: &ResourceLimits| {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &document,
                selection: None,
                schema: &schema,
                resource_limits: limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 82,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalCommand,
                operations: vec![noop()],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
    };
    assert!(compile_noop(&exact_limits)
        .unwrap()
        .mutation_plan
        .actions
        .is_empty());
    exact_limits.max_input_bytes = 17;
    assert_eq!(compile_noop(&exact_limits).unwrap_err().limit, Some(17));
}

#[test]
fn wide_target_paths_and_boundary_partition_work_stay_indexed_and_charged() {
    const BLOCKS: usize = 256;
    let source = json!({
        "type": "doc",
        "content": (0..BLOCKS)
            .map(|_| json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "x" }]
            }))
            .collect::<Vec<_>>()
    });
    let (doc, schema, limits, _editing_limits, _document) = diagnostic_doc(&source);
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mut compiler = super::mutation::MutationCompiler::new(
        85,
        &txn,
        &fragment,
        &schema,
        BLOCKS * 100,
        limits.max_input_bytes,
        0,
    )
    .unwrap();
    let traversal_work = compiler.total_mutation_work_for_test();
    assert!(traversal_work <= BLOCKS * 12, "{traversal_work}");

    let boundary_count = BLOCKS * 3 + 1;
    let boundaries = (0..u32::try_from(boundary_count).unwrap()).collect::<Vec<_>>();
    let before = compiler.total_mutation_work_for_test();
    let error = compiler
        .delete(0, 1, u32::try_from(BLOCKS * 3 - 1).unwrap(), &boundaries)
        .unwrap_err();
    assert_eq!(error.code, "OPERATION_INVALID");
    let charged = compiler.total_mutation_work_for_test() - before;
    assert!(charged >= BLOCKS * 15, "{charged}");
    assert!(charged <= BLOCKS * 32, "{charged}");
}

#[test]
fn wide_preflight_and_virtual_delete_are_linear_in_children_and_spans() {
    const TARGETS: usize = 128;
    let source = json!({
        "type": "doc",
        "content": (0..TARGETS)
            .map(|_| json!({
                "type": "paragraph",
                "content": [{ "type": "text", "text": "x" }]
            }))
            .collect::<Vec<_>>()
    });
    let (doc, schema, limits, _editing_limits, _document) = diagnostic_doc(&source);
    let mut plan = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let mut compiler = super::mutation::MutationCompiler::new(
            88,
            &txn,
            &fragment,
            &schema,
            TARGETS * TARGETS * 4,
            limits.max_input_bytes,
            0,
        )
        .unwrap();
        for index in (0..TARGETS).rev() {
            compiler
                .insert(index, u32::try_from(1 + index * 3).unwrap(), "y", &[])
                .unwrap();
        }
        compiler.finish(Some(TARGETS - 1)).unwrap()
    };
    let txn = doc.transact();
    let preflight_work = preflight_mutation_work_for_test(88, &plan, &txn).unwrap();
    assert!(preflight_work <= TARGETS * 8, "{preflight_work}");
    let exact_limit = plan.compilation_work_for_test() + preflight_work;
    plan.set_work_limit_for_test(exact_limit);
    preflight_mutation_plan(88, &plan, &txn).unwrap();
    plan.set_work_limit_for_test(exact_limit - 1);
    let one_over = preflight_mutation_plan(88, &plan, &txn).unwrap_err();
    assert_eq!(one_over.code, "OPERATION_LIMIT_EXCEEDED");
    assert_eq!(
        one_over.limit,
        Some(u64::try_from(exact_limit - 1).unwrap())
    );
    assert_eq!(one_over.actual, Some(u64::try_from(exact_limit).unwrap()));

    let multi = utf16_doc();
    {
        let mut txn = multi.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        for index in 0..TARGETS {
            paragraph.insert(
                &mut txn,
                u32::try_from(index).unwrap(),
                XmlTextPrelim::new("x"),
            );
        }
    }
    let txn = multi.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let mut compiler = super::mutation::MutationCompiler::new(
        89,
        &txn,
        &fragment,
        &schema,
        TARGETS * 20,
        limits.max_input_bytes,
        0,
    )
    .unwrap();
    compiler
        .delete(0, 1, u32::try_from(TARGETS + 1).unwrap(), &[])
        .unwrap();
    assert_eq!(compiler.virtual_delete_visits_for_test(), TARGETS);
}

#[test]
fn unicode_actions_store_checked_utf16_coordinates() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "🙂a" }]
        }]
    });
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
                request_id: 72,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert!(matches!(
        &compiled.mutation_plan.actions[0],
        YrsMutationAction::InsertText { index_utf16: 2, .. }
    ));
}

#[test]
fn unaffected_branch_identity_and_sticky_resolution_survive_local_edit() {
    let source: Value = serde_json::from_str(TWO_PARAGRAPHS).unwrap();
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
    let (unaffected_id, sticky, before_vector) = {
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let second = paragraph_text(&fragment, &txn, 1);
        let id = <XmlTextRef as AsRef<Branch>>::as_ref(&second).id();
        let sticky = StickyIndex::at(
            &txn,
            BranchPtr::from(<XmlTextRef as AsRef<Branch>>::as_ref(&second)),
            2,
            Assoc::After,
        )
        .unwrap();
        (id, sticky, txn.state_vector())
    };
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
                request_id: 73,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "!".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let estimate = compiled.encoded_growth_bound;
    {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(compiled.mutation_plan, &mut txn);
    }
    let txn = doc.transact();
    let fragment = txn.get_xml_fragment("prosemirror").unwrap();
    let second = paragraph_text(&fragment, &txn, 1);
    assert_eq!(
        <XmlTextRef as AsRef<Branch>>::as_ref(&second).id(),
        unaffected_id
    );
    let resolved = sticky.get_offset(&txn).unwrap();
    assert_eq!(resolved.index, 2);
    assert_eq!(resolved.branch.id(), unaffected_id);
    let update = txn.encode_state_as_update_v1(&before_vector);
    assert!(!update.is_empty());
    assert!(update.len() <= estimate);
    assert!(update.len() < TWO_PARAGRAPHS.len());
}

#[test]
fn local_update_growth_is_independent_of_unaffected_article_size() {
    let source = |tail: String| {
        json!({
            "type": "doc",
            "content": [
                {"type": "paragraph", "content": [{"type": "text", "text": "alpha"}]},
                {"type": "paragraph", "content": [{"type": "text", "text": tail}]}
            ]
        })
    };
    let operation = || TypedOperation::InsertText {
        at: point_for_test(1),
        text: "!".into(),
        marks: vec![],
    };
    let (_, _, _, small_update, small_estimate) =
        compile_and_execute(source("omega".into()), vec![operation()]);
    let (_, _, _, large_update, large_estimate) =
        compile_and_execute(source("z".repeat(16_384)), vec![operation()]);

    assert!(small_update <= small_estimate);
    assert!(large_update <= large_estimate);
    assert!(large_update <= small_update + 16);
    assert!(large_update < 256);
}

#[test]
fn estimator_bounds_large_text_and_attribute_payloads() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcdef" }]
        }]
    });
    let payload = "x".repeat(8_192);
    compile_and_execute(
        source,
        vec![TypedOperation::InsertText {
            at: point_for_test(3),
            text: payload.clone(),
            marks: vec![Mark::new(
                "link".into(),
                HashMap::from([("href".into(), Value::String(payload))]),
            )],
        }],
    );
}

#[test]
fn format_actions_split_at_mark_boundaries_inside_one_xml_text() {
    let schema = tiptap_schema();
    let limits = ResourceLimits::default();
    let editing_limits = EditingLimits::default();
    let doc = utf16_doc();
    let source = {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        let paragraph = fragment.insert(&mut txn, 0, XmlElementPrelim::empty("paragraph"));
        let text = paragraph.insert(&mut txn, 0, XmlTextPrelim::new("abcdef"));
        text.format(
            &mut txn,
            2,
            2,
            Attrs::from([("bold".into(), Any::Bool(true))]),
        );
        YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap()
    };
    let document = from_prosemirror_json(&source, &schema, UnknownTypeMode::Preserve).unwrap();
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
                request_id: 74,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::AddMark {
                    range: range_for_test(0, 6),
                    mark: Mark::new("italic".into(), HashMap::new()),
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(compiled.mutation_plan.actions.len(), 3);
    assert!(compiled
        .mutation_plan
        .actions
        .iter()
        .all(|action| matches!(action, YrsMutationAction::FormatText { .. })));
}

#[test]
fn preflight_rejects_a_target_changed_after_compilation() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "Hello" }]
        }]
    });
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
                request_id: 75,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![TypedOperation::InsertText {
                    at: point_for_test(1),
                    text: "x".into(),
                    marks: vec![],
                }],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    {
        let mut txn = doc.transact_mut();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        paragraph_text(&fragment, &txn, 0).insert(&mut txn, 0, "!");
    }
    let txn = doc.transact();
    let error = preflight_mutation_plan(75, &compiled.mutation_plan, &txn).unwrap_err();
    assert_eq!(error.code, "ENGINE_INVARIANT_FAILED");
    assert_eq!(error.operation_index, Some(0));
}

#[test]
fn replace_mark_then_replace_range_executes_to_preview() {
    let source = json!({
        "type": "doc",
        "content": [{
            "type": "paragraph",
            "content": [{ "type": "text", "text": "abcdef" }]
        }]
    });
    let mark = TypedOperation::ReplaceMark {
        range: range_for_test(1, 4),
        mark: Mark::new(
            "link".into(),
            HashMap::from([("href".into(), Value::String("a".into()))]),
        ),
    };
    let replace = TypedOperation::ReplaceRange {
        range: range_for_test(2, 3),
        content: Fragment::from(vec![Node::text("a".into(), vec![])]),
    };
    let (doc, schema, limits, editing_limits, document) = diagnostic_doc(&source);
    let compile = |operations| {
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
                request_id: 76,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations,
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    let marked = compile(vec![mark.clone()]);
    let compiled = compile(vec![mark.clone(), replace.clone()]);
    assert_eq!(marked.preview.root().text_content(), "abcdef");
    assert_eq!(compiled.preview.root().text_content(), "abadef");

    let mut executed_text = String::new();
    for action in compiled.mutation_plan.actions.clone() {
        let mut txn = doc.transact_mut();
        execute_mutation_plan(
            super::mutation::YrsMutationPlan::single_action_for_test(action),
            &mut txn,
        );
        drop(txn);
        let txn = doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        let decoded = YrsDocumentCodec::new(&schema, &limits)
            .read_json(&fragment, &txn)
            .unwrap();
        let decoded = from_prosemirror_json(&decoded, &schema, UnknownTypeMode::Preserve).unwrap();
        executed_text = decoded.root().text_content();
    }

    let (_, _, _, _, inverse_document) = diagnostic_doc(&source);
    let inverse_doc = utf16_doc();
    let codec = YrsDocumentCodec::new(&schema, &limits);
    {
        let mut txn = inverse_doc.transact_mut();
        let fragment = txn.get_or_insert_xml_fragment("prosemirror");
        codec
            .apply_json(&fragment, &mut txn, &json!({ "type": "doc" }), &source)
            .unwrap();
    }
    let inverse = {
        let txn = inverse_doc.transact();
        let fragment = txn.get_xml_fragment("prosemirror").unwrap();
        compile_transaction_with_yrs(
            CompilationContext {
                document: &inverse_document,
                selection: None,
                schema: &schema,
                resource_limits: &limits,
                editing_limits: &editing_limits,
                document_revision: 0,
                max_length: None,
            },
            TypedTransaction {
                request_id: 77,
                base_document_revision: 0,
                origin: TransactionOrigin::LocalInput,
                operations: vec![replace, mark],
                selection_intent: SelectionIntent::UseOperationResult,
                history_policy: HistoryPolicy::Auto,
            },
            &txn,
            &fragment,
        )
        .unwrap()
    };
    assert_eq!(executed_text, compiled.preview.root().text_content());
    assert_eq!(inverse.preview.root().text_content(), "abadef");
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn estimated_update_growth_bounds_supported_action_mixes(
        generated in prop::collection::vec((0u8..6, "[a-z]{1,3}"), 1..8)
    ) {
        let source = json!({
            "type": "doc",
            "content": [{
                "type": "paragraph",
                "content": [{ "type": "text", "text": "abcdef" }]
            }]
        });
        let operations = generated
            .into_iter()
            .map(|(kind, text)| match kind {
                0 => TypedOperation::InsertText {
                    at: point_for_test(2),
                    text,
                    marks: vec![],
                },
                1 => TypedOperation::DeleteRange {
                    range: range_for_test(2, 3),
                },
                2 => TypedOperation::ReplaceRange {
                    range: range_for_test(2, 3),
                    content: Fragment::from(vec![Node::text(text, vec![])]),
                },
                3 => TypedOperation::AddMark {
                    range: range_for_test(1, 4),
                    mark: Mark::new("bold".into(), HashMap::new()),
                },
                4 => TypedOperation::RemoveMark {
                    range: range_for_test(1, 4),
                    mark_type: "bold".into(),
                },
                _ => TypedOperation::ReplaceMark {
                    range: range_for_test(1, 4),
                    mark: Mark::new(
                        "link".into(),
                        HashMap::from([("href".into(), Value::String(text))]),
                    ),
                },
            })
            .collect();
        compile_and_execute(source, operations);
    }
}

fn point_for_test(offset: u32) -> RevisionedPosition {
    RevisionedPosition {
        offset,
        kind: EditorOffsetKind::Scalar,
        affinity: Affinity::After,
    }
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
